#[cfg(target_feature = "avx2")]
mod avx2;

use core::ops::BitAnd as _;
use core::ops::BitOr as _;
use core::ops::Not as _;

use ribbit::Integer as _;
use ribbit::u3;
use ribbit::u4;
use ribbit::u48;
use ribbit::u56;
use ribbit::u112;
use ribbit::u120;

pub(crate) trait Prefix: Send + Sync + Sized {
    const HAZARD_NULL: Self;

    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self;

    fn is_active(self) -> bool;

    fn is_conflict(self, hazards: &[Self; 4]) -> bool;

    fn bytes(&self) -> usize;

    fn is_node(self) -> bool;

    fn is_value(self) -> bool;

    /// For measurement purposes only
    fn age(self) -> u8;

    /// For measurement purposes only
    fn with_age(self, age: u8) -> Self;
}

// NOTE: this type is used for both **hazards**, which guard
// parts of the tree, and prefixes of retired edges.
#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, derive(Debug))]
pub struct Be {
    // Hazard: whether to protect nodes
    // Prefix: whether this is a node
    #[ribbit(get(rename = "is_node"))]
    pub(super) node: bool,

    // Hazard: whether to protect values
    // Prefix: whether this is a value
    #[ribbit(get(rename = "is_value"))]
    pub(super) value: bool,

    // Hazard: whether to protect overlaps (or just underneath prefix)
    // Prefix: ignore
    #[ribbit(get(rename = "is_overlap"))]
    pub(super) overlap: bool,

    // NOTE: at offset 3 so we don't need to shift bits
    len: u3,

    #[ribbit(offset = 8)]
    prefix: u56,
}

impl Be {
    #[inline]
    pub(crate) fn new_hazard(prefix: u64, bits: usize) -> ribbit::Packed<Self> {
        validate_eq!(bits & 0b111, 0);

        let bits = bits & 0b0111_1000;

        let bits = if cfg!(feature = "stat") {
            // Avoid clobbering logical age counter
            // Bits is > 0 (>= 8), since there can be no key with length 0
            bits - 8
        } else {
            bits
        };

        unsafe {
            ribbit::Packed::<Self>::from_raw_unchecked(
                // Protect nodes, values, and overlap
                Self::extract(prefix, bits) | bits as u64 | 0b0000_0111,
            )
        }
    }

    // Mask off everything except top `bits`
    #[inline]
    fn extract(prefix: u64, bits: usize) -> u64 {
        validate_eq!(bits & 0b111, 0);
        validate!((bits >> 3) <= u3::MAX.value() as usize);

        prefix & !(u64::MAX >> bits)
    }
}

impl Prefix for BePacked {
    const HAZARD_NULL: Self = Self::new(false, false, false, u3::new(0), u56::new(0));

    #[inline]
    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => unsafe {
                let prefix = Be::extract(self.into_raw(), bits);
                Self::from_raw_unchecked(prefix | bits as u64)
            },
            Some(_) | None => self,
        }
        .with_node(!value)
        .with_value(value)
    }

    #[inline]
    fn is_active(self) -> bool {
        // Protects either values or nodes
        self.into_raw() & 0b11 > 0
    }

    #[inline]
    fn is_conflict(self, hazards: &[Self; 4]) -> bool {
        simd!(
            "opt-no-prefix",
            self.is_conflict_avx2(hazards),
            hazards.iter().any(|hazard| self.is_conflict(*hazard)),
            "mismatch at {:x?} {:x?}",
            self,
            hazards,
        )
    }

    #[inline]
    fn is_node(self) -> bool {
        self.is_node()
    }

    #[inline]
    fn is_value(self) -> bool {
        self.is_value()
    }

    #[inline]
    fn bytes(&self) -> usize {
        self.len().value() as usize
    }

    /// For measurement purposes only
    #[inline]
    fn age(self) -> u8 {
        self.prefix().value() as u8
    }

    /// For measurement purposes only
    #[inline]
    fn with_age(self, age: u8) -> Self {
        self.with_prefix(
            self.prefix()
                .bitand(u56::from(u8::MAX).not())
                .bitor(u56::from(age)),
        )
    }
}

impl BePacked {
    #[inline]
    fn is_conflict(self, hazard: Self) -> bool {
        validate!(self.is_node() ^ self.is_value());

        // Case: `hazard` doesn't protect node or value
        if (hazard.into_raw() & self.into_raw()) & 0b11 == 0 {
            return false;
        }

        // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
        if !hazard.is_overlap() && hazard.len() > self.len() {
            return false;
        }

        let len = self.len().min(hazard.len());
        let bits = (len.value() as usize) << 3;
        Be::extract(self.into_raw() ^ hazard.into_raw(), bits) == 0
    }
}

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, derive(Debug))]
pub struct Le {
    prefix: u56,

    #[ribbit(get(rename = "is_node"))]
    pub(super) node: bool,

    #[ribbit(get(rename = "is_value"))]
    pub(super) value: bool,

    #[ribbit(get(rename = "is_overlap"))]
    pub(super) overlap: bool,

    len: u3,
}

impl Le {
    #[inline]
    #[cfg_attr(not(feature = "opt-no-int"), expect(dead_code))]
    pub(crate) fn new_hazard(prefix: u64, bits: usize) -> ribbit::Packed<Self> {
        validate_eq!(bits & 0b111, 0);

        let bits = if cfg!(feature = "stat") {
            // Avoid clobbering logical age counter
            // Bits is > 0 (>= 8), since there can be no key with length 0
            bits - 8
        } else {
            bits
        };

        unsafe {
            ribbit::Packed::<Self>::from_raw_unchecked(
                Self::extract(prefix, bits) | const { 0b111u64 << 56 } | ((bits as u64) << 56),
            )
        }
    }

    // Mask off everything except bottom `bits`
    #[inline]
    fn extract(prefix: u64, bits: usize) -> u64 {
        validate_eq!(bits & 0b111, 0);
        validate!((bits >> 3) <= u3::MAX.value() as usize);

        prefix & ((1u64 << bits) - 1)
    }
}

impl Prefix for LePacked {
    const HAZARD_NULL: Self = Self::new(u56::new(0), false, false, false, u3::new(0));

    #[inline]
    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => {
                let prefix = Le::extract(self.into_raw(), bits);
                Self::new(
                    unsafe { u56::new_unchecked(prefix) },
                    !value,
                    value,
                    false,
                    u3::new((bits >> 3) as u8),
                )
            }
            Some(_) | None => self.with_node(!value).with_value(value),
        }
    }

    #[inline]
    fn is_active(self) -> bool {
        // Protects either values or nodes
        self.into_raw() & const { 0b11u64 << 56 } > 0
    }

    #[inline]
    fn is_conflict(self, hazards: &[Self; 4]) -> bool {
        simd!(
            "opt-no-prefix",
            self.is_conflict_avx2(hazards),
            hazards.iter().any(|hazard| self.is_conflict(*hazard)),
            "mismatch at {:x?} {:x?}",
            self,
            hazards,
        )
    }

    #[inline]
    fn is_node(self) -> bool {
        self.is_node()
    }

    #[inline]
    fn is_value(self) -> bool {
        self.is_value()
    }

    #[inline]
    fn bytes(&self) -> usize {
        self.len().value() as usize
    }

    /// For measurement purposes only
    #[inline]
    fn age(self) -> u8 {
        (self.prefix().value() >> 48) as u8
    }

    /// For measurement purposes only
    #[inline]
    fn with_age(self, age: u8) -> Self {
        self.with_prefix(
            self.prefix()
                .bitand(const { u56::new(u48::MAX.value()) })
                .bitor(u56::new((age as u64) << 48)),
        )
    }
}

impl LePacked {
    #[inline]
    fn is_conflict(self, hazard: Self) -> bool {
        validate!(self.is_node() ^ self.is_value());

        // Case: `hazard` doesn't protect node or value
        if (hazard.into_raw() & self.into_raw()) & const { 0b11u64 << 56 } == 0 {
            return false;
        }

        // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
        if !hazard.is_overlap() && hazard.len() > self.len() {
            return false;
        }

        let len = self.len().min(hazard.len());
        let bits = (len.value() as usize) << 3;
        Le::extract(self.into_raw() ^ hazard.into_raw(), bits) == 0
    }
}

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, derive(Debug))]
pub struct Le128 {
    prefix: u120,

    #[ribbit(get(rename = "is_node"))]
    pub(super) node: bool,

    #[ribbit(get(rename = "is_value"))]
    pub(super) value: bool,

    #[ribbit(get(rename = "is_overlap"))]
    pub(super) overlap: bool,

    len: u4,
}

impl Le128 {
    #[inline]
    pub(crate) fn new_hazard(prefix: u128, bits: usize) -> ribbit::Packed<Self> {
        validate_eq!(bits & 0b111, 0);

        let bits = if cfg!(feature = "stat") {
            // Avoid clobbering logical age counter
            // Bits is > 0 (>= 8), since there can be no key with length 0
            bits - 8
        } else {
            bits
        };

        unsafe {
            ribbit::Packed::<Self>::from_raw_unchecked(
                Self::extract(prefix, bits) | const { 0b111u128 << 120 } | ((bits as u128) << 120),
            )
        }
    }

    // Mask off everything except bottom `bits`
    #[inline]
    fn extract(prefix: u128, bits: usize) -> u128 {
        validate_eq!(bits & 0b111, 0);
        validate!((bits >> 3) <= u4::MAX.value() as usize);

        prefix & ((1u128 << bits) - 1)
    }
}

impl Prefix for Le128Packed {
    const HAZARD_NULL: Self = Self::new(u120::new(0), false, false, false, u4::new(0));

    #[inline]
    fn into_prefix(self, value: bool, bits: Option<usize>) -> Self {
        match bits {
            Some(bits) if bits < (self.len().value() as usize) << 3 => {
                let prefix = Le128::extract(self.into_raw(), bits);
                Self::new(
                    unsafe { u120::new_unchecked(prefix) },
                    !value,
                    value,
                    false,
                    u4::new((bits >> 3) as u8),
                )
            }
            Some(_) | None => self.with_node(!value).with_value(value),
        }
    }

    #[inline]
    fn is_active(self) -> bool {
        // Protects either values or nodes
        self.into_raw() & const { 0b11u128 << 120 } > 0
    }

    #[inline]
    fn is_conflict(self, hazards: &[Self; 4]) -> bool {
        hazards.iter().any(|hazard| self.is_conflict(*hazard))
    }

    #[inline]
    fn is_node(self) -> bool {
        self.is_node()
    }

    #[inline]
    fn is_value(self) -> bool {
        self.is_value()
    }

    #[inline]
    fn bytes(&self) -> usize {
        self.len().value() as usize
    }

    /// For measurement purposes only
    #[inline]
    fn age(self) -> u8 {
        (self.prefix().value() >> 112) as u8
    }

    /// For measurement purposes only
    #[inline]
    fn with_age(self, age: u8) -> Self {
        self.with_prefix(
            self.prefix()
                .bitand(const { u120::new(u112::MAX.value()) })
                .bitor(u120::new((age as u128) << 112)),
        )
    }
}

impl Le128Packed {
    #[inline]
    fn is_conflict(self, hazard: Self) -> bool {
        validate!(self.is_node() ^ self.is_value());

        // Case: `hazard` doesn't protect node or value
        if (hazard.into_raw() & self.into_raw()) & const { 0b11u128 << 120 } == 0 {
            return false;
        }

        // Case: `hazard` protects prefixes only, and `prefix` is higher up the tree
        if !hazard.is_overlap() && hazard.len() > self.len() {
            return false;
        }

        let len = self.len().min(hazard.len());
        let bits = (len.value() as usize) << 3;
        Le128::extract(self.into_raw() ^ hazard.into_raw(), bits) == 0
    }
}
