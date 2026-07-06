//! Support for unsigned integer keys.

use ribbit::u6;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Bit;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

macro_rules! impl_key {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Key for $ty {
                type Read<'k> = Reader<$ty>;
                type Write = Writer<$ty>;
                type Borrowed = Self;
                type Insert<'k> = Self;

                type Edge = edge::Be;
                type Len = Bit;

                #[inline]
                fn as_insert(&self) -> Self::Insert<'_> {
                    *self
                }

                #[inline]
                fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
                where
                    Self: 'k,
                {
                    Reader::from(insert)
                }

                fn insert_to_key<'k>(insert: Self::Insert<'k>) -> Self
                where
                    Self: 'k,
                {
                    insert
                }

                #[inline]
                unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k> where Self: 'k{
                    writer.0
                }
            }

            impl key::Split for $ty {
                #[inline]
                fn split_last<'k>(key: &'k Self::Borrowed) -> (Self::Read<'k>, u8) {
                    let reader = Reader::from(key);
                    (
                        Reader {
                            buffer: reader.buffer,
                            len: reader.len.0.checked_sub(Self::Len::BYTE.0).map(Bit).expect("Non-empty"),
                        },
                        reader.buffer.least_significant_u8(),
                    )
                }
            }

            impl From<$ty> for Reader<$ty> {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self {
                        buffer: value,
                        len: Bit(<$ty as Native>::BITS),
                    }
                }
            }

            impl<'k> From<&'k $ty> for Reader<$ty> {
                #[inline]
                fn from(value: &'k $ty) -> Self {
                    Self::from(*value)
                }
            }

            impl<'k> From<&'k [u8]> for Reader<$ty> {
                #[inline]
                fn from(prefix: &'k [u8]) -> Self {
                    Self {
                        buffer: Native::from_be_bytes(prefix),
                        len: Bit(((prefix.len() << 3) as u8).min(<$ty as Native>::BITS)),
                    }
                }
            }

            impl<'k> From<&'k str> for Reader<$ty> {
                #[inline]
                fn from(prefix: &'k str) -> Self {
                    Self::from(prefix.as_bytes())
                }
            }

            impl<'k, const N: usize> From<&'k [u8; N]> for Reader<$ty> {
                #[inline]
                fn from(prefix: &'k [u8; N]) -> Self {
                    Self::from(prefix.as_slice())
                }
            }
        )*
    };
}

impl_key!(u16, u32, u128);

#[cfg(not(feature = "opt-no-int"))]
impl_key!(u64);

#[doc(hidden)]
#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Reader<N> {
    // NOTE: `buffer` is allowed to contain arbitrary bytes beyond
    // the most significant `len` bytes, but must clear them to
    // zero when (a) creating an edge to insert into the tree,
    // or (b) when creating a writer.
    pub(crate) buffer: N,
    len: Bit,
}

impl<N: Native> key::Read for Reader<N> {
    const LEN: Option<Self::Len> = Some(Bit(N::BITS));

    type Edge = edge::Be;
    type Len = Bit;

    #[inline]
    fn len(&self) -> Self::Len {
        self.len
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new(self.len.min(len.into()).0);
        edge::Be::new(self.buffer.most_significant_u64(), len)
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        (self.len > index.into()).then(|| self.buffer.get_u8(index.value()))
    }

    #[inline]
    unsafe fn get_byte_unchecked(&self, index: u6) -> u8 {
        self.buffer.get_u8(index.value())
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        Bit((edge.raw() ^ self.buffer.most_significant_u64()).leading_zeros() as u8)
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        validate!(end <= self.len());

        Self {
            buffer: self.buffer,
            len: end,
        }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());

        Self {
            buffer: self.buffer.unbounded_shl(start.0),
            len: self.len - start,
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let max = self.len.min(other.len).0;
        let len = Bit((self.buffer ^ other.buffer).leading_zeros().min(max) & !0b111);
        Self {
            buffer: self.buffer,
            len,
        }
    }
}

impl<N: Native> core::fmt::Debug for Reader<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let bytes = self.len().bytes();
        self.buffer
            .with_be_bytes(|buffer| f.debug_list().entries(&buffer[..bytes]).finish())
    }
}

#[doc(hidden)]
#[repr(transparent)]
#[derive(Default)]
pub struct Writer<N>(N);

impl<N: Native> key::Write<Reader<N>> for Writer<N> {
    type Len = Bit;

    #[inline]
    fn new(prefix: Reader<N>, edge: ribbit::Packed<edge::Be>) -> (Self, Self::Len) {
        let len = prefix.len() + edge.len().into();

        validate!(len.0 <= N::BITS);

        let writer = Self(
            prefix.buffer.most_significant(prefix.len.0)
                | N::from_most_significant_u64(edge.raw()).unbounded_shr(prefix.len.0),
        );

        (writer, len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Be>) -> Self::Len {
        self.0 = self.0.most_significant(start.0)
            | (N::from_u8(node) >> start.0)
            | (N::from_most_significant_u64(edge.raw()).unbounded_shr(8 + start.0));

        start + Bit::BYTE + edge.len().into()
    }
}

impl<N: Native> core::fmt::Debug for Writer<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0
            .with_be_bytes(|bytes| f.debug_list().entries(bytes).finish())
    }
}

/// Abstraction over unsigned native integer types.
pub(crate) trait Native:
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

macro_rules! impl_native {
    ($($ty:ty: $bits:expr, $into_u64:expr, $from_u64:expr, $into_u128:expr),* $(,)?) => {
        $(
            impl Native for $ty {
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

impl_native!(
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
