use core::cell::Cell;
use core::marker::PhantomData;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering;

use crate::Key;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::smr;

thread_local! {
    static GARBAGE_LOCAL: Cell<u32> = const { Cell::new(0) };
}

static GARBAGE_GLOBAL: AtomicU32 = AtomicU32::new(0);

// FIXME: configurable?
const GARBAGE_THRESHOLD: u32 = 256;

#[derive(Default)]
pub struct NoOp;

impl<K: Key, V: Value> Smr<K, V> for NoOp {
    type Guard<'g>
        = Guard<(), V>
    where
        V: 'g,
        Self: 'g;

    fn guard<'g>(&'g self, _: K::Read<'_>) -> Self::Guard<'g>
    where
        V: 'g,
    {
        Guard::default()
    }

    fn garbage(&self) -> u32 {
        GARBAGE_GLOBAL.load(Ordering::Relaxed)
    }
}

pub struct Guard<G, V> {
    _guard: PhantomData<G>,
    _value: PhantomData<V>,
}

impl<G, V> Default for Guard<G, V> {
    fn default() -> Self {
        Self {
            _guard: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<G, V: Value> smr::Guard<V> for Guard<G, V> {
    unsafe fn retire_node(&mut self, _bits: usize, _edge: ribbit::Packed<crate::raw::node::Ptr>) {
        if cfg!(feature = "stat-garbage") {
            GARBAGE_LOCAL.set(GARBAGE_LOCAL.get() + 1);

            if GARBAGE_LOCAL.get() > GARBAGE_THRESHOLD {
                GARBAGE_GLOBAL.fetch_add(GARBAGE_THRESHOLD, Ordering::Relaxed);
                GARBAGE_LOCAL.set(0);
            }
        }
    }

    unsafe fn retire_value(&mut self, _value: u64) {
        if cfg!(feature = "stat-garbage") {
            GARBAGE_LOCAL.set(GARBAGE_LOCAL.get() + 1);

            if GARBAGE_LOCAL.get() > GARBAGE_THRESHOLD {
                GARBAGE_GLOBAL.fetch_add(GARBAGE_THRESHOLD, Ordering::Relaxed);
                GARBAGE_LOCAL.set(0);
            }
        }
    }
}

impl<G, V: Value> From<G> for Guard<G, V> {
    #[inline]
    fn from(_: G) -> Self {
        Self::default()
    }
}
