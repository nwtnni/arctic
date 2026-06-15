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
            out.set_packed(*r#in);
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
    /// A `get_or_insert` on an empty header succeeds and is consistent with `get`
    #[cfg_attr(not(feature = "proptest"), expect(unused))]
    pub(in crate::raw::node) fn get_or_insert_empty<H>(key: u8)
    where
        H: crate::raw::node::header::Header,
    {
        let header = H::default();
        let get_or_insert = header.get_or_insert(key).expect("get_or_insert empty");
        let get = header.get(key).expect("get after get_or_insert");
        assert_eq!(get, get_or_insert)
    }

    macro_rules! impl_suite {
        ($type:ty) => {
            #[cfg(feature = "proptest")]
            proptest::proptest! {
                #[test]
                fn get_or_insert_empty(key: u8) {
                    crate::raw::node::header::tests::get_or_insert_empty::<$type>(key)
                }
            }
        };
    }
    pub(in crate::raw::node) use impl_suite;
}
