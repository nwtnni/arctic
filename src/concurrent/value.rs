use core::fmt::Debug;
use core::mem::ManuallyDrop;
use core::ops::Deref;

use crate::concurrent::smr;
use crate::concurrent::smr::Guard as _;
use crate::sequential;

/// Values that can safely be stored in a [`crate::concurrent::Map`].
///
/// Values may be either inline or indirect. An inline
/// value (e.g., u64) is stored directly in an edge and can be freely
/// copied. An indirect value (e.g., Box<T>) is a pointer to a separate
/// allocation; the pointer is stored in an edge.
///
/// Note: we don't need [`Send`] or [`Sync`] bounds here.
/// It's fine to create a concurrent map with non-Sync
/// values; the map instance just won't implement Sync.
pub unsafe trait Value: sequential::Value {
    /// We need this extra layer of indirection relative to [`crate::sequential::Map`]
    /// because edges can be concurrently modified.
    ///
    /// For an inline value, the sequential map can return a reference
    /// to the edge containing the value; the borrow checker ensures
    /// the edge is immutable. This is not true for the concurrent map,
    /// which instead needs to copy out the value and return a reference to
    /// the copy.
    ///
    /// For an indirect value, the concurrent map copies out a pointer
    /// and interprets it as reference.
    type Target;

    /// This is a type-level function that allows inline values to
    /// discard a [`crate::concurrent::smr::Guard`].
    type Guard<G>: smr::Guard<Self> + From<G>
    where
        G: smr::Guard<Self>;

    /// # Safety
    ///
    /// Caller must guarantee the following:
    /// - `raw` was created from [`crate::sequential::Value::into_raw`]
    /// - There are no calls to [`crate::sequential::Value::from_raw`] while `raw` is live
    /// - This value is not mutated while `raw` is live
    unsafe fn target_from_raw(raw: &u64) -> &Self::Target;
}

unsafe impl<T: Sized> Value for Box<T> {
    type Target = T;

    type Guard<G>
        = G
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn target_from_raw(raw: &u64) -> &Self::Target {
        let borrow = unsafe { (*raw as *const T).as_ref() };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }
}

// Note: references are inline values because a
// `&T` itself can be freely copied, even if
// `T` is not `Copy`.
unsafe impl<'v, T: 'v + Sized> Value for &'v T {
    type Target = Self;

    type Guard<G>
        = smr::no_op::Guard<G, Self>
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn target_from_raw(raw: &u64) -> &Self::Target {
        unsafe { core::mem::transmute::<&u64, &Self>(raw) }
    }
}

macro_rules! impl_integer {
    ($($ty:ty),*) => {
        $(
            unsafe impl Value for $ty {
                type Target = Self;

                type Guard<G>
                    = smr::no_op::Guard<G, Self>
                where
                    G: smr::Guard<Self>;

                #[inline]
                unsafe fn target_from_raw(raw: &u64) -> &Self::Target {
                    unsafe { core::mem::transmute::<&u64, &Self>(raw) }
                }
            }
        )*
    };
}

impl_integer!(u64, i64);

pub struct Owned<G: smr::Guard<V>, V: Value> {
    guard: V::Guard<G>,
    raw: u64,
}

impl<G, V> Owned<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    pub(crate) unsafe fn wrap(guard: G, raw: u64) -> Self {
        Self {
            guard: V::Guard::<G>::from(guard),
            raw,
        }
    }
}

impl<G, V> Deref for Owned<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    type Target = V::Target;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { V::target_from_raw(&self.raw) }
    }
}

impl<G: smr::Guard<V>, V: Value> Drop for Owned<G, V> {
    fn drop(&mut self) {
        unsafe { self.guard.retire_value(self.raw) }
    }
}

impl<G, V> Debug for Owned<G, V>
where
    G: smr::Guard<V>,
    V: Value,
    V::Target: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

pub struct Shared<G: smr::Guard<V>, V: Value> {
    _guard: V::Guard<G>,
    raw: u64,
}

impl<G, V> Shared<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    pub(crate) unsafe fn wrap(guard: G, raw: u64) -> Self {
        Self {
            _guard: V::Guard::<G>::from(guard),
            raw,
        }
    }
}

impl<G, V> Deref for Shared<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    type Target = V::Target;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { V::target_from_raw(&self.raw) }
    }
}

impl<G, V> Debug for Shared<G, V>
where
    G: smr::Guard<V>,
    V: Value,
    V::Target: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

pub struct Updated<G: smr::Guard<V>, V: Value> {
    guard: V::Guard<G>,
    old: u64,
    new: u64,
}

impl<G, V> Updated<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    pub(crate) unsafe fn wrap(guard: G, old: u64, new: u64) -> Self {
        Self {
            guard: V::Guard::<G>::from(guard),
            old,
            new,
        }
    }

    #[inline]
    pub fn old(&self) -> &V::Target {
        unsafe { V::target_from_raw(&self.old) }
    }

    #[inline]
    pub fn new(&self) -> &V::Target {
        unsafe { V::target_from_raw(&self.new) }
    }
}

impl<G: smr::Guard<V>, V: Value> Drop for Updated<G, V> {
    fn drop(&mut self) {
        unsafe { self.guard.retire_value(self.old) }
    }
}

impl<G, V> Debug for Updated<G, V>
where
    G: smr::Guard<V>,
    V: Value,
    V::Target: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updated")
            .field("old", self.old())
            .field("new", self.new())
            .finish()
    }
}

pub struct Upserted<G: smr::Guard<V>, V: Value> {
    guard: V::Guard<G>,
    old: Option<u64>,
    new: u64,
}

impl<G, V> Upserted<G, V>
where
    G: smr::Guard<V>,
    V: Value,
{
    pub(crate) unsafe fn wrap(guard: G, old: Option<u64>, new: u64) -> Self {
        Self {
            guard: V::Guard::<G>::from(guard),
            old,
            new,
        }
    }

    pub(crate) fn into_inserted(self) -> Result<Shared<G, V>, Self> {
        // https://internals.rust-lang.org/t/move-out-of-deref-for-manuallydrop/19216
        let upserted = ManuallyDrop::new(self);

        match upserted.old {
            None => Ok(Shared {
                // HACK: work around not being able to move out of deref
                _guard: unsafe { core::ptr::read(&upserted.guard) },
                raw: upserted.new,
            }),
            Some(_) => Err(ManuallyDrop::into_inner(upserted)),
        }
    }

    #[inline]
    pub fn old(&self) -> Option<&V::Target> {
        self.old
            .as_ref()
            .map(|old| unsafe { V::target_from_raw(old) })
    }

    #[inline]
    pub fn new(&self) -> &V::Target {
        unsafe { V::target_from_raw(&self.new) }
    }
}

impl<G: smr::Guard<V>, V: Value> Drop for Upserted<G, V> {
    fn drop(&mut self) {
        let Some(old) = self.old else { return };
        unsafe { self.guard.retire_value(old) }
    }
}

impl<G, V> Debug for Upserted<G, V>
where
    G: smr::Guard<V>,
    V: Value,
    V::Target: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upserted")
            .field("old", &self.old())
            .field("new", self.new())
            .finish()
    }
}
