use crate::Key;
use crate::NonNullSlice;
use crate::NullTerminatedSlice;
use crate::NullTerminatedVec;
use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::slice::Writer;

impl<'a> Key for &'a NullTerminatedSlice {
    type Borrowed = NullTerminatedSlice;
    type Insert<'k>
        = Self
    where
        Self: 'k;
    type Read<'k> = Reader<'k>;
    type Write = Writer<()>;
    type Edge = edge::Slice<()>;
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
        unsafe { NullTerminatedSlice::new_unchecked(writer.as_slice_unchecked()) }
    }

    fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as Key>::Read<'k>, u8) {
        let (reader, last) = Reader::from(key).0.split_second_last().expect("Non-empty");
        (key::slice::Reader(reader), last)
    }
}

pub type Reader<'k> = key::slice::Reader<'k, ()>;

impl<'k> From<&'k NullTerminatedVec> for Reader<'k> {
    #[inline]
    fn from(key: &'k NullTerminatedVec) -> Self {
        Self::from(key.as_null_terminated_slice())
    }
}

impl<'k> From<&'k NullTerminatedSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NullTerminatedSlice) -> Self {
        Self(key::vec::Reader {
            slice: key,
            terminate: (),
        })
    }
}
