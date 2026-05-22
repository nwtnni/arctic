use core::ptr::NonNull;

use ribbit::u14;

use crate::raw::Key;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;

/// Newtype guaranteeing this slice is not a prefix of
/// any other [`NonPrefixVec`] or [`NonPrefixSlice`].
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NonPrefixSlice([u8]);

impl NonPrefixSlice {
    /// # Safety
    ///
    /// Caller must guarantee that `slice` is not a prefix of any
    /// other [`NonPrefixVec`] or [`NonPrefixSlice`].
    #[inline]
    pub const unsafe fn new_unchecked(slice: &[u8]) -> &Self {
        // SAFETY: `NonPrefixSlice` is `repr(transparent)`
        unsafe { core::mem::transmute(slice) }
    }
}

impl core::ops::Deref for NonPrefixSlice {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> From<&'a NonPrefixSlice> for &'a [u8] {
    #[inline]
    fn from(str: &'a NonPrefixSlice) -> Self {
        // SAFETY: `NonPrefixSlice` is `repr(transparent)`
        unsafe { core::mem::transmute(str) }
    }
}

impl<'a> Key for &'a NonPrefixSlice {
    type Borrowed = NonPrefixSlice;
    type Insert<'k> = Self;
    type Read<'k> = Reader;
    type Write = Writer;
    type Edge = edge::Slice;
    type Len = key::vec::Len;

    #[inline]
    fn borrow_insert(&self) -> Self::Insert<'_> {
        *self
    }

    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed {
        todo!()
    }

    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        todo!()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reader(NonNull<[u8]>);

impl Reader {
    pub(crate) unsafe fn as_ref(&self) -> &[u8] {
        unsafe { self.0.as_ref() }
    }
}

impl<'k> From<&'k NonPrefixSlice> for Reader {
    #[inline]
    fn from(key: &'k NonPrefixSlice) -> Self {
        todo!()
    }
}

impl Default for Reader {
    #[inline]
    fn default() -> Self {
        Self(NonNull::from(&[]))
    }
}

impl key::Read for Reader {
    const LEN: Option<key::vec::Len> = None;

    type Edge = edge::Slice;
    type Len = key::vec::Len;

    // #[inline]
    // fn bits(&self) -> usize {
    //     unsafe { self.0.as_ref() }.len() << 3
    // }
    //
    // #[inline]
    // fn next(&mut self) -> Option<u8> {
    //     let reader = unsafe { self.0.as_ref() };
    //     let (head, tail) = reader.split_first()?;
    //     self.0 = NonNull::from(tail);
    //     Some(*head)
    // }
    //
    // #[inline]
    // fn read(
    //     &mut self,
    //     len: <<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
    // ) -> ribbit::Packed<edge::Slice> {
    //     if len == u14::new(0) {
    //         return ribbit::Packed::<edge::Slice>::DEFAULT;
    //     }
    //
    //     let reader = unsafe { self.0.as_ref() };
    //     let len = edge::Slice::min_len(len, reader.len());
    //     let edge = edge::Slice::new(reader, len);
    //
    //     self.0 = NonNull::from(&reader[len.value() as usize..]);
    //     edge
    // }

    fn len(&self) -> Self::Len {
        todo!()
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        todo!()
    }

    fn get_byte(&self, index: <ribbit::Packed<Self::Edge> as edge::Meta>::Len) -> Option<u8> {
        todo!()
    }

    fn match_prefix(&self, meta: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        todo!()
    }
    #[inline]
    fn prefix(self, bits: key::vec::Len) -> Self {
        todo!()
        // validate!(self.bits() >= bits);
        // Reader(NonNull::from(unsafe { &self.0.as_ref()[..bits >> 3] }))
    }

    #[inline]
    fn suffix(self, bits: key::vec::Len) -> Self {
        todo!()
        // validate!(self.bits() >= bits);
        // Reader(NonNull::from(unsafe { &self.0.as_ref()[bits >> 3..] }))
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = core::iter::zip(unsafe { self.0.as_ref() }, unsafe { other.0.as_ref() })
            .position(|(l, r)| l != r)
            .unwrap_or_else(|| self.0.len().min(other.0.len()));
        Self(NonNull::from(unsafe { &self.0.as_ref()[..index] }))
    }

    fn expand(
        &self,
        key: ribbit::Packed<Self::Edge>,
    ) -> Result<
        (
            ribbit::Packed<Self::Edge>,
            u8,
            u8,
            ribbit::Packed<Self::Edge>,
        ),
        (),
    > {
        todo!()
    }
}

#[derive(Clone, Default, Debug)]
pub struct Writer {
    last: ribbit::Packed<edge::Slice>,
    len: key::vec::Len,
}

impl key::Write<Reader> for Writer {
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
        todo!()
    }
}
