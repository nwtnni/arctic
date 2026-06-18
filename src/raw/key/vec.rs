//! Support for owned dynamically sized keys ([`Vec<u8>`]).

use ribbit::u6;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::Terminate;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Reader<'k, T> {
    pub(crate) slice: &'k [u8],
    pub(super) terminate: T,
}

impl<'k, T: Default> Reader<'k, T> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties.
    #[inline]
    pub fn new_prefix(prefix: &'k [u8]) -> Self {
        Self {
            slice: prefix,
            terminate: T::default(),
        }
    }
}

#[expect(private_bounds)]
impl<'k, T: Terminate> Reader<'k, T> {
    pub(super) fn split_last(&self) -> Option<(Self, u8)> {
        let (byte, slice) = self.slice.split_last()?;
        Some((
            Self {
                slice,
                terminate: self.terminate,
            },
            *byte,
        ))
    }

    pub(super) fn get_byte(&self, index: usize) -> Option<u8> {
        if let Some(byte) = self.slice.get(index) {
            return Some(*byte);
        }

        (self.terminate.get() && index == self.slice.len()).then_some(0)
    }
}

impl<'k> Reader<'k, ()> {
    pub(super) fn split_second_last(&self) -> Option<(Reader<'k, bool>, u8)> {
        let (slice, last) = self.slice.split_last_chunk::<2>()?;
        validate_eq!(last[1], 0);
        Some((
            Reader {
                slice,
                terminate: true,
            },
            last[0],
        ))
    }
}

impl<'k> Reader<'k, bool> {}

impl<T: Default> Default for Reader<'_, T> {
    #[inline]
    fn default() -> Self {
        Self::new_prefix(&[])
    }
}

impl<T: Terminate> key::Read for Reader<'_, T> {
    const LEN: Option<Self::Len> = None;
    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn len(&self) -> Self::Len {
        Byte(self.slice.len() + self.terminate.get() as usize)
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(key::read_u64(self.slice), len)
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        self.get_byte(index.bytes())
    }

    #[inline]
    fn match_exact(
        &self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<ribbit::Packed<Self::Edge> as edge::Meta>::Len> {
        // Avoid bit <-> byte conversion
        let len_edge = edge.len();
        let len_match = (edge.raw() ^ key::read_u64(self.slice)).trailing_zeros() as u8;
        (len_match >= len_edge.value()).then_some(len_edge)
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        Byte(((edge.raw() ^ key::read_u64(self.slice)).trailing_zeros() as usize) >> 3)
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        validate!(end <= self.len());
        let end = end.bytes();

        Self {
            slice: self.slice.get(..end).unwrap_or(self.slice),
            terminate: T::new(self.terminate.get() && (end > self.slice.len())),
        }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());
        let start = start.bytes();

        Self {
            // NOTE: slice key implementation requires us to preserve the
            // `self.slice` pointer, even if the slice is empty.
            slice: self.slice.get(start..).unwrap_or(&self.slice[..0]),
            terminate: T::new(self.terminate.get() && (start <= self.slice.len())),
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = key::common_prefix(self.slice, other.slice);

        Self {
            slice: &self.slice[..index],
            terminate: T::new(
                self.terminate.get()
                    && other.terminate.get()
                    && index == self.slice.len()
                    && index == other.slice.len(),
            ),
        }
    }
}

#[doc(hidden)]
#[repr(transparent)]
#[derive(Debug, Default)]
pub struct Writer(pub(super) Vec<u8>);

impl<'k, T: Terminate> key::Write<Reader<'k, T>> for Writer {
    type Len = Byte;

    #[inline]
    fn new(prefix: Reader<'k, T>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(prefix.slice);
        if prefix.terminate.get() {
            buffer.push(u8::MIN);
            validate_eq!(key.len().bits(), 0);
        } else {
            buffer.extend(key);
        }
        (Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Le>) -> Self::Len {
        validate!(start.0 <= self.0.len());
        self.0.truncate(start.0);
        self.0.push(node);
        self.0.extend(edge);
        Byte(self.0.len())
    }
}
