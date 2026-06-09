use core::marker::PhantomData;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::Order;
use crate::raw;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::cursor::path;
use crate::raw::iter::EntryIter;
use crate::raw::iter::Range;
use crate::raw::iter::RangeIter;
use crate::raw::iter::ValueIter;
use crate::raw::key::Read as _;

pub(crate) struct Shard<'g, 'k, K, R = RangeFull>
where
    K: Key,
{
    root: *mut Atomic<Edge<K::Edge>>,
    edge: ribbit::Packed<Edge<K::Edge>>,
    prefix: K::Read<'k>,
    range: R,
    _global: PhantomData<&'g Atomic<Edge<K::Edge>>>,
}

impl<'g, 'k, K, R> Shard<'g, 'k, K, R>
where
    K: Key,
    R: raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub(crate) unsafe fn new_all(root: &'g Atomic<Edge<K::Edge>>) -> Shard<'g, 'k, K, RangeFull> {
        let edge = root.load_packed(Ordering::Relaxed);
        unsafe { Shard::new(root as *const _ as *mut _, edge, K::Read::default(), ..) }
    }

    pub(crate) unsafe fn new_prefix(
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
    ) -> Shard<'g, 'k, K, RangeFull> {
        let mut cursor = unsafe { Cursor::<_, path::Discard>::new(root, prefix) };
        let Some(edge) = cursor.traverse_prefix() else {
            return unsafe { Shard::new(core::ptr::null_mut(), Edge::NULL, prefix, ..) };
        };
        let len = cursor.len();
        let prefix = prefix.prefix(len);
        unsafe { Shard::new(cursor.edge() as *const _ as *mut _, edge, prefix, ..) }
    }

    pub(crate) unsafe fn new_range(
        root: &'g Atomic<Edge<K::Edge>>,
        range: R,
        prefix: K::Read<'k>,
    ) -> Shard<'g, 'k, K, R>
    where
        R: Range<K::Read<'k>>,
    {
        validate_eq!(prefix, range.common_prefix());
        let mut cursor = unsafe { Cursor::<_, path::Discard>::new(root, prefix) };
        let Some(edge) = cursor.traverse_prefix() else {
            return unsafe { Shard::new(core::ptr::null_mut(), Edge::NULL, prefix, range) };
        };
        let len = cursor.len();
        let prefix = prefix.prefix(len);
        unsafe { Shard::new(cursor.edge() as *const _ as *mut _, edge, prefix, range) }
    }

    #[inline]
    unsafe fn new(
        root: *mut Atomic<Edge<K::Edge>>,
        edge: ribbit::Packed<Edge<K::Edge>>,
        prefix: K::Read<'k>,
        range: R,
    ) -> Shard<'g, 'k, K, R> {
        Shard {
            root,
            edge,
            prefix,
            range,
            _global: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn entries<O: Order>(&self, sort: bool) -> EntryIter<'g, 'k, K, R, O> {
        EntryIter(unsafe {
            RangeIter::new_unchecked(self.root, self.edge, self.prefix, sort, &self.range)
        })
    }

    #[inline]
    pub(crate) fn values<O: Order>(&self, sort: bool) -> ValueIter<'g, 'k, K, R, O> {
        ValueIter(unsafe {
            RangeIter::new_unchecked(self.root, self.edge, self.prefix, sort, &self.range)
        })
    }
}
