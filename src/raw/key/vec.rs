use core::fmt;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use ribbit::u6;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl Key for Vec<u8> {
    type Read<'k> = Reader<'k, { usize::MAX }>;
    type Write = Writer;
    type Borrowed = [u8];
    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
        Vec::from(borrow)
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

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Reader<'k, const N: usize>(pub(crate) &'k [u8]);

impl<'k, const N: usize> Reader<'k, N> {
    #[inline]
    fn next_u64(&self) -> u64 {
        if self.0.len() >= 8 {
            return unsafe { self.0.as_ptr().cast::<u64>().read_unaligned() };
        }

        // FIXME: try to avoid memcpy?
        // https://github.com/llvm/llvm-project/issues/87440
        // https://github.com/rust-lang/rust/issues/92993
        // https://github.com/rust-lang/rust/pull/37573
        let mut buffer = [0u8; 8];
        buffer[..self.0.len()].copy_from_slice(self.0);

        u64::from_le_bytes(buffer)
    }
}

impl<'k> From<&'k [u8]> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k [u8]) -> Self {
        Self(key)
    }
}

impl<'k> From<&'k Vec<u8>> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k Vec<u8>) -> Self {
        Self::from(key.as_slice())
    }
}

impl<const N: usize> Default for Reader<'_, N> {
    #[inline]
    fn default() -> Self {
        Self(&[])
    }
}

impl<const N: usize> key::Read for Reader<'_, N> {
    const LEN: Option<Self::Len> = if N == usize::MAX { None } else { Some(Len(N)) };

    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn len(&self) -> Self::Len {
        Len(self.0.len())
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(self.next_u64(), len)
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        self.0.get(index.bytes()).copied()
    }

    #[inline]
    fn match_exact(
        &self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<ribbit::Packed<Self::Edge> as edge::Meta>::Len> {
        // Avoid bit <-> byte conversion
        let len_edge = edge.len();
        let len_match = (edge.raw() ^ self.next_u64()).trailing_zeros() as u8;
        (len_match >= len_edge.value()).then_some(len_edge)
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        Len(((edge.raw() ^ self.next_u64()).trailing_zeros() as usize) >> 3)
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        Self(&self.0[..end.bytes()])
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());
        Self(&self.0[start.bytes()..])
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = core::iter::zip(self.0, other.0)
            .position(|(l, r)| l != r)
            .unwrap_or_else(|| self.0.len().min(other.0.len()));
        Self(&self.0[..index])
    }

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
        let buffer = self.next_u64();

        let len_match = (edge.raw() ^ buffer).trailing_zeros() as u8;
        if len_match >= edge.len().value() {
            return Err(());
        }

        validate!(self.len().bits() > len_match as usize);

        let len_start = u6::new(len_match & !0b111);
        let len_middle = len_start + const { u6::new(8) };

        let start = edge::Le::new(edge.raw(), len_start);
        let old_middle = (edge.raw() >> len_start.value()) as u8;
        let new_middle = (buffer >> len_start.value()) as u8;
        let end = edge::Le::new(edge.raw() >> len_middle.value(), edge.len() - len_middle);

        Ok((start, old_middle, new_middle, end))
    }
}

#[repr(transparent)]
#[derive(Default)]
pub struct Writer(pub(super) Vec<u8>);

impl<'k> key::Write<Reader<'k, { usize::MAX }>> for Writer {
    type Len = Len;

    #[inline]
    fn new(prefix: Reader<'k, { usize::MAX }>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(prefix.0);
        buffer.extend(key);
        (Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Le>) -> Self::Len {
        validate!(start.0 <= self.0.len());
        self.0.truncate(start.0);
        self.0.push(node);
        self.0.extend(edge);
        Len(self.0.len())
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Len(pub(super) usize);

impl key::Len<u6> for Len {
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

impl From<u6> for Len {
    #[inline]
    fn from(len: u6) -> Self {
        Self((len.value() >> 3) as usize)
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
