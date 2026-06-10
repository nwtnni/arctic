//! Support for [`String`] keys ([`NonNullString`]).

use core::borrow::Borrow as _;
use core::fmt;
use core::num::NonZeroUsize;

use ribbit::u6;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

/// Newtype guaranteeing this [`String`] (a) is non-empty,
/// and (b) does not contain any internal null bytes.
///
/// Property (a) is required so our set implementation has a meaningful byte to index.
/// Property (b) is required so that we can internally use a null
/// byte as a terminator, to maintain the prefix tree
/// precondition that no key is a prefix of another key.
#[repr(transparent)]
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonNullString(String);

impl NonNullString {
    /// # Safety
    ///
    /// Caller must guarantee that this string does not contain any null bytes.
    #[inline]
    pub const unsafe fn new_unchecked(string: String) -> Self {
        Self(string)
    }

    /// Returns a `NonNullString` if `string` does not contain a null byte,
    /// or else returns the original string.
    #[inline]
    pub const fn new(string: String) -> Result<Self, String> {
        match NonNullStr::new(string.as_str()) {
            None => Err(string),
            Some(_) => Ok(Self(string)),
        }
    }

    /// Get a reference to this string.
    #[inline]
    pub const fn as_non_null_str(&self) -> &NonNullStr {
        // SAFETY: `self.0` is neither empty nor contains null bytes
        unsafe { NonNullStr::new_unchecked(self.0.as_str()) }
    }
}

impl From<NonNullString> for String {
    #[inline]
    fn from(NonNullString(string): NonNullString) -> Self {
        string
    }
}

impl core::ops::Deref for NonNullString {
    type Target = NonNullStr;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_non_null_str()
    }
}

impl core::borrow::Borrow<NonNullStr> for NonNullString {
    #[inline]
    fn borrow(&self) -> &NonNullStr {
        self.as_non_null_str()
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for NonNullString {
    type Parameters = proptest::string::StringParam;
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with(args: Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Strategy as _;
        String::arbitrary_with(args)
            .prop_filter_map("contains null bytes", |string| {
                NonNullString::new(string).ok()
            })
            .boxed()
    }
}

/// Newtype guaranteeing this [str][`core::primitive::str`] (a) is non-empty
/// and (b) does not contain any internal null bytes.
///
/// Also see [`NonNullString`].
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonNullStr(str);

impl NonNullStr {
    /// # Safety
    ///
    /// Caller must guarantee that this string is non-empty and does not contain any null bytes.
    #[inline]
    pub const unsafe fn new_unchecked(str: &str) -> &Self {
        // SAFETY: `NonNullStr` is `repr(transparent)`
        unsafe { core::mem::transmute(str) }
    }

    /// Returns a `NonNullStr` if `str` does not contain a null byte.
    #[inline]
    pub const fn new(str: &str) -> Option<&Self> {
        if str.is_empty() {
            return None;
        }

        // HACK: `core::primitive::str::contains` is not const
        let mut i = 0;
        let slice = str.as_bytes();
        while i < slice.len() {
            if slice[i] == 0 {
                return None;
            }
            i += 1;
        }

        // SAFETY: checked if `str` contains null byte
        Some(unsafe { Self::new_unchecked(str) })
    }

    /// Get an owned copy of this string.
    #[inline]
    pub fn to_non_null_string(&self) -> NonNullString {
        self.to_owned()
    }

    /// Return the length of this [`NonNullStr`] in bytes.
    #[inline]
    pub const fn len(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.0.len()).expect("NonNullStr is non-empty")
    }
}

impl std::borrow::ToOwned for NonNullStr {
    type Owned = NonNullString;
    #[inline]
    fn to_owned(&self) -> Self::Owned {
        NonNullString(self.0.to_string())
    }
}

impl core::ops::Deref for NonNullStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> From<&'a NonNullStr> for &'a str {
    #[inline]
    fn from(str: &'a NonNullStr) -> Self {
        // SAFETY: `NonNullStr` is `repr(transparent)`
        unsafe { core::mem::transmute(str) }
    }
}

impl Key for NonNullString {
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Borrowed = NonNullStr;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = key::vec::Len;

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
        insert.to_owned()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        let (last, key) = writer.0.split_last().expect("String has terminator");
        validate_eq!(*last, 0);

        if_validate!(
            core::str::from_utf8(key)
                .ok()
                .and_then(NonNullStr::new)
                .unwrap(),
            unsafe { NonNullStr::new_unchecked(str::from_utf8_unchecked(key)) }
        )
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
    /// it may be empty, or contain null bytes, or be invalid UTF-8.
    #[inline]
    pub const fn new_prefix(prefix: &'k [u8]) -> Self {
        Self {
            slice: prefix,
            terminate: false,
        }
    }
}

impl<'k> From<&'k NonNullStr> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullStr) -> Self {
        Self {
            slice: key.as_bytes(),
            terminate: true,
        }
    }
}

impl<'k> From<&'k NonNullString> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullString) -> Self {
        Self::from(key.as_non_null_str())
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
    type Len = key::vec::Len;

    #[inline]
    fn len(&self) -> Self::Len {
        key::vec::Len(self.slice.len() + self.terminate as usize)
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
        key::vec::Len(((edge.raw() ^ key::read_u64(self.slice)).trailing_zeros() as usize) >> 3)
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
    type Len = key::vec::Len;

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
        key::vec::Len(self.0.len())
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
