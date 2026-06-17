use core::ops::Deref;
use core::ops::DerefMut;

cfg_select! {
    feature = "shuttle" => {
        use shuttle::sync::atomic;
        pub use shuttle::sync::Arc;
    }
    _ => {
        pub use std::sync::Arc;
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
