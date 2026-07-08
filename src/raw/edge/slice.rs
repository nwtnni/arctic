use core::fmt::Debug;

use ribbit::u13;
use ribbit::u48;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key::Terminate;

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, derive(Debug))]
pub struct Slice<T> {
    ptr: u48,
    len: u13,
    value: bool,
    frozen: bool,
    #[ribbit(size = 1)]
    pub(crate) terminate: T,
}

impl<T: ribbit::Pack<Packed: Default>> Slice<T> {
    #[inline]
    pub(crate) fn new(slice: &[u8]) -> ribbit::Packed<Self> {
        validate!(slice.len() < u16::MAX as usize);
        let len = u13::new(slice.len() as u16);
        let ptr = slice.as_ptr().expose_provenance() as u64;
        validate!(ptr > 1 && ptr < (1 << 48));
        ribbit::Packed::<Self>::new(
            u48::new(ptr),
            len,
            false,
            false,
            ribbit::Packed::<T>::default(),
        )
    }
}

impl<T: ribbit::Pack> SlicePacked<T> {
    #[inline]
    pub(crate) unsafe fn as_slice(&self) -> &[u8] {
        let ptr = self.ptr().value() as *const u8;
        if ptr.is_null() {
            return &[];
        }
        let len = self.len().value() as usize;
        unsafe { core::slice::from_raw_parts(ptr, len) }
    }
}

impl<T: Terminate> Default for SlicePacked<T> {
    fn default() -> Self {
        Self::NULL
    }
}

impl<T: ribbit::Pack> IntoIterator for SlicePacked<T> {
    type Item = u8;
    type IntoIter = std::vec::IntoIter<u8>;
    fn into_iter(self) -> Self::IntoIter {
        unsafe { self.as_slice().to_vec().into_iter() }
    }
}

impl<T: Terminate> edge::Meta for SlicePacked<T> {
    const NULL: Self = Self::new(
        u48::new(0),
        u13::new(0),
        false,
        false,
        <T as Terminate>::FALSE,
    );
    type Len = u13;

    #[inline]
    fn is_value(self) -> bool {
        self.value()
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.frozen()
    }

    #[inline]
    fn has_terminator(self) -> bool {
        self.terminate().get()
    }

    #[inline]
    fn with_frozen(self, frozen: bool) -> Self {
        self.with_frozen(frozen)
    }

    fn len(self) -> Self::Len {
        self.len() + u13::new(self.terminate().get() as u16)
    }

    fn with_value(self, value: bool) -> Self {
        self.with_value(value)
    }

    fn try_compress(self, byte: u8, child: Self) -> Option<Self> {
        validate!(!self.frozen());
        validate!(!self.value());

        let len_parent = self.len().value();
        let len_byte = T::try_compress(byte) as u16;
        let len_child = child.len().value();
        let len = u13::try_new(len_parent + len_byte + len_child).ok()?;

        // If we're compressing a terminator byte, then
        // the child must be an empty edge without a terminator
        validate!(len_byte == 1 || !child.terminate().get() && len_child == 0 && child.value());

        Some(
            Slice::new(unsafe {
                core::slice::from_raw_parts(
                    child
                        .as_slice()
                        .as_ptr()
                        .byte_sub((len_parent + len_byte) as usize),
                    len.bytes(),
                )
            })
            .with_value(child.value())
            .with_frozen(child.frozen())
            .with_terminate(T::new(len_byte == 0 || child.terminate().get())),
        )
    }

    #[inline]
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        let len = edge::Meta::len(self);
        if index >= len {
            return None;
        }

        let slice = unsafe { self.as_slice() };
        validate!(index.bytes() <= slice.len());

        let parent = Slice::new(&slice[..index.bytes()]);
        let byte = slice.get(index.bytes()).copied().unwrap_or(0);
        let index_child = (index + Self::Len::BYTE).bytes().min(slice.len());
        let child = Slice::new(&slice[index_child..])
            .with_value(self.value())
            .with_frozen(self.frozen())
            .with_terminate(T::new(
                self.terminate().get() && index.bytes() < slice.len(),
            ));

        Some((parent, byte, child))
    }
}

impl<T: Terminate> Eq for SlicePacked<T> {}

impl<T: Terminate> PartialEq for SlicePacked<T> {
    fn eq(&self, other: &Self) -> bool {
        unsafe {
            self.as_slice() == other.as_slice() && self.terminate().get() == other.terminate().get()
        }
    }
}

impl<T: Terminate> Ord for SlicePacked<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        unsafe { self.as_slice().cmp(other.as_slice()) }
    }
}

impl<T: Terminate> PartialOrd for SlicePacked<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl edge::Len for u13 {
    const MAX: Self = <u13 as ribbit::Integer>::MAX;
    const BYTE: Self = u13::new(1);

    fn bits(self) -> usize {
        (self.value() as usize) << 3
    }

    fn range_to(self) -> impl Iterator<Item = Self> {
        (0..=self.value()).map(Self::new)
    }
}
