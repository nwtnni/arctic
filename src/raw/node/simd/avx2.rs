use core::arch::x86_64::__m128i;
use core::arch::x86_64::_mm_cmpeq_epi16;
use core::arch::x86_64::_mm_cvtsi128_si64x;
use core::arch::x86_64::_mm_max_epu16;
use core::arch::x86_64::_mm_min_epu16;
use core::arch::x86_64::_mm_set1_epi16;
use core::arch::x86_64::_pext_u64;
use core::ptr::NonNull;

use ribbit::u2;
use ribbit::u4;

use crate::raw::node::KeyIter3;
use crate::raw::node::iter::KeyIndex;

#[inline]
pub(super) fn min_3<L: crate::raw::node::Lower>(
    _keys: u64,
    _len: u2,
    _lower: L,
) -> Option<KeyIndex> {
    todo!()
}

#[inline]
pub(super) fn max_3<U: crate::raw::node::Upper>(
    _keys: u64,
    _len: u2,
    _upper: U,
) -> Option<KeyIndex> {
    todo!()
}

#[inline]
pub(super) fn min_15<L: crate::raw::node::Lower>(
    _keys: u128,
    _len: u4,
    _lower: L,
) -> Option<KeyIndex> {
    todo!()
}

#[inline]
pub(super) fn max_15<U: crate::raw::node::Upper>(
    _keys: u128,
    _len: u4,
    _upper: U,
) -> Option<KeyIndex> {
    todo!()
}

#[inline]
pub(super) fn keys_3<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
    keys: u64,
    len: u2,
    lower: L,
    upper: U,
    out: &mut KeyIter3,
) {
    let mut bits = len.value() << 4;
    let mut iter = (keys << 8) | 0x0002_0001_0000;

    if lower.get() > u8::MIN || upper.get() < u8::MAX {
        let mask_len = !(u64::MAX << bits);
        let mask_range = mask_range_4(keys, lower.get(), upper.get());
        let mask_valid = mask_len & mask_range;

        iter = unsafe { _pext_u64(iter, mask_valid) };
        bits = mask_valid.count_ones() as u8;
    };

    unsafe { NonNull::from(&mut *out).cast::<u64>().write(iter) };
    out.0.head = 0;
    out.0.tail = bits >> 4;

    // HACK: make it easier to test against fallback
    if_validate! {
        out.0.entries[out.0.tail as usize..].iter_mut().for_each(|entry| {
            entry.key = 0;
            entry.index = 0;
        })
    }
}

#[inline]
fn mask_range_4(array: u64, min: u8, max: u8) -> u64 {
    let array = u128_to_avx(array as u128);

    let min = unsafe { _mm_set1_epi16(min as i16) };
    let max = unsafe { _mm_set1_epi16(max as i16) };

    let clamp_min = unsafe { _mm_max_epu16(array, min) };
    let clamp = unsafe { _mm_min_epu16(clamp_min, max) };
    let valid = unsafe { _mm_cmpeq_epi16(array, clamp) };

    (unsafe { _mm_cvtsi128_si64x(valid) } as u64)
}

#[inline]
const fn u128_to_avx(value: u128) -> __m128i {
    unsafe { core::mem::transmute::<u128, __m128i>(value) }
}

#[cfg(test)]
mod tests {
    crate::raw::node::simd::tests::impl_suite!(avx2);
}
