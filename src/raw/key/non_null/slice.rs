use crate::Key;
use crate::NonNullSlice;
use crate::NonNullVec;
use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::slice::Writer;

impl<'a> Key for &'a NonNullSlice {
    type Borrowed = NonNullSlice;
    type Insert<'k>
        = Self
    where
        Self: 'k;
    type Read<'k> = Reader<'k>;
    type Write = Writer<bool>;
    type Edge = edge::Slice<bool>;
    type Len = Byte;
    type Split = &'a NonNullSlice;

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
        unsafe { NonNullSlice::new_unchecked(writer.as_slice_unchecked()) }
    }

    fn split_last<'k>(key: &'k Self::Borrowed) -> (Self::Read<'k>, u8) {
        todo!()
    }
}

pub type Reader<'k> = crate::raw::key::slice::Reader<'k, bool>;

impl<'k> From<&'k NonNullVec> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullVec) -> Self {
        Self::from(key.as_non_null_slice())
    }
}

impl<'k> From<&'k NonNullSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullSlice) -> Self {
        Self(key::vec::Reader {
            slice: key,
            terminate: true,
        })
    }
}
