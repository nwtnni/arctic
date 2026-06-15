use core::borrow::Borrow as _;
use core::ops::ControlFlow;
use std::collections::BTreeMap;
use std::thread;

use arctic::concurrent::Map;
use rand::Rng as _;
use rand::RngExt as _;
use rand::SeedableRng;

trait Orthogonal: arctic::raw::Key {
    fn is_thread(&self, thread: u8) -> bool;
    fn mask(self, thread: u8) -> Self;
}

impl Orthogonal for u64 {
    fn is_thread(&self, thread: u8) -> bool {
        for bit in 0..8 {
            let set = ((thread >> bit) & 1) as u64;
            let byte = (bit << 3) + 4;

            if self & (1 << byte) != (set << byte) {
                return false;
            }
        }

        true
    }

    fn mask(mut self, thread: u8) -> Self {
        for bit in 0..8 {
            let set = ((thread >> bit) & 1) as u64;
            let byte = (bit << 3) + 4;

            self &= !(1 << byte);
            self |= set << byte;
        }
        self
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
fn orthogonal() {
    let map = &Map::<u64, u64>::new();
    let ops = &rand::distr::slice::Choose::new(OPS).unwrap();

    thread::scope(|scope| {
        for thread in 0..16 {
            scope.spawn(move || {
                let mut rng = rand::rngs::Xoshiro256PlusPlus::seed_from_u64(thread);
                let mut expected = BTreeMap::<u64, u64>::new();

                for _ in 0..10_000 {
                    let key = rng.next_u64().mask(thread as u8);

                    match rng.sample(ops) {
                        Op::Upsert => {
                            let value = rng.next_u64();
                            let upserted = map.upsert(key, value);
                            assert_eq!(upserted.old(), expected.insert(key, value).as_ref());
                            assert_eq!(*upserted.new(), value);
                        }
                        // Op::Update => todo!(),
                        // Op::Insert => todo!(),
                        Op::Remove => {
                            assert_eq!(map.remove(&key).as_deref(), expected.remove(&key).as_ref());
                        }
                        Op::Get => {
                            assert_eq!(map.get(&key).as_deref(), expected.get(&key));
                        }
                        Op::Range => {
                            let descend = rng.random::<bool>();
                            let mut lower = rng.next_u64();
                            let mut upper = rng.next_u64();

                            if lower > upper {
                                core::mem::swap(&mut lower, &mut upper);
                            }

                            let actual = map.range(lower..=upper);
                            let expected = expected.range(lower..=upper);
                            let mut expected = if descend {
                                Box::new(expected.rev())
                            } else {
                                Box::new(expected) as Box<dyn Iterator<Item = _>>
                            };

                            macro_rules! compare {
                                () => {
                                    |(key_actual, value_actual)| {
                                        if !key_actual.is_thread(thread as u8) {
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
