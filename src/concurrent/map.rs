//! Auxiliary types for use with [`crate::concurrent::Map`].

use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

use crate::Key;
use crate::concurrent::Shard;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::iter;
use crate::concurrent::smr;
use crate::concurrent::smr::Guard as _;
use crate::concurrent::value;
use crate::raw::Edge;
use crate::raw::Frozen;
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
/// Refer to [`crate::sequential::Map`] for an introduction.
/// The [`Map`] API differs in three ways: concurrent operations,
/// safe memory reclamation, and advanced point operations.
///
/// ## Concurrent operations
///
/// Unlike [`crate::sequential::Map`], an instance of [`Map`] can be shared
/// and modified concurrently across threads. Methods that usually require a mutable reference
/// (e.g., [`crate::sequential::Map::upsert`]) instead use atomics to synchronize internally,
/// allowing them to take an immutable reference (e.g., [`Map::upsert`]).
///
/// Note that scan operations are not linearizable. They do, however,
/// satisfy weaker guarantees: (a) scans observe keys at most once, in order;
/// and (b) scans observe all keys within bounds that were inserted before
/// the scan starts, and were not removed before the scan ends.
///
/// ## Safe memory reclamation
///
/// In order to provide wait-free reads, [`Map`] requires
/// a safe memory reclamation (SMR) mechanism to detect when
/// allocations are safe to free. This results in the following API changes:
///
/// 1. Values are always returned behind guards. For example,
///    while a successful [`crate::sequential::Map::update`] returns ownership of
///    the old value, a successful [`Map::update`] instead returns an [`Updated`]
///    guard that allows references to the old and new value.
///
///    The guard may have other restrictions depending on the SMR implementation:
///    for example, epoch-based SMR cannot free any memory while a guard is alive,
///    and hazard keys currently only support holding a single guard at a time.
///
/// 2. Values behind guards are always read-only. This can be worked around by
///    either using a value type with internal synchronization (e.g., `Box<Mutex<T>>`),
///    or by obtaining a mutable reference to [`Map`] and then using the
///    sequential API via [`Map::as_sequential`].
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
/// operations (ending in suffix `_with`, e.g., [`Map::update_with`]) that
/// take a closure.
///
/// This can be used to efficiently implement lazy value initialization,
/// or synchronization logic where the next value is computed from the
/// current value, and then atomically inserted or updated.
pub struct Map<K: Key, V: Value, S = Box<smr::Hazard<K, V>>> {
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
    /// Get a mutable view as a [`sequential::Map`] for temporary access to a more
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
    /// use arctic::concurrent;
    /// use arctic::sequential;
    /// use arctic::concurrent::smr;
    ///
    /// let mut map = concurrent::Map::<u32, u64>::default();
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
    /// For a mutable reference, see [`Map::as_sequential`] and [`sequential::Map::get_mut`].
    /// There is no way to safely get a mutable reference to a value from an immutable [`Map`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    ///
    /// let map = concurrent::Map::<u64, u64>::default();
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
        self.get_impl(reader)
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
    /// See [`Map::insert_with`] for dynamic control flow and value construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    /// use arctic::NonNullStr;
    /// use arctic::NonNullString;
    ///
    /// let map = concurrent::Map::<NonNullString, u64>::default();
    /// let key = NonNullStr::new("korlex").expect("Non-empty and no null byte");
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
    /// See [`Map::upsert_with`] for dynamic control flow and value construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    /// use arctic::NonNullString;
    /// use arctic::NonNullStr;
    ///
    /// let map = concurrent::Map::<NonNullString, u64>::default();
    /// let key = NonNullStr::new("arqad").expect("Non-empty and no null byte");
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
    /// See [`Map::update_with`] for dynamic control flow and value construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    ///
    /// let map = concurrent::Map::<u32, Box<u64>>::default();
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
    /// delete empty nodes. See [`Map::remove_non_recursive`] for a faster,
    /// but potentially memory-intensive alternative.
    ///
    /// Returns `Some(&old_value)` if the remove succeeded, or else `None` if
    /// there was no old value associated with `key`.
    ///
    /// See [`Map::remove_with`] for dynamic control flow.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    ///
    /// let map = concurrent::Map::<u128, u64>::default();
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
    /// This method is much faster than [`Map::remove`], because no traversal
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
    /// See [`Map::remove_non_recursive_with`] for dynamic control flow.
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
    pub fn all(&self) -> iter::Shard<'_, 'static, K, V, RangeFull, Guard<'_, K, V, S>> {
        let guard = self.smr.guard(K::Read::default());
        unsafe { Shard::new(guard, self.seq.raw.all()) }
    }

    /// Get an immutable reference to the subtree of keys beginning with `prefix`.
    pub fn prefix<'g, 'k>(
        &'g self,
        prefix: impl Into<K::Read<'k>>,
    ) -> iter::Shard<'g, 'k, K, V, RangeFull, Guard<'g, K, V, S>> {
        let prefix = prefix.into();
        let guard = self.smr.guard(prefix);
        unsafe { Shard::new(guard, self.seq.raw.prefix(prefix)) }
    }

    /// Get an immutable reference to the subtree of keys within `range`.
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
/// For example, a concurrent counter could use [`Map::upsert_with`] to either
/// insert one or update the current count by one, or an index could use
/// [`Map::remove_with`] to remove a value only if it hasn't been concurrently
/// updated.
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
    /// use arctic::concurrent;
    /// use arctic::NonNullStr;
    /// use arctic::NonNullString;
    ///
    /// let map = concurrent::Map::<NonNullString, Box<u64>>::default();
    /// let key = NonNullStr::new("zipir").expect("Non-empty and no null byte");
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
    /// use arctic::concurrent;
    /// use arctic::concurrent::map::Upsert;
    ///
    /// let map = concurrent::Map::<u16, Box<u64>>::default();
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
        initial: Option<V>,
        mut upsert: F,
    ) -> Upsert<'g, K, V, S>
    where
        F: FnMut(Option<&V::Borrowed>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::insert_as_read(key);
        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            match self.upsert_with_optimistic(reader, initial, &mut upsert) {
                Ok(update) => return update,
                Err(initial) => initial,
            }
        };

        self.upsert_with_pessimistic(reader, initial, upsert)
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
    /// use arctic::concurrent;
    /// use arctic::concurrent::map::Update;
    ///
    /// let map = concurrent::Map::<u64, Box<u64>>::default();
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
        initial: Option<V>,
        mut update: F,
    ) -> Update<'g, K, V, S>
    where
        F: FnMut(&V::Borrowed, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::Read::from(key);
        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            match self.update_with_optimistic(reader, initial, &mut update) {
                Ok(update) => return update,
                Err(initial) => initial,
            }
        };

        self.update_with_pessimistic(reader, initial, update)
    }

    /// If there is a value associated with `key`, call `remove` to determine whether
    /// to remove the value, recursively removing empty tree nodes.
    ///
    /// Returns a [`Remove`] enum.
    ///
    /// See also: [`Map::remove`], [`Map::remove_non_recursive`], [`Map::remove_non_recursive_with`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use core::ops::ControlFlow;
    ///
    /// use arctic::concurrent;
    /// use arctic::concurrent::map::Remove;
    ///
    /// let map = concurrent::Map::<u128, u64>::default();
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
        let Ok(remove) = self.remove_with_impl::<true, path::Retain<_>, _>(reader, &mut remove);
        remove
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
    /// See also: [`Map::remove`], [`Map::remove_with`], [`Map::remove_non_recursive`].
    pub fn remove_non_recursive_with<F>(
        &self,
        key: &K::Borrowed,
        mut with: F,
    ) -> Remove<'_, K, V, S>
    where
        F: FnMut(&V::Borrowed) -> ControlFlow<(), ()>,
    {
        let reader = K::Read::from(key);
        match self.remove_non_recursive_with_optimistic(reader, &mut with) {
            Ok(remove) => remove,
            Err(()) => self.remove_non_recursive_with_pessimistic(reader, &mut with),
        }
    }
}

/// Outcome of a call to [`Map::upsert_with`].
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

/// Outcome of a call to [`Map::update_with`].
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

/// Outcome of a call to [`Map::remove_with`].
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

/// # Private implementations
impl<K, V, S> Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Smr<K, V>,
{
    #[inline]
    fn get_impl<'g>(&'g self, reader: K::Read<'_>) -> Option<Shared<'g, K, V, S>> {
        let guard = self.smr.guard(reader);
        let value = unsafe {
            self.seq
                .raw
                .cursor::<path::Discard>(reader)
                .traverse_get()?
        };
        Some(unsafe { Shared::<'_, K, V, S>::wrap(guard, value) })
    }

    #[inline]
    fn upsert_with_optimistic<'g, 'k, F>(
        &'g self,
        reader: K::Read<'k>,
        initial: Option<V>,
        upsert: F,
    ) -> Result<Upsert<'g, K, V, S>, Option<V>>
    where
        F: FnMut(Option<&V::Borrowed>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        self.upsert_with_impl::<path::Discard, _>(reader, initial, upsert)
    }

    #[cold]
    fn upsert_with_pessimistic<'g, 'k, F>(
        &'g self,
        reader: K::Read<'k>,
        initial: Option<V>,
        upsert: F,
    ) -> Upsert<'g, K, V, S>
    where
        F: FnMut(Option<&V::Borrowed>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        stat::increment(stat::Counter::InsertPessimistic);
        match self.upsert_with_impl::<path::Retain<_>, _>(reader, initial, upsert) {
            Ok(upsert) => upsert,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn upsert_with_impl<'g, 'k, P, F>(
        &'g self,
        reader: K::Read<'k>,
        mut initial: Option<V>,
        mut upsert: F,
    ) -> Result<Upsert<'g, K, V, S>, Option<V>>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(Option<&V::Borrowed>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let mut guard = self.smr.guard(reader);
        let mut cursor = unsafe { self.seq.raw.cursor::<P>(reader) };

        loop {
            match cursor.traverse_insert() {
                cursor::Insert::Value {
                    value: old_value,
                    edge: old,
                } => {
                    let new_value = match upsert(
                        old_value
                            .as_ref()
                            .map(|old| unsafe { V::borrow_from_raw_unchecked(old) }),
                        &mut initial,
                    ) {
                        ControlFlow::Continue(value) => V::into_raw(value),
                        ControlFlow::Break(()) => {
                            return Ok(Upsert::Break {
                                old: old_value.map(|old_value| unsafe {
                                    Shared::<K, V, S>::wrap(guard, old_value)
                                }),
                                new: initial,
                            });
                        }
                    };

                    match cursor.create_path(old, new_value) {
                        // Restore value and fall through to freeze
                        Err(Frozen) => initial = Some(unsafe { V::from_raw_unchecked(new_value) }),

                        Ok((new, _)) => match cursor.edge().compare_exchange_packed(
                            old,
                            new,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => {
                                return Ok(Upsert::Success(unsafe {
                                    Upserted::<K, V, S>::wrap(guard, old_value, new_value)
                                }));
                            }
                            Err(_) => {
                                if let Some(node) = new.as_node() {
                                    unsafe {
                                        stat::increment(stat::Counter::FreeConflict);
                                        node.deallocate_recursive::<K::Edge>();
                                    }
                                }

                                initial = Some(unsafe { V::from_raw_unchecked(new_value) });
                                continue;
                            }
                        },
                    }
                }
                cursor::Insert::Replace {
                    node: old_node,
                    edge: old,
                } if !old.meta().is_frozen() => {
                    let (smo, new) = unsafe {
                        old_node.freeze::<K::Edge>();
                        old_node.replace(old.meta())
                    };
                    match cursor.edge().compare_exchange_packed(
                        old,
                        new,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            unsafe { guard.retire_node(cursor.len().bits(), old_node) };
                        }
                        Err(_) => {
                            // Does not go through SMR because `new` is still thread-local
                            if smo.is_allocate() {
                                let node = new.as_node().expect("Allocating SMO creates node");
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

            match cursor.freeze() {
                Err(_) => return Err(initial),
                Ok(None) => (),
                Ok(Some(node)) => unsafe { guard.retire_node(cursor.len().bits(), node) },
            }
        }
    }

    #[inline]
    fn update_with_optimistic<'g, F>(
        &'g self,
        reader: K::Read<'_>,
        initial: Option<V>,
        update: F,
    ) -> Result<Update<'g, K, V, S>, Option<V>>
    where
        F: FnMut(&V::Borrowed, &mut Option<V>) -> ControlFlow<(), V>,
    {
        self.update_with_impl::<path::Discard, _>(reader, initial, update)
    }

    #[cold]
    fn update_with_pessimistic<'g, F>(
        &'g self,
        reader: K::Read<'_>,
        initial: Option<V>,
        update: F,
    ) -> Update<'g, K, V, S>
    where
        F: FnMut(&V::Borrowed, &mut Option<V>) -> ControlFlow<(), V>,
    {
        stat::increment(stat::Counter::UpdatePessimistic);
        match self.update_with_impl::<path::Retain<_>, _>(reader, initial, update) {
            Ok(update) => update,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn update_with_impl<'g, 'k, P, F>(
        &'g self,
        reader: K::Read<'k>,
        mut initial: Option<V>,
        mut update: F,
    ) -> Result<Update<'g, K, V, S>, Option<V>>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(&V::Borrowed, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let mut guard = self.smr.guard(reader);
        let mut cursor = unsafe { self.seq.raw.cursor::<P>(reader) };

        loop {
            let updated = match cursor.traverse_update() {
                None => return Ok(Update::Absent { new: initial }),
                Some(Ok(old)) => old,
                Some(Err(Frozen)) => match cursor.freeze() {
                    Err(_) => return Err(initial),
                    Ok(None) => continue,
                    Ok(Some(node)) => unsafe {
                        guard.retire_node(cursor.len().bits(), node);
                        continue;
                    },
                },
            };

            let new_value = match update(
                unsafe { V::borrow_from_raw_unchecked(&updated.value) },
                &mut initial,
            ) {
                ControlFlow::Continue(new) => V::into_raw(new),
                ControlFlow::Break(()) => {
                    return Ok(Update::Break {
                        old: unsafe { Shared::<K, V, S>::wrap(guard, updated.value) },
                        new: initial,
                    });
                }
            };

            match cursor.edge().compare_exchange_packed(
                updated.edge,
                Edge::new_value(updated.edge.meta(), new_value),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Update::Success(unsafe {
                        Updated::<K, V, S>::wrap(guard, updated.value, new_value)
                    }));
                }
                Err(_) => {
                    initial = Some(unsafe { V::from_raw_unchecked(new_value) });
                }
            }
        }
    }

    #[inline]
    fn remove_non_recursive_with_optimistic<F>(
        &self,
        reader: K::Read<'_>,
        remove: &mut F,
    ) -> Result<Remove<'_, K, V, S>, ()>
    where
        F: FnMut(&V::Borrowed) -> ControlFlow<(), ()>,
    {
        self.remove_with_impl::<false, path::Discard, _>(reader, remove)
    }

    #[cold]
    fn remove_non_recursive_with_pessimistic<F>(
        &self,
        reader: K::Read<'_>,
        remove: &mut F,
    ) -> Remove<'_, K, V, S>
    where
        F: FnMut(&V::Borrowed) -> ControlFlow<(), ()>,
    {
        let Ok(remove) = self.remove_with_impl::<false, path::Retain<_>, _>(reader, remove);
        remove
    }

    #[inline]
    fn remove_with_impl<'g, 'k, const RECURSIVE: bool, P, F>(
        &'g self,
        reader: K::Read<'k>,
        remove: &mut F,
    ) -> Result<Remove<'g, K, V, S>, P::PopError>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(&V::Borrowed) -> ControlFlow<(), ()>,
    {
        let mut guard = self.smr.guard(reader);
        let mut cursor = unsafe { self.seq.raw.cursor::<P>(reader) };

        let updated = loop {
            let updated = match cursor.traverse_update() {
                None => return Ok(Remove::Absent),
                Some(Ok(old)) => old,
                Some(Err(Frozen)) => match cursor.freeze()? {
                    None => continue,
                    Some(node) => unsafe {
                        guard.retire_node(cursor.len().bits(), node);
                        continue;
                    },
                },
            };

            match remove(unsafe { V::borrow_from_raw_unchecked(&updated.value) }) {
                ControlFlow::Continue(()) => (),
                ControlFlow::Break(()) => {
                    return Ok(Remove::Break {
                        old: unsafe { Shared::<K, V, S>::wrap(guard, updated.value) },
                    });
                }
            }

            if cursor
                .edge()
                .compare_exchange_packed(
                    updated.edge,
                    Edge::NULL,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                break updated;
            }
        };

        if RECURSIVE {
            let mut trim = updated.edge.meta().len();

            'outer: while let Some(target) = cursor
                .pop()
                .unwrap_or_else(|_| panic!("Recursive remove requires path"))
            {
                if unsafe { target.len::<K::Edge>() } > 0 {
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
                                guard.retire_node(cursor.len().bits(), node);
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
                            unsafe { guard.retire_node(cursor.len().bits(), target) };
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

        Ok(Remove::Success {
            old: unsafe { Owned::<K, V, S>::wrap(guard, updated.value) },
        })
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
