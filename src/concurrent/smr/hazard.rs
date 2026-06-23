mod membarrier;
pub mod prefix;
mod thread;
pub(crate) use prefix::Prefix;
mod key;
pub use key::Key;

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::num::NonZeroU64;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::AtomicU64;
use core::sync::atomic::Ordering;

use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::stat;
use crate::sync::Atomic;

#[repr(C, align(64))]
#[derive(Default)]
struct Cache<T>(T);

/// Hazard key backend for safe memory reclamation.
///
/// <div class="warning">
///
/// **This backend currently has the following limitations**:
/// - Only one hazard can be installed at a time per thread, so calling `get`
///   while holding a reference from a previous `get`, for example, will panic.
/// - Only supports up to 256 threads.
///
/// </div>
///
/// Hazard keys are a new SMR scheme that use an operation's key to
/// approximate a set of [hazard pointers](https://en.wikipedia.org/wiki/Hazard_pointer).
///
/// To briefly illustrate the idea: every node and value in a tree can be associated
/// with a key prefix. For example, given the following tree:
///
/// ```text
///     N0 [ a | b ]
///        /    |
///       /     | c
///      /      |
///  N1 [f]  N2 [ d | e ]
///     /        /   |
///    /        /    | g
///   /        /     |
/// (V0)     (V1)   (V2)
/// ```
///
/// We have the following key prefixes:
///
/// | Id | Type  | Prefix |
/// |----|-------|-------|
/// | N0 | Node  |       |
/// | N1 | Node  | a     |
/// | N2 | Node  | bc    |
/// | V0 | Value | af    |
/// | V1 | Value | bcd   |
/// | V2 | Value | bceg  |
///
/// Second, note that each operation is also associated with
/// a key prefix. This can be a full key for point operations like
/// [`ConcurrentMap::get`][crate::concurrent::Map::get], or a key prefix for prefix
/// operations like [`ConcurrentMap::prefix`][crate::concurrent::Map::prefix].
///
/// Then the core insight is that a trie operation will never access
/// nodes or values whose key prefixes do not overlap with its own.
/// We use guards to ensure that a hazard key is installed
/// for the lifetime of an operation.
/// Guards protect all nodes and values with overlapping key prefixes from
/// reclamation.
///
/// In our example tree...
///
/// ```text
///     N0 [ a | b ]
///        /    |
///       /     | c
///      /      |
///  N1 [f]  N2 [ d | e ]
///     /        /   |
///    /        /    | g
///   /        /     |
/// (V0)     (V1)   (V2)
/// ```
///
/// A guard with key prefix `bceg` would protect
/// nodes N0 + N2 and value V2 from reclamation.
/// A guard with key prefix `b` would protect nodes N0 + N2
/// and values V1 + V2 from reclamation.
pub struct Hazard<K: Key, V: Value>(Box<Global<K, V>>);

impl<K: Key, V: Value> Default for Hazard<K, V> {
    fn default() -> Self {
        Self(Box::default())
    }
}

struct Global<K: Key, V: Value> {
    garbage: AtomicU64,

    // FIXME: jagged/triangular array
    hazards: [Cache<Atomic<K::Prefix>>; thread::MAX],
    locals: [UnsafeCell<Local<K::Prefix, V>>; thread::MAX],
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
                Cache(Atomic::new_packed(
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

impl<K: Key, V: Value> Hazard<K, V> {
    /// Configure the number of retired allocations that can accumulate per-thread
    /// before the thread loads all hazard pointers and attempts to reclaim them.
    #[inline]
    #[must_use]
    pub fn with_reclaim_threshold(mut self, reclaim_threshold: usize) -> Self {
        self.0.reclaim_threshold = reclaim_threshold;
        self
    }

    /// Enable or disable [`membarrier`](https://man7.org/linux/man-pages/man2/membarrier.2.html) optimization.
    #[inline]
    pub fn set_membarrier(&mut self, enable: bool) {
        *self.0.membarrier.get_mut() = enable
    }

    /// Enable [`membarrier`](https://man7.org/linux/man-pages/man2/membarrier.2.html) optimization.
    #[inline]
    pub fn enable_membarrier(&self) {
        self.0.membarrier.store(true, Ordering::Relaxed)
    }

    /// Eagerly reclaim all retired allocations.
    pub fn reclaim(&mut self) {
        self.0.reclaim()
    }
}

impl<K: Key, V: Value> Global<K, V> {
    fn reclaim(&mut self) {
        self.locals
            .iter_mut()
            .take(thread::count())
            .map(|local| local.get_mut())
            .flat_map(|local| local.retired.drain(..))
            .for_each(|(prefix, raw)| {
                stat::increment(stat::Counter::FreeReclaim);
                deallocate::<K::Prefix, V>(prefix, raw);
            })
    }
}

impl<K: Key, V: Value> Drop for Global<K, V> {
    fn drop(&mut self) {
        self.reclaim();
    }
}

impl<K: Key, V: Value> Smr<K, V> for Hazard<K, V> {
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
        let id = usize::from(thread::Id::current());
        let membarrier = self.0.membarrier.load(Ordering::Relaxed);
        let hazard = &self.0.hazards[id].0;
        let local = &self.0.locals[id];

        assert!(!hazard.load_packed(Ordering::Relaxed).is_active());
        hazard.store_packed(K::hazard(key), membarrier::fast_store_ordering(membarrier));
        membarrier::fast_barrier(membarrier);

        Guard {
            hazard,
            local,
            global: &self.0,
        }
    }

    fn garbage(&self) -> u32 {
        self.0.garbage.load(Ordering::Relaxed) as u32
    }
}

#[repr(align(64))]
struct Local<P: ribbit::Pack<Packed: Prefix>, V> {
    garbage: i32,
    cycle: usize,
    snapshot: Vec<ribbit::Packed<P>>,
    retired: Vec<(ribbit::Packed<P>, u64)>,
    _value: PhantomData<V>,
}

impl<K: Key, V: Value> Global<K, V> {
    #[cold]
    fn flush(global: &Global<K, V>, local: &mut Local<K::Prefix, V>) {
        stat::max(stat::Max::RetireCache, local.retired.len() as u64);

        membarrier::slow(global.membarrier.load(Ordering::Relaxed));

        local.snapshot.extend(
            global.hazards[..thread::count().next_multiple_of(4)]
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

            if cfg!(feature = "stat")
                && let Some(record) = match prefix.bytes() {
                    0 => Some(stat::Record::ReclaimAge0),
                    1 => Some(stat::Record::ReclaimAge1),
                    2 => Some(stat::Record::ReclaimAge2),
                    3 => Some(stat::Record::ReclaimAge3),
                    _ => None,
                }
            {
                stat::record(record, prefix.age() as u64 + 1);
            }

            stat::increment(stat::Counter::FreeRetire);
            deallocate::<K::Prefix, V>(*prefix, *raw);
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

/// Guard for [`Hazard`] SMR backend.
pub struct Guard<'g, K: Key, V: Value> {
    hazard: &'g Atomic<K::Prefix>,
    local: &'g UnsafeCell<Local<K::Prefix, V>>,
    global: &'g Global<K, V>,
}

impl<'g, K: Key, V: Value> smr::Guard<V> for Guard<'g, K, V> {
    unsafe fn retire_node(&mut self, _bits: usize, node: NonZeroU64) {
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

        local.retired.push((prefix, node.get()));
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

fn deallocate<P: ribbit::Pack<Packed: Prefix>, V: Value>(prefix: ribbit::Packed<P>, raw: u64) {
    if prefix.is_node() {
        unsafe {
            let ptr = NonZeroU64::new(raw).unwrap();
            smr::deallocate_node(ptr)
        }
    } else {
        unsafe { smr::deallocate_value::<V>(raw) }
    }
}
