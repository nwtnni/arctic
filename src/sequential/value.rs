use core::ops::Deref;
use std::rc::Rc;

/// Values that can be safely stored in a [`SequentialMap`][crate::sequential::Map].
///
/// # Safety
///
/// Implementer must guarantee that `Self` has the same memory layout as a `u64`.
pub unsafe trait Value: Sized {
    /// Erase the type of this value, returning a u64.
    fn into_raw(self) -> u64;

    /// # Safety
    ///
    /// Caller must guarantee that:
    /// - `raw` was created by a previous [`Value::into_raw`] call.
    /// - `from_raw` is called at most once for each [`Value::into_raw`] call.
    /// - There are no live borrows when [`Value::from_raw_unchecked`] is called.
    unsafe fn from_raw_unchecked(raw: u64) -> Self;
}

// NOTE: `Sized` is required so that &T is not a fat pointer and fits in 8 bytes
unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    #[inline]
    fn into_raw(self) -> u64 {
        const {
            assert!(core::mem::size_of::<Self>() == core::mem::size_of::<u64>());
            assert!(core::mem::align_of::<Self>() == core::mem::align_of::<u64>());
        }

        (self as *const T).expose_provenance() as u64
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        let borrow = unsafe { core::ptr::with_exposed_provenance::<T>(raw as usize).as_ref() };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }
}

// NOTE: `Sized` is required so that Box<T> is not a fat pointer and fits in 8 bytes
unsafe impl<T: Sized> Value for Box<T> {
    #[inline]
    fn into_raw(self) -> u64 {
        const {
            assert!(core::mem::size_of::<Self>() == core::mem::size_of::<u64>());
            assert!(core::mem::align_of::<Self>() == core::mem::align_of::<u64>());
        }

        Box::into_raw(self).expose_provenance() as u64
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        unsafe { Box::from_raw(core::ptr::with_exposed_provenance_mut::<T>(raw as usize)) }
    }
}

// NOTE: `Sized` is required so that Arc<T> is not a fat pointer and fits in 8 bytes
unsafe impl<T: Sized> Value for Arc<T> {
    #[inline]
    fn into_raw(self) -> u64 {
        const {
            assert!(core::mem::size_of::<Self>() == core::mem::size_of::<u64>());
            assert!(core::mem::align_of::<Self>() == core::mem::align_of::<u64>());
        }

        crate::sync::Arc::into_raw(self.0).expose_provenance() as u64
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        Self(unsafe {
            crate::sync::Arc::from_raw(core::ptr::with_exposed_provenance(raw as usize))
        })
    }
}

// NOTE: `Sized` is required so that Rc<T> is not a fat pointer and fits in 8 bytes
unsafe impl<T: Sized> Value for Rc<T> {
    #[inline]
    fn into_raw(self) -> u64 {
        const {
            assert!(core::mem::size_of::<Self>() == core::mem::size_of::<u64>());
            assert!(core::mem::align_of::<Self>() == core::mem::align_of::<u64>());
        }

        Rc::into_raw(self).expose_provenance() as u64
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        unsafe { Rc::from_raw(core::ptr::with_exposed_provenance(raw as usize)) }
    }
}

/// Transparent wrapper for [`Arc<T>`][crate::sync::Arc<T>] smart pointer
/// that can be borrowed as an [`ArcRef<T>`][crate::concurrent::value::ArcRef].
#[repr(transparent)]
#[derive(Debug)]
pub struct Arc<T>(pub(crate) crate::sync::Arc<T>);

impl<T> From<crate::sync::Arc<T>> for Arc<T> {
    #[inline]
    fn from(arc: crate::sync::Arc<T>) -> Self {
        Self(arc)
    }
}

impl<T> From<Arc<T>> for crate::sync::Arc<T> {
    #[inline]
    fn from(Arc(arc): Arc<T>) -> Self {
        arc
    }
}

impl<T> Deref for Arc<T> {
    type Target = crate::sync::Arc<T>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

macro_rules! impl_integer {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                #[inline]
                fn into_raw(self) -> u64 {
                    self as u64
                }

                #[inline]
                unsafe fn from_raw_unchecked(raw: u64) -> Self {
                    raw as $ty
                }

            }
        )*
    };
}

impl_integer!(u64, i64);
