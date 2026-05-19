use core::cmp::Ordering;
use core::fmt;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use ribbit::u6;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

impl Key for Vec<u8> {
    type Read<'k> = Reader<'k, { usize::MAX }>;
    type Write = Writer;
    type Borrowed = [u8];
    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
        Vec::from(borrow)
    }

    #[inline]
    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed {
        let (last, key) = writer.0.split_last().expect("Vec has terminator");
        validate_eq!(*last, TERMINATOR[0]);
        key
    }

    #[inline]
    unsafe fn from_writer_unchecked(mut writer: Self::Write) -> Self {
        let last = writer.0.pop().expect("Vec has terminator");
        validate_eq!(last, TERMINATOR[0]);
        writer.0
    }
}

impl Key for String {
    type Read<'k> = Reader<'k, { usize::MAX }>;
    type Write = Writer;
    type Borrowed = str;
    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn clone_from_borrow(borrow: &Self::Borrowed) -> Self {
        String::from(borrow)
    }

    #[inline]
    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed {
        let (last, key) = writer.0.split_last().expect("Vec has terminator");
        validate_eq!(*last, TERMINATOR[0]);

        if_validate!(core::str::from_utf8(key).unwrap(), unsafe {
            core::str::from_utf8_unchecked(key)
        })
    }

    #[inline]
    unsafe fn from_writer_unchecked(mut writer: Self::Write) -> Self {
        let last = writer.0.pop().expect("Vec has terminator");
        validate_eq!(last, TERMINATOR[0]);

        if_validate!(String::from_utf8(writer.0).unwrap(), unsafe {
            String::from_utf8_unchecked(writer.0)
        })
    }
}

// https://github.com/surrealdb/vart/issues/13
static TERMINATOR: &[u8] = &[0];

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Reader<'k, const N: usize> {
    pub(crate) slice: &'k [u8],
    pub(crate) terminate: bool,
}

impl<'k, const N: usize> Reader<'k, N> {
    #[inline]
    fn next_u64(&self) -> u64 {
        if self.slice.len() >= 8 {
            return unsafe { self.slice.as_ptr().cast::<u64>().read_unaligned() };
        }

        // FIXME: try to avoid memcpy?
        // https://github.com/llvm/llvm-project/issues/87440
        // https://github.com/rust-lang/rust/issues/92993
        // https://github.com/rust-lang/rust/pull/37573
        let mut buffer = [0u8; 8];
        buffer[..self.slice.len()].copy_from_slice(self.slice);
        buffer[self.slice.len()] = if self.terminate { TERMINATOR[0] } else { 0 };

        u64::from_le_bytes(buffer)
    }
}

impl<'k> From<&'k [u8]> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k [u8]) -> Self {
        Self {
            slice: key,
            terminate: true,
        }
    }
}

impl<'k> From<&'k Vec<u8>> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k Vec<u8>) -> Self {
        Self::from(key.as_slice())
    }
}

impl<'k> From<&'k str> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k str) -> Self {
        Self::from(key.as_bytes())
    }
}

impl<'k> From<&'k String> for Reader<'k, { usize::MAX }> {
    #[inline]
    fn from(key: &'k String) -> Self {
        Self::from(key.as_str())
    }
}

impl<const N: usize> Default for Reader<'_, N> {
    #[inline]
    fn default() -> Self {
        Self {
            slice: &[],
            terminate: false,
        }
    }
}

impl<const N: usize> key::Read for Reader<'_, N> {
    const LEN: Option<Self::Len> = if N == usize::MAX { None } else { Some(Len(N)) };

    type Edge = edge::Le;
    type Len = Len;

    #[inline]
    fn len(&self) -> Self::Len {
        Len(self.slice.len() + self.terminate as usize)
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(self.next_u64(), len)
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        let index = index.bytes();

        if let Some(byte) = self.slice.get(index) {
            return Some(*byte);
        }

        if self.terminate && index == self.slice.len() {
            Some(TERMINATOR[0])
        } else {
            None
        }
    }

    #[inline]
    fn match_exact(
        &self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<ribbit::Packed<Self::Edge> as edge::Meta>::Len> {
        // Avoid bit <-> byte conversion
        let len_edge = edge.len();
        let len_match = (edge.raw() ^ self.next_u64()).trailing_zeros() as u8;
        (len_match >= len_edge.value()).then_some(len_edge)
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        Len(((edge.raw() ^ self.next_u64()).trailing_zeros() as usize) >> 3)
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        let end = end.bytes();

        if end <= self.slice.len() {
            Self {
                slice: &self.slice[..end],
                terminate: false,
            }
        } else {
            Self {
                slice: &TERMINATOR[..end - self.slice.len()],
                terminate: false,
            }
        }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());
        let start = start.bytes();

        if start <= self.slice.len() {
            Self {
                slice: &self.slice[start..],
                terminate: self.terminate,
            }
        } else {
            Self {
                slice: &TERMINATOR[start - self.slice.len()..],
                terminate: false,
            }
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        match core::iter::zip(self.slice, other.slice).position(|(l, r)| *l != *r) {
            Some(index) => Self {
                slice: &self.slice[..index],
                terminate: false,
            },
            None => match self.slice.len().cmp(&other.slice.len()) {
                Ordering::Less => Self {
                    slice: self.slice,
                    terminate: false,
                },
                Ordering::Equal => Self {
                    slice: self.slice,
                    terminate: self.terminate && other.terminate,
                },
                Ordering::Greater => Self {
                    slice: other.slice,
                    terminate: false,
                },
            },
        }
    }

    fn expand(
        &self,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Result<
        (
            ribbit::Packed<Self::Edge>,
            u8,
            u8,
            ribbit::Packed<Self::Edge>,
        ),
        (),
    > {
        let buffer = self.next_u64();

        let len_match = (edge.raw() ^ buffer).trailing_zeros() as u8;
        if len_match >= edge.len().value() {
            return Err(());
        }

        validate!(self.len().bits() > len_match as usize);

        let len_start = u6::new(len_match & !0b111);
        let len_middle = len_start + const { u6::new(8) };

        let start = edge::Le::new(edge.raw(), len_start);
        let old_middle = (edge.raw() >> len_start.value()) as u8;
        let new_middle = (buffer >> len_start.value()) as u8;
        let end = edge::Le::new(edge.raw() >> len_middle.value(), edge.len() - len_middle);

        Ok((start, old_middle, new_middle, end))
    }
}

#[repr(transparent)]
#[derive(Default)]
pub struct Writer(pub(super) Vec<u8>);

impl<'k> key::Write<Reader<'k, { usize::MAX }>> for Writer {
    type Len = Len;

    #[inline]
    fn new(prefix: Reader<'k, { usize::MAX }>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(prefix.slice);
        if prefix.terminate {
            buffer.push(TERMINATOR[0]);
            validate_eq!(key.len().bits(), 0);
        }
        buffer.extend(key);
        (Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Le>) -> Self::Len {
        validate!(start.0 <= self.0.len());
        self.0.truncate(start.0);
        self.0.push(node);
        self.0.extend(edge);
        Len(self.0.len())
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Len(pub(super) usize);

impl key::Len<u6> for Len {
    const ZERO: Self = Self(0);
    const BYTE: Self = Self(1);

    #[inline]
    fn bits(self) -> usize {
        self.0 << 3
    }

    #[inline]
    fn bytes(self) -> usize {
        self.0
    }
}

impl From<u6> for Len {
    #[inline]
    fn from(len: u6) -> Self {
        Self((len.value() >> 3) as usize)
    }
}

impl Add for Len {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Len {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Len {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl SubAssign for Len {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}
