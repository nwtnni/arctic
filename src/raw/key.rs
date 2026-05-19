pub mod array;
mod discard;
pub mod int;
pub mod slow;
pub mod vec;

pub(crate) use discard::Discard;

use core::borrow::Borrow;
use core::fmt;
use core::ops::Add;
use core::ops::AddAssign;
use core::ops::Sub;
use core::ops::SubAssign;

use crate::raw::edge;
use crate::raw::edge::Meta as _;

pub trait Key: Borrow<Self::Borrowed> {
    type Borrowed: 'static + ?Sized;

    #[expect(private_bounds)]
    type Read<'k>: Read<Edge = Self::Edge, Len = Self::Len> + From<&'k Self::Borrowed>;

    #[expect(private_bounds)]
    type Write: for<'k> Write<Self::Read<'k>>;

    #[expect(private_bounds)]
    type Edge: ribbit::Pack<Packed: edge::Meta>;

    #[expect(private_bounds)]
    type Len: Len<<ribbit::Packed<Self::Edge> as edge::Meta>::Len>;

    unsafe fn borrow_writer_unchecked(writer: &Self::Write) -> &Self::Borrowed;

    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self;

    fn clone_from_borrow(borrowed: &Self::Borrowed) -> Self;
}

pub(crate) trait Read: Copy + fmt::Debug + Default + Eq {
    // Hint for fixed-size keys
    const LEN: Option<Self::Len>;

    type Edge: ribbit::Pack<Packed: edge::Meta>;
    type Len: Len<<ribbit::Packed<Self::Edge> as edge::Meta>::Len>;

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
    >;

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

#[expect(private_bounds)]
pub trait Len<L: edge::Len>:
    Sized
    + Copy
    + AddAssign
    + Add<Output = Self>
    + SubAssign
    + Sub<Output = Self>
    + PartialOrd
    + From<L>
    + fmt::Debug
{
    const ZERO: Self;
    const BYTE: Self;

    fn bits(self) -> usize;
    fn bytes(self) -> usize;
}

// #[cfg(test)]
// mod tests {
//     use crate::raw::Key;
//     use crate::raw::edge;
//     use crate::raw::edge::Len as _;
//     use crate::raw::key::Len as _;
//     use crate::raw::key::Read as _;
//
//     pub(super) fn take_all<K: Key>(array: &[u8], key: &K::Borrowed, lens: &[usize]) {
//         let mut reader = K::Read::from(key);
//         let mut index = 0;
//         let mut actual = Vec::<()>::new();
//
//         for len in lens
//             .iter()
//             .copied()
//             .map(|bytes| bytes << 3)
//             .map(<<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Len::new)
//         {
//             assert_eq!(reader.len().bytes(), array.len() - index);
//
//             let bytes = len.bits() >> 3;
//
//             actual.clear();
//             todo!();
//             // actual.extend(reader.read(len));
//             // assert_eq!(actual, &array[index..][..bytes]);
//
//             index += bytes;
//         }
//
//         todo!()
//         // assert_eq!(reader.len().bytes(), array.len() - index);
//         // assert_eq!(reader.get(), array.get(index).copied());
//     }
// }
