use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::Deref;

use crate::Order;
use crate::raw;
use crate::raw::Edge;
use crate::raw::Key;
use crate::sequential::Value;

pub struct Prefix<'k, 'g, K: Key, V, R> {
    inner: raw::iter::Prefix<'k, 'g, K, R>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, K, V, R> Prefix<'k, 'g, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub(crate) unsafe fn new(prefix: raw::iter::Prefix<'k, 'g, K, R>) -> Self {
        Self {
            inner: prefix,
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn entries<O: Order>(&self) -> EntryIter<'k, 'g, K, V, R, O> {
        EntryIter {
            inner: self.inner.entries::<O>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values<O: Order>(&self) -> ValueIter<'k, 'g, K, V, R, O> {
        ValueIter {
            inner: self.inner.values::<O>(),
            _value: PhantomData,
        }
    }
}

pub struct PrefixMut<'k, 'g, K: Key, V, R>(Prefix<'k, 'g, K, V, R>);

impl<'k, 'g, K, V, R> PrefixMut<'k, 'g, K, V, R>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
{
    #[inline]
    pub(crate) unsafe fn new(prefix: Prefix<'k, 'g, K, V, R>) -> Self {
        Self(prefix)
    }

    #[inline]
    pub fn entries_mut<O: Order>(&mut self) -> EntryIterMut<'k, 'g, K, V, R, O> {
        EntryIterMut {
            inner: self.0.inner.entries::<O>(),
            _value: PhantomData,
        }
    }

    #[inline]
    pub fn values_mut<O: Order>(&mut self) -> ValueIterMut<'k, 'g, K, V, R, O> {
        ValueIterMut {
            inner: self.0.inner.values::<O>(),
            _value: PhantomData,
        }
    }
}

impl<'k, 'g, K: Key, V: Value, R: raw::iter::Range<K::Read<'k>>> Deref
    for PrefixMut<'k, 'g, K, V, R>
{
    type Target = Prefix<'k, 'g, K, V, R>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Iterator over keys and values
pub struct EntryIter<'k, 'g, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::EntryIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, K, V, R, O> EntryIter<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&K::Borrowed, &'g V)> {
        self.inner.lend().map(|(key, _, edge)| {
            (key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_ref()
            })
        })
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((&K::Borrowed, &'g V)) -> ControlFlow<()>>(
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

impl<'k, 'g, K, V, R, O> Iterator for EntryIter<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    type Item = (K, &'g V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (key.to_owned(), value))
    }
}

pub struct EntryIterMut<'k, 'g, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::EntryIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g mut V>,
}

impl<'k, 'g, K, V, R, O> EntryIterMut<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(&K::Borrowed, &'g mut V)> {
        self.inner.lend().map(|(key, _, edge)| {
            (key, unsafe {
                Edge::as_value_unchecked(edge).cast::<V>().as_mut()
            })
        })
    }

    #[inline]
    pub fn for_each_internal<F: FnMut((&K::Borrowed, &'g mut V)) -> ControlFlow<()>>(
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

impl<'k, 'g, K, V, R, O> Iterator for EntryIterMut<'k, 'g, K, V, R, O>
where
    K: Key,
    V: Value,
    R: raw::iter::Range<K::Read<'k>>,
    O: Order,
{
    type Item = (K, &'g mut V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.lend().map(|(key, value)| (key.to_owned(), value))
    }
}

pub struct ValueIter<'k, 'g, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::ValueIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g V>,
}

impl<'k, 'g, K, V, R, O> ValueIter<'k, 'g, K, V, R, O>
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

impl<'k, 'g, K, V, R, O> Iterator for ValueIter<'k, 'g, K, V, R, O>
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

pub struct ValueIterMut<'k, 'g, K: Key, V, R: raw::iter::Range<K::Read<'k>>, O> {
    inner: raw::iter::ValueIter<'k, 'g, K, R, O>,
    _value: PhantomData<&'g mut V>,
}

impl<'k, 'g, K, V, R, O> ValueIterMut<'k, 'g, K, V, R, O>
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

impl<'k, 'g, K, V, R, O> Iterator for ValueIterMut<'k, 'g, K, V, R, O>
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
