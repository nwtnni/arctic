//! Support for dynamically sized keys that end with a null terminator.

use core::borrow::Borrow;
use core::num::NonZeroUsize;
use core::ops::Deref;

pub mod slice;
pub mod vec;

/// Newtype guaranteeing this [`Vec<u8>`] (a) is not empty,
/// and (b) does not contain any null bytes.
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonNullVec(Vec<u8>);

impl NonNullVec {
    /// # Safety
    ///
    /// Caller must guarantee that `slice` (a) is non-empty and (b) does not contain null bytes.
    #[inline]
    pub const unsafe fn new_unchecked(vec: Vec<u8>) -> Self {
        Self(vec)
    }

    /// Return a `NonNullSlice` if `slice` satisfies guarantees of [`NonNullSlice::new_unchecked`].
    pub const fn new(vec: Vec<u8>) -> Result<Self, Vec<u8>> {
        match NonNullSlice::new(vec.as_slice()) {
            None => Err(vec),
            Some(_) => Ok(Self(vec)),
        }
    }

    /// Returns a borrowed slice.
    #[inline]
    pub const fn as_non_null_slice(&self) -> &NonNullSlice {
        unsafe { NonNullSlice::new_unchecked(self.0.as_slice()) }
    }
}

impl From<NonNullVec> for Vec<u8> {
    #[inline]
    fn from(NonNullVec(vec): NonNullVec) -> Self {
        vec
    }
}

impl core::ops::Deref for NonNullVec {
    type Target = NonNullSlice;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_non_null_slice()
    }
}

impl core::borrow::Borrow<NonNullSlice> for NonNullVec {
    #[inline]
    fn borrow(&self) -> &NonNullSlice {
        self.as_non_null_slice()
    }
}

impl AsRef<NonNullSlice> for NonNullVec {
    #[inline]
    fn as_ref(&self) -> &NonNullSlice {
        self.as_non_null_slice()
    }
}

/// Newtype guaranteeing this slice (a) is non-empty,
/// and (b) does not contain any null bytes.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonNullSlice([u8]);

impl NonNullSlice {
    /// # Safety
    ///
    /// Caller must guarantee that `slice` (a) is non-empty and (b) does not contain null bytes.
    #[inline]
    pub const unsafe fn new_unchecked(slice: &[u8]) -> &Self {
        // SAFETY: `NonNullSlice` is `repr(transparent)`
        unsafe { core::mem::transmute(slice) }
    }

    /// Return a `NonNullSlice` if `slice` satisfies guarantees of [`NonNullSlice::new_unchecked`].
    #[inline]
    pub const fn new(slice: &[u8]) -> Option<&Self> {
        if slice.is_empty() {
            return None;
        }

        // HACK: `contains` is not const
        let mut i = 0;
        while i < slice.len() {
            if slice[i] == 0 {
                return None;
            }
            i += 1;
        }

        Some(unsafe { Self::new_unchecked(slice) })
    }

    /// Returns an owned copy of this slice.
    #[inline]
    pub fn to_non_null_vec(&self) -> NonNullVec {
        unsafe { NonNullVec::new_unchecked(self.0.to_owned()) }
    }

    /// Return the length in bytes.
    #[inline]
    pub const fn len(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.0.len()).expect("NonNullSlice is non-empty")
    }

    /// Return a borrowed slice of the underlying bytes.
    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        // SAFETY: `NullTerminatedSlice` is `repr(transparent)`
        unsafe { core::mem::transmute::<&NonNullSlice, &[u8]>(self) }
    }
}

impl<'a> From<&'a NonNullSlice> for &'a [u8] {
    #[inline]
    fn from(slice: &'a NonNullSlice) -> &'a [u8] {
        slice.as_slice()
    }
}

impl Borrow<[u8]> for NonNullSlice {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for NonNullSlice {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Deref for NonNullSlice {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
