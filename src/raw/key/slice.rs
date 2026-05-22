use ribbit::u14;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::edge::Slice;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::vec::NonPrefixSlice;

impl Key for &'_ NonPrefixSlice {
    type Borrowed = NonPrefixSlice;
    type Insert<'k>
        = Self
    where
        Self: 'k;
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Edge = edge::Slice;
    type Len = key::vec::Len;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        *self
    }

    #[inline]
    fn borrow_insert<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        Reader::from(insert)
    }

    fn clone_insert<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k,
    {
        insert
    }

    unsafe fn borrow_writer_unchecked<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        let len = writer.len.bytes();
        let suffix = unsafe { writer.last.as_slice() };
        unsafe {
            NonPrefixSlice::new_unchecked(core::slice::from_raw_parts(
                suffix.as_ptr().byte_sub(len - suffix.len()),
                len,
            ))
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader<'k>(pub(crate) &'k [u8]);

impl<'k> From<&'k NonPrefixSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        Self(key)
    }
}

impl key::Read for Reader<'_> {
    const LEN: Option<key::vec::Len> = None;

    type Edge = edge::Slice;
    type Len = key::vec::Len;

    fn len(&self) -> Self::Len {
        key::vec::Len(self.0.len())
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = len.bytes().min(self.len().bytes());
        Slice::new(&self.0[..len])
    }

    fn get_byte(&self, index: <ribbit::Packed<Self::Edge> as edge::Meta>::Len) -> Option<u8> {
        self.0.get(index.bytes()).copied()
    }

    fn match_prefix(&self, meta: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        key::vec::Len(
            core::iter::zip(self.0, unsafe { meta.as_slice() })
                .position(|(l, r)| l != r)
                .unwrap_or_else(|| unsafe { self.0.len().min(meta.as_slice().len()) }),
        )
    }

    #[inline]
    fn prefix(self, len: key::vec::Len) -> Self {
        validate!(self.len() >= len);
        Reader(&self.0[..len.bytes()])
    }

    #[inline]
    fn suffix(self, len: key::vec::Len) -> Self {
        validate!(self.len() >= len);
        Reader(&self.0[len.bytes()..])
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = core::iter::zip(self.0, other.0)
            .position(|(l, r)| l != r)
            .unwrap_or_else(|| self.0.len().min(other.0.len()));
        Self(&self.0[..index])
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
        let len_match = self.match_prefix(edge);
        if len_match >= edge.len().into() {
            return Err(());
        }
        let edge = unsafe { edge.as_slice() };

        let len_start = u14::new(len_match.0 as u16);
        let len_middle = len_start + u14::new(1);

        let start = edge::Slice::new(&edge[..len_start.bytes()]);
        let old_middle = edge[len_start.bytes()];
        let new_middle = self.0[len_start.bytes()];
        let end = edge::Slice::new(&edge[len_middle.bytes()..][..edge.len() - len_middle.bytes()]);

        Ok((start, old_middle, new_middle, end))
    }
}

#[derive(Clone, Default, Debug)]
pub struct Writer {
    last: ribbit::Packed<edge::Slice>,
    len: key::vec::Len,
}

impl key::Write<Reader<'_>> for Writer {
    type Len = key::vec::Len;

    fn new(prefix: Reader, key: ribbit::Packed<edge::Slice>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        (Writer { last: key, len }, len)
    }

    fn replace(&mut self, start: Self::Len, _: u8, edge: ribbit::Packed<edge::Slice>) -> Self::Len {
        validate!(start <= self.len);
        self.len = start + key::vec::Len::BYTE + edge.len().into();
        self.last = edge;
        self.len
    }
}

impl From<u14> for key::vec::Len {
    fn from(value: u14) -> Self {
        Self(value.value() as usize)
    }
}
