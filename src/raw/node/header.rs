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
pub(in crate::raw) unsafe trait Header: Debug + Default + Sized {
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
    #![cfg_attr(not(feature = "proptest"), expect(unused))]

    /// A successful `get` means `get_or_insert` returns the same index
    pub(crate) fn get_implies_get_or_insert<H>(header: H)
    where
        H: crate::raw::node::header::Header,
    {
        for key in u8::MIN..=u8::MAX {
            if let Some(index) = header.get(key) {
                assert_eq!(header.get_or_insert(key), Some(index));
            }
        }
    }

    /// A successful `get_or_insert` means `get` returns the same index
    pub(crate) fn get_or_insert_implies_get<H>(header: H)
    where
        H: crate::raw::node::header::Header,
    {
        for key in u8::MIN..=u8::MAX {
            if let Some(index) = header.get_or_insert(key) {
                assert_eq!(header.get(key), Some(index));
            }
        }
    }

    /// Consecutive `get_or_insert` calls return the same index
    pub(crate) fn get_or_insert_idempotent<H>(header: H, key: u8)
    where
        H: crate::raw::node::header::Header,
    {
        let index = header.get_or_insert(key);
        for _ in 0..5 {
            assert_eq!(header.get_or_insert(key), index);
        }
    }

    /// Every key returned from `keys` is visible to `get` and `get_or_insert`
    pub(crate) fn keys_get_consistent<H>(header: H, lower: u8, upper: u8)
    where
        H: crate::raw::node::header::Header,
    {
        let mut keys = H::KeyIter::default();
        header.keys(Some(lower), Some(upper), &mut keys);
        for KeyIndex { key, index } in keys {
            assert_eq!(header.get(key), Some(index));
            assert_eq!(header.get_or_insert(key), Some(index));
        }
    }

    /// Freezing prevents insertion
    pub(crate) fn freeze_no_insert<H>(header: H)
    where
        H: crate::raw::node::header::Header,
    {
        header.freeze();

        for key in 0..=u8::MAX {
            match header.get(key) {
                None => assert!(header.get_or_insert(key).is_none()),
                Some(index) => assert_eq!(header.get_or_insert(key), Some(index)),
            }
        }
    }

    // Guarantees lower >= upper
    #[cfg(feature = "proptest")]
    proptest::prop_compose! {
        pub(crate) fn bound()
        (lower in u8::MIN..=u8::MAX)
        (lower in proptest::strategy::Just(lower), upper in lower..=u8::MAX) -> (u8, u8) {
            (lower, upper)
        }
    }

    macro_rules! impl_suite {
        ($strategy:expr) => {
            #[cfg(feature = "proptest")]
            proptest::proptest! {
                #![proptest_config(proptest::test_runner::Config::with_cases(100_000))]

                #[test]
                fn get_implies_get_or_insert(header in $strategy) {
                    crate::raw::node::header::tests::get_implies_get_or_insert(header)
                }

                #[test]
                fn get_or_insert_implies_get(header in $strategy) {
                    crate::raw::node::header::tests::get_or_insert_implies_get(header)
                }

                #[test]
                fn get_or_insert_idempotent(header in $strategy, key: u8) {
                    crate::raw::node::header::tests::get_or_insert_idempotent(header, key)
                }

                #[test]
                fn keys_get_consistent(header in $strategy, (lower, upper) in crate::raw::node::header::tests::bound()) {
                    crate::raw::node::header::tests::keys_get_consistent(header, lower, upper)
                }

                #[test]
                fn freeze_no_insert(header in $strategy) {
                    crate::raw::node::header::tests::freeze_no_insert(header)
                }
            }
        };
    }
    pub(crate) use impl_suite;

    use crate::raw::node::KeyIndex;
}
