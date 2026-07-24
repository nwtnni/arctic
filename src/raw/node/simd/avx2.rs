use core::arch::x86_64::__m128i;
use core::arch::x86_64::__m256i;
use core::arch::x86_64::_mm_adds_epu8;
use core::arch::x86_64::_mm_cmpeq_epi8;
use core::arch::x86_64::_mm_cmpeq_epi16;
use core::arch::x86_64::_mm_cmplt_epi8;
use core::arch::x86_64::_mm_cvtsi128_si64x;
use core::arch::x86_64::_mm_max_epu8;
use core::arch::x86_64::_mm_max_epu16;
use core::arch::x86_64::_mm_min_epu8;
use core::arch::x86_64::_mm_min_epu16;
use core::arch::x86_64::_mm_movemask_epi8;
use core::arch::x86_64::_mm_set1_epi8;
use core::arch::x86_64::_mm_set1_epi16;
use core::arch::x86_64::_mm_shuffle_epi8;
use core::arch::x86_64::_mm_unpackhi_epi8;
use core::arch::x86_64::_mm_unpacklo_epi8;
use core::arch::x86_64::_mm256_setr_m128i;
use core::arch::x86_64::_mm256_store_si256;
use core::arch::x86_64::_pext_u64;
use core::ptr::NonNull;

use ribbit::u2;
use ribbit::u4;

use crate::raw::node::KeyIter3;
use crate::raw::node::KeyIter15;
use crate::raw::node::KeyIter47;
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

// https://talkchess.com/viewtopic.php?t=78804
// https://stackoverflow.com/questions/72098296/how-to-create-a-left-packed-vector-of-indices-of-the-0s-in-one-simd-vector
// http://const.me/articles/simd/simd.pdf
#[inline]
pub(super) fn keys_15<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
    keys: u128,
    len: u4,
    lower: L,
    upper: U,
    out: &mut KeyIter15,
) {
    let (iter, len) = if lower.get() > u8::MIN || upper.get() < u8::MAX {
        let mask_len = mask_len(len.value());
        let mask_range = mask_range(keys, lower, upper);
        let mask_valid = mask_len & mask_range;
        compress_16(mask_valid, U8_SEQ, keys)
    } else {
        (interleave(U8_SEQ, keys), len.value())
    };

    unsafe {
        _mm256_store_si256(out as *mut _ as _, iter);
    }
    out.0.head = 0;
    out.0.tail = len;

    // HACK: make it easier to test against fallback
    if_validate! {
        out.0.entries[out.0.tail as usize..].iter_mut().for_each(|entry| {
            entry.key = 0;
            entry.index = 0;
        })
    }
}

#[inline]
pub(super) fn keys_47<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
    indices: [u128; 16],
    len: u8,
    lower: L,
    upper: U,
    out: &mut KeyIter47,
) {
    validate!(len <= 0x7F, "AVX2 only supports signed byte comparison");

    let len_u8 = unsafe { _mm_set1_epi8(len as i8) };
    let mut len = 0;

    #[inline]
    fn keys(i: u8) -> u128 {
        avx_to_u128(unsafe { _mm_adds_epu8(u128_to_avx(U8_SEQ), _mm_set1_epi8((i * 16) as i8)) })
    }

    if lower.get() > u8::MIN || upper.get() < u8::MAX {
        let i = lower.get() / 16;
        let j = upper.get() / 16;

        for (k, indices) in indices[i as usize..=j as usize].iter().copied().enumerate() {
            let keys = keys(i + k as u8);
            let mask_len = avx_to_u128(unsafe { _mm_cmplt_epi8(u128_to_avx(indices), len_u8) });
            let mask_range = mask_range(keys, lower, upper);
            len += unsafe {
                compress_store_47(
                    mask_len & mask_range,
                    indices,
                    keys,
                    &mut out.0.entries[len as usize..],
                )
            };
        }
    } else {
        for (i, indices) in indices.iter().copied().enumerate() {
            let keys = keys(i as u8);
            let mask_len = avx_to_u128(unsafe { _mm_cmplt_epi8(u128_to_avx(indices), len_u8) });
            len += unsafe {
                compress_store_47(mask_len, indices, keys, &mut out.0.entries[len as usize..])
            }
        }
    }

    out.0.head = 0;
    out.0.tail = len;

    // HACK: make it easier to test against fallback
    if_validate! {
        out.0.entries[out.0.tail as usize..].iter_mut().for_each(|entry| {
            entry.key = 0;
            entry.index = 0;
        })
    }
}

/// Compress and interleave bytes from `lo` and `hi` specified by `mask` into
/// the lowest positions in the output register. The value of higher positions
/// is arbitrary.
#[inline]
fn compress_16(mask: u128, lo: u128, hi: u128) -> (__m256i, u8) {
    let mask_bit = mask_byte_to_bit(mask);
    let len = mask_bit.count_ones() as u8;

    cfg_select! {
        all(target_feature = "avx512vbmi2", target_feature = "avx512vl") => unsafe {
            let out = core::arch::x86_64::_mm256_mask_compress_epi16(
                core::arch::x86_64::_mm256_set1_epi8(0),
                mask_bit,
                interleave(lo, hi),
            );
            (out, len)
        }
        // https://stackoverflow.com/a/36951611
        // https://stackoverflow.com/a/61431303
        _ => {
            // Expand each bit to a nibble
            let mask_nibble = unsafe { core::arch::x86_64::_pdep_u64(mask_bit as u64, 0x1111_1111_1111_1111) } * 0xF;

            // Select and compress masked nibbles
            let shuffle = unsafe { _pext_u64(0xFEDC_BA98_7654_3210, mask_nibble) };

            // Expand shuffle to low u8 of each u16 lane
            let shuffle = unsafe { core::arch::x86_64::_mm_cvtepu8_epi16(core::arch::x86_64::_mm_cvtsi64_si128(shuffle as i64)) };

            // Shift high nibble of low u8 to low nibble of high u8
            let shuffle = unsafe {
                core::arch::x86_64::_mm_and_si128(
                    core::arch::x86_64::_mm_or_si128(
                        shuffle,
                        core::arch::x86_64::_mm_slli_epi16::<4>(shuffle)
                    ),
                    _mm_set1_epi8(0x0F),
                )
            };

            let lo = avx_to_u128(unsafe { _mm_shuffle_epi8(u128_to_avx(lo), shuffle)});
            let hi = avx_to_u128(unsafe { _mm_shuffle_epi8(u128_to_avx(hi), shuffle)});
            let out = interleave(lo, hi);
            (out, len)
        }
    }
}

/// # Safety
///
/// Caller must guarantee `out` has length >= 16.
#[inline]
unsafe fn compress_store_47(mask: u128, lo: u128, hi: u128, out: &mut [KeyIndex]) -> u8 {
    validate!(out.len() >= 16);

    cfg_select! {
        // TODO: https://lemire.me/blog/2025/02/14/avx-512-gotcha-avoid-compressing-words-to-memory-with-amd-zen-4-processors/
        all(target_feature = "avx512vbmi2", target_feature = "avx512vl") => {
            let mask_bit = mask_byte_to_bit(mask);

            unsafe {
                core::arch::x86_64::_mm256_mask_compressstoreu_epi16(
                    out.as_mut_ptr().cast::<i16>(),
                    mask_bit,
                    interleave(lo, hi),
                );
            }

            mask_bit.count_ones() as u8
        }
        _ => {
            let (data, len) = compress_16(mask, lo, hi);

            // NOTE: this forces KeyIter63 to take up an extra 32 bytes to
            // avoid out-of-bound stores. Is there a better alternative for AVX2?
            unsafe {
                core::arch::x86_64::_mm256_storeu_si256(
                    out.as_mut_ptr().cast(),
                    data,
                );
            }

            len
        }
    }
}

#[inline]
fn interleave(lo: u128, hi: u128) -> __m256i {
    let lo = u128_to_avx(lo);
    let hi = u128_to_avx(hi);
    unsafe { _mm256_setr_m128i(_mm_unpacklo_epi8(lo, hi), _mm_unpackhi_epi8(lo, hi)) }
}

/// Output has 8 bits set for each byte in `array` that is within `min..=max` (unsigned).
#[inline]
fn mask_range<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
    array: u128,
    lower: L,
    upper: U,
) -> u128 {
    let array = u128_to_avx(array);

    let min = unsafe { _mm_set1_epi8(lower.get() as i8) };
    let max = unsafe { _mm_set1_epi8(upper.get() as i8) };

    // https://stackoverflow.com/a/28383095
    let clamp_min = unsafe { _mm_max_epu8(array, min) };
    let clamp = unsafe { _mm_min_epu8(clamp_min, max) };

    avx_to_u128(unsafe { _mm_cmpeq_epi8(array, clamp) })
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

/// Output has 8 bits set for each byte in `array` below `len`
#[inline]
fn mask_len(len: u8) -> u128 {
    avx_to_u128(unsafe { _mm_cmplt_epi8(u128_to_avx(U8_SEQ), _mm_set1_epi8(len as i8)) })
}

/// Convert byte mask to bit mask
#[inline]
fn mask_byte_to_bit(mask: u128) -> u16 {
    unsafe { _mm_movemask_epi8(u128_to_avx(mask)) as u16 }
}

const U8_SEQ: u128 = 0x0F0E_0D0C_0B0A_0908_0706_0504_0302_0100u128;

#[inline]
const fn avx_to_u128(value: __m128i) -> u128 {
    unsafe { core::mem::transmute::<__m128i, u128>(value) }
}

#[inline]
const fn u128_to_avx(value: u128) -> __m128i {
    unsafe { core::mem::transmute::<u128, __m128i>(value) }
}

#[cfg(test)]
mod tests {
    crate::raw::node::simd::tests::impl_suite!(avx2);
}
