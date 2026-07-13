use core::marker::PhantomData;
use core::ops::ControlFlow;

use crate::Key;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw;
use crate::raw::iter::Order;

/// Immutable reference to a subtree rooted at a key prefix, optionally bounded by a key range.
pub struct Shard<'g, 'k, K: Key, V, R, G> {
    _guard: G,
    inner: raw::Shard<'g, 'k, K, R>,
    _value: PhantomData<V>,
}

impl<'g, 'k, K, V, R, G> Shard<'g, 'k, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    G: smr::Guard<V>,
{
    #[inline]
    pub(super) unsafe fn new(
        guard: G,
        prefix: raw::Shard<'g, 'k, K, R>,
    ) -> Shard<'g, 'k, K, V, R, G> {
        Shard {
            _guard: guard,
            inner: prefix,
            _value: PhantomData,
        }
    }
}

impl<'g, 'k, K, V, R, G> Shard<'g, 'k, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    G: smr::Guard<V>,
{
    /// Get an iterator over keys and immutable references to values in `O` order.
    #[inline]
    pub fn entries(&self, order: Order) -> EntryIter<'_, 'k, K, V, R> {
        EntryIter {
            inner: self.inner.entries(Some(order)),
            value: 0,
            _value: PhantomData,
        }
    }

    /// Get an iterator over immutable references to values in `O` order.
    #[inline]
    pub fn values(&self, order: Order) -> ValueIter<'_, 'k, K, V, R> {
        ValueIter {
            inner: self.inner.values(Some(order)),
            value: 0,
            _value: PhantomData,
        }
    }
}

/// Iterator over keys and references to values.
pub struct EntryIter<'g, 'k, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>> {
    inner: raw::iter::EntryIter<'g, 'k, K, R>,
    value: u64,
    _value: PhantomData<V>,
}

impl<'g, 'k, K, V, R> EntryIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    /// Lending equivalent to [`Iterator::next`] that borrows the current key and
    /// value from this [`EntryIter`].
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Insert<'_>, &V::Borrowed)> {
        self.inner.lend().map(|(key, value, _)| {
            self.value = value;

            // Synchronizes with either:
            // - `Ordering::Release` compare_exchange in `upsert_with_raw`
            // - `V::Release` compare_exchange in `update_with_raw`
            if let Some(acquire) = V::ACQUIRE {
                crate::sync::atomic::fence(acquire);
            }

            (key, unsafe { V::borrow_from_raw_unchecked(&self.value) })
        })
    }

    /// Internal iteration over keys and immutable references to values.
    #[inline]
    pub fn try_fold<F, B, C>(mut self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, (K::Insert<'_>, &V::Borrowed)) -> ControlFlow<B, C>,
    {
        self.inner.try_fold(init, |acc, (key, value, _)| {
            self.value = value;

            // Synchronizes with either:
            // - `Ordering::Release` compare_exchange in `upsert_with_raw`
            // - `V::Release` compare_exchange in `update_with_raw`
            if let Some(acquire) = V::ACQUIRE {
                crate::sync::atomic::fence(acquire);
            }

            apply(
                acc,
                (key, unsafe { V::borrow_from_raw_unchecked(&self.value) }),
            )
        })
    }
}

impl<'g, 'k, K, V, R> Iterator for EntryIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    V::Borrowed: Clone,
    R: raw::iter::Range<K::Read<'k>>,
{
    type Item = (K, V::Borrowed);

    // FIXME: specialize for `Arc` values
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
            .map(|(key, value)| (K::insert_to_key(key), value.clone()))
    }
}

/// Iterator over references to values.
pub struct ValueIter<'g, 'k, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>> {
    inner: raw::iter::ValueIter<'g, 'k, K, R>,
    value: u64,
    _value: PhantomData<V>,
}

impl<'g, 'k, K, V, R> ValueIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    /// Lending equivalent to [`Iterator::next`] that borrows the current value from this [`EntryIter`].
    #[inline]
    pub fn lend(&mut self) -> Option<&V::Borrowed> {
        self.inner.lend().map(|(value, _)| {
            self.value = value;

            // Synchronizes with either:
            // - `Ordering::Release` compare_exchange in `upsert_with_raw`
            // - `V::Release` compare_exchange in `update_with_raw`
            if let Some(acquire) = V::ACQUIRE {
                crate::sync::atomic::fence(acquire);
            }

            unsafe { V::borrow_from_raw_unchecked(&self.value) }
        })
    }

    /// Internal iteration over immutable references to values.
    #[inline]
    pub fn try_fold<F: FnMut(C, &V::Borrowed) -> ControlFlow<B, C>, B, C>(
        mut self,
        init: C,
        mut apply: F,
    ) -> ControlFlow<B, C> {
        self.inner.try_fold(init, |acc, (value, _)| {
            self.value = value;

            // Synchronizes with either:
            // - `Ordering::Release` compare_exchange in `upsert_with_raw`
            // - `V::Release` compare_exchange in `update_with_raw`
            if let Some(acquire) = V::ACQUIRE {
                crate::sync::atomic::fence(acquire);
            }

            apply(acc, unsafe { V::borrow_from_raw_unchecked(&self.value) })
        })
    }
}

impl<'g, 'k, K, V, R> Iterator for ValueIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    V::Borrowed: Clone,
    R: raw::iter::Range<K::Read<'k>>,
{
    type Item = V::Borrowed;

    // FIXME: specialize for `Arc` values
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().cloned()
    }
}
