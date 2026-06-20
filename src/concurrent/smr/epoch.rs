use core::cell::UnsafeCell;
use core::num::NonZeroU64;
use core::sync::atomic::Ordering;

use crossbeam_epoch::LocalHandle;

use crate::Key;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::stat;

pub struct Epoch {
    collector: crossbeam_epoch::Collector,
    locals: [UnsafeCell<Option<LocalHandle>>; smr::thread::MAX],
}

unsafe impl Send for Epoch {}
unsafe impl Sync for Epoch {}

impl Default for Epoch {
    fn default() -> Self {
        Self {
            collector: crossbeam_epoch::Collector::default(),
            locals: core::array::from_fn(|_| UnsafeCell::new(None)),
        }
    }
}

impl Epoch {
    pub fn with_bag_capacity(max_objects: usize) -> Self {
        crossbeam_epoch::set_bag_capacity(max_objects);
        Self::default()
    }

    fn local(&self) -> &LocalHandle {
        let id = smr::thread::Id::current();
        let local = &self.locals[usize::from(id)];
        match unsafe { local.get().as_ref().unwrap() } {
            Some(local) => local,
            None => self.local_cold(),
        }
    }

    #[cold]
    fn local_cold(&self) -> &LocalHandle {
        let id = smr::thread::Id::current();
        let local = &self.locals[usize::from(id)];
        unsafe { local.get().as_mut().unwrap() }.insert(self.collector.register())
    }
}

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
        self.local().pin()
    }

    fn garbage(&self) -> u32 {
        let garbage = crossbeam_epoch::GLOBAL_GARBAGE_COUNT.load(Ordering::Relaxed);
        garbage as u32
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
