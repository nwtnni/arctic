use crate::Key;
use crate::NullTerminatedSlice;
use crate::NullTerminatedVec;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl Key for &'_ NullTerminatedSlice {
    type Borrowed = NullTerminatedSlice;
    type Insert<'k>
        = Self
    where
        Self: 'k;
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Edge = edge::Slice;
    type Len = Byte;

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
            NullTerminatedSlice::new_unchecked(core::slice::from_raw_parts(
                suffix.as_ptr().byte_sub(len - suffix.len()),
                len,
            ))
        }
    }
}

/// Key reader that can represent byte prefixes of [`NullTerminatedSlice`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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

impl key::Read for Reader<'_> {
    const LEN: Option<Byte> = None;

    type Edge = edge::Slice;
    type Len = Byte;

    fn len(&self) -> Self::Len {
        Byte(self.0.len())
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = len.bytes().min(self.len().bytes());
        edge::Slice::new(&self.0[..len])
    }

    fn get_byte(&self, index: <ribbit::Packed<Self::Edge> as edge::Meta>::Len) -> Option<u8> {
        self.0.get(index.bytes()).copied()
    }

    fn match_prefix(&self, meta: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        let other = unsafe { meta.as_slice() };
        Byte(key::common_prefix(self.0, other))
    }

    #[inline]
    fn prefix(self, len: Byte) -> Self {
        validate!(self.len() >= len);
        Reader(&self.0[..len.bytes()])
    }

    #[inline]
    fn suffix(self, len: Byte) -> Self {
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
    len: Byte,
}

impl key::Write<Reader<'_>> for Writer {
    type Len = Byte;

    fn new(prefix: Reader, key: ribbit::Packed<edge::Slice>) -> (Self, Self::Len) {
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
