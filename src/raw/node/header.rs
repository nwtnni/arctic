//! Defines a common trait [`Header`] for node types with header metadata.
//!
//! Also contains a shared implementation of the [`crate::raw::node::Node`]
//! for node types with headers.

use core::fmt::Debug;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::KeyIndex;
use crate::raw::node::Node;

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

    unsafe fn new_unchecked<const CAPACITY: usize>(
        keys: &[u8],
        edges: &[ribbit::Packed<edge::Raw>],
    ) -> Box<Node<CAPACITY, Self>> {
        if_validate!(assert!(crate::raw::is_unique(keys)));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= CAPACITY);

        let mut node = Box::new(Node::default());
        unsafe { Self::initialize_unchecked(&mut node.header, keys) };

        for (out, r#in) in node.edges.iter_mut().zip(edges) {
            *out.get_mut_packed() = *r#in;
        }

        node
    }

    unsafe fn initialize_unchecked(&mut self, keys: &[u8]);

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
        /// Concurrent calls to `get_or_insert` return the same index
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_or_insert_consistent<H>(header: H, key: u8)
        where
            H: crate::raw::node::header::Header,
        {
            crate::sync::check_dfs(1, move || {
                let header = header.clone();
                crate::sync::thread::scope(|scope| {
                    let a = scope.spawn(|| header.get_or_insert(key));
                    let b = header.get_or_insert(key);
                    let a = a.join().unwrap();
                    assert_eq!(a, b);
                });
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
                    fn get_or_insert_concurrent_consistent(header in $strategy, key: u8) {
                        concurrent::get_or_insert_consistent(header, key)
                    }
                }
            }
        };
    }
    pub(crate) use impl_suite;
}
