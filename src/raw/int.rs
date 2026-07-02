/// Abstraction over unsigned native integer types.
pub(crate) trait Int:
    'static
    + Sized
    + Copy
    + Default
    + core::fmt::Debug
    + Ord
    + Eq
    + core::ops::Shl<u8, Output = Self>
    + core::ops::ShlAssign<u8>
    + core::ops::Shr<u8, Output = Self>
    + core::ops::BitXor<Output = Self>
    + core::ops::BitOr<Output = Self>
    + core::ops::BitOrAssign
    + core::ops::Not<Output = Self>
    + core::ops::BitAnd<Output = Self>
{
    const MAX: Self;
    const BITS: u8;

    fn from_be_bytes(bytes: &[u8]) -> Self;

    fn with_be_bytes<F: FnOnce(&[u8]) -> T, T>(self, apply: F) -> T;

    fn most_significant_u64(self) -> u64;

    fn get_u8(self, bits: u8) -> u8;

    #[inline]
    fn most_significant(self, bits: u8) -> Self {
        Self::MAX.unbounded_shr(bits).not().bitand(self)
    }

    fn unbounded_shl(self, bits: u8) -> Self;
    fn unbounded_shr(self, bits: u8) -> Self;
    fn leading_zeros(self) -> u8;

    fn from_most_significant_u64(value: u64) -> Self;
    fn from_u8(value: u8) -> Self;

    fn least_significant_u8(self) -> u8;
}

macro_rules! impl_int {
    ($($ty:ty: $bits:expr, $into_u64:expr, $from_u64:expr, $into_u128:expr),* $(,)?) => {
        $(
            impl Int for $ty {
                const MAX: Self = <$ty>::MAX;
                const BITS: u8 = <$ty>::BITS as u8;

                #[inline]
                fn from_be_bytes(bytes: &[u8]) -> Self {
                    Self::from_be_bytes(core::array::from_fn(|i| bytes.get(i).copied().unwrap_or(0)))
                }

                #[inline]
                fn with_be_bytes<F: FnOnce(&[u8]) -> T, T>(self, apply: F) -> T {
                    apply(&self.to_be_bytes())
                }

                #[inline]
                fn most_significant_u64(self) -> u64 {
                    $into_u64(self)
                }

                #[inline]
                fn get_u8(self, bits: u8) -> u8 {
                    <$ty>::rotate_left(self, 8 + bits as u32) as u8
                }

                #[inline]
                fn unbounded_shl(self, bits: u8) -> Self {
                    <$ty>::unbounded_shl(self, bits as u32)
                }

                #[inline]
                fn unbounded_shr(self, bits: u8) -> Self {
                    <$ty>::unbounded_shr(self, bits as u32)
                }

                #[inline]
                fn leading_zeros(self) -> u8 {
                    <$ty>::leading_zeros(self) as u8
                }

                #[inline]
                fn from_most_significant_u64(value: u64) -> Self {
                    $from_u64(value)
                }

                #[inline]
                fn from_u8(value: u8) -> Self {
                    (value as $ty).rotate_right(8)
                }

                #[inline]
                fn least_significant_u8(self) -> u8 {
                    self as u8
                }
            }
        )*
    };
}

impl_int!(
    u16: 16, |from: Self| {
        (from as u64) << 48
    }, |into: u64| {
        (into >> 48) as Self
    }, |from: Self| {
        (from as u128) << 112
    },

    u32: 32, |from: Self| {
        (from as u64) << 32
    }, |into: u64| {
        (into >> 32) as Self
    }, |from: Self| {
        (from as u128) << 96
    },

    u64: 64, core::convert::identity, core::convert::identity, |from: Self| {
        (from as u128) << 64
    },

    u128: 128, |into: u128| {
        (into >> 64) as u64
    }, |from: u64| {
        (from as u128) << 64
    }, core::convert::identity,
);
