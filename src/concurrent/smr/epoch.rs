use core::num::NonZeroU64;

use crate::Key;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::stat;

/// [`crossbeam_epoch::Collector`] backend for safe memory reclamation.
///
/// Uses the default global collector.
///
/// <div class="warning">
///
/// In our [benchmarks](https://github.com/nwtnni/index-bench), we found the
/// [`MAX_OBJECTS`](https://github.com/crossbeam-rs/crossbeam/blob/05f9478b333ead58c0bf8e5a37d9ef9bd3b5bf17/crossbeam-epoch/src/internal.rs#L66)
/// constant causes a throughput bottleneck, but your mileage may vary.
///
/// </div>
///
/// # Examples
///
/// ```rust
/// use arctic::ConcurrentMap;
/// use arctic::concurrent::smr::Epoch;
///
/// let map = ConcurrentMap::<u64, Box<u64>, Epoch>::new();
/// ```
#[derive(Default)]
pub struct Epoch;

impl<K: Key, V: Value> Smr<K, V> for Box<Epoch> {
    type Guard<'g>
        = crossbeam_epoch::Guard
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, _: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        crossbeam_epoch::pin()
    }
}

impl<V: Value> smr::Guard<V> for crossbeam_epoch::Guard {
    unsafe fn retire_node(&mut self, _bits: usize, node: NonZeroU64) {
        stat::increment(stat::Counter::Retire);

        unsafe {
            self.defer_unchecked(move || {
                smr::deallocate_node(node);
            });
        }
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        unsafe {
            self.defer_unchecked(move || smr::deallocate_value::<V>(value));
        }
    }
}
