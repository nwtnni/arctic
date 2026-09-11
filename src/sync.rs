//! Abstraction layer for mocking synchronization primitives
//! with concurrency testing frameworks.

use core::fmt::Debug;
use core::marker::PhantomData;
use core::sync::atomic::Ordering;

cfg_select! {
    feature = "shuttle" => {
        pub use shuttle::sync::atomic;
        pub use shuttle::thread;
        pub use shuttle::sync::Arc;
    }
    _ => {
        pub use std::sync::Arc;
        pub use std::thread;
        pub mod atomic {
            pub use core::sync::atomic::AtomicU16;
            pub use core::sync::atomic::AtomicU64;
            pub use core::sync::atomic::fence;
            pub use ribbit::atomic::AtomicU128;
        }
    }
}

// pub(crate) type Atomic<T> =
//     ribbit::Atomic<T, <<<T as ribbit::Pack>::Packed as ribbit::Unpack>::Loose as Loose>::Atomic>;

pub trait Convert<T>: Copy {
    fn into_raw(self) -> T;
    unsafe fn from_raw_unchecked(raw: T) -> Self;
}

macro_rules! impl_atomic {
    ($unsigned:ty, $inner:ident, $outer:ident) => {
        #[repr(transparent)]
        pub(crate) struct $outer<T = $unsigned> {
            raw: atomic::$inner,
            r#type: PhantomData<T>,
        }

        impl<T: Convert<$unsigned>> $outer<T> {
            #[inline]
            pub(crate) fn new(raw: T) -> Self {
                Self {
                    raw: atomic::$inner::new(raw.into_raw()),
                    r#type: PhantomData,
                }
            }

            #[inline]
            pub(crate) const unsafe fn from_raw_unchecked(raw: $unsigned) -> Self {
                Self {
                    raw: atomic::$inner::new(raw),
                    r#type: PhantomData,
                }
            }

            #[inline]
            pub(crate) fn load(&self, ordering: Ordering) -> T {
                unsafe { T::from_raw_unchecked(self.raw.load(ordering)) }
            }

            #[inline]
            #[allow(unused)]
            pub(crate) fn get(&mut self) -> T {
                unsafe { T::from_raw_unchecked(*self.raw.get_mut()) }
            }

            #[inline]
            pub(crate) fn set(&mut self, new: T) {
                *self.raw.get_mut() = new.into_raw();
            }

            #[inline]
            pub(crate) fn compare_exchange(
                &self,
                old: T,
                new: T,
                success: Ordering,
                failure: Ordering,
            ) -> Result<T, T> {
                self.raw
                    .compare_exchange(old.into_raw(), new.into_raw(), success, failure)
                    .map(|old| unsafe { T::from_raw_unchecked(old) })
                    .map_err(|new| unsafe { T::from_raw_unchecked(new) })
            }
        }

        impl<T: Convert<$unsigned>> Clone for $outer<T> {
            #[inline]
            fn clone(&self) -> Self {
                unsafe { Self::from_raw_unchecked(self.raw.load(Ordering::Relaxed)) }
            }
        }

        impl<T: Convert<$unsigned> + Default> Default for $outer<T> {
            #[inline]
            fn default() -> Self {
                Self::new(T::default())
            }
        }

        impl<T: Convert<$unsigned> + Debug> Debug for $outer<T> {
            #[inline]
            fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
                self.raw.load(Ordering::Relaxed).fmt(fmt)
            }
        }

        impl Convert<$unsigned> for $unsigned {
            #[inline]
            fn into_raw(self) -> $unsigned {
                self
            }

            #[inline]
            unsafe fn from_raw_unchecked(raw: $unsigned) -> Self {
                raw
            }
        }
    };
}

impl_atomic!(u64, AtomicU64, Atomic64);
impl_atomic!(u128, AtomicU128, Atomic128);

#[doc(hidden)]
pub fn check_dfs<F>(_count: Option<usize>, run: F)
where
    F: Fn() + Send + Sync + 'static,
{
    cfg_select! {
        feature = "shuttle" => { shuttle::check_dfs(run, _count); }
        _ => {
            run();
        }
    }
}

#[doc(hidden)]
pub fn check_pct<F>(_count: usize, _depth: usize, run: F)
where
    F: Fn() + Send + Sync + 'static,
{
    cfg_select! {
        feature = "shuttle" => { shuttle::check_pct(run, _count, _depth); }
        _ => {
            run();
        }
    }
}
