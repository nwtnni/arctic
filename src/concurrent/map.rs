//! Auxiliary types for use with [`ConcurrentMap`][crate::concurrent::Map].

use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

#[cfg_attr(not(doc), expect(unused))]
use crate::ConcurrentMap;
use crate::Key;
#[cfg_attr(not(doc), expect(unused))]
use crate::SequentialMap;
use crate::concurrent::Shard;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::iter;
use crate::concurrent::smr;
use crate::concurrent::smr::Guard as _;
use crate::concurrent::value;
use crate::raw::Edge;
use crate::raw::cursor;
use crate::raw::cursor::Path;
use crate::raw::cursor::path;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key::Len as _;
use crate::sequential;
use crate::stat;

/// See [`smr::Guard`].
pub type Guard<'g, K, V, S> = <S as Smr<K, V>>::Guard<'g>;

/// See [`value::Owned`].
pub type Owned<'g, K, V, S> = value::Owned<Guard<'g, K, V, S>, V>;

/// See [`value::Shared`].
pub type Shared<'g, K, V, S> = value::Shared<Guard<'g, K, V, S>, V>;

/// See [`value::Updated`].
pub type Updated<'g, K, V, S> = value::Updated<Guard<'g, K, V, S>, V>;

/// See [`value::Upserted`].
pub type Upserted<'g, K, V, S> = value::Upserted<Guard<'g, K, V, S>, V>;

/// Lock-free concurrent map that supports lexicographically ordered, non-linearizable range and prefix scans.
///
/// # Usage
///
/// Refer to [`SequentialMap`] for an introduction.
/// The [`ConcurrentMap`] API differs in three ways: concurrent operations,
/// safe memory reclamation, and advanced point operations.
///
/// ## Concurrent operations
///
/// Unlike [`SequentialMap`], an instance of [`ConcurrentMap`] can be shared
/// and modified concurrently across threads. Methods that usually require a mutable reference
/// (e.g., [`SequentialMap::upsert`]) instead use atomics to synchronize internally,
/// allowing them to take an immutable reference (e.g., [`ConcurrentMap::upsert`]).
///
/// Note that scan operations are not linearizable. They do, however,
/// satisfy weaker guarantees: (a) scans observe keys at most once, in order;
/// and (b) scans observe all keys within bounds that were inserted before
/// the scan starts, and were not removed before the scan ends.
///
/// ## Safe memory reclamation
///
/// In order to provide wait-free reads, [`ConcurrentMap`] requires
/// a safe memory reclamation (SMR) mechanism to detect when
/// allocations are safe to free. This results in the following API changes:
///
/// 1. Values are always returned behind guards. For example,
///    while a successful [`sequential::Map::update`] returns ownership of
///    the old value, a successful [`ConcurrentMap::update`] instead returns an [`Updated`]
///    guard that allows references to the old and new value.
///
///    The guard may have other restrictions depending on the SMR implementation:
///    for example, epoch-based SMR cannot free any memory while a guard is alive,
///    and hazard keys currently only support holding a single guard at a time.
///
/// 2. Values behind guards are always read-only. This can be worked around by
///    either using a value type with internal synchronization (e.g., `Box<Mutex<T>>`),
///    or by obtaining a mutable reference to [`ConcurrentMap`] and then using the
///    sequential API via [`ConcurrentMap::as_sequential`].
///
/// 3. Values distinguish between inline (e.g., integers) and indirect (e.g., `Box<T>`).
///    In short, we return [`Value::Borrowed`] instead of `&V`, because the memory location
///    where `V` itself is stored may be concurrently updated.
///    (See [`Value`] for more information.)
///
/// ## Advanced point operations
///
/// Point operations can internally fail and retry under contention.
/// We give the caller control over retries by providing variants of point
/// operations (ending in suffix `_with`, e.g.,
/// [`ConcurrentMap::update_with`]) that
/// take a closure.
///
/// This can be used to efficiently implement lazy value initialization,
/// or synchronization logic where the next value is computed from the
/// current value, and then atomically inserted or updated.
pub struct Map<K: Key, V: Value, S = smr::Default> {
    smr: S,
    seq: sequential::Map<K, V>,
}

impl<K: Key, V: Value, S: Default> Default for Map<K, V, S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Key, V: Value, S: Default> Map<K, V, S> {
    /// Construct an empty map with the default safe memory reclamation state.
    pub fn new() -> Self {
        Self::with_smr(S::default())
    }
}

impl<K: Key, V: Value, S> Map<K, V, S> {
    /// Construct an empty map with the given safe memory reclamation state.
    pub const fn with_smr(smr: S) -> Self {
        Self {
            smr,
            seq: sequential::Map::<K, V>::new(),
        }
    }
}

/// # Basic operations
impl<K: Key, V: Value, S: Smr<K, V>> Map<K, V, S> {
    /// Get a mutable view as a [`SequentialMap`] for temporary access to a more
    /// efficient and flexible single-threaded API. For permanent access, use
    /// [`From`].
    ///
    /// This method is safe because `&mut` guarantees this thread holds the
    /// only reference to the underlying map.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use core::ops::ControlFlow;
    /// use std::thread;
    ///
    /// use arctic::ConcurrentMap;
    /// use arctic::sequential;
    /// use arctic::concurrent::smr;
    ///
    /// let mut map = ConcurrentMap::<u32, u64>::default();
    ///
    /// // Concurrently insert into map
    /// thread::scope(|scope| {
    ///     let map = &map;
    ///     for id in 0..8 {
    ///         scope.spawn(move || {
    ///             map.insert(id, id as u64).expect("Key is not present");
    ///         });
    ///     }
    /// });
    ///
    /// // Access sequential entry API
    /// map.as_sequential()
    ///     .entry(8)
    ///     .or_insert(8);
    ///
    /// // Access sequential mutable iteration API
    /// map.as_sequential()
    ///     .range_mut(5..=12)
    ///     .entries_mut::<arctic::Ascend>()
    ///     .for_each_internal(|(key, value)| {
    ///         assert!(key >= 5);
    ///         assert!(key <= 8, "Inserted up to 8");
    ///         assert_eq!(key, *value as u32);
    ///         *value += 1;
    ///         ControlFlow::Continue(())
    ///     });
    ///
    /// // Sanity check that mutations are visible from concurrent map
    /// let mut len = 0;
    /// map.all()
    ///     .entries::<arctic::Descend>()
    ///     .for_each_internal(|(key, value)|{
    ///         let expected = if key >= 5 { key + 1 } else { key };
    ///         assert_eq!(*value as u32, expected);
    ///         len += 1;
    ///         ControlFlow::Continue(())
    ///     });
    /// assert_eq!(len, 9);
    /// ```
    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        &mut self.seq
    }

    /// Get an immutable reference to the underlying safe memory reclamation state.
    #[inline]
    pub fn smr(&self) -> &S {
        &self.smr
    }

    /// Get a mutable reference to the underlying safe memory reclamation state.
    #[inline]
    pub fn smr_mut(&mut self) -> &mut S {
        &mut self.smr
    }
}

/// # Point operations
///
/// This set of operations operates on a single key-value pair.
///
/// These operations are linearizable.
impl<K: Key, V: Value, S: Smr<K, V>> Map<K, V, S> {
    /// Returns an immutable reference to the value associated with `key`.
    ///
    /// For a mutable reference, see [`ConcurrentMap::as_sequential`] and
    /// [`SequentialMap::get_mut`].
    /// There is no way to safely get a mutable reference to a value from an immutable [`Map`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::ConcurrentMap;
    ///
    /// let map = ConcurrentMap::<u64, u64>::default();
    /// let key = 64;
    ///
    /// assert!(map.get(&key).is_none());
    ///
    /// match map.insert(key, 3) {
    ///     Err(_) => unreachable!(),
    ///     Ok(new) => assert_eq!(*new, 3),
    /// }
    ///
    /// match map.get(&key) {
    ///     None => unreachable!(),
    ///     Some(value) => assert_eq!(*value, 3),
    /// }
    /// ```
    pub fn get<'g>(&'g self, key: &K::Borrowed) -> Option<Shared<'g, K, V, S>> {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(reader);
        let value = unsafe { self.get_raw(&mut guard, reader)? };
        Some(unsafe { Shared::<'_, K, V, S>::wrap(guard, value) })
    }

    /// If there is no value associated with `key`, associate it with `value`.
    ///
    /// <div class="warning">
    ///
    /// This is **not** the same behavior as the standard library
    /// (e.g., [`std::collections::BTreeMap::insert`]); see [`Map::upsert`] if
    /// an existing value should be updated instead.)
    ///
    /// </div>
    ///
    /// Returns `Ok(&new_value)` if the insert succeeded,
    /// or else `Err((&old_value, new_value))` if there is an existing
    /// `old_value` associated with the key.
    ///
    /// See [`ConcurrentMap::insert_with`] for dynamic control flow and value construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::key::Str;
    /// use arctic::key::NonNull;
    /// use arctic::ConcurrentMap;
    ///
    /// let map = ConcurrentMap::<&'static Str<NonNull>, u64>::default();
    /// let key = Str::new("korlex").expect("No null byte");
    ///
    /// // Key is not present, insert succeeds
    /// match map.insert(key, 3) {
    ///     Err(_) => unreachable!(),
    ///     Ok(new) => assert_eq!(*new, 3),
    /// }
    ///
    /// // Key is present, insert fails
    /// match map.insert(key, 5) {
    ///     Err((old, new)) => {
    ///         assert_eq!(*old, 3);
    ///         assert_eq!(new, 5);
    ///     }
    ///     Ok(_) => unreachable!(),
    /// }
    /// ```
    #[expect(clippy::type_complexity)]
    pub fn insert<'g, 'k>(
        &'g self,
        key: K::Insert<'k>,
        value: V,
    ) -> Result<Shared<'g, K, V, S>, (Shared<'g, K, V, S>, V)> {
        let mut value = Some(value);
        self.insert_with(key, || value.take().expect("Call thunk once"))
            .map_err(|(shared, initial)| {
                (
                    shared,
                    value
                        .xor(initial)
                        .expect("Value must be in thunk or initial"),
                )
            })
    }

    /// Unconditionally associate `key` with `value`.
    ///
    /// Returns an [`Upserted`] guard that provides immutable references
    /// to the (optional) old value and the newly updated (or inserted) value.
    ///
    /// See [`ConcurrentMap::upsert_with`] for dynamic control flow and value construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::key::BoxedStr;
    /// use arctic::key::Terminated;
    /// use arctic::key::Str;
    /// use arctic::ConcurrentMap;
    ///
    /// let map = ConcurrentMap::<BoxedStr<Terminated<b'\n'>>, u64>::default();
    /// let key = Str::new("arqad\n").expect("Newline terminated");
    ///
    /// // Key is not present, upsert performs insert
    /// let upserted = map.upsert(key, 3);
    /// assert_eq!(upserted.old(), None);
    /// assert_eq!(*upserted.new(), 3);
    ///
    /// // Key is present, upsert performs update
    /// let upserted = map.upsert(key, 5);
    /// assert_eq!(upserted.old().copied(), Some(3));
    /// assert_eq!(*upserted.new(), 5);
    /// ```
    pub fn upsert<'k>(&self, key: K::Insert<'k>, value: V) -> Upserted<'_, K, V, S> {
        match self.upsert_with(key, Some(value), |_, new| {
            ControlFlow::<(), _>::Continue(new.take().expect("Value is always initialized"))
        }) {
            Upsert::Success(upserted) => upserted,
            Upsert::Break { .. } => unreachable!(),
        }
    }

    /// If there is a value associated with `key`, update it to `value`.
    ///
    /// Returns `Ok((&old_value, &new_value))` if the update succeeded,
    /// or else `Err(new_value)` if there was no old value associated with `key`.
    ///
    /// See [`ConcurrentMap::update_with`] for dynamic control flow and value construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::ConcurrentMap;
    ///
    /// let map = ConcurrentMap::<u32, Box<u64>>::default();
    ///
    /// match map.update(&37, Box::new(5)) {
    ///     Err(new) => assert_eq!(*new, 5),
    ///     Ok(_) => unreachable!(),
    /// }
    ///
    /// match map.insert(37, Box::new(3)) {
    ///     Err(_) => unreachable!(),
    ///     Ok(new) => assert_eq!(*new, 3),
    /// }
    ///
    /// match map.update(&37, Box::new(5)) {
    ///     Err(_) => unreachable!(),
    ///     Ok(updated) => {
    ///         assert_eq!(*updated.old(), 3);
    ///         assert_eq!(*updated.new(), 5);
    ///     },
    /// }
    /// ```
    pub fn update<'g>(&'g self, key: &K::Borrowed, value: V) -> Result<Updated<'g, K, V, S>, V> {
        match self.update_with(key, Some(value), |_, initial| {
            ControlFlow::<(), _>::Continue(initial.take().expect("Value is always initialized"))
        }) {
            Update::Absent { new: Some(initial) } => Err(initial),
            Update::Success(updated) => Ok(updated),
            Update::Absent { new: None } | Update::Break { .. } => unreachable!(),
        }
    }

    /// If there is a value associated with `key`, remove it from the map,
    /// recursively removing empty tree nodes.
    ///
    /// This method is slow because it must keep a traversal stack, and scan and
    /// delete empty nodes. See [`ConcurrentMap::remove_non_recursive`]
    /// for a faster, but potentially memory-intensive alternative.
    ///
    /// Returns `Some(&old_value)` if the remove succeeded, or else `None` if
    /// there was no old value associated with `key`.
    ///
    /// See [`ConcurrentMap::remove_with`] for dynamic control flow.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::ConcurrentMap;
    ///
    /// let map = ConcurrentMap::<u128, u64>::default();
    /// let key = 0xabc;
    ///
    /// assert!(map.remove(&key).is_none());
    /// map.insert(key, 5).expect("Key is not present");
    /// match map.remove(&key) {
    ///     None => unreachable!(),
    ///     Some(removed) => assert_eq!(*removed, 5),
    /// }
    /// ```
    pub fn remove<'g>(&'g self, key: &K::Borrowed) -> Option<Owned<'g, K, V, S>> {
        match self.remove_with(key, |_| ControlFlow::Continue(())) {
            Remove::Absent => None,
            Remove::Success { old } => Some(old),
            Remove::Break { old: _ } => unreachable!(),
        }
    }

    /// If there is a value associated with `key`, remove it from the map,
    /// **without** recursively removing empty tree nodes.
    ///
    /// <div class="warning">
    ///
    /// This method is much faster than [`ConcurrentMap::remove`],
    /// because no traversal
    /// stack or node scanning and replacement is necessary; however, it means
    /// the memory usage of the tree is no longer correlated with the number of
    /// keys and values it contains.
    ///
    /// This method should only be used if removals are rare or removed keys
    /// are expected to be reinserted.
    //
    /// </div>
    ///
    /// Returns `Some(&old_value)` if the remove succeeded, or else `None` if
    /// there was no old value associated with `key`.
    ///
    /// See [`ConcurrentMap::remove_non_recursive_with`] for dynamic control flow.
    pub fn remove_non_recursive(&self, key: &K::Borrowed) -> Option<Owned<'_, K, V, S>> {
        match self.remove_non_recursive_with(key, |_| ControlFlow::Continue(())) {
            Remove::Absent => None,
            Remove::Success { old } => Some(old),
            Remove::Break { old: _ } => unreachable!(),
        }
    }
}

/// # Scan operations
///
/// This set of operations allows the caller to select a subtree
/// (by prefix or range) for non-linearizable iteration.
impl<K, V, S> Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Smr<K, V>,
{
    /// Get an immutable reference to the entire tree.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::ConcurrentMap;
    ///
    /// let map = ConcurrentMap::<u64, u64>::default();
    /// map.insert(1, 2).expect("Key not present");
    /// map.insert(3, 4).expect("Key not present");
    ///
    /// assert_eq!(map.all().entries::<arctic::Ascend>().count(), 2);
    /// ```
    pub fn all(&self) -> iter::Shard<'_, 'static, K, V, RangeFull, Guard<'_, K, V, S>> {
        let guard = self.smr.guard(K::Read::default());
        unsafe { Shard::new(guard, self.seq.raw.all()) }
    }

    /// Get an immutable reference to the subtree of keys beginning with `prefix`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    /// use arctic::ConcurrentMap;
    /// use arctic::key::BoxedStr;
    /// use arctic::key::NonNull;
    /// use arctic::key::Str;
    ///
    /// let map = ConcurrentMap::<BoxedStr<NonNull>, Box<u64>>::default();
    ///
    /// for (key, value) in [("prefix-one", 3), ("prefix-two", 2), ("three", 1)] {
    ///     map.insert(
    ///         Str::new(key).expect("No null byte"),
    ///         Box::new(value),
    ///     ).expect("Key not present");
    /// }
    ///
    /// // Get all key value pairs where key starts with prefix
    /// //
    /// // Need a temporary binding here since lifetimes of references
    /// // returned from iterators is tied to this shard
    /// //
    /// // Note: prefix does not need to satisfy any particular invariants;
    /// // can be invalid UTF-8 or contain null or terminator bytes
    /// let prefix = map.prefix("prefix");
    ///
    /// let entries: concurrent::EntryIter<_, _, _, _, _> = prefix.entries::<arctic::Ascend>();
    ///
    /// // WARNING: using `entries` as `Iterator` requires cloning keys,
    /// // which is expensive here due to BoxedStr keys
    /// assert_eq!(entries.count(), 2);
    ///
    /// // Can use lending iterator API to avoid cloning
    /// let mut entries: concurrent::EntryIter<_, _, _, _, _> = prefix.entries::<arctic::Ascend>();
    /// while let Some((key, _)) = entries.lend() {
    ///     assert!(key.as_str().starts_with("prefix"));
    /// }
    /// ```
    pub fn prefix<'g, 'k>(
        &'g self,
        prefix: impl Into<K::Read<'k>>,
    ) -> iter::Shard<'g, 'k, K, V, RangeFull, Guard<'g, K, V, S>> {
        let prefix = prefix.into();
        let guard = self.smr.guard(prefix);
        unsafe { Shard::new(guard, self.seq.raw.prefix(prefix)) }
    }

    /// Get an immutable reference to the subtree of keys within `range`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::ConcurrentMap;
    ///
    /// let map = ConcurrentMap::<u64, u64>::default();
    /// map.insert(1, 2).expect("Key not present");
    /// map.insert(3, 4).expect("Key not present");
    /// map.insert(5, 6).expect("Key not present");
    ///
    /// let range = map.range(3..=7);
    ///
    /// for (key, value) in range.entries::<arctic::Descend>() {
    ///     assert!((3..=7).contains(&key));
    /// }
    /// ```
    pub fn range<'g, 'k, R>(&'g self, range: R) -> iter::Shard<'g, 'k, K, V, R, Guard<'g, K, V, S>>
    where
        R: crate::raw::iter::Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        let guard = self.smr.guard(prefix);
        unsafe { Shard::new(guard, self.seq.raw.range(range, prefix)) }
    }
}

/// # Advanced point operations
///
/// This set of operations extends the point operations to take a closure,
/// allowing the caller to dynamically break out of an operation or lazily
/// allocate a value. Importantly, this closure can observe the value
/// currently associated with a key before deciding what to do, which enables
/// more complex coordination in a concurrent setting.
///
/// For example, a concurrent counter could use
/// [`ConcurrentMap::upsert_with`] to either
/// insert one or update the current count by one, or an index could use
/// [`ConcurrentMap::remove_with`] to
/// remove a value only if it hasn't been concurrently updated.
///
/// These operations are linearizable.
impl<K, V, S> Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Smr<K, V>,
{
    /// If there is no value associated with `key`, call the provided `insert` closure
    /// to compute a new value.
    ///
    /// The closure is called at most once, even under contention; the value will be
    /// reused once allocated.
    ///
    /// Returns `Ok(&new_value)` if the insert succeeded,
    /// or else `Err((&old_value, new_value))` if there is an existing
    /// `old_value` associated with the key. `new_value` is `None`
    /// if the closure was never called, or `Some` if this insert
    /// was pre-empted by a concurrent insert to the same key.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use core::ops::ControlFlow;
    ///
    /// use arctic::ConcurrentMap;
    /// use arctic::key::BoxedStr;
    /// use arctic::key::NonNull;
    /// use arctic::key::Str;
    ///
    /// let map = ConcurrentMap::<BoxedStr<NonNull>, Box<u64>>::default();
    /// let key = Str::new("zipir").expect("No null byte");
    ///
    /// // Key not present, new value lazily allocated
    /// match map.insert_with(key, || Box::new(10)) {
    ///     Ok(new) => {
    ///         assert_eq!(*new, 10);
    ///     }
    ///     Err(_) => unreachable!(),
    /// }
    ///
    /// // Key present, new value not allocated
    /// match map.insert_with(key, || Box::new(15)) {
    ///     Ok(_) => unreachable!(),
    ///     Err((old, new)) => {
    ///         assert_eq!(*old, 10);
    ///         assert!(new.is_none());
    ///     },
    /// }
    /// ```
    #[expect(clippy::type_complexity)]
    pub fn insert_with<'g, 'k, F>(
        &'g self,
        key: K::Insert<'k>,
        insert: F,
    ) -> Result<Shared<'g, K, V, S>, (Shared<'g, K, V, S>, Option<V>)>
    where
        F: FnOnce() -> V,
    {
        let mut thunk = Some(insert);

        match self.upsert_with(key, None, |old, new| match old {
            None => ControlFlow::Continue(match new.take() {
                None => (thunk.take().expect("Call thunk once"))(),
                Some(new) => new,
            }),
            Some(_) => ControlFlow::Break(()),
        }) {
            Upsert::Success(upserted) => Ok(upserted
                .try_into_inserted()
                .unwrap_or_else(|_| unreachable!("Continue on `None`"))),
            Upsert::Break { old, new } => Err((old.expect("Break on `Some`"), new)),
        }
    }

    /// Associate `key` with `value`, calling the provided `upsert` closure to
    /// break or compute a new value.
    ///
    /// The closure may be called multiple times under contention,
    /// and takes an immutable reference to the current value (if there is one), as well as `initial`
    /// (on the first call) or `Some(prev_value)` (on subsequent calls); use [`Option::take`]
    /// to move out of the option.
    ///
    /// Returns an [`Upsert`] enum.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use core::ops::ControlFlow;
    ///
    /// use arctic::ConcurrentMap;
    /// use arctic::concurrent::map::Upsert;
    ///
    /// let map = ConcurrentMap::<u16, Box<u64>>::default();
    /// let key = 20;
    ///
    /// // Key not present, closure continues, new value lazily allocated
    /// match map.upsert_with(key, None, |old, new| {
    ///     assert!(old.is_none());
    ///     assert!(new.is_none());
    ///     ControlFlow::Continue(Box::new(9))
    /// }) {
    ///     Upsert::Success(upserted) => {
    ///         assert!(upserted.old().is_none());
    ///         assert_eq!(*upserted.new(), 9);
    ///     },
    ///     Upsert::Break { .. } => unreachable!(),
    /// }
    ///
    /// // Key present, closure breaks, new value not allocated
    /// match map.upsert_with(key, None, |old, new| {
    ///     assert!(old.copied() == Some(9));
    ///     assert!(new.is_none());
    ///     ControlFlow::Break(())
    /// }) {
    ///     Upsert::Success(_) => unreachable!(),
    ///     Upsert::Break { old, new } => {
    ///         assert_eq!(old.as_deref().copied(), Some(9));
    ///         assert!(new.is_none());
    ///     },
    /// }
    ///
    /// // Key present, closure continues, new value lazily allocated (and reused under contention)
    /// match map.upsert_with(key, None, |old, new| {
    ///     let next = old.copied().unwrap_or(0) + 1;
    ///
    ///     ControlFlow::Continue(
    ///         new.take()
    ///             // Reuse allocation under contention
    ///             .map(|mut new: Box<u64>| {
    ///                 *new = next;
    ///                 new
    ///             })
    ///             // Allocate new value
    ///             .unwrap_or_else(|| Box::new(next)))
    /// }) {
    ///     Upsert::Success(updated) => {
    ///         assert_eq!(updated.old().copied(), Some(9));
    ///         assert_eq!(*updated.new(), 10);
    ///     }
    ///     _ => unreachable!(),
    /// }
    /// ```
    pub fn upsert_with<'g, 'k, F>(
        &'g self,
        key: K::Insert<'k>,
        mut initial: Option<V>,
        mut upsert: F,
    ) -> Upsert<'g, K, V, S>
    where
        F: FnMut(Option<&V::Borrowed>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::insert_as_read(key);
        let mut guard = self.smr.guard(reader);

        // NOTE: this is a macro so we get disjoint mutable borrows of `initial`
        macro_rules! upsert {
            () => {
                |old: Option<u64>, new: Option<u64>| {
                    initial = new.map(|new| V::from_raw_unchecked(new));

                    match upsert(
                        old.as_ref().map(|old| V::borrow_from_raw_unchecked(old)),
                        &mut initial,
                    ) {
                        ControlFlow::Continue(new) => ControlFlow::Continue(new.into_raw()),
                        ControlFlow::Break(()) => ControlFlow::Break(()),
                    }
                }
            };
        }

        let upsert = match if cfg!(feature = "opt-no-path") {
            unsafe {
                self.upsert_with_optimistic(
                    &mut guard,
                    reader,
                    initial.take().map(V::into_raw),
                    upsert!(),
                )
            }
        } else {
            Err(())
        } {
            Ok(upsert) => upsert,
            Err(()) => unsafe {
                self.upsert_with_pessimistic(
                    &mut guard,
                    reader,
                    initial.take().map(V::into_raw),
                    upsert!(),
                )
            },
        };

        match upsert {
            UpsertRaw::Success { old, new } => {
                Upsert::Success(unsafe { Upserted::<K, V, S>::wrap(guard, old, new) })
            }
            UpsertRaw::Break { old } => Upsert::Break {
                old: old.map(|old| unsafe { Shared::<K, V, S>::wrap(guard, old) }),
                new: initial,
            },
        }
    }

    /// If there is a value associated with `key`, call the provided `update` closure
    /// to break or compute a new value.
    ///
    /// The closure may be called multiple times under contention,
    /// and takes an immutable reference to the current value, as well as `initial`
    /// (on the first call) or `Some(prev_value)` (on subsequent calls); use [`Option::take`]
    /// to move out of the option.
    ///
    /// Returns an [`Update`] enum.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use core::ops::ControlFlow;
    ///
    /// use arctic::ConcurrentMap;
    /// use arctic::concurrent::map::Update;
    ///
    /// let map = ConcurrentMap::<u64, Box<u64>>::default();
    /// let key = 5;
    ///
    /// // Key not present, closure never called, new value not allocated
    /// match map.update_with(&key, None, |_, _| unreachable!()) {
    ///     Update::Absent { new } => assert!(new.is_none()),
    ///     Update::Success { .. } | Update::Break { .. } => unreachable!(),
    /// }
    ///
    /// map.insert(key, Box::new(29)).expect("Key not present");
    ///
    /// // Key present, closure breaks, new value not allocated
    /// match map.update_with(&key, None, |_, _| ControlFlow::Break(())) {
    ///     Update::Break { old, new } => {
    ///         assert_eq!(*old, 29);
    ///         assert!(new.is_none());
    ///     }
    ///     Update::Absent { .. } | Update::Success { .. } => unreachable!(),
    /// }
    ///
    /// // Key present, closure continues, new value lazily allocated (and reused under contention)
    /// match map.update_with(&key, None, |old, new| {
    ///     ControlFlow::Continue(
    ///         new.take()
    ///             // Reuse allocation under contention
    ///             .map(|mut new: Box<u64>| {
    ///                 *new = *old + 1;
    ///                 new
    ///             })
    ///             // Allocate new value
    ///             .unwrap_or_else(|| Box::new(*old + 1)))
    /// }) {
    ///     Update::Success(updated) => {
    ///         assert_eq!(*updated.old(), 29);
    ///         assert_eq!(*updated.new(), 30);
    ///     }
    ///     Update::Absent { .. } | Update::Break { .. } => unreachable!(),
    /// }
    /// ```
    pub fn update_with<'g, F>(
        &'g self,
        key: &K::Borrowed,
        mut initial: Option<V>,
        mut update: F,
    ) -> Update<'g, K, V, S>
    where
        F: FnMut(&V::Borrowed, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(reader);

        // NOTE: this is a macro so we get disjoint mutable borrows of `initial`
        macro_rules! update {
            () => {
                |old: u64, new: Option<u64>| {
                    initial = new.map(|new| V::from_raw_unchecked(new));

                    match update(V::borrow_from_raw_unchecked(&old), &mut initial) {
                        ControlFlow::Continue(new) => ControlFlow::Continue(new.into_raw()),
                        ControlFlow::Break(()) => ControlFlow::Break(()),
                    }
                }
            };
        }

        let update = match if cfg!(feature = "opt-no-path") {
            unsafe {
                self.update_with_optimistic(
                    &mut guard,
                    reader,
                    initial.take().map(V::into_raw),
                    update!(),
                )
            }
        } else {
            Err(())
        } {
            Ok(update) => update,
            Err(()) => unsafe {
                self.update_with_pessimistic(
                    &mut guard,
                    reader,
                    initial.take().map(V::into_raw),
                    update!(),
                )
            },
        };

        match update {
            UpdateRaw::Absent { new } => Update::Absent {
                new: new.map(|new| unsafe { V::from_raw_unchecked(new) }),
            },
            UpdateRaw::Success { old, new } => {
                Update::Success(unsafe { Updated::<K, V, S>::wrap(guard, old, new) })
            }
            UpdateRaw::Break { old } => Update::Break {
                old: unsafe { Shared::<K, V, S>::wrap(guard, old) },
                new: initial,
            },
        }
    }

    /// If there is a value associated with `key`, call `remove` to determine whether
    /// to remove the value, recursively removing empty tree nodes.
    ///
    /// Returns a [`Remove`] enum.
    ///
    /// See also: [`ConcurrentMap::remove`],
    /// [`ConcurrentMap::remove_non_recursive`],
    /// [`ConcurrentMap::remove_non_recursive_with`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use core::ops::ControlFlow;
    ///
    /// use arctic::ConcurrentMap;
    /// use arctic::concurrent::map::Remove;
    ///
    /// let map = ConcurrentMap::<u128, u64>::default();
    /// let key = 0xfeed;
    ///
    /// // Key not present, closure never called
    /// match map.remove_with(&key, |_| unreachable!()) {
    ///     Remove::Absent => (),
    ///     Remove::Success { .. } | Remove::Break { .. } => unreachable!(),
    /// }
    ///
    /// map.insert(key, 1).expect("Key not present");
    ///
    /// // Key present, closure breaks, value not removed
    /// match map.remove_with(&key, |old| {
    ///     assert_eq!(*old, 1);
    ///     ControlFlow::Break(())
    /// }) {
    ///     Remove::Break { old } => assert_eq!(*old, 1),
    ///     Remove::Absent | Remove::Success { .. } => unreachable!(),
    /// }
    ///
    /// assert_eq!(map.get(&key).as_deref().copied(), Some(1));
    ///
    /// // Key present, closure continues, value removed
    /// match map.remove_with(&key, |old| {
    ///     if *old > 0 {
    ///         ControlFlow::Continue(())
    ///     } else {
    ///         ControlFlow::Break(())
    ///     }
    /// }) {
    ///     Remove::Success { old } => assert_eq!(*old, 1),
    ///     Remove::Absent | Remove::Break { .. } => unreachable!(),
    /// }
    ///
    /// assert!(map.get(&key).is_none());
    /// ```
    pub fn remove_with<'g, F>(&'g self, key: &K::Borrowed, mut remove: F) -> Remove<'g, K, V, S>
    where
        F: FnMut(&V::Borrowed) -> ControlFlow<(), ()>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(reader);
        let Ok(remove) = unsafe {
            self.remove_with_raw::<true, path::Retain<_>, _>(&mut guard, reader, |value| {
                remove(V::borrow_from_raw_unchecked(&value))
            })
        };

        match remove {
            RemoveRaw::Absent => Remove::Absent,
            RemoveRaw::Success { old } => Remove::Success {
                old: unsafe { Owned::<K, V, S>::wrap(guard, old) },
            },
            RemoveRaw::Break { old } => Remove::Break {
                old: unsafe { Shared::<K, V, S>::wrap(guard, old) },
            },
        }
    }

    /// If there is a value associated with `key`, call `remove` to determine whether
    /// to remove the value, **without** recursively removing empty tree nodes.
    ///
    /// <div class="warning">
    ///
    /// See warning on [`Map::remove_non_recursive`].
    ///
    /// </div>
    ///
    /// Returns a [`Remove`] enum.
    ///
    /// See also: [`ConcurrentMap::remove`],
    /// [`ConcurrentMap::remove_with`],
    /// [`ConcurrentMap::remove_non_recursive`].
    pub fn remove_non_recursive_with<F>(
        &self,
        key: &K::Borrowed,
        mut remove: F,
    ) -> Remove<'_, K, V, S>
    where
        F: FnMut(&V::Borrowed) -> ControlFlow<(), ()>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(reader);
        let mut remove = |value: u64| remove(unsafe { V::borrow_from_raw_unchecked(&value) });

        let remove = match if cfg!(feature = "opt-no-path") {
            unsafe { self.remove_non_recursive_with_optimistic(&mut guard, reader, &mut remove) }
        } else {
            Err(())
        } {
            Ok(remove) => remove,
            Err(()) => unsafe {
                self.remove_non_recursive_with_pessimistic(&mut guard, reader, &mut remove)
            },
        };

        match remove {
            RemoveRaw::Absent => Remove::Absent,
            RemoveRaw::Success { old } => Remove::Success {
                old: unsafe { Owned::<K, V, S>::wrap(guard, old) },
            },
            RemoveRaw::Break { old } => Remove::Break {
                old: unsafe { Shared::<K, V, S>::wrap(guard, old) },
            },
        }
    }
}

/// Outcome of a call to [`ConcurrentMap::upsert_with`].
pub enum Upsert<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr<K, V> + 'g,
{
    /// Value was successfully upserted.
    Success(Upserted<'g, K, V, S>),
    /// Closure returned [`core::ops::ControlFlow::Break`].
    Break {
        /// Latest value observed by closure.
        old: Option<Shared<'g, K, V, S>>,
        /// Latest value passed as argument or returned from closure.
        new: Option<V>,
    },
}

/// Type-erased version of [`Upsert`].
enum UpsertRaw {
    Success { old: Option<u64>, new: u64 },
    Break { old: Option<u64> },
}

/// Outcome of a call to [`ConcurrentMap::update_with`].
pub enum Update<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr<K, V> + 'g,
{
    /// Key was not present.
    Absent {
        /// Latest value passed as argument or returned from closure.
        new: Option<V>,
    },
    /// Value was successfully updated.
    Success(Updated<'g, K, V, S>),
    /// Closure returned [`core::ops::ControlFlow::Break`].
    Break {
        /// Latest value observed by closure.
        old: Shared<'g, K, V, S>,
        /// Latest value passed as argument or returned from closure.
        new: Option<V>,
    },
}

/// Type-erased version of [`Update`].
enum UpdateRaw {
    Absent { new: Option<u64> },
    Success { old: u64, new: u64 },
    Break { old: u64 },
}

/// Outcome of a call to [`ConcurrentMap::remove_with`].
pub enum Remove<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr<K, V> + 'g,
{
    /// Key was not present.
    Absent,
    /// Value was successfully removed.
    Success {
        /// Value that was removed.
        old: Owned<'g, K, V, S>,
    },
    /// Closure returned [`core::ops::ControlFlow::Break`].
    Break {
        /// Latest value observed by closure.
        old: Shared<'g, K, V, S>,
    },
}

/// Type-erased version of [`Remove`].
enum RemoveRaw {
    Absent,
    Success { old: u64 },
    Break { old: u64 },
}

/// # Private implementations
///
/// These methods erase value types and accept arbitrary key readers and SMR guards.
/// This reduces monomorphization and allows a future `concurrent::Set` implementation
/// to reuse this logic, at the cost of reducing type safety.
///
/// # Safety
///
/// Caller must guarantee:
/// - `_guard` protects nodes and values under `reader` for its lifetime.
/// - When inserting or upserting, `reader` preserves the prefix property.
/// - `initial` and every value returned from a closure was created via `V::into_raw`.
impl<K, V, S> Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Smr<K, V>,
{
    #[inline]
    unsafe fn get_raw<'g>(&'g self, _guard: &mut S::Guard<'g>, reader: K::Read<'_>) -> Option<u64> {
        unsafe { self.seq.raw.cursor::<path::Discard>(reader).traverse_get() }
    }

    #[inline]
    unsafe fn upsert_with_optimistic<'g, 'k, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'k>,
        initial: Option<u64>,
        upsert: F,
    ) -> Result<UpsertRaw, ()>
    where
        F: FnMut(Option<u64>, Option<u64>) -> ControlFlow<(), u64>,
    {
        unsafe { self.upsert_with_raw::<path::Discard, _>(guard, reader, initial, upsert) }
    }

    #[cold]
    unsafe fn upsert_with_pessimistic<'g, 'k, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'k>,
        initial: Option<u64>,
        upsert: F,
    ) -> UpsertRaw
    where
        F: FnMut(Option<u64>, Option<u64>) -> ControlFlow<(), u64>,
    {
        stat::increment(stat::Counter::InsertPessimistic);
        let Ok(upsert) =
            unsafe { self.upsert_with_raw::<path::Retain<_>, _>(guard, reader, initial, upsert) };
        upsert
    }

    #[inline]
    unsafe fn upsert_with_raw<'g, 'k, P, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'k>,
        mut initial: Option<u64>,
        mut upsert: F,
    ) -> Result<UpsertRaw, P::PopError>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(Option<u64>, Option<u64>) -> ControlFlow<(), u64>,
    {
        let mut cursor = unsafe { self.seq.raw.cursor::<P>(reader) };

        loop {
            match cursor.traverse_insert() {
                cursor::Insert::Value {
                    value: old_value,
                    edge: old_edge,
                } => {
                    let new_value = match upsert(old_value, initial) {
                        ControlFlow::Continue(new_value) => new_value,
                        ControlFlow::Break(()) => {
                            return Ok(UpsertRaw::Break { old: old_value });
                        }
                    };

                    if old_edge.meta().is_frozen() {
                        // Restore value and fall through to freeze
                        initial = Some(new_value);
                    } else {
                        let (new_edge, _) = cursor.create_path(old_edge, new_value);
                        match cursor.edge().compare_exchange_packed(
                            old_edge,
                            new_edge,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => {
                                return Ok(UpsertRaw::Success {
                                    old: old_value,
                                    new: new_value,
                                });
                            }
                            Err(_) => {
                                if let Some(node) = new_edge.as_node() {
                                    unsafe {
                                        stat::increment(stat::Counter::FreeConflict);
                                        node.deallocate_recursive::<K::Edge>();
                                    }
                                }

                                initial = Some(new_value);
                                continue;
                            }
                        }
                    }
                }
                cursor::Insert::Replace {
                    node: old_node,
                    edge: old_edge,
                } if !old_edge.meta().is_frozen() => {
                    let (smo, new_edge) = unsafe {
                        old_node.freeze::<K::Edge>();
                        old_node.replace(old_edge.meta())
                    };
                    match cursor.edge().compare_exchange_packed(
                        old_edge,
                        new_edge,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            unsafe { guard.retire_node(cursor.len().bits(), old_node.into_raw()) };
                        }
                        Err(_) => {
                            // Does not go through SMR because `new` is still thread-local
                            if smo.is_allocate() {
                                let node = new_edge.as_node().expect("Allocating SMO creates node");
                                unsafe {
                                    stat::increment(stat::Counter::FreeConflict);
                                    node.deallocate();
                                }
                            }
                        }
                    }

                    continue;
                }

                // Fall through to freeze
                cursor::Insert::Replace { .. } => (),
            }

            match cursor.freeze()? {
                None => (),
                Some(node) => unsafe { guard.retire_node(cursor.len().bits(), node.into_raw()) },
            }
        }
    }

    #[inline]
    unsafe fn update_with_optimistic<'g, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'_>,
        initial: Option<u64>,
        update: F,
    ) -> Result<UpdateRaw, ()>
    where
        F: FnMut(u64, Option<u64>) -> ControlFlow<(), u64>,
    {
        unsafe { self.update_with_raw::<path::Discard, _>(guard, reader, initial, update) }
    }

    #[cold]
    unsafe fn update_with_pessimistic<'g, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'_>,
        initial: Option<u64>,
        update: F,
    ) -> UpdateRaw
    where
        F: FnMut(u64, Option<u64>) -> ControlFlow<(), u64>,
    {
        stat::increment(stat::Counter::UpdatePessimistic);
        let Ok(update) =
            unsafe { self.update_with_raw::<path::Retain<_>, _>(guard, reader, initial, update) };
        update
    }

    #[inline]
    unsafe fn update_with_raw<'g, 'k, P, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'k>,
        mut initial: Option<u64>,
        mut update: F,
    ) -> Result<UpdateRaw, P::PopError>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(u64, Option<u64>) -> ControlFlow<(), u64>,
    {
        let mut cursor = unsafe { self.seq.raw.cursor::<P>(reader) };

        loop {
            let cursor::Update {
                value: old_value,
                edge: old_edge,
            } = match cursor.traverse_update() {
                None => return Ok(UpdateRaw::Absent { new: initial }),
                Some(update) if !update.edge.meta().is_frozen() => update,
                Some(_) => match cursor.freeze()? {
                    None => continue,
                    Some(node) => unsafe {
                        guard.retire_node(cursor.len().bits(), node.into_raw());
                        continue;
                    },
                },
            };

            let new_value = match update(old_value, initial) {
                ControlFlow::Continue(new_value) => new_value,
                ControlFlow::Break(()) => {
                    return Ok(UpdateRaw::Break { old: old_value });
                }
            };

            match cursor.edge().compare_exchange_packed(
                old_edge,
                Edge::new_value(old_edge.meta(), new_value),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(UpdateRaw::Success {
                        old: old_value,
                        new: new_value,
                    });
                }
                Err(_) => {
                    initial = Some(new_value);
                }
            }
        }
    }

    #[inline]
    unsafe fn remove_non_recursive_with_optimistic<'g, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'_>,
        remove: F,
    ) -> Result<RemoveRaw, ()>
    where
        F: FnMut(u64) -> ControlFlow<(), ()>,
    {
        unsafe { self.remove_with_raw::<false, path::Discard, _>(guard, reader, remove) }
    }

    #[cold]
    unsafe fn remove_non_recursive_with_pessimistic<'g, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'_>,
        remove: F,
    ) -> RemoveRaw
    where
        F: FnMut(u64) -> ControlFlow<(), ()>,
    {
        let Ok(remove) =
            unsafe { self.remove_with_raw::<false, path::Retain<_>, _>(guard, reader, remove) };
        remove
    }

    #[inline]
    unsafe fn remove_with_raw<'g, 'k, const RECURSIVE: bool, P, F>(
        &'g self,
        guard: &mut S::Guard<'g>,
        reader: K::Read<'k>,
        mut remove: F,
    ) -> Result<RemoveRaw, P::PopError>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(u64) -> ControlFlow<(), ()>,
    {
        let mut cursor = unsafe { self.seq.raw.cursor::<P>(reader) };

        let (value, edge) = loop {
            let cursor::Update { value, edge } = match cursor.traverse_update() {
                None => return Ok(RemoveRaw::Absent),
                Some(update) if !update.edge.meta().is_frozen() => update,
                Some(_) => match cursor.freeze()? {
                    None => continue,
                    Some(node) => unsafe {
                        guard.retire_node(cursor.len().bits(), node.into_raw());
                        continue;
                    },
                },
            };

            match remove(value) {
                ControlFlow::Continue(()) => (),
                ControlFlow::Break(()) => {
                    return Ok(RemoveRaw::Break { old: value });
                }
            }

            if cursor
                .edge()
                .compare_exchange_packed(edge, Edge::NULL, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break (value, edge);
            }
        };

        if RECURSIVE {
            let mut trim = edge.meta().len();

            'outer: while let Some(target) = cursor
                .pop()
                .unwrap_or_else(|_| panic!("Recursive remove requires path"))
            {
                if unsafe { target.len::<K::Edge>() } > 1 {
                    break 'outer;
                }

                cursor.trim(K::Len::BYTE + trim.into());

                loop {
                    let old = match cursor.traverse_prefix() {
                        None => break 'outer,
                        Some(old) if !old.meta().is_frozen() => old,
                        Some(_) => match cursor.freeze() {
                            Err(_) => unreachable!("Recursive remove requires path"),
                            Ok(None) => continue,
                            Ok(Some(node)) => unsafe {
                                guard.retire_node(cursor.len().bits(), node.into_raw());
                                continue;
                            },
                        },
                    };

                    let (smo, new) = match old.child() {
                        None => break 'outer,
                        Some(edge::Child::Value(_)) => unreachable!("Prefix precondition"),
                        Some(edge::Child::Node(node)) if node == target => unsafe {
                            node.freeze::<K::Edge>();
                            node.replace(old.meta())
                        },
                        // Must have been replaced by someone else
                        Some(edge::Child::Node(_)) => break 'outer,
                    };

                    match cursor.edge().compare_exchange_packed(
                        old,
                        new,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(old) => {
                            unsafe { guard.retire_node(cursor.len().bits(), target.into_raw()) };
                            trim = old.meta().len();
                            continue 'outer;
                        }
                        Err(_) => {
                            if smo.is_allocate()
                                && let Some(node) = new.as_node()
                            {
                                stat::increment(stat::Counter::FreeConflict);
                                unsafe { node.deallocate() };
                            }
                        }
                    }
                }
            }
        }

        Ok(RemoveRaw::Success { old: value })
    }
}

impl<K, V, S> From<sequential::Map<K, V>> for Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Default,
{
    #[inline]
    fn from(seq: sequential::Map<K, V>) -> Self {
        Self {
            smr: S::default(),
            seq,
        }
    }
}

impl<K, V, S> From<Map<K, V, S>> for sequential::Map<K, V>
where
    K: Key,
    V: Value,
{
    #[inline]
    fn from(map: Map<K, V, S>) -> sequential::Map<K, V> {
        map.seq
    }
}

#[cfg(test)]
mod tests {
    use crate::Ascend;
    use crate::Descend;
    use crate::concurrent::Map;
    use crate::key::BoxedSlice;
    use crate::key::BoxedStr;
    use crate::key::NonNull;
    use crate::key::Slice;
    use crate::key::Str;
    use crate::key::Terminated;
    use crate::raw::key::Read as _;

    #[test]
    fn smoke() {
        let map = Map::<BoxedStr<NonNull>, _>::default();
        map.upsert(unsafe { Slice::new_unchecked("abcd") }, 1u64);
        assert_eq!(
            map.get(unsafe { Slice::new_unchecked("abcd") })
                .as_deref()
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn smoke_u64_key() {
        let map = Map::<[u8; 8], _>::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        map.upsert(&key, 1u64);
        assert_eq!(map.get(&key).as_deref().copied(), Some(1));
    }

    #[test]
    fn smoke_value_ref() {
        let values = [0, 1, 2, 3, 4, 5];
        let map = Map::<u64, &u64>::default();

        for (key, value) in values.iter().enumerate() {
            map.upsert(key as u64, value);
        }

        #[expect(clippy::needless_range_loop)]
        for key in 0..values.len() {
            let value = map.get(&(key as u64)).as_deref().copied().unwrap();
            assert!(core::ptr::eq(value, &values[key]));
        }
    }

    #[test]
    fn smoke_value_box() {
        let values = [0, 1, 2, 3, 4, 5];
        let map = Map::<u64, Box<u64>>::default();

        for (key, value) in values.iter().enumerate() {
            map.upsert(key as u64, Box::new(*value));
        }

        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for key in (0..values.len()).cycle().take(100_000) {
                        let value = map.get(&(key as u64)).as_deref().copied().unwrap();
                        assert_eq!(key, value as usize);
                    }
                });
            }
        });

        // TODO: multiple hazards?
        // let a = map.get(3);
        // let b = map.get(5);
        // assert_ne!(a.as_deref(), b.as_deref());

        for key in 0..values.len() {
            let value = map.get(&(key as u64)).as_deref().copied().unwrap();
            assert_eq!(key, value as usize);
        }
    }

    #[test]
    fn scan_value() {
        let map = Map::<u64, _>::default();
        let key = 1u64;
        map.upsert(key, 2u64);
        assert_eq!(
            map.range(1u64..=1u64)
                .entries::<Ascend>()
                .collect::<Vec<_>>(),
            vec![(1, 2)]
        );
    }

    #[test]
    fn scan_node3() {
        insert_all(0u64..3);
    }

    #[test]
    fn scan_node256() {
        insert_all(0u64..256);
    }

    #[test]
    fn scan_gap() {
        let map = insert_all((0u64..512).step_by(2));
        assert_eq!(
            map.range(256u64..=511u64)
                .entries::<Ascend>()
                .collect::<Vec<_>>(),
            (256..512)
                .step_by(2)
                .map(|key| (key, key / 2))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn node3_overwrite() {
        let mut map = Map::<u64, _>::default();

        for value in [1u64, 2, 3] {
            map.upsert(1, value);
            assert_eq!(map.get(&1).as_deref().copied(), Some(value));
        }

        assert_eq!(map.as_sequential().all().entries::<Ascend>().count(), 1);

        map.as_sequential()
            .all()
            .entries::<Ascend>()
            .for_each_internal(|(key, value)| {
                assert_eq!(key, 1);
                assert_eq!(*value, 3);
                core::ops::ControlFlow::Continue(())
            });
    }

    #[test]
    fn node3_reverse() {
        insert_all((0u16..3).rev());
    }

    #[test]
    fn node3_full() {
        insert_all(0u16..3);
    }

    #[test]
    fn node3_expand() {
        insert_all(0u16..4);
    }

    #[test]
    fn node15_full() {
        insert_all(0u16..15);
    }

    #[test]
    fn node15_expand() {
        insert_all(0u16..16);
    }

    #[test]
    fn node47_full() {
        insert_all(0u16..47);
    }

    #[test]
    fn node47_expand() {
        insert_all(0u16..61);
    }

    #[test]
    fn node256_full() {
        insert_all(0u16..=255);
    }

    #[test]
    fn range_reverse() {
        let map = Map::<u64, _>::default();

        for key in [5, 1, 4, 3, 2] {
            map.upsert(key, key);
            assert_eq!(map.get(&key).as_deref().copied(), Some(key));
        }

        assert_eq!(
            map.range(2..=4).entries::<Descend>().collect::<Vec<_>>(),
            vec![(4, 4), (3, 3), (2, 2)]
        );
    }

    #[test]
    fn split_edges() {
        let mut key = (1..100).collect::<Vec<_>>();
        insert_all(core::iter::from_fn(|| {
            if key.is_empty() {
                None
            } else {
                let mut next = key.clone();
                next.push(0);
                key.pop();
                let next = next.into_boxed_slice();
                Some(BoxedSlice::<Terminated<0>>::new(next).unwrap())
            }
        }));
    }

    #[test]
    fn one_long_key() {
        insert_all([BoxedStr::<NonNull>::new("a".repeat(1000)).unwrap()]);
    }

    #[test]
    fn short_key() {
        insert_all([BoxedStr::<NonNull>::new("\n".to_string()).unwrap()]);
    }

    #[test]
    fn two_long_keys() {
        insert_all([
            BoxedStr::<NonNull>::new("a".repeat(1000)).unwrap(),
            BoxedStr::<NonNull>::new("b".repeat(1000)).unwrap(),
        ]);
    }

    #[test]
    fn smoke_key_slice() {
        let keys = ["ad", "abc"];
        let map = crate::concurrent::Map::<&Str<NonNull>, u64>::new();
        map.insert(Str::new(keys[0]).unwrap(), 0)
            .unwrap_or_else(|(_, _)| panic!());
        map.insert(Str::new(keys[1]).unwrap(), 1)
            .unwrap_or_else(|(_, _)| panic!());

        let temp = "adabc";
        assert_eq!(
            map.get(Str::new(&temp[..2]).unwrap()).as_deref().copied(),
            Some(0)
        );
        assert_eq!(
            map.get(Str::new(&temp[2..]).unwrap()).as_deref().copied(),
            Some(1)
        );
    }

    #[test]
    fn key_slice_long_prefix() {
        let keys = (0..10)
            .map(|i| "a".repeat(100) + &i.to_string())
            .collect::<Vec<_>>();
        let map = crate::concurrent::Map::<&Slice<NonNull>, u64>::new();
        for (i, key) in keys.iter().enumerate() {
            map.insert(Slice::new(key.as_bytes()).unwrap(), i as u64)
                .unwrap();
        }
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(
                map.get(Slice::new(key.as_bytes()).unwrap())
                    .as_deref()
                    .copied(),
                Some(i as u64)
            );
        }
    }

    fn insert_all<I, K>(iter: I) -> Map<K, u64>
    where
        I: IntoIterator<Item = K>,
        K: crate::Key + Clone + Ord + core::fmt::Debug,
    {
        let mut keys = iter
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u64))
            .collect::<Vec<_>>();

        let mut map = Map::default();

        for (key, value) in &keys {
            map.upsert(key.as_insert(), *value);
            assert_eq!(map.get(key.borrow()).as_deref().copied(), Some(*value));
        }

        for (key, value) in &keys {
            assert_eq!(map.get(key.borrow()).as_deref().copied(), Some(*value));
        }

        let mut iter = map.as_sequential().all().entries::<Ascend>();
        let mut count = 0;
        while iter.lend().is_some() {
            count += 1;
        }
        drop(iter);

        assert_eq!(count, keys.len());

        keys.sort_by(|(l, _), (r, _)| l.cmp(r));

        // Sequential iteration
        map.as_sequential()
            .all()
            .entries::<Ascend>()
            .zip(&keys)
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(*lv, *rv);
            });

        let Some(((first, _), (last, _))) = keys.first().zip(keys.last()) else {
            return map;
        };

        // Concurrent prefix scan, non-linearizable
        map.prefix(K::Read::from(first.borrow()).common_prefix(K::Read::from(last.borrow())))
            .entries::<Descend>()
            .zip(keys.iter().rev())
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });

        // Concurrent range scan, non-linearizable
        let mut i = 0;
        map.range(first.borrow()..=last.borrow())
            .entries::<Descend>()
            .zip(keys.iter().rev())
            .for_each(|((lk, lv), (rk, rv))| {
                i += 1;
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });
        assert_eq!(i, keys.len());

        map
    }
}
