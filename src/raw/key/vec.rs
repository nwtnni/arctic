//! Support for [`Vec<u8>`] keys ([`NonPrefixVec`]).

use core::borrow::Borrow as _;

use ribbit::u6;

use crate::NonPrefixSlice;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::Terminate;

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
    type Read<'k> = Reader<'k, ()>;
    type Write = Writer;
    type Borrowed = NonPrefixSlice;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Byte;
    type Split = NonPrefixVec;

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

    fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as Key>::Read<'k>, u8) {
        todo!()
    }
}

impl<'k> From<&'k NonPrefixVec> for Reader<'k, ()> {
    #[inline]
    fn from(key: &'k NonPrefixVec) -> Self {
        Self::from(key.as_non_prefix_slice())
    }
}

impl<'k> From<&'k NonPrefixSlice> for Reader<'k, ()> {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        Self {
            slice: key,
            terminate: (),
        }
    }
}

/// Key reader that can represent byte prefixes.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Reader<'k, T> {
    pub(crate) slice: &'k [u8],
    pub(super) terminate: T,
}

impl<'k, T: Default> Reader<'k, T> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties:
    /// it may be empty, or be a prefix of another key.
    #[inline]
    pub fn new_prefix(prefix: &'k [u8]) -> Self {
        Self {
            slice: prefix,
            terminate: T::default(),
        }
    }
}

impl<'k, T: Terminate> Reader<'k, T> {
    pub(super) fn split_last(&self) -> Option<(Self, u8)> {
        let (byte, slice) = self.slice.split_last()?;
        Some((
            Self {
                slice,
                terminate: self.terminate,
            },
            *byte,
        ))
    }

    pub(super) fn get_byte(&self, index: usize) -> Option<u8> {
        if let Some(byte) = self.slice.get(index) {
            return Some(*byte);
        }

        (self.terminate.get() && index == self.slice.len()).then_some(0)
    }
}

impl<'k> Reader<'k, ()> {
    pub(super) fn split_second_last(&self) -> Option<(Reader<'k, bool>, u8)> {
        let (slice, last) = self.slice.split_last_chunk::<2>()?;
        validate_eq!(last[1], 0);
        Some((
            Reader {
                slice,
                terminate: true,
            },
            last[0],
        ))
    }
}

impl<'k> Reader<'k, bool> {}

impl<T: Default> Default for Reader<'_, T> {
    #[inline]
    fn default() -> Self {
        Self::new_prefix(&[])
    }
}

impl<T: Terminate> key::Read for Reader<'_, T> {
    const LEN: Option<Self::Len> = None;
    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn len(&self) -> Self::Len {
        Byte(self.slice.len() + self.terminate.get() as usize)
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
        self.get_byte(index.bytes())
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
            terminate: T::new(self.terminate.get() && (end > self.slice.len())),
        }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());
        let start = start.bytes();

        Self {
            slice: self.slice.get(start..).unwrap_or_default(),
            terminate: T::new(self.terminate.get() && (start <= self.slice.len())),
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = key::common_prefix(self.slice, other.slice);

        Self {
            slice: &self.slice[..index],
            terminate: T::new(
                self.terminate.get()
                    && other.terminate.get()
                    && index == self.slice.len()
                    && index == other.slice.len(),
            ),
        }
    }
}

#[doc(hidden)]
#[repr(transparent)]
#[derive(Debug, Default)]
pub struct Writer(pub(super) Vec<u8>);

impl<'k, T: Terminate> key::Write<Reader<'k, T>> for Writer {
    type Len = Byte;

    #[inline]
    fn new(prefix: Reader<'k, T>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(prefix.slice);
        if prefix.terminate.get() {
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
