//! Support for borrowed dynamically sized `&[u8]` keys.

use ribbit::u14;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::edge::Slice;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::Terminate;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Reader<'k, T>(pub(crate) key::vec::Reader<'k, T>);

impl<'k, T: Default> Reader<'k, T> {
    #[inline]
    pub fn new_prefix(prefix: &'k [u8]) -> Self {
        Self(key::vec::Reader::new_prefix(prefix))
    }
}

impl<T: Terminate> key::Read for Reader<'_, T> {
    const LEN: Option<Byte> = None;

    type Edge = edge::Slice;
    type Len = Byte;

    fn len(&self) -> Self::Len {
        self.0.len()
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = len.bytes().min(self.len().bytes());
        Slice::new(&self.0.slice[..len])
    }

    fn get_byte(&self, index: u14) -> Option<u8> {
        self.0.get_byte(index.bytes())
    }

    fn match_prefix(&self, meta: ribbit::Packed<edge::Slice>) -> Self::Len {
        let other = unsafe { meta.as_slice() };
        Byte(key::common_prefix(self.0.slice, other))
    }

    #[inline]
    fn prefix(self, end: Byte) -> Self {
        Self(self.0.prefix(end))
    }

    #[inline]
    fn suffix(self, start: Byte) -> Self {
        Self(self.0.suffix(start))
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        Self(self.0.common_prefix(other.0))
    }
}

#[doc(hidden)]
#[derive(Clone, Default, Debug)]
pub struct Writer {
    last: ribbit::Packed<edge::Slice>,
    len: Byte,
}

impl Writer {
    pub(super) unsafe fn as_slice_unchecked<'a>(&self) -> &'a [u8] {
        let len = self.len.bytes();
        let suffix = unsafe { self.last.as_slice() };
        unsafe { core::slice::from_raw_parts(suffix.as_ptr().byte_sub(len - suffix.len()), len) }
    }
}

impl<T: Terminate> key::Write<Reader<'_, T>> for Writer {
    type Len = Byte;

    fn new(prefix: Reader<'_, T>, key: ribbit::Packed<edge::Slice>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        (Writer { last: key, len }, len)
    }

    fn replace(&mut self, start: Self::Len, _: u8, edge: ribbit::Packed<edge::Slice>) -> Self::Len {
        validate!(start <= self.len);
        self.len = start + Byte::BYTE + edge.len().into();
        self.last = edge;
        self.len
    }
}
