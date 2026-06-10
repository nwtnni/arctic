use core::marker::PhantomData;
use core::ops::ControlFlow;

use crate::Key;
use crate::Order;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw;

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
    pub fn entries<O: Order>(&self) -> EntryIter<'_, 'k, K, V, R, O, G> {
        EntryIter {
            inner: self.inner.entries::<O>(true),
            value: 0,
            _guard: PhantomData,
            _value: PhantomData,
        }
    }

    /// Get an iterator over immutable references to values in `O` order.
    #[inline]
    pub fn values<O: Order>(&self) -> ValueIter<'_, 'k, K, V, R, O, G> {
        ValueIter {
            inner: self.inner.values::<O>(true),
            value: 0,
            _guard: PhantomData,
            _value: PhantomData,
        }
    }
}

/// Iterator over keys and references to values.
pub struct EntryIter<'g, 'k, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>, O, G> {
    inner: raw::iter::EntryIter<'g, 'k, K, R, O>,
    value: u64,
    _guard: PhantomData<&'g G>,
    _value: PhantomData<V>,
}

impl<'g, 'k, K, V, R, O, G> EntryIter<'g, 'k, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    /// Lending equivalent to [`Iterator::next`] that borrows the current key and
    /// value from this [`EntryIter`].
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Insert<'_>, &V::Target)> {
        self.inner.lend().map(|(key, value, _)| {
            self.value = value;
            (key, unsafe { V::target_from_raw(&self.value) })
        })
    }

    /// Internal iteration over keys and immutable references to values.
    #[inline]
    pub fn for_each_internal<F: FnMut((K::Insert<'_>, &V::Target)) -> ControlFlow<()>>(
        mut self,
        mut apply: F,
    ) {
        self.inner.for_each_internal(|(key, value, _)| {
            self.value = value;
            apply((key, unsafe { V::target_from_raw(&self.value) }))
        })
    }
}

impl<'g, 'k, K, V, R, O, G> Iterator for EntryIter<'g, 'k, K, V, R, O, G>
where
    K: Key,
    V: Value,
    V::Target: Clone,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    type Item = (K, V::Target);

    // FIXME: specialize for `Arc` values
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
            .map(|(key, value)| (K::insert_to_key(key), value.clone()))
    }
}

/// Iterator over references to values.
pub struct ValueIter<'g, 'k, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>, O, G> {
    inner: raw::iter::ValueIter<'g, 'k, K, R, O>,
    value: u64,
    _guard: PhantomData<&'g G>,
    _value: PhantomData<V>,
}

impl<'g, 'k, K, V, R, O, G> ValueIter<'g, 'k, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    /// Lending equivalent to [`Iterator::next`] that borrows the current value from this [`EntryIter`].
    #[inline]
    pub fn lend(&mut self) -> Option<&V::Target> {
        self.inner.lend().map(|(value, _)| {
            self.value = value;
            unsafe { V::target_from_raw(&self.value) }
        })
    }

    /// Internal iteration over immutable references to values.
    #[inline]
    pub fn for_each_internal<F: FnMut(&V::Target) -> ControlFlow<()>>(mut self, mut apply: F) {
        self.inner.for_each_internal(|(value, _)| {
            self.value = value;
            apply(unsafe { V::target_from_raw(&self.value) })
        })
    }
}

impl<'g, 'k, K, V, R, O, G> Iterator for ValueIter<'g, 'k, K, V, R, O, G>
where
    K: Key,
    V: Value,
    V::Target: Clone,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    type Item = V::Target;

    // FIXME: specialize for `Arc` values
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().cloned()
    }
}
