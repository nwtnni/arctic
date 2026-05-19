use core::convert::Infallible;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::key;
use crate::raw::node;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<R: key::Read> {
    /// Key before matching on `edge`
    pub(super) reader: R,

    /// Edge to match
    pub(super) edge: NonNull<Atomic<Edge<R::Edge>>>,

    /// Number of bytes matched along `edge`
    pub(super) len: <ribbit::Packed<R::Edge> as edge::Meta>::Len,

    /// Node underneath `edge`
    pub(super) node: ribbit::Packed<node::Ptr<R::Edge>>,
}

pub(crate) trait Path<R>: Default
where
    R: key::Read,
{
    type PopError;

    fn trim(&mut self, len: R::Len);

    fn push(&mut self, segment: Segment<R>);
    fn pop(&mut self) -> Result<Option<Segment<R>>, Self::PopError>;
}

#[derive(Default)]
pub(crate) struct Discard;

impl<R> Path<R> for Discard
where
    R: key::Read,
{
    type PopError = ();

    #[inline]
    fn trim(&mut self, _: R::Len) {}

    #[inline]
    fn push(&mut self, _segment: Segment<R>) {}

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<R>>, Self::PopError> {
        Err(())
    }
}

pub(crate) struct Retain<R: key::Read>(Vec<Segment<R>>);

impl<R> Path<R> for Retain<R>
where
    R: key::Read,
{
    type PopError = Infallible;

    #[inline]
    fn trim(&mut self, len: R::Len) {
        self.0
            .iter_mut()
            .for_each(|segment| segment.reader = segment.reader.prefix(segment.reader.len() - len))
    }

    #[inline]
    fn push(&mut self, segment: Segment<R>) {
        self.0.push(segment);
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<R>>, Self::PopError> {
        Ok(self.0.pop())
    }
}

impl<R: key::Read> Default for Retain<R> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
