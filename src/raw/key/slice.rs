//! Support for `&[u8]` keys (`NonPrefixSlice`).

use core::num::NonZeroUsize;

use ribbit::u14;

use crate::NonPrefixVec;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::edge::Slice;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

/// Newtype guaranteeing this slice (a) is non-empty,
/// and (b) is not a prefix of any other [`NonPrefixVec`] or [`NonPrefixSlice`].
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonPrefixSlice([u8]);

impl NonPrefixSlice {
    /// # Safety
    ///
    /// Caller must guarantee that `slice` is neither empty, nor a prefix of any
    /// other [`NonPrefixVec`] or [`NonPrefixSlice`].
    #[inline]
    pub const unsafe fn new_unchecked(slice: &[u8]) -> &Self {
        // SAFETY: `NonPrefixSlice` is `repr(transparent)`
        unsafe { core::mem::transmute(slice) }
    }

    #[inline]
    pub fn to_non_prefix_vec(&self) -> NonPrefixVec {
        unsafe { NonPrefixVec::new_unchecked(self.0.to_owned()) }
    }

    #[inline]
    pub const fn len(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.0.len()).expect("NonPrefixSlice is non-empty")
    }
}

impl std::borrow::ToOwned for NonPrefixSlice {
    type Owned = NonPrefixVec;
    #[inline]
    fn to_owned(&self) -> Self::Owned {
        self.to_non_prefix_vec()
    }
}

impl core::ops::Deref for NonPrefixSlice {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> From<&'a NonPrefixSlice> for &'a [u8] {
    #[inline]
    fn from(str: &'a NonPrefixSlice) -> Self {
        // SAFETY: `NonPrefixSlice` is `repr(transparent)`
        unsafe { core::mem::transmute(str) }
    }
}

impl Key for &'_ NonPrefixSlice {
    type Borrowed = NonPrefixSlice;
    type Insert<'k>
        = Self
    where
        Self: 'k;
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Edge = edge::Slice;
    type Len = key::vec::Len;

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

    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        let len = writer.len.bytes();
        let suffix = unsafe { writer.last.as_slice() };
        unsafe {
            NonPrefixSlice::new_unchecked(core::slice::from_raw_parts(
                suffix.as_ptr().byte_sub(len - suffix.len()),
                len,
            ))
        }
    }
}

/// Key reader that can represent byte prefixes of [`NonPrefixSlice`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader<'k>(pub(crate) &'k [u8]);

impl<'k> Reader<'k> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties:
    /// it may be empty, or be a prefix of another key.
    #[inline]
    pub const fn new_prefix(prefix: &'k [u8]) -> Self {
        Self(prefix)
    }
}

impl<'k> From<&'k NonPrefixSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        Self(key)
    }
}

impl key::Read for Reader<'_> {
    const LEN: Option<key::vec::Len> = None;

    type Edge = edge::Slice;
    type Len = key::vec::Len;

    fn len(&self) -> Self::Len {
        key::vec::Len(self.0.len())
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = len.bytes().min(self.len().bytes());
        Slice::new(&self.0[..len])
    }

    fn get_byte(&self, index: <ribbit::Packed<Self::Edge> as edge::Meta>::Len) -> Option<u8> {
        self.0.get(index.bytes()).copied()
    }

    fn match_prefix(&self, meta: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        let other = unsafe { meta.as_slice() };
        key::vec::Len(key::common_prefix(self.0, other))
    }

    #[inline]
    fn prefix(self, len: key::vec::Len) -> Self {
        validate!(self.len() >= len);
        Reader(&self.0[..len.bytes()])
    }

    #[inline]
    fn suffix(self, len: key::vec::Len) -> Self {
        validate!(self.len() >= len);
        Reader(&self.0[len.bytes()..])
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = key::common_prefix(self.0, other.0);
        Self(&self.0[..index])
    }

    fn split_last(self) -> Option<(Self, u8)> {
        let (byte, slice) = self.0.split_last()?;
        Some((Reader(slice), *byte))
    }
}

#[doc(hidden)]
#[derive(Clone, Default, Debug)]
pub struct Writer {
    last: ribbit::Packed<edge::Slice>,
    len: key::vec::Len,
}

impl key::Write<Reader<'_>> for Writer {
    type Len = key::vec::Len;

    fn new(prefix: Reader, key: ribbit::Packed<edge::Slice>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        (Writer { last: key, len }, len)
    }

    fn replace(&mut self, start: Self::Len, _: u8, edge: ribbit::Packed<edge::Slice>) -> Self::Len {
        validate!(start <= self.len);
        self.len = start + key::vec::Len::BYTE + edge.len().into();
        self.last = edge;
        self.len
    }
}

impl From<u14> for key::vec::Len {
    fn from(value: u14) -> Self {
        Self(value.value() as usize)
    }
}
