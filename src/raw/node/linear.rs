//! A linear node is a node whose header can be atomically updated.
//!
//! Primarily an implementation detail to factor out some common
//! logic between [`crate::raw::node::Node3`] and [`crate::raw::node::Node15`].

use core::fmt::Debug;
use core::sync::atomic::Ordering;

use crate::Atomic;
use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::KeyIndex;
use crate::raw::node::Node;

#[repr(C, align(64))]
pub(crate) struct Linear<const LEN: usize, H: ribbit::Pack> {
    pub(super) header: Atomic<H>,
    pub(super) edges: [Atomic<edge::Raw>; LEN],
}

impl<const LEN: usize, H> Default for Linear<LEN, H>
where
    H: ribbit::Pack<Packed: Default>,
{
    fn default() -> Self {
        Self {
            header: Atomic::new_packed(H::Packed::default()),
            edges: core::array::from_fn(|_| Atomic::new_packed(edge::Raw::NULL)),
        }
    }
}

unsafe impl<const LEN: usize, H> Node for Linear<LEN, H>
where
    H: ribbit::Pack<Packed: Header + Default>,
{
    const TYPE: node::Type = <H::Packed as Header>::TYPE;
    const CAPACITY: usize = <H::Packed as Header>::CAPACITY;
    type KeyIter = <H::Packed as Header>::KeyIter;

    unsafe fn new_unchecked(keys: &[u8], edges: &[ribbit::Packed<edge::Raw>]) -> Box<Self> {
        if_validate!(crate::assert_unique(keys));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= Self::CAPACITY);

        let mut node = Box::new(Self::default());
        let header = unsafe { <ribbit::Packed<H> as Header>::new_unchecked(keys) };

        node.header.set_packed(header);

        for (out, r#in) in node.edges.iter_mut().zip(edges) {
            out.set_packed(*r#in);
        }

        node
    }

    #[inline]
    fn keys<L: node::Lower, U: node::Upper>(&self, lower: L, upper: U, iter: &mut Self::KeyIter) {
        self.header
            .load_packed(Ordering::Relaxed)
            .keys(lower, upper, iter)
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
        self.header.load_packed(Ordering::Relaxed).get(key)
    }

    #[inline]
    fn get_or_insert_key(&self, key: u8) -> Option<u8> {
        let mut old = self.header.load_packed(Ordering::Relaxed);

        loop {
            let new = match old.get_or_insert(key) {
                Ok(index) => return Some(index),
                Err(None) => return None,
                Err(Some(new)) => new,
            };

            match self.header.compare_exchange_packed(
                old,
                new,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break Some(old.len()),
                Err(conflict) => old = conflict,
            }
        }
    }

    #[inline]
    fn freeze_header(&self) -> usize {
        let mut header = self.header.load_packed(Ordering::Relaxed);

        while !header.is_frozen() {
            match self.header.compare_exchange_packed(
                header,
                header.freeze(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => header = conflict,
            }
        }

        header.len() as usize
    }

    #[inline]
    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex> {
        self.header.load_packed(Ordering::Relaxed).min(lower)
    }

    #[inline]
    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex> {
        self.header.load_packed(Ordering::Relaxed).max(upper)
    }
}

impl<const LEN: usize, H> Debug for Linear<LEN, H>
where
    H: ribbit::Pack<Packed: Debug>,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = const {
            if LEN == 3 {
                "Node3"
            } else if LEN == 15 {
                "Node15"
            } else {
                unreachable!()
            }
        };

        f.debug_struct(name)
            .field("header", &self.header)
            .field("edges", &self.edges)
            .finish()
    }
}

pub(super) trait Header: ribbit::Unpack + core::fmt::Debug {
    const TYPE: node::Type;
    const CAPACITY: usize;
    type KeyIter: Default + Iterator<Item = KeyIndex> + core::fmt::Debug;

    unsafe fn new_unchecked(keys: &[u8]) -> Self;

    fn freeze(self) -> Self;

    fn is_frozen(self) -> bool;

    fn len(self) -> u8;

    fn get(self, key: u8) -> Option<u8>;

    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>>;

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
        iter: &mut Self::KeyIter,
    );

    fn min<L: node::Lower>(self, lower: L) -> Option<node::KeyIndex>;
    fn max<U: node::Upper>(self, upper: U) -> Option<node::KeyIndex>;
}
