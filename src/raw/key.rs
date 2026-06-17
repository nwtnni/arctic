//! Implementations of [`Key`].

pub mod array;
mod discard;
pub mod int;
pub mod non_null;
pub mod null_terminated;
pub mod slice;
mod slow;
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

/// Lexicographically ordered byte sequence that can be stored in an adaptive radix tree.
///
/// Must satisfy two preconditions:
/// (1) keys are non-empty (for efficient set implementation),
/// and (2) no key is a prefix of any other key (for internal invariants).
///
/// The following table depicts the most relevant key properties
/// for users of this crate. Methods that can insert into the tree
/// take `Insert<'_>`; other methods take `&'_ Borrowed`.
/// Using the [`Iterator`] API may be expensive for dynamically
/// allocated key types, as they need to be constructed and cloned
/// during traversal; see [`crate::sequential::Map`] for workarounds.
///
/// | Key Family | Example                                       | Insert<'_>                                    | Borrowed                                  | Clone in iterator? |
/// |------------|-----------------------------------------------|-----------------------------------------------|-------------------------------------------|--------------------|
/// | Integer    | u64                                           | u64                                           | u64                                       | N                  |
/// | Array      | [u8; 5]                                       | `&'_ [u8; 5]`                                 | [u8; 5]                                   | Y                  |
/// | Slice      | [`&'a NonPrefixSlice`][crate::NonPrefixSlice] | [`&'a NonPrefixSlice`][crate::NonPrefixSlice] | [`NonPrefixSlice`][crate::NonPrefixSlice] | N                  |
/// | Vec        | [`NonNullString`][crate::NonNullString]       | [`&'_ NonNullStr`][crate::NonNullStr]         | [`NonNullStr`][crate::NonNullStr]         | Y                  |
pub trait Key: Borrow<Self::Borrowed> {
    /// A non-allocated byte sequence that a key can be cheaply borrowed as.
    type Borrowed: 'static + ?Sized + Debug;

    /// Keys can either have edges that store bytes inline (e.g., [`string::NonNullString`]),
    /// or as references (e.g., [`slice::NonPrefixSlice`]).
    ///
    /// The former can take borrowed bytes with any lifetime when inserting,
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
    type Edge: ribbit::Pack<Packed: edge::Meta> + Send + Sync;

    /// Key length.
    #[expect(private_bounds)]
    type Len: Len + From<<ribbit::Packed<Self::Edge> as edge::Meta>::Len>;

    /// Convert the key type to the insert type.
    fn as_insert(&self) -> Self::Insert<'_>;

    /// Convert the insert type to a reader with appropriate lifetime.
    fn insert_as_read<'k>(insert: Self::Insert<'k>) -> Self::Read<'k>
    where
        Self: 'k;

    /// Convert the insert type to the key type.
    fn insert_to_key<'k>(insert: Self::Insert<'k>) -> Self
    where
        Self: 'k;

    /// Convert a reference to a writer into the insert type.
    ///
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

    fn split_last(self) -> Option<(Self, u8)>;
}

pub(crate) trait Write<R: Read>: fmt::Debug + Default {
    type Len: Copy + fmt::Debug;

    fn new(prefix: R, key: ribbit::Packed<R::Edge>) -> (Self, Self::Len);

    /// Replace bytes starting at `start` with bytes from `node` and `edge`
    fn replace(&mut self, start: Self::Len, node: u8, edge: ribbit::Packed<R::Edge>) -> Self::Len;
}

/// Key length.
pub(crate) trait Len:
    Sized
    + Copy
    + AddAssign
    + Add<Output = Self>
    + SubAssign
    + Sub<Output = Self>
    + PartialOrd
    + fmt::Debug
{
    /// Length of an empty key.
    const ZERO: Self;

    /// Length of a key with a single byte.
    const BYTE: Self;

    /// Return the key length in bits.
    fn bits(self) -> usize;

    /// Return the key length in bytes.
    fn bytes(self) -> usize;
}

// TODO: optimize?
#[inline]
fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    core::iter::zip(left, right)
        .position(|(l, r)| l != r)
        .unwrap_or_else(|| left.len().min(right.len()))
}

// TODO: optimize?
#[inline]
fn read_u64(slice: &[u8]) -> u64 {
    if slice.len() >= 8 {
        return unsafe { slice.as_ptr().cast::<u64>().read_unaligned() };
    }

    // FIXME: try to avoid memcpy?
    // https://github.com/llvm/llvm-project/issues/87440
    // https://github.com/rust-lang/rust/issues/92993
    // https://github.com/rust-lang/rust/pull/37573
    let mut buffer = [0u8; 8];
    buffer[..slice.len()].copy_from_slice(slice);

    u64::from_le_bytes(buffer)
}
