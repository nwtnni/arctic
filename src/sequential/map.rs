//! Auxiliary types for use with [`crate::sequential::Map`].

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

/// Non-concurrent map that supports lexicographically ordered range and prefix scans.
///
/// # Usage
///
/// [`Map`] supports both point and scan operations, and tries to be roughly
/// compatible with the standard library's [`BTreeMap`][std::collections::BTreeMap].
///
/// In general, radix trees do not explicitly store keys; they are implicitly
/// encoded in the structure of the tree. This means that operations on [`Map`]
/// generally take references to keys (see [`crate::Key`]). Operations that insert and typically
/// would take an owned key, like [`BTreeMap::insert`][std::collections::BTreeMap::insert],
/// instead take a [`Key::Insert<'_>`][crate::Key::Insert]. Operations that
/// do not insert a new key take a [`&Key::Borrowed`][crate::Key::Borrowed].
///
/// ## Point operations
///
/// The main caveat here is that [`Map::insert`] errors if the key is present,
/// whereas [`BTreeMap::insert`][std::collections::BTreeMap::insert]
/// updates. (To match the standard library behavior, use [`Map::upsert`] instead.)
/// For more complex conditional logic, the [`Map::entry`] API mimics
/// [`BTreeMap::entry`][std::collections::BTreeMap::entry].
///
/// ## Scan operations
///
/// For scan operations, [`Map`] exposes a two-phase API: the caller first selects
/// a subtree (e.g., [`Map::prefix`] or [`Map::range_mut`]). This returns
/// a [`Shard`] or [`ShardMut`], which can then be iterated over
/// (e.g., [`Shard::entries`] or [`ShardMut::values_mut`]).
/// This is in contrast to the standard library, where [`BTreeMap::range`][std::collections::BTreeMap::range]
/// directly returns an iterator.
///
/// If the key type (see [`crate::Key`]) is dynamically allocated, like [`crate::NonNullString`],
/// iterating over keys can be expensive, as a key buffer must be updated
/// during traversal, and then cloned once per key. This can be mitigated by
/// (a) iterating over values instead of entries, (b) using the lending API
/// (e.g., [`EntryIter::lend`]), which borrows from the iterator's internal
/// buffer, or (c) using the internal iteration API[^iter] (e.g., [`EntryIterMut::for_each_internal`]),
/// which also borrows from the iterator and can be much faster.
///
/// [^iter]: Should ideally replace with custom [`Iterator::try_fold`] implementation,
/// but this currently uses the unstable Try trait.
/// See also [this issue](https://github.com/nnethercote/perf-book/issues/70).
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

/// # Basic operations
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
}

/// # Point operations
///
/// This set of operations operates on a single key-value pair.
impl<K, V> Map<K, V>
where
    K: Key,
    V: Value,
{
    /// Returns an immutable reference to the value associated with `key`.
    ///
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
    ///
    /// For an immutable reference, see [`Map::get`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential::Map;
    ///
    /// let mut map = Map::<u64, u64>::new();
    /// let key = 1;
    /// map.insert(key, 2).expect("Key is not present");
    /// let value = map.get_mut(&key).expect("Key is present");
    /// *value = 3;
    /// assert_eq!(map.get(&key), Some(&3));
    /// ```
    pub fn get_mut(&mut self, key: &K::Borrowed) -> Option<&mut V> {
        let mut cursor = unsafe { self.raw.cursor::<path::Discard>(key) };
        cursor.traverse_get()?;
        Some(unsafe { cursor.as_value_unchecked().cast::<V>().as_mut() })
    }

    /// If there is no value associated with `key`, associate it with `value`.
    ///
    /// <div class="warning">
    ///
    /// This is **not** the same behavior as the standard library
    /// (e.g., [`std::collections::BTreeMap::insert`]); see [`Map::upsert`] if
    /// an existing value should be updated instead.
    ///
    /// </div>
    ///
    /// Returns `Ok(&mut new_value)` if the insert succeeded,
    /// or else `Err((&mut old_value, new_value))` if there is an existing
    /// `old_value` associated with the key.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::key::BoxedStr;
    /// use arctic::key::NonNull;
    /// use arctic::key::Str;
    /// use arctic::sequential::Map;
    ///
    /// let mut map = Map::<BoxedStr<NonNull>, Box<u64>>::new();
    /// let key = Str::<NonNull>::new("regent").expect("No null byte");
    ///
    /// // Key not present, insert succeeds
    /// match map.insert(key, Box::new(3)) {
    ///     Ok(new) => assert_eq!(**new, 3),
    ///     Err(_) => unreachable!(),
    /// }
    ///
    /// // Key not present, insert fails
    /// match map.insert(key, Box::new(26)) {
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
    /// use arctic::key::BoxedStr;
    /// use arctic::key::Terminated;
    /// use arctic::key::Str;
    /// use arctic::sequential::Map;
    ///
    /// let mut map = Map::<BoxedStr<Terminated<b'\n'>>, u64>::new();
    /// let key = Str::new("silent\n").expect("Newline terminated");
    ///
    /// // Key not present, upsert performs insert
    /// match map.upsert(key, 2) {
    ///     Ok(_) => unreachable!(),
    ///     Err(new) => assert_eq!(*new, 2),
    /// }
    ///
    /// // Key present, upsert performs update
    /// match map.upsert(key, 26) {
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
    /// let key = [0, 1, 2];
    ///
    /// // Key not present, update fails
    /// match map.update(&key, Box::new(5)) {
    ///     Ok(_) => unreachable!(),
    ///     Err(new) => assert_eq!(*new, 5),
    /// }
    ///
    /// map.insert(&key, Box::new(9));
    ///
    /// // Key present, update succeeds
    /// match map.update(&key, Box::new(10)) {
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

    /// Get a logical entry associated with `key` (see also [`std::collections::BTreeMap::entry`]).
    ///
    /// This is a lazy operation, and does not allocate or modify the tree structure.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::key::Str;
    /// use arctic::key::NonNull;
    /// use arctic::sequential::Map;
    ///
    /// let mut counter = Map::<&'static Str<NonNull>, u64>::new();
    /// let claw = Str::new("claw").expect("No null byte");
    /// let hotfix = Str::new("hotfix").expect("No null byte");
    /// let hologram = Str::new("hologram").expect("No null byte");
    ///
    /// for key in [claw, claw, hotfix, hologram, claw] {
    ///     *counter.entry(key).or_default() += 1;
    /// }
    ///
    /// assert_eq!(*counter.get(hologram).unwrap(), 1);
    /// assert_eq!(*counter.get(hotfix).unwrap(), 1);
    /// assert_eq!(*counter.get(claw).unwrap(), 3);
    /// ```
    pub fn entry<'k>(&mut self, key: K::Insert<'k>) -> Entry<'_, 'k, K, V> {
        self.entry_impl(K::insert_as_read(key))
    }
}

/// # Scan operations
///
/// This set of operations allows the caller to select a subtree
/// (by prefix or range) for iteration.
impl<K, V> Map<K, V>
where
    K: Key,
    V: Value,
{
    /// Get an immutable reference to the entire tree.
    #[inline]
    pub fn all(&self) -> Shard<'_, 'static, K, V, RangeFull> {
        unsafe { Shard::new(self.raw.all()) }
    }

    /// Get an immutable reference to the subtree of keys beginning with `prefix`.
    #[inline]
    pub fn prefix<'k>(&self, prefix: K::Read<'k>) -> Shard<'_, 'k, K, V, RangeFull> {
        unsafe { Shard::new(self.raw.prefix(prefix)) }
    }

    /// Get an immutable reference to the subtree of keys within `range`.
    #[inline]
    pub fn range<'k, R>(&self, range: R) -> Shard<'_, 'k, K, V, R>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        unsafe { Shard::new(self.raw.range(range, prefix)) }
    }

    /// Get a mutable reference to the entire tree.
    #[inline]
    pub fn all_mut(&mut self) -> ShardMut<'_, 'static, K, V, RangeFull> {
        unsafe { ShardMut::new(self.all()) }
    }

    /// Get a mutable reference to the subtree of keys beginning with `prefix`.
    #[inline]
    pub fn prefix_mut<'k>(&mut self, prefix: K::Read<'k>) -> ShardMut<'_, 'k, K, V, RangeFull> {
        unsafe { ShardMut::new(self.prefix(prefix)) }
    }

    /// Get a mutable reference to the subtree of keys within `range`.
    #[inline]
    pub fn range_mut<'k, R>(&mut self, range: R) -> ShardMut<'_, 'k, K, V, R>
    where
        R: raw::iter::Range<K::Read<'k>>,
    {
        unsafe { ShardMut::new(self.range(range)) }
    }
}

/// # Private implementations
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
                edge::Child::Value(value) => drop(unsafe { V::from_raw_unchecked(value) }),
                edge::Child::Node(node) => unsafe {
                    stat::increment(stat::Counter::FreeDrop);
                    node.deallocate();
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
            let old = unsafe { *self.cursor.edge_mut().get_mut_packed() };
            let old_node = old.as_node().expect("Replace implies node");
            let (smo, new) = unsafe { old_node.replace(old.meta()) };
            // No concurrent operations, so must be node replacement with larger node
            validate_eq!(smo, crate::raw::Smo::ReplaceNode);
            *unsafe { self.cursor.edge_mut() }.get_mut_packed() = new;
            stat::increment(stat::Counter::FreeRetire);
            unsafe { old_node.deallocate() };
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
                    *self.cursor.edge_mut().get_mut_packed() = head;

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
