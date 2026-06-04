//! [`Le`] stores edge metadata for little-endian systems.

use core::cmp;
use core::ops::BitAnd as _;
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
    #[ribbit(offset = 59)]
    len: u3,
}

impl Le {
    const MASK_FLAG: u64 = 0b0000_0111u64 << 56;
    const MASK_LEN: u64 = 0b0011_1000 << 56;

    #[inline]
    pub(crate) fn new(value: u64, len: u6) -> ribbit::Packed<Self> {
        validate_eq!(len.value() & 0b111, 0);
        unsafe {
            ribbit::Packed::<Self>::new_unchecked(
                value & Self::mask(len) | ((len.value() as u64) << 56),
            )
        }
    }

    #[inline]
    fn mask(len: u6) -> u64 {
        (1 << len.bits()) - 1
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
        self.raw()
            .to_le_bytes()
            .into_iter()
            .take(self.len().value() as usize)
    }
}

impl edge::Meta for LePacked {
    const NULL: Self = Self::new(u56::new(0), false, false, u3::new(0));

    type Len = u6;

    #[inline]
    fn len(self) -> u6 {
        unsafe { u6::new_unchecked(((self.raw() & Le::MASK_LEN) >> 56) as u8) }
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

    fn try_compress(self, byte: u8, child: Self) -> Option<Self> {
        validate!(!self.frozen());
        validate!(!self.value());

        let len_parent = edge::Meta::len(self);
        let len_byte = Self::Len::BYTE.value();
        let len_child = edge::Meta::len(child).value();
        let len = u6::try_new(len_parent.value() + len_byte + len_child).ok()?;
        let index_child = (len_parent.value() + len_byte) as u32;

        Some(unsafe {
            Self::new_unchecked(
                // Parent prefix
                self.raw()
                    // Byte
                    .bitor((byte as u64) << len_parent.value())
                    // Child prefix
                    .bitor(child.raw() << index_child)
                    // Length and flags
                    .bitand(Le::mask(len))
                    .bitor((len.value() as u64) << 56)
                    .bitor(child.raw() & Le::MASK_FLAG),
            )
        })
    }

    #[inline]
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        let len = edge::Meta::len(self);
        if index >= len {
            return None;
        }

        let parent = Le::new(self.raw(), index);
        let byte = (self.raw() >> index.value()) as u8;
        let index_child = index + Self::Len::BYTE;
        let len_child = len - index_child;

        let child = unsafe {
            Self::new_unchecked(
                (self.raw() >> index_child.value())
                    .bitand(Le::mask(len_child))
                    .bitor((len_child.value() as u64) << 56)
                    .bitor(self.raw() & Le::MASK_FLAG),
            )
        };

        Some((parent, byte, child))
    }
}

impl Eq for LePacked {}

impl PartialEq for LePacked {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        ((self.raw() ^ other.raw()) & !Le::MASK_FLAG) == 0
    }
}

impl Ord for LePacked {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self == other {
            return cmp::Ordering::Equal;
        }

        self.raw().swap_bytes().cmp(&other.raw().swap_bytes())
    }
}

impl PartialOrd for LePacked {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for Le {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Just;
        use proptest::strategy::Strategy as _;
        (
            bool::arbitrary(),
            bool::arbitrary(),
            0u8..=<u3 as ribbit::traits::Integer>::MAX.value(),
        )
            .prop_flat_map(|(value, frozen, len)| {
                (
                    Just(value),
                    Just(frozen),
                    Just(len),
                    (0..(1u64 << (len << 3))),
                )
            })
            .prop_map(|(value, frozen, len, prefix)| Self {
                value,
                frozen,
                len: u3::new(len),
                prefix: u56::new(prefix),
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    crate::raw::edge::tests::impl_suite!(crate::raw::edge::Le);
}
