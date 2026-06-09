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
    _value: PhantomData<V>,
}

impl<K, V> Default for Map<K, V>
where
    K: Key,
    V: Value,
{
    fn default() -> Self {
        Self::new()
    }
}

/// # Point operations
impl<K, V> Map<K, V>
where
    K: Key,
    V: Value,
{
    /// Constructs a new empty map. Does not allocate.
    #[inline]
    pub const fn new() -> Self {
        Self {
            raw: raw::Map::new(),
            _value: PhantomData,
        }
    }

    /// Returns an immutable reference to the value associated with `key`.
    /// For a mutable reference, see [`Map::get_mut`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential::Map;
    ///
    /// let mut map = Map::<u64, u64>::new();
    /// map.insert(1, 2).expect("Key is not present");
    /// assert_eq!(map.get(&1), Some(&2));
    /// assert_eq!(map.get(&2), None);
    /// ```
    pub fn get(&self, key: &K::Borrowed) -> Option<&V> {
        let reader = K::Read::from(key);
        self.get_impl(reader)
    }

    /// Returns a mutable reference to the value associated with `key`.
    /// For an immutable reference, see [`Map::get`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential::Map;
    ///
    /// let mut map = Map::<u64, u64>::new();
    /// map.insert(1, 2).expect("Key is not present");
    /// let value = map.get_mut(&1).expect("Key is present");
    /// *value = 3;
    /// assert_eq!(map.get(&1), Some(&3));
    /// ```
    pub fn get_mut(&mut self, key: &K::Borrowed) -> Option<&mut V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(key) };
        cursor.traverse_get()?;
        Some(unsafe { cursor.as_value_unchecked().cast::<V>().as_mut() })
    }

    /// If there is a value associated with `key`, update it to `value`.
    ///
    /// Returns `Ok((old_value, &mut new_value))` if the update succeeded,
    /// or else `Err(new_value)` if there was no old value associated with `key`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential::Map;
    ///
    /// let mut map = Map::<[u8; 3], Box<u64>>::new();
    ///
    /// match map.update(&[0, 1, 2], Box::new(5)) {
    ///     Ok(_) => unreachable!(),
    ///     Err(new) => assert_eq!(*new, 5),
    /// }
    ///
    /// map.insert(&[0, 1, 2], Box::new(9));
    ///
    /// match map.update(&[0, 1, 2], Box::new(10)) {
    ///     Ok((old, new)) => {
    ///         assert_eq!(*old, 9);
    ///         assert_eq!(**new, 10);
    ///     },
    ///     Err(_) => unreachable!(),
    /// }
    /// ```
    pub fn update(&mut self, key: &K::Borrowed, value: V) -> Result<(V, &mut V), V> {
        match self.entry_impl(K::Read::from(key)) {
            Entry::Vacant(_) => Err(value),
            Entry::Occupied(mut entry) => {
                let old = entry.insert(value);
                Ok((old, entry.into_mut()))
            }
        }
    }

    /// If there is no value associated with `key`, associate it with `value`.
    /// Note that this is **not** the same behavior as the standard library
    /// (e.g., [`std::collections::BTreeMap::insert`]); see [`Map::upsert`] if
    /// an existing value should be updated instead.
    ///
    /// Returns `Ok(&mut new_value)` if the insert succeeded,
    /// or else `Err((&mut old_value, new_value))` if there is an existing
    /// `old_value` associated with the key.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential::Map;
    /// use arctic::NonNullStr;
    /// use arctic::NonNullString;
    ///
    /// let mut map = Map::<NonNullString, Box<u64>>::new();
    ///
    /// match map.insert(NonNullStr::new("regent").unwrap(), Box::new(3)) {
    ///     Ok(new) => assert_eq!(**new, 3),
    ///     Err(_) => unreachable!(),
    /// }
    ///
    /// match map.insert(NonNullStr::new("regent").unwrap(), Box::new(26)) {
    ///     Ok(_) => unreachable!(),
    ///     Err((old, new)) => {
    ///         assert_eq!(**old, 3);
    ///         assert_eq!(*new, 26);
    ///     },
    /// }
    /// ```
    pub fn insert<'k>(&mut self, key: K::Insert<'k>, value: V) -> Result<&mut V, (&mut V, V)> {
        match self.entry(key) {
            Entry::Vacant(entry) => Ok(entry.insert(value)),
            Entry::Occupied(entry) => Err((entry.into_mut(), value)),
        }
    }

    /// Unconditionally associate `key` with `value`.
    ///
    /// Returns `Ok((old_value, &mut new_value))` if this updated `old_value`,
    /// or `Err(&mut new_value)` if there was no value associated with `key`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential::Map;
    /// use arctic::NonNullStr;
    /// use arctic::NonNullString;
    ///
    /// let mut map = Map::<NonNullString, u64>::new();
    ///
    /// match map.upsert(NonNullStr::new("silent").unwrap(), 2) {
    ///     Ok(_) => unreachable!(),
    ///     Err(new) => assert_eq!(*new, 2),
    /// }
    ///
    /// match map.upsert(NonNullStr::new("silent").unwrap(), 26) {
    ///     Ok((old, new)) => {
    ///         assert_eq!(old, 2);
    ///         assert_eq!(*new, 26);
    ///     },
    ///     Err(_) => unreachable!(),
    /// }
    /// ```
    pub fn upsert<'k>(&mut self, key: K::Insert<'k>, value: V) -> Result<(V, &mut V), &mut V> {
        match self.entry(key) {
            Entry::Vacant(entry) => Err(entry.insert(value)),
            Entry::Occupied(mut entry) => {
                let old = entry.insert(value);
                Ok((old, entry.into_mut()))
            }
        }
    }

    /// Get a logical entry associated with `key`. This is a lazy operation, and does
    /// not allocate or modify the tree structure. (Also see [`std::collections::BTreeMap::entry`].)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential::Map;
    /// use arctic::NonNullStr;
    /// use arctic::NonNullString;
    ///
    /// let mut counter = Map::<NonNullString, u64>::new();
    ///
    /// for key in ["claw", "claw", "hotfix", "hologram", "claw"]
    ///     .into_iter()
    ///     .map(NonNullStr::new)
    ///     .map(Option::unwrap)
    /// {
    ///     *counter.entry(key).or_default() += 1;
    /// }
    ///
    /// assert_eq!(*counter.get(NonNullStr::new("hologram").unwrap()).unwrap(), 1);
    /// assert_eq!(*counter.get(NonNullStr::new("hotfix").unwrap()).unwrap(), 1);
    /// assert_eq!(*counter.get(NonNullStr::new("claw").unwrap()).unwrap(), 3);
    /// ```
    pub fn entry<'k>(&mut self, key: K::Insert<'k>) -> Entry<'_, 'k, K, V> {
        self.entry_impl(K::insert_as_read(key))
    }
}

/// # Range and prefix operations
impl<K, V> Map<K, V>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub fn all(&self) -> Shard<'_, 'static, K, V, RangeFull> {
        unsafe { Shard::new(self.raw.all()) }
    }

    #[inline]
    pub fn prefix<'k>(&self, prefix: K::Read<'k>) -> Shard<'_, 'k, K, V, RangeFull> {
        unsafe { Shard::new(self.raw.prefix(prefix)) }
    }

    #[inline]
    pub fn range<'k, R>(&self, range: R) -> Shard<'_, 'k, K, V, R>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        unsafe { Shard::new(self.raw.range(range, prefix)) }
    }

    #[inline]
    pub fn all_mut(&mut self) -> ShardMut<'_, 'static, K, V, RangeFull> {
        unsafe { ShardMut::new(self.all()) }
    }

    #[inline]
    pub fn prefix_mut<'k>(&mut self, prefix: K::Read<'k>) -> ShardMut<'_, 'k, K, V, RangeFull> {
        unsafe { ShardMut::new(self.prefix(prefix)) }
    }

    #[inline]
    pub fn range_mut<'k, R>(&mut self, range: R) -> ShardMut<'_, 'k, K, V, R>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        unsafe { ShardMut::new(self.range(range)) }
    }
}

impl<K, V> Map<K, V>
where
    K: Key,
    V: Value,
{
    #[inline]
    pub(super) fn get_impl(&self, reader: K::Read<'_>) -> Option<&V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(reader) };
        cursor.traverse_get()?;
        Some(unsafe { cursor.as_value_unchecked().cast::<V>().as_ref() })
    }

    #[inline]
    pub(super) fn entry_impl<'k>(&mut self, reader: K::Read<'k>) -> Entry<'_, 'k, K, V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(reader) };

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
        self.raw.postorder().for_each_internal(|_, child| {
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
            let (smo, new) = unsafe { old_node.replace(old.meta()) };
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
