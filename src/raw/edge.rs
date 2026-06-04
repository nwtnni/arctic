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

use ribbit::Atomic;
use ribbit::OptionExt as _;

use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node3;

/// A fat pointer to a value or a node.
///
/// Generic over [`Meta`] to support different byte orderings depending on key type.
#[derive(Copy, Clone, Default, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked), eq)]
pub(crate) struct Edge<M> {
    #[ribbit(size = 64)]
    pub(crate) meta: M,

    #[ribbit(get(rename = "child_raw"))]
    child: u64,
}

impl<M: ribbit::Pack<Packed: Meta>> Edge<M> {
    pub(crate) const NULL: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(<M::Packed as Meta>::NULL, 0);

    /// Create an edge with the given metadata and node.
    #[inline]
    pub(super) fn new_node(
        meta: ribbit::Packed<M>,
        node: ribbit::Packed<node::Ptr<M>>,
    ) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(meta.with_value(false), node.raw().get())
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
    pub(crate) fn new_path<R>(
        mut reader: R,
        value: u64,
    ) -> (
        ribbit::Packed<Self>,
        Option<NonNull<ribbit::Atomic<Edge<M>>>>,
    )
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
        let null = !self.meta().is_value() && self.child_raw() == 0;
        validate!(!null || self.unfreeze() == Edge::NULL);
        null
    }

    /// Return `Some(node)` if this edge has a node child.
    #[inline]
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<node::Ptr<M>>> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { ribbit::Packed::<Option<node::Ptr<M>>>::new_unchecked(self.child_raw()) }
    }

    /// Return `Some(child)` if this edge has a child.
    #[inline]
    pub(crate) fn child(self) -> Option<Child<M>> {
        let raw = self.child_raw();
        if self.meta().is_value() {
            Some(Child::Value(raw))
        } else {
            unsafe { ribbit::Packed::<Option<node::Ptr<M>>>::new_unchecked(raw) }.map(Child::Node)
        }
    }

    /// Clear the frozen bit from this edge.
    #[inline]
    pub(super) fn unfreeze(self) -> Self {
        self.with_meta(self.meta().with_frozen(false))
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
pub(crate) enum Child<M> {
    Node(ribbit::Packed<node::Ptr<M>>),
    Value(u64),
}

impl<M> Debug for Child<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(node) => f.debug_tuple("Node").field(node).finish(),
            Self::Value(value) => f.debug_tuple("Value").field(value).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::fmt::Debug;

    use crate::raw::edge::Len;
    use crate::raw::edge::Meta;

    pub(super) fn expand_compress_inverse<M>(meta: M)
    where
        M: ribbit::Pack<Packed: Meta<Len: Debug>>,
    {
        let meta = meta.pack();

        for index in meta.len().range_to() {
            let Some((parent, byte, child)) = meta.try_expand(index) else {
                assert_eq!(index, meta.len());
                continue;
            };

            let actual = parent.try_compress(byte, child).unwrap();
            assert!(
                actual == meta
                    && actual.is_frozen() == meta.is_frozen()
                    && actual.is_value() == meta.is_value(),
                "Expand-compress mismatch:\n\
                {meta:x?}@{index:x?}\n\
                {parent:x?} - {byte:x?} - {child:x?}\n\
                {actual:x?}",
            );
        }
    }
}
