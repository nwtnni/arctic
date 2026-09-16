//! [`Be`] stores edge metadata for integers and big-endian systems.

use core::cmp;
use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::BitOr as _;

use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key::Len as _;
use crate::raw::key::len::Bit;
use crate::sync::Convert;

/// Edge metadata storing compressed edge bytes starting at most significant byte.
///
/// Optimized for integer keys, or slice keys on big-endian systems.
// Layout:
// - 0: value
// - 1: frozen
// - 2: padding for len
// - 3..6: len
// - 6..8: zero
// - 8..16: least significant key byte
// - ...
// - 56..64: most significant key byte
#[derive(Copy, Clone)]
pub struct Be<const CAPACITY: u8>(u64);

impl<const CAPACITY: u8> Be<CAPACITY> {
    const MASK_VALUE: u64 = 1;
    const MASK_FROZEN: u64 = 1 << 1;

    #[inline]
    pub(crate) fn new(value: u64, len: Bit<CAPACITY>) -> Self {
        unsafe { Self::from_raw_unchecked(value & Self::mask(len) | len.into_u8() as u64) }
    }

    #[inline]
    fn mask(len: Bit<CAPACITY>) -> u64 {
        !(u64::MAX >> len.bits())
    }
}

impl<const CAPACITY: u8> edge::Meta for Be<CAPACITY> {
    const NULL: Self = Self(0);

    type Len = Bit<CAPACITY>;

    #[inline]
    fn len(self) -> Self::Len {
        Bit::new_masked(self.into_raw() as u8)
    }

    #[inline]
    fn is_value(self) -> bool {
        self.0 & Self::MASK_VALUE > 0
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.0 & Self::MASK_FROZEN > 0
    }

    #[inline]
    fn with_value(self, value: bool) -> Self {
        Self(if value {
            self.0 | Self::MASK_VALUE
        } else {
            self.0 & !Self::MASK_VALUE
        })
    }

    #[inline]
    fn with_frozen(self, frozen: bool) -> Self {
        Self(if frozen {
            self.0 | Self::MASK_FROZEN
        } else {
            self.0 & !Self::MASK_FROZEN
        })
    }

    fn try_compress(self, byte: u8, child: Self) -> Option<Self> {
        validate!(!self.is_frozen());
        validate!(!self.is_value());

        let len_parent = self.len();
        let len_child = child.len();
        let len = Bit::try_add(len_parent, Self::Len::BYTE, len_child)?;
        let index_child = (len_parent + Self::Len::BYTE).into_u8();

        Some(unsafe {
            Self::from_raw_unchecked(
                // Parent prefix
                self.into_raw()
                    // Byte
                    .bitor((byte as u64).rotate_right(index_child as u32))
                    // Child prefix
                    .bitor(child.into_raw() >> index_child)
                    // Length
                    .bitand(Be::mask(len))
                    .bitor(len.into_u8() as u64)
                    // Preserve child flags
                    .bitor(child.into_raw() & (Self::MASK_VALUE | Self::MASK_FROZEN)),
            )
        })
    }

    #[inline]
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        let index = index.align_down();
        let len = self.len();
        if index >= len {
            return None;
        }

        let len_parent = index;
        let parent = Be::new(self.into_raw(), len_parent);

        let index_child = len_parent + Self::Len::BYTE;
        let len_child = len - index_child;

        let byte = self.into_raw().rotate_left(index_child.into_u8() as u32) as u8;

        let child = unsafe {
            Self::from_raw_unchecked(
                (self.into_raw() << index_child.into_u8())
                    .bitand(Self::mask(len_child))
                    .bitor(len_child.into_u8() as u64)
                    .bitor(self.into_raw() & (Self::MASK_VALUE | Self::MASK_FROZEN)),
            )
        };

        Some((parent, byte, child))
    }
}

impl<const CAPACITY: u8> IntoIterator for Be<CAPACITY> {
    type Item = u8;
    type IntoIter = core::iter::Take<core::array::IntoIter<u8, 8>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.into_raw()
            .to_be_bytes()
            .into_iter()
            .take(self.len().bytes())
    }
}

impl<const CAPACITY: u8> Eq for Be<CAPACITY> {}

impl<const CAPACITY: u8> PartialEq for Be<CAPACITY> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        ((self.into_raw() ^ other.into_raw()) & !(Self::MASK_VALUE | Self::MASK_FROZEN)) == 0
    }
}

impl<const CAPACITY: u8> Ord for Be<CAPACITY> {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self == other {
            return cmp::Ordering::Equal;
        }

        self.into_raw().cmp(&other.into_raw())
    }
}

impl<const CAPACITY: u8> PartialOrd for Be<CAPACITY> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<const CAPACITY: u8> Convert<u64> for Be<CAPACITY> {
    #[inline]
    fn into_raw(self) -> u64 {
        self.0
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        Self(raw)
    }
}

impl<const CAPACITY: u8> Debug for Be<CAPACITY> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Be")
            .field("value", &self.is_value())
            .field("frozen", &self.is_frozen())
            .field("keys", &&self.0.to_be_bytes()[..self.len().bytes()])
            .finish()
    }
}

#[cfg(feature = "proptest")]
impl<const CAPACITY: u8> proptest::arbitrary::Arbitrary for Be<CAPACITY> {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Just;
        use proptest::strategy::Strategy as _;
        (bool::arbitrary(), bool::arbitrary(), 0u8..=CAPACITY >> 3)
            .prop_flat_map(|(value, frozen, len)| {
                (
                    Just(value),
                    Just(frozen),
                    Just(len),
                    (0u64..(1u64 << (len << 3))).prop_map(|prefix| prefix.swap_bytes() >> 8),
                )
            })
            .prop_map(|(value, frozen, len, prefix)| {
                Self::new(prefix, Bit::new_masked(len << 3))
                    .with_value(value)
                    .with_frozen(frozen)
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    crate::raw::edge::tests::impl_suite!(crate::raw::edge::Be::<56>);
}
