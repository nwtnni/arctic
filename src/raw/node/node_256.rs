//! [`Node256`] provides a **total** mapping of 256 key-edge pairs.
//!
//! This is the simplest node representation, and requires the most memory.
//! It is not linear because it has no header metadata at all.

use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Edge;
use crate::raw::node::Node;
use crate::raw::node::iter;
use crate::raw::node::iter::KeyIndex;

/// [`Node`] representation that contains exactly 256 key-edge pairs.
#[repr(C, align(4096))]
pub(crate) struct Node256<M: ribbit::Pack>([Atomic<Edge<M>>; 256]);

const_assert_size_align!(Node256::<()>, 4096, 4096);

impl<M> Default for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn default() -> Self {
        Self(core::array::from_fn(|_| Atomic::new_packed(Edge::NULL)))
    }
}

unsafe impl<M> Node<M> for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const TYPE: node::Type = node::Type::Node256;
    const CAPACITY: usize = 256;
    type KeyIter = KeyIter256;

    unsafe fn new_unchecked(keys: &[u8], edges: &[ribbit::Packed<Edge<M>>]) -> Box<Self> {
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
    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.0
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<Edge<M>>] {
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
    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex> {
        self.0
            .iter()
            .enumerate()
            .skip(lower.get() as usize)
            .find_map(|(index, edge)| {
                if edge.load_packed(Ordering::Relaxed).is_null() {
                    return None;
                } else {
                    Some(iter::KeyIndex {
                        index: index as u8,
                        key: index as u8,
                    })
                }
            })
    }

    #[inline]
    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex> {
        self.0
            .iter()
            .enumerate()
            .rev()
            .skip(upper.get() as usize)
            .find_map(|(index, edge)| {
                if edge.load_packed(Ordering::Relaxed).is_null() {
                    return None;
                } else {
                    Some(iter::KeyIndex {
                        index: index as u8,
                        key: index as u8,
                    })
                }
            })
    }
}

impl<M> Debug for Node256<M>
where
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node256").field("edges", &self.0).finish()
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, Debug)]
pub(crate) struct KeyIter256 {
    head: u16,
    tail: u16,
}

impl KeyIter256 {
    #[inline]
    fn new<L: node::iter::Lower, U: node::iter::Upper>(lower: L, upper: U) -> Self {
        Self {
            head: lower.get() as u16,
            tail: upper.get() as u16 + 1,
        }
    }
}

impl Iterator for KeyIter256 {
    type Item = KeyIndex;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        let next = self.head as u8;
        self.head += 1;
        Some(KeyIndex {
            key: next,
            index: next,
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (self.tail - self.head) as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for KeyIter256 {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

impl DoubleEndedIterator for KeyIter256 {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.head == self.tail {
            return None;
        }

        self.tail -= 1;
        Some(KeyIndex {
            key: self.tail as u8,
            index: self.tail as u8,
        })
    }
}

impl From<KeyIter256> for node::KeyIter {
    #[inline]
    fn from(iter: KeyIter256) -> Self {
        node::KeyIter::new_256(iter)
    }
}
