use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use ribbit::u6;
use ribbit::u13;

/// Key length.
pub(crate) trait Len:
    Sized
    + Copy
    + AddAssign
    + Add<Output = Self>
    + SubAssign
    + Sub<Output = Self>
    + PartialOrd
    + core::fmt::Debug
{
    /// Length of an empty key.
    const ZERO: Self;

    /// Length of a key with a single byte.
    const BYTE: Self;

    /// Return the key length in bits.
    fn bits(self) -> usize;

    /// Return the key length in bytes.
    fn bytes(self) -> usize;
}

#[doc(hidden)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Byte(pub(super) usize);

impl Len for Byte {
    const ZERO: Self = Self(0);
    const BYTE: Self = Self(1);

    #[inline]
    fn bits(self) -> usize {
        self.0 << 3
    }

    #[inline]
    fn bytes(self) -> usize {
        self.0
    }
}

impl From<u6> for Byte {
    #[inline]
    fn from(len: u6) -> Self {
        Self((len.value() >> 3) as usize)
    }
}

impl From<u13> for Byte {
    #[inline]
    fn from(len: u13) -> Self {
        Self(len.value() as usize)
    }
}

impl From<Byte> for u6 {
    #[inline]
    fn from(len: Byte) -> Self {
        u6::extract_u64((len.0 << 3) as u64, 0)
    }
}

impl From<Byte> for u13 {
    #[inline]
    fn from(len: Byte) -> Self {
        u13::extract_u64(len.0 as u64, 0)
    }
}

impl Add for Byte {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Byte {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Byte {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for Byte {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

#[doc(hidden)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bit(pub(super) u8);

impl From<u6> for Bit {
    #[inline]
    fn from(len: u6) -> Self {
        Self(len.value())
    }
}

impl From<Bit> for u6 {
    #[inline]
    fn from(len: Bit) -> Self {
        const MASK: u8 = 0b0011_1000;
        unsafe { u6::new_unchecked(len.0 & MASK) }
    }
}

impl Len for Bit {
    const ZERO: Self = Self(0);
    const BYTE: Self = Self(8);

    #[inline]
    fn bits(self) -> usize {
        self.0 as usize
    }

    #[inline]
    fn bytes(self) -> usize {
        (self.0 >> 3) as usize
    }
}

impl Add for Bit {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Bit {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Bit {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for Bit {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}
