//! Defines a common trait [`Header`] for node types with header metadata.
//!
//! Also contains a shared implementation of the [`crate::raw::node::Node`]
//! for node types with headers.

use core::fmt::Debug;

use crate::raw::node;
use crate::raw::node::KeyIndex;

/// # Safety
///
/// Implementer must guarantee indices returned from all methods were
/// previously successfully inserted by `get_or_insert`.
pub(in crate::raw) unsafe trait Header:
    Clone + Debug + Default + Sized + Send + Sync + 'static
{
    /// A runtime representation of the node type.
    const TYPE: node::Type;
    type KeyIter: Default + Iterator<Item = KeyIndex> + core::fmt::Debug;

    fn freeze(&self) -> usize;

    fn get(&self, key: u8) -> Option<u8>;

    fn get_or_insert(&self, key: u8) -> Option<u8>;

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        &self,
        lower: L,
        upper: U,
        iter: &mut Self::KeyIter,
    );

    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex>;
    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex>;

    /// The number of byte to edge mappings contained in this header.
    ///
    /// NOTE: this can differ from the number of non-null children
    /// contained in the node.
    #[cfg_attr(not(test), expect(unused))]
    fn len(&self) -> usize;

    /// Whether this header has its frozen bit set.
    #[cfg_attr(not(test), expect(unused))]
    fn is_frozen(&self) -> bool;
}

#[cfg(test)]
pub(super) mod tests {
    /// Correctness properties that hold for sequential executions.
    pub(crate) mod sequential {
        use crate::raw::node::KeyIndex;
        use crate::raw::node::header::Header;
        use crate::raw::set::Set256;

        /// A successful `get` means `get_or_insert` returns the same index
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_implies_get_or_insert<H: Header>(header: H) {
            for key in u8::MIN..=u8::MAX {
                if let Some(index) = header.get(key) {
                    assert_eq!(header.get_or_insert(key), Some(index));
                }
            }
        }

        /// A successful `get_or_insert` means `get` returns the same index
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_or_insert_implies_get<H: Header>(header: H) {
            for key in u8::MIN..=u8::MAX {
                if let Some(index) = header.get_or_insert(key) {
                    assert_eq!(header.get(key), Some(index));
                }
            }
        }

        /// Consecutive `get_or_insert` calls return the same index
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_or_insert_idempotent<H: Header>(header: H, key: u8) {
            let index = header.get_or_insert(key);
            for _ in 0..5 {
                assert_eq!(header.get_or_insert(key), index);
            }
        }

        /// Every entry returned from `keys`:
        /// 1. Is consistent with `get` and `get_or_insert`
        /// 2. Has a unique key byte and index
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn keys_get_consistent<H: Header>(header: H, lower: u8, upper: u8) {
            let mut key_set = Set256::<core::sync::atomic::AtomicU64>::default();
            let mut index_set = Set256::<core::sync::atomic::AtomicU64>::default();

            let mut keys = H::KeyIter::default();
            header.keys(Some(lower), Some(upper), &mut keys);

            for KeyIndex { key, index } in keys {
                assert_eq!(header.get(key), Some(index));
                assert_eq!(header.get_or_insert(key), Some(index));

                assert!((lower..=upper).contains(&key));
                assert!(key_set.insert_mut(key));
                assert!(index_set.insert_mut(index));
            }
        }

        /// Freezing prevents insertion
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn freeze_no_insert<H: Header>(header: H) {
            header.freeze();

            for key in 0..=u8::MAX {
                match header.get(key) {
                    None => assert!(header.get_or_insert(key).is_none()),
                    Some(index) => assert_eq!(header.get_or_insert(key), Some(index)),
                }
            }
        }
    }

    /// Correctness properties that hold for concurrent executions.
    pub(crate) mod concurrent {
        use crate::raw::set::Set256;

        /// Concurrent calls to `get_or_insert` for the same key are idempotent.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_or_insert_same_idempotent<H>(header: H, key: u8)
        where
            H: crate::raw::node::header::Header,
        {
            crate::sync::check_dfs(None, move || {
                let header = header.clone();
                crate::sync::thread::scope(|scope| {
                    let index_a = scope.spawn(|| header.get_or_insert(key));
                    let index_b = header.get_or_insert(key);
                    let index_a = index_a.join().unwrap();
                    assert_eq!(index_a, index_b);
                });
            })
        }

        /// Concurrent calls to `get_or_insert` for different keys are consistent.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_or_insert_different_consistent<H>(header: H, key_a: u8, key_b: u8)
        where
            H: crate::raw::node::header::Header,
        {
            assert_ne!(key_a, key_b);

            let expect_a = header.get(key_a);
            let expect_b = header.get(key_b);
            let len = header.len();

            crate::sync::check_dfs(None, move || {
                let header = header.clone();
                crate::sync::thread::scope(|scope| {
                    let index_a = scope.spawn(|| header.get_or_insert(key_a));
                    let index_b = header.get_or_insert(key_b);
                    let index_a = index_a.join().unwrap();
                    match ((expect_a, index_a), (expect_b, index_b)) {
                        // Impossible
                        ((Some(_), None), _) | (_, (Some(_), None)) => {
                            unreachable!("get_or_insert must return existing mapping")
                        }
                        // Two insertions
                        ((None, Some(index_a)), (None, Some(index_b))) => {
                            assert!(
                                index_a as usize == len && index_b as usize == len + 1
                                    || index_a as usize == len + 1 && index_b as usize == len,
                                "Inconsistent insertions: {header:#x?}"
                            );
                        }
                        // Zero insertions because both keys are present
                        ((Some(expect_a), Some(index_a)), (Some(expect_b), Some(index_b))) => {
                            assert_eq!(expect_a, index_a);
                            assert_eq!(expect_b, index_b);
                        }
                        // One insertion, other may fail (None, None) or find key (Some, Some)
                        ((None, Some(inserted)), (expect, index))
                        | ((expect, index), (None, Some(inserted))) => {
                            assert_eq!(inserted as usize, len);
                            assert_eq!(expect, index);
                            if expect.is_none() {
                                assert!(header.is_frozen() || len + 1 == H::TYPE.capacity());
                            }
                        }
                        // Zero insertions because of at last one failure to insert
                        ((expect_a, index_a), (expect_b, index_b)) => {
                            assert_eq!(expect_a, index_a);
                            assert_eq!(expect_b, index_b);
                            assert!(header.is_frozen() || len == H::TYPE.capacity());
                        }
                    }
                });
            })
        }

        /// If concurrent calls to `get` and `get_or_insert` conflict:
        /// - If the key was present, then it is observed by both
        /// - Otherwise, `get` linearizes before or after the `get_or_insert`
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_get_or_insert_same_consistent<H>(header: H, key: u8)
        where
            H: crate::raw::node::header::Header,
        {
            let expect = header.get(key);
            let len = header.len();

            crate::sync::check_dfs(None, move || {
                let header = header.clone();
                crate::sync::thread::scope(|scope| {
                    let get = scope.spawn(|| header.get(key));
                    let get_or_insert = header.get_or_insert(key);
                    let get = get.join().unwrap();

                    match expect {
                        // Key was present in header
                        Some(expect) => {
                            assert_eq!(get, Some(expect));
                            assert_eq!(get_or_insert, Some(expect));
                        }
                        None => match (get, get_or_insert) {
                            // `get` linearized before `get_or_insert`
                            // `get_or_insert` appended a mapping
                            (None, Some(get_or_insert)) => {
                                assert_eq!(get_or_insert as usize, len);
                            }
                            // `get` linearized after `get_or_insert`
                            (Some(get), Some(get_or_insert)) => {
                                assert_eq!(get, get_or_insert);
                            }
                            // Header is frozen, or cannot hold more mappings
                            (None, None) => {
                                assert!(header.is_frozen() || len == H::TYPE.capacity());
                            }
                            (Some(get), None) => {
                                unreachable!("Get observed non-existent index {get:?}")
                            }
                        },
                    }
                });
            })
        }

        /// If a call to `get_or_insert` overlaps with `keys`:
        /// - If the key was present, then it is observed by both
        /// - Otherwise, `get_or_insert` linearizes before or after `keys`
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_or_insert_keys_consistent<H>(header: H, key: u8)
        where
            H: crate::raw::node::header::Header,
        {
            let expect = header.get(key);
            let len = header.len();
            let mut expected_key_set = Set256::<core::sync::atomic::AtomicU64>::default();
            let mut iter = H::KeyIter::default();
            header.keys(None, None, &mut iter);
            for entry in iter {
                assert!(expected_key_set.insert_mut(entry.key));
            }

            crate::sync::check_pct(1_000, 8, move || {
                let header = header.clone();
                crate::sync::thread::scope(|scope| {
                    let get_or_insert = scope.spawn(|| header.get_or_insert(key));
                    let mut iter = H::KeyIter::default();
                    header.keys(None, None, &mut iter);
                    let get_or_insert = get_or_insert.join().unwrap();

                    let mut actual_key_set = Set256::<core::sync::atomic::AtomicU64>::default();
                    let mut actual_index = None;
                    for entry in iter {
                        assert!(actual_key_set.insert_mut(entry.key));
                        if entry.key == key {
                            actual_index = Some(entry.index);
                        }
                    }

                    match (expect, get_or_insert) {
                        (Some(expect), Some(get_or_insert)) => {
                            assert_eq!(get_or_insert, expect);
                            assert_eq!(actual_key_set.len(), len);
                        }
                        (None, None) => {
                            assert!(header.is_frozen() || len == H::TYPE.capacity());
                            assert_eq!(actual_key_set.len(), len);
                        }
                        (Some(_), None) => {
                            unreachable!("get_or_insert must return existing mapping")
                        }
                        (None, Some(index)) => {
                            assert_eq!(index as usize, len);
                            if let Some(actual) = actual_index {
                                assert_eq!(actual, index);
                                actual_key_set.remove_mut(key);
                            }
                            assert_eq!(actual_key_set, expected_key_set);
                        }
                    }
                })
            })
        }
    }

    macro_rules! impl_suite {
        ($strategy:expr) => {
            #[cfg(feature = "proptest")]
            mod sequential {
                #[allow(unused)]
                use proptest::strategy::Strategy as _;

                use crate::raw::node::header::tests::sequential;
                use crate::raw::node::iter::bound;

                proptest::proptest! {
                    #![proptest_config(proptest::test_runner::Config::with_cases(100_000))]

                    #[test]
                    fn get_implies_get_or_insert(header in $strategy) {
                        sequential::get_implies_get_or_insert(header)
                    }

                    #[test]
                    fn get_or_insert_implies_get(header in $strategy) {
                        sequential::get_or_insert_implies_get(header)
                    }

                    #[test]
                    fn get_or_insert_idempotent(header in $strategy, key: u8) {
                        sequential::get_or_insert_idempotent(header, key)
                    }

                    #[test]
                    fn keys_get_consistent(header in $strategy, (lower, upper) in bound()) {
                        sequential::keys_get_consistent(header, lower, upper)
                    }

                    #[test]
                    fn freeze_no_insert(header in $strategy) {
                        sequential::freeze_no_insert(header)
                    }
                }
            }

            #[cfg(feature = "proptest")]
            mod concurrent {
                #[allow(unused)]
                use proptest::strategy::Strategy as _;

                use crate::raw::node::header::tests::concurrent;

                proptest::proptest! {
                    #![proptest_config(proptest::test_runner::Config::with_cases(1_000))]
                    #[test]
                    fn get_or_insert_same_idempotent(header in $strategy, key: u8) {
                        concurrent::get_or_insert_same_idempotent(header, key)
                    }

                    #[test]
                    fn get_or_insert_different_consistent(header in $strategy, key_a: u8, key_b: u8) {
                        proptest::prop_assume!(key_a != key_b, "Keys must be different");
                        concurrent::get_or_insert_different_consistent(header, key_a, key_b)
                    }

                    #[test]
                    fn get_get_or_insert_same_consistent(header in $strategy, key: u8) {
                        concurrent::get_get_or_insert_same_consistent(header, key)
                    }

                    #[test]
                    fn get_or_insert_keys_consistent(header in $strategy, key: u8) {
                        concurrent::get_or_insert_keys_consistent(header, key)
                    }
                }
            }
        };
    }
    pub(crate) use impl_suite;
}
