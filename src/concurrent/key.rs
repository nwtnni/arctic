use std::ffi::CString;

use crate::NonNullString;
use crate::NonPrefixVec;
use crate::concurrent::smr::hazard;
use crate::raw;
use crate::raw::Int;
use crate::raw::key;
use crate::raw::key::Len;
use crate::raw::key::Read as _;

pub trait Key: raw::Key {
    type Prefix: ribbit::Pack<Packed: hazard::Prefix>;

    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix>;
}

type Le = hazard::prefix::Le128;

impl Key for NonPrefixVec {
    type Prefix = Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_vec(reader)
    }
}

impl Key for CString {
    type Prefix = Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_vec(reader)
    }
}

impl Key for NonNullString {
    type Prefix = Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_string(reader)
    }
}

impl<const N: usize> Key for [u8; N] {
    type Prefix = Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_vec(reader)
    }
}

impl<'a> Key for &'a crate::raw::key::slice::NonPrefixSlice {
    type Prefix = Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        let reader = unsafe { reader.as_ref() };

        // let mut buffer = [0u8; 8];
        // let len = reader.len().min(7);

        let mut buffer = [0u8; 16];
        let len = reader.len().min(15);

        buffer[..len].copy_from_slice(&reader[..len]);

        // hazard::prefix::Le::new_hazard(u64::from_le_bytes(buffer), len << 3)
        Le::new_hazard(u128::from_le_bytes(buffer), len << 3)
    }
}

macro_rules! impl_integer {
    ($($integer:ty),* $(,)?) => {
        $(
            impl Key for $integer {
                type Prefix = hazard::prefix::Be;

                #[inline]
                fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
                    hazard_integer(reader)
                }
            }
        )*
    }
}

impl_integer!(u16, u32, u128);

#[cfg(feature = "opt-no-int")]
impl Key for u64 {
    type Prefix = hazard::prefix::Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        let len = reader.len().bytes().min(7);
        let reader = reader.buffer;
        let mut buffer = [0u8; 8];
        buffer[..len].copy_from_slice(&reader[..len]);
        hazard::prefix::Le::new_hazard(u64::from_le_bytes(buffer), len << 3)
    }
}

#[cfg(not(feature = "opt-no-int"))]
impl_integer!(u64);

#[inline]
fn hazard_integer<I: Int>(reader: key::int::Reader<I>) -> ribbit::Packed<hazard::prefix::Be> {
    hazard::prefix::Be::new_hazard(
        reader.buffer.most_significant_u64(),
        if I::BITS < 64 {
            Len::bits(reader.len())
        } else {
            Len::bits(reader.len()).min(56)
        },
    )
}

#[inline]
fn hazard_vec<const N: usize>(reader: key::vec::Reader<'_, N>) -> ribbit::Packed<Le> {
    let prefix = if reader.0.len() >= 16 {
        unsafe { reader.0.as_ptr().cast::<u128>().read_unaligned() }
    } else {
        let mut buffer = [0u8; 16];
        buffer[..reader.0.len()].copy_from_slice(reader.0);
        u128::from_le_bytes(buffer)
    };

    Le::new_hazard(prefix, reader.len().bytes().min(15) << 3)
}

#[inline]
fn hazard_string(reader: key::string::Reader<'_>) -> ribbit::Packed<Le> {
    let prefix = if reader.slice.len() >= 16 {
        unsafe { reader.slice.as_ptr().cast::<u128>().read_unaligned() }
    } else {
        let mut buffer = [0u8; 16];
        buffer[..reader.slice.len()].copy_from_slice(reader.slice);
        u128::from_le_bytes(buffer)
    };

    Le::new_hazard(prefix, reader.len().bytes().min(15) << 3)
}
