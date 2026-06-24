#![cfg(feature = "rand")]

use core::borrow::Borrow as _;
use core::fmt::Debug;
use core::ops::Bound;
use core::ops::ControlFlow;
use std::collections::BTreeMap;
use std::thread;

use arctic::concurrent::Map;
use arctic::key::BoxedStr;
use arctic::key::NonNull;
use rand::Rng as _;
use rand::RngExt as _;
use rand::SeedableRng;

trait Orthogonal: ::arctic::Key {
    fn is_thread(key: Self::Insert<'_>, thread: u8) -> bool;
    fn mask(self, thread: u8) -> Self;
}

impl Orthogonal for u64 {
    fn is_thread(key: Self::Insert<'_>, thread: u8) -> bool {
        key as u8 == thread
    }

    fn mask(mut self, thread: u8) -> Self {
        self &= !(u8::MAX as u64);
        self |= thread as u64;
        self
    }
}

impl Orthogonal for BoxedStr<NonNull> {
    fn is_thread(key: Self::Insert<'_>, thread: u8) -> bool {
        key.as_str().as_bytes().last() == Some(&(thread + 1))
    }

    fn mask(self, thread: u8) -> Self {
        let mut string = self.into_boxed_slice().into_string();
        assert!(thread < 0b0111_1111);
        string.push((thread + 1) as char);
        unsafe { Self::new_unchecked(string.into_boxed_str()) }
    }
}

enum Op {
    Upsert,
    // Update,
    // Insert,
    Remove,
    Get,
    Range,
}

static OPS: &[Op] = &[
    Op::Upsert,
    // Op::Update,
    // Op::Insert,
    Op::Remove,
    Op::Get,
    Op::Range,
];

#[test]
fn orthogonal_u64_u64() {
    orthogonal::<u64>();
}

#[test]
fn orthogonal_boxed_str_non_null_u64() {
    orthogonal::<BoxedStr<NonNull>>();
}

fn orthogonal<K>()
where
    K: ::arctic::Key + Orthogonal + Debug + Ord + Clone,
    K::Borrowed: Ord + Eq,
    rand::distr::StandardUniform: rand::distr::Distribution<K>,
{
    let map = &Map::<K, u64>::new();
    let ops = &rand::distr::slice::Choose::new(OPS).unwrap();

    thread::scope(|scope| {
        for thread in 0..16 {
            scope.spawn(move || {
                let mut rng = rand::rngs::Xoshiro256PlusPlus::seed_from_u64(thread);
                let mut expected = BTreeMap::<K, u64>::new();

                for _ in 0..10_000 {
                    let key = rng.random::<K>().mask(thread as u8);

                    match rng.sample(ops) {
                        Op::Upsert => {
                            let value = rng.next_u64();
                            let upserted = map.upsert(key.as_insert(), value);
                            assert_eq!(upserted.old(), expected.insert(key.clone(), value).as_ref());
                            assert_eq!(*upserted.new(), value);
                        }
                        // Op::Update => todo!(),
                        // Op::Insert => todo!(),
                        Op::Remove => {
                            assert_eq!(map.remove(key.borrow()).as_deref(), expected.remove(key.borrow()).as_ref());
                        }
                        Op::Get => {
                            assert_eq!(map.get(key.borrow()).as_deref(), expected.get(key.borrow()));
                        }
                        Op::Range => {
                            let descend = rng.random::<bool>();
                            let mut lower = rng.random::<K>();
                            let mut upper = rng.random::<K>();

                            if lower > upper {
                                core::mem::swap(&mut lower, &mut upper);
                            }

                            let actual = map.range(lower.borrow()..=upper.borrow());
                            let expected = expected.range((Bound::Included(lower.borrow()), Bound::Included(upper.borrow())));
                            let mut expected = if descend {
                                Box::new(expected.rev())
                            } else {
                                Box::new(expected) as Box<dyn Iterator<Item = _>>
                            };

                            macro_rules! compare {
                                () => {
                                    |(key_actual, value_actual)| {
                                        if !K::is_thread(key_actual, thread as u8) {
                                            return ControlFlow::Continue(());
                                        }

                                        let key_actual = key_actual.borrow();
                                        let (key_expected, value_expected) = expected.next().unwrap();
                                        assert_eq!(
                                            key_actual,
                                            key_expected.borrow(),
                                            "actual key: {key_actual:x?}, expected key: {key_expected:x?}, lower: {lower:x?}, upper: {upper:x?}",
                                        );
                                        assert_eq!(
                                            value_actual, value_expected,
                                            "actual value: {value_actual:x?}, expected value: {value_expected:x?}",
                                        );
                                        ControlFlow::Continue(())
                                    }
                                };
                            }

                            if descend {
                                actual
                                    .entries::<arctic::Descend>()
                                    .for_each_internal(compare!())
                            } else {
                                actual
                                    .entries::<arctic::Ascend>()
                                    .for_each_internal(compare!())
                            }

                            let next = expected.next();
                            assert!(next.is_none(), "Missing entry {next:?}");
                        }
                    }
                }
            });
        }
    });
}
