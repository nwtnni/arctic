//! An edge is a fat pointer comprising edge compression metadata and child pointer.

mod be;
mod le;
mod slice;

pub(crate) use be::Be;
pub(crate) use le::Le;
use ribbit::u6;
pub(crate) use slice::Slice;

use core::fmt::Debug;
use core::marker::PhantomData;
use core::ops::Add;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::sync::Atomic128;
use crate::sync::Convert;

/// A fat pointer to a value or a node.
///
/// Generic over [`Meta`] to support different byte orderings depending on key type.
#[derive(Copy, Clone, Default)]
#[repr(transparent)]
pub(crate) struct Edge<M> {
    raw: u128,
    meta: PhantomData<M>,
}

impl<M: Meta> Edge<M> {
    pub(crate) const NULL: Self = Self {
        raw: 0,
        meta: PhantomData,
    };

    const MASK_CHILD: u128 = (1 << 64) - 1;
    const SHIFT_META: usize = 64;

    #[inline]
    pub(crate) fn meta(self) -> M {
        unsafe { M::from_raw_unchecked((self.raw >> Self::SHIFT_META) as u64) }
    }

    #[inline]
    pub(crate) fn with_meta(self, meta: M) -> Self {
        unsafe {
            Self::from_raw_unchecked(
                self.raw & Self::MASK_CHILD | ((meta.into_raw() as u128) << Self::SHIFT_META),
            )
        }
    }

    #[inline]
    fn child_raw(self) -> u64 {
        self.raw as u64
    }

    #[inline]
    pub(super) unsafe fn from_raw_ref(raw: &Atomic128<Raw>) -> &Atomic128<Self> {
        unsafe { core::mem::transmute(raw) }
    }

    #[inline]
    pub(super) unsafe fn from_raw_mut(raw: &mut Atomic128<Raw>) -> &mut Atomic128<Self> {
        unsafe { core::mem::transmute(raw) }
    }

    /// Create an edge with the given metadata and node.
    #[inline]
    pub(super) fn new_node(meta: M, node: node::Ptr) -> Self {
        unsafe {
            Self::from_raw_unchecked(
                ((meta.with_value(false).into_raw() as u128) << Self::SHIFT_META)
                    | Some(node).into_raw() as u128,
            )
        }
    }

    /// Create an edge with the given metadata and value.
    #[inline]
    pub(crate) fn new_value(meta: M, value: u64) -> Self {
        unsafe {
            Self::from_raw_unchecked(
                (meta.with_value(true).into_raw() as u128) << Self::SHIFT_META | value as u128,
            )
        }
    }

    /// Given a pointer to an edge, get a pointer to that edge's value.
    ///
    /// # Safety
    ///
    /// - Caller must ensure `edge` points to an edge with a value child
    /// - Caller must ensure `edge` is not modified while holding the returned pointer
    #[inline]
    pub(crate) unsafe fn as_value_unchecked(edge: NonNull<Atomic128<Self>>) -> NonNull<u64> {
        unsafe {
            validate!(edge.as_ref().load(Ordering::Relaxed).meta().is_value());

            if cfg!(target_endian = "little") {
                edge
            } else {
                edge.byte_add(8)
            }
            .cast::<u64>()
        }
    }

    /// Create a new edge mapping `reader` to `value`, recursively
    /// creating intermediate nodes if necessary.
    ///
    /// Returns the head of the path--the root edge--and the tail--either
    /// `None` if the root edge itself contains the value,
    /// or `Some(tail)` where `tail` is the stable heap-allocated
    /// address of the edge containing the value.
    ///
    /// The tail is currently only used by the sequential map, to
    /// return a direct pointer to newly inserted values without
    /// re-traversing the new path. (The concurrent map never
    /// returns direct pointers.)
    #[inline]
    pub(crate) fn new_path<R>(mut reader: R, value: u64) -> (Self, Option<NonNull<Atomic128<Self>>>)
    where
        R: key::Read<Edge = M>,
    {
        let edge = reader.get_edge(<M as edge::Meta>::Len::MAX);

        let Some(byte) = reader.get_byte(edge.len()) else {
            // Fast path: remaining bytes fit in one edge
            return (Self::new_value(edge, value), None);
        };

        reader = reader.suffix(R::Len::BYTE + edge.len().into());

        // Key always fits in one edge
        if R::LEN.is_some_and(|len| len <= <M as edge::Meta>::Len::MAX.into()) {
            validate!(false);
            unsafe { core::hint::unreachable_unchecked() }
        }

        // Key fits in one edge except at root
        if R::LEN.is_some_and(|len| len == R::Len::BYTE + <M as edge::Meta>::Len::MAX.into()) {
            crate::cold();
        }

        // Slow path: allocate recursive path of Node3s
        let (head, tail) = Node3::new_path(edge, byte, reader, value);
        (head, Some(tail))
    }

    /// Freeze `edge` by atomically setting its frozen bit.
    #[inline]
    pub(crate) fn freeze(edge: &Atomic128<Self>) {
        let mut old = edge.load(Ordering::Relaxed);

        while !old.meta().is_frozen() {
            match edge.compare_exchange(
                old,
                old.with_meta(old.meta().with_frozen(true)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
    }

    /// Return `true` if this edge has no child.
    #[inline]
    pub(crate) fn is_null(self) -> bool {
        let null = self.unfreeze().raw == Self::NULL.raw;
        validate!(
            null || self.meta().is_value() || self.child_raw() > 0,
            "Edge must be null, a value, or a node"
        );
        null
    }

    /// Return `Some(node)` if this edge has a node child.
    #[inline]
    pub(crate) fn as_node(self) -> Option<node::Ptr> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { Option::<node::Ptr>::from_raw_unchecked(self.child_raw()) }
    }

    /// Return `Some(child)` if this edge has a child.
    #[inline]
    pub(crate) fn child(self) -> Option<Child> {
        let raw = self.child_raw();
        if self.meta().is_value() {
            Some(Child::Value(raw))
        } else {
            unsafe { Option::<node::Ptr>::from_raw_unchecked(raw) }.map(Child::Node)
        }
    }

    /// Clear the frozen bit from this edge.
    #[inline]
    pub(super) fn unfreeze(self) -> Self {
        self.with_meta(self.meta().with_frozen(false))
    }

    /// Erase this edge's metadata type.
    #[inline]
    pub(super) fn erase(self) -> Raw {
        Raw(self.raw)
    }
}

impl<M: Copy> Convert<u128> for Edge<M> {
    #[inline]
    fn into_raw(self) -> u128 {
        self.raw
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u128) -> Self {
        Self {
            raw,
            meta: PhantomData,
        }
    }
}

impl<M> Debug for Edge<M>
where
    M: Meta + core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());
        debug.field("data", &self.child());

        debug.finish()
    }
}

/// An edge with its metadata type erased.
///
/// Used to reduce code generation, as most node logic is independent of the edge type.
#[derive(Copy, Clone, Debug)]
pub(crate) struct Raw(u128);

impl Raw {
    pub(crate) const NULL: Self = Self(0);
}

impl crate::sync::Convert<u128> for Raw {
    #[inline]
    fn into_raw(self) -> u128 {
        self.0
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u128) -> Self {
        Self(raw)
    }
}

/// Edge compression and child pointer metadata.
pub(crate) trait Meta:
    core::fmt::Debug + Ord + IntoIterator<Item = u8> + Sized + Convert<u64>
{
    /// Null edge with no compressed edge bytes or child
    const NULL: Self;

    /// Representation of compressed edge byte length.
    type Len: Len;

    /// Whether the child pointer is a value.
    fn is_value(self) -> bool;

    /// Whether this edge is frozen.
    fn is_frozen(self) -> bool;

    /// Whether this edge has an implicit terminator byte.
    #[inline]
    fn is_terminate(self) -> bool {
        false
    }

    /// The length of compressed edge bytes.
    fn len(self) -> Self::Len;

    /// Indicate whether this is a value.
    fn with_value(self, value: bool) -> Self;

    /// Indicate whether this edge is frozen.
    fn with_frozen(self, frozen: bool) -> Self;

    /// Try to join two edges into one.
    ///
    /// Returns `None` if edge cannot hold all bytes.
    fn try_compress(self, byte: u8, child: Self) -> Option<Self>;

    /// Try to split one edge into two.
    ///
    /// Returns `None` if `index` is greater or equal to the edge length.
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)>;
}

/// Length of compressed bytes along an edge.
pub(crate) trait Len: Copy + Eq + Ord + Add<Output = Self> + Debug {
    const MAX: Self;
    const BYTE: Self;

    #[cfg_attr(not(test), expect(unused))]
    fn range_to(self) -> impl Iterator<Item = Self>;

    fn bits(self) -> usize;

    #[inline]
    fn bytes(self) -> usize {
        self.bits() >> 3
    }
}

impl Len for u6 {
    const MAX: Self = u6::new(56);
    const BYTE: Self = u6::new(8);

    #[inline]
    fn bits(self) -> usize {
        self.value() as usize
    }

    fn range_to(self) -> impl Iterator<Item = Self> {
        (0..=self.value()).step_by(8).flat_map(Self::try_new)
    }
}

/// Non-null child of an edge.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Child {
    Node(node::Ptr),
    Value(u64),
}

#[cfg(test)]
mod tests {
    /// Correctness properties that hold for sequential executions.
    pub(crate) mod sequential {
        use core::fmt::Debug;

        use crate::raw::Edge;
        use crate::raw::edge::Len;
        use crate::raw::edge::Meta;

        /// An expansion followed by a compression results in the same edge.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn expand_compress_inverse<M>(meta: M)
        where
            M: Meta<Len: Debug>,
        {
            for index in meta.len().range_to() {
                let Some((parent, byte, child)) = meta.try_expand(index) else {
                    assert_eq!(index, meta.len());
                    continue;
                };

                let actual = parent.try_compress(byte, child).unwrap();
                assert!(
                    actual == meta
                    // NOTE: `Eq` implementation ignores flags for scan
                    // purposes, so we check them manually here.
                    && actual.is_frozen() == meta.is_frozen()
                    && actual.is_value() == meta.is_value(),
                    "Expand compress mismatch:\n\
                    {meta:x?}@{index:x?}\n\
                    {parent:x?} - {byte:x?} - {child:x?}\n\
                    {actual:x?}",
                );
            }
        }

        /// An expansion (a) preserves total key bytes, and (b) preserves flags in the child edge.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn expand_correct<M>(meta: M)
        where
            M: Meta<Len: Debug>,
        {
            for index in meta.len().range_to() {
                let Some((parent, byte, child)) = meta.try_expand(index) else {
                    assert_eq!(index, meta.len());
                    continue;
                };

                assert_eq!(
                    meta.len(),
                    parent.len() + <M as Meta>::Len::BYTE + child.len(),
                    "Expand length mismatch:\n\
                    {meta:x?}@{index:x?}\n\
                    {parent:x?} - {byte:x?} - {child:x?}",
                );

                assert!(
                    meta.is_frozen() == child.is_frozen() && meta.is_value() == child.is_value(),
                    "Expand child mismatch:\n\
                    {meta:x?}@{index:x?}\n\
                    {parent:x?} - {byte:x?} - {child:x?}",
                );

                assert!(
                    !parent.is_frozen() && !parent.is_value(),
                    "Expand parent mismatch:\n\
                    {meta:x?}@{index:x?}\n\
                    {parent:x?} - {byte:x?} - {child:x?}",
                );
            }
        }

        /// `M::eq` is reflexive.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        #[expect(clippy::eq_op)]
        pub(crate) fn eq_reflexive<M>(meta: M)
        where
            M: Meta,
        {
            assert_eq!(meta, meta)
        }

        /// `M::cmp` returns equal if and only if `M::eq`.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn eq_ord_consistent<M>(left: M, right: M)
        where
            M: Meta,
        {
            assert_eq!(left.cmp(&right).is_eq(), left == right)
        }

        /// `left < right` if and only if `right > left`.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn ord_duality<M>(left: M, right: M)
        where
            M: Meta,
        {
            assert_eq!(left.cmp(&right), right.cmp(&left).reverse())
        }

        /// `M::cmp` ignores freeze and value flag bits.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn ord_ignores_flags<M>(left: M, right: M)
        where
            M: Meta,
        {
            assert_eq!(
                left.cmp(&right),
                left.with_value(!left.is_value()).cmp(&right)
            );

            assert_eq!(
                left.cmp(&right),
                left.with_frozen(!left.is_frozen()).cmp(&right)
            );

            assert_eq!(
                left.cmp(&right),
                left.with_value(!left.is_value())
                    .with_frozen(!left.is_frozen())
                    .cmp(&right)
            );
        }

        /// `Edge::new_value` creates an edge with the value bit set.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn new_value_is_value<M>(meta: M, value: u64)
        where
            M: Meta,
        {
            assert!(Edge::<M>::new_value(meta, value).meta().is_value())
        }

        /// `M::into_iter` returns an iterator of `M::len` bytes.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn into_iter_len_consistent<M>(meta: M)
        where
            M: Meta,
        {
            let len = meta.len().bytes();
            assert_eq!(meta.into_iter().count(), len);
        }
    }

    macro_rules! impl_suite {
        ($type:ty) => {
            #[cfg(feature = "proptest")]
            mod sequential {
                use crate::raw::edge::tests::sequential;
                use ribbit::Pack as _;

                proptest::proptest! {
                    #![proptest_config(proptest::test_runner::Config::with_cases(100_000))]

                    #[test]
                    fn expand_compress_inverse(meta: $type) {
                        sequential::expand_compress_inverse::<$type>(meta.pack())
                    }

                    #[test]
                    fn expand_correct(meta: $type) {
                        sequential::expand_correct::<$type>(meta.pack())
                    }

                    #[test]
                    fn eq_reflexive(meta: $type) {
                        sequential::eq_reflexive::<$type>(meta.pack())
                    }

                    #[test]
                    fn eq_ord_consistent(left: $type, right: $type) {
                        sequential::eq_ord_consistent::<$type>(left.pack(), right.pack())
                    }

                    #[test]
                    fn ord_duality(left: $type, right: $type) {
                        sequential::ord_duality::<$type>(left.pack(), right.pack())
                    }

                    #[test]
                    fn ord_ignores_flags(left: $type, right: $type) {
                        sequential::ord_ignores_flags::<$type>(left.pack(), right.pack())
                    }

                    #[test]
                    fn new_value_is_value(meta: $type, value: u64) {
                        sequential::new_value_is_value::<$type>(meta.pack(), value)
                    }

                    #[test]
                    fn into_iter_len_consistent(meta: $type) {
                        sequential::into_iter_len_consistent::<$type>(meta.pack())
                    }
                }
            }
        };
    }
    pub(super) use impl_suite;
}
