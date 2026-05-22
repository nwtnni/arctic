//! [`Le`] stores edge metadata for little-endian systems.

use core::cmp;
use core::ops::BitOr as _;

use ribbit::u3;
use ribbit::u6;
use ribbit::u56;

use crate::raw::edge;
use crate::raw::edge::Len as _;

/// Edge metadata storing compressed edge bytes starting at least significant byte.
///
/// Optimized for slice keys on little-endian systems.
#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, debug)]
pub struct Le {
    prefix: u56,
    value: bool,
    frozen: bool,
    inline: bool,
    #[ribbit(offset = 59)]
    len: u3,
}

impl Le {
    const MASK_FLAG: u64 = 0b0000_0111u64 << 56;
    const MASK_LEN: u64 = 0b0011_1000 << 56;

    #[inline]
    pub(crate) fn new(value: u64, len: u6) -> ribbit::Packed<Self> {
        validate_eq!(len.value() & 0b111, 0);
        let mask = (1 << len.bits()) - 1;
        unsafe {
            ribbit::Packed::<Self>::new_unchecked(value & mask | ((len.value() as u64) << 56))
        }
    }
}

impl LePacked {
    #[inline]
    pub(crate) fn raw(self) -> u64 {
        self.value
    }
}

impl IntoIterator for LePacked {
    type Item = u8;
    type IntoIter = core::iter::Take<core::array::IntoIter<u8, 8>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.value
            .to_le_bytes()
            .into_iter()
            .take(self.len().value() as usize)
    }
}

impl edge::Meta for LePacked {
    const NULL: Self = Self::new(u56::new(0), false, false, false, u3::new(0));

    type Len = u6;

    #[inline]
    fn len(self) -> u6 {
        unsafe { u6::new_unchecked(((self.value & Le::MASK_LEN) >> 56) as u8) }
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
    fn with_value(self, value: bool) -> Self {
        self.with_value(value)
    }

    #[inline]
    fn with_frozen(self, frozen: bool) -> Self {
        self.with_frozen(frozen)
    }

    #[inline]
    fn with_inline(self, inline: bool) -> Self {
        self.with_inline(inline)
    }

    #[inline]
    fn with_key(self, key: Self) -> Self {
        validate_eq!(key.value & Le::MASK_FLAG, 0);
        unsafe { Self::new_unchecked(self.value & Le::MASK_FLAG | key.value) }
    }

    #[inline]
    fn compress(self, byte: u8, child: Self) -> Option<Self> {
        validate!(!self.frozen());

        let parent_bits = ((self.value & Le::MASK_LEN) >> 56) as u8;
        let child_bits = ((child.value & Le::MASK_LEN) >> 56) as u8;
        let len = u6::try_new(parent_bits + 8 + child_bits).ok()?;

        Some(
            Le::new(
                (self.value & ((1 << parent_bits) - 1))
                    .bitor((byte as u64) << parent_bits)
                    .bitor(child.value << (parent_bits + 8)),
                len,
            )
            .with_value(child.value()),
        )
    }
}

impl Eq for LePacked {}

impl PartialEq for LePacked {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        ((self.value ^ other.value) & !Le::MASK_FLAG) == 0
    }
}

impl Ord for LePacked {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self == other {
            return cmp::Ordering::Equal;
        }

        self.value.swap_bytes().cmp(&other.value.swap_bytes())
    }
}

impl PartialOrd for LePacked {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}
