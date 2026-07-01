//! Implementations of [`Smr`].
//!
//! By default, [`ConcurrentMap`][crate::concurrent::Map] uses [seize](https://docs.rs/seize/0.5.1/seize/)
//! for safe memory reclamation. Support for
//! [crossbeam-epoch](https://docs.rs/crossbeam-epoch/0.9.18/crossbeam_epoch/)
//! and hazard keys can be enabled with the
//! `smr-seize` and `smr-hazard` Cargo features, respectively.
//! Downstream users can also implement [`Smr`] and [`Guard`]
//! to provide their own SMR backends.

#[cfg(feature = "smr-epoch")]
mod epoch;
/// Auxiliary types for use with hazard keys.
#[cfg(feature = "smr-hazard")]
pub mod hazard;
/// Auxiliary types for use with no-op SMR.
pub mod no_op;
#[cfg(feature = "smr-seize")]
mod seize;

use core::num::NonZeroU64;

#[cfg(feature = "smr-epoch")]
pub use epoch::Epoch;
#[doc(inline)]
#[cfg(feature = "smr-hazard")]
pub use hazard::Hazard;
pub use no_op::NoOp;
#[cfg(feature = "smr-seize")]
pub use seize::Seize;

cfg_select! {
    feature = "smr-seize" => {
        /// Default [`Smr`] backend.
        pub type Default = Seize;
    }
    _ => {
        /// Default [`Smr`] backend.
        pub type Default = NoOp;
    }
}

use crate::Key;
use crate::concurrent::Value;
use crate::stat;

/// Provides [safe memory reclamation](https://arxiv.org/abs/2509.02457) for the
/// given key and value type.
pub trait Smr<K: Key, V: Value> {
    /// Guard type that protects nodes and values during its lifetime,
    /// can be used to retire allocations, and unprotects when it is dropped.
    type Guard<'g>: Guard<V>
    where
        V: 'g,
        Self: 'g;

    /// Construct a [`Guard`] that protects nodes and values associated with `prefix`.
    fn guard<'g>(&'g self, prefix: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g;

    /// Estimate the peak number of unreclaimed allocations.
    fn garbage(&self) -> u32 {
        0
    }
}

/// Protects allocations from deallocation, and allows allocations to be retired.
///
/// External implementations may call [`deallocate_node`] and [`deallocate_value`]
/// to deallocate nodes and values
/// (passed via [`Guard::retire_node`] and [`Guard::retire_value`]) when they
/// can determine there are no live references.
pub trait Guard<V: Value> {
    /// Retire an internal node with prefix length `bits`.
    ///
    /// # Safety
    ///
    /// Caller must guarantee `node` is a valid node pointer.
    unsafe fn retire_node(&mut self, bits: usize, node: NonZeroU64);

    /// Retire a value.
    ///
    /// # Safety
    ///
    /// Caller must guarantee `value` is a valid value pointer.
    unsafe fn retire_value(&mut self, value: u64);
}

/// Deallocate a previously retired node.
///
/// # Safety
///
/// Caller must guarantee there are no live references to `node`,
/// and that `node` was previously retired via [`Guard::retire_node`].
pub unsafe fn deallocate_node(node: NonZeroU64) {
    stat::increment(stat::Counter::FreeRetire);
    unsafe { ribbit::Packed::<crate::raw::node::Ptr>::from_raw_unchecked(node).deallocate() }
}

/// Deallocate a previously retired value.
///
/// # Safety
///
/// Caller must guarantee there are no live references to `value`,
/// and that `value` was previously retired via [`Guard::<V>::retire_value`][`Guard::retire_value`].
pub unsafe fn deallocate_value<V: Value>(value: u64) {
    stat::increment(stat::Counter::FreeRetire);
    drop(unsafe { V::from_raw_unchecked(value) })
}
