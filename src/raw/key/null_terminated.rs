//! Support for dynamically sized keys that end with a null terminator.

use core::borrow::Borrow;
use core::num::NonZeroUsize;
use core::ops::Deref;

pub mod slice;
pub mod vec;

/// Newtype guaranteeing this [`Vec<u8>`] (a) has length > 1,
/// (b) does not contain interior null bytes,
/// and (c) ends with a null terminator.
#[repr(transparent)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NullTerminatedVec(Vec<u8>);

impl NullTerminatedVec {
    /// # Safety
    ///
    /// Caller must guarantee that `vec` (a) has length > 1,
    /// (b) does not contain interior null bytes, and (c) ends with a null byte.
    #[inline]
    pub const unsafe fn new_unchecked(vec: Vec<u8>) -> Self {
        Self(vec)
    }

    /// Return a `NullTerminatedVec` if `slice` satisfies conditions of [`NullTerminatedVec::new_unchecked`].
    pub const fn new(vec: Vec<u8>) -> Result<Self, Vec<u8>> {
        match NullTerminatedSlice::new(vec.as_slice()) {
            None => Err(vec),
            Some(_) => Ok(Self(vec)),
        }
    }

    /// Returns a borrowed slice.
    #[inline]
    pub const fn as_null_terminated_slice(&self) -> &NullTerminatedSlice {
        unsafe { NullTerminatedSlice::new_unchecked(self.0.as_slice()) }
    }
}

impl From<NullTerminatedVec> for Vec<u8> {
    #[inline]
    fn from(NullTerminatedVec(vec): NullTerminatedVec) -> Self {
        vec
    }
}

impl core::ops::Deref for NullTerminatedVec {
    type Target = NullTerminatedSlice;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_null_terminated_slice()
    }
}

impl core::borrow::Borrow<NullTerminatedSlice> for NullTerminatedVec {
    #[inline]
    fn borrow(&self) -> &NullTerminatedSlice {
        self.as_null_terminated_slice()
    }
}

impl AsRef<NullTerminatedSlice> for NullTerminatedVec {
    #[inline]
    fn as_ref(&self) -> &NullTerminatedSlice {
        self.as_null_terminated_slice()
    }
}

/// Newtype guaranteeing this slice (a) has length > 1,
/// (b) does not contain interior null bytes,
/// and (c) ends with a null terminator.
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NullTerminatedSlice([u8]);

impl NullTerminatedSlice {
    /// # Safety
    ///
    /// Caller must guarantee that `slice` (a) has length > 1,
    /// (b) does not contain interior null bytes, and (c) ends with a null byte.
    #[inline]
    pub const unsafe fn new_unchecked(slice: &[u8]) -> &Self {
        // SAFETY: `NullTerminatedSlice` is `repr(transparent)`
        unsafe { core::mem::transmute(slice) }
    }

    /// Return a `NullTerminatedSlice` if `slice` satisfies conditions of [`NullTerminatedSlice::new_unchecked`].
    pub const fn new(slice: &[u8]) -> Option<&Self> {
        if slice.len() <= 1 {
            return None;
        }

        // HACK: `contains` is not const
        let mut i = 0;
        while i < slice.len() - 1 {
            if slice[i] == 0 {
                return None;
            }
            i += 1;
        }

        match slice.last() {
            Some(0) => (),
            _ => return None,
        }

        Some(unsafe { Self::new_unchecked(slice) })
    }

    /// Returns an owned copy of this slice.
    #[inline]
    pub fn to_null_terminated_vec(&self) -> NullTerminatedVec {
        unsafe { NullTerminatedVec::new_unchecked(self.0.to_owned()) }
    }

    /// Return the length in bytes.
    #[inline]
    pub const fn len(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.0.len()).expect("NullTerminatedSlice is non-empty")
    }

    /// Return a borrowed slice of the underlying bytes.
    #[inline]
    pub const fn as_slice(&self) -> &[u8] {
        // SAFETY: `NullTerminatedSlice` is `repr(transparent)`
        unsafe { core::mem::transmute::<&NullTerminatedSlice, &[u8]>(self) }
    }
}

impl<'a> From<&'a NullTerminatedSlice> for &'a [u8] {
    #[inline]
    fn from(slice: &'a NullTerminatedSlice) -> &'a [u8] {
        slice.as_slice()
    }
}

impl Borrow<[u8]> for NullTerminatedSlice {
    #[inline]
    fn borrow(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for NullTerminatedSlice {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Deref for NullTerminatedSlice {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}
