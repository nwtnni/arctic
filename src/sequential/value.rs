use std::rc::Rc;
use std::sync::Arc;

/// Values that can be stored in a [`crate::sequential::Map`].
///
/// # Safety
///
/// Implementer must guarantee that `Self` has the same memory layout as a `u64`.
pub unsafe trait Value {
    fn into_raw(self) -> u64;

    /// # Safety
    ///
    /// Caller must guarantee that:
    /// - `raw` was created by a previous [`Value::into_raw`] call.
    /// - `from_raw` is called at most once for each [`Value::into_raw`] call.
    /// - There are no live borrows when [`Value::from_raw`] is called.
    unsafe fn from_raw(raw: u64) -> Self;
}

// NOTE: `Sized` is required so that &T is not a fat pointer and fits in 8 bytes
unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    #[inline]
    fn into_raw(self) -> u64 {
        (self as *const T).expose_provenance() as u64
    }

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        let borrow = unsafe { core::ptr::with_exposed_provenance::<T>(raw as usize).as_ref() };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }
}

// NOTE: `Sized` is required so that Box<T> is not a fat pointer and fits in 8 bytes
unsafe impl<T: Sized> Value for Box<T> {
    #[inline]
    fn into_raw(self) -> u64 {
        Box::into_raw(self).expose_provenance() as u64
    }

    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        unsafe { Box::from_raw(core::ptr::with_exposed_provenance_mut::<T>(raw as usize)) }
    }
}

// NOTE: `Sized` is required so that Arc<T> is not a fat pointer and fits in 8 bytes
unsafe impl<T: Sized> Value for Arc<T> {
    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        unsafe { Arc::from_raw(core::ptr::with_exposed_provenance(raw as usize)) }
    }

    #[inline]
    fn into_raw(self) -> u64 {
        Arc::into_raw(self).expose_provenance() as u64
    }
}

// NOTE: `Sized` is required so that Rc<T> is not a fat pointer and fits in 8 bytes
unsafe impl<T: Sized> Value for Rc<T> {
    #[inline]
    unsafe fn from_raw(raw: u64) -> Self {
        unsafe { Rc::from_raw(core::ptr::with_exposed_provenance(raw as usize)) }
    }

    #[inline]
    fn into_raw(self) -> u64 {
        Rc::into_raw(self).expose_provenance() as u64
    }
}

macro_rules! impl_integer {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                #[inline]
                unsafe fn from_raw(raw: u64) -> Self {
                    raw as $ty
                }

                #[inline]
                fn into_raw(self) -> u64 {
                    self as u64
                }
            }
        )*
    };
}

impl_integer!(u64, i64);
