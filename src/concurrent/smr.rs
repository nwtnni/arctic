pub mod epoch;
pub mod hazard;
pub mod no_op;
pub mod seize;
mod thread;

pub use epoch::Epoch;
pub use hazard::Hazard;
pub use no_op::NoOp;
pub use seize::Seize;

use crate::Key;
use crate::concurrent::Value;
use crate::raw::node;

/// Provides [safe memory reclamation](https://arxiv.org/abs/2509.02457) for the
/// given key and value type.
pub trait Smr<K: Key, V: Value> {
    type Guard<'g>: Guard<V>
    where
        V: 'g,
        Self: 'g;

    /// Construct a guard that prevents values whose keys start with `prefix`
    /// from being reclaimed for the lifetime of the guard.
    fn guard<'g>(&'g self, prefix: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g;

    /// Estimate the peak number of unreclaimed allocations for benchmarking.
    fn garbage(&self) -> u32 {
        0
    }
}

/// A guard that can be used to safely retire allocations, delaying their
/// reclamation until it is safe.
pub trait Guard<V: Value + ?Sized> {
    /// Retire a pointer to a node with prefix length `bits`.
    unsafe fn retire_node(&mut self, bits: usize, node: ribbit::Packed<node::Ptr>);

    /// Retire a pointer to a value (can be reconstructed via [`crate::sequential::Value::from_raw`]).
    unsafe fn retire_value(&mut self, value: u64);
}
