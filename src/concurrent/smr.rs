pub mod epoch;
pub mod hazard;
pub mod no_op;
pub mod seize;
mod thread;

pub use epoch::Epoch;
pub use hazard::Hazard;
pub use no_op::NoOp;
pub use seize::Seize;

use crate::concurrent::Key;
use crate::concurrent::Value;
use crate::raw::edge;
use crate::raw::node;

/// Provides [safe memory reclamation](https://arxiv.org/abs/2509.02457).
pub trait Smr<K: Key, V: Value> {
    type Guard<'g>: Guard<V>
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, key: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g;

    fn garbage(&self) -> u32;
}

pub trait Guard<V: Value + ?Sized> {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: edge::Meta>>(
        &mut self,
        bits: usize,
        edge: ribbit::Packed<node::Ptr<M>>,
    );

    unsafe fn retire_value(&mut self, raw: u64);
}
