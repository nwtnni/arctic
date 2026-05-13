mod entry;
mod iter;
mod value;

pub use entry::Entry;
pub use iter::EntryIter;
pub use iter::EntryIterMut;
pub use iter::Prefix;
pub use iter::PrefixMut;
pub use iter::ValueIter;
pub use iter::ValueIterMut;
pub use value::Value;

use core::cell::Cell;
use core::marker::PhantomData;
use core::ops::RangeFull;

use crate::Ascend;
use crate::Key;
use crate::raw;
use crate::raw::cursor::path;
use crate::stat;

#[repr(transparent)]
pub struct Map<K: Key, V: Value> {
    pub(crate) raw: raw::Map<K>,
    _not_sync: PhantomData<Cell<()>>,
    _value: PhantomData<V>,
}

impl<K, V> Default for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn default() -> Self {
        Self {
            raw: raw::Map::default(),
            _not_sync: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K, V> Map<K, V>
where
    K: Key,
    V: Value,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &K::Borrowed) -> Option<&V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(key) };
        cursor.traverse_get()?;
        Some(unsafe { cursor.as_value_unchecked().cast::<V>().as_ref() })
    }

    pub fn get_mut(&mut self, key: &K::Borrowed) -> Option<&mut V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(key) };
        cursor.traverse_get()?;
        Some(unsafe { cursor.as_value_unchecked().cast::<V>().as_mut() })
    }

    pub fn upsert(&mut self, key: &K::Borrowed, value: V) -> Result<&mut V, (V, &mut V)> {
        match self.entry(key) {
            Entry::Vacant(entry) => Ok(entry.insert(value)),
            Entry::Occupied(mut entry) => {
                let old = entry.insert(value);
                Err((old, entry.into_mut()))
            }
        }
    }

    pub fn insert(&mut self, key: &K::Borrowed, value: V) -> Result<&mut V, (&mut V, V)> {
        match self.entry(key) {
            Entry::Vacant(entry) => Ok(entry.insert(value)),
            Entry::Occupied(entry) => Err((entry.into_mut(), value)),
        }
    }

    pub fn update(&mut self, key: &K::Borrowed, value: V) -> Result<(V, &mut V), V> {
        match self.entry(key) {
            Entry::Vacant(_) => Err(value),
            Entry::Occupied(mut entry) => {
                let old = entry.insert(value);
                Ok((old, entry.into_mut()))
            }
        }
    }

    pub fn entry<'k>(&mut self, key: &'k K::Borrowed) -> Entry<'_, 'k, K, V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(key) };

        match cursor.traverse_insert() {
            raw::cursor::Insert::Value {
                old_value: Some(_),
                old: _,
            } => Entry::Occupied(entry::Occupied {
                value: unsafe { cursor.as_value_unchecked().cast::<V>() },
                _value: PhantomData,
            }),

            raw::cursor::Insert::Value {
                old_value: None,
                old: _,
            } => Entry::Vacant(entry::Vacant {
                cursor,
                replace: false,
                _value: PhantomData,
            }),

            raw::cursor::Insert::Replace { .. } => Entry::Vacant(entry::Vacant {
                cursor,
                replace: true,
                _value: PhantomData,
            }),
        }
    }

    #[inline]
    pub fn all(&self) -> Prefix<'static, '_, K, V, RangeFull> {
        unsafe { Prefix::new(self.raw.all()) }
    }

    #[inline]
    pub fn prefix<'k>(
        &self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<Prefix<'k, '_, K, V, RangeFull>> {
        Some(unsafe { Prefix::new(self.raw.prefix(prefix)?) })
    }

    #[inline]
    pub fn range<'k, R>(&self, range: R) -> Option<Prefix<'k, '_, K, V, R>>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        Some(unsafe { iter::Prefix::new(self.raw.range(range, prefix)?) })
    }

    #[inline]
    pub fn all_mut(&mut self) -> PrefixMut<'static, '_, K, V, RangeFull> {
        unsafe { PrefixMut::new(self.all()) }
    }

    #[inline]
    pub fn prefix_mut<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<PrefixMut<'k, '_, K, V, RangeFull>> {
        Some(unsafe { PrefixMut::new(self.prefix(prefix)?) })
    }

    #[inline]
    pub fn range_mut<'k, R>(&mut self, range: R) -> Option<PrefixMut<'k, '_, K, V, R>>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        Some(unsafe { PrefixMut::new(self.range(range)?) })
    }
}

impl<'k, K, V> FromIterator<(&'k K::Borrowed, V)> for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn from_iter<T: IntoIterator<Item = (&'k K::Borrowed, V)>>(iter: T) -> Self {
        let mut map = Map::default();
        for (key, value) in iter {
            let _ = map.upsert(key, value);
        }
        map
    }
}

impl<'g, K, V> IntoIterator for &'g Map<K, V>
where
    K: Key,
    V: Value,
{
    type Item = (K, &'g V);
    type IntoIter = EntryIter<'static, 'g, K, V, RangeFull, Ascend>;
    fn into_iter(self) -> Self::IntoIter {
        self.all().entries::<Ascend>()
    }
}

impl<'g, K, V> IntoIterator for &'g mut Map<K, V>
where
    K: Key,
    V: Value,
{
    type Item = (K, &'g mut V);
    type IntoIter = EntryIterMut<'static, 'g, K, V, RangeFull, Ascend>;
    fn into_iter(self) -> Self::IntoIter {
        self.all_mut().entries_mut::<Ascend>()
    }
}

impl<K, V> Drop for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn drop(&mut self) {
        self.raw.postorder().for_each_internal(|edge, _| unsafe {
            edge.deallocate(|value| drop(V::from_raw(value)), stat::Counter::FreeDrop);
        })
    }
}
