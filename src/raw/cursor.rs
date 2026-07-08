pub(crate) mod path;
pub(crate) use path::Path;

use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::stat;
use crate::sync::Atomic;

/// Tree traversal state.
pub(crate) struct Cursor<'g, R: key::Read, P> {
    len: R::Len,

    /// Current key reader
    reader: R,

    /// Edge this cursor currently points to
    edge: NonNull<Atomic<Edge<R::Edge>>>,

    /// Path this cursor has taken
    path: P,

    _global: PhantomData<&'g Atomic<Edge<R::Edge>>>,
}

/// Outcome of [`Cursor::traverse_insert`] indicating if
/// traversal terminated at a value, or if an SMO is
/// required to continue traversal.
pub(crate) enum Insert<M: ribbit::Pack<Packed: edge::Meta>> {
    /// Either a value was found, or there is no
    /// value for this key.
    ///
    /// NOTE: unlike [`Update`], it is possible for
    /// `value.map(Child::Value) != edge.child()`, in the
    /// case that an edge expansion is required at
    /// an edge that has a value child.
    Value {
        value: Option<u64>,
        edge: ribbit::Packed<Edge<M>>,
    },

    /// Node replacement is required to continue traversal.
    ///
    /// Guaranteed that `Some(Child::Node(node)) == edge.child()`.
    Replace {
        node: ribbit::Packed<node::Ptr>,
        edge: ribbit::Packed<Edge<M>>,
    },
}

/// Outcome of [`Cursor::traverse_value`].
///
/// Guaranteed that `Some(Child::Value(value)) == edge.child()`.
pub(crate) struct Value<M: ribbit::Pack<Packed: edge::Meta>> {
    pub(crate) value: u64,
    pub(crate) edge: ribbit::Packed<Edge<M>>,
}

/// Outcome of [`Cursor::freeze`].
pub(crate) enum Freeze {
    /// Freeze suceeded, either due to successfully replacing
    /// the node ourselves, in which case we need to retire
    /// `Some(node)`, or due to another thread concurrently
    /// replacing the node, in which case this will contain `None`.
    Success(Option<ribbit::Packed<node::Ptr>>),

    /// Detected a concurrent edge expansion, so caller
    /// must re-traverse to frozen node.
    Traverse,
}

impl<'g, R> Cursor<'g, R, path::Discard>
where
    R: key::Read,
{
    /// Traverse to the value associated with the key, if it exists.
    #[inline]
    pub(crate) fn traverse_get(&mut self) -> Option<u64> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);
            let len = self.reader.match_exact(edge.meta())?;

            match edge.child()? {
                edge::Child::Node(node) => {
                    // SAFETY: prefix precondition implies search key cannot equal node prefix
                    let byte = unsafe { self.reader.get_byte_unchecked(len) };

                    // Skip `self.push` call since `get` never back-tracks
                    self.edge = unsafe { node.get(byte) }.map(NonNull::from)?.cast();
                    self.reader = self.reader.suffix(R::Len::BYTE + len.into());
                }
                edge::Child::Value(value) => {
                    // Prefix precondition implies search key must match
                    validate!(self.reader.len() == len.into());
                    return Some(value);
                }
            }
        }
    }
}

impl<'g, R, P> Cursor<'g, R, P>
where
    R: key::Read,
    P: Path<R>,
{
    /// # Safety
    ///
    /// Caller must ensure that all nodes underneath `root` along the path associated
    /// with `reader` live at least as long as this struct.
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic<Edge<R::Edge>>, reader: R) -> Self {
        Self {
            len: R::Len::ZERO,
            edge: NonNull::from(root),
            reader,
            path: P::default(),
            _global: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn edge(&self) -> &'g Atomic<Edge<R::Edge>> {
        unsafe { self.edge.as_ref() }
    }

    #[inline]
    pub(crate) unsafe fn edge_mut(&mut self) -> &'g mut Atomic<Edge<R::Edge>> {
        unsafe { self.edge.as_mut() }
    }

    #[inline]
    pub(crate) unsafe fn as_value_unchecked(&self) -> NonNull<u64> {
        unsafe { Edge::as_value_unchecked(self.edge) }
    }

    #[inline]
    pub(crate) fn len(&self) -> R::Len {
        self.len
    }

    /// Traverse to the root of the subtree prefixed by the key, if it exists.
    pub(crate) fn traverse_prefix(&mut self) -> Option<ribbit::Packed<Edge<R::Edge>>> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);
            let child = edge.child()?;
            let meta = edge.meta();

            let len_edge = meta.len();
            let len_prefix = self.reader.match_prefix(meta);

            if len_prefix >= len_edge.into()
                && let edge::Child::Node(node) = child
                && let Some(byte) = self.reader.get_byte(len_edge)
            {
                let next = unsafe { node.get(byte) }?;
                self.push(len_edge, node, next);
                continue;
            }

            if len_prefix < self.reader.len() {
                return None;
            } else {
                return Some(edge);
            }
        }
    }

    /// Traverse to the value associated with the key.
    ///
    /// Returns `None` if there is no such edge,
    /// or `Some(value)` otherwise.
    pub(crate) fn traverse_value(&mut self) -> Option<Value<R::Edge>> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);
            let len = self.reader.match_exact(edge.meta())?;

            match edge.child()? {
                edge::Child::Node(node) => {
                    // SAFETY: prefix precondition implies search key cannot equal node prefix
                    let byte = unsafe { self.reader.get_byte_unchecked(len) };

                    let next = unsafe { node.get(byte) }?;
                    self.push(len, node, next);
                    continue;
                }
                edge::Child::Value(value) => {
                    // Prefix precondition implies search key must match
                    validate!(self.reader.len() == len.into());

                    return Some(Value { value, edge });
                }
            }
        }
    }

    /// Traverse to the node associated with the key.
    ///
    /// Returns the edge length to the node if successful,
    /// or else returns the remaining key length.
    pub(crate) fn traverse_node(
        &mut self,
    ) -> Result<<ribbit::Packed<R::Edge> as edge::Meta>::Len, R::Len> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);
            let Some(len) = self.reader.match_exact(edge.meta()) else {
                return Err(self.reader.len());
            };

            match edge.child() {
                None => return Err(self.reader.len()),
                Some(edge::Child::Value(_)) => unreachable!("Prefix condition"),
                Some(edge::Child::Node(node)) => {
                    let Some(byte) = self.reader.get_byte(len) else {
                        // Found target node
                        return Ok(len);
                    };

                    let Some(next) = (unsafe { node.get(byte) }) else {
                        return Err(self.reader.len());
                    };

                    self.push(len, node, next);
                    continue;
                }
            }
        }
    }

    /// Traverse to the edge associated with the key, or to
    /// the first edge where an SMO would be necessary to
    /// insert the key.
    pub(crate) fn traverse_insert(&mut self) -> Insert<R::Edge> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);

            let Some(child) = edge.child() else {
                // Case: no child, create path
                return Insert::Value { value: None, edge };
            };

            let Some(len) = self.reader.match_exact(edge.meta()) else {
                // Case: partial match, expand edge
                return Insert::Value { value: None, edge };
            };

            match child {
                edge::Child::Node(node) => {
                    // SAFETY: prefix precondition implies search key cannot equal node prefix
                    let byte = unsafe { self.reader.get_byte_unchecked(len) };

                    let Some(next) = (unsafe { node.get_or_insert(byte) }) else {
                        // Case: node replacement
                        return Insert::Replace { node, edge };
                    };

                    self.push(len, node, next);
                }
                edge::Child::Value(value) => {
                    // Prefix precondition implies search key must match
                    validate!(self.reader.len() == len.into());

                    return Insert::Value {
                        value: Some(value),
                        edge,
                    };
                }
            }
        }
    }

    /// Locally create a path from the current edge
    /// to insert this key value pair. May create nodes recursively if
    /// the remaining key is long.
    #[expect(clippy::type_complexity)]
    pub(crate) fn create_path(
        &self,
        old: ribbit::Packed<Edge<R::Edge>>,
        value: u64,
    ) -> (
        ribbit::Packed<Edge<R::Edge>>,
        Option<NonNull<Atomic<Edge<R::Edge>>>>,
    ) {
        let meta = old.meta();
        let len = self.reader.match_prefix(meta).into();

        match meta.try_expand(len) {
            None => Edge::new_path(self.reader, value),
            Some((parent, old_byte, old_child)) => {
                let new_byte = unsafe { self.reader.get_byte_unchecked(len) };
                let (new_child, tail_path) =
                    Edge::new_path(self.reader.suffix(R::Len::BYTE + len.into()), value);

                // NOTE: must put new allocation first because
                // `deallocate_recursive` recurses on first edge
                let (head, tail_expand) = Node3::new_expand(
                    parent,
                    [new_byte, old_byte],
                    [new_child, old.with_meta(old_child)],
                );

                // If `tail_path` has stable address, use it, otherwise
                // use address of first `Node3` edge
                (head, Some(tail_path.unwrap_or(tail_expand)))
            }
        }
    }

    /// Freeze and replace the closest node along the traversal path
    /// such that (a) the parent edge of this node is unfrozen, and
    /// (b) every subsequent edge along the traversal path is frozen.
    ///
    /// # Example
    ///
    /// ```text
    ///              root   self.edge
    ///                 |   |
    ///                 v   v
    ///               +---+---+---+
    ///               | a | b | c |
    ///               +---+---+---+
    ///                 |   |   |
    ///                 v  a|   v
    ///                 1  b|   2
    /// old_len = 2         |
    ///                     v
    ///                   +---+---+---+
    /// old_node -------> | d |   |   |
    ///                   +---+---+---+
    ///                     |
    ///                     v
    ///                   +---+---+---+
    ///                   | e |   |   |
    ///                   +---+---+---+
    ///                     |
    ///                     v
    ///                     3
    /// ```
    ///
    /// # Safety
    ///
    /// Caller must guarantee `old_len` and `old_node` are consistent
    /// with the cursor: if `old_node` has prefix `p + e` for some `e`,
    /// where `len(e) == old_len`, then `self.edge` must currently have
    /// prefix `p`.
    #[cold]
    pub(crate) unsafe fn freeze(
        &mut self,
        mut old_len: <ribbit::Packed<R::Edge> as edge::Meta>::Len,
        mut old_node: ribbit::Packed<node::Ptr>,
    ) -> Result<Freeze, P::PopError> {
        let mut old_edge = self.edge().load_packed(Ordering::Acquire);
        let mut pop = 1;

        let old_node = loop {
            // If `old_edge` is already frozen, we won't be able to CAS it after
            // replacing `old_node`. Continue popping until we reach the
            // closest unfrozen edge.
            //
            // ```text
            //      closest unfrozen edge
            //                 |
            //                 v
            //               +---+---+---+
            //               |   |   |   |
            //               +---+---+---+
            //                 |
            //                 v
            //               +---+---+---+
            // self.edge --> | F | F | F |
            //               +---+---+---+
            //        old_edge |
            //                 v
            //               +---+---+---+
            // old_node ---> | F | F | F |
            //               +---+---+---+
            //                 |
            //                 v
            //                 1
            // ```
            while old_edge.meta().is_frozen() {
                let (next_len, next_node) = self.pop()?.expect("Root edge cannot be frozen");
                old_len = next_len;
                old_node = next_node;
                old_edge = self.edge().load_packed(Ordering::Acquire);
                pop += 1;
            }

            match old_edge.child() {
                // Node hasn't changed since we traversed
                Some(edge::Child::Node(node)) if node == old_node => {
                    validate_eq!(old_len, old_edge.meta().len());
                }

                Some(edge::Child::Node(_)) => match old_edge.meta().len().cmp(&old_len) {
                    // A concurrent edge expansion must have happened.
                    // Caller must re-traverse through expanded node.
                    //
                    // ```text
                    //              root   self.edge
                    //                 |   |
                    //                 v   v
                    //               +---+---+---+
                    //               | a | b | c |
                    //               +---+---+---+
                    //                 |   |   |
                    // new_len = 1     v  a|   v
                    //                 1   |   2
                    //                     |
                    //                     |
                    //                   +---+---+---+
                    // new_node -------> | b |   |   |
                    //                   +---+---+---+
                    //                     |
                    //                     v
                    // old_len = 2       +---+---+---+
                    // old_node -------> | d |   |   |
                    //                   +---+---+---+
                    //                     |
                    //                     v
                    //                   +---+---+---+
                    //                   | e |   |   |
                    //                   +---+---+---+
                    //                     |
                    //                     v
                    //                     3
                    // ```
                    core::cmp::Ordering::Less => break Freeze::Traverse,

                    // Node must have been replaced...
                    //
                    // ```text
                    //              root   self.edge
                    //                 |   |
                    //                 v   v
                    //               +---+---+---+
                    //               | a | b | c |
                    //               +---+---+---+
                    //                 |   |   |
                    //                 v  a|   v
                    //                 1  b|   2
                    //                     |
                    //                     v
                    // new_len = 2       +---+---+-----+---+
                    // new_node -------> | d |   | ... |   |
                    //                   +---+---+-----+---+
                    //                     |
                    //                     v
                    //                   +---+---+---+
                    //                   | e |   |   |
                    //                   +---+---+---+
                    //                     |
                    //                     v
                    //                     3
                    // ```
                    core::cmp::Ordering::Equal

                    // or removed via edge compression.
                    //
                    // ```text
                    //              root   self.edge
                    //                 |   |
                    //                 v   v
                    //               +---+---+---+
                    //               | a | b | c |
                    //               +---+---+---+
                    //                 |   |   |
                    //                 v  a|   v
                    // new_len = 3     1  b|   2
                    //                    d|
                    //                     v
                    //                   +---+---+---+
                    // new_node -------> | e |   |   |
                    //                   +---+---+---+
                    //                     |
                    //                     v
                    //                     3
                    // ```
                    | core::cmp::Ordering::Greater => break Freeze::Success(None),
                },

                // Node must have been removed.
                child @ (None | Some(edge::Child::Value(_))) => {
                    // NOTE: usually the `edge::Child::Value` case is unreachable
                    // due to the prefix condition. In the specific case of unsized,
                    // non-null keys, however, `old_node` can be concurrently replaced
                    // with a value, with an implicit terminator byte in `edge::Slice`.
                    validate!(child.is_none_or(|_| old_edge.meta().is_terminate()));
                    break Freeze::Success(None);
                }
            };

            let (smo, new_edge) = unsafe {
                old_node.freeze::<R::Edge>();
                old_node.replace(old_edge.meta())
            };

            match self.edge().compare_exchange_packed(
                old_edge,
                new_edge,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break Freeze::Success(Some(old_node)),
                Err(conflict) => {
                    if smo.is_allocate() {
                        let new_node = new_edge.as_node().expect("Allocating SMO creates node");
                        stat::increment(stat::Counter::FreeConflict);
                        // SAFETY: `new_node` has not been made globally visible,
                        // so it is safe to deallocate without SMR.
                        unsafe { new_node.deallocate() };
                    }
                    old_edge = conflict;
                }
            };
        };

        stat::record(stat::Record::FreezePop, pop);
        Ok(old_node)
    }

    #[inline]
    fn push(
        &mut self,
        len: <ribbit::Packed<R::Edge> as edge::Meta>::Len,
        node: ribbit::Packed<node::Ptr>,
        edge: &'g Atomic<edge::Raw>,
    ) {
        self.path.push(path::Segment {
            reader: self.reader,
            len,
            edge: core::mem::replace(&mut self.edge, NonNull::from(edge).cast()),
            node,
        });

        // 1 extra byte for node
        let delta = R::Len::BYTE + len.into();
        self.len += delta;
        self.reader = self.reader.suffix(delta);
    }

    #[inline]
    pub(crate) fn pop(
        &mut self,
    ) -> Result<
        Option<(
            <ribbit::Packed<R::Edge> as edge::Meta>::Len,
            ribbit::Packed<node::Ptr>,
        )>,
        P::PopError,
    > {
        let Some(segment) = self.path.pop()? else {
            return Ok(None);
        };
        self.len -= R::Len::BYTE + segment.len.into();
        self.reader = segment.reader;
        self.edge = segment.edge;
        Ok(Some((segment.len, segment.node)))
    }

    #[inline]
    pub(crate) fn trim(&mut self, len: R::Len) {
        self.path.trim(len);
        self.reader = self.reader.prefix(self.reader.len() - len);
    }
}
