pub mod dynamic;
pub mod integer;
pub mod slice;

use core::fmt;
use core::marker::PhantomData;

use crate::raw::edge;

pub trait Key {
    type Borrow<'k>: Copy;
    type BorrowInsert<'k>: Copy;

    #[expect(private_bounds)]
    type Read<'k>: Read<Edge = Self::Edge> + From<Self::Borrow<'k>> + From<Self::BorrowInsert<'k>>;

    #[expect(private_bounds)]
    type Write: Write<Edge = Self::Edge> + for<'k> From<Self::Read<'k>>;

    #[expect(private_bounds)]
    type Edge: ribbit::Pack<Packed: edge::Meta>;

    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w>;

    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self;

    fn clone_from_borrow<'k>(borrow: Self::Borrow<'k>) -> Self;

    fn borrow<'k>(&'k self) -> Self::Borrow<'k>;

    fn borrow_insert<'k>(&'k self) -> Self::BorrowInsert<'k>;

    // Key length in bytes
    fn len(borrow: Self::Borrow<'_>) -> usize;
}

pub(crate) trait Read: Copy + fmt::Debug + Default {
    // Hint for fixed-size keys
    const BITS: Option<usize>;

    type Edge: ribbit::Pack<Packed: edge::Meta>;

    fn bits(&self) -> usize;

    #[inline]
    fn bytes(&self) -> usize {
        self.bits() >> 3
    }

    // Linear reads for cursor traversal
    fn next(&mut self) -> Option<u8>;

    #[inline]
    unsafe fn next_unchecked(&mut self) -> u8 {
        match self.next() {
            Some(byte) => byte,
            None if cfg!(feature = "validate") => unreachable!(),
            None => unsafe { core::hint::unreachable_unchecked() },
        }
    }

    fn read(
        &mut self,
        len: <<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len,
    ) -> <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key;

    #[inline]
    fn match_exact(
        &mut self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> Option<<<<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len> {
        let key = edge::Meta::key(edge);
        let len = edge::Key::len(key);
        (self.read(len) == key).then_some(len)
    }

    #[inline]
    fn match_inexact(
        &mut self,
        edge: <Self::Edge as ribbit::Pack>::Packed,
    ) -> (
        <<Self::Edge as ribbit::Pack>::Packed as edge::Meta>::Key,
        bool,
    ) {
        let key = edge::Meta::key(edge);
        let len = edge::Key::len(key);
        let read = self.read(len);
        (read, read == key)
    }

    fn trim(&mut self, bits: usize);

    // Prefix operations for prefix and range iteration
    fn prefix(self, bits: usize) -> Self;
    fn suffix(self, bits: usize) -> Self;
    fn common_prefix(self, other: Self) -> Self;
}

pub(crate) trait Write: Clone + fmt::Debug + Default {
    type Len: Copy;
    type Edge: ribbit::Pack<Packed: edge::Meta>;

    fn len_from_bits(bits: usize) -> Self::Len;

    /// Write bytes starting at `start` with bytes from `edge`
    ///
    /// Caller must ensure `start` is equal to the current length of this writer
    fn write(&mut self, start: Self::Len, edge: ribbit::Packed<Self::Edge>) -> Self::Len;

    /// Replace bytes starting at `start` with bytes from `node` and `edge`
    fn replace(
        &mut self,
        start: Self::Len,
        node: u8,
        edge: ribbit::Packed<Self::Edge>,
    ) -> Self::Len;
}

#[derive(Clone)]
pub(crate) struct Ignore<M>(PhantomData<M>);

impl<M> Default for Ignore<M> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<M> core::fmt::Debug for Ignore<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Ignore")
    }
}
impl<M> PartialEq for Ignore<M> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl<M> Eq for Ignore<M> {}
impl<M> Ord for Ignore<M> {
    fn cmp(&self, _: &Self) -> core::cmp::Ordering {
        core::cmp::Ordering::Equal
    }
}
impl<M> PartialOrd for Ignore<M> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<M> Write for Ignore<M>
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    type Len = ();
    type Edge = M;

    #[inline]
    fn len_from_bits(_bits: usize) -> Self::Len {}

    #[inline]
    fn write(&mut self, (): Self::Len, _edge: ribbit::Packed<Self::Edge>) -> Self::Len {}

    #[inline]
    fn replace(&mut self, _start: Self::Len, _node: u8, _edge: ribbit::Packed<Self::Edge>) {}
}

impl<R, M> From<R> for Ignore<M>
where
    R: Read<Edge = M>,
{
    fn from(_: R) -> Self {
        Self(PhantomData)
    }
}

macro_rules! impl_unsigned_int {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Key for $ty {
                type Read<'k> = integer::Reader<$ty>;
                type Write = integer::Writer<$ty>;
                type Borrow<'k> = Self;
                type BorrowInsert<'k> = Self;

                type Edge = edge::Be;

                #[inline]
                fn borrow(&self) -> Self {
                    *self
                }

                #[inline]
                fn borrow_insert(&self) -> Self {
                    *self
                }

                #[inline]
                fn clone_from_borrow<'k>(borrow: Self::Borrow<'k>) -> Self {
                    borrow
                }

                #[inline]
                unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
                    writer.into_key_unchecked()
                }

                #[inline]
                unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
                    writer.into_key_unchecked()
                }

                #[inline]
                fn len(_: Self::Borrow<'_>) -> usize {
                    <$ty as integer::Uint>::BYTES as usize
                }
            }
        )*
    };
}

impl_unsigned_int!(u16, u32, u128);

#[cfg(feature = "opt-no-int")]
impl Key for u64 {
    type Read<'k> = integer::Slow;
    type Write = dynamic::Writer;
    type Borrow<'k> = Self;

    type Edge = edge::Le;

    #[inline]
    fn borrow(&self) -> Self {
        *self
    }

    #[inline]
    fn borrow_write(&self) -> Self {
        *self
    }

    #[inline]
    fn clone_from_borrow<'k>(borrow: Self::Borrow<'k>) -> Self {
        borrow
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        let buffer: &[u8; 8] = writer.0.as_slice().try_into().unwrap();
        u64::from_be_bytes(*buffer)
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        let buffer: [u8; 8] = writer.0.try_into().unwrap();
        u64::from_be_bytes(buffer)
    }

    #[inline]
    fn len(_: Self::Borrow<'_>) -> usize {
        <u64 as integer::Uint>::BYTES as usize
    }
}

#[cfg(not(feature = "opt-no-int"))]
impl_unsigned_int!(u64);

impl Key for Vec<u8> {
    type Read<'k> = dynamic::Reader<'k>;
    type Write = dynamic::Writer;
    type Borrow<'k> = &'k [u8];
    type BorrowInsert<'k> = &'k [u8];

    type Edge = edge::Le;

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    fn borrow_insert(&self) -> Self::Borrow<'_> {
        self
    }

    #[inline]
    fn clone_from_borrow<'k>(borrow: Self::Borrow<'k>) -> Self {
        Vec::from(borrow)
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        &writer.0
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        writer.0
    }

    #[inline]
    fn len(slice: Self::Borrow<'_>) -> usize {
        slice.len()
    }
}

impl<'w> From<&'w dynamic::Writer> for &'w [u8] {
    #[inline]
    fn from(writer: &'w dynamic::Writer) -> Self {
        writer.0.as_slice()
    }
}

impl From<dynamic::Writer> for Vec<u8> {
    #[inline]
    fn from(writer: dynamic::Writer) -> Self {
        writer.0
    }
}

impl Key for String {
    type Read<'k> = dynamic::Reader<'k>;
    type Write = dynamic::Writer;
    type Borrow<'k> = &'k str;
    type BorrowInsert<'k> = &'k str;

    type Edge = edge::Le;

    #[inline]
    fn borrow<'k>(&'k self) -> Self::Borrow<'k> {
        self
    }

    #[inline]
    fn borrow_insert(&self) -> Self::Borrow<'_> {
        self
    }

    #[inline]
    fn clone_from_borrow<'k>(borrow: Self::Borrow<'k>) -> Self {
        String::from(borrow)
    }

    #[inline]
    unsafe fn borrow_writer_unchecked<'w>(writer: &'w Self::Write) -> Self::Borrow<'w> {
        if cfg!(feature = "validate") {
            core::str::from_utf8(&writer.0).unwrap()
        } else {
            unsafe { core::str::from_utf8_unchecked(&writer.0) }
        }
    }

    #[inline]
    unsafe fn from_writer_unchecked(writer: Self::Write) -> Self {
        if cfg!(feature = "validate") {
            String::from_utf8(writer.0).unwrap()
        } else {
            unsafe { String::from_utf8_unchecked(writer.0) }
        }
    }

    #[inline]
    fn len(string: Self::Borrow<'_>) -> usize {
        string.len()
    }
}

impl<'w> From<&'w dynamic::Writer> for &'w str {
    #[inline]
    fn from(writer: &'w dynamic::Writer) -> Self {
        str::from_utf8(writer.0.as_slice()).expect("key::Write should be valid UTF-8")
    }
}

#[cfg(test)]
mod tests {
    use crate::raw::Key;
    use crate::raw::edge;
    use crate::raw::edge::Len as _;
    use crate::raw::key::Read as _;

    pub(super) fn take_all<'k, K: Key>(
        array: &[u8],
        key: impl Into<K::Borrow<'k>>,
        lens: &[usize],
    ) {
        let mut reader = K::Read::from(key.into());
        let mut index = 0;
        let mut actual = Vec::new();

        for len in
            lens.iter().copied().map(|bytes| bytes << 3).map(
                <<<K::Edge as ribbit::Pack>::Packed as edge::Meta>::Key as edge::Key>::Len::new,
            )
        {
            assert_eq!(reader.bytes(), array.len() - index);

            let bytes = len.bits() >> 3;

            actual.clear();
            actual.extend(reader.read(len));
            assert_eq!(actual, &array[index..][..bytes]);

            index += bytes;
        }

        assert_eq!(reader.bytes(), array.len() - index);
        assert_eq!(reader.next(), array.get(index).copied());
    }
}
