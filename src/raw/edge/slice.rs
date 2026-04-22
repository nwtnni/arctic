use ribbit::u14;
use ribbit::u48;

use crate::raw::edge;

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, debug)]
pub struct Slice {
    ptr: u48,
    len: u14,
    value: bool,
    frozen: bool,
}

impl Slice {
    const MASK_META: u64 = 0b11u64 << 62;
    const MASK_KEY: u64 = !Self::MASK_META;

    #[inline]
    pub(crate) fn new(slice: &[u8], len: u14) -> ribbit::Packed<Self> {
        let ptr = slice.as_ptr() as u64;
        ribbit::Packed::<Self>::new(u48::new(ptr), len, false, false)
    }

    #[inline]
    pub(crate) fn min_len(edge: u14, reader: usize) -> u14 {
        u14::new((edge.value() as usize).min(reader) as u16)
    }
}

impl SlicePacked {
    fn with_meta(self, meta: Self) -> Self {
        unsafe { Self::new_unchecked(self.value | (meta.value & Slice::MASK_META)) }
    }

    unsafe fn as_slice(&self) -> &[u8] {
        let ptr = self.ptr().value() as *const u8;
        let len = self.len().value() as usize;
        unsafe { core::slice::from_raw_parts(ptr, len) }
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
    const DEFAULT: Self = Self::new(u48::new(1), u14::new(0), false, false);

    type Key = Self;

    #[inline]
    fn new(key: Self::Key, value: bool) -> Self {
        key.with_value(value)
    }

    #[inline]
    fn key(self) -> Self::Key {
        unsafe { Self::new_unchecked(self.value & Slice::MASK_KEY) }
    }

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

    #[inline]
    fn expand(self, new: Self::Key) -> Result<(Self, u8, Self), ()> {
        let old = unsafe { self.as_slice() };
        let new = unsafe { new.as_slice() };
        let min = old.len().min(new.len());

        let prefix = core::iter::zip(old, new)
            .position(|(l, r)| l != r)
            .unwrap_or(min);

        if prefix == min {
            return Err(());
        }

        let parent = Slice::new(old, u14::new(prefix as u16)).with_value(false);
        let middle = old[prefix];
        let child = Slice::new(
            &old[prefix + 1..],
            u14::new((old.len() - prefix - 1) as u16),
        )
        .with_meta(self);
        Ok((parent, middle, child))
    }

    #[inline]
    fn compress(self, _byte: u8, _child: Self) -> Option<Self> {
        validate!(self.frozen());

        todo!()
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

impl edge::Key for SlicePacked {
    type Len = u14;

    #[inline]
    fn len(self) -> Self::Len {
        self.len()
    }

    #[inline]
    fn prefix(self, _len: Self::Len) -> Self {
        todo!()
    }
}

impl edge::Len for u14 {
    const MAX: Self = u14::new((1u16 << 14) - 1);

    fn new(bits: usize) -> Self {
        u14::new((bits >> 3) as u16)
    }

    fn bits(self) -> usize {
        (self.value() as usize) << 3
    }
}
