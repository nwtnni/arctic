use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::Deref;

use crate::raw;
use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::iter::Order;
use crate::sequential::Value;

/// Immutable reference to a subtree rooted at a key prefix,
/// optionally bounded by a key range.
pub struct Shard<'g, 'k, K: Key, V, R> {
    inner: raw::Shard<'g, 'k, K, R>,
    _value: PhantomData<&'g V>,
}

impl<'g, 'k, K, V, R> Shard<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub(crate) unsafe fn new(prefix: raw::Shard<'g, 'k, K, R>) -> Self {
        Self {
            inner: prefix,
            _value: PhantomData,
        }
    }

    /// Get an iterator over keys and immutable references to values in `order`.
    #[inline]
    pub fn entries(&self, order: Order) -> EntryIter<'g, 'k, K, V, R> {
        EntryIter {
            inner: self.inner.entries(Some(order)),
            _value: PhantomData,
        }
    }

    /// Get an iterator over immutable references to values in `order`.
    #[inline]
    pub fn values(&self, order: Order) -> ValueIter<'g, 'k, K, V, R> {
        ValueIter {
            inner: self.inner.values(Some(order)),
            _value: PhantomData,
        }
    }
}

/// Mutable reference to a subtree rooted at a key prefix,
/// optionally bounded by a key range.
pub struct ShardMut<'g, 'k, K: Key, V, R>(Shard<'g, 'k, K, V, R>);

impl<'g, 'k, K, V, R> ShardMut<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub(crate) unsafe fn new(prefix: Shard<'g, 'k, K, V, R>) -> Self {
        Self(prefix)
    }

    /// Get an iterator over keys and mutable references to values in `O` order.
    #[inline]
    pub fn entries_mut(&mut self, order: Order) -> EntryIterMut<'g, 'k, K, V, R> {
        EntryIterMut {
            inner: self.0.inner.entries(Some(order)),
            _value: PhantomData,
        }
    }

    /// Get an iterator over mutable references to values in `O` order.
    #[inline]
    pub fn values_mut(&mut self, order: Order) -> ValueIterMut<'g, 'k, K, V, R> {
        ValueIterMut {
            inner: self.0.inner.values(Some(order)),
            _value: PhantomData,
        }
    }
}

impl<'g, 'k, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>> Deref
    for ShardMut<'g, 'k, K, V, R>
{
    type Target = Shard<'g, 'k, K, V, R>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Iterator over keys and immutable references to values.
pub struct EntryIter<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>> {
    inner: raw::iter::EntryIter<'g, 'k, K, R>,
    _value: PhantomData<&'g V>,
}

impl<'g, 'k, K, V, R> EntryIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    /// Lending equivalent to [`Iterator::next`] that borrows the current key from this [`EntryIter`].
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Insert<'_>, &'g V)> {
        self.inner.lend().map(|(key, _, edge)| {
            (key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_ref()
            })
        })
    }

    /// Internal iteration over keys and immutable references to values.
    #[inline]
    pub fn try_fold<F, B, C>(self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, (K::Insert<'_>, &'g V)) -> ControlFlow<B, C>,
    {
        self.inner.try_fold(init, |acc, (key, _, edge)| {
            apply(
                acc,
                (key, unsafe {
                    Edge::as_value_unchecked(edge).cast::<V>().as_ref()
                }),
            )
        })
    }
}

impl<'g, 'k, K, V, R> Iterator for EntryIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    type Item = (K, &'g V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
            .map(|(key, value)| (K::insert_to_key(key), value))
    }
}

/// Iterator over keys and mutable references to values.
pub struct EntryIterMut<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>> {
    inner: raw::iter::EntryIter<'g, 'k, K, R>,
    _value: PhantomData<&'g mut V>,
}

impl<'g, 'k, K, V, R> EntryIterMut<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    /// Lending equivalent to [`Iterator::next`] that borrows the current key from this [`EntryIter`].
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Insert<'_>, &'g mut V)> {
        self.inner.lend().map(|(key, _, edge)| {
            (key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_mut()
            })
        })
    }

    /// Internal iteration over keys and mutable references to values.
    #[inline]
    pub fn try_fold<F, B, C>(self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, (K::Insert<'_>, &'g mut V)) -> ControlFlow<B, C>,
    {
        self.inner.try_fold(init, |acc, (key, _, edge)| {
            apply(
                acc,
                (key, unsafe {
                    Edge::as_value_unchecked(edge).cast::<V>().as_mut()
                }),
            )
        })
    }
}

impl<'g, 'k, K, V, R> Iterator for EntryIterMut<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    type Item = (K, &'g mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
            .map(|(key, value)| (K::insert_to_key(key), value))
    }
}

/// Iterator over references to values.
pub struct ValueIter<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>> {
    inner: raw::iter::ValueIter<'g, 'k, K, R>,
    _value: PhantomData<&'g V>,
}

impl<'g, 'k, K, V, R> ValueIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    /// Internal iteration over immutable references to values.
    #[inline]
    pub fn try_fold<F, B, C>(self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, &'g V) -> ControlFlow<B, C>,
    {
        self.inner.try_fold(init, |acc, (_, edge)| {
            apply(acc, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_ref()
            })
        })
    }
}

impl<'g, 'k, K, V, R> Iterator for ValueIter<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    type Item = &'g V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .lend()
            .map(|(_, edge)| unsafe { Edge::as_value_unchecked(edge).cast::<V>().as_ref() })
    }
}

/// Iterator over mutable references to values.
pub struct ValueIterMut<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>> {
    inner: raw::iter::ValueIter<'g, 'k, K, R>,
    _value: PhantomData<&'g mut V>,
}

impl<'g, 'k, K, V, R> ValueIterMut<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    /// Internal iteration over mutable references to values.
    #[inline]
    pub fn try_fold<F, B, C>(self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, &'g mut V) -> ControlFlow<B, C>,
    {
        self.inner.try_fold(init, |acc, (_, edge)| {
            apply(acc, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_mut()
            })
        })
    }
}

impl<'g, 'k, K, V, R> Iterator for ValueIterMut<'g, 'k, K, V, R>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
{
    type Item = &'g mut V;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .lend()
            .map(|(_, edge)| unsafe { Edge::as_value_unchecked(edge).cast::<V>().as_mut() })
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;
    use core::ops::ControlFlow;

    use crate::Order;
    use crate::sequential::Map;

    #[test]
    fn indirect_values_mut() {
        let mut map = Map::<u64, _>::default();

        for i in 0..1024 {
            map.upsert(i, Box::new(i)).unwrap_err();
        }

        map.all_mut()
            .values_mut(Order::Ascend)
            .try_fold((), |(), value| {
                **value += 1;
                ControlFlow::<Infallible>::Continue(())
            });

        map.all()
            .entries(Order::Descend)
            .try_fold((), |(), (key, value)| {
                assert_eq!(key + 1, **value);
                ControlFlow::<Infallible>::Continue(())
            });
    }

    #[test]
    fn direct_values_mut() {
        let mut map = Map::<u64, _>::default();

        for i in 0..1024 {
            map.upsert(i, i).unwrap_err();
        }

        map.all_mut()
            .values_mut(Order::Ascend)
            .try_fold((), |(), value| {
                *value += 1;
                ControlFlow::<Infallible>::Continue(())
            });

        map.all()
            .entries(Order::Descend)
            .try_fold((), |(), (key, value)| {
                assert_eq!(key + 1, *value);
                ControlFlow::<Infallible>::Continue(())
            });
    }
}
