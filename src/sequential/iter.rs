use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::Deref;

use crate::Order;
use crate::raw;
use crate::raw::Edge;
use crate::raw::Key;
use crate::sequential::Value;

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

    #[inline]
    pub fn entries<O: Order>(&self) -> EntryIter<'g, 'k, K, V, R, O> {
        EntryIter {
            inner: self.inner.entries::<O>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values<O: Order>(&self) -> ValueIter<'g, 'k, K, V, R, O> {
        ValueIter {
            inner: self.inner.values::<O>(),
            _value: PhantomData,
        }
    }
}

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

    #[inline]
    pub fn entries_mut<O: Order>(&mut self) -> EntryIterMut<'g, 'k, K, V, R, O> {
        EntryIterMut {
            inner: self.0.inner.entries::<O>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values_mut<O: Order>(&mut self) -> ValueIterMut<'g, 'k, K, V, R, O> {
        ValueIterMut {
            inner: self.0.inner.values::<O>(),
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

/// Iterator over keys and references to values.
pub struct EntryIter<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::EntryIter<'g, 'k, K, R, O>,
    _value: PhantomData<&'g V>,
}

impl<'g, 'k, K, V, R, O> EntryIter<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Insert<'_>, &'g V)> {
        self.inner.lend().map(|(key, _, edge)| {
            (key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_ref()
            })
        })
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((K::Insert<'_>, &'g V)) -> ControlFlow<()>>(
        self,
        mut apply: F,
    ) {
        self.inner.for_each_internal(|(key, _, edge)| {
            apply((key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_ref()
            }))
        })
    }
}

impl<'g, 'k, K, V, R, O> Iterator for EntryIter<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    type Item = (K, &'g V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
            .map(|(key, value)| (K::insert_to_key(key), value))
    }
}

/// Iterator over keys and mutable references to values.
pub struct EntryIterMut<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::EntryIter<'g, 'k, K, R, O>,
    _value: PhantomData<&'g mut V>,
}

impl<'g, 'k, K, V, R, O> EntryIterMut<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Insert<'_>, &'g mut V)> {
        self.inner.lend().map(|(key, _, edge)| {
            (key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_mut()
            })
        })
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((K::Insert<'_>, &'g mut V)) -> ControlFlow<()>>(
        self,
        mut apply: F,
    ) {
        self.inner.for_each_internal(|(key, _, edge)| {
            apply((key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_mut()
            }))
        })
    }
}

impl<'g, 'k, K, V, R, O> Iterator for EntryIterMut<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    type Item = (K, &'g mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend()
            .map(|(key, value)| (K::insert_to_key(key), value))
    }
}

/// Iterator over references to values.
pub struct ValueIter<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::ValueIter<'g, 'k, K, R, O>,
    _value: PhantomData<&'g V>,
}

impl<'g, 'k, K, V, R, O> ValueIter<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub fn for_each_internal<F: FnMut(&'g V) -> ControlFlow<()>>(self, mut apply: F) {
        self.inner.for_each_internal(|(_, edge)| {
            apply(unsafe { Edge::as_value_unchecked(edge).cast::<V>().as_ref() })
        })
    }
}

impl<'g, 'k, K, V, R, O> Iterator for ValueIter<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
    O: Order,
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
pub struct ValueIterMut<'g, 'k, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::ValueIter<'g, 'k, K, R, O>,
    _value: PhantomData<&'g mut V>,
}

impl<'g, 'k, K, V, R, O> ValueIterMut<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub fn for_each_internal<F: FnMut(&'g mut V) -> ControlFlow<()>>(self, mut apply: F) {
        self.inner.for_each_internal(|(_, edge)| {
            apply(unsafe { Edge::as_value_unchecked(edge).cast::<V>().as_mut() })
        })
    }
}

impl<'g, 'k, K, V, R, O> Iterator for ValueIterMut<'g, 'k, K, V, R, O>
where
    K: Key,
    V: Value,
    R: crate::raw::iter::Range<K::Read<'k>>,
    O: Order,
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
    use core::ops::ControlFlow;

    use crate::Ascend;
    use crate::Descend;
    use crate::sequential::Map;

    #[test]
    fn indirect_values_mut() {
        let mut map = Map::<u64, _>::default();

        for i in 0..1024 {
            map.upsert(&i, Box::new(i)).unwrap();
        }

        map.all_mut()
            .values_mut::<Ascend>()
            .for_each_internal(|value| {
                **value += 1;
                ControlFlow::Continue(())
            });

        map.all()
            .entries::<Descend>()
            .for_each_internal(|(key, value)| {
                assert_eq!(key + 1, **value);
                ControlFlow::Continue(())
            });
    }

    #[test]
    fn direct_values_mut() {
        let mut map = Map::<u64, _>::default();

        for i in 0..1024 {
            map.upsert(&i, i).unwrap();
        }

        map.all_mut()
            .values_mut::<Ascend>()
            .for_each_internal(|value| {
                *value += 1;
                ControlFlow::Continue(())
            });

        map.all()
            .entries::<Descend>()
            .for_each_internal(|(key, value)| {
                assert_eq!(key + 1, *value);
                ControlFlow::Continue(())
            });
    }
}
