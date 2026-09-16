//! Support for unsigned integer keys.

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::len::Bit;
use crate::raw::key::len::Byte;
use crate::sync::Convert as _;

macro_rules! impl_key {
    ($($ty:ty, $key:expr, $edge:expr);* $(,)?) => {
        $(
            impl Key for $ty {
                type Read<'k> = Reader<$key, $edge, $ty>;
                type Write = Writer<$ty>;
                type Borrowed = Self;
                type Insert<'k> = Self;

                type Edge = edge::Be<$edge>;
                type Len = Bit<$key>;

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
                            len: reader.len - Self::Len::BYTE,
                        },
                        reader.buffer.least_significant_u8(),
                    )
                }
            }

            impl From<$ty> for Reader<$key, $edge, $ty> {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self {
                        buffer: value,
                        len: unsafe { Bit::new_unchecked(<$ty as Native>::BITS) },
                    }
                }
            }

            impl<'k> From<&'k $ty> for Reader<$key, $edge, $ty> {
                #[inline]
                fn from(value: &'k $ty) -> Self {
                    Self::from(*value)
                }
            }

            impl<'k> From<&'k [u8]> for Reader<$key, $edge, $ty> {
                #[inline]
                fn from(prefix: &'k [u8]) -> Self {
                    Self {
                        buffer: Native::from_be_bytes(prefix),
                        len:  Byte::new(prefix.len()).into() ,
                    }
                }
            }

            impl<'k> From<&'k str> for Reader<$key, $edge, $ty> {
                #[inline]
                fn from(prefix: &'k str) -> Self {
                    Self::from(prefix.as_bytes())
                }
            }

            impl<'k, const N: usize> From<&'k [u8; N]> for Reader<$key, $edge, $ty> {
                #[inline]
                fn from(prefix: &'k [u8; N]) -> Self {
                    Self::from(prefix.as_slice())
                }
            }
        )*
    };
}

impl_key!(
    u16, 16, 16;
    u32, 32, 32;
    u128, 128, 56
);

#[cfg(not(feature = "opt-no-int"))]
impl_key!(u64, 64, 56);

#[doc(hidden)]
#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Reader<const KEY: u8, const EDGE: u8, N> {
    // NOTE: `buffer` is allowed to contain arbitrary bytes beyond
    // the most significant `len` bytes, but must clear them to
    // zero when (a) creating an edge to insert into the tree,
    // or (b) when creating a writer.
    pub(crate) buffer: N,
    len: Bit<KEY>,
}

impl<const KEY: u8, const EDGE: u8, N: Native> key::Read for Reader<KEY, EDGE, N>
where
    Bit<KEY>: From<Bit<EDGE>>,
    Bit<EDGE>: From<Bit<KEY>>,
{
    const LEN: Option<Self::Len> = Some(Bit::<KEY>::MAX);

    type Edge = edge::Be<EDGE>;
    type Len = Bit<KEY>;

    #[inline]
    fn len(&self) -> Self::Len {
        self.len
    }

    #[inline]
    fn get_edge(&self, len: <Self::Edge as edge::Meta>::Len) -> Self::Edge {
        let len = Self::Len::min::<EDGE, KEY>(len, self.len);
        edge::Be::new(self.buffer.most_significant_u64(), len)
    }

    #[inline]
    fn get_byte(&self, index: <Self::Edge as edge::Meta>::Len) -> Option<u8> {
        (self.len > index.into()).then(|| self.buffer.get_u8(index.into_u8()))
    }

    #[inline]
    unsafe fn get_byte_unchecked(&self, index: <Self::Edge as edge::Meta>::Len) -> u8 {
        self.buffer.get_u8(index.into_u8())
    }

    #[inline]
    fn match_exact(&self, edge: Self::Edge) -> Option<<Self::Edge as edge::Meta>::Len> {
        let len_match =
            (edge.into_raw() ^ self.buffer.most_significant_u64()).leading_zeros() as u8;
        let len_edge = edge.len();
        (len_match >= len_edge.into_u8()).then_some(len_edge)
    }

    #[inline]
    fn match_prefix(&self, edge: Self::Edge) -> <Self::Edge as edge::Meta>::Len {
        let len_match = (edge.into_raw() ^ self.buffer.most_significant_u64()
            // HACK: branchless clamp to `Self::Edge::Len::MAX`
            | const { 1u64.rotate_right(<Self::Edge as edge::Meta>::Len::MAX.into_u8() as u32 + 1) })
        .leading_zeros() as u8;

        unsafe { Bit::new_unchecked(len_match) }
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
            buffer: self.buffer.unbounded_shl(start.into_u8()),
            len: self.len - start,
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let max = self.len.min(other.len).into_u8();
        let len = Bit::new_masked((self.buffer ^ other.buffer).leading_zeros().min(max));
        Self {
            buffer: self.buffer,
            len,
        }
    }
}

impl<const KEY: u8, const EDGE: u8, N: Native> core::fmt::Debug for Reader<KEY, EDGE, N>
where
    Bit<KEY>: From<Bit<EDGE>>,
    Bit<EDGE>: From<Bit<KEY>>,
{
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

impl<const KEY: u8, const EDGE: u8, N: Native> key::Write<Reader<KEY, EDGE, N>> for Writer<N>
where
    Bit<KEY>: From<Bit<EDGE>>,
    Bit<EDGE>: From<Bit<KEY>>,
{
    type Len = Bit<KEY>;

    #[inline]
    fn new(prefix: Reader<KEY, EDGE, N>, edge: edge::Be<EDGE>) -> (Self, Self::Len) {
        let len = prefix.len() + edge.len().into();

        validate!(len.into_u8() <= N::BITS);

        let writer = Self(
            prefix.buffer.most_significant(prefix.len.into_u8())
                | N::from_most_significant_u64(edge.into_raw()).unbounded_shr(prefix.len.into_u8()),
        );

        (writer, len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: edge::Be<EDGE>) -> Self::Len {
        self.0 = self.0.most_significant(start.into_u8())
            | (N::from_u8(node) >> start.into_u8())
            | (N::from_most_significant_u64(edge.into_raw()).unbounded_shr(8 + start.into_u8()));

        start + Bit::<KEY>::BYTE + edge.len().into()
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
                const BITS: u8 = $bits;

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
