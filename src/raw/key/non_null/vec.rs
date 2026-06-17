use crate::Key;
use crate::NonNullSlice;
use crate::NonNullVec;
use crate::raw::edge;
use crate::raw::key::Byte;
use crate::raw::key::vec::Writer;

pub type Reader<'k> = crate::raw::key::vec::Reader<'k, bool>;

impl Key for NonNullVec {
    type Read<'k> = Reader<'k>;
    type Write = Writer;
    type Borrowed = NonNullSlice;
    type Insert<'k> = &'k Self::Borrowed;
    type Edge = edge::Le;
    type Len = Byte;
    type Split = Self;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self.as_non_null_slice()
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
        insert.to_non_null_vec()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        let (last, key) = writer.0.split_last().expect("Implicit null terminator");
        validate_eq!(*last, 0);

        if_validate!(NonNullSlice::new(key).unwrap(), unsafe {
            NonNullSlice::new_unchecked(key)
        })
    }

    fn split_last<'k>(key: &'k Self::Borrowed) -> (Self::Read<'k>, u8) {
        todo!()
    }
}

impl<'k> From<&'k NonNullVec> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullVec) -> Self {
        Self::from(key.as_non_null_slice())
    }
}

impl<'k> From<&'k NonNullSlice> for Reader<'k> {
    #[inline]
    fn from(key: &'k NonNullSlice) -> Self {
        Self {
            slice: key.as_slice(),
            terminate: true,
        }
    }
}
