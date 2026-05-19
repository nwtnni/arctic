use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::ptr::NonNull;

use ribbit::Atomic;
use ribbit::Pack as _;
use ribbit::u2;

use crate::raw::Edge;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::node_256;

pub(crate) struct NodeIter<'g, M: ribbit::Pack> {
    keys: KeyIter,
    edges: NonNull<Atomic<Edge<M>>>,

    #[cfg(feature = "validate")]
    len: u16,

    _slice: PhantomData<&'g [Atomic<Edge<M>>]>,
}

impl<'g, M: ribbit::Pack> NodeIter<'g, M> {
    /// # SAFETY
    ///
    /// Caller must guarantee all indices produced by `keys` are < `edges.len()`.
    #[inline]
    pub(crate) unsafe fn new(keys: KeyIter, edges: &'g [Atomic<Edge<M>>]) -> Self {
        Self {
            keys,
            edges: NonNull::from(edges).cast(),

            #[cfg(feature = "validate")]
            len: edges.len() as u16,

            _slice: PhantomData,
        }
    }
}

impl<'g, M: ribbit::Pack> NodeIter<'g, M> {
    #[inline]
    pub(crate) fn try_into_single(mut self) -> Result<(u8, NonNull<Atomic<Edge<M>>>), Self> {
        if self.size_hint().0 == 1 {
            Ok(self.next().expect("Size hint is exact"))
        } else {
            Err(self)
        }
    }
}

impl<'g, M: ribbit::Pack> Iterator for NodeIter<'g, M> {
    type Item = (u8, NonNull<Atomic<Edge<M>>>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let KeyIndex { key, index } = self.keys.next()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.len,
            "index is {} but len is {}",
            index,
            self.len,
        );

        let edge = unsafe { self.edges.add(index as usize) };
        Some((key, edge))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.keys.size_hint()
    }
}

impl<'g, M: ribbit::Pack> DoubleEndedIterator for NodeIter<'g, M> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        let KeyIndex { key, index } = self.keys.next_back()?;

        #[cfg(feature = "validate")]
        validate!(
            (index as u16) < self.len,
            "index is {} but len is {}",
            index,
            self.len,
        );

        let edge = unsafe { self.edges.add(index as usize) };
        Some((key, edge))
    }
}

impl<'g, M: ribbit::Pack> ExactSizeIterator for NodeIter<'g, M> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

#[repr(C)]
pub(crate) union KeyIter {
    node_3: linear::KeyIter3,
    node_15: NonNull<linear::KeyIter<15>>,
    node_47: NonNull<linear::KeyIter<63>>,
    node_256: node_256::KeyIter,
    raw: u64,
}

#[repr(C)]
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) struct KeyIndex {
    pub(super) index: u8,
    pub(super) key: u8,
}

impl KeyIndex {
    pub(crate) const DEFAULT: Self = Self { key: 0, index: 0 };
}

impl PartialOrd for KeyIndex {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for KeyIndex {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key).then(self.index.cmp(&other.index))
    }
}

impl Debug for KeyIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#.02X}:{:#.02X}", self.key, self.index)
    }
}

impl KeyIter {
    pub(crate) const ROOT: Self = Self {
        node_3: linear::KeyIter3::new_3([KeyIndex::DEFAULT; 3], 1),
    };

    const TAG_15: usize = (node::Type::Node15 as usize) << 62;
    const TAG_47: usize = (node::Type::Node47 as usize) << 62;

    #[inline]
    fn r#type(&self) -> ribbit::Packed<node::Type> {
        unsafe {
            // SAFETY: shifting u64 by 62 bits, so only 2 bits can remain
            ribbit::Packed::<node::Type>::new_unchecked(u2::new_unchecked((self.raw >> 62) as u8))
        }
    }

    #[inline]
    pub(super) fn new_3(node_3: linear::KeyIter3) -> Self {
        let iter = Self { node_3 };
        validate_eq!(iter.r#type(), node::Type::Node3.pack());
        iter
    }

    #[inline]
    pub(super) fn new_15(node_15: Box<linear::KeyIter<15>>) -> Self {
        let iter = Self {
            node_15: NonNull::from(Box::leak(node_15))
                .map_addr(|addr| unsafe { NonZeroUsize::new_unchecked(addr.get() | Self::TAG_15) }),
        };
        validate_eq!(iter.r#type(), node::Type::Node15.pack());
        iter
    }

    #[inline]
    pub(super) fn new_47(node_47: Box<linear::KeyIter<63>>) -> Self {
        let iter = Self {
            node_47: NonNull::from(Box::leak(node_47))
                .map_addr(|addr| unsafe { NonZeroUsize::new_unchecked(addr.get() | Self::TAG_47) }),
        };
        validate_eq!(iter.r#type(), node::Type::Node47.pack());
        iter
    }

    #[inline]
    pub(super) fn new_256(node_256: node_256::KeyIter) -> Self {
        let iter = Self { node_256 };
        validate_eq!(iter.r#type(), node::Type::Node256.pack());
        iter
    }

    #[inline]
    unsafe fn as_node_15_unchecked(&self) -> NonNull<linear::KeyIter<15>> {
        validate_eq!(self.r#type(), node::Type::Node15.pack());
        unsafe {
            self.node_15.map_addr(|addr| {
                validate_eq!(addr.get() & Self::TAG_15, Self::TAG_15);
                NonZeroUsize::new_unchecked(addr.get() ^ Self::TAG_15)
            })
        }
    }

    #[inline]
    unsafe fn as_node_47_unchecked(&self) -> NonNull<linear::KeyIter<63>> {
        validate_eq!(self.r#type(), node::Type::Node47.pack());
        unsafe {
            self.node_47.map_addr(|addr| {
                validate_eq!(addr.get() & Self::TAG_47, Self::TAG_47);
                NonZeroUsize::new_unchecked(addr.get() ^ Self::TAG_47)
            })
        }
    }
}

impl Iterator for KeyIter {
    type Item = KeyIndex;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        node::dispatch!(
            self.r#type(),
            unsafe { &mut self.node_3 }.next(),
            unsafe { self.as_node_15_unchecked().as_mut() }.next(),
            unsafe { self.as_node_47_unchecked().as_mut() }.next(),
            unsafe { &mut self.node_256 }
                .next()
                .map(|key| KeyIndex { key, index: key }),
        )
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        node::dispatch!(
            self.r#type(),
            unsafe { &self.node_3 }.size_hint(),
            unsafe { self.as_node_15_unchecked().as_ref() }.size_hint(),
            unsafe { self.as_node_47_unchecked().as_ref() }.size_hint(),
            unsafe { &self.node_256 }.size_hint(),
        )
    }
}

impl DoubleEndedIterator for KeyIter {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        node::dispatch!(
            self.r#type(),
            unsafe { &mut self.node_3 }.next_back(),
            unsafe { self.as_node_15_unchecked().as_mut() }.next_back(),
            unsafe { self.as_node_47_unchecked().as_mut() }.next_back(),
            unsafe { &mut self.node_256 }
                .next_back()
                .map(|key| KeyIndex { key, index: key }),
        )
    }
}

impl ExactSizeIterator for KeyIter {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

impl Drop for KeyIter {
    fn drop(&mut self) {
        node::dispatch!(
            self.r#type(),
            (),
            drop(unsafe { Box::from_raw(self.as_node_15_unchecked().as_ptr()) }),
            drop(unsafe { Box::from_raw(self.as_node_47_unchecked().as_ptr()) }),
            (),
        )
    }
}

pub(crate) trait Lower: Copy + Default + Debug {
    const UNBOUND: bool = false;
    fn get(self) -> u8;
    fn check(self, byte: u8) -> bool;
}

pub(crate) trait Upper: Copy + Default + Debug {
    const UNBOUND: bool = false;
    fn get(self) -> u8;
    fn check(self, byte: u8) -> bool;
}

impl<T> Lower for Unbound<T> {
    const UNBOUND: bool = true;

    #[inline]
    fn get(self) -> u8 {
        0
    }
    #[inline]
    fn check(self, _byte: u8) -> bool {
        false
    }
}

impl<T> Upper for Unbound<T> {
    const UNBOUND: bool = true;

    #[inline]
    fn get(self) -> u8 {
        255
    }
    #[inline]
    fn check(self, _byte: u8) -> bool {
        false
    }
}

impl Lower for Option<u8> {
    #[inline]
    fn get(self) -> u8 {
        self.unwrap_or(0)
    }
    #[inline]
    fn check(self, byte: u8) -> bool {
        self == Some(byte)
    }
}

impl Upper for Option<u8> {
    #[inline]
    fn get(self) -> u8 {
        self.unwrap_or(255)
    }
    #[inline]
    fn check(self, byte: u8) -> bool {
        self == Some(byte)
    }
}
