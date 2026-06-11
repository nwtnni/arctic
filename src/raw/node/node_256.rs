//! [`Node256`] provides a **total** mapping of 256 key-edge pairs.
//!
//! This is the simplest node representation, and requires the most memory.
//! It is not linear because it has no header metadata at all.

use core::fmt::Debug;

use crate::Atomic;
use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::KeyIter256;
use crate::raw::node::Node;

/// [`Node`] representation that contains exactly 256 key-edge pairs.
#[repr(C, align(4096))]
pub(crate) struct Node256([Atomic<edge::Raw>; 256]);

const_assert_size_align!(Node256, 4096, 4096);

impl Default for Node256 {
    fn default() -> Self {
        Self(core::array::from_fn(|_| {
            Atomic::new_packed(edge::Raw::NULL)
        }))
    }
}

unsafe impl Node for Node256 {
    const TYPE: node::Type = node::Type::Node256;
    const CAPACITY: usize = 256;
    type KeyIter = KeyIter256;

    unsafe fn new_unchecked(keys: &[u8], edges: &[ribbit::Packed<edge::Raw>]) -> Box<Self> {
        if_validate!(crate::assert_unique(keys));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= Self::CAPACITY);

        let mut node = Box::new(Self::default());
        for (key, edge) in keys.iter().zip(edges) {
            node.0[*key as usize].set_packed(*edge);
        }
        node
    }

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
    fn edges(&self) -> &[Atomic<edge::Raw>] {
        &self.0
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<edge::Raw>] {
        &mut self.0
    }

    #[inline]
    fn get_key(&self, key: u8) -> Option<u8> {
        Some(key)
    }

    #[inline]
    fn get_or_insert_key(&self, key: u8) -> Option<u8> {
        Some(key)
    }

    #[inline]
    fn freeze_header(&self) -> usize {
        Self::CAPACITY
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
}

impl Debug for Node256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node256").field("edges", &self.0).finish()
    }
}
