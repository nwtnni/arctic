use core::num::NonZeroU64;

use crate::Key;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw::node;
use crate::stat;

use seize::Guard as _;

#[derive(Default)]
pub struct Seize(seize::Collector);

impl Seize {
    pub fn with_batch_size(batch_size: usize) -> Self {
        Self(seize::Collector::new().batch_size(batch_size))
    }
}

impl<K: Key, V: Value> Smr<K, V> for Seize {
    type Guard<'g>
        = seize::LocalGuard<'g>
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, _: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        self.0.enter()
    }

    fn garbage(&self) -> u32 {
        self.0.garbage()
    }
}

impl<'g, V: Value> smr::Guard<V> for seize::LocalGuard<'g> {
    unsafe fn retire_node(&mut self, _bits: usize, node: ribbit::Packed<node::Ptr>) {
        stat::increment(stat::Counter::Retire);

        unsafe {
            self.defer_retire(node.into_raw().get() as *mut (), |ptr, _| {
                stat::increment(stat::Counter::FreeRetire);
                let ptr = NonZeroU64::new(ptr as u64).unwrap();
                node::Ptr::from_raw_unchecked(ptr).deallocate();
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
        //
        unsafe {
            self.defer_retire(value as *mut (), |ptr, _| {
                stat::increment(stat::Counter::FreeRetire);
                drop(V::from_raw_unchecked(ptr as u64))
            });
        }
    }
}
