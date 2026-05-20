#![expect(rustdoc::private_intra_doc_links)]

//! This module contains:
//! - The structure of the tree ([`crate::raw::edge`], [`crate::raw::node`], [`crate::raw::key`])
//! - Range iteration over the tree ([`crate::raw::iter`])
//! - Point traversal over the tree ([`crate::raw::cursor`])
//!
//! This module is "raw" with respect to:
//! - Safe memory reclamation ([`crate::concurrent::smr`])
//! - Mutable vs. immutable access
//! - Value types ([`crate::sequential::Value`], [`crate::concurrent::Value`])
//!
//! The purpose of this module is to re-use as much code as possible between the
//! sequential ([`crate::sequential::Map`]) and concurrent ([`crate::concurrent::Map`])
//! tree implementations, and between instantiations of these trees with different
//! value types.

pub(crate) mod cursor;
pub(crate) mod edge;
mod int;
pub(crate) mod iter;
pub mod key;
pub(crate) mod map;
pub(crate) mod node;

pub(crate) use cursor::Cursor;
pub(crate) use edge::Edge;
pub use key::Key;
pub(crate) use map::Map;

pub(crate) use int::Int;

#[derive(Debug)]
pub(crate) struct Frozen;

/// Structural modification operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Smo {
    ReplaceNode,
    DeleteNode,
    CompressEdge,
}

impl Smo {
    #[inline]
    pub fn is_allocate(self) -> bool {
        matches!(self, Self::ReplaceNode)
    }
}
