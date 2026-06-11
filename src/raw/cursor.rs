pub(crate) mod path;
pub(crate) use path::Path;

use core::marker::PhantomData;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use crate::Atomic;
use crate::raw::Edge;
use crate::raw::Frozen;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node3;
use crate::stat;

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

/// Outcome of [`Cursor::traverse_update`].
///
/// Guaranteed that `Some(Child::Value(value)) == edge.child()`.
pub(crate) struct Update<M: ribbit::Pack<Packed: edge::Meta>> {
    pub(crate) value: u64,
    pub(crate) edge: ribbit::Packed<Edge<M>>,
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

            match edge.child()? {
                edge::Child::Node(node) => {
                    let len = self.reader.match_exact(edge.meta())?;
                    // SAFETY: prefix precondition implies search key cannot equal node prefix
                    let byte = unsafe { self.reader.get_byte_unchecked(len) };

                    // Skip `self.push` call since `get` never back-tracks
                    self.edge = unsafe { node.get(byte) }.map(NonNull::from)?.cast();
                    self.reader = self.reader.suffix(R::Len::BYTE + len.into());
                }
                edge::Child::Value(value) => {
                    // Prefix precondition implies search key must match
                    validate!(
                        self.reader
                            .match_exact(edge.meta())
                            .is_some_and(|len| self.reader.len() == len.into())
                    );
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

    /// Traverse to the edge associated with the key.
    ///
    /// Returns `None` if there is no such edge,
    /// `Some(Err(Frozen))` if this edge is frozen,
    /// or `Some(Ok(updated))` otherwise.
    pub(crate) fn traverse_update(&mut self) -> Option<Result<Update<R::Edge>, Frozen>> {
        loop {
            let edge = self.edge().load_packed(Ordering::Acquire);

            match edge.child()? {
                edge::Child::Node(node) => {
                    let len = self.reader.match_exact(edge.meta())?;
                    // SAFETY: prefix precondition implies search key cannot equal node prefix
                    let byte = unsafe { self.reader.get_byte_unchecked(len) };

                    let next = unsafe { node.get(byte) }?;
                    self.push(len, node, next);
                    continue;
                }
                edge::Child::Value(value) => {
                    // Prefix precondition implies search key must match
                    validate!(
                        self.reader
                            .match_exact(edge.meta())
                            .is_some_and(|len| self.reader.len() == len.into())
                    );

                    return Some({
                        if edge.meta().is_frozen() {
                            Err(Frozen)
                        } else {
                            Ok(Update { value, edge })
                        }
                    });
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
                    validate!(
                        self.reader
                            .match_exact(edge.meta())
                            .is_some_and(|len| self.reader.len() == len.into())
                    );

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
    pub(crate) fn create_path(
        &self,
        old: ribbit::Packed<Edge<R::Edge>>,
        value: u64,
    ) -> Result<
        (
            ribbit::Packed<Edge<R::Edge>>,
            Option<NonNull<Atomic<Edge<R::Edge>>>>,
        ),
        Frozen,
    > {
        let meta = old.meta();
        if meta.is_frozen() {
            return Err(Frozen);
        }

        let len = self.reader.match_prefix(meta).into();

        let new = match meta.try_expand(len) {
            None => Edge::new_path(self.reader, value),
            Some((parent, old_byte, old_child)) => {
                let new_byte = unsafe { self.reader.get_byte_unchecked(len) };
                let (new_child, _) =
                    Edge::new_path(self.reader.suffix(R::Len::BYTE + len.into()), value);

                // NOTE: must put new allocation first because
                // `deallocate_recursive` recurses on first edge
                let (head, tail) = Node3::new_expand(
                    parent,
                    [new_byte, old_byte],
                    [new_child, old.with_meta(old_child)],
                );

                (head, Some(tail))
            }
        };

        Ok(new)
    }

    /// Freeze and replace the node containing `self.edge`.
    ///
    /// Returns `Err(_)` if the path does not support popping,
    /// `Ok(Some(node))` if this thread successfully replaced `node`,
    /// or `Ok(None)` if this thread did not replace the node (e.g.,
    /// another thread won the CAS race or an edge expansion SMO pushed
    /// down the frozen node).
    #[cold]
    pub(crate) fn freeze(&mut self) -> Result<Option<ribbit::Packed<node::Ptr>>, P::PopError> {
        let mut node = self.pop()?.expect("Root edge cannot be frozen");
        let mut edge = self.edge().load_packed(Ordering::Acquire);
        let mut pop = 1;

        let old = loop {
            while edge.meta().is_frozen() {
                node = self.pop()?.expect("Root edge cannot be frozen");
                edge = self.edge().load_packed(Ordering::Acquire);
                pop += 1;
            }

            let meta = edge.meta();

            let old = match edge.child() {
                Some(edge::Child::Node(old)) if old == node => old,
                // Child has changed since we last traversed
                // Optimistically assume that node replacement was completed by a different thread
                None | Some(_) => break None,
            };

            let (op, new) = unsafe {
                node.freeze::<R::Edge>();
                node.replace(meta)
            };

            match self.edge().compare_exchange_packed(
                edge,
                new,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    break Some(old);
                }
                Err(conflict) => {
                    if op.is_allocate() {
                        let node = new.as_node().expect("Allocating SMO creates node");
                        unsafe {
                            stat::increment(stat::Counter::FreeConflict);
                            node.deallocate();
                        }
                    }
                    edge = conflict;
                }
            };
        };

        stat::record(stat::Record::FreezePop, pop);
        Ok(old)
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
    pub(crate) fn pop(&mut self) -> Result<Option<ribbit::Packed<node::Ptr>>, P::PopError> {
        let Some(segment) = self.path.pop()? else {
            return Ok(None);
        };
        self.len -= R::Len::BYTE + segment.len.into();
        self.reader = segment.reader;
        self.edge = segment.edge;
        Ok(Some(segment.node))
    }

    #[inline]
    pub(crate) fn trim(&mut self, len: R::Len) {
        self.path.trim(len);
        self.reader = self.reader.prefix(self.reader.len() - len);
    }
}
