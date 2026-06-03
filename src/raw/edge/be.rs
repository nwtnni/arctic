//! [`Be`] stores edge metadata for integers and big-endian systems.

use core::cmp;
use core::ops::BitAnd as _;
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
        validate_eq!(len.bits() & 0b111, 0);
        unsafe {
            ribbit::Packed::<Self>::new_unchecked(value & Self::mask(len) | len.bits() as u64)
        }
    }

    #[inline]
    fn mask(len: u6) -> u64 {
        !(u64::MAX >> len.bits())
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
        self.raw()
            .to_be_bytes()
            .into_iter()
            .take(self.len().value() as usize)
    }
}

impl edge::Meta for BePacked {
    const NULL: Self = Self::new(false, false, u3::new(0), u56::new(0));

    type Len = u6;

    #[inline]
    fn len(self) -> Self::Len {
        unsafe { u6::new_unchecked((self.raw() & Be::MASK_LEN) as u8) }
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
                    .bitor((byte as u64).rotate_right(index_child))
                    // Child prefix
                    .bitor(child.raw() >> index_child)
                    // Length and flags
                    .bitand(Be::mask(len))
                    .bitor(len.value() as u64)
                    .bitor(child.raw() & Be::MASK_FLAG),
            )
        })
    }

    #[inline]
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        let len = edge::Meta::len(self);
        if index >= len {
            return None;
        }

        let parent = Be::new(self.raw(), index);
        let byte = self.raw().get_u8(index.value());
        let index_child = index + Self::Len::BYTE;
        let len_child = len - index_child;

        let child = unsafe {
            Self::new_unchecked(
                (self.raw() << index_child.value())
                    .bitand(Be::mask(len_child))
                    .bitor(len_child.value() as u64)
                    .bitor(self.raw() & Be::MASK_FLAG),
            )
        };

        Some((parent, byte, child))
    }
}

impl Eq for BePacked {}

impl PartialEq for BePacked {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        ((self.raw() ^ other.raw()) & !Be::MASK_FLAG) == 0
    }
}

impl Ord for BePacked {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        if self == other {
            return cmp::Ordering::Equal;
        }

        self.raw().cmp(&other.raw())
    }
}

impl PartialOrd for BePacked {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for Be {
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
                    (0..(1u64 << (len << 3))).prop_map(|prefix| prefix.swap_bytes() >> 8),
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
    use proptest::proptest;

    #[cfg(feature = "proptest")]
    proptest! {
         #[test]
         fn expand_compress_inverse(meta: crate::raw::edge::Be) {
             crate::raw::edge::tests::expand_compress_inverse(meta)
         }
    }
}
