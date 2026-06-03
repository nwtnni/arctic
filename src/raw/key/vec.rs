use core::borrow::Borrow as _;
use core::fmt;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use ribbit::traits::Integer as _;
use ribbit::u6;
use ribbit::u14;

use crate::NonPrefixSlice;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

/// Newtype guaranteeing this [`Vec`] (a) is not empty, and (b) is not a prefix of
/// any other [`NonPrefixVec`] or [`NonPrefixSlice`].
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonPrefixVec(Vec<u8>);

impl NonPrefixVec {
    /// # Safety
    ///
    /// Caller must guarantee that `vec` is neither empty, nor a prefix of any
    /// other [`NonPrefixVec`] or [`NonPrefixSlice`].
    pub const unsafe fn new_unchecked(vec: Vec<u8>) -> Self {
        Self(vec)
    }

    #[inline]
    pub const fn as_non_prefix_slice(&self) -> &NonPrefixSlice {
        // SAFETY: `self.0` is not a prefix
        unsafe { NonPrefixSlice::new_unchecked(self.0.as_slice()) }
    }
}

impl From<NonPrefixVec> for Vec<u8> {
    #[inline]
    fn from(NonPrefixVec(vec): NonPrefixVec) -> Self {
        vec
    }
}

impl core::ops::Deref for NonPrefixVec {
    type Target = NonPrefixSlice;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_non_prefix_slice()
    }
}

impl core::borrow::Borrow<NonPrefixSlice> for NonPrefixVec {
    #[inline]
    fn borrow(&self) -> &NonPrefixSlice {
        self.as_non_prefix_slice()
    }
}

impl Key for NonPrefixVec {
    type Read<'k> = Reader<'k, { usize::MAX }>;
    type Write = Writer;
    type Borrowed = NonPrefixSlice;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self.borrow()
    }

    #[inline]
    fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        Reader::from(insert)
    }

    #[inline]
    fn insert_to_key<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k,
    {
        insert.to_non_prefix_vec()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        unsafe { NonPrefixSlice::new_unchecked(&writer.0) }
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

impl<'k> From<&'k NonPrefixSlice> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        Self(key)
    }
}

impl<'k> From<&'k NonPrefixVec> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k NonPrefixVec) -> Self {
        Self::from(key.as_non_prefix_slice())
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

impl key::Len for Len {
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

impl From<Len> for u6 {
    #[inline]
    fn from(len: Len) -> Self {
        u6::masked_new((len.0 << 3) as u8)
    }
}

impl From<Len> for u14 {
    #[inline]
    fn from(len: Len) -> Self {
        u14::masked_new(len.0 as u16)
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
