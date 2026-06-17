//! Support for byte array keys (`[u8; N]`).

use core::fmt;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl<const N: usize> Key for [u8; N] {
    type Read<'k> = Reader<'k, N>;
    type Write = Writer<N>;
    type Borrowed = [u8; N];
    type Insert<'k> = &'k Self;
    type Edge = edge::Le;
    type Len = Byte;
    type Split = [u8; N];

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

    fn split_last<'k>(key: &'k Self::Borrowed) -> (Self::Read<'k>, u8) {
        const {
            assert!(N > 0);
        }

        todo!()
    }
}

impl<'k, const N: usize> From<&'k [u8; N]> for Reader<'k, N> {
    #[inline]
    fn from(array: &'k [u8; N]) -> Self {
        Self(key::vec::Reader {
            slice: array,
            terminate: (),
        })
    }
}

/// Key reader that can represent byte prefixes of arrays.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Reader<'k, const N: usize>(pub(crate) key::vec::Reader<'k, ()>);

impl<'k, const N: usize> Reader<'k, N> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties:
    /// it may be empty, or be a prefix of another key.
    #[inline]
    pub fn new_prefix(prefix: &'k [u8]) -> Self {
        Self(key::vec::Reader::new_prefix(prefix))
    }
}

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

impl<const N: usize> fmt::Debug for Writer<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
