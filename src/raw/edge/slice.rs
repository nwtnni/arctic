use ribbit::u14;
use ribbit::u48;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, debug)]
pub struct Slice {
    ptr: ribbit::u48,
    len: ribbit::u14,
    value: bool,
    frozen: bool,
}

impl Slice {
    #[inline]
    pub(crate) fn new(slice: &[u8]) -> ribbit::Packed<Self> {
        validate!(slice.len() < u16::MAX as usize);
        let len = u14::new(slice.len() as u16);
        let ptr = slice.as_ptr().expose_provenance() as u64;
        validate!(ptr < (1 << 48));
        ribbit::Packed::<Self>::new(u48::new(ptr), len, false, false)
    }
}

impl SlicePacked {
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

impl Default for SlicePacked {
    fn default() -> Self {
        Self::NULL
    }
}

impl IntoIterator for SlicePacked {
    type Item = u8;
    type IntoIter = std::vec::IntoIter<u8>;
    fn into_iter(self) -> Self::IntoIter {
        unsafe { self.as_slice().to_vec().into_iter() }
    }
}

impl edge::Meta for SlicePacked {
    const NULL: Self = Self::new(u48::new(0), u14::new(0), false, false);
    type Len = u14;

    #[inline]
    fn is_value(self) -> bool {
        self.value()
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.frozen()
    }

    #[inline]
    fn with_frozen(self, frozen: bool) -> Self {
        self.with_frozen(frozen)
    }

    fn len(self) -> Self::Len {
        Self::len(self)
    }

    fn with_value(self, value: bool) -> Self {
        self.with_value(value)
    }

    fn try_compress(self, _: u8, child: Self) -> Option<Self> {
        validate!(!self.frozen());
        validate!(!self.value());

        let len_parent = self.len().value();
        let len_byte = Self::Len::BYTE.value();
        let len_child = child.len().value();
        let len = u14::try_new(len_parent + len_byte + len_child).ok()?;

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
            .with_frozen(child.frozen()),
        )
    }

    #[inline]
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        let len = self.len();
        if index >= len {
            return None;
        }

        let slice = unsafe { self.as_slice() };

        let parent = Slice::new(&slice[..index.bytes()]);
        let byte = slice[index.bytes()];
        let index_child = index + Self::Len::BYTE;
        let child = Slice::new(&slice[index_child.bytes()..])
            .with_value(self.value())
            .with_frozen(self.frozen());

        Some((parent, byte, child))
    }
}

impl Eq for SlicePacked {}

impl PartialEq for SlicePacked {
    fn eq(&self, other: &Self) -> bool {
        unsafe { self.as_slice() == other.as_slice() }
    }
}

impl Ord for SlicePacked {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        unsafe { self.as_slice().cmp(other.as_slice()) }
    }
}

impl PartialOrd for SlicePacked {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl edge::Len for u14 {
    const MAX: Self = u14::new((1u16 << 14) - 1);
    const BYTE: Self = u14::new(1);

    fn bits(self) -> usize {
        (self.value() as usize) << 3
    }

    fn range_to(self) -> impl Iterator<Item = Self> {
        (0..=self.value()).map(Self::new)
    }
}
