#![cfg_attr(not(test), expect(unused))]

use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::u6;
use ribbit::u56;

use crate::raw::edge::Len as _;
use crate::raw::node;
use crate::sequential;

type AtomicU64 = <u64 as crate::sync::Loose>::Atomic;

pub(crate) union Set {
    raw: u64,
    set_8: ribbit::Packed<Set8>,
    set_256: NonNull<Set256<AtomicU64>>,
}

unsafe impl sequential::Value for Set {
    #[inline]
    fn into_raw(self) -> u64 {
        unsafe { self.raw }
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        Set { raw }
    }
}

impl Default for Set {
    fn default() -> Self {
        Self { set_8: Set8::EMPTY }
    }
}

impl Set {
    pub fn contains(&self, byte: u8) -> bool {
        match self.as_ref() {
            Ref::Set8(set_8) => set_8.contains(byte),
            Ref::Set256(set_256) => set_256.contains(byte),
        }
    }

    pub fn insert_mut(&mut self, byte: u8) -> bool {
        let set_256 = match self.as_mut() {
            RefMut::Set8(set_8) => match set_8.try_insert_mut(byte) {
                Ok(inserted) => return inserted,
                Err(()) => unsafe { self.expand_mut_unchecked() },
            },
            RefMut::Set256(set_256) => set_256,
        };

        set_256.insert_mut(byte)
    }

    /// # Safety
    ///
    /// Caller must ensure `self` is `Set8`.
    unsafe fn expand_mut_unchecked(&mut self) -> &mut Set256<AtomicU64> {
        validate!(unsafe { self.raw >> 56 } <= 56);

        let mut set_256 = Box::new(Set256::default());

        unsafe { self.set_8 }.with_bytes(|bytes| {
            bytes.iter().for_each(|byte| {
                set_256.insert_mut(*byte);
            });
        });

        let mut set_256 = NonNull::new(Box::into_raw(set_256)).expect("Box is non-null");
        *self = Self {
            set_256: set_256.map_addr(|address| {
                validate!(address.get() < (1 << 56));
                address.saturating_add(64 << 56)
            }),
        };
        unsafe { set_256.as_mut() }
    }

    fn as_ref<'g>(&'g self) -> Ref<'g> {
        if unsafe { self.raw >> 56 } <= 56 {
            Ref::Set8(unsafe { &self.set_8 })
        } else {
            Ref::Set256(unsafe {
                self.set_256
                    .map_addr(|address| {
                        validate_eq!(address.get() >> 56, 64);
                        NonZeroUsize::new(address.get() ^ (64 << 56)).unwrap()
                    })
                    .as_ref()
            })
        }
    }

    fn as_mut<'g>(&'g mut self) -> RefMut<'g> {
        if unsafe { self.raw >> 56 } <= 56 {
            RefMut::Set8(unsafe { &mut self.set_8 })
        } else {
            RefMut::Set256(unsafe {
                self.set_256
                    .map_addr(|address| {
                        validate_eq!(address.get() >> 56, 64);
                        NonZeroUsize::new(address.get() ^ (64 << 56)).unwrap()
                    })
                    .as_mut()
            })
        }
    }
}

enum Ref<'a> {
    Set8(&'a ribbit::Packed<Set8>),
    Set256(&'a Set256<AtomicU64>),
}

enum RefMut<'a> {
    Set8(&'a mut ribbit::Packed<Set8>),
    Set256(&'a mut Set256<AtomicU64>),
}

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 64)]
struct Set8 {
    set: u56,
    len: u6,
}

impl Set8 {
    const EMPTY: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(u56::new(0), u6::new(0));
}

impl Set8Packed {
    fn contains(&self, byte: u8) -> bool {
        node::simd::get_15(self.into_raw() as u128, byte) < self.len().bytes() as u8
    }

    fn try_insert_mut(&mut self, byte: u8) -> Result<bool, ()> {
        if self.contains(byte) {
            return Ok(false);
        }

        if self.len().bits() >= 56 {
            validate!(self.len().bits() == 56);
            return Err(());
        }

        let byte = (byte as u64) << self.len().bits();
        *self = unsafe { Self::from_raw_unchecked((self.into_raw() | byte) + (8 << 56)) };
        Ok(true)
    }

    fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        let buffer = self.into_raw().to_le_bytes();
        let len = self.len().bytes();
        with(&buffer[..len])
    }
}

#[repr(C)]
#[derive(Default)]
pub(super) struct Set256<R>([ribbit::Atomic<u64, R>; 4]);

impl<R: ribbit::atomic::Raw<u64>> Set256<R> {
    pub(super) fn contains(&self, byte: u8) -> bool {
        let (i, bit) = Self::index(byte);
        self.0[i].load(Ordering::Relaxed) & bit == bit
    }

    pub(super) fn insert_mut(&mut self, byte: u8) -> bool {
        let (i, bit) = Self::index(byte);
        let row = self.0[i].get_mut_packed();
        if *row & bit == bit {
            return false;
        }
        *row |= bit;
        true
    }

    #[cfg_attr(not(feature = "proptest"), expect(unused))]
    pub(super) fn remove_mut(&mut self, byte: u8) -> bool {
        let (i, bit) = Self::index(byte);
        let row = self.0[i].get_mut_packed();
        let old = (*row & bit) > 0;
        *row &= !bit;
        old
    }

    #[inline]
    fn index(byte: u8) -> (usize, u64) {
        let i = byte / 64;
        let j = byte % 64;
        (i as usize, 1u64 << j)
    }

    #[cfg_attr(not(test), expect(unused))]
    pub(super) fn len(&self) -> usize {
        self.0
            .iter()
            .map(|row| row.load(Ordering::Relaxed).count_ones() as usize)
            .sum()
    }

    #[cfg_attr(not(feature = "proptest"), expect(unused))]
    pub(super) fn iter(&self) -> Iter256 {
        Iter256(core::array::from_fn(|i| self.0[i].load(Ordering::Relaxed)))
    }
}

impl<R: ribbit::atomic::Raw<u64>> Eq for Set256<R> {}

impl<R: ribbit::atomic::Raw<u64>> PartialEq for Set256<R> {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .map(|row| row.load(Ordering::Relaxed))
            .eq(other.0.iter().map(|row| row.load(Ordering::Relaxed)))
    }
}

impl<R: ribbit::atomic::Raw<u64>> Clone for Set256<R> {
    fn clone(&self) -> Self {
        Self(core::array::from_fn(|i| {
            ribbit::Atomic::new(self.0[i].load(Ordering::Relaxed))
        }))
    }
}

impl<R: ribbit::atomic::Raw<u64>> core::fmt::Debug for Set256<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Set256").field(&self.0).finish()
    }
}

pub(super) struct Iter256([u64; 4]);

impl Iterator for Iter256 {
    type Item = u8;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.iter_mut().enumerate().find_map(|(i, row)| {
            let j = row.trailing_zeros();

            if j == u64::BITS {
                return None;
            }

            *row ^= 1 << j;
            Some((i as u8) * 64 + j as u8)
        })
    }
}

#[cfg(feature = "proptest")]
impl proptest::bits::BitSetLike for Set256<core::sync::atomic::AtomicU64> {
    fn new_bitset(max: usize) -> Self {
        assert!(max <= 256, "Only supports 256 bit sets");
        Self::default()
    }

    fn len(&self) -> usize {
        256
    }

    fn test(&self, ix: usize) -> bool {
        self.contains(ix as u8)
    }

    fn set(&mut self, ix: usize) {
        self.insert_mut(ix as u8);
    }

    fn clear(&mut self, ix: usize) {
        self.remove_mut(ix as u8);
    }
}

#[cfg(test)]
mod tests {
    use crate::raw::Set;

    #[test]
    fn smoke_set_8() {
        let mut set = Set::default();

        for i in 0..8 {
            assert!(set.insert_mut(i));
        }

        for i in 0..8 {
            assert!(set.contains(i));
        }
    }

    #[test]
    fn smoke_set_256() {
        let mut set = Set::default();

        for i in 0..=255 {
            assert!(set.insert_mut(i));
        }

        for i in 0..=255 {
            assert!(set.contains(i));
        }
    }
}
