use core::cell::Cell;
use core::marker::PhantomData;

use ribbit::atomic::Atomic128;

use crate::iter::PostorderIter;
use crate::iter::PrefixIter;
use crate::iter::Sort;
use crate::stat;
use crate::Edge;
use crate::Key;
use crate::Value;

#[repr(transparent)]
pub struct Map<K, V: Value> {
    root: Atomic128<Edge<V>>,
    _not_sync: PhantomData<Cell<()>>,
    _key: PhantomData<K>,
}

impl<K, V: Value> Default for Map<K, V> {
    fn default() -> Self {
        Self {
            root: Atomic128::default(),
            _not_sync: PhantomData,
            _key: PhantomData,
        }
    }
}

impl<K, V: Value> Map<K, V> {
    pub(crate) fn root(&self) -> &Atomic128<Edge<V>> {
        &self.root
    }

    pub(crate) fn postorder<'g>(&'g self) -> PostorderIter<'g, V> {
        unsafe { PostorderIter::new(&self.root) }
    }
}

impl<K: Key, V: Value> Map<K, V> {
    #[expect(unused_variables)]
    #[inline]
    pub fn get(&self, key: K::Borrow<'_>) -> Option<u64> {
        let mut r = K::Read::from(key);
        let mut edge_ref = self.root();

        loop {
            let edge = edge_ref.load_packed(Ordering::Relaxed);
            let meta = edge.meta();

            let _ = meta.key().match_exact(&mut r)?;

            match edge.child()? {
                edge::Child::Node(node) => {
                    let byte = r.next()?;
                    let node = unsafe { node.into_ref_unchecked() };
                    edge_ref = node.get(byte)?;
                }
                edge::Child::Value(value) => {
                    return Some(value.raw());
                }
            }
        }

    }

    #[expect(unused_variables)]
    #[inline]
    pub fn insert(&mut self, key: K::Borrow<'_>, value: u64) -> Option<u64> {
        // how to do structural expansions?
        todo!()
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn remove(&mut self, key: K::Borrow<'_>) -> Option<u64> {
        unsafe { self.write(key, |_old| Edge::DEFAULT) }
    }

    #[expect(unused_variables)]
    #[inline]
    pub fn update(&mut self, key: K::Borrow<'_>, value: u64) -> Option<u64> {
        unsafe { self.write(key, |old| {
            let packed = unsafe { ribbit::Packed::<edge::Value<V>>::new_unchecked(value) };
            old.with_value(packed)
        }) }
    }

    #[inline]
    unsafe fn write<F>(
        &mut self,
        key: K::Borrow<'_>,
        mut exchange: F,
    ) -> Option<V>
    where
        F: FnMut(ribbit::Packed<Edge<V>>) -> ribbit::Packed<Edge<V>>,
    {
        let mut r = K::Read::from(key);
        let mut edge_ref = self.root();

        loop {
            let old = edge_ref.load_packed(Ordering::Relaxed);
            let meta = old.meta();

            let _ = meta.match_exact(&mut r);

            match old.child()? {
                edge::Child::Node(node) => {
                    let b = r.next()?;
                    let node = unsafe { node.into_ref_unchecked() };
                    edge_ref = node.get(b)?;
                }
                edge::Child::Value(value) => {
                    let new = exchange(old);
                    edge_ref.store_packed(new, Ordering::Relaxed);

                    let old_value = { old.as_value().unwrap_unchecked() }.raw();
                    return Some(old_value);
                }
            }
        }
    }

    pub fn iter<S: Sort>(&self) -> Iter<'_, K, V, S> {
        Iter(unsafe { PrefixIter::new_unchecked(&self.root, K::Write::default()) })
    }
}

pub struct Iter<'g, K: Key, V, S: Sort>(PrefixIter<'g, 'static, K::Write, V, S>);

impl<'g, K, V, S> Iter<'g, K, V, S>
where
    K: Key,
    V: Value,
    S: Sort,
{
    #[inline]
    pub fn lend(&mut self) -> Option<(K::Borrow<'_>, V)> {
        self.0.lend().map(|(key, value)| {
            (unsafe { K::borrow_writer_unchecked(key) }, unsafe {
                // FIXME: borrow without guard
                V::from_data(value)
            })
        })
    }
}

impl<K, V, S> Iterator for Iter<'_, K, V, S>
where
    K: Key,
    V: Value,
    S: crate::iter::Sort,
{
    type Item = (K, V);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.0.lend().map(|(key, value)| {
            (unsafe { K::from_writer_unchecked(key.clone()) }, unsafe {
                V::from_data(value)
            })
        })
    }
}

impl<K, V: Value> Drop for Map<K, V> {
    fn drop(&mut self) {
        self.postorder().for_each(|edge, _| unsafe {
            edge.deallocate(stat::Counter::FreeDrop);
        })
    }
}
