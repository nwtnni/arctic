mod iter;
mod key;
pub mod smr;
mod value;

use core::ops::ControlFlow;
use core::ops::RangeFull;
use core::sync::atomic::Ordering;

use smr::Global as _;
use smr::Guard as _;

use crate::raw;
use crate::raw::Cursor;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::cursor;
use crate::raw::cursor::path;
use crate::raw::edge;
use crate::raw::edge::Key as _;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::sequential;
use crate::stat;

pub use iter::EntryIter;
pub use iter::Prefix;
pub use iter::ValueIter;
pub use key::Key;
pub use smr::Smr;
pub use value::Value;

pub type Guard<'g, K, V, S> =
    <<S as Smr>::Global<<K as Key>::Prefix, V> as smr::Global<<K as Key>::Prefix, V>>::Guard<'g>;
pub type Owned<'g, K, V, S> = value::Owned<Guard<'g, K, V, S>, V>;
pub type Shared<'g, K, V, S> = value::Shared<Guard<'g, K, V, S>, V>;
pub type Updated<'g, K, V, S> = value::Updated<Guard<'g, K, V, S>, V>;
pub type Upserted<'g, K, V, S> = value::Upserted<Guard<'g, K, V, S>, V>;

pub struct Map<K: Key, V: Value, S: Smr = smr::Hazard> {
    smr: S::Global<K::Prefix, V>,
    inner: sequential::Map<K, V>,
}

unsafe impl<K: Key, V: Value + Send + Sync, S: Smr> Sync for Map<K, V, S> {}

impl<K: crate::Key, V: Value, S: Smr> Default for Map<K, V, S> {
    fn default() -> Self {
        Self {
            smr: S::Global::default(),
            inner: sequential::Map::<K, V>::default(),
        }
    }
}

impl<K: Key, V: Value, S: Smr> Map<K, V, S> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_smr(smr: S::Global<K::Prefix, V>) -> Self {
        Self {
            smr,
            inner: sequential::Map::<K, V>::default(),
        }
    }

    #[inline]
    pub fn as_sequential(&mut self) -> &mut sequential::Map<K, V> {
        &mut self.inner
    }

    #[inline]
    pub fn smr(&self) -> &S::Global<K::Prefix, V> {
        &self.smr
    }

    #[inline]
    pub fn smr_mut(&mut self) -> &mut S::Global<K::Prefix, V> {
        &mut self.smr
    }
}

pub enum Update<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K::Prefix, V>: 'g,
{
    Absent {
        initial: Option<V>,
    },
    Success(Updated<'g, K, V, S>),
    Break {
        old: Shared<'g, K, V, S>,
        initial: Option<V>,
    },
}

pub enum Remove<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K::Prefix, V>: 'g,
{
    Absent,
    Success { old: Owned<'g, K, V, S> },
    Break { old: Shared<'g, K, V, S> },
}

pub enum Upsert<'g, K, V, S>
where
    K: Key,
    V: Value + 'g,
    S: Smr,
    S::Global<K::Prefix, V>: 'g,
{
    Success(Upserted<'g, K, V, S>),
    Break {
        old: Option<Shared<'g, K, V, S>>,
        initial: Option<V>,
    },
}

impl<K, V, S> Map<K, V, S>
where
    K: Key,
    V: Value + Send + Sync,
    S: Smr,
{
    #[inline]
    pub fn get(&self, key: K::Borrow<'_>) -> Option<Shared<K, V, S>> {
        let reader = K::Read::from(key);
        let guard = self.smr.guard(K::hazard(reader));
        let value =
            unsafe { Cursor::<K, path::Discard>::new(self.inner.root(), reader).traverse_get()? };
        Some(unsafe { Shared::<'_, K, V, S>::wrap(guard, value) })
    }

    #[inline]
    pub fn update(&self, key: K::Borrow<'_>, value: V) -> Result<Updated<K, V, S>, V> {
        match self.update_with(key, Some(value), |_, initial| {
            ControlFlow::<(), _>::Continue(initial.take().expect("Value is always initialized"))
        }) {
            Update::Absent {
                initial: Some(initial),
            } => Err(initial),
            Update::Success(updated) => Ok(updated),
            Update::Absent { initial: None } | Update::Break { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn update_with<F>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        mut update: F,
    ) -> Update<K, V, S>
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
    fn update_with_optimistic<F>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Result<Update<K, V, S>, Option<V>>
    where
        F: FnMut(&V::Target, &mut Option<V>) -> ControlFlow<(), V>,
    {
        self.update_with_impl::<path::Discard, _>(key, initial, update)
    }

    #[cold]
    fn update_with_pessimistic<F>(
        &self,
        key: K::Borrow<'_>,
        initial: Option<V>,
        update: F,
    ) -> Update<K, V, S>
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
    fn update_with_impl<'k, H, F>(
        &self,
        key: K::Borrow<'k>,
        mut initial: Option<V>,
        mut update: F,
    ) -> Result<Update<K, V, S>, Option<V>>
    where
        H: path::History<'k, K>,
        F: FnMut(&V::Target, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.inner.root(), reader) };

        loop {
            let old = match cursor.traverse_update() {
                None => return Ok(Update::Absent { initial }),
                Some(Ok(old)) => old,
                Some(Err(Frozen)) => match cursor.freeze() {
                    Err(_) => return Err(initial),
                    Ok(None) => continue,
                    Ok(Some(node)) => unsafe {
                        guard.retire_node(cursor.bits(), node);
                        continue;
                    },
                },
            };

            validate!(old.meta().is_value());

            let old_value = unsafe { old.into_value_unchecked() };
            let new_value = match update(unsafe { V::target_from_raw(&old_value) }, &mut initial) {
                ControlFlow::Continue(new) => V::into_raw(new),
                ControlFlow::Break(()) => {
                    return Ok(Update::Break {
                        old: unsafe { Shared::<K, V, S>::wrap(guard, old.into_value_unchecked()) },
                        initial,
                    });
                }
            };

            match cursor.edge().compare_exchange_packed(
                old,
                unsafe { old.with_value_unchecked(new_value) },
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Ok(Update::Success(unsafe {
                        Updated::<K, V, S>::wrap(guard, old_value, new_value)
                    }));
                }
                Err(_) => {
                    initial = Some(unsafe { V::from_raw(new_value) });
                }
            }
        }
    }

    #[inline]
    pub fn remove_non_recursive(&self, key: K::Borrow<'_>) -> Option<Owned<'_, K, V, S>> {
        match self.remove_non_recursive_with(key, |_| ControlFlow::Continue(())) {
            Remove::Absent => None,
            Remove::Success { old } => Some(old),
            Remove::Break { old: _ } => unreachable!(),
        }
    }

    #[inline]
    pub fn remove_non_recursive_with<F>(
        &self,
        key: K::Borrow<'_>,
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
        key: K::Borrow<'_>,
        with: &mut F,
    ) -> Result<Remove<'_, K, V, S>, ()>
    where
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        self.remove_with_impl::<false, path::Discard, _>(key, with)
    }

    #[cold]
    fn remove_non_recursive_with_pessimistic<'k, F>(
        &self,
        key: K::Borrow<'k>,
        with: &mut F,
    ) -> Remove<'_, K, V, S>
    where
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        let Ok(remove) = self.remove_with_impl::<false, path::Retain<'k, K>, _>(key, with);
        remove
    }

    #[inline]
    pub fn remove(&self, key: K::Borrow<'_>) -> Option<Owned<K, V, S>> {
        match self.remove_with(key, |_| ControlFlow::Continue(())) {
            Remove::Absent => None,
            Remove::Success { old } => Some(old),
            Remove::Break { old: _ } => unreachable!(),
        }
    }

    #[inline]
    pub fn remove_with<'k, F>(&self, key: K::Borrow<'k>, mut with: F) -> Remove<K, V, S>
    where
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        let Ok(remove) = self.remove_with_impl::<true, path::Retain<'k, K>, _>(key, &mut with);
        remove
    }

    #[inline]
    fn remove_with_impl<'k, const RECURSIVE: bool, H, F>(
        &self,
        key: K::Borrow<'k>,
        remove: &mut F,
    ) -> Result<Remove<K, V, S>, H::PopError>
    where
        H: path::History<'k, K>,
        F: FnMut(&V::Target) -> ControlFlow<(), ()>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.inner.root(), reader) };

        let old = loop {
            let old = match cursor.traverse_update() {
                None => return Ok(Remove::Absent),
                Some(Ok(old)) => old,
                Some(Err(Frozen)) => match cursor.freeze()? {
                    None => continue,
                    Some(node) => unsafe {
                        guard.retire_node(cursor.bits(), node);
                        continue;
                    },
                },
            };

            validate!(old.meta().is_value());

            let old_value = unsafe { old.into_value_unchecked() };

            match remove(unsafe { V::target_from_raw(&old_value) }) {
                ControlFlow::Continue(()) => (),
                ControlFlow::Break(()) => {
                    return Ok(Remove::Break {
                        old: unsafe { Shared::<K, V, S>::wrap(guard, old_value) },
                    });
                }
            }

            if cursor
                .edge()
                .compare_exchange_packed(old, Edge::DEFAULT, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break old;
            }
        };

        if RECURSIVE {
            let mut trim = old.meta().key().len();

            'outer: while let Some(target) = cursor
                .pop()
                .unwrap_or_else(|_| panic!("Recursive remove requires path"))
            {
                if unsafe { target.len() } > 0 {
                    break 'outer;
                }

                cursor.trim(trim.bits() + 8);

                loop {
                    let Some(old) = cursor.traverse_prefix() else {
                        break 'outer;
                    };

                    let new = match old.child() {
                        None => break 'outer,
                        Some(edge::Child::Value(_)) => unreachable!(),
                        Some(edge::Child::Node(node)) if node == target => {
                            unsafe { node.replace(old.meta()) }.1
                        }
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
                            unsafe { guard.retire_node(cursor.bits(), target) };
                            trim = old.meta().key().len();
                            continue 'outer;
                        }
                        // FIXME: help freeze
                        Err(conflict) if conflict.meta().is_frozen() => todo!(),
                        Err(_) => {
                            if let Some(node) = new.as_node() {
                                unsafe { node.deallocate(stat::Counter::FreeConflict) };
                            }
                        }
                    }
                }
            }
        }

        Ok(Remove::Success {
            old: unsafe { Owned::<K, V, S>::wrap(guard, old.into_value_unchecked()) },
        })
    }

    #[inline]
    pub fn upsert(&self, key: K::BorrowInsert<'_>, value: V) -> Upserted<'_, K, V, S> {
        match self.upsert_with(key, Some(value), |_, new| {
            ControlFlow::<(), _>::Continue(new.take().expect("Value is always initialized"))
        }) {
            Upsert::Success(upserted) => upserted,
            Upsert::Break { .. } => unreachable!(),
        }
    }

    #[inline]
    pub fn insert(
        &self,
        key: K::BorrowInsert<'_>,
        value: V,
    ) -> Result<Shared<K, V, S>, (Shared<K, V, S>, V)> {
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

    #[inline]
    pub fn insert_with<F>(
        &self,
        key: K::BorrowInsert<'_>,
        insert: F,
    ) -> Result<Shared<K, V, S>, (Shared<K, V, S>, Option<V>)>
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
                .into_inserted()
                .unwrap_or_else(|_| unreachable!("Continue on `None`"))),
            Upsert::Break { old, initial } => Err((old.expect("Break on `Some`"), initial)),
        }
    }

    #[inline]
    pub fn upsert_with<F>(
        &self,
        key: K::BorrowInsert<'_>,
        initial: Option<V>,
        mut upsert: F,
    ) -> Upsert<K, V, S>
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
    fn upsert_with_optimistic<F>(
        &self,
        key: K::BorrowInsert<'_>,
        initial: Option<V>,
        upsert: F,
    ) -> Result<Upsert<K, V, S>, Option<V>>
    where
        F: FnMut(Option<&V::Target>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        self.upsert_with_impl::<path::Discard, _>(key, initial, upsert)
    }

    #[cold]
    fn upsert_with_pessimistic<F>(
        &self,
        key: K::BorrowInsert<'_>,
        initial: Option<V>,
        upsert: F,
    ) -> Upsert<K, V, S>
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
    fn upsert_with_impl<'k, H, F>(
        &self,
        key: K::BorrowInsert<'k>,
        mut initial: Option<V>,
        mut upsert: F,
    ) -> Result<Upsert<K, V, S>, Option<V>>
    where
        H: path::History<'k, K>,
        F: FnMut(Option<&V::Target>, &mut Option<V>) -> ControlFlow<(), V>,
    {
        let reader = K::Read::from(key);
        let mut guard = self.smr.guard(K::hazard(reader));
        let mut cursor = unsafe { Cursor::<_, H>::new(self.inner.root(), reader) };

        loop {
            match cursor.traverse_insert() {
                cursor::Insert::Value {
                    old_value,
                    old,
                    key,
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
                                old: old
                                    .as_value()
                                    .map(|old| unsafe { Shared::<K, V, S>::wrap(guard, old) }),
                                initial,
                            });
                        }
                    };

                    match cursor.insert(old, key, new_value) {
                        // Restore value and fall through to freeze
                        Err(Frozen) => initial = Some(unsafe { V::from_raw(new_value) }),

                        Ok(new) => match cursor.edge().compare_exchange_packed(
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
                cursor::Insert::Smo { old_node, old } if !old.meta().is_frozen() => {
                    let (smo, new) = unsafe { old_node.replace(old.meta()) };
                    match cursor.edge().compare_exchange_packed(
                        old,
                        new,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => {
                            if let Some(node) = old.as_node() {
                                unsafe { guard.retire_node(cursor.bits(), node) };
                            }
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
                cursor::Insert::Smo { .. } => (),
            }

            match cursor.freeze() {
                Err(_) => return Err(initial),
                Ok(None) => (),
                Ok(Some(node)) => unsafe { guard.retire_node(cursor.bits(), node) },
            }
        }
    }

    pub fn all(&self) -> iter::Prefix<'static, '_, K, V, RangeFull, Guard<'_, K, V, S>> {
        let guard = self.smr.guard(K::hazard(K::Read::default()));
        let prefix = unsafe { raw::iter::Prefix::<K>::new_all(self.inner.root()) };
        unsafe { Prefix::new(guard, prefix) }
    }

    pub fn prefix<'k>(
        &self,
        prefix: impl Into<K::Read<'k>>,
    ) -> Option<iter::Prefix<'k, '_, K, V, RangeFull, Guard<'_, K, V, S>>> {
        let prefix = prefix.into();
        let guard = self.smr.guard(K::hazard(prefix));
        let prefix = unsafe { raw::iter::Prefix::<K>::new_prefix(self.inner.root(), prefix) }?;
        Some(unsafe { Prefix::new(guard, prefix) })
    }

    pub fn range<'k, R>(
        &self,
        range: R,
    ) -> Option<iter::Prefix<'k, '_, K, V, R, Guard<'_, K, V, S>>>
    where
        R: crate::raw::iter::Range<'k, K>,
    {
        // FIXME: avoid recomputing common prefix?
        let guard = self.smr.guard(K::hazard(range.common_prefix()));
        let prefix = unsafe { raw::iter::Prefix::new_range(self.inner.root(), range) }?;
        Some(unsafe { Prefix::new(guard, prefix) })
    }
}

impl<K, V, S> From<sequential::Map<K, V>> for Map<K, V, S>
where
    K: Key,
    V: Value,
    S: Smr,
{
    fn from(inner: sequential::Map<K, V>) -> Self {
        Self {
            smr: S::Global::default(),
            inner,
        }
    }
}

impl<K, V, S> From<Map<K, V, S>> for sequential::Map<K, V>
where
    K: Key,
    V: Value,
    S: Smr,
{
    fn from(map: Map<K, V, S>) -> sequential::Map<K, V> {
        map.inner
    }
}
