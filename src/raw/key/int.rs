//! Support for integer keys (`u16`, `u32`, `u64`, `u128`).

use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use ribbit::u6;

use crate::raw::Int;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
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
                type Len = Len;

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

            impl From<$ty> for Reader<$ty> {
                #[inline]
                fn from(value: $ty) -> Self {
                    Self {
                        buffer: value,
                        len: Len(<$ty as Int>::BITS),
                    }
                }
            }

            impl<'k> From<&'k $ty> for Reader<$ty> {
                #[inline]
                fn from(value: &'k $ty) -> Self {
                    Self::from(*value)
                }
            }
        )*
    };
}

impl_key!(u16, u32, u128);

#[cfg(not(feature = "opt-no-int"))]
impl_key!(u64);

/// NOTE: `buffer` is allowed to contain arbitrary bytes beyond
/// the most significant `len` bytes, but must clear them to
/// zero when (a) creating an edge to insert into the tree,
/// or (b) when creating a writer.
#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Reader<I> {
    pub(crate) buffer: I,
    len: Len,
}

impl<I: Int> key::Read for Reader<I> {
    const LEN: Option<Self::Len> = Some(Len(I::BITS));

    type Edge = edge::Be;
    type Len = Len;

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
        Len((edge.raw() ^ self.buffer.most_significant_u64()).leading_zeros() as u8)
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
        let len = Len((self.buffer ^ other.buffer).leading_zeros().min(max) & !0b111);
        Self {
            buffer: self.buffer,
            len,
        }
    }

    #[inline]
    fn split_last(self) -> Option<(Self, u8)> {
        Some((
            Self {
                buffer: self.buffer,
                len: self.len.0.checked_sub(Self::Len::BYTE.0).map(Len)?,
            },
            self.buffer.least_significant_u8(),
        ))
    }
}

impl<I: Int> core::fmt::Debug for Reader<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let bytes = self.len().bytes();
        self.buffer
            .with_be_bytes(|buffer| f.debug_list().entries(&buffer[..bytes]).finish())
    }
}

#[repr(transparent)]
#[derive(Default)]
pub struct Writer<I>(I);

impl<I: Int> key::Write<Reader<I>> for Writer<I> {
    type Len = Len;

    #[inline]
    fn new(prefix: Reader<I>, edge: ribbit::Packed<edge::Be>) -> (Self, Self::Len) {
        let len = prefix.len() + edge.len().into();

        validate!(len.0 <= I::BITS);

        let writer = Self(
            prefix.buffer.most_significant(prefix.len.0)
                | I::from_most_significant_u64(edge.raw()).unbounded_shr(prefix.len.0),
        );

        (writer, len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Be>) -> Self::Len {
        self.0 = self.0.most_significant(start.0)
            | (I::from_u8(node) >> start.0)
            | (I::from_most_significant_u64(edge.raw()).unbounded_shr(8 + start.0));

        start + Len::BYTE + edge.len().into()
    }
}

impl<I: Int> core::fmt::Debug for Writer<I> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0
            .with_be_bytes(|bytes| f.debug_list().entries(bytes).finish())
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Len(u8);

impl From<u6> for Len {
    #[inline]
    fn from(len: u6) -> Self {
        Self(len.value())
    }
}

impl From<Len> for u6 {
    #[inline]
    fn from(len: Len) -> Self {
        const MASK: u8 = 0b0011_1000;
        unsafe { u6::new_unchecked(len.0 & MASK) }
    }
}

impl key::Len for Len {
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

impl Add for Len {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Len {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Len {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for Len {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}
