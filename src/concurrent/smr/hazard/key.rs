use crate::NonNullSlice;
use crate::NonPrefixSlice;
use crate::NullTerminatedSlice;
use crate::concurrent::smr::hazard;
use crate::raw;
use crate::raw::Int;
use crate::raw::key;
use crate::raw::key::Len;
use crate::raw::key::Read as _;
use crate::raw::key::Terminate;

pub trait Key: raw::Key {
    #[expect(private_bounds)]
    type Prefix: ribbit::Pack<Packed: hazard::Prefix>;

    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix>;
}

type Le = hazard::prefix::Le128;

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

impl<T: Terminate, K: for<'k> raw::Key<Read<'k> = key::vec::Reader<'k, T>>> Key for K {
    type Prefix = Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_vec(reader)
    }
}

macro_rules! impl_slice {
    ($ty:ty) => {
        impl Key for &'_ $ty {
            type Prefix = Le;

            #[inline]
            fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
                hazard_slice(reader)
            }
        }
    };
}

impl_slice!(NonPrefixSlice);
impl_slice!(NonNullSlice);
impl_slice!(NullTerminatedSlice);

#[inline]
fn hazard_vec<T: key::Terminate>(reader: key::vec::Reader<'_, T>) -> ribbit::Packed<Le> {
    let prefix = if reader.slice.len() >= 16 {
        unsafe { reader.slice.as_ptr().cast::<u128>().read_unaligned() }
    } else {
        let mut buffer = [0u8; 16];
        buffer[..reader.slice.len()].copy_from_slice(reader.slice);
        u128::from_le_bytes(buffer)
    };

    Le::new_hazard(prefix, reader.len().bytes().min(15) << 3)
}

#[inline]
fn hazard_slice<T: key::Terminate>(reader: key::slice::Reader<'_, T>) -> ribbit::Packed<Le> {
    hazard_vec(reader.0)
}

impl<const N: usize> Key for [u8; N] {
    type Prefix = Le;

    #[inline]
    fn hazard(reader: Self::Read<'_>) -> ribbit::Packed<Self::Prefix> {
        hazard_vec(reader.0)
    }
}
