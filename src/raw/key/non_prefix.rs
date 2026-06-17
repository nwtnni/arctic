use core::num::NonZeroUsize;
use core::ops::Deref;
use std::borrow::ToOwned;

pub mod slice;
pub mod vec;

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

impl Deref for NonPrefixVec {
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

impl ToOwned for NonPrefixSlice {
    type Owned = NonPrefixVec;
    #[inline]
    fn to_owned(&self) -> Self::Owned {
        self.to_non_prefix_vec()
    }
}

impl Deref for NonPrefixSlice {
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
