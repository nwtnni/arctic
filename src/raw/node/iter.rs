//! Iteration over keys and key-edge pairs of a single node.

use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroUsize;
use core::ptr::NonNull;

use fearless_simd::u16x16;

use crate::raw::edge;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::sync::Atomic128;

/// Iterator over key-edge pairs.
pub(crate) struct EntryIter<'g> {
    keys: KeyIter,
    edges: NonNull<Atomic128<edge::Raw>>,

    #[cfg(feature = "validate")]
    len: u16,

    _slice: PhantomData<&'g [Atomic128<edge::Raw>]>,
}

impl<'g> EntryIter<'g> {
    /// # SAFETY
    ///
    /// Caller must guarantee all indices produced by `keys` are < `edges.len()`.
    #[inline]
    pub(crate) unsafe fn new(keys: KeyIter, edges: &'g [Atomic128<edge::Raw>]) -> Self {
        Self {
            keys,
            edges: NonNull::from(edges).cast(),

            #[cfg(feature = "validate")]
            len: edges.len() as u16,

            _slice: PhantomData,
        }
    }
}

impl<'g> Iterator for EntryIter<'g> {
    type Item = (u8, NonNull<Atomic128<edge::Raw>>);

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

impl<'g> DoubleEndedIterator for EntryIter<'g> {
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

impl<'g> ExactSizeIterator for EntryIter<'g> {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

/// Byte lower bound for range scans.
pub(crate) trait Lower: Copy + Default + Debug {
    fn get(self) -> u8;
    fn check(self, byte: u8) -> bool;
}

/// Byte upper bound for range scans.
pub(crate) trait Upper: Copy + Default + Debug {
    fn get(self) -> u8;
    fn check(self, byte: u8) -> bool;
}

impl<T> Lower for Unbound<T> {
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

// Strategy that generates (lower: u8, upper: u8) bound with lower <= upper
#[cfg(feature = "proptest")]
proptest::prop_compose! {
    pub(super) fn bound()
    (lower in u8::MIN..=u8::MAX)
    (lower in proptest::strategy::Just(lower), upper in lower..=u8::MAX) -> (u8, u8) {
        (lower, upper)
    }
}

/// Iterator over (key byte, edge index). Optimized for space because
/// tree iteration requires keeping a stack of `KeyIter`s.
#[repr(C)]
pub(crate) union KeyIter {
    /// Stored inline.
    ///
    /// [`KeyIter1`] fits in 2 bytes.
    node_1: KeyIter1,

    /// Stored inline.
    ///
    /// We know that:
    /// - [`crate::raw::node::Type::Node3`] has value 0.
    /// - [`crate::raw::node::linear::KeyIter3`]'s `tail` field is laid out at
    ///   the highest byte address, and its value is <= 3.
    ///
    /// This leaves bits 3..8 at the highest byte address available.
    node_3: KeyIter3,

    /// Stored in `Box`.
    ///
    /// [`KeyIter15`] is 32-byte aligned, and we assume pointers
    /// are 56 bytes or less, leaving bits 56..64 available (assuming little-endian).
    /// Combined with the `node_3` constraint, this leaves us with bits 59..64.
    node_15: NonNull<KeyIter15>,

    /// Stored in `Box`.
    ///
    /// Same reasoning as `node_15`.
    node_47: NonNull<KeyIter47>,

    /// Stored inline.
    ///
    /// [`KeyIter256`] fits in 4 bytes.
    node_256: KeyIter256,

    raw: [u8; 8],
}

const_assert_size_align!(KeyIter, 8, 8);

#[cfg(not(target_endian = "little"))]
compile_error!("arctic::raw::node::iter::KeyIter representation requires little-endian system");

/// Discriminant offset within a single byte.
const SHIFT_TYPE_BYTE: usize = 3;

/// Discriminant offset within a pointer (little-endian).
const SHIFT_TYPE_ADDR: usize = 56 + SHIFT_TYPE_BYTE;

const _: () = assert!(align_of::<KeyIter15>() == 32);
const TYPE_15: usize = (node::Type::Node15 as usize) << SHIFT_TYPE_ADDR;

const _: () = assert!(align_of::<KeyIter47>() == 32);
const TYPE_47: usize = (node::Type::Node47 as usize) << SHIFT_TYPE_ADDR;

const MASK_TYPE_ADDR: usize = 0b111 << SHIFT_TYPE_ADDR;

/// Enum with a single possible bit representation.
#[repr(u8)]
#[derive(Copy, Clone, Default, Debug)]
enum Type1 {
    #[default]
    Type = (node::Type::Node1 as u8) << SHIFT_TYPE_BYTE,
}

/// Enum with a single possible bit representation.
#[repr(u8)]
#[derive(Copy, Clone, Default, Debug)]
enum Type256 {
    #[default]
    Type = (node::Type::Node256 as u8) << SHIFT_TYPE_BYTE,
}

impl KeyIter {
    pub(crate) const DUMMY: Self = KeyIter {
        node_1: KeyIter1::DUMMY,
    };

    #[inline]
    fn r#type(&self) -> node::Type {
        unsafe { node::Type::new_masked(self.raw[7] >> SHIFT_TYPE_BYTE) }
    }

    #[inline]
    unsafe fn as_node_15_unchecked(&self) -> NonNull<KeyIter15> {
        validate_eq!(self.r#type(), node::Type::Node15);

        unsafe {
            self.node_15.map_addr(|addr| {
                validate_eq!(addr.get() & TYPE_15, TYPE_15);
                NonZeroUsize::new_unchecked(addr.get() ^ TYPE_15)
            })
        }
    }

    #[inline]
    unsafe fn as_node_47_unchecked(&self) -> NonNull<KeyIter47> {
        validate_eq!(self.r#type(), node::Type::Node47);

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
            unsafe { &mut self.node_1 }.next(),
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
            unsafe { &self.node_1 }.size_hint(),
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
            unsafe { &mut self.node_1 }.next_back(),
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
            (),
            drop(unsafe { Box::from_raw(self.as_node_15_unchecked().as_ptr()) }),
            drop(unsafe { Box::from_raw(self.as_node_47_unchecked().as_ptr()) }),
            (),
        )
    }
}

impl From<KeyIter1> for node::KeyIter {
    #[inline]
    fn from(iter: KeyIter1) -> Self {
        let iter = Self { node_1: iter };
        validate_eq!(iter.r#type(), node::Type::Node1);
        iter
    }
}

impl From<KeyIter3> for node::KeyIter {
    #[inline]
    fn from(iter: KeyIter3) -> Self {
        let iter = Self { node_3: iter };
        validate_eq!(iter.r#type(), node::Type::Node3);
        iter
    }
}

impl From<Box<KeyIter15>> for node::KeyIter {
    #[inline]
    fn from(iter: Box<KeyIter15>) -> Self {
        let iter = Self {
            node_15: NonNull::from(Box::leak(iter)).map_addr(|addr| {
                validate_eq!(
                    addr.get() & MASK_TYPE_ADDR,
                    0,
                    "Type does not clobber address",
                );

                // SAFETY: `Self::TYPE_15 > 0`
                unsafe { NonZeroUsize::new_unchecked(addr.get() | TYPE_15) }
            }),
        };

        validate_eq!(iter.r#type(), node::Type::Node15);
        iter
    }
}

impl From<Box<KeyIter47>> for node::KeyIter {
    #[inline]
    fn from(iter: Box<KeyIter47>) -> Self {
        let iter = Self {
            node_47: NonNull::from(Box::leak(iter)).map_addr(|addr| {
                validate_eq!(
                    addr.get() & MASK_TYPE_ADDR,
                    0,
                    "Type does not clobber address",
                );

                // SAFETY: `Self::TYPE_47 > 0`
                unsafe { NonZeroUsize::new_unchecked(addr.get() | TYPE_47) }
            }),
        };

        validate_eq!(iter.r#type(), node::Type::Node47);
        iter
    }
}

impl From<KeyIter256> for node::KeyIter {
    #[inline]
    fn from(iter: KeyIter256) -> Self {
        let iter = Self { node_256: iter };
        validate_eq!(iter.r#type(), node::Type::Node256);
        iter
    }
}

/// NOTE: We order `head` and `tail` fields at the end
/// to allow `entries` to be filled in with a single
/// aligned SIMD write.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) struct KeyIterN<const N: usize> {
    pub(super) entries: [node::iter::KeyIndex; N],
    pub(super) head: u8,
    pub(super) tail: u8,
}

impl<const N: usize> Default for KeyIterN<N> {
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
pub(in crate::raw) struct KeyIter3(pub(super) KeyIterN<3>);

#[repr(C, align(32))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct KeyIter15(pub(super) KeyIterN<15>);

// NOTE: has 63 entries instead of 47 to allow
// unmasked 16-byte SIMD writes.
#[repr(C, align(32))]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct KeyIter47(pub(super) KeyIterN<63>);

const_assert_size_align!(KeyIter3, 8, 8);
const_assert_size_align!(KeyIter15, 32, 32);
const_assert_size_align!(KeyIter47, 128, 32);

macro_rules! impl_key_iter {
    ($ty:ty, $len:expr $(,)?) => {
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

impl_key_iter!(KeyIter3, 3);
impl_key_iter!(KeyIter15, 15);
impl_key_iter!(KeyIter47, 63);

impl KeyIter3 {
    #[inline]
    pub(super) fn sort(&mut self) {
        if self.0.tail <= 1 {
            return;
        }

        let mut a = self.0.entries[0];
        let mut b = self.0.entries[1];

        if self.0.tail == 2 {
            self.0.entries[0] = a.min(b);
            self.0.entries[1] = a.max(b);
            return;
        }

        let mut c = self.0.entries[2];

        if a > b {
            core::mem::swap(&mut a, &mut b);
        }

        if a > c {
            core::mem::swap(&mut a, &mut c);
        }

        if b > c {
            core::mem::swap(&mut b, &mut c);
        }

        self.0.entries[0] = a;
        self.0.entries[1] = b;
        self.0.entries[2] = c;
    }
}

impl KeyIter15 {
    #[inline]
    pub(super) fn sort(&mut self) {
        let len = self.0.tail;

        fearless_simd::dispatch!(*crate::raw::SIMD, simd => {
            let ptr = NonNull::from(&mut *self).cast::<u16x16<_>>();
            let unsorted = unsafe { ptr.read() };
            let sorted = node::simd::sort_u16x16(simd, unsorted, len);
            unsafe { ptr.write(sorted) };
        });

        self.0.head = 0;
        self.0.tail = len;
    }
}

#[repr(C, align(8))]
#[derive(Copy, Clone, Default)]
pub(crate) struct KeyIter1 {
    key: u8,
    pub(super) done: bool,
    _pad: [u8; 5],
    _type: Type1,
}

impl KeyIter1 {
    const DUMMY: Self = Self {
        key: 0,
        done: true,
        _pad: [0u8; 5],
        _type: Type1::Type,
    };

    #[inline]
    pub(super) fn new<L: Lower, U: Upper>(lower: L, upper: U, key: u8) -> Self {
        Self {
            key,
            done: key < lower.get() || key > upper.get(),
            _pad: [0; 5],
            _type: Type1::Type,
        }
    }
}

impl Iterator for KeyIter1 {
    type Item = KeyIndex;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        self.done = true;
        Some(KeyIndex {
            key: self.key,
            index: 0,
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = (!self.done) as usize;
        (len, Some(len))
    }
}

impl ExactSizeIterator for KeyIter1 {
    #[inline]
    fn len(&self) -> usize {
        let (lower, upper) = self.size_hint();
        validate_eq!(upper, Some(lower));
        lower
    }
}

impl DoubleEndedIterator for KeyIter1 {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        self.done = true;
        Some(KeyIndex {
            key: self.key,
            index: 0,
        })
    }
}

impl Debug for KeyIter1 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyIter1")
            .field("key", &self.key)
            .field("done", &self.done)
            .finish()
    }
}

#[repr(C, align(8))]
#[derive(Copy, Clone, Default)]
pub(crate) struct KeyIter256 {
    head: u16,
    tail: u16,
    _pad: [u8; 3],
    _type: Type256,
}

impl KeyIter256 {
    #[inline]
    pub(super) fn new<L: node::iter::Lower, U: node::iter::Upper>(lower: L, upper: U) -> Self {
        Self {
            head: lower.get() as u16,
            tail: upper.get() as u16 + 1,
            _pad: [0; 3],
            _type: Type256::Type,
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

impl Debug for KeyIter256 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyIter256")
            .field("head", &self.head)
            .field("tail", &self.tail)
            .finish()
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
