//! A node is a partial map from key byte ([`u8`]) to edge ([`crate::raw::Edge`]).
//!
//! Adaptive radix trees use different node representations
//! depending on occupancy to reduce memory overhead. Roughly speaking,
//! each node representation consists of some header metadata and an
//! array of edges. This module implements the various node representations
//! (e.g., [`Node3`], [`Node256`]) and the shared interface they implement ([`Node`]).
//!
//! At runtime, we use [`Type`] to distinguish between representations, and
//! [`Ptr`] as a more performant alternative relative to an enum or
//! `&dyn Node` that fits in 8 bytes (and hence within a [`crate::raw::Edge`]).

use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::OptionExt as _;

mod iter;
mod linear;
mod node_15;
mod node_256;
mod node_3;
mod node_47;
pub(super) mod simd;

pub(crate) use iter::EntryIter;
pub(crate) use iter::KeyIter;
pub(crate) use iter::Lower;
pub(crate) use iter::Upper;
pub(crate) use node_3::Node3;
pub(crate) use node_15::Node15;
pub(crate) use node_47::Node47;
pub(crate) use node_256::Node256;

use crate::raw::Edge;
use crate::raw::Smo;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Unbound;
use crate::stat;
use linear::Linear;

/// A node is a partial mapping from `u8` to [`crate::raw::Edge`].
///
/// # Safety
///
/// Implementations must ensure that all returned key indices are within
/// `self.edges()` and `self.edges_mut()`.
unsafe trait Node<M>: Default
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    /// A runtime representation of the node type.
    const TYPE: Type;

    /// The maximum number of entries this node can contain.
    const CAPACITY: usize;

    type KeyIter: Iterator<Item = iter::KeyIndex> + Into<KeyIter>;

    /// Returns a new node populated with `keys` and `edges`.
    ///
    /// # Safety
    ///
    /// Caller must ensure the following:
    /// - `keys.len() == edges.len()`
    /// - `keys.len() <= Self::CAPACITY`
    /// - Keys are unique
    /// - Edges are unique
    unsafe fn new_unchecked(keys: &[u8], edges: &[ribbit::Packed<Edge<M>>]) -> Box<Self>;

    /// Returns a sorted iterator over this node's keys.
    fn keys<L: iter::Lower, U: iter::Upper>(&self, lower: L, upper: U) -> Self::KeyIter;

    fn edges(&self) -> &[Atomic<Edge<M>>];

    fn edges_mut(&mut self) -> &mut [Atomic<Edge<M>>];

    /// # Safety
    ///
    /// Implementer must guarantee that `Some(index)` is within `self.edges()`
    fn get_key(&self, key: u8) -> Option<u8>;

    /// # Safety
    ///
    /// Implementer must guarantee that `Some(index)` is within `self.edges()`
    fn get_or_insert_key(&self, key: u8) -> Option<u8>;

    /// Freeze this node's header (i.e., its non-edge metadata).
    ///
    /// Returns the number of edges that must be frozen.
    fn freeze_header(&self) -> usize;
}

fn replace<const CAPACITY: usize, M: ribbit::Pack<Packed: edge::Meta>, N: Node<M>>(
    node: &N,
    meta: ribbit::Packed<M>,
    freeze: bool,
    keys: &mut [u8; CAPACITY],
    edges: &mut [ribbit::Packed<Edge<M>>; CAPACITY],
) -> (Smo, ribbit::Packed<Edge<M>>) {
    const {
        // HACK: can't use associated constant as array length
        assert!(CAPACITY == N::CAPACITY);
    }

    // Caller must not call replace if doomed to fail CAS
    validate!(!meta.is_frozen());

    // Can only call replace on nodes
    validate!(!meta.is_value());

    if freeze {
        let len = node.freeze_header();
        node.edges().iter().take(len).for_each(Edge::freeze)
    }

    let len = node
        .keys(Unbound::<()>::default(), Unbound::<()>::default())
        .map(|iter::KeyIndex { key, index }| {
            let index = index as usize;
            let edge = if_validate!(&node.edges()[index], unsafe {
                node.edges().get_unchecked(index)
            })
            .load_packed(Ordering::Relaxed);
            (key, edge)
        })
        .filter(|(_, edge)| !edge.is_null())
        .map(|(key, edge)| match freeze {
            true => {
                validate!(
                    edge.meta().is_frozen(),
                    "Edge must be frozen before replace",
                );
                (key, edge.unfreeze())
            }
            false => {
                validate!(!edge.meta().is_frozen(), "Edge must not be frozen",);
                (key, edge)
            }
        })
        .zip(core::iter::zip(&mut *keys, &mut *edges))
        .map(|((key_old, edge_old), (key_new, edge_new))| {
            *key_new = key_old;
            *edge_new = edge_old;
        })
        .count();

    if len == 0 {
        return (Smo::DeleteNode, Edge::NULL);
    } else if len == 1 {
        let key = keys[0];
        let edge = edges[0];
        if let Some(meta) = meta.try_compress(key, edge.meta()) {
            return (Smo::CompressEdge, edge.with_meta(meta));
        }
    }

    // Heuristic: assume a full node should be expanded
    let node = unsafe { Ptr::new_unchecked(len == N::CAPACITY, &keys[..len], &edges[..len]) };
    let edge = Edge::new_node(meta, node);
    (Smo::ReplaceNode, edge)
}

/// Node type discriminant.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 2, eq, debug, packed(rename = "TypePacked"))]
pub(crate) enum Type {
    Node3 = 0,
    Node15 = 1,
    Node47 = 2,
    Node256 = 3,
}

/// Optimization for branching on node type.
///
/// We use a manual if-else chain instead of a match here because LLVM generates
/// a jump table for the latter. In our experiments, we observe that a jump table
/// in hot loops causes significant slowdowns: the jump table causes more branch
/// mispredictions, and the mispredicted branches cause excess cache coherence
/// traffic for cache lines that would otherwise be untouched.
///
/// We use a macro instead of a function because there is no way to express mutually
/// exclusive closures as parameters. We sometimes need $node3, $node15, $node47, and
/// $node256 to borrow the same data mutably.
macro_rules! dispatch {
    ($type:expr, $node3:expr, $node15:expr, $node47:expr, $node256:expr $(,)?) => {{
        if cfg!(feature = "opt-no-dispatch") {
            use crate::raw::node::Type;
            use ribbit::Unpack as _;
            match $type.unpack() {
                Type::Node3 => $node3,
                Type::Node15 => $node15,
                Type::Node47 => $node47,
                Type::Node256 => $node256,
            }
        } else {
            let r#type = $type.value.value();
            let hi = r#type & 0b10;
            let lo = r#type & 0b01;

            if hi == 0 {
                if lo == 0 { $node3 } else { $node15 }
            } else if lo == 0 {
                $node47
            } else {
                $node256
            }
        }
    }};
}
pub(super) use dispatch;

/// Pointer to a node representation.
///
/// Conceptually the same as the following type:
///
/// ```ignore
/// enum Ptr<M> {
///     Node3(NonNull<Node3<M>>),
///     Node15(NonNull<Node15<M>>),
///     Node47(NonNull<Node47<M>>),
///     Node256(NonNull<Node256<M>>),
/// }
/// ```
///
/// But takes up 8 bytes, is compatible with `ribbit`, and avoids
/// jump tables when dispatching (see [`crate::raw::node::dispatch`]).
#[derive(ribbit::Pack)]
#[ribbit(size = 64, packed(rename = PtrPacked), eq, nonzero)]
pub(crate) struct Ptr<M> {
    #[ribbit(size = 2, get(vis = "pub(crate)"))]
    r#type: Type,

    #[ribbit(with(skip))]
    _placeholder: NonZeroU32,

    _meta: PhantomData<M>,
}

impl<M> Copy for Ptr<M> {}
impl<M> Clone for Ptr<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Ptr<M> {
    const MASK_TYPE: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TYPE;
}

impl<M> Ptr<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    unsafe fn new_unchecked(
        grow: bool,
        keys: &[u8],
        edges: &[ribbit::Packed<Edge<M>>],
    ) -> ribbit::Packed<Self> {
        validate_eq!(keys.len(), edges.len());

        let len = keys.len();
        let len = if grow { len + 1 } else { len };

        if len < 4 {
            Self::new(unsafe { Node3::new_unchecked(keys, edges) })
        } else if len < 16 {
            Self::new(unsafe { Node15::new_unchecked(keys, edges) })
        } else if len < 48 {
            Self::new(unsafe { Node47::new_unchecked(keys, edges) })
        } else {
            Self::new(unsafe { Node256::new_unchecked(keys, edges) })
        }
    }

    #[inline]
    pub(crate) unsafe fn from_raw_unchecked(raw: u64) -> ribbit::Packed<Self> {
        let node = unsafe { ribbit::Packed::<Option<Ptr<M>>>::new_unchecked(raw) };
        if_validate!(node.unwrap(), unsafe { node.unwrap_unchecked() })
    }

    // The only way a larger node can be created is through node replacement.
    #[inline]
    pub(crate) fn new_node_3(node: Box<Node3<M>>) -> ribbit::Packed<Self> {
        Self::new(node)
    }

    fn new<N: Node<M>>(node: Box<N>) -> ribbit::Packed<Self> {
        // NOTE: we rely on address (usize) <-> u64 conversions here
        const _: () = assert!(size_of::<usize>() == size_of::<u64>());

        let ptr = NonNull::from(Box::leak(node)).as_ptr().expose_provenance() as u64;

        validate_eq!(ptr & Self::MASK_TYPE, 0);

        unsafe {
            ribbit::Packed::<Self>::new_unchecked(NonZeroU64::new_unchecked(N::TYPE as u64 | ptr))
        }
    }
}

/// Reduce dispatch boilerplate when every branch is identical.
macro_rules! impl_forward {
    ($ptr:expr, $closure:expr) => {
        $ptr.dispatch($closure, $closure, $closure, $closure)
    };
}

impl<M> PtrPacked<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    #[inline]
    pub(crate) fn raw(self) -> NonZeroU64 {
        self.value
    }

    pub(crate) unsafe fn len(self) -> u8 {
        impl_forward!(self, |node| unsafe { node.as_ref() }.edges())
            .iter()
            .filter(|edge| !edge.load_packed(Ordering::Relaxed).is_null())
            .count() as u8
    }

    #[inline]
    pub(crate) unsafe fn get<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        let (index, edges) = impl_forward!(self, |node| {
            let node = unsafe { node.as_ref() };
            let index = node.get_key(key);
            let edges = node.edges();
            (index, edges)
        });

        let index = index? as usize;
        Some(if_validate!(&edges[index], unsafe {
            edges.get_unchecked(index)
        }))
    }

    #[inline]
    pub(crate) unsafe fn get_or_insert<'g>(self, key: u8) -> Option<&'g Atomic<Edge<M>>> {
        let (index, edges) = impl_forward!(self, |node| {
            let node = unsafe { node.as_ref() };
            let index = node.get_or_insert_key(key);
            let edges = node.edges();
            (index, edges)
        });

        let index = index? as usize;
        Some(if_validate!(&edges[index], unsafe {
            edges.get_unchecked(index)
        }))
    }

    pub(crate) unsafe fn replace(
        self,
        parent: ribbit::Packed<M>,
        freeze: bool,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        self.dispatch(
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    freeze,
                    &mut [0u8; 3],
                    &mut [Edge::NULL; 3],
                )
            },
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    freeze,
                    &mut [0u8; 15],
                    &mut [Edge::NULL; 15],
                )
            },
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    freeze,
                    &mut [0u8; 47],
                    &mut [Edge::NULL; 47],
                )
            },
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    freeze,
                    &mut [0u8; 256],
                    &mut [Edge::NULL; 256],
                )
            },
        )
    }

    pub(crate) unsafe fn entries<'g, L: Lower, U: Upper>(
        self,
        lower: L,
        upper: U,
    ) -> EntryIter<'g, M> {
        let (keys, edges) = impl_forward!(self, |node| {
            let node = unsafe { node.as_ref() };
            (KeyIter::from(node.keys(lower, upper)), node.edges())
        });

        unsafe { EntryIter::new(keys, edges) }
    }

    pub(crate) unsafe fn entry_or_entries<'g, L: Lower, U: Upper>(
        self,
        lower: L,
        upper: U,
    ) -> Result<(u8, NonNull<ribbit::Atomic<Edge<M>>>), EntryIter<'g, M>> {
        let iter = self
            .dispatch(
                |node| {
                    let node = unsafe { node.as_ref() };
                    let mut keys = node.keys(lower, upper);
                    let edges = node.edges();
                    match keys.size_hint().1 {
                        Some(1) => {
                            let pair = keys.next().expect("Size hint is exact");
                            Ok((pair.key, NonNull::from(&edges[pair.index as usize])))
                        }
                        _ => Err((keys.into(), edges)),
                    }
                },
                |node| {
                    let node = unsafe { node.as_ref() };
                    Err((node.keys(lower, upper).into(), node.edges()))
                },
                |node| {
                    let node = unsafe { node.as_ref() };
                    Err((node.keys(lower, upper).into(), node.edges()))
                },
                |node| {
                    let node = unsafe { node.as_ref() };
                    Err((node.keys(lower, upper).into(), node.edges()))
                },
            )
            .map_err(|(keys, edges)| unsafe { EntryIter::new(keys, edges) });

        stat::increment(if iter.is_ok() {
            stat::Counter::EntriesOne
        } else {
            stat::Counter::EntriesMany
        });

        iter
    }

    /// # Safety
    ///
    /// Caller must ensure there are no other references to this node.
    pub(crate) unsafe fn deallocate(self, counter: stat::Counter) {
        stat::increment(counter);
        impl_forward!(self, |node| drop(unsafe { Box::from_raw(node.as_ptr()) }))
    }

    /// Deallocate recursive `Node3`s created by [`crate::raw::Edge::new_path`].
    /// Does not deallocate the final value.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - There are no other references to this node.
    /// - This is a Node3 created by [`crate::raw::Edge::new_path`].
    pub(crate) unsafe fn deallocate_recursive(self, counter: stat::Counter) {
        stat::increment(counter);

        let mut prev: Option<NonNull<Node3<_>>> = None;
        let mut next = self;
        let mut done = false;

        while !done {
            next.dispatch(
                |mut node_3| match unsafe { node_3.as_mut() }.edges_mut()[0]
                    .get_packed()
                    .child()
                {
                    None => unreachable!(),
                    Some(edge::Child::Value(_)) => {
                        if let Some(node_3) = prev {
                            drop(unsafe { Box::from_raw(node_3.as_ptr()) });
                        }
                        done = true;
                    }
                    Some(edge::Child::Node(node)) => {
                        prev = Some(node_3);
                        next = node;
                    }
                },
                |_| unreachable!(),
                |_| unreachable!(),
                |_| unreachable!(),
            );
        }
    }

    #[inline(always)]
    pub(crate) fn dispatch<N3, N15, N47, N256, T>(
        self,
        node_3: N3,
        node_15: N15,
        node_47: N47,
        node_256: N256,
    ) -> T
    where
        N3: FnOnce(NonNull<Node3<M>>) -> T,
        N15: FnOnce(NonNull<Node15<M>>) -> T,
        N47: FnOnce(NonNull<Node47<M>>) -> T,
        N256: FnOnce(NonNull<Node256<M>>) -> T,
    {
        let ptr = NonNull::<u8>::new(core::ptr::with_exposed_provenance_mut(
            (self.value.get() & Ptr::<M>::MASK_PTR) as usize,
        ));
        let ptr = if_validate!(ptr.unwrap(), unsafe { ptr.unwrap_unchecked() });

        dispatch!(
            self.r#type(),
            node_3(ptr.cast()),
            node_15(ptr.cast()),
            node_47(ptr.cast()),
            node_256(ptr.cast()),
        )
    }
}

impl<M> Debug for PtrPacked<M> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("type", &self.r#type())
            .field("ptr", &(self.value.get() & Ptr::<M>::MASK_PTR))
            .finish()
    }
}
