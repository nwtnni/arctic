use core::borrow::Borrow as _;

use crate::NonNullVec;
use crate::NullTerminatedSlice;
use crate::NullTerminatedVec;
use crate::raw::Key;
use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::vec::Writer;

pub type Reader<'k> = key::vec::Reader<'k, ()>;

impl Key for NullTerminatedVec {
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Borrowed = NullTerminatedSlice;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Byte;
    type Split = NonNullVec;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self.borrow()
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
        insert.to_null_terminated_vec()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        if_validate!(NullTerminatedSlice::new(&writer.0).unwrap(), unsafe {
            NullTerminatedSlice::new_unchecked(&writer.0)
        })
    }

    fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as Key>::Read<'k>, u8) {
        Reader::from(key).split_second_last().expect("Non-empty")
    }
}

impl<'k> From<&'k NullTerminatedVec> for Reader<'k> {
    #[inline]
    fn from(key: &'k NullTerminatedVec) -> Self {
        Self::from(key.as_null_terminated_slice())
    }
}

impl<'k> From<&'k NullTerminatedSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NullTerminatedSlice) -> Self {
        Self {
            slice: key,
            terminate: (),
        }
    }
}
