use ribbit::u14;
use ribbit::u48;

use crate::raw::edge;
use crate::raw::edge::Meta as _;

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
        validate!(ptr < (1 << 48));
        ribbit::Packed::<Self>::new(u48::new(ptr), len, false, false)
    }
}

impl SlicePacked {
    pub(crate) unsafe fn as_slice(&self) -> &[u8] {
        let ptr = self.ptr().value() as *const u8;
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
    const NULL: Self = Self::new(u48::new(1), u14::new(0), false, false);
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

    // #[inline]
    // fn expand(self, new: Self::Key) -> Result<(Self, u8, Self), ()> {
    //     let old = unsafe { self.as_slice() };
    //     let new = unsafe { new.as_slice() };
    //     let min = old.len().min(new.len());
    //
    //     let prefix = core::iter::zip(old, new)
    //         .position(|(l, r)| l != r)
    //         .unwrap_or(min);
    //
    //     if prefix == min {
    //         return Err(());
    //     }
    //
    //     let parent = Slice::new(old, u14::new(prefix as u16)).with_value(false);
    //     let middle = old[prefix];
    //     let child = Slice::new(
    //         &old[prefix + 1..],
    //         u14::new((old.len() - prefix - 1) as u16),
    //     )
    //     .with_meta(self);
    //     Ok((parent, middle, child))
    // }

    #[inline]
    fn compress(self, byte: u8, child: Self) -> Option<Self> {
        validate!(self.frozen());
        todo!()
        //
        // let parent = unsafe { self.as_slice() };
        // let child = unsafe { child.as_slice() };
        // let len = u16::try_from(parent.len() + 1 + child.len())
        //     .ok()
        //     .and_then(|len| u14::try_from(len).ok())?;
    }

    fn len(self) -> Self::Len {
        Self::len(self)
    }

    fn with_value(self, value: bool) -> Self {
        self.with_value(value)
    }

    fn with_key(self, key: Self) -> Self {
        unsafe { Self::new_unchecked(self.value & Slice::MASK_META | key.value) }
    }

    fn with_inline(self, inline: bool) -> Self {
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

impl edge::Len for u14 {
    const MAX: Self = u14::new((1u16 << 14) - 1);

    // fn new(bits: usize) -> Self {
    //     u14::new((bits >> 3) as u16)
    // }

    fn bits(self) -> usize {
        (self.value() as usize) << 3
    }
}
