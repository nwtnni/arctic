//! Defines a common trait [`Header`] for node types with header metadata.
//!
//! Also contains a shared implementation of the [`crate::raw::node::Node`]
//! for node types with headers.

use core::fmt::Debug;

use crate::Atomic;
use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::KeyIndex;

#[repr(C, align(64))]
pub(crate) struct Node<const LEN: usize, H> {
    pub(super) header: H,
    pub(super) edges: [Atomic<edge::Raw>; LEN],
}

impl<const LEN: usize, H> Default for Node<LEN, H>
where
    H: Default,
{
    fn default() -> Self {
        Self {
            header: H::default(),
            edges: core::array::from_fn(|_| Atomic::new_packed(edge::Raw::NULL)),
        }
    }
}

unsafe impl<const LEN: usize, H> node::Node for Node<LEN, H>
where
    H: Default + Header,
{
    const TYPE: node::Type = <H as Header>::TYPE;
    const CAPACITY: usize = LEN;
    type KeyIter = <H as Header>::KeyIter;

    unsafe fn new_unchecked(keys: &[u8], edges: &[ribbit::Packed<edge::Raw>]) -> Box<Self> {
        if_validate!(crate::assert_unique(keys));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= Self::CAPACITY);

        let mut node = Box::new(Self::default());
        unsafe { <H as Header>::initialize_unchecked(&mut node.header, keys) };

        for (out, r#in) in node.edges.iter_mut().zip(edges) {
            out.set_packed(*r#in);
        }

        node
    }

    #[inline]
    fn keys<L: node::Lower, U: node::Upper>(&self, lower: L, upper: U, iter: &mut Self::KeyIter) {
        self.header.keys(lower, upper, iter)
    }

    #[inline]
    fn edges(&self) -> &[Atomic<edge::Raw>] {
        &self.edges
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<edge::Raw>] {
        &mut self.edges
    }

    #[inline]
    fn get_key(&self, key: u8) -> Option<u8> {
        self.header.get(key)
    }

    #[inline]
    fn get_or_insert_key(&self, key: u8) -> Option<u8> {
        self.header.get_or_insert(key)
    }

    #[inline]
    fn freeze_header(&self) -> usize {
        self.header.freeze()
    }

    #[inline]
    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex> {
        self.header.min(lower)
    }

    #[inline]
    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex> {
        self.header.max(upper)
    }
}

impl<const LEN: usize, H> Debug for Node<LEN, H>
where
    H: Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = const {
            if LEN == 3 {
                "Node3"
            } else if LEN == 15 {
                "Node15"
            } else {
                assert!(LEN == 47);
                "Node47"
            }
        };

        f.debug_struct(name)
            .field("header", &self.header)
            .field("edges", &self.edges)
            .finish()
    }
}

pub(super) trait Header: Debug + Sized {
    const TYPE: node::Type;
    type KeyIter: Default + Iterator<Item = KeyIndex> + core::fmt::Debug;

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
