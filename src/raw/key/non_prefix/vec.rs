use crate::Key;
use crate::NonPrefixSlice;
use crate::NonPrefixVec;
use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::vec::Writer;

pub type Reader<'k> = key::vec::Reader<'k, ()>;

impl Key for NonPrefixVec {
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Borrowed = NonPrefixSlice;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Byte;
    type Split = NonPrefixVec;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self.as_non_prefix_slice()
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
        insert.to_non_prefix_vec()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        unsafe { NonPrefixSlice::new_unchecked(&writer.0) }
    }

    fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as Key>::Read<'k>, u8) {
        todo!()
    }
}

impl<'k> From<&'k NonPrefixVec> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonPrefixVec) -> Self {
        Self::from(key.as_non_prefix_slice())
    }
}

impl<'k> From<&'k NonPrefixSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        Self {
            slice: key,
            terminate: (),
        }
    }
}
