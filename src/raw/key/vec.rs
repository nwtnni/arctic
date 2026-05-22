use core::borrow::Borrow as _;
use core::ffi::CStr;
use core::fmt;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;
use std::ffi::CString;

use ribbit::u6;

use crate::NonPrefixSlice;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

/// Newtype guaranteeing this [`Vec`] is not a prefix of
/// any other [`NonPrefixVec`] or [`NonPrefixSlice`].
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonPrefixVec(Vec<u8>);

impl NonPrefixVec {
    /// # Safety
    ///
    /// Caller must guarantee that `vec` is not a prefix of any
    /// other `NonPrefixVec` or `NonPrefixSlice`.
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
    type Target = Vec<u8>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
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
    fn borrow_insert<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        Reader::from(insert)
    }

    #[inline]
    fn clone_insert<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k,
    {
        insert.to_non_prefix_vec()
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        unsafe { NonPrefixSlice::new_unchecked(&writer.0) }
    }
}

impl Key for CString {
    type Read<'k> = Reader<'k, { usize::MAX }>;
    type Write = Writer;
    type Borrowed = CStr;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self.borrow()
    }

    #[inline]
    fn borrow_insert<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        Reader::from(insert)
    }

    #[inline]
    fn clone_insert<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k,
    {
        insert.to_owned()
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        if_validate!(CStr::from_bytes_with_nul(&writer.0).unwrap(), unsafe {
            CStr::from_bytes_with_nul_unchecked(&writer.0)
        })
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

impl<'k> From<&'k CStr> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k CStr) -> Self {
        Self(key.to_bytes_with_nul())
    }
}

impl<'k> From<&'k CString> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k CString) -> Self {
        Self::from(key.as_c_str())
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
