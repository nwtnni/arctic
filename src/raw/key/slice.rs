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
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::Terminate;

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

    /// Get an owned copy of this slice.
    #[inline]
    pub fn to_non_prefix_vec(&self) -> NonPrefixVec {
        unsafe { NonPrefixVec::new_unchecked(self.0.to_owned()) }
    }

    /// Return the length of this [`NonPrefixSlice`] in bytes.
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

impl<'a> Key for &'a NonPrefixSlice {
    type Borrowed = NonPrefixSlice;
    type Insert<'k>
        = Self
    where
        Self: 'k;
    type Read<'k> = Reader<'k, ()>;
    type Write = Writer;
    type Edge = edge::Slice;
    type Len = Byte;
    type Split = &'a NonPrefixSlice;

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
        unsafe { NonPrefixSlice::new_unchecked(writer.as_slice_unchecked()) }
    }

    fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as Key>::Read<'k>, u8) {
        todo!()
    }
}

impl<'k> From<&'k NonPrefixSlice> for Reader<'k, ()> {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        Self(crate::raw::key::vec::Reader {
            slice: key,
            terminate: (),
        })
    }
}

/// Key reader that can represent byte prefixes of [`NonPrefixSlice`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Reader<'k, T>(pub(crate) crate::raw::key::vec::Reader<'k, T>);

impl<'k, T: Default> Reader<'k, T> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties:
    /// it may be empty, or be a prefix of another key.
    #[inline]
    pub fn new_prefix(prefix: &'k [u8]) -> Self {
        Self(crate::raw::key::vec::Reader::new_prefix(prefix))
    }
}

impl<T: Terminate> key::Read for Reader<'_, T> {
    const LEN: Option<Byte> = None;

    type Edge = edge::Slice;
    type Len = Byte;

    fn len(&self) -> Self::Len {
        self.0.len()
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = len.bytes().min(self.len().bytes());
        Slice::new(&self.0.slice[..len])
    }

    fn get_byte(&self, index: u14) -> Option<u8> {
        self.0.get_byte(index.bytes())
    }

    fn match_prefix(&self, meta: ribbit::Packed<edge::Slice>) -> Self::Len {
        let other = unsafe { meta.as_slice() };
        Byte(key::common_prefix(self.0.slice, other))
    }

    #[inline]
    fn prefix(self, end: Byte) -> Self {
        Self(self.0.prefix(end))
    }

    #[inline]
    fn suffix(self, start: Byte) -> Self {
        Self(self.0.suffix(start))
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        Self(self.0.common_prefix(other.0))
    }
}

#[doc(hidden)]
#[derive(Clone, Default, Debug)]
pub struct Writer {
    last: ribbit::Packed<edge::Slice>,
    len: Byte,
}

impl Writer {
    pub(super) unsafe fn as_slice_unchecked<'a>(&self) -> &'a [u8] {
        let len = self.len.bytes();
        let suffix = unsafe { self.last.as_slice() };
        unsafe { core::slice::from_raw_parts(suffix.as_ptr().byte_sub(len - suffix.len()), len) }
    }
}

impl<T: Terminate> key::Write<Reader<'_, T>> for Writer {
    type Len = Byte;

    fn new(prefix: Reader<'_, T>, key: ribbit::Packed<edge::Slice>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        (Writer { last: key, len }, len)
    }

    fn replace(&mut self, start: Self::Len, _: u8, edge: ribbit::Packed<edge::Slice>) -> Self::Len {
        validate!(start <= self.len);
        self.len = start + Byte::BYTE + edge.len().into();
        self.last = edge;
        self.len
    }
}
