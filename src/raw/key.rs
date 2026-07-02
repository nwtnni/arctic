//! Abstraction over various key types.
//!
//! Unlike comparison-based maps like [`BTreeMap`][std::collections::BTreeMap],
//! which accept keys implementing [`PartialOrd`], or hash-based maps
//! like [`HashMap`][std::collections::HashMap], which accept keys implementing
//! [`Hash`][core::hash::Hash] and [`Eq`], the maps and sets in this crate are
//! backed by a radix tree, which accepts keys represented by a sequence of bytes.
//!
//! We abstract over such keys with the [`Key`] trait. Our implementation additionally
//! requires keys to satisfy the **prefix property**: no key is a prefix of another
//! key[^1]. Fixed-size key types like integers (`u8`-`u128`) and arrays (`[u8; N]`)
//! naturally satisfy the prefix property, but dynamically-sized key types require
//! additional infrastructure.
//!
//! We establish type safety by providing wrappers for `Box<[u8]>` ([`BoxedSlice`])
//! and `[u8]` ([`Slice`]) that are parameterized by an [`Invariant`]: currently,
//! this can be either [`NonNull`] or [`Terminated`], which is sufficient to
//! guarantee the prefix property.
//!
//! [^1]: Internally, this prevents one key prefix from mapping to both a node and a value.

mod discard;
mod len;
mod sized;
mod r#unsized;

pub(crate) use discard::Discard;
pub(crate) use len::Bit;
pub(crate) use len::Byte;
pub(crate) use len::Len;
#[cfg_attr(not(feature = "smr-hazard"), expect(unused))]
pub(crate) use sized::unsigned;
pub(crate) use r#unsized::Terminate;
pub(crate) use r#unsized::boxed_slice;
#[cfg_attr(not(feature = "smr-hazard"), expect(unused))]
pub(crate) use r#unsized::slice;

pub use r#unsized::Invariant;
pub use r#unsized::NonNull;
pub use r#unsized::Terminated;
pub use r#unsized::boxed_slice::BoxedSlice;
pub use r#unsized::slice::Slice;

/// Convenience type alias for a [`Slice`] that is backed by a [`str`].
pub type Str<I> = Slice<I, str>;
/// Convenience type alias for a [`BoxedSlice`] that is backed by a [`str`].
pub type BoxedStr<I> = BoxedSlice<I, str>;

use core::borrow::Borrow;
use core::fmt;
use core::fmt::Debug;

use crate::raw::edge;
use crate::raw::edge::Meta as _;

/// Byte sequence that can be stored in an adaptive radix tree.
///
/// Must satisfy the prefix property: no key is a prefix of any
/// other key. Fixed-size keys (e.g., [`u64`], `[u8; N]`) trivially
/// satisfy this property, but dynamically sized keys (slices, boxed slices)
/// require some additional [`Invariant`][unsized::Invariant]s.
///
/// The following table depicts the most relevant key properties
/// for users of this crate. Methods that can insert into the tree
/// take `Insert<'_>`; other methods take `&'_ Borrowed`.
/// Using the [`Iterator`] API may be expensive for dynamically
/// allocated key types, as they need to be constructed and cloned
/// during traversal; see [`crate::sequential::Map`] for workarounds.
///
/// | Key Family  | Example                                   | Insert<'_>                          | Borrowed                        | Clone in iterator? |
/// |-------------|-------------------------------------------|-------------------------------------|---------------------------------|--------------------|
/// | Integer     | u64                                       | u64                                 | u64                             | N                  |
/// | Array       | [u8; 5]                                   | `&'_ [u8; 5]`                       | [u8; 5]                         | Y                  |
/// | Slice       | [`&'a Slice<NonNull>`][Slice]             | [`&'a Slice<NonNull>`][Slice]       | [`Slice<NonNull>`][Slice]       | N                  |
/// | Boxed Slice | [`BoxedStr<Terminated<b'\n'>>`][BoxedStr] | [`&'_ Str<Terminated<b'\n'>>`][Str] | [`Str<Terminated<b'\n'>>`][Str] | Y                  |
pub trait Key: Borrow<Self::Borrowed> {
    /// A non-allocated byte sequence that a key can be cheaply borrowed as.
    type Borrowed: 'static + ?Sized + Debug;

    /// Keys can either have edges that store inline bytes (e.g., u64, [`BoxedSlice`]),
    /// or pointers (i.e., [`Slice`]).
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

/// Key types that can split off their last byte,
/// which enables an efficient set implementation.
pub trait Split: Key {
    /// Split a key into a reader and the last byte.
    fn split_last<'k>(key: &'k Self::Borrowed) -> (Self::Read<'k>, u8);
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
