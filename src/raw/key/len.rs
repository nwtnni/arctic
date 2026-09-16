use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

/// Key length.
pub(crate) trait Len:
    Sized
    + Copy
    + AddAssign
    + Add<Output = Self>
    + SubAssign
    + Sub<Output = Self>
    + Default
    + Eq
    + Ord
    + core::fmt::Debug
{
    /// Length of an empty key.
    const ZERO: Self;

    /// Length of a key with a single byte.
    const BYTE: Self;

    const MAX: Self;

    /// Return the key length in bits.
    fn bits(self) -> usize;

    /// Return the key length in bytes.
    fn bytes(self) -> usize;

    #[cfg_attr(not(test), expect(unused))]
    fn range_to(self) -> impl Iterator<Item = Self>;
}

#[doc(hidden)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Byte<const MAX: usize = { usize::MAX }>(usize);

impl Byte {
    #[inline]
    pub(crate) const fn new(len: usize) -> Self {
        Self(len)
    }
}

impl<const MAX: usize> Byte<MAX> {
    #[inline]
    pub(crate) fn new_clamped(len: usize) -> Self {
        Self(len.min(MAX))
    }

    #[inline]
    pub(crate) const fn new_const<const LEN: usize>() -> Self {
        const { assert!(LEN <= MAX) };
        Self(LEN)
    }

    #[inline]
    pub(crate) const unsafe fn new_unchecked(len: usize) -> Self {
        validate!(len <= MAX);
        Self(len)
    }

    #[inline]
    pub(crate) fn try_add(parent: Self, byte: Self, child: Self) -> Option<Self> {
        let sum = parent.0 + byte.0 + child.0;
        (sum <= MAX).then_some(sum).map(Self)
    }

    #[inline]
    pub(crate) fn min_bit<const BIT: u8>(self, len: Bit<BIT>) -> Bit<BIT> {
        const { assert!(((BIT as usize) << 3) <= MAX) };

        Bit((self.0 << 3).min(len.0 as usize) as u8)
    }

    #[inline]
    pub(crate) fn min_byte<const BYTE: usize>(self, len: Byte<BYTE>) -> Byte<BYTE> {
        const { assert!(BYTE <= MAX) };

        Byte(self.0.min(len.0))
    }
}

impl<const MAX: usize> From<bool> for Byte<MAX> {
    #[inline]
    fn from(value: bool) -> Self {
        const { assert!(MAX >= 1) };
        Self(value as usize)
    }
}

impl From<Byte<{ (1 << 13) - 1 }>> for Byte {
    #[inline]
    fn from(len: Byte<{ (1 << 13) - 1 }>) -> Self {
        Self(len.bytes())
    }
}

impl<const MAX: usize> Len for Byte<MAX> {
    const ZERO: Self = Self(0);
    const BYTE: Self = Self(1);
    const MAX: Self = Self(MAX);

    #[inline]
    fn bits(self) -> usize {
        self.0 << 3
    }

    #[inline]
    fn bytes(self) -> usize {
        self.0
    }

    #[inline]
    fn range_to(self) -> impl Iterator<Item = Self> {
        (0..=self.0).map(Self)
    }
}

impl<const MAX: usize> Add for Byte<MAX> {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        let value = Self(self.0 + rhs.0);
        validate!(value <= Self::MAX);
        value
    }
}

impl<const MAX: usize> AddAssign for Byte<MAX> {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
        validate!(*self <= Self::MAX);
    }
}

impl<const MAX: usize> Sub for Byte<MAX> {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        validate!(self >= rhs);
        Self(self.0 - rhs.0)
    }
}

impl<const MAX: usize> SubAssign for Byte<MAX> {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        validate!(*self >= rhs);
        self.0 -= rhs.0;
    }
}

#[doc(hidden)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bit<const MAX: u8>(u8);

impl<const MAX: u8> Bit<MAX> {
    const MASK: u8 = 0b11_1000;

    #[inline]
    pub(crate) const fn new_const<const LEN: u8>() -> Self {
        const { assert!(LEN <= MAX) };
        Self(LEN)
    }

    #[inline]
    pub(crate) fn new_masked(len: u8) -> Self {
        // validate!(len <= MAX | 0b111);
        let len = Self(len & Self::MASK);
        validate!(len <= Self::MAX);
        len
    }

    #[inline]
    pub(crate) unsafe fn new_unchecked(len: u8) -> Self {
        validate!(len <= MAX);
        Self(len)
    }

    #[inline]
    pub(crate) fn min<const LO: u8, const HI: u8>(lo: Bit<LO>, hi: Bit<HI>) -> Bit<LO> {
        const { assert!(LO <= HI) };
        Bit(lo.0.min(hi.0))
    }

    #[inline]
    pub(crate) fn align_down(self) -> Self {
        Self(self.0 & Self::MASK)
    }

    #[inline]
    pub(crate) fn try_add(parent: Self, byte: Self, child: Self) -> Option<Self> {
        let sum = parent.0 + byte.0 + child.0;
        (sum <= MAX).then_some(sum).map(Self)
    }

    #[inline]
    pub(crate) const fn into_u8(self) -> u8 {
        self.0
    }
}

impl<const MAX: u8> Len for Bit<MAX> {
    const ZERO: Self = Self(0);
    const BYTE: Self = Self(8);
    const MAX: Self = Self(MAX);

    #[inline]
    fn bits(self) -> usize {
        self.0 as usize
    }

    #[inline]
    fn bytes(self) -> usize {
        (self.0 >> 3) as usize
    }

    #[inline]
    fn range_to(self) -> impl Iterator<Item = Self> {
        (0..=self.0).step_by(8).map(Self)
    }
}

impl<const MAX: u8> Add for Bit<MAX> {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        let value = Self(self.0 + rhs.0);
        validate!(value <= Self::MAX);
        value
    }
}

impl<const MAX: u8> AddAssign for Bit<MAX> {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
        validate!(*self <= Self::MAX);
    }
}

impl<const MAX: u8> Sub for Bit<MAX> {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        validate!(self >= rhs);
        Self(self.0 - rhs.0)
    }
}

impl<const MAX: u8> SubAssign for Bit<MAX> {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        validate!(*self >= rhs);
        self.0 -= rhs.0;
    }
}

impl From<Bit<56>> for Bit<64> {
    #[inline]
    fn from(len: Bit<56>) -> Self {
        Self(len.0)
    }
}

impl From<Bit<56>> for Bit<128> {
    #[inline]
    fn from(len: Bit<56>) -> Self {
        Self(len.0)
    }
}

impl<const BIT: u8, const BYTE: usize> From<Byte<BYTE>> for Bit<BIT> {
    #[inline]
    fn from(len: Byte<BYTE>) -> Self {
        const { assert!(BYTE <= ((BIT as usize) << 3)) };

        Self((len.0 << 3) as u8)
    }
}

impl<const BIT: u8, const BYTE: usize> From<Bit<BIT>> for Byte<BYTE> {
    #[inline]
    fn from(len: Bit<BIT>) -> Self {
        const { assert!(((BIT as usize) << 3) <= BYTE) };

        Self(len.bytes())
    }
}
