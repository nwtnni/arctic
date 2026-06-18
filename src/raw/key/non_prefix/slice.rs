use crate::Key;
use crate::NonPrefixSlice;
use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::slice::Writer;

pub type Reader<'k> = key::slice::Reader<'k, ()>;

impl<'a> Key for &'a NonPrefixSlice {
    type Borrowed = NonPrefixSlice;
    type Insert<'k>
        = Self
    where
        Self: 'k;
    type Read<'k> = Reader<'k>;
    type Write = Writer<()>;
    type Edge = edge::Slice<()>;
    type Len = Byte;
    type Split = &'a NonPrefixSlice;

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
        unsafe { NonPrefixSlice::new_unchecked(writer.as_slice_unchecked()) }
    }

    fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as Key>::Read<'k>, u8) {
        let (reader, last) = Reader::from(key).0.split_last().expect("Non-empty");
        (key::slice::Reader(reader), last)
    }
}

impl<'k> From<&'k NonPrefixSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        Self(key::vec::Reader {
            slice: key,
            terminate: (),
        })
    }
}
