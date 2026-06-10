//! Support for byte array keys (`[u8; N]`).

use core::fmt;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl<const N: usize> Key for [u8; N] {
    type Read<'k> = key::vec::Reader<'k, N>;
    type Write = Writer<N>;
    type Borrowed = [u8; N];
    type Insert<'k> = &'k Self;
    type Edge = edge::Le;
    type Len = key::vec::Len;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self
    }

    #[inline]
    fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        key::vec::Reader::from(insert)
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

impl<'k, const N: usize> From<&'k [u8; N]> for key::vec::Reader<'k, N> {
    #[inline]
    fn from(array: &'k [u8; N]) -> Self {
        key::vec::Reader(array.as_slice())
    }
}

#[repr(transparent)]
pub struct Writer<const N: usize>(pub(super) [u8; N]);

impl<const N: usize> Default for Writer<N> {
    #[inline]
    fn default() -> Self {
        Self([0; N])
    }
}

impl<'k, const N: usize> key::Write<key::vec::Reader<'k, N>> for Writer<N> {
    type Len = key::vec::Len;

    #[inline]
    fn new(prefix: key::vec::Reader<'k, N>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        let mut buffer = [0u8; N];
        buffer[..prefix.0.len()].copy_from_slice(prefix.0);
        buffer[prefix.0.len()..]
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
        start + key::vec::Len::BYTE + edge.len().into()
    }
}

impl<const N: usize> fmt::Debug for Writer<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
