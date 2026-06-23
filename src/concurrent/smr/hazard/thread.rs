// https://github.com/Amanieu/thread_local-rs/blob/2ed68653c6ad8c41a23e9e422914aa92af5a98cd/src/thread_id.rs
// https://github.com/ibraheemdev/seize/blob/4e746342f6b8a383234b491d3cf4cae697fdad28/src/raw/tls/thread_id.rs

use core::cell::Cell;
use core::cmp;
use core::num::NonZeroUsize;
use core::sync::atomic::AtomicUsize;
use core::sync::atomic::Ordering;
use std::collections::BinaryHeap;
use std::sync::Mutex;

thread_local! {
    static ID_FAST: Cell<Option<Id>> = const { Cell::new(None) };
    static ID_SLOW: Cell<Option<Guard>> = const { Cell::new(None) };
}

pub(super) const MAX: usize = 256;
static NEXT: AtomicUsize = AtomicUsize::new(1);
static FREE: Mutex<BinaryHeap<cmp::Reverse<NonZeroUsize>>> = Mutex::new(BinaryHeap::new());

#[repr(transparent)]
#[derive(Copy, Clone)]
pub(super) struct Id(NonZeroUsize);

pub(super) fn count() -> usize {
    NEXT.load(Ordering::Relaxed) - 1
}

impl Id {
    #[inline]
    pub(super) fn current() -> Id {
        ID_FAST.with(|id| match id.get() {
            Some(id) => id,
            None => Self::allocate(id),
        })
    }

    #[cold]
    fn allocate(fast: &Cell<Option<Id>>) -> Id {
        let id = if let Ok(mut map) = FREE.try_lock()
            && let Some(cmp::Reverse(id)) = map.pop()
        {
            id
        } else {
            NonZeroUsize::new(NEXT.fetch_add(1, Ordering::Relaxed)).unwrap()
        };

        assert!(id.get() <= MAX);

        let id = Id(id);
        fast.set(Some(id));
        ID_SLOW.set(Some(Guard(id)));
        id
    }
}

impl From<Id> for usize {
    fn from(Id(id): Id) -> Self {
        id.get() - 1
    }
}

struct Guard(Id);

impl Drop for Guard {
    fn drop(&mut self) {
        ID_FAST.try_with(|id| id.set(None)).ok();
        FREE.lock().unwrap().push(cmp::Reverse(self.0.0));
    }
}
