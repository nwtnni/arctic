use core::fmt;

use ribbit::u6;

use crate::Key;
use crate::NonNullSlice;
use crate::NonNullVec;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl Key for NonNullVec {
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Borrowed = NonNullSlice;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self.as_non_null_slice()
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
        insert.to_non_null_vec()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        let (last, key) = writer.0.split_last().expect("Implicit null terminator");
        validate_eq!(*last, 0);

        if_validate!(NonNullSlice::new(key).unwrap(), unsafe {
            NonNullSlice::new_unchecked(key)
        })
    }
}

/// Key reader that can represent byte slices of [`NonNullStr`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Reader<'k> {
    pub(crate) slice: &'k [u8],
    terminate: bool,
}

impl<'k> Reader<'k> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties:
    /// it may be empty, or contain arbitrary null bytes.
    #[inline]
    pub const fn new_prefix(prefix: &'k [u8]) -> Self {
        Self {
            slice: prefix,
            terminate: false,
        }
    }
}

impl<'k> From<&'k NonNullVec> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullVec) -> Self {
        Self::from(key.as_non_null_slice())
    }
}

impl<'k> From<&'k NonNullSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullSlice) -> Self {
        Self {
            slice: key.as_slice(),
            terminate: true,
        }
    }
}

impl Default for Reader<'_> {
    #[inline]
    fn default() -> Self {
        Self::new_prefix(&[])
    }
}

impl key::Read for Reader<'_> {
    const LEN: Option<Self::Len> = None;

    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn len(&self) -> Self::Len {
        Byte(self.slice.len() + self.terminate as usize)
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(key::read_u64(self.slice), len)
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        let index = index.bytes();

        if let Some(byte) = self.slice.get(index) {
            return Some(*byte);
        }

        (index == self.slice.len() && self.terminate).then_some(0)
    }

    #[inline]
    fn match_exact(
        &self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<ribbit::Packed<Self::Edge> as edge::Meta>::Len> {
        // Avoid bit <-> byte conversion
        let len_edge = edge.len();
        let len_match = (edge.raw() ^ key::read_u64(self.slice)).trailing_zeros() as u8;
        (len_match >= len_edge.value()).then_some(len_edge)
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        Byte(((edge.raw() ^ key::read_u64(self.slice)).trailing_zeros() as usize) >> 3)
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        validate!(end <= self.len());
        let end = end.bytes();

        Self {
            slice: self.slice.get(..end).unwrap_or(self.slice),
            terminate: (end > self.slice.len()) && self.terminate,
        }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());
        let start = start.bytes();

        Self {
            slice: self.slice.get(start..).unwrap_or_default(),
            terminate: (start <= self.slice.len()) && self.terminate,
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        // Only case where terminator is preserved
        if self == other {
            return self;
        }

        let index = key::common_prefix(self.slice, other.slice);
        Self {
            slice: &self.slice[..index],
            terminate: false,
        }
    }

    fn split_last(self) -> Option<(Self, u8)> {
        let (byte, slice) = self.slice.split_last()?;
        Some((
            Reader {
                slice,
                terminate: self.terminate,
            },
            *byte,
        ))
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
        buffer.extend_from_slice(prefix.slice);
        if prefix.terminate {
            buffer.push(u8::MIN);
            validate_eq!(key.len().bits(), 0);
        } else {
            buffer.extend(key);
        }
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
