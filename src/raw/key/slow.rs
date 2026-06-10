//! Benchmarking baseline for integer keys.

use ribbit::u6;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;

#[cfg(feature = "opt-no-int")]
impl crate::raw::Key for u64 {
    type Read<'k> = Reader;
    type Write = key::array::Writer<8>;
    type Borrowed = Self;
    type Edge = edge::Le;
    type Len = key::vec::Len;

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

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader {
    pub(crate) buffer: [u8; 8],
    len: key::vec::Len,
}

impl Reader {
    #[inline]
    unsafe fn new_unchecked(buffer: u64, bits: u8) -> Self {
        validate!(bits <= 64);
        validate_eq!(bits & 0b111, 0);
        validate_eq!(buffer & !u64::MAX.unbounded_shl(bits as u32), buffer);
        let buffer = buffer.to_be_bytes();
        Self {
            buffer,
            len: key::vec::Len((bits as usize) >> 3),
        }
    }
}

impl key::Read for Reader {
    const LEN: Option<Self::Len> = Some(key::vec::Len(8));

    type Edge = edge::Le;
    type Len = key::vec::Len;

    #[inline]
    fn len(&self) -> Self::Len {
        self.len
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(u64::from_le_bytes(self.buffer), u6::new(len.bits() as u8))
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        let index = index.bytes();
        if index < self.len.bytes() {
            self.buffer.get(index).copied()
        } else {
            None
        }
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> key::vec::Len {
        key::vec::Len(
            self.buffer
                .into_iter()
                .zip(edge)
                .take(self.len.bytes())
                .position(|(l, r)| l != r)
                .unwrap_or(self.len.bytes()),
        )
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
            .map(key::vec::Len)
            .unwrap_or(len);
        let mut buffer = [0u8; 8];
        buffer[..len_prefix.bytes()].copy_from_slice(&self.buffer[..len_prefix.bytes()]);
        Self {
            buffer,
            len: len_prefix,
        }
    }

    fn split_last(self) -> Option<(Self, u8)> {
        let len = self
            .len
            .0
            .checked_sub(Self::Len::BYTE.0)
            .map(key::vec::Len)?;

        Some((
            Self {
                buffer: self.buffer,
                len,
            },
            self.buffer[len.bytes()],
        ))
    }

    // fn expand(
    //     &self,
    //     edge: ribbit::Packed<Self::Edge>,
    // ) -> Result<
    //     (
    //         ribbit::Packed<Self::Edge>,
    //         u8,
    //         u8,
    //         ribbit::Packed<Self::Edge>,
    //     ),
    //     (),
    // > {
    //     let len_match = self.match_prefix(edge);
    //     if len_match >= edge.len().into() {
    //         return Err(());
    //     }
    //
    //     validate!(self.len > len_match);
    //     let len_start = u6::new(len_match.bits() as u8);
    //     let len_middle = len_start + const { u6::new(8) };
    //     let len_end = u6::new((edge.len().bits() - len_middle.bits()) as u8);
    //
    //     let edge = u64::to_le_bytes(edge.raw());
    //
    //     let mut start = [0u8; 8];
    //     start[..len_start.bytes()].copy_from_slice(&edge[..len_start.bytes()]);
    //     let start = edge::Le::new(u64::from_le_bytes(start), len_start);
    //
    //     let old_middle = edge[len_start.bytes()];
    //     let new_middle = self.buffer[len_start.bytes()];
    //
    //     let mut end = [0u8; 8];
    //     end[..len_end.bytes()].copy_from_slice(&edge[len_middle.bytes()..][..len_end.bytes()]);
    //     let end = edge::Le::new(u64::from_le_bytes(end), len_end);
    //
    //     Ok((start, old_middle, new_middle, end))
    // }
}

impl From<u64> for Reader {
    #[inline]
    fn from(key: u64) -> Self {
        unsafe { Reader::new_unchecked(key, 64) }
    }
}

impl<'k> From<&'k u64> for Reader {
    #[inline]
    fn from(key: &'k u64) -> Self {
        unsafe { Reader::new_unchecked(*key, 64) }
    }
}

impl key::Write<Reader> for key::array::Writer<8> {
    type Len = key::vec::Len;

    #[inline]
    fn new(prefix: Reader, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len + key.len().into();
        let mut buffer = [0u8; 8];
        buffer[..prefix.len.bytes()].copy_from_slice(&prefix.buffer[..prefix.len.bytes()]);
        buffer[prefix.len.bytes()..]
            .iter_mut()
            .zip(key)
            .for_each(|(out, r#in)| {
                *out = r#in;
            });
        (key::array::Writer(buffer), len)
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
