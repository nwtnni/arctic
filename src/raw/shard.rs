use core::marker::PhantomData;
use core::ops::RangeFull;
use core::ptr::NonNull;

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
    root: NonNull<Atomic<Edge<K::Edge>>>,
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
        unsafe { Shard::new(root, K::Read::default(), ..) }
    }

    pub(crate) unsafe fn new_prefix(
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
    ) -> Option<Shard<'g, 'k, K, RangeFull>> {
        let mut cursor = unsafe { Cursor::<_, path::Discard>::new(root, prefix) };
        cursor.traverse_prefix()?;
        let root = cursor.edge();
        let len = cursor.len();
        let prefix = prefix.prefix(len);
        Some(unsafe { Shard::new(root, prefix, ..) })
    }

    pub(crate) unsafe fn new_range(
        root: &'g Atomic<Edge<K::Edge>>,
        range: R,
        prefix: K::Read<'k>,
    ) -> Option<Shard<'g, 'k, K, R>>
    where
        R: Range<K::Read<'k>>,
    {
        validate_eq!(prefix, range.common_prefix());
        let mut cursor = unsafe { Cursor::<_, path::Discard>::new(root, prefix) };
        cursor.traverse_prefix()?;

        let root = cursor.edge();
        let len = cursor.len();
        let prefix = prefix.prefix(len);

        Some(unsafe { Shard::new(root, prefix, range) })
    }

    #[inline]
    unsafe fn new(
        root: &'g Atomic<Edge<K::Edge>>,
        prefix: K::Read<'k>,
        range: R,
    ) -> Shard<'g, 'k, K, R> {
        Shard {
            root: NonNull::from(root),
            prefix,
            range,
            _global: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn entries<O: Order>(&self) -> EntryIter<'g, 'k, K, R, O> {
        EntryIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, &self.range) })
    }

    #[inline]
    pub(crate) fn values<O: Order>(&self) -> ValueIter<'g, 'k, K, R, O> {
        ValueIter(unsafe { RangeIter::new_unchecked(self.root, self.prefix, &self.range) })
    }
}
