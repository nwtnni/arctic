pub mod array;
mod discard;
pub mod int;
pub mod slice;
pub mod slow;
pub mod string;
pub mod vec;

pub(crate) use discard::Discard;

use core::borrow::Borrow;
use core::fmt;
use core::fmt::Debug;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use crate::raw::edge;
use crate::raw::edge::Meta as _;

/// Lexicographically ordered byte sequence that can be stored
/// in an adaptive radix tree. Must satisfy the precondition that
/// no key is a prefix of any other key.
pub trait Key: Borrow<Self::Borrowed> {
    /// A non-allocated byte sequence that a key can be cheaply borrowed as.
    type Borrowed: 'static + ?Sized + Debug;

    /// Keys can either have edges that store bytes inline (e.g., [`string::NonNullString`]),
    /// or as references (e.g., [`slice::NonPrefixSlice`]).
    ///
    /// The former can take any borrowed bytes with any lifetime when inserting,
    /// but the latter can only take borrowed bytes that outlive the key type.
    type Insert<'k>: Copy + Borrow<Self::Borrowed>
    where
        Self: 'k;

    /// Tracks key length and allows extracting edges and slicing key bytes.
    #[expect(private_bounds)]
    type Read<'k>: Read<Edge = Self::Edge, Len = Self::Len> + From<&'k Self::Borrowed>;

    /// Constructs a key from an initial reader prefix and sequence of bytes and edges.
    #[expect(private_bounds)]
    type Write: for<'k> Write<Self::Read<'k>>;

    /// Edge metadata.
    #[expect(private_bounds)]
    type Edge: ribbit::Pack<Packed: edge::Meta>;

    /// Key length.
    #[expect(private_bounds)]
    type Len: Len + From<<ribbit::Packed<Self::Edge> as edge::Meta>::Len>;

    /// The key type itself always has a long enough lifetime for insertion.
    fn as_insert(&self) -> Self::Insert<'_>;

    fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k;

    fn insert_to_key<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k;

    /// # Safety
    ///
    /// Caller must guarantee that `writer` contains a valid key.
    unsafe fn write_as_insert<'k>(writer: &'k Self::Write) -> Self::Insert<'k>
    where
        Self: 'k;
}

pub(crate) trait Read: Copy + fmt::Debug + Default + Eq {
    // Hint for fixed-size keys
    const LEN: Option<Self::Len>;

    type Edge: ribbit::Pack<Packed: edge::Meta>;
    type Len: Len
        + From<<ribbit::Packed<Self::Edge> as edge::Meta>::Len>
        + Into<<ribbit::Packed<Self::Edge> as edge::Meta>::Len>;

    fn len(&self) -> Self::Len;

    fn get_edge(
        &self,
        len: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> ribbit::Packed<Self::Edge>;

    fn get_byte(&self, index: <ribbit::Packed<Self::Edge> as edge::Meta>::Len) -> Option<u8>;

    #[inline]
    unsafe fn get_byte_unchecked(
        &self,
        index: <ribbit::Packed<Self::Edge> as edge::Meta>::Len,
    ) -> u8 {
        match self.get_byte(index) {
            Some(byte) => byte,
            None => if_validate!(unreachable!(), unsafe {
                core::hint::unreachable_unchecked()
            }),
        }
    }

    #[inline]
    fn match_exact(
        &self,
        meta: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<ribbit::Packed<Self::Edge> as edge::Meta>::Len> {
        let len = self.match_prefix(meta);
        (len >= meta.len().into()).then_some(meta.len())
    }

    fn match_prefix(&self, meta: <Self::Edge as ribbit::Pack>::Packed) -> Self::Len;

    fn prefix(self, end: Self::Len) -> Self;
    fn suffix(self, start: Self::Len) -> Self;
    fn common_prefix(self, other: Self) -> Self;
}

pub(crate) trait Write<R: Read>: fmt::Debug + Default {
    type Len: Copy + fmt::Debug;

    fn new(prefix: R, key: ribbit::Packed<R::Edge>) -> (Self, Self::Len);

    /// Replace bytes starting at `start` with bytes from `node` and `edge`
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<R::Edge>) -> Self::Len;
}

pub trait Len:
    Sized
    + Copy
    + AddAssign
    + Add<Output = Self>
    + SubAssign
    + Sub<Output = Self>
    + PartialOrd
    + fmt::Debug
{
    const ZERO: Self;
    const BYTE: Self;

    fn bits(self) -> usize;
    fn bytes(self) -> usize;
}
