//! Unlike traditional hazard pointers, we use hazard *prefixes*,
//! which over-approxmiate a set of hazard pointers using a key prefix.
//!
//! First, note that every node and value in a trie can be associated
//! with a key prefix. For example, given the following trie:
//!
//! ```text
//!     N0 [ a | b ]
//!        /    |
//!       /     | c
//!      /      |
//!  N1 [f]  N2 [ d | e ]
//!     /        /   |
//!    /        /    | g
//!   /        /     |
//! (V0)     (V1)   (V2)
//! ```
//!
//! We have the following key prefixes:
//!
//! | Id | Type  | Prefix |
//! +----+-------|-------+
//! | N0 | Node  |       |
//! | N1 | Node  | a     |
//! | N2 | Node  | bc    |
//! | V0 | Value | af    |
//! | V1 | Value | bcd   |
//! | V2 | Value | bceg  |
//!
//! Second, note that each trie operation is also associated with
//! a key prefix. This can be a full key for point operations like
//! [`crate::concurrent::MapRef::get`], or a key prefix for prefix
//! operations like [`crate::concurrent::MapRef::prefix`].
//!
//! Then the core insight is that a trie operation will never access
//! nodes or values whose key prefixes do not overlap with its own.
//! We use guards to ensure that a hazard prefix is installed
//! for the lifetime of an operation.
//! Guards protect all nodes and values with overlapping key prefixes from
//! reclamation.
//!
//! In our example trie...
//!
//! ```text
//!     N0 [ a | b ]
//!        /    |
//!       /     | c
//!      /      |
//!  N1 [f]  N2 [ d | e ]
//!     /        /   |
//!    /        /    | g
//!   /        /     |
//! (V0)     (V1)   (V2)
//! ```
//!
//! A guard with key prefix `bceg` would protect
//! nodes N0 + N2 and value V2 from reclamation.
//! A guard with key prefix `b` would protect nodes N0 + N2
//! and values V1 + V2 from reclamation.

mod membarrier;
pub mod prefix;
pub(crate) use prefix::Prefix;

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use crate::concurrent::Key;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw::node;
use crate::stat;

pub struct Hazard;

impl Smr for Hazard {
    type Global<K, V>
        = Box<Global<K, V>>
    where
        K: Key,
        V: Value;
}

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

pub struct Global<K: Key, V: Value> {
    garbage: AtomicU64,

    // FIXME: jagged/triangular array
    hazards: [Cache<ribbit::Atomic<K::Prefix>>; smr::thread::MAX],
    locals: [UnsafeCell<Local<K::Prefix, V>>; smr::thread::MAX],
    membarrier: AtomicBool,
    reclaim_threshold: usize,
    value: PhantomData<V>,
}

unsafe impl<K: Key, V: Value> Send for Global<K, V> {}
unsafe impl<K: Key, V: Value> Sync for Global<K, V> {}

impl<K: Key, V: Value> Default for Global<K, V> {
    fn default() -> Self {
        Self {
            garbage: AtomicU64::new(0),
            hazards: core::array::from_fn(|_| {
                Cache(ribbit::Atomic::new_packed(
                    <<K::Prefix as ribbit::Pack>::Packed as Prefix>::HAZARD_NULL,
                ))
            }),
            locals: core::array::from_fn(|_| {
                UnsafeCell::new(Local {
                    garbage: 0,
                    cycle: 0,
                    snapshot: Vec::new(),
                    retired: Vec::new(),
                    _value: PhantomData,
                })
            }),
            membarrier: AtomicBool::new(false),
            reclaim_threshold: 64,
            value: PhantomData,
        }
    }
}

impl<K: Key, V: Value> Global<K, V> {
    #[inline]
    #[must_use]
    pub fn with_reclaim_threshold(mut self, reclaim_threshold: usize) -> Self {
        self.reclaim_threshold = reclaim_threshold;
        self
    }

    #[inline]
    pub fn set_membarrier(&mut self, enable: bool) {
        *self.membarrier.get_mut() = enable
    }

    /// Eagerly reclaim all retired allocations
    pub fn reclaim(&mut self) {
        self.locals
            .iter_mut()
            .take(smr::thread::count())
            .map(|local| local.get_mut())
            .flat_map(|local| local.retired.drain(..))
            .for_each(|(prefix, raw)| {
                deallocate::<K::Prefix, V>(prefix, raw, stat::Counter::FreeReclaim);
            })
    }
}

impl<K: Key, V: Value> Drop for Global<K, V> {
    fn drop(&mut self) {
        self.reclaim();
    }
}

impl<K: Key, V: Value> smr::Global<K, V> for Box<Global<K, V>> {
    type Guard<'g>
        = Guard<'g, K, V>
    where
        V: 'g,
        Self: 'g;

    #[inline]
    fn guard<'g>(&'g self, key: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        let id = usize::from(smr::thread::Id::current());
        let membarrier = self.membarrier.load(Ordering::Relaxed);
        let hazard = &self.hazards[id].0;
        let local = &self.locals[id];

        validate!(!hazard.load_packed(Ordering::Relaxed).is_active());
        hazard.store_packed(K::hazard(key), membarrier::fast_store_ordering(membarrier));
        membarrier::fast_barrier(membarrier);

        Guard {
            hazard,
            local,
            global: self,
        }
    }

    fn garbage(&self) -> u32 {
        self.garbage.load(Ordering::Relaxed) as u32
    }
}

#[repr(align(64))]
pub struct Local<P: ribbit::Pack<Packed: Prefix>, V> {
    garbage: i32,
    cycle: usize,
    snapshot: Vec<ribbit::Packed<P>>,
    retired: Vec<(ribbit::Packed<P>, u64)>,
    _value: PhantomData<V>,
}

impl<K: Key, V: Value> Global<K, V> {
    #[inline]
    pub fn enable_membarrier(&self) {
        self.membarrier.store(true, Ordering::Relaxed)
    }

    #[cold]
    fn flush(global: &Global<K, V>, local: &mut Local<K::Prefix, V>) {
        stat::max(stat::Max::RetireCache, local.retired.len() as u64);

        membarrier::slow(global.membarrier.load(Ordering::Relaxed));

        local.snapshot.extend(
            global.hazards[..smr::thread::count().next_multiple_of(4)]
                .iter()
                .map(|hazard| hazard.0.load_packed(Ordering::Relaxed)),
        );

        let mut freed = 0;
        let (chunks, leftover) = local.snapshot.as_chunks::<4>();
        validate!(leftover.is_empty());

        local.retired.retain_mut(|(prefix, raw)| {
            if chunks.iter().any(|chunk| prefix.is_conflict(chunk)) {
                stat::increment(stat::Counter::HazardMatch);
                if cfg!(feature = "stat") {
                    *prefix = prefix.with_age(prefix.age().saturating_add(1));
                }
                return true;
            }

            if cfg!(feature = "stat") {
                stat::record(stat::Record::ReclaimDepth, prefix.bytes() as u64);
            }
            freed += 1;

            validate!(prefix.is_value() ^ prefix.is_node());

            if cfg!(feature = "stat") {
                if let Some(record) = match prefix.bytes() {
                    0 => Some(stat::Record::ReclaimAge0),
                    1 => Some(stat::Record::ReclaimAge1),
                    2 => Some(stat::Record::ReclaimAge2),
                    3 => Some(stat::Record::ReclaimAge3),
                    _ => None,
                } {
                    stat::record(record, prefix.age() as u64 + 1);
                }
            }

            deallocate::<K::Prefix, V>(*prefix, *raw, stat::Counter::FreeRetire);
            false
        });

        if cfg!(feature = "stat-garbage") {
            local.garbage -= freed;

            if local.garbage <= -(global.reclaim_threshold as i32) {
                global
                    .garbage
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |garbage| {
                        let old_count = garbage >> 32;
                        let old_max = garbage as u32;

                        let new_count = old_count - ((-local.garbage) as u64);
                        Some(new_count << 32 | (old_max as u64))
                    })
                    .unwrap();
                local.garbage = 0;
            }
        }

        local.snapshot.clear();
        stat::record(stat::Record::Flush, freed as u64);
    }
}

pub struct Guard<'g, K: Key, V: Value> {
    hazard: &'g ribbit::Atomic<K::Prefix>,
    local: &'g UnsafeCell<Local<K::Prefix, V>>,
    global: &'g Global<K, V>,
}

impl<'g, K: Key, V: Value> smr::Guard<V> for Guard<'g, K, V> {
    #[expect(private_bounds)]
    #[expect(private_interfaces)]
    unsafe fn retire_node<M: ribbit::Pack<Packed: crate::raw::edge::Meta>>(
        &mut self,
        _bits: usize,
        node: ribbit::Packed<node::Ptr<M>>,
    ) {
        stat::increment(stat::Counter::Retire);

        let prefix = self
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(false, Some(_bits));

        let local = unsafe { &mut *self.local.get() };

        if cfg!(feature = "stat-garbage") {
            local.garbage += 1;

            if local.garbage >= self.global.reclaim_threshold as i32 {
                self.global
                    .garbage
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |garbage| {
                        let old_count = garbage >> 32;
                        let old_max = garbage as u32;

                        let new_count = old_count + local.garbage as u64;
                        let new_max = old_max.max(new_count as u32);
                        Some(new_count << 32 | (new_max as u64))
                    })
                    .unwrap();
                local.garbage = 0;
            }
        }

        local.retired.push((prefix, node.raw().get()));
    }

    unsafe fn retire_value(&mut self, value: u64) {
        stat::increment(stat::Counter::Retire);

        let prefix = self
            .hazard
            .load_packed(Ordering::Relaxed)
            .into_prefix(true, None);

        let local = unsafe { &mut *self.local.get() };

        if cfg!(feature = "stat-garbage") {
            local.garbage += 1;

            if local.garbage >= self.global.reclaim_threshold as i32 {
                self.global
                    .garbage
                    .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |garbage| {
                        let old_count = garbage >> 32;
                        let old_max = garbage as u32;

                        let new_count = old_count + local.garbage as u64;
                        let new_max = old_max.max(new_count as u32);
                        Some(new_count << 32 | (new_max as u64))
                    })
                    .unwrap();
                local.garbage = 0;
            }
        }

        local.retired.push((prefix, value));
    }
}

impl<'g, K: Key, V: Value> Drop for Guard<'g, K, V> {
    fn drop(&mut self) {
        self.hazard
            .store_packed(ribbit::Packed::<K::Prefix>::HAZARD_NULL, Ordering::Relaxed);

        let local = unsafe { &mut *self.local.get() };
        if local.retired.len() < self.global.reclaim_threshold {
            local.cycle = 0;
            return;
        }

        if local.cycle == 0 {
            Global::flush(self.global, local)
        }

        // FIXME: introduce separate configuration
        local.cycle = if local.cycle == self.global.reclaim_threshold {
            0
        } else {
            local.cycle + 1
        };
    }
}

fn deallocate<P: ribbit::Pack<Packed: Prefix>, V: Value>(
    prefix: ribbit::Packed<P>,
    raw: u64,
    counter: stat::Counter,
) {
    if prefix.is_node() {
        unsafe {
            // FIXME: type of edge meta is irrelevant here
            crate::raw::node::Ptr::<crate::raw::edge::Be>::from_raw_unchecked(raw)
                .deallocate(counter);
        }
    } else {
        unsafe {
            stat::increment(counter);
            drop(V::from_raw(raw));
        }
    }
}
