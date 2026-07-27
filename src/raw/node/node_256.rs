//! [`Node256`] provides a **total** mapping of 256 key-edge pairs.
//!
//! This is the simplest node representation, and requires the most memory.
//! It is not linear because it has no header metadata at all.

use core::fmt::Debug;
use core::ops::Deref;

use crate::raw::node;
use crate::raw::node::KeyIter256;

const CAPACITY: usize = 256;

/// [`Node`] representation that contains exactly 256 key-edge pairs.
#[repr(C, align(4096))]
#[derive(Default)]
pub(super) struct Node256(node::Node<CAPACITY, Header>);

const_assert_size_align!(Node256, 4096, 4096);

impl Node256 {
    pub(super) unsafe fn new_unchecked(
        keys: &[u8],
        edges: &[ribbit::Packed<crate::raw::edge::Raw>],
    ) -> Box<Self> {
        validate!(crate::raw::is_unique(keys));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= CAPACITY);

        let mut node = Box::new(Self::default());
        for (index, edge) in core::iter::zip(keys, edges) {
            *node.0.edges[*index as usize].get_mut_packed() = *edge;
        }

        node
    }
}

impl Deref for Node256 {
    type Target = node::Node<CAPACITY, Header>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Header;

unsafe impl node::Header for Header {
    const TYPE: node::Type = node::Type::Node256;
    type KeyIter = KeyIter256;

    #[inline]
    fn keys<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
        iter: &mut Self::KeyIter,
    ) {
        *iter = KeyIter256::new(lower, upper)
    }

    #[inline]
    fn get(&self, key: u8) -> Option<u8> {
        Some(key)
    }

    #[inline]
    fn get_or_insert(&self, key: u8) -> Option<u8> {
        Some(key)
    }

    #[inline]
    fn freeze(&self) -> usize {
        CAPACITY
    }

    #[inline]
    fn min<L: node::Lower>(&self, _lower: L) -> Option<node::KeyIndex> {
        todo!()
        // self.0
        //     .iter()
        //     .enumerate()
        //     .skip(lower.get() as usize)
        //     .find_map(|(index, edge)| {
        //         if edge.load_packed(Ordering::Relaxed).is_null() {
        //             return None;
        //         } else {
        //             Some(iter::KeyIndex {
        //                 index: index as u8,
        //                 key: index as u8,
        //             })
        //         }
        //     })
    }

    #[inline]
    fn max<U: node::Upper>(&self, _upper: U) -> Option<node::KeyIndex> {
        todo!()
        // self.0
        //     .iter()
        //     .enumerate()
        //     .rev()
        //     .skip(upper.get() as usize)
        //     .find_map(|(index, edge)| {
        //         if edge.load_packed(Ordering::Relaxed).is_null() {
        //             return None;
        //         } else {
        //             Some(iter::KeyIndex {
        //                 index: index as u8,
        //                 key: index as u8,
        //             })
        //         }
        //     })
    }

    #[inline]
    fn len(&self) -> usize {
        CAPACITY
    }

    #[inline]
    fn is_frozen(&self) -> bool {
        false
    }
}
