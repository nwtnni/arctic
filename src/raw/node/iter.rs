//! Iteration over keys and key-edge pairs of a single node.

use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::ops::Deref;
use core::ops::DerefMut;
use core::ptr::NonNull;

use ribbit::Atomic;
use ribbit::Pack as _;
use ribbit::traits::Integer as _;
use ribbit::u2;

use crate::raw::Edge;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::raw::node::linear;
use crate::raw::node::node_256;

/// Iterator over key-edge pairs.
pub(crate) struct EntryIter<'g, M: ribbit::Pack> {
    keys: KeyIter,
    edges: NonNull<Atomic<Edge<M>>>,

    #[cfg(feature = "validate")]
    len: u16,

    _slice: PhantomData<&'g [Atomic<Edge<M>>]>,
}

impl<'g, M: ribbit::Pack> EntryIter<'g, M> {
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

impl<'g, M: ribbit::Pack> Iterator for EntryIter<'g, M> {
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

impl<'g, M: ribbit::Pack> DoubleEndedIterator for EntryIter<'g, M> {
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

impl<'g, M: ribbit::Pack> ExactSizeIterator for EntryIter<'g, M> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

/// Iterator over (key byte, edge index). Heavily optimized for space because
/// (a) range iteration requires keeping a stack of `KeyIter`s, and
/// (b) most of them are `KeyIter3`, which is 8 bytes.
///
/// We can trivially get the size down to 9-16 bytes by allocating large variants
/// and keeping a separate discriminant. It turns out it is possible to get the size
/// down to 8 bytes, but this requires delicate reasoning about endianness,
/// allocation alignment, and struct layout.
#[repr(C)]
pub(crate) union KeyIter {
    /// Stored inline.
    ///
    /// We know that:
    /// - [`crate::raw::node::Type::Node3`] has value 0.
    /// - [`crate::raw::node::linear::KeyIter3`]'s `tail` field is laid out at
    ///   the highest byte address, and its value is <= 3.
    ///
    /// This leaves bits 3..8 at the highest byte address available.
    node_3: linear::KeyIter3,

    /// Stored in `Box`.
    ///
    /// [`crate::raw::node::linear::KeyIter15`] is 32-byte aligned, and we assume pointers
    /// are 56 bytes or less, leaving bits 0..5 and 56..64 available (addresses are
    /// endian-dependent). Combined with the `node_3` constraint, this leaves us
    /// with exactly bits 3..5 of the highest byte, endian-independent.
    node_15: NonNull<linear::KeyIter15>,

    /// Stored in `Box`.
    ///
    /// Same reasoning as `node_15`.
    node_47: NonNull<linear::KeyIter63>,

    /// Stored inline.
    ///
    /// [`crate::raw::node::node_256::KeyIter`] fits in 4 bytes, so we can move it
    /// relatively freely.
    node_256: KeyIter256,

    raw: [u8; 8],
}

const_assert_size_align!(KeyIter, 8, 8);

/// Wrapper around [`crate::raw::node::node_256::KeyIter`] that
/// includes discriminant at highest byte address.
#[repr(C, align(8))]
#[derive(Copy, Clone)]
struct KeyIter256 {
    iter: node_256::KeyIter,
    _pad: [u8; 3],
    _type: Type256,
}

impl Deref for KeyIter256 {
    type Target = node_256::KeyIter;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.iter
    }
}

impl DerefMut for KeyIter256 {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.iter
    }
}

/// Discriminant offset within a single byte.
const TYPE_SHIFT_BYTE: usize = 3;

/// Discriminant offset within a pointer (requires endian-dependent
/// shift to reach highest byte address).
const TYPE_SHIFT_PTR: usize = if cfg!(target_endian = "little") {
    56 + TYPE_SHIFT_BYTE
} else {
    TYPE_SHIFT_BYTE
};

const _: () = assert!(align_of::<linear::KeyIter15>() == 32);
const TYPE_15: usize = (node::Type::Node15 as usize) << TYPE_SHIFT_PTR;

const _: () = assert!(align_of::<linear::KeyIter63>() == 32);
const TYPE_47: usize = (node::Type::Node47 as usize) << TYPE_SHIFT_PTR;

/// Enum with a single possible bit representation.
#[repr(u8)]
#[derive(Copy, Clone)]
enum Type256 {
    Type = (node::Type::Node256 as u8) << TYPE_SHIFT_BYTE,
}

impl KeyIter {
    // HACK: used for postorder traversal
    pub(crate) const ROOT: Self = Self {
        node_3: linear::KeyIter3::new_3([KeyIndex::DEFAULT; 3], 1),
    };

    #[inline]
    fn r#type(&self) -> ribbit::Packed<node::Type> {
        // `node_3` and `node_256` are structs with endian-independent layout
        // `node_15` and `node_47` use an endian-dependent shift when encoding
        let byte = unsafe { self.raw[7] } >> TYPE_SHIFT_BYTE;

        let r#type = if cfg!(target_endian = "little") {
            // Pointer bits 61..64 are zero
            u2::new(byte)
        } else {
            // Pointer bits 5..8 may have data
            u2::masked_new(byte)
        };

        // SAFETY: every `u2` is a valid `ribbit::Packed<node::Type>`
        unsafe { ribbit::Packed::<node::Type>::new_unchecked(r#type) }
    }

    #[inline]
    pub(super) fn new_3(node_3: linear::KeyIter3) -> Self {
        let iter = Self { node_3 };
        validate_eq!(iter.r#type(), node::Type::Node3.pack());
        iter
    }

    #[inline]
    pub(super) fn new_15(node_15: Box<linear::KeyIter15>) -> Self {
        let iter = Self {
            node_15: NonNull::from(Box::leak(node_15)).map_addr(|addr| {
                validate_eq!(
                    u2::masked_new((addr.get() >> TYPE_SHIFT_PTR) as u8),
                    u2::new(0),
                    "Type does not clobber address",
                );

                // SAFETY: `Self::TYPE_15 > 0`
                unsafe { NonZeroUsize::new_unchecked(addr.get() | TYPE_15) }
            }),
        };

        validate_eq!(iter.r#type(), node::Type::Node15.pack());
        iter
    }

    #[inline]
    pub(super) fn new_47(node_47: Box<linear::KeyIter63>) -> Self {
        let iter = Self {
            node_47: NonNull::from(Box::leak(node_47)).map_addr(|addr| {
                validate_eq!(
                    u2::masked_new((addr.get() >> TYPE_SHIFT_PTR) as u8),
                    u2::new(0),
                    "Type does not clobber address",
                );

                // SAFETY: `Self::TYPE_47 > 0`
                unsafe { NonZeroUsize::new_unchecked(addr.get() | TYPE_47) }
            }),
        };

        validate_eq!(iter.r#type(), node::Type::Node47.pack());
        iter
    }

    #[inline]
    pub(super) fn new_256(node_256: node_256::KeyIter) -> Self {
        let iter = Self {
            node_256: KeyIter256 {
                iter: node_256,
                _pad: [0; 3],
                _type: Type256::Type,
            },
        };

        validate_eq!(iter.r#type(), node::Type::Node256.pack());
        iter
    }

    #[inline]
    unsafe fn as_node_15_unchecked(&self) -> NonNull<linear::KeyIter15> {
        validate_eq!(self.r#type(), node::Type::Node15.pack());

        unsafe {
            self.node_15.map_addr(|addr| {
                validate_eq!(addr.get() & TYPE_15, TYPE_15);
                NonZeroUsize::new_unchecked(addr.get() ^ TYPE_15)
            })
        }
    }

    #[inline]
    unsafe fn as_node_47_unchecked(&self) -> NonNull<linear::KeyIter63> {
        validate_eq!(self.r#type(), node::Type::Node47.pack());

        unsafe {
            self.node_47.map_addr(|addr| {
                validate_eq!(addr.get() & TYPE_47, TYPE_47);
                NonZeroUsize::new_unchecked(addr.get() ^ TYPE_47)
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
            unsafe { &mut self.node_256 }.next(),
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
            unsafe { &mut self.node_256 }.next_back(),
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

/// A key byte and the edge index it is mapped to.
///
/// NOTE: These fields are ordered so that interpreting the struct
/// as a u16 results in the correct ordering (by key first, then index),
/// for SIMD purposes.
#[repr(C, align(2))]
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) struct KeyIndex {
    #[cfg(target_endian = "little")]
    pub(super) index: u8,

    pub(super) key: u8,

    #[cfg(target_endian = "big")]
    pub(super) index: u8,
}

const_assert_size_align!(KeyIndex, 2, 2);

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
        // SAFETY: `Self` is repr(C) and has same size and alignment as u16
        let actual = unsafe {
            core::mem::transmute_copy::<Self, u16>(self)
                .cmp(&core::mem::transmute_copy::<Self, u16>(other))
        };

        validate_eq!(
            actual,
            self.key.cmp(&other.key).then(self.index.cmp(&other.index))
        );

        actual
    }
}

impl Debug for KeyIndex {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:#.02X}:{:#.02X}", self.key, self.index)
    }
}

/// Byte lower bound for range iteration.
pub(crate) trait Lower: Copy + Default + Debug {
    const UNBOUND: bool = false;
    fn get(self) -> u8;
    fn check(self, byte: u8) -> bool;
}

/// Byte upper bound for range iteration.
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
