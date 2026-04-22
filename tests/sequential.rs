use core::fmt::Debug;
use std::collections::BTreeMap;

use arctic::Ascend;
use proptest::arbitrary::Arbitrary;
use proptest::prelude::Just;
use proptest::prelude::Strategy as _;
use proptest::prop_oneof;
use proptest::sample::Selector;
use proptest_state_machine::ReferenceStateMachine;
use proptest_state_machine::StateMachineTest;
use proptest_state_machine::prop_state_machine;

prop_state_machine! {
    #[test]
    fn u64_u64(
        sequential
        1000
        =>
        Arctic<u64, u64>
    );
}

#[derive(Clone, Debug)]
pub enum Transition<K, V> {
    Upsert(K, V),
    Remove(K),
}

#[derive(Debug, Clone)]
struct Map<K, V>(BTreeMap<K, V>);

impl<K, V> ReferenceStateMachine for Map<K, V>
where
    K: Arbitrary + Clone + Debug + Default + Ord + 'static,
    V: Arbitrary + Clone + Debug + 'static,
{
    type State = Self;
    type Transition = Transition<K, V>;

    fn init_state() -> proptest::prelude::BoxedStrategy<Self::State> {
        Just(Self(BTreeMap::new())).boxed()
    }

    fn transitions(state: &Self::State) -> proptest::prelude::BoxedStrategy<Self::Transition> {
        let state = state.clone();
        prop_oneof![
            1 => (K::arbitrary(), V::arbitrary()).prop_map(|(key, value)| Transition::Upsert(key, value)),
            1 => proptest::prelude::any::<Selector>().prop_map(move |selector| {
                let key = if state.0.is_empty() {
                    K::default()
                } else {
                    selector.select(state.0.keys()).clone()
                };
                Transition::Remove(key)
            }),
        ].boxed()
    }

    fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State {
        match transition {
            Transition::Upsert(key, value) => {
                state.0.insert(key.clone(), value.clone());
            }
            Transition::Remove(key) => {
                state.0.remove(key);
            }
        }
        state
    }
}

struct Arctic<K: arctic::Key, V: arctic::Value>(arctic::concurrent::Map<K, V>);

impl<K, V> StateMachineTest for Arctic<K, V>
where
    K: arctic::Key + Arbitrary + Clone + Debug + Default + Ord + 'static,
    V: arctic::Value + Arbitrary + Clone + Debug + Send + Sync + 'static,
    V::Target: Debug + PartialEq,
{
    type SystemUnderTest = Self;

    type Reference = Map<K, V>;

    fn init_test(_: &<Self::Reference as ReferenceStateMachine>::State) -> Self::SystemUnderTest {
        Arctic(arctic::concurrent::Map::default())
    }

    fn apply(
        mut state: Self::SystemUnderTest,
        expected: &<Self::Reference as ReferenceStateMachine>::State,
        transition: <Self::Reference as ReferenceStateMachine>::Transition,
    ) -> Self::SystemUnderTest {
        match transition {
            Transition::Upsert(key, value) => {
                state.0.upsert(K::borrow_insert(&key), value);
            }
            Transition::Remove(key) => {
                state.0.remove(K::borrow(&key));
            }
        }

        state
            .0
            .as_sequential()
            .all()
            .entries::<Ascend>()
            .zip(expected.0.iter())
            .for_each(
                |((actual_key, actual_value), (expected_key, expected_value))| {
                    assert_eq!(actual_key, *expected_key);
                    assert_eq!(actual_value, expected_value.as_target());
                },
            );

        state
    }
}
