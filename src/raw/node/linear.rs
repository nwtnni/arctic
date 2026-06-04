//! A linear node is a node whose header can be atomically updated.
//!
//! Primarily an implementation detail to factor out some common
//! logic between [`crate::raw::node::Node3`] and [`crate::raw::node::Node15`].

use core::fmt::Debug;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::Edge;
use crate::raw::node::Node;

#[repr(C, align(64))]
pub(crate) struct Linear<const LEN: usize, H: ribbit::Pack, M: ribbit::Pack>
where
    <H::Packed as ribbit::Unpack>::Loose: ribbit::atomic::Loose,
{
    pub(super) header: Atomic<H>,
    pub(super) edges: [Atomic<Edge<M>>; LEN],
}

impl<const LEN: usize, H, M> Default for Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Default>,
    <H::Packed as ribbit::Unpack>::Loose: ribbit::atomic::Loose,
    M: ribbit::Pack<Packed: edge::Meta>,
{
    fn default() -> Self {
        Self {
            header: Atomic::new_packed(H::Packed::default()),
            edges: core::array::from_fn(|_| Atomic::new_packed(Edge::NULL)),
        }
    }
}

unsafe impl<const LEN: usize, H, M> Node<M> for Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Header + Default>,
    <H::Packed as ribbit::Unpack>::Loose: ribbit::atomic::Loose,
    M: ribbit::Pack<Packed: edge::Meta>,
{
    const TYPE: node::Type = <H::Packed as Header>::TYPE;
    const CAPACITY: usize = <H::Packed as Header>::CAPACITY;
    type KeyIter = <H::Packed as Header>::KeyIter;

    unsafe fn new_unchecked(keys: &[u8], edges: &[ribbit::Packed<Edge<M>>]) -> Box<Self> {
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
    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        &self,
        lower: L,
        upper: U,
    ) -> Self::KeyIter {
        self.header
            .load_packed(Ordering::Relaxed)
            .keys(lower, upper)
    }

    #[inline]
    fn edges(&self) -> &[Atomic<Edge<M>>] {
        &self.edges
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<Edge<M>>] {
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
}

impl<const LEN: usize, H, M> Debug for Linear<LEN, H, M>
where
    H: ribbit::Pack<Packed: Debug>,
    <H::Packed as ribbit::Unpack>::Loose: ribbit::atomic::Loose,
    M: ribbit::Pack<Packed: edge::Meta + Debug>,
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
    type KeyIter: Iterator<Item = node::iter::KeyIndex> + Into<node::KeyIter>;

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
    ) -> Self::KeyIter;
}

/// NOTE: We order `head` and `tail` fields at the end
/// to allow `entries` to be filled in with a single
/// aligned SIMD write.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) struct KeyIter<const N: usize> {
    pub(super) entries: [node::iter::KeyIndex; N],
    pub(super) head: u8,
    pub(super) tail: u8,
}

impl<const N: usize> Default for KeyIter<N> {
    fn default() -> Self {
        Self {
            entries: [node::iter::KeyIndex { key: 0, index: 0 }; N],
            head: 0,
            tail: 0,
        }
    }
}

#[repr(C, align(8))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct KeyIter3(pub(super) KeyIter<3>);

#[repr(C, align(32))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct KeyIter15(pub(super) KeyIter<15>);

#[repr(C, align(32))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct KeyIter63(pub(super) KeyIter<63>);

const_assert_size_align!(KeyIter3, 8, 8);
const_assert_size_align!(KeyIter15, 32, 32);
const_assert_size_align!(KeyIter63, 128, 32);

macro_rules! impl_key_iter {
    ($ty:ty, $len:expr $(, $new:ident)?) => {
        $(
            impl $ty {
                #[inline]
                pub(super) const fn $new(entries: [node::iter::KeyIndex; $len], len: u8) -> Self {
                    validate!(len as usize <= entries.len());
                    Self(KeyIter {
                        head: 0,
                        tail: len,
                        entries,
                    })
                }
            }
        )?

        impl Iterator for $ty {
            type Item = node::iter::KeyIndex;
            #[inline]
            fn next(&mut self) -> Option<Self::Item> {
                if self.0.head == self.0.tail {
                    return None;
                }

                let next = self.0.entries.get(self.0.head as usize).copied()?;
                self.0.head += 1;
                Some(next)
            }

            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                let len = (self.0.tail - self.0.head) as usize;
                (len, Some(len))
            }
        }

        impl DoubleEndedIterator for $ty {
            #[inline]
            fn next_back(&mut self) -> Option<Self::Item> {
                if self.0.head == self.0.tail {
                    return None;
                }

                self.0.tail -= 1;
                self.0.entries.get(self.0.tail as usize).copied()
            }
        }

        impl ExactSizeIterator for $ty {
            #[inline]
            fn len(&self) -> usize {
                let (lower, upper) = self.size_hint();
                validate_eq!(upper, Some(lower));
                lower
            }
        }
    };
}

impl_key_iter!(KeyIter3, 3, new_3);
impl_key_iter!(KeyIter15, 15);
impl_key_iter!(KeyIter63, 63);
