//! [`Le`] stores edge metadata for little-endian systems.

use core::cmp;
use core::fmt::Debug;
use core::ops::BitAnd as _;
use core::ops::BitOr as _;

use ribbit::u6;

use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::sync::Convert;

/// Edge metadata storing compressed edge bytes starting at least significant byte.
///
/// Optimized for slice keys on little-endian systems.
// Layout:
// - 0..8: most significant key byte
// - ...
// - 48..56: least significant key byte
// - 56: value
// - 57: frozen
// - 58..61: len
// - 61..64: zero
#[derive(Copy, Clone)]
pub struct Le(u64);

impl Le {
    const MASK_VALUE: u64 = 1 << 56;
    const MASK_FROZEN: u64 = 1 << 57;
    const SHIFT_LEN: u64 = 58;

    #[inline]
    pub(crate) fn new(value: u64, len: ribbit::u6) -> Self {
        validate_eq!(len.value() & 0b111, 0);
        Self(value & Self::mask(len) | ((len.value() as u64) << Self::SHIFT_LEN))
    }

    #[inline]
    fn mask(len: u6) -> u64 {
        (1 << len.bits()) - 1
    }
}

impl edge::Meta for Le {
    const NULL: Self = Self(0);

    type Len = u6;

    #[inline]
    fn len(self) -> u6 {
        unsafe { u6::new_unchecked((self.into_raw() >> Self::SHIFT_LEN) as u8) }
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
        let len_byte = Self::Len::BYTE.value();
        let len_child = edge::Meta::len(child).value();
        let len = u6::try_new(len_parent.value() + len_byte + len_child).ok()?;
        let index_child = (len_parent.value() + len_byte) as u32;

        Some(Self(
            // Parent prefix
            self.into_raw()
                // Byte
                .bitor((byte as u64) << len_parent.value())
                // Child prefix
                .bitor(child.into_raw() << index_child)
                // Length
                .bitand(Le::mask(len))
                .bitor((len.value() as u64) << Self::SHIFT_LEN)
                // Preserve child flags
                .bitor(child.into_raw() & (Self::MASK_VALUE | Self::MASK_FROZEN)),
        ))
    }

    #[inline]
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        let len = edge::Meta::len(self);
        if index >= len {
            return None;
        }

        let parent = Le::new(self.into_raw(), index);
        let byte = (self.into_raw() >> index.value()) as u8;
        let index_child = index + Self::Len::BYTE;
        let len_child = len - index_child;

        let child = Self(
            (self.into_raw() >> index_child.value())
                .bitand(Le::mask(len_child))
                .bitor((len_child.value() as u64) << Self::SHIFT_LEN)
                .bitor(self.into_raw() & (Self::MASK_VALUE | Self::MASK_FROZEN)),
        );

        Some((parent, byte, child))
    }
}

impl IntoIterator for Le {
    type Item = u8;
    type IntoIter = core::iter::Take<core::array::IntoIter<u8, 8>>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.into_raw()
            .to_le_bytes()
            .into_iter()
            .take(self.len().bytes())
    }
}

impl Eq for Le {}

impl PartialEq for Le {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        ((self.into_raw() ^ other.into_raw()) & !(Self::MASK_VALUE | Self::MASK_FROZEN)) == 0
    }
}

impl Ord for Le {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self == other {
            return cmp::Ordering::Equal;
        }

        self.into_raw()
            .swap_bytes()
            .cmp(&other.into_raw().swap_bytes())
    }
}

impl PartialOrd for Le {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Convert<u64> for Le {
    #[inline]
    fn into_raw(self) -> u64 {
        self.0
    }

    #[inline]
    unsafe fn from_raw_unchecked(into_raw: u64) -> Self {
        Self(into_raw)
    }
}

impl Debug for Le {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Le")
            .field("value", &self.is_value())
            .field("frozen", &self.is_frozen())
            .field("keys", &&self.0.to_le_bytes()[..self.len().bytes()])
            .finish()
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for Le {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Just;
        use proptest::strategy::Strategy as _;
        (bool::arbitrary(), bool::arbitrary(), 0u8..=7u8)
            .prop_flat_map(|(value, frozen, len)| {
                (
                    Just(value),
                    Just(frozen),
                    Just(len),
                    (0..(1u64 << (len << 3))),
                )
            })
            .prop_map(|(value, frozen, len, prefix)| {
                Self::new(prefix, u6::new(len << 3))
                    .with_value(value)
                    .with_frozen(frozen)
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    crate::raw::edge::tests::impl_suite!(crate::raw::edge::Le);
}
