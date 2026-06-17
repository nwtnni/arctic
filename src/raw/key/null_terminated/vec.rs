use core::borrow::Borrow as _;
use core::fmt;

use ribbit::u6;

use crate::NullTerminatedSlice;
use crate::NullTerminatedVec;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl Key for NullTerminatedVec {
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Borrowed = NullTerminatedSlice;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Byte;

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
        insert.to_null_terminated_vec()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        unsafe { NullTerminatedSlice::new_unchecked(&writer.0) }
    }
}

/// Key reader that can represent byte prefixes of [`NullTerminatedSlice`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Reader<'k>(pub(crate) &'k [u8]);

impl<'k> Reader<'k> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties:
    /// it may be empty or contain arbitrary null bytes.
    #[inline]
    pub const fn new_prefix(prefix: &'k [u8]) -> Self {
        Self(prefix)
    }
}

impl<'k> From<&'k NullTerminatedVec> for Reader<'k> {
    #[inline]
    fn from(key: &'k NullTerminatedVec) -> Self {
        Self(key)
    }
}

impl<'k> From<&'k NullTerminatedSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NullTerminatedSlice) -> Self {
        Self(key)
    }
}

impl Default for Reader<'_> {
    #[inline]
    fn default() -> Self {
        Self(&[])
    }
}

impl key::Read for Reader<'_> {
    const LEN: Option<Self::Len> = None;
    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn len(&self) -> Self::Len {
        Byte(self.0.len())
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(key::read_u64(self.0), len)
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
        let len_match = (edge.raw() ^ key::read_u64(self.0)).trailing_zeros() as u8;
        (len_match >= len_edge.value()).then_some(len_edge)
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        Byte(((edge.raw() ^ key::read_u64(self.0)).trailing_zeros() as usize) >> 3)
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
        let index = key::common_prefix(self.0, other.0);
        Self(&self.0[..index])
    }

    fn split_last(self) -> Option<(Self, u8)> {
        let (byte, slice) = self.0.split_last()?;
        Some((Reader(slice), *byte))
    }
}

#[doc(hidden)]
#[repr(transparent)]
#[derive(Default)]
pub struct Writer(pub(super) Vec<u8>);

impl<'k> key::Write<Reader<'k>> for Writer {
    type Len = Byte;

    #[inline]
    fn new(prefix: Reader<'k>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
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
        Byte(self.0.len())
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
