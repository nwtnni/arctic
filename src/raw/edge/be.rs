//! [`Be`] stores edge metadata for integers and big-endian systems.

use core::cmp;
use core::ops::BitOr as _;

use ribbit::u3;
use ribbit::u6;
use ribbit::u56;

use crate::raw::Int;
use crate::raw::edge;
use crate::raw::edge::Len as _;

/// Edge metadata storing compressed edge bytes starting at most significant byte.
///
/// Optimized for integer keys, or slice keys on big-endian systems.
#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, debug)]
pub struct Be {
    value: bool,
    frozen: bool,
    inline: bool,
    #[ribbit(offset = 3)]
    len: u3,
    #[ribbit(offset = 8)]
    prefix: u56,
}

impl Be {
    const MASK_FLAG: u64 = 0b111;
    const MASK_LEN: u64 = 0b11_1000;

    #[inline]
    pub(crate) fn new(value: u64, len: u6) -> ribbit::Packed<Self> {
        validate_eq!(len.value() & 0b111, 0);
        let mask = !(u64::MAX >> len.bits());
        unsafe { ribbit::Packed::<Self>::new_unchecked(value & mask | len.bits() as u64) }
    }
}

impl BePacked {
    #[inline]
    pub(crate) fn raw(self) -> u64 {
        self.value
    }
}

impl IntoIterator for BePacked {
    type Item = u8;
    type IntoIter = core::iter::Take<core::array::IntoIter<u8, 8>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.value
            .to_be_bytes()
            .into_iter()
            .take(self.len().value() as usize)
    }
}

impl edge::Meta for BePacked {
    const NULL: Self = Self::new(false, false, false, u3::new(0), u56::new(0));

    type Len = u6;

    #[inline]
    fn len(self) -> Self::Len {
        unsafe { u6::new_unchecked((self.value & Be::MASK_LEN) as u8) }
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
        validate_eq!(key.value & Be::MASK_FLAG, 0);
        unsafe { Self::new_unchecked(self.value & Be::MASK_FLAG | key.value) }
    }

    #[inline]
    fn try_join(self, byte: u8, child: Self) -> Option<Self> {
        validate!(!self.frozen());

        let len_parent = edge::Meta::len(self).value();
        let len_byte = Self::Len::BYTE.value();
        let len_child = edge::Meta::len(child).value();
        let len = u6::try_new(len_parent + len_byte + len_child).ok()?;

        let shift = (len_parent + len_byte) as u32;

        Some(
            Be::new(
                self.value
                    .most_significant(len_parent)
                    .bitor((byte as u64).rotate_right(shift))
                    .bitor(child.value >> shift),
                len,
            )
            .with_value(child.value()),
        )
    }

    #[inline]
    fn try_split(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        let len = edge::Meta::len(self);
        if index >= len {
            return None;
        }

        let parent = Be::new(self.raw(), index);
        let byte = self.raw().get_u8(index.value());
        let index_child = index + Self::Len::BYTE;
        let child = Be::new(self.raw() << index_child.value(), len - index_child);

        Some((parent, byte, child))
    }
}

impl Eq for BePacked {}

impl PartialEq for BePacked {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        ((self.value ^ other.value) & !Be::MASK_FLAG) == 0
    }
}

impl Ord for BePacked {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self == other {
            return cmp::Ordering::Equal;
        }

        self.value.cmp(&other.value)
    }
}

impl PartialOrd for BePacked {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}
