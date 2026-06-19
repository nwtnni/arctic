//! Support for borrowed dynamically sized `&[u8]` keys.

use core::fmt::Debug;
use core::marker::PhantomData;
use core::num::NonZeroUsize;

use ribbit::u13;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::r#unsized;
use crate::raw::key::r#unsized::Terminate;
use crate::raw::key::r#unsized::owned::BoxedSlice;

/// # Safety
///
/// Implementer must guarantee that `Raw` is unsized
/// and repr(transparent) with `[u8]`.
pub unsafe trait Raw: 'static + AsRef<[u8]> + Debug {
    #[expect(clippy::wrong_self_convention)]
    fn into_boxed(&self) -> Box<Self>;
}
unsafe impl Raw for [u8] {
    #[inline]
    fn into_boxed(&self) -> Box<Self> {
        Box::from(self)
    }
}
unsafe impl Raw for str {
    #[inline]
    fn into_boxed(&self) -> Box<Self> {
        Box::from(self)
    }
}

#[repr(transparent)]
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Slice<I, R: ?Sized = [u8]> {
    invariant: PhantomData<I>,
    raw: R,
}

impl<I, R> Slice<I, R>
where
    I: r#unsized::Invariant,
    R: ?Sized + Raw,
{
    pub fn new(key: &R) -> Result<&Self, I::Error> {
        I::validate(key.as_ref())?;
        // Invariants checked above
        Ok(unsafe { Self::new_unchecked(key) })
    }

    #[inline]
    pub fn len(&self) -> NonZeroUsize {
        let len = self.raw.as_ref().len();
        if_validate!(NonZeroUsize::new(len).unwrap(), unsafe {
            NonZeroUsize::new_unchecked(len)
        })
    }
}

impl<I, R: ?Sized> Slice<I, R> {
    /// # Safety
    ///
    /// Caller must ensure `key` upholds invariants, i.e., `I::validate(key)` would return `Ok`.
    #[inline]
    pub const unsafe fn new_unchecked(key: &R) -> &Self {
        unsafe { core::mem::transmute::<&R, &Self>(key) }
    }

    /// Get a reference to the underlying buffer.
    #[inline]
    pub const fn as_raw(&self) -> &R {
        &self.raw
    }
}

impl<I, R: ?Sized> AsRef<R> for Slice<I, R> {
    #[inline]
    fn as_ref(&self) -> &R {
        self.as_raw()
    }
}

impl<I, R> ToOwned for Slice<I, R>
where
    R: ?Sized + Raw,
{
    type Owned = BoxedSlice<I, R>;
    fn to_owned(&self) -> Self::Owned {
        unsafe { BoxedSlice::new_unchecked(self.as_raw().into_boxed()) }
    }
}

impl<'a, I, R> crate::Key for &'a Slice<I, R>
where
    I: r#unsized::Invariant,
    R: ?Sized + Raw,
{
    type Borrowed = Slice<I, R>;

    type Insert<'k>
        = &'a Slice<I, R>
    where
        Self: 'k;

    type Read<'k> = Reader<'k, I::Terminate>;
    type Write = Writer<I>;
    type Edge = edge::Slice<I::Terminate>;
    type Len = Byte;
    // type Split = &'a Slice<I::Split>;

    fn as_insert(&self) -> Self::Insert<'_> {
        self
    }

    fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k,
    {
        Self::Read::from(insert)
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
        unsafe { writer.as_slice_unchecked() }
    }

    // fn split_last<'k>(key: &'k Self::Borrowed) -> (<Self::Split as crate::Key>::Read<'k>, u8) {
    //     r#unsized::owned::Reader::from(key)
    //         .split()
    //         .map(|(reader, last)| (Reader(reader), last))
    //         .expect("Non-empty")
    // }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Reader<'k, T>(pub(crate) r#unsized::owned::Reader<'k, T>);

impl<'k, I, R> From<&'k Slice<I, R>> for Reader<'k, I::Terminate>
where
    I: r#unsized::Invariant,
    R: ?Sized + Raw,
{
    #[inline]
    fn from(key: &'k Slice<I, R>) -> Self {
        Self(r#unsized::owned::Reader::from(key))
    }
}

impl<'k, T: Default> Reader<'k, T> {
    #[inline]
    pub fn new_prefix(prefix: &'k [u8]) -> Self {
        Self(r#unsized::owned::Reader::new_prefix(prefix))
    }
}

impl<T: Terminate> key::Read for Reader<'_, T> {
    const LEN: Option<Byte> = None;

    type Edge = edge::Slice<T>;
    type Len = Byte;

    fn len(&self) -> Self::Len {
        self.0.len()
    }

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let min = len.bytes().min(self.0.slice.len());
        edge::Slice::new(&self.0.slice[..min]).with_terminate(T::new(
            self.0.terminate.get() && len.bytes() == self.0.slice.len() + 1,
        ))
    }

    fn get_byte(&self, index: u13) -> Option<u8> {
        self.0.get_byte(index.bytes())
    }

    fn match_prefix(&self, meta: ribbit::Packed<edge::Slice<T>>) -> Self::Len {
        let other = unsafe { meta.as_slice() };

        let index = r#unsized::common_prefix(self.0.slice, other);
        let terminate = self.0.terminate.get()
            && index == self.0.slice.len()
            && index == other.len()
            && meta.terminate().get();

        Byte(index + terminate as usize)
    }

    #[inline]
    fn prefix(self, end: Byte) -> Self {
        Self(self.0.prefix(end))
    }

    #[inline]
    fn suffix(self, start: Byte) -> Self {
        Self(self.0.suffix(start))
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        Self(self.0.common_prefix(other.0))
    }
}

#[doc(hidden)]
#[derive(Clone, Default, Debug)]
pub struct Writer<I: r#unsized::Invariant> {
    last: ribbit::Packed<edge::Slice<I::Terminate>>,
    len: Byte,
}

impl<I: r#unsized::Invariant> Writer<I> {
    unsafe fn as_slice_unchecked<'a, R: ?Sized>(&self) -> &'a Slice<I, R> {
        let len = self.len.bytes();
        let suffix = unsafe { self.last.as_slice() };
        let raw = I::Terminate::trim(unsafe {
            core::slice::from_raw_parts(suffix.as_ptr().byte_sub(len - suffix.len()), len)
        });
        unsafe { Slice::<I, R>::new_unchecked(core::mem::transmute_copy::<&[u8], &R>(&raw)) }
    }
}

impl<I: r#unsized::Invariant> key::Write<Reader<'_, I::Terminate>> for Writer<I> {
    type Len = Byte;

    fn new(
        prefix: Reader<'_, I::Terminate>,
        key: ribbit::Packed<edge::Slice<I::Terminate>>,
    ) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        (Writer { last: key, len }, len)
    }

    fn replace(
        &mut self,
        start: Self::Len,
        _: u8,
        edge: ribbit::Packed<edge::Slice<I::Terminate>>,
    ) -> Self::Len {
        validate!(start <= self.len);
        self.len = start + Byte::BYTE + edge.len().into();
        self.last = edge;
        self.len
    }
}
