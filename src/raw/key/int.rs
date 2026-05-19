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

                type Edge = edge::Be;
                type Len = Len;

                #[inline]
                fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
                    *borrow
                }

                #[inline]
                unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed {
                    &writer.0
                }

                #[inline]
                unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
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
            buffer: self.buffer.most_significant(len.0),
            len,
        }
    }

    #[inline]
    fn expand(
        &self,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Result<
        (
            ribbit::Packed<Self::Edge>,
            u8,
            u8,
            ribbit::Packed<Self::Edge>,
        ),
        (),
    > {
        let len_match = self.match_prefix(edge);
        if len_match >= edge.len().into() {
            return Err(());
        }

        validate!(self.len >= len_match);
        let len_start = u6::new(len_match.0 & !0b111);
        let len_middle = len_start + const { u6::new(8) };

        let start = edge::Be::new(edge.raw(), len_start);
        let old_middle = edge.raw().get_u8(len_start.value());
        let new_middle = self.buffer.get_u8(len_start.value());
        let end = edge::Be::new(edge.raw() << len_middle.value(), edge.len() - len_middle);

        Ok((start, old_middle, new_middle, end))
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
            prefix.buffer | I::from_most_significant_u64(edge.raw()).unbounded_shr(prefix.len.0),
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

impl key::Len<u6> for Len {
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

// #[cfg(test)]
// mod tests {
//     use crate::raw::key::tests::take_all;
//
//     #[test]
//     fn smoke() {
//         take_all_u64(0x1234_5678_9ABC_DEF0u64, &[7, 1]);
//     }
//
//     #[test]
//     fn take_0() {
//         take_all_u64(0x1234_5678_9ABC_DEF0u64, &[0, 1, 0]);
//     }
//
//     #[test]
//     fn take_1() {
//         take_all_u64(0x1234_5678_9ABC_DEF0u64, &[1, 1, 1, 1, 1, 1, 1, 1]);
//     }
//
//     fn take_all_u64(key: u64, lens: &[usize]) {
//         take_all::<u64>(key.to_be_bytes().as_slice(), &key, lens)
//     }
// }
