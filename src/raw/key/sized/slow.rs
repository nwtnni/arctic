//! Benchmarking baseline for integer keys.

use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::len::Bit;
use crate::raw::key::len::Byte;

#[cfg(feature = "opt-no-int")]
impl crate::raw::Key for u64 {
    type Read<'k> = Reader;
    type Write = key::sized::array::Writer<8>;
    type Borrowed = Self;
    type Edge = edge::Le;
    type Len = Byte<8>;

    type Insert<'k> = Self;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        *self
    }

    #[inline]
    fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        Reader::from(insert)
    }

    fn insert_to_key<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k,
    {
        insert
    }

    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        Self::from_be_bytes(writer.0)
    }
}

#[cfg(feature = "opt-no-int")]
impl crate::key::Split for u64 {
    #[inline]
    fn split_last<'k>(key: &'k Self::Borrowed) -> (Self::Read<'k>, u8) {
        let reader = Reader::from(key);
        (
            Reader {
                buffer: reader.buffer,
                len: Byte::new_const::<7>(),
            },
            reader.buffer[7],
        )
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader {
    pub(crate) buffer: [u8; 8],
    len: Byte<8>,
}

impl Reader {
    #[inline]
    fn new(key: &[u8], len: Byte<8>) -> Self {
        Self {
            buffer: core::array::from_fn(|i| key.get(i).copied().unwrap_or(0)),
            len,
        }
    }
}

impl key::Read for Reader {
    const LEN: Option<Self::Len> = Some(Byte::new_const::<8>());

    type Edge = edge::Le;
    type Len = Byte<8>;

    #[inline]
    fn len(&self) -> Self::Len {
        self.len
    }

    #[inline]
    fn get_edge(&self, len: <Self::Edge as edge::Meta>::Len) -> Self::Edge {
        let len = len.min(self.len.into());
        edge::Le::new(u64::from_le_bytes(self.buffer), len)
    }

    #[inline]
    fn get_byte(&self, index: <Self::Edge as edge::Meta>::Len) -> Option<u8> {
        let index = index.bytes();
        if index < self.len.bytes() {
            self.buffer.get(index).copied()
        } else {
            None
        }
    }

    #[inline]
    fn match_prefix(&self, edge: Self::Edge) -> <Self::Edge as edge::Meta>::Len {
        Bit::from(Byte::new(
            self.buffer
                .into_iter()
                .zip(edge)
                .take(self.len.bytes())
                .position(|(l, r)| l != r)
                .unwrap_or(self.len.bytes()),
        ))
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        let mut buffer = [0u8; 8];
        buffer[..end.bytes()].copy_from_slice(&self.buffer[..end.bytes()]);
        Self { buffer, len: end }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        let mut buffer = [0u8; 8];
        let len = self.len - start;
        buffer[..len.bytes()].copy_from_slice(&self.buffer[start.bytes()..][..len.bytes()]);
        Self { buffer, len }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let len = self.len.min(other.len);
        let len_prefix = self.buffer[..len.bytes()]
            .iter()
            .zip(&other.buffer[..len.bytes()])
            .position(|(l, r)| l != r)
            .map(|len| unsafe { Byte::new_unchecked(len) })
            .unwrap_or(len);
        let mut buffer = [0u8; 8];
        buffer[..len_prefix.bytes()].copy_from_slice(&self.buffer[..len_prefix.bytes()]);
        Self {
            buffer,
            len: len_prefix,
        }
    }
}

impl From<u64> for Reader {
    #[inline]
    fn from(key: u64) -> Self {
        Reader::from(&key.to_be_bytes())
    }
}

impl<'k> From<&'k u64> for Reader {
    #[inline]
    fn from(key: &'k u64) -> Self {
        Reader::from(*key)
    }
}

impl<'k> From<&'k [u8]> for Reader {
    #[inline]
    fn from(key: &'k [u8]) -> Self {
        Reader::new(key, Byte::new_clamped(key.len()))
    }
}

impl<'k> From<&'k str> for Reader {
    #[inline]
    fn from(key: &'k str) -> Self {
        Reader::from(key.as_bytes())
    }
}

impl<'k, const N: usize> From<&'k [u8; N]> for Reader {
    #[inline]
    fn from(key: &'k [u8; N]) -> Self {
        Reader::from(key.as_slice())
    }
}

impl key::Write<Reader> for key::sized::array::Writer<8> {
    type Len = Byte<8>;

    #[inline]
    fn new(prefix: Reader, key: edge::Le) -> (Self, Self::Len) {
        let len = prefix.len + key.len().into();
        let mut buffer = [0u8; 8];
        buffer[..prefix.len.bytes()].copy_from_slice(&prefix.buffer[..prefix.len.bytes()]);
        buffer[prefix.len.bytes()..]
            .iter_mut()
            .zip(key)
            .for_each(|(out, r#in)| {
                *out = r#in;
            });
        (key::sized::array::Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: edge::Le) -> Self::Len {
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
