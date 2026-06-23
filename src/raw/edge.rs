//! An edge is a fat pointer comprising edge compression metadata and child pointer.

mod be;
mod le;
mod slice;

pub(crate) use be::Be;
pub(crate) use le::Le;
use ribbit::u6;
pub(crate) use slice::Slice;

use core::fmt::Debug;
use core::ops::Add;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Unpack as _;

use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::sync::Atomic;

/// A fat pointer to a value or a node.
///
/// Generic over [`Meta`] to support different byte orderings depending on key type.
#[derive(Copy, Clone, Default, ribbit::Pack)]
#[ribbit(size = 128, derive(Eq))]
pub(crate) struct Edge<M> {
    #[ribbit(size = 64)]
    pub(crate) meta: M,

    #[ribbit(get(rename = "child_raw"))]
    child: u64,
}

/// An edge with its metadata type erased.
///
/// Used to reduce code generation, as most node logic is independent of the edge type.
#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, derive(Debug))]
pub(crate) struct Raw(u128);

impl Raw {
    pub(crate) const NULL: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(0);
}

impl<M: ribbit::Pack<Packed: Meta>> Edge<M> {
    pub(crate) const NULL: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(<M::Packed as Meta>::NULL, 0);

    #[inline]
    pub(super) unsafe fn from_raw_ref(raw: &Atomic<Raw>) -> &Atomic<Self> {
        unsafe { core::mem::transmute(raw) }
    }

    #[inline]
    pub(super) unsafe fn from_raw_mut(raw: &mut Atomic<Raw>) -> &mut Atomic<Self> {
        unsafe { core::mem::transmute(raw) }
    }

    /// Create an edge with the given metadata and node.
    #[inline]
    pub(super) fn new_node(
        meta: ribbit::Packed<M>,
        node: ribbit::Packed<node::Ptr>,
    ) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(meta.with_value(false), node.into_raw().get())
    }

    /// Create an edge with the given metadata and value.
    #[inline]
    pub(crate) fn new_value(meta: ribbit::Packed<M>, value: u64) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(meta.with_value(true), value)
    }

    /// Given a pointer to an edge, get a pointer to that edge's value.
    ///
    /// # Safety
    ///
    /// - Caller must ensure `edge` points to an edge with a value child
    /// - Caller must ensure `edge` is not modified while holding the returned pointer
    #[inline]
    pub(crate) unsafe fn as_value_unchecked(edge: NonNull<Atomic<Self>>) -> NonNull<u64> {
        unsafe {
            validate!(
                edge.as_ref()
                    .load_packed(Ordering::Relaxed)
                    .meta()
                    .is_value()
            );

            if cfg!(target_endian = "little") {
                edge.byte_add(8)
            } else {
                edge
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
    #[expect(clippy::type_complexity)]
    pub(crate) fn new_path<R>(
        mut reader: R,
        value: u64,
    ) -> (ribbit::Packed<Self>, Option<NonNull<Atomic<Edge<M>>>>)
    where
        R: key::Read<Edge = M>,
    {
        let edge = reader.get_edge(<ribbit::Packed<M> as edge::Meta>::Len::MAX);

        let Some(byte) = reader.get_byte(edge.len()) else {
            // Fast path: remaining bytes fit in one edge
            return (Self::new_value(edge, value), None);
        };

        reader = reader.suffix(R::Len::BYTE + edge.len().into());

        // Key always fits in one edge
        if R::LEN.is_some_and(|len| len <= <ribbit::Packed<M> as edge::Meta>::Len::MAX.into()) {
            validate!(false);
            unsafe { core::hint::unreachable_unchecked() }
        }

        // Key fits in one edge except at root
        if R::LEN.is_some_and(|len| {
            len == R::Len::BYTE + <ribbit::Packed<M> as edge::Meta>::Len::MAX.into()
        }) {
            crate::cold();
        }

        // Slow path: allocate recursive path of Node3s
        let (head, tail) = Node3::new_path(edge, byte, reader, value);
        (head, Some(tail))
    }

    /// Freeze `edge` by atomically setting its frozen bit.
    #[inline]
    pub(crate) fn freeze(edge: &Atomic<Self>) {
        let mut old = edge.load_packed(Ordering::Relaxed);

        while !old.meta().is_frozen() {
            match edge.compare_exchange_packed(
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
}

impl<M: ribbit::Pack<Packed: Meta>> EdgePacked<M> {
    /// Return `true` if this edge has no child.
    #[inline]
    pub(crate) fn is_null(self) -> bool {
        let null = self.unfreeze() == Edge::NULL;
        validate!(
            null || self.meta().is_value() || self.child_raw() > 0,
            "Edge must be null, a value, or a node"
        );
        null
    }

    /// Return `Some(node)` if this edge has a node child.
    #[inline]
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<node::Ptr>> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { ribbit::Packed::<Option<node::Ptr>>::from_raw_unchecked(self.child_raw()) }
    }

    /// Return `Some(child)` if this edge has a child.
    #[inline]
    pub(crate) fn child(self) -> Option<Child> {
        let raw = self.child_raw();
        if self.meta().is_value() {
            Some(Child::Value(raw))
        } else {
            unsafe { ribbit::Packed::<Option<node::Ptr>>::from_raw_unchecked(raw) }.map(Child::Node)
        }
    }

    /// Clear the frozen bit from this edge.
    #[inline]
    pub(super) fn unfreeze(self) -> Self {
        self.with_meta(self.meta().with_frozen(false))
    }

    /// Erase this edge's metadata type.
    #[inline]
    pub(super) fn erase(self) -> ribbit::Packed<edge::Raw> {
        ribbit::Packed::<edge::Raw>::new(self.into_raw())
    }
}

impl<M: ribbit::Pack> Debug for EdgePacked<M>
where
    M::Packed: Meta + core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());
        debug.field("data", &self.child());

        debug.finish()
    }
}

/// Edge compression and child pointer metadata.
pub(crate) trait Meta:
    ribbit::Unpack + core::fmt::Debug + Ord + IntoIterator<Item = u8>
{
    /// Null edge with no compressed edge bytes or child
    const NULL: Self;

    /// Representation of compressed edge byte length.
    type Len: Len;

    /// Whether the child pointer is a value.
    fn is_value(self) -> bool;

    /// Whether this edge is frozen.
    fn is_frozen(self) -> bool;

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
///
/// Currently only implemented by `u6`, but hoping
/// to support longer edges for borrowed keys eventually.
pub(crate) trait Len: Copy + Eq + Add<Output = Self> {
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
    Node(ribbit::Packed<node::Ptr>),
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
        pub(crate) fn expand_compress_inverse<M>(meta: ribbit::Packed<M>)
        where
            M: ribbit::Pack<Packed: Meta<Len: Debug>>,
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
        pub(crate) fn expand_correct<M>(meta: ribbit::Packed<M>)
        where
            M: ribbit::Pack<Packed: Meta<Len: Debug>>,
        {
            for index in meta.len().range_to() {
                let Some((parent, byte, child)) = meta.try_expand(index) else {
                    assert_eq!(index, meta.len());
                    continue;
                };

                assert_eq!(
                    meta.len(),
                    parent.len() + <ribbit::Packed::<M> as Meta>::Len::BYTE + child.len(),
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
        pub(crate) fn eq_reflexive<M>(meta: ribbit::Packed<M>)
        where
            M: ribbit::Pack<Packed: Meta>,
        {
            assert_eq!(meta, meta)
        }

        /// `M::cmp` returns equal if and only if `M::eq`.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn eq_ord_consistent<M>(left: ribbit::Packed<M>, right: ribbit::Packed<M>)
        where
            M: ribbit::Pack<Packed: Meta>,
        {
            assert_eq!(left.cmp(&right).is_eq(), left == right)
        }

        /// `left < right` if and only if `right > left`.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn ord_duality<M>(left: ribbit::Packed<M>, right: ribbit::Packed<M>)
        where
            M: ribbit::Pack<Packed: Meta>,
        {
            assert_eq!(left.cmp(&right), right.cmp(&left).reverse())
        }

        /// `M::cmp` ignores freeze and value flag bits.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn ord_ignores_flags<M>(left: ribbit::Packed<M>, right: ribbit::Packed<M>)
        where
            M: ribbit::Pack<Packed: Meta>,
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
        pub(crate) fn new_value_is_value<M>(meta: ribbit::Packed<M>, value: u64)
        where
            M: ribbit::Pack<Packed: Meta>,
        {
            assert!(Edge::<M>::new_value(meta, value).meta().is_value())
        }

        /// `M::into_iter` returns an iterator of `M::len` bytes.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn into_iter_len_consistent<M>(meta: ribbit::Packed<M>)
        where
            M: ribbit::Pack<Packed: Meta>,
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
