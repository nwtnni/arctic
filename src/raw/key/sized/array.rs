//! Support for byte array keys (`[u8; N]`).

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::boxed_slice;

impl<const N: usize> Key for [u8; N] {
    type Read<'k> = Reader<'k, N>;
    type Write = Writer<N>;
    type Borrowed = [u8; N];
    type Insert<'k> = &'k Self;
    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self
    }

    #[inline]
    fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        Reader::from(insert)
    }

    #[inline]
    fn insert_to_key<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k,
    {
        *insert
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> &'k Self::Borrowed
    where
        Self: 'k,
    {
        &writer.0
    }
}

impl<const N: usize> key::Split for [u8; N] {
    // NOTE: should split into ([u8; N - 1], u8) ideally, but can't
    // use const generic expressions yet
    fn split_last<'k>(key: &'k Self::Borrowed) -> (Self::Read<'k>, u8) {
        const {
            assert!(N > 0);
        }

        let (last, slice) = key.split_last().expect("Non-empty");
        (Reader(boxed_slice::Reader::new_prefix(slice)), *last)
    }
}

impl<'k, const N: usize> From<&'k [u8]> for Reader<'k, N> {
    #[inline]
    fn from(prefix: &'k [u8]) -> Self {
        Reader(boxed_slice::Reader::from(prefix))
    }
}

impl<'k, const N: usize> From<&'k str> for Reader<'k, N> {
    #[inline]
    fn from(prefix: &'k str) -> Self {
        Self::from(prefix.as_bytes())
    }
}

impl<'k, const N: usize, const M: usize> From<&'k [u8; N]> for Reader<'k, M> {
    #[inline]
    fn from(prefix: &'k [u8; N]) -> Self {
        Self::from(prefix.as_slice())
    }
}

#[doc(hidden)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Reader<'k, const N: usize>(pub(crate) boxed_slice::Reader<'k, ()>);

impl<'k, const N: usize> key::Read for Reader<'k, N> {
    const LEN: Option<Self::Len> = Some(Byte(N));
    type Edge = edge::Le;
    type Len = Byte;

    fn len(&self) -> Self::Len {
        self.0.len()
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        self.0.get_edge(len)
    }

    fn get_byte(&self, index: <ribbit::Packed<Self::Edge> as edge::Meta>::Len) -> Option<u8> {
        self.0.get_byte(index.bytes())
    }

    fn match_prefix(&self, meta: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        self.0.match_prefix(meta)
    }

    fn prefix(self, end: Self::Len) -> Self {
        Self(self.0.prefix(end))
    }

    fn suffix(self, start: Self::Len) -> Self {
        Self(self.0.suffix(start))
    }

    fn common_prefix(self, other: Self) -> Self {
        Self(self.0.common_prefix(other.0))
    }
}

#[doc(hidden)]
#[repr(transparent)]
#[derive(Debug)]
pub struct Writer<const N: usize>(pub(super) [u8; N]);

impl<const N: usize> Default for Writer<N> {
    #[inline]
    fn default() -> Self {
        Self([0; N])
    }
}

impl<'k, const N: usize> key::Write<Reader<'k, N>> for Writer<N> {
    type Len = Byte;

    #[inline]
    fn new(prefix: Reader<'k, N>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        let mut buffer = [0u8; N];
        buffer[..prefix.0.len().bytes()].copy_from_slice(prefix.0.slice);
        buffer[prefix.0.len().bytes()..]
            .iter_mut()
            .zip(key)
            .for_each(|(out, r#in)| {
                *out = r#in;
            });
        (Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Le>) -> Self::Len {
        self.0[start.bytes()] = node;
        self.0[start.bytes() + 1..]
            .iter_mut()
            .zip(edge)
            .for_each(|(out, r#in)| {
                *out = r#in;
            });
        start + Byte::BYTE + edge.len().into()
    }
}
