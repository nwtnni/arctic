//! Support for owned dynamically sized keys ([`Vec<u8>`], [`Box<[u8]>`][Box]).

use core::borrow::Borrow;
use core::fmt::Debug;
use core::marker::PhantomData;
use core::ops::Deref;
use std::ffi::CString;

use ribbit::u6;

use crate::Key;
use crate::key::Terminated;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Byte;
use crate::raw::key::Len as _;
use crate::raw::key::Read as _;
use crate::raw::key::r#unsized;
use crate::raw::key::r#unsized::Terminate;
use crate::raw::key::r#unsized::slice::Slice;

#[repr(transparent)]
#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct BoxedSlice<I, R: ?Sized = [u8]> {
    invariant: PhantomData<I>,
    raw: Box<R>,
}

impl<I, R: ?Sized> Clone for BoxedSlice<I, R>
where
    Box<R>: Clone,
{
    #[inline]
    fn clone(&self) -> Self {
        Self {
            invariant: PhantomData,
            raw: self.raw.clone(),
        }
    }
}

impl<I, R> BoxedSlice<I, R>
where
    I: r#unsized::Invariant,
    R: ?Sized + r#unsized::slice::Raw,
{
    #[inline]
    pub fn new(raw: impl Into<Box<R>>) -> Result<Self, (Box<R>, I::Error)> {
        let raw = raw.into();
        match Slice::<I, R>::new(&raw) {
            Ok(_) => Ok(unsafe { Self::new_unchecked(raw) }),
            Err(error) => Err((raw, error)),
        }
    }
}

impl<I, R: ?Sized> BoxedSlice<I, R> {
    #[inline]
    pub const unsafe fn new_unchecked(raw: Box<R>) -> Self {
        Self {
            invariant: PhantomData,
            raw,
        }
    }

    /// Get a reference to the underlying slice.
    #[inline]
    pub const fn as_slice(&self) -> &Slice<I, R> {
        unsafe { Slice::new_unchecked(&self.raw) }
    }
}

impl<I, R: ?Sized> Deref for BoxedSlice<I, R> {
    type Target = Slice<I, R>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl<I, R: ?Sized> Borrow<Slice<I, R>> for BoxedSlice<I, R> {
    #[inline]
    fn borrow(&self) -> &Slice<I, R> {
        self.as_slice()
    }
}

impl<I, R: ?Sized> AsRef<Slice<I, R>> for BoxedSlice<I, R> {
    #[inline]
    fn as_ref(&self) -> &Slice<I, R> {
        self.as_slice()
    }
}

impl From<CString> for BoxedSlice<Terminated<0>, [u8]> {
    fn from(string: CString) -> Self {
        // SAFETY: `CString` is null terminated
        unsafe { Self::new_unchecked(string.into_bytes_with_nul().into_boxed_slice()) }
    }
}

impl<I, R> Key for BoxedSlice<I, R>
where
    I: r#unsized::Invariant,
    R: ?Sized + r#unsized::slice::Raw,
{
    type Read<'k> = Reader<'k, I::Terminate>;
    type Write = Writer;
    type Borrowed = Slice<I, R>;
    type Insert<'k> = &'k Slice<I, R>;
    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn as_insert(&self) -> Self::Insert<'_> {
        self.as_slice()
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
        insert.to_owned()
    }

    #[inline]
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k,
    {
        unsafe { writer.as_slice_unchecked() }
    }
}

impl<'k, I, R> From<&'k Slice<I, R>> for Reader<'k, I::Terminate>
where
    I: r#unsized::Invariant,
    R: ?Sized + r#unsized::slice::Raw,
{
    #[inline]
    fn from(slice: &'k Slice<I, R>) -> Self {
        Self {
            slice: slice.as_raw().as_ref(),
            terminate: I::Terminate::TRUE,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Reader<'k, T> {
    pub(crate) slice: &'k [u8],
    pub(super) terminate: T,
}

impl<'k, T: Default> Reader<'k, T> {
    /// Construct a [`Reader`] representing `prefix`, for use in scan operations.
    ///
    /// Note that `prefix` does not need to satisfy any particular properties.
    #[inline]
    pub fn new_prefix(prefix: &'k [u8]) -> Self {
        Self {
            slice: prefix,
            terminate: T::default(),
        }
    }
}

#[expect(private_bounds)]
impl<'k, T: Terminate> Reader<'k, T> {
    #[inline]
    pub(crate) fn get_byte(&self, index: usize) -> Option<u8> {
        if let Some(byte) = self.slice.get(index) {
            return Some(*byte);
        }

        (self.terminate.get() && index == self.slice.len()).then_some(0)
    }
}

impl<T: Default> Default for Reader<'_, T> {
    #[inline]
    fn default() -> Self {
        Self::new_prefix(&[])
    }
}

impl<T: Terminate> key::Read for Reader<'_, T> {
    const LEN: Option<Self::Len> = None;
    type Edge = edge::Le;
    type Len = Byte;

    #[inline]
    fn len(&self) -> Self::Len {
        Byte(self.slice.len() + self.terminate.get() as usize)
    }

    #[inline]
    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge> {
        let len = u6::new((self.len().bits()).min(len.bits()) as u8);
        edge::Le::new(r#unsized::read_u64(self.slice), len)
    }

    #[inline]
    fn get_byte(&self, index: u6) -> Option<u8> {
        self.get_byte(index.bytes())
    }

    #[inline]
    fn match_exact(
        &self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<ribbit::Packed<Self::Edge> as edge::Meta>::Len> {
        // Avoid bit <-> byte conversion
        let len_edge = edge.len();
        let len_match = (edge.raw() ^ r#unsized::read_u64(self.slice)).trailing_zeros() as u8;
        (len_match >= len_edge.value()).then_some(len_edge)
    }

    #[inline]
    fn match_prefix(&self, edge: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len {
        Byte(((edge.raw() ^ r#unsized::read_u64(self.slice)).trailing_zeros() as usize) >> 3)
    }

    #[inline]
    fn prefix(self, end: Self::Len) -> Self {
        validate!(end <= self.len());
        let end = end.bytes();

        Self {
            slice: self.slice.get(..end).unwrap_or(self.slice),
            terminate: T::new(self.terminate.get() && (end > self.slice.len())),
        }
    }

    #[inline]
    fn suffix(self, start: Self::Len) -> Self {
        validate!(start <= self.len());
        let start = start.bytes();

        Self {
            // NOTE: slice key implementation requires us to preserve the
            // `self.slice` pointer, even if the slice is empty.
            slice: self.slice.get(start..).unwrap_or(&self.slice[..0]),
            terminate: T::new(self.terminate.get() && (start <= self.slice.len())),
        }
    }

    #[inline]
    fn common_prefix(self, other: Self) -> Self {
        let index = r#unsized::common_prefix(self.slice, other.slice);

        Self {
            slice: &self.slice[..index],
            terminate: T::new(
                self.terminate.get()
                    && other.terminate.get()
                    && index == self.slice.len()
                    && index == other.slice.len(),
            ),
        }
    }
}

#[doc(hidden)]
#[repr(transparent)]
#[derive(Debug, Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    unsafe fn as_slice_unchecked<I: r#unsized::Invariant, R: ?Sized>(&self) -> &Slice<I, R> {
        let raw = I::Terminate::trim(self.0.as_slice());
        unsafe { Slice::<I, R>::new_unchecked(core::mem::transmute_copy::<&[u8], &R>(&raw)) }
    }
}

impl<'k, T: Terminate> key::Write<Reader<'k, T>> for Writer {
    type Len = Byte;

    #[inline]
    fn new(prefix: Reader<'k, T>, key: ribbit::Packed<edge::Le>) -> (Self, Self::Len) {
        let len = prefix.len() + key.len().into();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(prefix.slice);
        if prefix.terminate.get() {
            buffer.push(u8::MIN);
            validate_eq!(key.len().bits(), 0);
        } else {
            buffer.extend(key);
        }
        (Writer(buffer), len)
    }

    #[inline]
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<edge::Le>) -> Self::Len {
        validate!(start.0 <= self.0.len());
        self.0.truncate(start.0);
        self.0.push(node);
        self.0.extend(edge);
        Byte(self.0.len())
    }
}
