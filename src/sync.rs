use core::ops::Deref;
use core::ops::DerefMut;
use core::sync::atomic::Ordering;

cfg_select! {
    feature = "shuttle" => {
        use shuttle::sync::atomic;
        pub(crate) use shuttle::thread;
        pub use shuttle::sync::Arc;
    }
    _ => {
        pub use std::sync::Arc;
        pub(crate) use std::thread;
        use ribbit::atomic;
    }
}

pub(crate) type Atomic<T> =
    ribbit::Atomic<T, <<<T as ribbit::Pack>::Packed as ribbit::Unpack>::Loose as Loose>::Atomic>;

// Wrapper for ribbit::Loose that conditionally compiles with
// shuttle or std atomic types as ribbit::Atomic backend
pub(crate) trait Loose: Sized {
    type Atomic: ribbit::atomic::Raw<Self>;
}

macro_rules! impl_raw {
    ($inner:ty, $atomic:ident) => {
        impl Loose for $inner {
            type Atomic = $atomic;
        }

        #[derive(Default, Debug)]
        pub(crate) struct $atomic(atomic::$atomic);

        impl $atomic {
            #[inline]
            pub(crate) const fn new(value: $inner) -> Self {
                Self(atomic::$atomic::new(value))
            }

            #[inline]
            pub(crate) fn load(&self, ordering: Ordering) -> $inner {
                // HACK: when nesting shuttle inside proptest, the latter calls
                // `Debug::fmt` on atomic types outside of the shuttle execution context.
                #[cfg(feature = "shuttle")]
                if shuttle_core::runtime::execution::ExecutionState::try_with(|_| ()).is_err() {
                    return unsafe { self.0.raw_load() };
                }

                self.0.load(ordering)
            }
        }

        impl Deref for $atomic {
            type Target = atomic::$atomic;
            #[inline]
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $atomic {
            #[inline]
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl Clone for $atomic {
            #[inline]
            fn clone(&self) -> Self {
                Self::new(self.load(core::sync::atomic::Ordering::Relaxed))
            }
        }

        ribbit::impl_raw!($inner, $atomic);
    };
}

impl_raw!(u16, AtomicU16);
impl_raw!(u64, AtomicU64);
impl_raw!(u128, AtomicU128);

#[doc(hidden)]
pub fn check_dfs<F>(_count: usize, run: F)
where
    F: Fn() + Send + Sync + 'static,
{
    cfg_select! {
        feature = "shuttle" => { shuttle::check_dfs(run, None); }
        _ => {
            for _ in 0.._count {
                run();
            }
        }
    }
}
