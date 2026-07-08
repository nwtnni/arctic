use core::arch::x86_64::__m128i;
use core::arch::x86_64::__m256i;
use core::arch::x86_64::_mm_adds_epu8;
use core::arch::x86_64::_mm_blend_epi16;
use core::arch::x86_64::_mm_cmpeq_epi8;
use core::arch::x86_64::_mm_cmpeq_epi16;
use core::arch::x86_64::_mm_cmplt_epi8;
use core::arch::x86_64::_mm_cvtsi128_si64x;
use core::arch::x86_64::_mm_max_epu8;
use core::arch::x86_64::_mm_max_epu16;
use core::arch::x86_64::_mm_min_epu8;
use core::arch::x86_64::_mm_min_epu16;
use core::arch::x86_64::_mm_movemask_epi8;
use core::arch::x86_64::_mm_set_epi64x;
use core::arch::x86_64::_mm_set1_epi8;
use core::arch::x86_64::_mm_set1_epi16;
use core::arch::x86_64::_mm_setr_epi8;
use core::arch::x86_64::_mm_shuffle_epi8;
use core::arch::x86_64::_mm_unpackhi_epi8;
use core::arch::x86_64::_mm_unpacklo_epi8;
use core::arch::x86_64::_mm256_blend_epi16;
use core::arch::x86_64::_mm256_cvtepi8_epi16;
use core::arch::x86_64::_mm256_extracti128_si256;
use core::arch::x86_64::_mm256_max_epu16;
use core::arch::x86_64::_mm256_min_epu16;
use core::arch::x86_64::_mm256_or_si256;
use core::arch::x86_64::_mm256_permute2x128_si256;
use core::arch::x86_64::_mm256_set_m128i;
use core::arch::x86_64::_mm256_setr_epi8;
use core::arch::x86_64::_mm256_setr_m128i;
use core::arch::x86_64::_mm256_shuffle_epi8;
use core::arch::x86_64::_mm256_store_si256;
use core::arch::x86_64::_pext_u64;
use core::ptr::NonNull;

use ribbit::u2;
use ribbit::u4;

use crate::raw::node::KeyIter3;
use crate::raw::node::KeyIter15;
use crate::raw::node::KeyIter47;
use crate::raw::node::iter::KeyIndex;

/// https://richardstartin.github.io/posts/finding-bytes
/// https://orlp.net/blog/extracting-depositing-bits/
/// https://lemire.me/blog/2022/01/21/swar-explained-parsing-eight-digits/
/// https://lamport.azurewebsites.net/pubs/multiple-byte.pdf
#[inline]
pub(super) fn get_3(array: u64, key: u8) -> u8 {
    const LOWER: u64 = 0x0000_00FF_00FF_00FF;
    const OVERFLOW: u64 = 0x0000_0100_0100_0100;

    let key = key as u64;
    // LLVM is smart enough to turn this into an `imul`
    let key = key | (key << 16) | (key << 32);

    // Convert key bytes to zero
    let key_to_zero = array ^ key;

    // Set overflow bit for byte if byte is non-zero
    let equal_zero = key_to_zero + LOWER;

    // Extract overflow bits
    unsafe { core::arch::x86_64::_pext_u64(equal_zero, OVERFLOW) }.trailing_ones() as u8
}

#[inline]
pub(super) fn get_15(array: u128, key: u8) -> u8 {
    let array = u128_to_avx(array);
    let key = unsafe { _mm_set1_epi8(key as i8) };
    let mask = unsafe { _mm_cmpeq_epi8(array, key) };
    unsafe { _mm_movemask_epi8(mask) }.trailing_zeros() as u8
}

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
pub(super) fn sort_3(iter: &mut KeyIter3) {
    let len = iter.0.tail;
    if len <= 1 {
        return;
    }
    let fill = u64::MAX << (len << 4);

    {
        let sorted = NonNull::from(&mut *iter).cast::<u64>();
        unsafe { sorted.write(bitonic_sort_4(sorted.read() | fill)) };
    }
    iter.0.head = 0;
    iter.0.tail = len;
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
pub(super) fn sort_15(out: &mut KeyIter15) {
    let len = out.0.tail;

    // Fill unused bytes with 0xFF
    let fill = unsafe { _mm256_cvtepi8_epi16(u128_to_avx(!mask_len(len))) };

    {
        let sorted = NonNull::from(&mut *out).cast::<__m256i>();

        let iter = unsafe { bitonic_sort_16(_mm256_or_si256(sorted.read(), fill), len) };

        unsafe {
            sorted.write(iter);
        }
    }
    out.0.head = 0;
    out.0.tail = len;
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

/// https://en.wikipedia.org/wiki/Bitonic_sorter
/// https://github.com/Geolm/simd_bitonic
/// https://hal.inria.fr/hal-01512970v1/document
#[inline]
fn bitonic_sort_4(input: u64) -> u64 {
    const RECOMBINE_1: u64 = 0x2301;
    const SORT_1: u64 = RECOMBINE_1;
    const BLEND_1: i32 = 0b1010;

    const RECOMBINE_2: u64 = 0x0123;
    const BLEND_2: i32 = 0b1100;

    #[inline]
    fn bitonic_step<const SHUFFLE: u64, const BLEND: i32>(input: __m128i) -> __m128i {
        const fn extract(shuffle: u64, index: u8) -> i8 {
            // `% 8` to repeat across lanes, `/ 2` for u16 granularity, `* 4` for bit width
            let shift = (index % 8 / 2) * 4;
            let select = (shuffle >> shift) & 0b1111;
            // Mix bit from top/bottom u16 back in
            ((select << 1) | (index as u64 & 1)) as i8
        }

        let shuffle = unsafe {
            _mm_shuffle_epi8(
                input,
                _mm_setr_epi8(
                    const { extract(SHUFFLE, 0) },
                    const { extract(SHUFFLE, 1) },
                    const { extract(SHUFFLE, 2) },
                    const { extract(SHUFFLE, 3) },
                    const { extract(SHUFFLE, 4) },
                    const { extract(SHUFFLE, 5) },
                    const { extract(SHUFFLE, 6) },
                    const { extract(SHUFFLE, 7) },
                    const { extract(SHUFFLE, 8) },
                    const { extract(SHUFFLE, 9) },
                    const { extract(SHUFFLE, 10) },
                    const { extract(SHUFFLE, 11) },
                    const { extract(SHUFFLE, 12) },
                    const { extract(SHUFFLE, 13) },
                    const { extract(SHUFFLE, 14) },
                    const { extract(SHUFFLE, 15) },
                ),
            )
        };

        let min = unsafe { _mm_min_epu16(input, shuffle) };
        let max = unsafe { _mm_max_epu16(input, shuffle) };

        unsafe { _mm_blend_epi16::<BLEND>(min, max) }
    }

    let mut input = unsafe { _mm_set_epi64x(0, input as i64) };

    input = bitonic_step::<RECOMBINE_1, BLEND_1>(input);
    input = bitonic_step::<RECOMBINE_2, BLEND_2>(input);
    input = bitonic_step::<SORT_1, BLEND_1>(input);

    (unsafe { _mm_cvtsi128_si64x(input) } as u64)
}

/// https://en.wikipedia.org/wiki/Bitonic_sorter
/// https://github.com/Geolm/simd_bitonic
/// https://hal.inria.fr/hal-01512970v1/document
#[inline]
fn bitonic_sort_16(mut input: __m256i, len: u8) -> __m256i {
    const RECOMBINE_1: u64 = 0x6745_2301;
    const SORT_1: u64 = RECOMBINE_1;
    const BLEND_1: i32 = 0b1010_1010;

    const RECOMBINE_2: u64 = 0x4567_0123;
    const SORT_2: u64 = 0x5476_1032;
    const BLEND_2: i32 = 0b1100_1100;

    const RECOMBINE_4: u64 = 0x0123_4567;
    const SORT_4: u64 = 0x3210_7654;
    const BLEND_4: i32 = 0b1111_0000;

    const RECOMBINE_8: u64 = 0x0123_4567;
    const BLEND_8: i32 = 0b1111_1111;

    #[inline]
    fn bitonic_step<const SHUFFLE: u64, const BLEND: i32>(input: __m256i) -> __m256i {
        const fn extract(shuffle: u64, index: u8) -> i8 {
            // `% 16` to repeat across lanes, `/ 2` for u16 granularity, `* 4` for bit width
            let shift = (index % 16 / 2) * 4;
            let select = (shuffle >> shift) & 0b1111;
            // Mix bit from top/bottom u16 back in
            ((select << 1) | (index as u64 & 1)) as i8
        }

        // Shuffling across lanes requires different intrinsic
        let swap = if BLEND == BLEND_8 {
            unsafe { _mm256_permute2x128_si256::<0b0000_0001>(input, input) }
        } else {
            input
        };

        let shuffle = unsafe {
            _mm256_shuffle_epi8(
                swap,
                _mm256_setr_epi8(
                    const { extract(SHUFFLE, 0) },
                    const { extract(SHUFFLE, 1) },
                    const { extract(SHUFFLE, 2) },
                    const { extract(SHUFFLE, 3) },
                    const { extract(SHUFFLE, 4) },
                    const { extract(SHUFFLE, 5) },
                    const { extract(SHUFFLE, 6) },
                    const { extract(SHUFFLE, 7) },
                    const { extract(SHUFFLE, 8) },
                    const { extract(SHUFFLE, 9) },
                    const { extract(SHUFFLE, 10) },
                    const { extract(SHUFFLE, 11) },
                    const { extract(SHUFFLE, 12) },
                    const { extract(SHUFFLE, 13) },
                    const { extract(SHUFFLE, 14) },
                    const { extract(SHUFFLE, 15) },
                    const { extract(SHUFFLE, 16) },
                    const { extract(SHUFFLE, 17) },
                    const { extract(SHUFFLE, 18) },
                    const { extract(SHUFFLE, 19) },
                    const { extract(SHUFFLE, 20) },
                    const { extract(SHUFFLE, 21) },
                    const { extract(SHUFFLE, 22) },
                    const { extract(SHUFFLE, 23) },
                    const { extract(SHUFFLE, 24) },
                    const { extract(SHUFFLE, 25) },
                    const { extract(SHUFFLE, 26) },
                    const { extract(SHUFFLE, 27) },
                    const { extract(SHUFFLE, 28) },
                    const { extract(SHUFFLE, 29) },
                    const { extract(SHUFFLE, 30) },
                    const { extract(SHUFFLE, 31) },
                ),
            )
        };

        let min = unsafe { _mm256_min_epu16(input, shuffle) };
        let max = unsafe { _mm256_max_epu16(input, shuffle) };

        if BLEND == BLEND_8 {
            unsafe {
                _mm256_set_m128i(
                    _mm256_extracti128_si256::<1>(max),
                    _mm256_extracti128_si256::<0>(min),
                )
            }
        } else {
            unsafe { _mm256_blend_epi16::<BLEND>(min, max) }
        }
    }

    input = bitonic_step::<RECOMBINE_1, BLEND_1>(input);

    input = bitonic_step::<RECOMBINE_2, BLEND_2>(input);
    input = bitonic_step::<SORT_1, BLEND_1>(input);

    input = bitonic_step::<RECOMBINE_4, BLEND_4>(input);
    input = bitonic_step::<SORT_2, BLEND_2>(input);
    input = bitonic_step::<SORT_1, BLEND_1>(input);

    if len <= 8 {
        input
    } else {
        input = bitonic_step::<RECOMBINE_8, BLEND_8>(input);
        input = bitonic_step::<SORT_4, BLEND_4>(input);
        input = bitonic_step::<SORT_2, BLEND_2>(input);
        bitonic_step::<SORT_1, BLEND_1>(input)
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
    if L::UNBOUND && U::UNBOUND {
        return u128::MAX;
    }

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
    use core::arch::x86_64::__m256i;
    use core::arch::x86_64::_mm256_loadu_si256;
    use core::arch::x86_64::_mm256_set_epi16;
    use core::arch::x86_64::_mm256_setr_epi16;

    use crate::raw::node::simd::avx2::bitonic_sort_16;

    crate::raw::node::simd::tests::impl_suite!(avx2);

    #[test]
    fn sort_zero() {
        use core::arch::x86_64::_mm256_set1_epi16;
        let input = unsafe { _mm256_set1_epi16(0) };
        assert_sort(input, input)
    }

    #[test]
    fn sort_ordered() {
        let input =
            unsafe { _mm256_setr_epi16(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15) };
        assert_sort(input, input)
    }

    #[test]
    fn sort_reverse() {
        let input =
            unsafe { _mm256_set_epi16(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15) };
        let output =
            unsafe { _mm256_setr_epi16(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15) };
        assert_sort(input, output)
    }

    #[test]
    fn sort_regression() {
        let input = unsafe { _mm256_setr_epi16(3, 4, 2, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5) };
        let output = unsafe { _mm256_setr_epi16(2, 3, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5) };
        assert_sort(input, output)
    }

    // Example from https://inria.hal.science/hal-01512970v1/document
    #[test]
    fn sort_8() {
        let input = unsafe { _mm256_setr_epi16(6, 7, 8, 5, 2, 1, 4, 5, 9, 9, 9, 9, 9, 9, 9, 9) };
        let output = unsafe { _mm256_setr_epi16(1, 2, 4, 5, 5, 6, 7, 8, 9, 9, 9, 9, 9, 9, 9, 9) };
        assert_sort(input, output)
    }

    // https://en.wikipedia.org/wiki/Sorting_network#Zero-one_principle
    #[test]
    fn sort_exhaustive_zero_one() {
        let mut buffer = [0u16; 16];

        for i in 0..=u16::MAX {
            for (j, value) in buffer.iter_mut().enumerate() {
                *value = (i >> j) & 1;
            }

            let input = unsafe { _mm256_loadu_si256(buffer.as_ptr().cast()) };
            buffer.sort_unstable();
            let output = unsafe { _mm256_loadu_si256(buffer.as_ptr().cast()) };
            assert_sort(input, output)
        }
    }

    fn assert_sort(input: __m256i, expected: __m256i) {
        let actual = bitonic_sort_16(input, 128);
        assert_eq!(
            unsafe { core::mem::transmute::<__m256i, [u16; 16]>(actual) },
            unsafe { core::mem::transmute::<__m256i, [u16; 16]>(expected) },
        )
    }
}
