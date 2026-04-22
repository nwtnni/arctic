use core::ptr::NonNull;

use ribbit::u14;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;

impl<'a> Key for &'a [u8] {
    type Borrow<'k> = &'k [u8];
    type BorrowInsert<'k> = &'a [u8];

    type Read<'k> = Reader;
    type Write = Writer;

    type Edge = edge::Slice;

    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    fn borrow_insert(&self) -> Self::BorrowInsert<'_> {
        *self
    }

    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        &writer.0
    }

    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        todo!()
    }

    fn clone_from_borrow<'k>(borrow: Self::Borrow<'k>) -> Self {
        todo!()
    }

    fn len(borrow: Self::Borrow<'_>) -> usize {
        borrow.len()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader(NonNull<[u8]>);

impl Reader {
    pub(crate) unsafe fn as_ref(&self) -> &[u8] {
        unsafe { self.0.as_ref() }
    }
}

impl<'k> From<&'k [u8]> for Reader {
    #[inline]
    fn from(key: &'k [u8]) -> Self {
        Self(NonNull::from(key))
    }
}

impl Default for Reader {
    #[inline]
    fn default() -> Self {
        Self(NonNull::from(&[]))
    }
}

impl key::Read for Reader {
    const BITS: Option<usize> = None;

    type Edge = edge::Slice;

    #[inline]
    fn bits(&self) -> usize {
        unsafe { self.0.as_ref() }.len() << 3
    }

    #[inline]
    fn next(&mut self) -> Option<u8> {
        let reader = unsafe { self.0.as_ref() };
        let (head, tail) = reader.split_first()?;
        self.0 = NonNull::from(tail);
        Some(*head)
    }

    #[inline]
    fn read(
        &mut self,
        len: <<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
    ) -> ribbit::Packed<edge::Slice> {
        if len == u14::new(0) {
            return ribbit::Packed::<edge::Slice>::DEFAULT;
        }

        let reader = unsafe { self.0.as_ref() };
        let len = edge::Slice::min_len(len, reader.len());
        let edge = edge::Slice::new(reader, len);

        self.0 = NonNull::from(&reader[len.value() as usize..]);
        edge
    }

    #[inline]
    fn trim(&mut self, bits: usize) {
        let reader = unsafe { self.0.as_ref() };
        self.0 = NonNull::from(&reader[..reader.len() - (bits >> 3)]);
    }

    #[inline]
    fn prefix(self, bits: usize) -> Self {
        validate!(self.bits() >= bits);
        Reader(NonNull::from(unsafe { &self.0.as_ref()[..bits >> 3] }))
    }

    #[inline]
    fn suffix(self, bits: usize) -> Self {
        validate!(self.bits() >= bits);
        Reader(NonNull::from(unsafe { &self.0.as_ref()[bits >> 3..] }))
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = core::iter::zip(unsafe { self.0.as_ref() }, unsafe { other.0.as_ref() })
            .position(|(l, r)| l != r)
            .unwrap_or_else(|| self.0.len().min(other.0.len()));
        Self(NonNull::from(unsafe { &self.0.as_ref()[..index] }))
    }
}

impl From<Reader> for Writer {
    fn from(reader: Reader) -> Self {
        Writer(unsafe { reader.0.as_ref().to_vec() })
    }
}

#[derive(Clone, Default, Debug)]
pub struct Writer(Vec<u8>);

impl key::Write for Writer {
    type Len = usize;
    type Edge = edge::Slice;

    fn len_from_bits(bits: usize) -> Self::Len {
        bits >> 3
    }

    fn write(&mut self, start: Self::Len, edge: ribbit::Packed<Self::Edge>) -> Self::Len {
        validate_eq!(self.0.len(), start);
        self.0.extend(unsafe { edge.as_slice() });
        self.0.len()
    }

    fn replace(
        &mut self,
        start: Self::Len,
        node: u8,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Self::Len {
        validate!(start <= self.0.len());
        self.0.truncate(start);
        self.0.push(node);
        self.0.extend(unsafe { edge.as_slice() });
        self.0.len()
    }
}
