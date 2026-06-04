use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

use crate::concurrent::Key;
use crate::concurrent::Shard;
use crate::concurrent::Smr;
use crate::concurrent::Value;
use crate::concurrent::iter;
use crate::concurrent::smr;
use crate::concurrent::smr::Global as _;
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
pub type Guard<'g, K, V, S> = <<S as Smr>::Global<K, V> as smr::Global<K, V>>::Guard<'g>;

/// See [`value::Owned`].
pub type Owned<'g, K, V, S> = value::Owned<Guard<'g, K, V, S>, V>;

/// See [`value::Shared`].
pub type Shared<'g, K, V, S> = value::Shared<Guard<'g, K, V, S>, V>;

/// See [`value::Updated`].
pub type Updated<'g, K, V, S> = value::Updated<Guard<'g, K, V, S>, V>;

/// See [`value::Upserted`].
pub type Upserted<'g, K, V, S> = value::Upserted<Guard<'g, K, V, S>, V>;

pub struct Map<K: Key, V: Value, S: Smr = smr::Hazard> {
    smr: S::Global<K, V>,
    seq: sequential::Map<K, V>,
}

unsafe impl<K: Key, V: Value + Send + Sync, S: Smr> Sync for Map<K, V, S> {}

impl<K: crate::Key, V: Value, S: Smr> Default for Map<K, V, S> {
    fn default() -> Self {
        Self {
            smr: S::Global::default(),
            seq: sequential::Map::<K, V>::default(),
        }
    }
}

impl<K: Key, V: Value, S: Smr> Map<K, V, S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_smr(smr: S::Global<K, V>) -> Self {
        Self {
            smr,
            seq: sequential::Map::<K, V>::default(),
        }
    }

    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        &mut self.seq
    }

    #[inline]
    pub fn smr(&self) -> &S::Global<K, V> {
        &self.smr
    }

    #[inline]
    pub fn smr_mut(&mut self) -> &mut S::Global<K, V> {
        &mut self.smr
    }
}

/// Outcome of a call to [`Map::update_with`].
pub enum Update<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K, V>: 'g,
{
    /// Key was not present.
    Absent {
        /// Latest value passed as argument or returned from caller closure.
        new: Option<V>,
    },
    /// Value was successfully updated.
    Success(Updated<'g, K, V, S>),
    /// Caller closure returned [`core::ops::ControlFlow::Break`].
    Break {
        /// Latest value observed by closure.
        old: Shared<'g, K, V, S>,
        new: Option<V>,
    },
}

/// Outcome of a call to [`Map::remove_with`].
pub enum Remove<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K, V>: 'g,
{
    /// Key was not present.
    Absent,
    /// Value was successfully removed.
    Success { old: Owned<'g, K, V, S> },
    /// Caller closure returned [`core::ops::ControlFlow::Break`].
    Break {
        /// Latest value observed by closure.
        old: Shared<'g, K, V, S>,
    },
}

/// Outcome of a call to [`Map::upsert_with`].
pub enum Upsert<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K, V>: 'g,
{
    /// Value was successfully upserted.
    Success(Upserted<'g, K, V, S>),
    /// Caller closure returned [`core::ops::ControlFlow::Break`].
    Break {
        /// Latest value observed by closure.
        old: Option<Shared<'g, K, V, S>>,
        /// Latest value passed as argument or returned from caller closure.
        new: Option<V>,
    },
}

impl<K, V, S> Map<K, V, S>
where
    K: Key,
    V: Value + Send + Sync,
    S: Smr,
{
    /// Retrieve a key-value pair.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    ///
    /// let mut map = concurrent::Map::<u64, u64>::default();
    ///
    /// assert!(map.get(&64).is_none());
    ///
    /// match map.insert(&64, 3) {
    ///     Err(_) => unreachable!(),
    ///     Ok(new) => assert_eq!(*new, 3),
    /// }
    ///
    /// match map.get(&64) {
    ///     None => unreachable!(),
    ///     Some(value) => assert_eq!(*value, 3),
    /// }
    /// ```
    pub fn get<'g>(&'g self, key: &K::Borrowed) -> Option<Shared<'g, K, V, S>> {
        let reader = K::Read::from(key);
        let guard = self.smr.guard(reader);
        let value = unsafe {
            self.seq
                .raw
                .cursor::<path::Discard>(reader)
                .traverse_get()?
        };
        Some(unsafe { Shared::<'_, K, V, S>::wrap(guard, value) })
    }

    /// Update an existing key-value pair.
    ///
    /// Returns references to the old and newly updated value if the update succeeded,
    /// or else returns the owned new value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    ///
    /// let mut map = concurrent::Map::<u32, Box<u64>>::default();
    ///
    /// match map.update(&37, Box::new(5)) {
    ///     Err(new) => assert_eq!(*new, 5),
    ///     Ok(_) => unreachable!(),
    /// }
    ///
    /// match map.insert(&37, Box::new(3)) {
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

    pub fn update_with<'g, F>(
        &'g self,
        key: &K::Borrowed,
        initial: Option<V>,
        mut update: F,
    ) -> Update<'g, K, V, S>
    where
        F: FnMut(&V::Target, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            match self.update_with_optimistic(key, initial, &mut update) {
                Ok(update) => return update,
                Err(initial) => initial,
            }
        };

        self.update_with_pessimistic(key, initial, update)
    }

    #[inline]
    fn update_with_optimistic<'g, F>(
        &'g self,
        key: &K::Borrowed,
        initial: Option<V>,
        update: F,
    ) -> Result<Update<'g, K, V, S>, Option<V>>
    where
        F: FnMut(&V::Target, &mut Option<V>) -> ControlFlow<(), V>,
    {
        self.update_with_impl::<path::Discard, _>(key, initial, update)
    }

    #[cold]
    fn update_with_pessimistic<'g, F>(
        &'g self,
        key: &K::Borrowed,
        initial: Option<V>,
        update: F,
    ) -> Update<'g, K, V, S>
    where
        F: FnMut(&V::Target, &mut Option<V>) -> ControlFlow<(), V>,
    {
        stat::increment(stat::Counter::UpdatePessimistic);
        match self.update_with_impl::<path::Retain<_>, _>(key, initial, update) {
            Ok(update) => update,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn update_with_impl<'g, 'k, P, F>(
        &'g self,
        key: &'k K::Borrowed,
        mut initial: Option<V>,
        mut update: F,
    ) -> Result<Update<'g, K, V, S>, Option<V>>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(&V::Target, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::Read::from(key);
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

            let new_value =
                match update(unsafe { V::target_from_raw(&updated.value) }, &mut initial) {
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
                    initial = Some(unsafe { V::from_raw(new_value) });
                }
            }
        }
    }

    pub fn remove_non_recursive(&self, key: &K::Borrowed) -> Option<Owned<'_, K, V, S>> {
        match self.remove_non_recursive_with(key, |_| ControlFlow::Continue(())) {
            Remove::Absent => None,
            Remove::Success { old } => Some(old),
            Remove::Break { old: _ } => unreachable!(),
        }
    }

    pub fn remove_non_recursive_with<F>(
        &self,
        key: &K::Borrowed,
        mut with: F,
    ) -> Remove<'_, K, V, S>
    where
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        match self.remove_non_recursive_with_optimistic(key, &mut with) {
            Ok(remove) => remove,
            Err(()) => self.remove_non_recursive_with_pessimistic(key, &mut with),
        }
    }

    #[inline]
    fn remove_non_recursive_with_optimistic<F>(
        &self,
        key: &K::Borrowed,
        with: &mut F,
    ) -> Result<Remove<'_, K, V, S>, ()>
    where
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        self.remove_with_impl::<false, path::Discard, _>(key, with)
    }

    #[cold]
    fn remove_non_recursive_with_pessimistic<F>(
        &self,
        key: &K::Borrowed,
        with: &mut F,
    ) -> Remove<'_, K, V, S>
    where
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        let Ok(remove) = self.remove_with_impl::<false, path::Retain<_>, _>(key, with);
        remove
    }

    pub fn remove<'g>(&'g self, key: &K::Borrowed) -> Option<Owned<'g, K, V, S>> {
        match self.remove_with(key, |_| ControlFlow::Continue(())) {
            Remove::Absent => None,
            Remove::Success { old } => Some(old),
            Remove::Break { old: _ } => unreachable!(),
        }
    }

    pub fn remove_with<'g, F>(&'g self, key: &K::Borrowed, mut with: F) -> Remove<'g, K, V, S>
    where
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        let Ok(remove) = self.remove_with_impl::<true, path::Retain<_>, _>(key, &mut with);
        remove
    }

    #[inline]
    fn remove_with_impl<'g, 'k, const RECURSIVE: bool, P, F>(
        &'g self,
        key: &'k K::Borrowed,
        remove: &mut F,
    ) -> Result<Remove<'g, K, V, S>, P::PopError>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        let reader = K::Read::from(key);
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

            match remove(unsafe { V::target_from_raw(&updated.value) }) {
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
                if unsafe { target.len() } > 0 {
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
                            node.replace(old.meta(), true)
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
                                unsafe { node.deallocate(stat::Counter::FreeConflict) };
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

    /// Insert a key-value pair whether or not `self` contains `key`.
    ///
    /// Returns references to the (optional) old value and the newly inserted value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    /// use arctic::NonNullString;
    /// use arctic::NonNullStr;
    ///
    /// let mut map = concurrent::Map::<NonNullString, u64>::default();
    /// let key = NonNullStr::new("hello").expect("No null byte");
    ///
    /// let upserted = map.upsert(key, 3);
    /// assert_eq!(upserted.old(), None);
    /// assert_eq!(*upserted.new(), 3);
    ///
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

    /// Insert a key-value pair **if `self` does not contain `key`**. To overwrite
    /// an existing key-value pair instead, see [`Self::upsert`].
    ///
    /// Returns a reference to the newly inserted value (`Ok(new)`) if the insertion
    /// succeeded, or a reference to the old value and the owned new value (`Err((old, new))`).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::concurrent;
    ///
    /// let mut map = concurrent::Map::<u64, u64>::default();
    ///
    /// match map.insert(&64, 3) {
    ///     Err(_) => unreachable!(),
    ///     Ok(new) => assert_eq!(*new, 3),
    /// }
    ///
    /// match map.insert(&64, 5) {
    ///     Err((old, new)) => {
    ///         assert_eq!(*old, 3);
    ///         assert_eq!(new, 5);
    ///     }
    ///     Ok(_) => unreachable!(),
    /// }
    /// ```
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
            Upsert::Break { old, new: initial } => Err((old.expect("Break on `Some`"), initial)),
        }
    }

    pub fn upsert_with<'g, 'k, F>(
        &'g self,
        key: K::Insert<'k>,
        initial: Option<V>,
        mut upsert: F,
    ) -> Upsert<'g, K, V, S>
    where
        F: FnMut(Option<&V::Target>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let initial = if cfg!(feature = "opt-no-path") {
            initial
        } else {
            match self.upsert_with_optimistic(key, initial, &mut upsert) {
                Ok(update) => return update,
                Err(initial) => initial,
            }
        };

        self.upsert_with_pessimistic(key, initial, upsert)
    }

    #[inline]
    fn upsert_with_optimistic<'g, 'k, F>(
        &'g self,
        key: K::Insert<'k>,
        initial: Option<V>,
        upsert: F,
    ) -> Result<Upsert<'g, K, V, S>, Option<V>>
    where
        F: FnMut(Option<&V::Target>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        self.upsert_with_impl::<path::Discard, _>(key, initial, upsert)
    }

    #[cold]
    fn upsert_with_pessimistic<'g, 'k, F>(
        &'g self,
        key: K::Insert<'k>,
        initial: Option<V>,
        upsert: F,
    ) -> Upsert<'g, K, V, S>
    where
        F: FnMut(Option<&V::Target>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        stat::increment(stat::Counter::InsertPessimistic);
        match self.upsert_with_impl::<path::Retain<_>, _>(key, initial, upsert) {
            Ok(upsert) => upsert,
            Err(_) => unreachable!(),
        }
    }

    #[inline]
    fn upsert_with_impl<'g, 'k, P, F>(
        &'g self,
        key: K::Insert<'k>,
        mut initial: Option<V>,
        mut upsert: F,
    ) -> Result<Upsert<'g, K, V, S>, Option<V>>
    where
        P: Path<K::Read<'k>>,
        F: FnMut(Option<&V::Target>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::insert_as_read(key);
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
                            .map(|old| unsafe { V::target_from_raw(old) }),
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
                        Err(Frozen) => initial = Some(unsafe { V::from_raw(new_value) }),

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
                                        node.deallocate_recursive(stat::Counter::FreeConflict);
                                    }
                                }

                                initial = Some(unsafe { V::from_raw(new_value) });
                                continue;
                            }
                        },
                    }
                }
                cursor::Insert::Replace {
                    node: old_node,
                    edge: old,
                } if !old.meta().is_frozen() => {
                    let (smo, new) = unsafe { old_node.replace(old.meta(), true) };
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
                                    node.deallocate(stat::Counter::FreeConflict);
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

    pub fn all(&self) -> iter::Shard<'_, 'static, K, V, RangeFull, Guard<'_, K, V, S>> {
        let guard = self.smr.guard(K::Read::default());
        unsafe { Shard::new(guard, self.seq.raw.all()) }
    }

    pub fn prefix<'g, 'k>(
        &'g self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<iter::Shard<'g, 'k, K, V, RangeFull, Guard<'g, K, V, S>>> {
        let prefix = prefix.into();
        let guard = self.smr.guard(prefix);
        Some(unsafe { Shard::new(guard, self.seq.raw.prefix(prefix)?) })
    }

    pub fn range<'g, 'k, R>(
        &'g self,
        range: R,
    ) -> Option<iter::Shard<'g, 'k, K, V, R, Guard<'g, K, V, S>>>
    where
        R: crate::raw::iter::Range<K::Read<'k>>,
    {
        let prefix = range.common_prefix();
        let guard = self.smr.guard(prefix);
        Some(unsafe { Shard::new(guard, self.seq.raw.range(range, prefix)?) })
    }
}

impl<K, V, S> From<sequential::Map<K, V>> for Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Smr,
{
    #[inline]
    fn from(seq: sequential::Map<K, V>) -> Self {
        Self {
            smr: S::Global::default(),
            seq,
        }
    }
}

impl<K, V, S> From<Map<K, V, S>> for sequential::Map<K, V>
where
    K: Key,
    V: Value,
    S: Smr,
{
    #[inline]
    fn from(map: Map<K, V, S>) -> sequential::Map<K, V> {
        map.seq
    }
}
