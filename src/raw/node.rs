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
use core::num::NonZeroU32;
use core::num::NonZeroU64;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

mod header;
mod iter;
mod node_15;
mod node_256;
mod node_3;
mod node_47;
pub(super) mod simd;

pub(crate) use iter::EntryIter;
pub(crate) use iter::KeyIndex;
pub(crate) use iter::KeyIter;
pub(crate) use iter::Lower;
pub(crate) use iter::Upper;
pub(super) use node_3::Node3;

use crate::raw::Edge;
use crate::raw::Smo;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Unbound;
use crate::raw::node::header::Header;
use crate::raw::node::iter::KeyIter3;
use crate::raw::node::iter::KeyIter15;
use crate::raw::node::iter::KeyIter47;
use crate::raw::node::iter::KeyIter256;
use crate::raw::node::node_15::Node15;
use crate::raw::node::node_47::Node47;
use crate::raw::node::node_256::Node256;
use crate::stat;
use crate::sync::Atomic;

/// A node is a partial mapping from `u8` to [`edge::Raw`].
#[derive(Debug)]
#[repr(C, align(64))]
pub(super) struct Node<const CAPACITY: usize, H> {
    pub(super) header: H,
    pub(super) edges: [Atomic<edge::Raw>; CAPACITY],
}

impl<const CAPACITY: usize, H: Default> Default for Node<CAPACITY, H> {
    fn default() -> Self {
        Self {
            header: H::default(),
            edges: core::array::from_fn(|_| Atomic::new_packed(edge::Raw::NULL)),
        }
    }
}

impl<const CAPACITY: usize, H: Header> Node<CAPACITY, H> {
    /// Initializes an unsorted iterator over this node's keys.
    #[inline]
    fn keys<L: Lower, U: Upper>(&self, lower: L, upper: U, iter: &mut H::KeyIter) {
        self.header.keys(lower, upper, iter)
    }

    #[inline]
    fn edges(&self) -> &[Atomic<edge::Raw>] {
        &self.edges
    }

    #[inline]
    fn edges_mut(&mut self) -> &mut [Atomic<edge::Raw>] {
        &mut self.edges
    }

    #[inline]
    fn get_key(&self, key: u8) -> Option<u8> {
        self.header.get(key)
    }

    #[inline]
    fn get_or_insert_key(&self, key: u8) -> Option<u8> {
        self.header.get_or_insert(key)
    }

    /// Freeze this node's header (i.e., its non-edge metadata).
    ///
    /// Returns the number of edges that must be frozen.
    #[inline]
    fn freeze_header(&self) -> usize {
        self.header.freeze()
    }

    #[inline]
    fn min<L: Lower>(&self, lower: L) -> Option<KeyIndex> {
        self.header.min(lower)
    }

    #[inline]
    fn max<U: Upper>(&self, upper: U) -> Option<KeyIndex> {
        self.header.max(upper)
    }
}

fn replace<const CAPACITY: usize, M: ribbit::Pack<Packed: edge::Meta>, H: Header>(
    node: &Node<CAPACITY, H>,
    meta: ribbit::Packed<M>,
    keys: &mut [u8; CAPACITY],
    edges: &mut [ribbit::Packed<Edge<M>>; CAPACITY],
) -> (Smo, ribbit::Packed<Edge<M>>) {
    // Caller must not call replace if doomed to fail CAS
    validate!(!meta.is_frozen());

    // Can only call replace on nodes
    validate!(!meta.is_value());

    let mut iter = H::KeyIter::default();
    node.keys(
        Unbound::<()>::default(),
        Unbound::<()>::default(),
        &mut iter,
    );

    let len = iter
        .map(|iter::KeyIndex { key, index }| {
            let index = index as usize;
            let raw = if_validate!(&node.edges()[index], unsafe {
                node.edges().get_unchecked(index)
            });
            let edge = unsafe { Edge::from_raw_ref(raw) }.load_packed(Ordering::Relaxed);
            (key, edge)
        })
        .filter(|(_, edge)| !edge.is_null())
        .map(|(key, edge)| (key, edge.unfreeze()))
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
    let new = unsafe {
        Ptr::new_unchecked(
            len == CAPACITY,
            &keys[..len],
            core::mem::transmute::<&[ribbit::Packed<Edge<M>>], &[ribbit::Packed<edge::Raw>]>(
                &edges[..len],
            ),
        )
    };
    let edge = Edge::new_node(meta, new);
    (Smo::ReplaceNode, edge)
}

/// Node type discriminant.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ribbit::Pack)]
#[ribbit(size = 2, derive(Debug, Eq))]
pub(crate) enum Type {
    Node3 = 0,
    Node15 = 1,
    Node47 = 2,
    Node256 = 3,
}

impl Type {
    #[cfg_attr(not(test), expect(unused))]
    pub(crate) const fn capacity(self) -> usize {
        match self {
            Self::Node3 => 3,
            Self::Node15 => 15,
            Self::Node47 => 47,
            Self::Node256 => 256,
        }
    }
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
            let r#type = $type.into_raw().value();
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
/// enum Ptr {
///     Node3(NonNull<Node3>),
///     Node15(NonNull<Node15>),
///     Node47(NonNull<Node47>),
///     Node256(NonNull<Node256>),
/// }
/// ```
///
/// But takes up 8 bytes, is compatible with `ribbit`, and avoids
/// jump tables when dispatching (see [`crate::raw::node::dispatch`]).
#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 64, derive(Eq), non_zero, new(vis = ""))]
pub(crate) struct Ptr {
    #[ribbit(size = 2, get(vis = "pub(crate)"))]
    r#type: crate::raw::node::Type,

    #[ribbit(with(skip))]
    _placeholder: NonZeroU32,
}

impl Ptr {
    const MASK_TYPE: u64 = 0b111;
    const MASK_PTR: u64 = !Self::MASK_TYPE;
}

impl Ptr {
    unsafe fn new_unchecked(
        grow: bool,
        keys: &[u8],
        edges: &[ribbit::Packed<edge::Raw>],
    ) -> ribbit::Packed<Self> {
        validate_eq!(keys.len(), edges.len());

        let len = keys.len();
        let len = if grow { len + 1 } else { len };

        // NOTE: leave room for at least one insert, in the
        // case where a full node header with some null children
        // is replaced and subsequently appended to.
        if len < 3 {
            unsafe { Self::new::<Node3, Atomic<node_3::Header>>(Node3::new_unchecked(keys, edges)) }
        } else if len < 14 {
            unsafe {
                Self::new::<Node15, Atomic<node_15::Header>>(Node15::new_unchecked(keys, edges))
            }
        } else if len < 47 {
            unsafe { Self::new::<Node47, node_47::Header>(Node47::new_unchecked(keys, edges)) }
        } else {
            unsafe { Self::new::<Node256, node_256::Header>(Node256::new_unchecked(keys, edges)) }
        }
    }

    // The only way a larger node can be created is through node replacement.
    #[inline]
    pub(super) fn new_node_3(node: Box<Node3>) -> ribbit::Packed<Self> {
        unsafe { Self::new::<_, Atomic<node_3::Header>>(node) }
    }

    unsafe fn new<N, H: Header>(node: Box<N>) -> ribbit::Packed<Self> {
        // NOTE: we rely on address (usize) <-> u64 conversions here
        const _: () = assert!(size_of::<usize>() == size_of::<u64>());

        let ptr = NonNull::from(Box::leak(node)).as_ptr().expose_provenance() as u64;

        validate_eq!(ptr & Self::MASK_TYPE, 0);

        unsafe {
            ribbit::Packed::<Self>::from_raw_unchecked(NonZeroU64::new_unchecked(
                H::TYPE as u64 | ptr,
            ))
        }
    }
}

/// Reduce dispatch boilerplate for identical branches.
macro_rules! dispatch_all {
    ($ptr:expr, $closure:expr) => {
        $ptr.dispatch($closure, $closure, $closure, $closure)
    };
}

/// # Edge metadata independent methods
impl PtrPacked {
    #[inline]
    pub(crate) unsafe fn get<'g>(self, key: u8) -> Option<&'g Atomic<edge::Raw>> {
        let (index, edges) = dispatch_all!(self, |node| {
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
    pub(crate) unsafe fn get_or_insert<'g>(self, key: u8) -> Option<&'g Atomic<edge::Raw>> {
        let (index, edges) = dispatch_all!(self, |node| {
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

    pub(crate) unsafe fn entries<'g, L: Lower, U: Upper>(
        self,
        sort: bool,
        lower: L,
        upper: U,
    ) -> EntryIter<'g> {
        let (keys, edges) = self.dispatch(
            |node| {
                let node = unsafe { node.as_ref() };
                let mut iter = KeyIter3::default();
                node.keys(lower, upper, &mut iter);
                if sort {
                    iter.sort();
                }
                (iter.into(), node.edges())
            },
            |node| {
                let node = unsafe { node.as_ref() };
                let mut iter = Box::new(KeyIter15::default());
                node.keys(lower, upper, &mut iter);
                if sort {
                    iter.sort();
                }
                (iter.into(), node.edges())
            },
            |node| {
                let node = unsafe { node.as_ref() };
                let mut iter = Box::new(KeyIter47::default());
                node.keys(lower, upper, &mut iter);
                (iter.into(), node.edges())
            },
            |node| {
                let node = unsafe { node.as_ref() };
                let mut iter = KeyIter256::default();
                node.keys(lower, upper, &mut iter);
                (iter.into(), node.edges())
            },
        );

        unsafe { EntryIter::new(keys, edges) }
    }

    pub(crate) unsafe fn entry_or_entries<'g, L: Lower, U: Upper>(
        self,
        sort: bool,
        lower: L,
        upper: U,
    ) -> Result<(u8, NonNull<Atomic<edge::Raw>>), EntryIter<'g>> {
        // Deduplicate with `entries`?
        let iter = self
            .dispatch(
                |node| {
                    let node = unsafe { node.as_ref() };
                    let mut iter = KeyIter3::default();
                    node.keys(lower, upper, &mut iter);
                    if sort {
                        iter.sort();
                    }
                    let edges = node.edges();
                    match iter.0.tail {
                        1 => {
                            let KeyIndex { key, index } = iter.0.entries[0];
                            Ok((key, NonNull::from(&edges[index as usize])))
                        }
                        _ => Err((iter.into(), edges)),
                    }
                },
                |node| {
                    let node = unsafe { node.as_ref() };
                    let mut iter = Box::new(KeyIter15::default());
                    node.keys(lower, upper, &mut iter);
                    if sort {
                        iter.sort();
                    }
                    Err((iter.into(), node.edges()))
                },
                |node| {
                    let node = unsafe { node.as_ref() };
                    let mut iter = Box::new(KeyIter47::default());
                    node.keys(lower, upper, &mut iter);
                    Err((iter.into(), node.edges()))
                },
                |node| {
                    let node = unsafe { node.as_ref() };
                    let mut iter = KeyIter256::default();
                    node.keys(lower, upper, &mut iter);
                    Err((iter.into(), node.edges()))
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

    #[expect(unused)]
    pub(crate) fn min<L: Lower>(self, lower: L) -> Option<KeyIndex> {
        dispatch_all!(self, |node| unsafe { node.as_ref().min(lower) })
    }

    #[expect(unused)]
    pub(crate) fn max<U: Upper>(self, upper: U) -> Option<KeyIndex> {
        dispatch_all!(self, |node| unsafe { node.as_ref().max(upper) })
    }

    /// Deallocate this node.
    ///
    /// # Safety
    ///
    /// Caller must ensure there are no other references to this node.
    pub(crate) unsafe fn deallocate(self) {
        dispatch_all!(self, |node| drop(unsafe { Box::from_raw(node.as_ptr()) }))
    }

    #[inline(always)]
    fn dispatch<N3, N15, N47, N256, T>(
        self,
        node_3: N3,
        node_15: N15,
        node_47: N47,
        node_256: N256,
    ) -> T
    where
        N3: FnOnce(NonNull<Node3>) -> T,
        N15: FnOnce(NonNull<Node15>) -> T,
        N47: FnOnce(NonNull<Node47>) -> T,
        N256: FnOnce(NonNull<Node256>) -> T,
    {
        let ptr = NonNull::<u8>::new(core::ptr::with_exposed_provenance_mut(
            (self.into_raw().get() & Ptr::MASK_PTR) as usize,
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

/// # Edge metadata dependent methods
impl PtrPacked {
    pub(crate) unsafe fn len<M: ribbit::Pack<Packed: edge::Meta>>(self) -> u8 {
        dispatch_all!(self, |node| unsafe { node.as_ref() }.edges())
            .iter()
            .map(|raw| unsafe { Edge::<M>::from_raw_ref(raw) })
            .filter(|edge| !edge.load_packed(Ordering::Relaxed).is_null())
            .count() as u8
    }

    pub(crate) unsafe fn freeze<M: ribbit::Pack<Packed: edge::Meta>>(self) {
        dispatch_all!(self, |node| {
            let node = unsafe { node.as_ref() };
            let len = node.freeze_header();
            node.edges()
                .iter()
                .take(len)
                .map(|raw| unsafe { Edge::<M>::from_raw_ref(raw) })
                .for_each(Edge::freeze)
        });
    }

    pub(crate) unsafe fn replace<M: ribbit::Pack<Packed: edge::Meta>>(
        self,
        parent: ribbit::Packed<M>,
    ) -> (Smo, ribbit::Packed<Edge<M>>) {
        self.dispatch(
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    &mut [0u8; 3],
                    &mut [Edge::NULL; 3],
                )
            },
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    &mut [0u8; 15],
                    &mut [Edge::NULL; 15],
                )
            },
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    &mut [0u8; 47],
                    &mut [Edge::NULL; 47],
                )
            },
            |node| {
                replace(
                    unsafe { node.as_ref() },
                    parent,
                    &mut [0u8; 256],
                    &mut [Edge::NULL; 256],
                )
            },
        )
    }

    /// Deallocate recursive `Node3`s created by [`crate::raw::Edge::new_path`].
    /// Does not deallocate the final value.
    ///
    /// # Safety
    ///
    /// Caller must ensure:
    /// - There are no other references to this node.
    /// - This is a Node3 created by [`crate::raw::Edge::new_path`].
    pub(crate) unsafe fn deallocate_recursive<M>(self)
    where
        M: ribbit::Pack<Packed: edge::Meta>,
    {
        let mut prev: Option<NonNull<Node3>> = None;
        let mut next = self;
        let mut done = false;

        while !done {
            next.dispatch(
                |mut node_3| match unsafe {
                    Edge::<M>::from_raw_mut(&mut node_3.as_mut().edges_mut()[0])
                }
                .get_mut_packed()
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
}

impl Debug for PtrPacked {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Node")
            .field("type", &self.r#type())
            .field("ptr", &(self.into_raw().get() & Ptr::MASK_PTR))
            .finish()
    }
}
