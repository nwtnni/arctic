#![cfg(feature = "proptest")]

use core::borrow::Borrow as _;
use core::fmt::Debug;
use core::ops::ControlFlow;
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use proptest::arbitrary::Arbitrary;
use proptest::prelude::Just;
use proptest::prelude::Strategy as _;
use proptest::prop_oneof;
use proptest::sample::Selector;
use proptest_state_machine::ReferenceStateMachine;
use proptest_state_machine::StateMachineTest;

macro_rules! impl_suite {
    ($mod:ident, $ty:ident) => {
        mod $mod {
            use super::$ty;

            type NonNullString = arctic::key::BoxedStr<arctic::key::NonNull>;

            proptest_state_machine::prop_state_machine! {
                #[test]
                fn u16_u64(
                    sequential
                    1000
                    =>
                    $ty<u16, u64>
                );

                #[test]
                fn u32_u64(
                    sequential
                    1000
                    =>
                    $ty<u32, u64>
                );

                #[test]
                fn u64_u64(
                    sequential
                    1000
                    =>
                    $ty<u64, u64>
                );

                #[test]
                fn u128_u64(
                    sequential
                    1000
                    =>
                    $ty<u128, u64>
                );

                #[test]
                fn boxed_str_non_null_u64(
                    sequential
                    1000
                    =>
                    $ty<NonNullString, u64>
                );
            }
        }
    };
}

impl_suite!(sequential, Sequential);
impl_suite!(concurrent, Concurrent);

#[derive(Clone, Debug)]
pub enum Transition<K, V> {
    Upsert(K, V),
    Update(K, V),
    Insert(K, V),
    Remove(K),
    Get(K),
    Range { descend: bool, lower: K, upper: K },
}

#[derive(Debug, Clone)]
struct Map<K, V> {
    map: BTreeMap<K, V>,
    prev: Option<V>,
}

impl<K, V> ReferenceStateMachine for Map<K, V>
where
    K: Arbitrary + Clone + Debug + Default + Ord + 'static,
    V: Arbitrary + Clone + Debug + 'static,
{
    type State = Self;
    type Transition = Transition<K, V>;

    fn init_state() -> proptest::prelude::BoxedStrategy<Self::State> {
        Just(Self {
            map: BTreeMap::new(),
            prev: None,
        })
        .boxed()
    }

    fn transitions(state: &Self::State) -> proptest::prelude::BoxedStrategy<Self::Transition> {
        prop_oneof![
            1 => (K::arbitrary(), V::arbitrary()).prop_map(|(key, value)| Transition::Upsert(key, value)),
            1 => (K::arbitrary(), V::arbitrary()).prop_map(|(key, value)| Transition::Update(key, value)),
            1 => (K::arbitrary(), V::arbitrary()).prop_map(|(key, value)| Transition::Insert(key, value)),
            1 => K::arbitrary().prop_map(|key| Transition::Get(key)),

            1 => proptest::prelude::any::<Selector>().prop_map({
                let state = state.clone();
                move |selector| {
                    let key = if state.map.is_empty() {
                        K::default()
                    } else {
                        selector.select(state.map.keys()).clone()
                    };
                    Transition::Remove(key)
                }
            }),
            1 => (bool::arbitrary(), K::arbitrary(), proptest::prelude::any::<Selector>()).prop_map({
                let state = state.clone();
                move |(descend, random, selector)| {
                    if state.map.is_empty() {
                        return Transition::Range { descend, lower: K::default(), upper: K::default() };
                    }

                    let mut lower = random;
                    let mut upper = selector.select(state.map.keys()).clone();

                    if lower > upper {
                        core::mem::swap(&mut lower, &mut upper);
                    }

                    Transition::Range { descend, lower, upper }
                }
            })
        ].boxed()
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Upsert(key, value) => {
                state.prev = state.map.insert(key.clone(), value.clone());
            }
            Transition::Update(key, value) => match state.map.entry(key.clone()) {
                Entry::Occupied(mut entry) => state.prev = Some(entry.insert(value.clone())),
                Entry::Vacant(_) => state.prev = None,
            },
            Transition::Insert(key, value) => match state.map.entry(key.clone()) {
                Entry::Vacant(entry) => {
                    state.prev = None;
                    entry.insert(value.clone());
                }
                Entry::Occupied(entry) => {
                    state.prev = Some(entry.get().clone());
                }
            },
            Transition::Remove(key) => {
                state.prev = state.map.remove(key);
            }
            Transition::Get(_) | Transition::Range { .. } => (),
        }
        state
    }
}

struct Sequential<K: arctic::Key, V: arctic::concurrent::Value>(arctic::sequential::Map<K, V>);

impl<K, V> StateMachineTest for Sequential<K, V>
where
    K: arctic::Key + Arbitrary + Clone + Debug + Default + Ord + 'static,
    K::Borrowed: Ord + core::fmt::Debug,
    V: arctic::concurrent::Value + Arbitrary + Clone + Debug + Eq + Send + Sync + 'static,
    V::Borrowed: Debug + PartialEq + PartialEq<V> + Clone,
    for<'a> Option<&'a V::Borrowed>: PartialEq<Option<&'a V>>,
{
    type SystemUnderTest = Self;

    type Reference = Map<K, V>;

    fn init_test(_: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
        Sequential(arctic::sequential::Map::default())
    }

    fn apply(
        mut state: Self::SystemUnderTest,
        expected: &<Self::Reference as ReferenceStateMachine>::State,
        transition: <Self::Reference as ReferenceStateMachine>::Transition,
    ) -> Self::SystemUnderTest {
        match transition {
            Transition::Upsert(key, value) => {
                let upserted = state.0.upsert(K::as_insert(&key), value.clone());
                match upserted {
                    Ok((old, new)) => {
                        assert_eq!(Some(&old), expected.prev.as_ref());
                        assert_eq!(new, &value);
                    }
                    Err(new) => assert_eq!(new, &value),
                }

                assert_eq!(state.0.get(key.borrow()), Some(&value));
            }
            Transition::Update(key, value) => match state.0.update(key.borrow(), value.clone()) {
                Ok((old, new)) => {
                    assert_eq!(Some(&old), expected.prev.as_ref());
                    assert_eq!(new, &value);
                    assert_eq!(state.0.get(key.borrow()), Some(&value));
                }
                Err(new) => {
                    assert!(expected.prev.is_none());
                    assert!(state.0.get(key.borrow()).is_none());
                    assert_eq!(new, value);
                }
            },
            Transition::Insert(key, value) => {
                match state.0.insert(K::as_insert(&key), value.clone()) {
                    Ok(new) => {
                        assert_eq!(new, &value);
                        assert_eq!(state.0.get(key.borrow()), Some(&value));
                    }
                    Err((old, new)) => {
                        assert_eq!(Some(&*old), expected.prev.as_ref());
                        assert_eq!(new, value);
                        let value = (*old).clone();
                        assert_eq!(
                            state
                                .0
                                .get(key.borrow())
                                .as_deref()
                                .expect("Insert previous is Some"),
                            &value
                        );
                    }
                }
            }
            Transition::Remove(key) => {
                let removed = state.0.remove(K::borrow(&key));
                assert_eq!(removed, expected.prev);
                assert!(state.0.get(key.borrow()).is_none());
            }
            Transition::Get(key) => {
                assert_eq!(state.0.get(key.borrow()), expected.map.get(key.borrow()));
            }
            Transition::Range {
                descend,
                lower,
                upper,
            } => {
                let actual = state.0.range(lower.borrow()..=upper.borrow());
                let expected = expected.map.range::<K, _>(lower.clone()..=upper.clone());
                let mut expected = if descend {
                    Box::new(expected.rev())
                } else {
                    Box::new(expected) as Box<dyn Iterator<Item = _>>
                };

                macro_rules! compare {
                        () => {
                            |(key_actual, value_actual)| {
                                let key_actual: &K::Borrowed = key_actual.borrow();
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

        state
    }
}

struct Concurrent<K: arctic::Key, V: arctic::concurrent::Value>(arctic::concurrent::Map<K, V>);

impl<K, V> StateMachineTest for Concurrent<K, V>
where
    K: arctic::Key + Arbitrary + Clone + Debug + Default + Ord + 'static,
    K::Borrowed: Ord + core::fmt::Debug,
    V: arctic::concurrent::Value + Arbitrary + Clone + Debug + Eq + Send + Sync + 'static,
    V::Borrowed: Debug + PartialEq + PartialEq<V> + Clone,
    for<'a> Option<&'a V::Borrowed>: PartialEq<Option<&'a V>>,
{
    type SystemUnderTest = Self;

    type Reference = Map<K, V>;

    fn init_test(_: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
        Concurrent(arctic::concurrent::Map::default())
    }

    fn apply(
        state: Self::SystemUnderTest,
        expected: &<Self::Reference as ReferenceStateMachine>::State,
        transition: <Self::Reference as ReferenceStateMachine>::Transition,
    ) -> Self::SystemUnderTest {
        match transition {
            Transition::Upsert(key, value) => {
                let upserted = state.0.upsert(K::as_insert(&key), value.clone());
                assert_eq!(upserted.old(), expected.prev.as_ref());
                assert_eq!(upserted.new(), &value);
                drop(upserted);
                assert_eq!(state.0.get(key.borrow()).as_deref(), Some(&value));
            }
            Transition::Update(key, value) => match state.0.update(key.borrow(), value.clone()) {
                Ok(updated) => {
                    assert_eq!(
                        updated.old(),
                        expected.prev.as_ref().expect("Update previous is Some"),
                    );
                    assert_eq!(updated.new(), &value);
                    drop(updated);
                    assert_eq!(state.0.get(key.borrow()).as_deref(), Some(&value));
                }
                Err(new) => {
                    assert_eq!(new, value);
                    assert!(state.0.get(key.borrow()).is_none());
                }
            },
            Transition::Insert(key, value) => {
                match state.0.insert(K::as_insert(&key), value.clone()) {
                    Ok(new) => {
                        assert_eq!(&*new, &value);
                        assert_eq!(state.0.get(key.borrow()).as_deref(), Some(&value));
                    }
                    Err((old, new)) => {
                        assert_eq!(
                            &*old,
                            expected.prev.as_ref().expect("Insert previous is Some")
                        );
                        assert_eq!(new, value);
                        let value = (*old).clone();
                        drop(old);
                        assert_eq!(
                            state
                                .0
                                .get(key.borrow())
                                .as_deref()
                                .expect("Insert previous is Some"),
                            &value
                        );
                    }
                }
            }
            Transition::Remove(key) => {
                let removed = state.0.remove(K::borrow(&key));
                assert_eq!(removed.as_deref(), expected.prev.as_ref());
                assert!(state.0.get(key.borrow()).is_none());
            }
            Transition::Get(key) => {
                assert_eq!(
                    state.0.get(key.borrow()).as_deref(),
                    expected.map.get(key.borrow()),
                );
            }
            Transition::Range {
                descend,
                lower,
                upper,
            } => {
                let actual = state.0.range(lower.borrow()..=upper.borrow());
                let expected = expected.map.range::<K, _>(lower.clone()..=upper.clone());
                let mut expected = if descend {
                    Box::new(expected.rev())
                } else {
                    Box::new(expected) as Box<dyn Iterator<Item = _>>
                };

                macro_rules! compare {
                        () => {
                            |(key_actual, value_actual)| {
                                let key_actual: &K::Borrowed = key_actual.borrow();
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

        state
    }
}
