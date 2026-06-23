use core::num::NonZeroU64;

use crate::Key;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::stat;

use seize::Guard as _;

/// [`seize::Collector`] backend for safe memory reclamation.
///
/// Defaults to a batch size of 256, which we found to provide
/// the best balance of throughput and reclamation efficiency
/// in our benchmarks.
///
/// # Examples
///
/// ```rust
/// use arctic::ConcurrentMap;
/// use arctic::concurrent::smr::Seize;
///
/// let map = ConcurrentMap::<u64, Box<u64>, Seize>::with_smr(Seize::from(
///     seize::Collector::new().batch_size(256)
/// ));
/// ```
pub struct Seize(seize::Collector);

impl Default for Seize {
    fn default() -> Self {
        Self(seize::Collector::default().batch_size(256))
    }
}

impl From<seize::Collector> for Seize {
    fn from(collector: seize::Collector) -> Self {
        Self(collector)
    }
}

impl From<Seize> for seize::Collector {
    fn from(Seize(collector): Seize) -> Self {
        collector
    }
}

impl<K: Key, V: Value> Smr<K, V> for Seize {
    type Guard<'g>
        = seize::LocalGuard<'g>
    where
        V: 'g,
        Self: 'g;

    // NOTE: seize documentation says to call `seize::Guard::protect`
    // on every pointer load, which loads with `SeqCst` ordering
    // under the hood, but it's not clear to me why this is necessary?
    //
    // At least in arctic, pointers are always installed via a CAS
    // with `AcqRel` ordering, so we should always see the correct
    // contents of the allocation with `Acquire` loads.
    fn guard<'g>(&'g self, _: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        self.0.enter()
    }
}

impl<'g, V: Value> smr::Guard<V> for seize::LocalGuard<'g> {
    unsafe fn retire_node(&mut self, _bits: usize, node: NonZeroU64) {
        stat::increment(stat::Counter::Retire);

        unsafe {
            self.defer_retire(node.get() as *mut (), |ptr, _| {
                let node = NonZeroU64::new(ptr as u64).unwrap();
                smr::deallocate_node(node);
            })
        }
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        // HACK: Unfortunately, Seize does not natively support `defer_unchecked`.
        // However, `defer_retire` does take an arbitrary closure to run at retire-time,
        // and passes the `ptr` argument directly to it...
        //
        // See: [`seize::raw::Collector::add`] and [`seize::raw::Collector::try_retire`].
        unsafe {
            self.defer_retire(value as *mut (), |ptr, _| {
                smr::deallocate_value::<V>(ptr as u64)
            });
        }
    }
}
