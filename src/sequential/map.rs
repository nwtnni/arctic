use core::cell::Cell;
use core::marker::PhantomData;
use core::ops::RangeFull;
use core::ptr::NonNull;

use crate::Ascend;
use crate::raw;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::Key;
use crate::raw::cursor::path;
use crate::raw::edge;
use crate::sequential::EntryIter;
use crate::sequential::EntryIterMut;
use crate::sequential::Shard;
use crate::sequential::ShardMut;
use crate::sequential::Value;
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

    pub fn upsert<'k>(&mut self, key: K::Insert<'k>, value: V) -> Result<&mut V, (V, &mut V)> {
        match self.entry(key) {
            Entry::Vacant(entry) => Ok(entry.insert(value)),
            Entry::Occupied(mut entry) => {
                let old = entry.insert(value);
                Err((old, entry.into_mut()))
            }
        }
    }

    pub fn insert<'k>(&mut self, key: K::Insert<'k>, value: V) -> Result<&mut V, (&mut V, V)> {
        match self.entry(key) {
            Entry::Vacant(entry) => Ok(entry.insert(value)),
            Entry::Occupied(entry) => Err((entry.into_mut(), value)),
        }
    }

    pub fn update<'k>(&mut self, key: K::Insert<'k>, value: V) -> Result<(V, &mut V), V> {
        match self.entry(key) {
            Entry::Vacant(_) => Err(value),
            Entry::Occupied(mut entry) => {
                let old = entry.insert(value);
                Ok((old, entry.into_mut()))
            }
        }
    }

    pub fn entry<'k>(&mut self, key: K::Insert<'k>) -> Entry<'_, 'k, K, V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(key) };

        match cursor.traverse_insert() {
            raw::cursor::Insert::Value {
                value: Some(_),
                edge: _,
            } => Entry::Occupied(Occupied {
                value: unsafe { cursor.as_value_unchecked().cast::<V>() },
                _value: PhantomData,
            }),

            raw::cursor::Insert::Value {
                value: None,
                edge: _,
            } => Entry::Vacant(Vacant {
                cursor,
                replace: false,
                _value: PhantomData,
            }),

            raw::cursor::Insert::Replace { .. } => Entry::Vacant(Vacant {
                cursor,
                replace: true,
                _value: PhantomData,
            }),
        }
    }

    #[inline]
    pub fn all(&self) -> Shard<'_, 'static, K, V, RangeFull> {
        unsafe { Shard::new(self.raw.all()) }
    }

    #[inline]
    pub fn prefix<'k>(
        &self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<Shard<'_, 'k, K, V, RangeFull>> {
        Some(unsafe { Shard::new(self.raw.prefix(prefix)?) })
    }

    #[inline]
    pub fn range<'k, R>(&self, range: R) -> Option<Shard<'_, 'k, K, V, R>>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        Some(unsafe { Shard::new(self.raw.range(range, prefix)?) })
    }

    #[inline]
    pub fn all_mut(&mut self) -> ShardMut<'_, 'static, K, V, RangeFull> {
        unsafe { ShardMut::new(self.all()) }
    }

    #[inline]
    pub fn prefix_mut<'k>(
        &mut self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<ShardMut<'_, 'k, K, V, RangeFull>> {
        Some(unsafe { ShardMut::new(self.prefix(prefix)?) })
    }

    #[inline]
    pub fn range_mut<'k, R>(&mut self, range: R) -> Option<ShardMut<'_, 'k, K, V, R>>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        Some(unsafe { ShardMut::new(self.range(range)?) })
    }
}

impl<'k, K, V> FromIterator<(K::Insert<'k>, V)> for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn from_iter<T: IntoIterator<Item = (K::Insert<'k>, V)>>(iter: T) -> Self {
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
    type IntoIter = EntryIter<'g, 'static, K, V, RangeFull, Ascend>;
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
    type IntoIter = EntryIterMut<'g, 'static, K, V, RangeFull, Ascend>;
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
        self.raw.postorder().for_each_internal(|edge, _| {
            let Some(child) = edge.child() else { return };

            stat::increment(stat::Counter::FreeDrop);

            // SAFETY: we have exclusive access to nodes and values in destructor
            match child {
                edge::Child::Value(value) => drop(unsafe { V::from_raw(value) }),
                edge::Child::Node(node) => unsafe {
                    node.deallocate(stat::Counter::FreeDrop);
                },
            }
        })
    }
}

pub enum Entry<'g, 'k, K, V>
where
    K: Key,
    V: Value + 'g,
{
    Vacant(Vacant<'g, 'k, K, V>),
    Occupied(Occupied<'g, V>),
}

impl<'g, 'k, K: Key, V: Value + 'g> Entry<'g, 'k, K, V> {
    #[inline]
    pub fn or_insert(self, default: V) -> &'g mut V {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default),
        }
    }

    #[inline]
    pub fn or_insert_with<F: FnOnce() -> V>(self, default: F) -> &'g mut V {
        match self {
            Self::Occupied(entry) => entry.into_mut(),
            Self::Vacant(entry) => entry.insert(default()),
        }
    }

    #[inline]
    pub fn and_modify<F>(self, modify: F) -> Self
    where
        F: FnOnce(&mut V),
    {
        match self {
            Self::Occupied(mut entry) => {
                modify(entry.get_mut());
                Self::Occupied(entry)
            }
            Self::Vacant(entry) => Self::Vacant(entry),
        }
    }
}

impl<'g, 'k, K: Key, V: Value + Default + 'g> Entry<'g, 'k, K, V> {
    #[inline]
    pub fn or_default(self) -> &'g mut V {
        self.or_insert_with(V::default)
    }
}

pub struct Vacant<'g, 'k, K: Key, V: Value + 'g> {
    pub(super) cursor: Cursor<'g, K::Read<'k>, path::Discard>,
    pub(super) replace: bool,
    pub(super) _value: PhantomData<&'g mut V>,
}

pub struct Occupied<'g, V: Value + 'g> {
    pub(super) value: NonNull<V>,
    pub(super) _value: PhantomData<&'g mut V>,
}

impl<'g, 'k, K: Key, V: Value + 'g> Vacant<'g, 'k, K, V> {
    #[inline]
    pub fn insert(self, value: V) -> &'g mut V {
        self.insert_entry(value).into_mut()
    }

    pub fn insert_entry(mut self, value: V) -> Occupied<'g, V> {
        let new_value = V::into_raw(value);

        if self.replace {
            let old = unsafe { self.cursor.edge_mut().get_packed() };
            let old_node = old.as_node().expect("Replace implies node");
            let (smo, new) = unsafe { old_node.replace::<false>(old.meta()) };
            // No concurrent operations, so must be node replacement with larger node
            validate_eq!(smo, crate::raw::Smo::ReplaceNode);
            unsafe { self.cursor.edge_mut() }.set_packed(new);
            unsafe { old_node.deallocate(stat::Counter::FreeRetire) };
        }

        match self.cursor.traverse_insert() {
            crate::raw::cursor::Insert::Value {
                value: Some(_),
                edge: _,
            }
            | crate::raw::cursor::Insert::Replace { .. } => unreachable!(),
            crate::raw::cursor::Insert::Value {
                value: None,
                edge: old,
            } => match self.cursor.create_path(old, new_value) {
                Err(Frozen) => unreachable!(),
                Ok((head, tail)) => unsafe {
                    self.cursor.edge_mut().set_packed(head);

                    let value = match tail {
                        None => self.cursor.as_value_unchecked(),
                        Some(tail) => Edge::as_value_unchecked(tail),
                    };

                    Occupied {
                        value: value.cast::<V>(),
                        _value: PhantomData,
                    }
                },
            },
        }
    }
}

impl<'g, V: Value> Occupied<'g, V> {
    #[inline]
    pub fn get(&self) -> &V {
        unsafe { self.value.as_ref() }
    }

    #[inline]
    pub fn get_mut(&mut self) -> &mut V {
        unsafe { self.value.as_mut() }
    }

    #[inline]
    pub fn insert(&mut self, value: V) -> V {
        unsafe { core::mem::replace(self.value.as_mut(), value) }
    }

    #[inline]
    pub fn into_mut(mut self) -> &'g mut V {
        unsafe { self.value.as_mut() }
    }

    #[inline]
    pub fn and_modify<F: FnOnce(&mut V)>(mut self, modify: F) {
        modify(unsafe { self.value.as_mut() })
    }
}
