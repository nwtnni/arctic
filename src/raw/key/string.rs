//! Support for [`String`] keys ([`NonNullString`]).

use core::borrow::Borrow as _;
use core::num::NonZeroUsize;

use crate::NonNullVec;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::key::Byte;
use crate::raw::key::vec::Writer;

/// Newtype guaranteeing this [`String`] (a) is non-empty,
/// and (b) does not contain any internal null bytes.
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

pub type Reader<'k> = crate::raw::key::vec::Reader<'k, bool>;

impl Key for NonNullString {
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Borrowed = NonNullStr;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Byte;
    type Split = NonNullVec;

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

    fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as Key>::Read<'k>, u8) {
        todo!()
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
