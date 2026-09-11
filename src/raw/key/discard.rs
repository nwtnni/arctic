use core::marker::PhantomData;

use crate::raw::key::Read;
use crate::raw::key::Write;

pub(crate) struct Discard<R>(PhantomData<R>);

impl<R> Default for Discard<R> {
    #[inline]
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<R> core::fmt::Debug for Discard<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Discard")
    }
}

impl<R: Read> Write<R> for Discard<R> {
    type Len = ();

    fn new(_: R, _: R::Edge) -> (Self, Self::Len) {
        (Self(PhantomData), ())
    }

    #[inline]
    fn replace(&mut self, _: Self::Len, _: u8, _: R::Edge) -> Self::Len {}
}
