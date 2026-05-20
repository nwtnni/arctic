use core::marker::PhantomData;
use core::ops::ControlFlow;

use crate::Order;
use crate::concurrent::Key;
use crate::concurrent::Value;
use crate::concurrent::smr;
use crate::raw;

pub struct Prefix<'k, 'g, K: Key, V, R, G> {
    _guard: G,
    inner: raw::iter::Prefix<'k, 'g, K, R>,
    _value: PhantomData<V>,
}

impl<'k, 'g, K, V, R, G> Prefix<'k, 'g, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    G: smr::Guard<V>,
{
    #[inline]
    pub(super) unsafe fn new(
        guard: G,
        prefix: raw::iter::Prefix<'k, 'g, K, R>,
    ) -> Prefix<'k, 'g, K, V, R, G> {
        Prefix {
            _guard: guard,
            inner: prefix,
            _value: PhantomData,
        }
    }
}

impl<'k, 'g, K, V, R, G> Prefix<'k, 'g, K, V, R, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn entries<O: Order>(&self) -> EntryIter<'k, '_, K, V, R, O, G> {
        EntryIter {
            inner: self.inner.entries::<O>(),
            value: 0,
            _guard: PhantomData,
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values<O: Order>(&self) -> ValueIter<'k, '_, K, V, R, O, G> {
        ValueIter {
            inner: self.inner.values::<O>(),
            value: 0,
            _guard: PhantomData,
            _value: PhantomData,
        }
    }
}

/// Iterator over keys and values
pub struct EntryIter<'k, 'l, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>, O, G> {
    inner: raw::iter::EntryIter<'k, 'l, K, R, O>,
    value: u64,
    _guard: PhantomData<&'l G>,
    _value: PhantomData<V>,
}

impl<'k, 'l, K, V, R, O, G> EntryIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&K::Borrowed, &V::Target)> {
        self.inner.lend().map(|(key, value, _)| {
            self.value = value;
            (key, unsafe { V::target_from_raw(&self.value) })
        })
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((&K::Borrowed, &V::Target)) -> ControlFlow<()>>(
        mut self,
        mut apply: F,
    ) {
        self.inner.for_each_internal(|(key, value, _)| {
            self.value = value;
            apply((key, unsafe { V::target_from_raw(&self.value) }))
        })
    }
}

impl<'k, 'l, K, V, R, O, G> Iterator for EntryIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    V::Target: Clone,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    type Item = (K, V::Target);

    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
            .map(|(key, value)| (key.to_owned(), value.clone()))
    }
}

/// Iterator over values only
pub struct ValueIter<'k, 'l, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>, O, G> {
    inner: raw::iter::ValueIter<'k, 'l, K, R, O>,
    value: u64,
    _guard: PhantomData<&'l G>,
    _value: PhantomData<V>,
}

impl<'k, 'l, K, V, R, O, G> ValueIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    #[inline]
    pub fn lend(&mut self) -> Option<&V::Target> {
        self.inner.lend().map(|(value, _)| {
            self.value = value;
            unsafe { V::target_from_raw(&self.value) }
        })
    }

    #[inline]
    pub fn for_each_internal<F: FnMut(&V::Target) -> ControlFlow<()>>(mut self, mut apply: F) {
        self.inner.for_each_internal(|(value, _)| {
            self.value = value;
            apply(unsafe { V::target_from_raw(&self.value) })
        })
    }
}

impl<'k, 'l, K, V, R, O, G> Iterator for ValueIter<'k, 'l, K, V, R, O, G>
where
    K: Key,
    V: Value,
    V::Target: Clone,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
    G: smr::Guard<V>,
{
    type Item = V::Target;

    fn next(&mut self) -> Option<Self::Item> {
        self.lend().cloned()
    }
}
