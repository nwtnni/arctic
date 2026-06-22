//! Values that can safely be stored in a [`concurrent::Map`][crate::concurrent::Map],
//! and referenced behind an [`smr::Guard`].

use core::borrow::Borrow;
use core::fmt::Debug;
use core::mem::ManuallyDrop;
use core::ops::Deref;

use crate::concurrent::smr;
use crate::concurrent::smr::Guard as _;
use crate::sequential;
pub use crate::sequential::value::Arc;

/// Values that can safely be stored in a [`concurrent::Map`][crate::concurrent::Map].
///
/// Values may be either inline or indirect. An inline
/// value (e.g., [`u64`]) is stored directly in an edge and can be freely
/// copied. An indirect value (e.g., [`Box<T>`]) is a pointer to a separate
/// allocation; the pointer is stored in an edge.
///
/// Note: we don't need [`Send`] or [`Sync`] bounds here.
/// It's fine to create a concurrent map with non-Sync
/// values; the map instance just won't implement Sync.
pub trait Value: sequential::Value + Borrow<Self::Borrowed> {
    /// We need this extra layer of indirection relative to [`sequential::Map`]
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
    type Borrowed;

    /// This is a type-level function that allows inline values to
    /// discard a [`smr::Guard`].
    type Guard<G>: smr::Guard<Self> + From<G>
    where
        G: smr::Guard<Self>;

    /// # Safety
    ///
    /// Caller must guarantee the following:
    /// - `raw` was created from [sequential::Value::into_raw`]
    /// - There are no calls to [`sequential::Value::from_raw_unchecked`] while `raw` is live
    /// - This value is not mutated while `raw` is live
    unsafe fn borrow_from_raw_unchecked(raw: &u64) -> &Self::Borrowed;
}

macro_rules! impl_integer {
    ($($ty:ty),*) => {
        $(
            impl Value for $ty {
                type Borrowed = Self;

                type Guard<G>
                    = smr::no_op::Guard<G, Self>
                where
                    G: smr::Guard<Self>;

                #[inline]
                unsafe fn borrow_from_raw_unchecked(raw: &u64) -> &Self::Borrowed {
                    unsafe { core::mem::transmute::<&u64, &Self>(raw) }
                }
            }
        )*
    };
}

impl_integer!(u64, i64);

// Note: references are inline values because a
// `&T` itself can be freely copied, even if
// `T` is not `Copy`.
impl<'v, T: 'v + Sized> Value for &'v T {
    type Borrowed = Self;

    type Guard<G>
        = smr::no_op::Guard<G, Self>
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn borrow_from_raw_unchecked(raw: &u64) -> &Self::Borrowed {
        unsafe { core::mem::transmute::<&u64, &Self>(raw) }
    }
}

impl<T: Sized> Value for Box<T> {
    type Borrowed = T;

    type Guard<G>
        = G
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn borrow_from_raw_unchecked(raw: &u64) -> &Self::Borrowed {
        let borrow = unsafe { core::ptr::with_exposed_provenance::<T>((*raw) as usize).as_ref() };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }
}

impl<T: Sized> Value for Arc<T> {
    type Borrowed = ArcRef<T>;

    type Guard<G>
        = G
    where
        G: smr::Guard<Self>;

    #[inline]
    unsafe fn borrow_from_raw_unchecked(raw: &u64) -> &Self::Borrowed {
        let borrow = unsafe {
            core::ptr::with_exposed_provenance::<T>((*raw) as usize)
                .cast::<ArcRef<T>>()
                .as_ref()
        };
        if_validate!(borrow.unwrap(), unsafe { borrow.unwrap_unchecked() })
    }
}

impl<T> Borrow<ArcRef<T>> for crate::sequential::value::Arc<T> {
    fn borrow(&self) -> &ArcRef<T> {
        unsafe { core::mem::transmute::<&T, &ArcRef<T>>(self.0.as_ref()) }
    }
}

/// Transparent wrapper for [`Arc<T>`] pointee that can
/// be safely cloned into an [`Arc<T>`] via [`ToOwned`].
#[repr(transparent)]
#[derive(Debug)]
pub struct ArcRef<T>(T);

impl<T> Deref for ArcRef<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> ToOwned for ArcRef<T> {
    type Owned = Arc<T>;
    /// Clone into an owned `Arc` by incrementing the strong reference count.
    fn to_owned(&self) -> Self::Owned {
        // SAFETY: `ArcRef` is `repr(transparent)`
        let ptr = unsafe { core::mem::transmute::<&Self, &T>(self) };

        // SAFETY: SMR guarantees `ptr` is not yet freed,
        // so strong count must be >= 1
        unsafe { crate::sync::Arc::increment_strong_count(ptr) };

        // SAFETY: `ptr` was returned from `Arc::into_raw`
        Arc(unsafe { crate::sync::Arc::from_raw(ptr) })
    }
}

/// Guard that provides read-only access to a removed value while
/// preventing the value from being freed. Retires the value on drop.
///
/// Note: this value may still be concurrently accessed by other
/// threads, so this guard cannot safely provide mutable access.
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
    type Target = V::Borrowed;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { V::borrow_from_raw_unchecked(&self.raw) }
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
    V::Borrowed: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

/// Guard that provides read-only access to a value while
/// preventing the value from being freed.
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
    type Target = V::Borrowed;

    #[inline]
    fn deref(&self) -> &Self::Target {
        unsafe { V::borrow_from_raw_unchecked(&self.raw) }
    }
}

impl<G, V> Debug for Shared<G, V>
where
    G: smr::Guard<V>,
    V: Value,
    V::Borrowed: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.deref().fmt(f)
    }
}

/// Guard that provides read-only access to both the old
/// and new values of an atomic update operation,
/// preventing both from being freed.
///
/// Retires the old value on drop.
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

    /// Return the old value before updating.
    #[inline]
    pub fn old(&self) -> &V::Borrowed {
        unsafe { V::borrow_from_raw_unchecked(&self.old) }
    }

    /// Return the new value after updating.
    #[inline]
    #[expect(clippy::new_ret_no_self)]
    pub fn new(&self) -> &V::Borrowed {
        unsafe { V::borrow_from_raw_unchecked(&self.new) }
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
    V::Borrowed: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Updated")
            .field("old", self.old())
            .field("new", self.new())
            .finish()
    }
}

/// Guard that provides read-only access to both the old
/// and new values of an atomic upsert operation,
/// preventing both from being freed.
///
/// Retires the old value on drop, if it existed.
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

    pub(crate) fn try_into_inserted(self) -> Result<Shared<G, V>, Self> {
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

    /// Return the old value before upserting.
    #[inline]
    pub fn old(&self) -> Option<&V::Borrowed> {
        self.old
            .as_ref()
            .map(|old| unsafe { V::borrow_from_raw_unchecked(old) })
    }

    /// Return the new value after upserting.
    #[inline]
    #[expect(clippy::new_ret_no_self)]
    pub fn new(&self) -> &V::Borrowed {
        unsafe { V::borrow_from_raw_unchecked(&self.new) }
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
    V::Borrowed: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Upserted")
            .field("old", &self.old())
            .field("new", self.new())
            .finish()
    }
}
