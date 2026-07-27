use core::convert::Infallible;
use core::fmt::Debug;
use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::key;
use crate::raw::node;
use crate::sync::Atomic;

/// A path along the tree is composed of 0 or more path segments.
pub(crate) struct Segment<R: key::Read> {
    /// Key before matching on `edge`
    pub(super) reader: R,

    /// Edge to match
    pub(super) edge: NonNull<Atomic<Edge<R::Edge>>>,

    /// Number of bytes matched along `edge`
    pub(super) len: <ribbit::Packed<R::Edge> as edge::Meta>::Len,

    /// Node underneath `edge`
    pub(super) node: ribbit::Packed<node::Ptr>,
}

pub(crate) trait Path<R>: Default
where
    R: key::Read,
{
    type PopError: Debug;

    #[inline]
    fn trim(&mut self, _len: R::Len) {}

    #[inline]
    fn len(&self) -> R::Len {
        <R::Len as key::Len>::ZERO
    }

    fn push(&mut self, segment: Segment<R>) -> R;
    fn pop(&mut self) -> Result<Option<Segment<R>>, Self::PopError>;
}

/// Discard all path information.
pub(crate) struct Discard<R>(PhantomData<R>);

impl<R> Path<R> for Discard<R>
where
    R: key::Read,
{
    type PopError = ();

    #[inline]
    fn push(&mut self, segment: Segment<R>) -> R {
        segment
            .reader
            .suffix(<R::Len as key::Len>::BYTE + segment.len.into())
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<R>>, Self::PopError> {
        Err(())
    }
}

impl<R> Default for Discard<R> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

/// Retain only length information.
pub(crate) struct Len<R: key::Read>(R::Len);

impl<R> Path<R> for Len<R>
where
    R: key::Read,
{
    type PopError = ();

    #[inline]
    fn push(&mut self, segment: Segment<R>) -> R {
        let delta = <R::Len as key::Len>::BYTE + segment.len.into();
        self.0 += delta;
        segment.reader.suffix(delta)
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<R>>, Self::PopError> {
        Err(())
    }

    #[inline]
    fn len(&self) -> <R as key::Read>::Len {
        self.0
    }
}

impl<R: key::Read> Default for Len<R> {
    fn default() -> Self {
        Self(<R::Len as key::Len>::ZERO)
    }
}

/// HACK: cursor length information is only used (a) in scans
/// after the initial `traverse_prefix`, and (b) in point operations
/// when retiring with hazard keys (to determine the prefix when retiring a node).
///
/// Conservatively assume that if the `smr-hazard` feature is enabled,
/// then hazard keys are being used. If not, it's still safe to track
/// the length.
#[cfg(feature = "smr-hazard")]
pub(crate) type Point<R> = Len<R>;
#[cfg(not(feature = "smr-hazard"))]
pub(crate) type Point<R> = Discard<R>;

/// Retain full path (and length, if hazard keys are enabled).
pub(crate) struct Full<R: key::Read> {
    #[cfg(feature = "smr-hazard")]
    len: R::Len,
    path: Vec<Segment<R>>,
}

impl<R> Path<R> for Full<R>
where
    R: key::Read,
{
    type PopError = Infallible;

    #[inline]
    fn trim(&mut self, len: R::Len) {
        self.path.iter_mut().for_each(|segment| {
            validate!(segment.reader.len() >= len);
            segment.reader = segment.reader.prefix(segment.reader.len() - len)
        })
    }

    #[inline]
    fn push(&mut self, segment: Segment<R>) -> R {
        let delta = <R::Len as key::Len>::BYTE + segment.len.into();
        #[cfg(feature = "smr-hazard")]
        {
            self.len += delta;
        }
        self.path.push_mut(segment).reader.suffix(delta)
    }

    #[inline]
    fn pop(&mut self) -> Result<Option<Segment<R>>, Self::PopError> {
        let Some(segment) = self.path.pop() else {
            return Ok(None);
        };

        #[cfg(feature = "smr-hazard")]
        {
            self.len -= <R::Len as key::Len>::BYTE + segment.len.into();
        }

        Ok(Some(segment))
    }

    #[cfg(feature = "smr-hazard")]
    #[inline]
    fn len(&self) -> <R as key::Read>::Len {
        self.len
    }
}

impl<R: key::Read> Default for Full<R> {
    fn default() -> Self {
        Self {
            #[cfg(feature = "smr-hazard")]
            len: <R::Len as key::Len>::ZERO,
            path: Vec::new(),
        }
    }
}
