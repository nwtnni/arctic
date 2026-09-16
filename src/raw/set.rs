use core::num::NonZeroUsize;
use core::ptr::NonNull;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use crate::key::Len as _;
use crate::sequential;

type Bit = crate::key::len::Bit<56>;

pub(crate) union Set {
    raw: u64,
    set_7: Set7,
    set_256: NonNull<Set256>,
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
        Self { set_7: Set7::EMPTY }
    }
}

impl Set {
    pub fn contains(&self, byte: u8) -> bool {
        match self.as_ref() {
            Ref::Set7(set_7) => set_7.contains(byte),
            Ref::Set256(set_256) => set_256.contains(byte),
        }
    }

    pub fn insert_mut(&mut self, byte: u8) -> bool {
        let set_256 = match self.as_mut() {
            RefMut::Set7(set_7) => match set_7.try_insert_mut(byte) {
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
    unsafe fn expand_mut_unchecked(&mut self) -> &mut Set256 {
        validate!(unsafe { self.raw >> 56 } <= 56);

        let mut set_256 = Box::new(Set256::default());

        unsafe { self.set_7 }.with_bytes(|bytes| {
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
            Ref::Set7(unsafe { &self.set_7 })
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
            RefMut::Set7(unsafe { &mut self.set_7 })
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
    Set7(&'a Set7),
    Set256(&'a Set256),
}

enum RefMut<'a> {
    Set7(&'a mut Set7),
    Set256(&'a mut Set256),
}

#[derive(Copy, Clone)]
struct Set7(u64);

impl Set7 {
    const EMPTY: Self = Self(0);
    const SHIFT_LEN: usize = 56;

    // https://graphics.stanford.edu/~seander/bithacks.html#ZeroInWord
    #[inline]
    fn contains(&self, byte: u8) -> bool {
        let byte = byte as u64;

        // LLVM is smart enough to turn this into an imul
        let broadcast = byte
            | (byte << 8)
            | (byte << 16)
            | (byte << 24)
            | (byte << 32)
            | (byte << 40)
            | (byte << 48);

        crate::raw::find_zero(self.0 ^ broadcast) < self.len().into_u8() >> 3
    }

    fn len(&self) -> Bit {
        unsafe { Bit::new_unchecked((self.0 >> Self::SHIFT_LEN) as u8) }
    }

    fn try_insert_mut(&mut self, byte: u8) -> Result<bool, ()> {
        if self.contains(byte) {
            return Ok(false);
        }

        if self.len() == Bit::MAX {
            return Err(());
        }

        let byte = (byte as u64) << self.len().into_u8();
        self.0 = (self.0 | byte) + (8 << 56);
        Ok(true)
    }

    fn with_bytes<F: FnOnce(&[u8]) -> T, T>(&self, with: F) -> T {
        let buffer = self.0.to_le_bytes();
        let len = self.len().bytes();
        with(&buffer[..len])
    }
}

#[repr(C)]
#[derive(Debug, Default)]
pub(super) struct Set256([AtomicU64; 4]);

impl Set256 {
    pub(super) fn contains(&self, byte: u8) -> bool {
        let (i, bit) = Self::index(byte);
        self.0[i].load(Ordering::Relaxed) & bit == bit
    }

    pub(super) fn insert_mut(&mut self, byte: u8) -> bool {
        let (i, bit) = Self::index(byte);
        let row = self.0[i].get_mut();
        if *row & bit == bit {
            return false;
        }
        *row |= bit;
        true
    }

    #[cfg_attr(not(any(test, feature = "proptest")), expect(unused))]
    pub(super) fn remove_mut(&mut self, byte: u8) -> bool {
        let (i, bit) = Self::index(byte);
        let row = self.0[i].get_mut();
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

impl Eq for Set256 {}

impl PartialEq for Set256 {
    fn eq(&self, other: &Self) -> bool {
        self.0
            .iter()
            .map(|row| row.load(Ordering::Relaxed))
            .eq(other.0.iter().map(|row| row.load(Ordering::Relaxed)))
    }
}

impl Clone for Set256 {
    fn clone(&self) -> Self {
        Self(core::array::from_fn(|i| {
            AtomicU64::new(self.0[i].load(Ordering::Relaxed))
        }))
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
impl proptest::bits::BitSetLike for Set256 {
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
    fn smoke_set_7() {
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
