use core::arch::x86_64::__m128i;
use core::arch::x86_64::__m256i;
use core::arch::x86_64::_mm_adds_epu8;
use core::arch::x86_64::_mm_and_si128;
use core::arch::x86_64::_mm_blend_epi16;
use core::arch::x86_64::_mm_cmpeq_epi8;
use core::arch::x86_64::_mm_cmpeq_epi16;
use core::arch::x86_64::_mm_cmplt_epi8;
use core::arch::x86_64::_mm_cvtepu8_epi16;
use core::arch::x86_64::_mm_cvtsi64_si128;
use core::arch::x86_64::_mm_cvtsi128_si64x;
use core::arch::x86_64::_mm_max_epu8;
use core::arch::x86_64::_mm_max_epu16;
use core::arch::x86_64::_mm_min_epu8;
use core::arch::x86_64::_mm_min_epu16;
use core::arch::x86_64::_mm_movemask_epi8;
use core::arch::x86_64::_mm_or_si128;
use core::arch::x86_64::_mm_set_epi64x;
use core::arch::x86_64::_mm_set1_epi8;
use core::arch::x86_64::_mm_set1_epi16;
use core::arch::x86_64::_mm_setr_epi8;
use core::arch::x86_64::_mm_shuffle_epi8;
use core::arch::x86_64::_mm_slli_epi16;
use core::arch::x86_64::_mm_unpackhi_epi8;
use core::arch::x86_64::_mm_unpacklo_epi8;
use core::arch::x86_64::_mm256_blend_epi16;
use core::arch::x86_64::_mm256_extracti128_si256;
use core::arch::x86_64::_mm256_max_epu16;
use core::arch::x86_64::_mm256_min_epu16;
use core::arch::x86_64::_mm256_permute2x128_si256;
use core::arch::x86_64::_mm256_set_m128i;
use core::arch::x86_64::_mm256_setr_epi8;
use core::arch::x86_64::_mm256_setr_m128i;
use core::arch::x86_64::_mm256_shuffle_epi8;
use core::arch::x86_64::_mm256_store_si256;
use core::arch::x86_64::_mm256_storeu_si256;
use core::arch::x86_64::_pdep_u64;
use core::arch::x86_64::_pext_u64;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::u2;
use ribbit::u4;

use crate::raw::node::iter::KeyIndex;
use crate::raw::node::linear::KeyIter3;
use crate::raw::node::linear::KeyIter15;
use crate::raw::node::linear::KeyIter63;

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
pub(super) fn keys_3<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
    keys: u64,
    len: u2,
    lower: L,
    upper: U,
) -> KeyIter3 {
    const INDICES: u64 = 0x0002_0001_0000;

    let mut bits = len.value() << 4;
    let mut entries = (keys << 8) | INDICES;

    if lower.get() > u8::MIN || upper.get() < u8::MAX {
        let mask_len = !(u64::MAX << bits);
        let mask_range = mask_range_4(keys, lower.get(), upper.get());
        let mask_valid = mask_len & mask_range;

        entries = unsafe { _pext_u64(entries, mask_valid) };
        bits = mask_valid.count_ones() as u8;
    };

    let entries = if bits <= 16 {
        entries
    } else {
        bitonic_sort_4(entries | (u64::MAX << bits))
    };

    let mut iter = unsafe { core::mem::transmute::<u64, KeyIter3>(entries) };
    iter.0.head = 0;
    iter.0.tail = bits >> 4;

    // HACK: make it easier to test against fallback
    if_validate! {
        iter.0.entries[iter.0.tail as usize..].iter_mut().for_each(|entry| {
            entry.key = 0;
            entry.index = 0;
        })
    }

    iter
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
    let mask_len = mask_len(len.value());

    let (iter, len) = if lower.get() > u8::MIN || upper.get() < u8::MAX {
        let mask_range = mask_range(keys, lower, upper);
        compress_15(mask_len & mask_range, U8_SEQ, keys)
    } else {
        let fill = !mask_len;
        (interleave(U8_SEQ | fill, keys | fill), len.value())
    };

    let sorted = bitonic_sort_16(iter, len);

    unsafe {
        _mm256_store_si256(out as *mut _ as _, sorted);
    }
    out.0.head = 0;
    out.0.tail = len;
}

#[inline]
pub(super) fn keys_47<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
    indices: &[Atomic<u128>; 16],
    lower: L,
    upper: U,
    len: u8,
    out: &mut KeyIter63,
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

        for (k, indices) in indices[i as usize..=j as usize]
            .iter()
            .map(|indices| indices.load(Ordering::Relaxed))
            .enumerate()
        {
            let keys = keys(i + k as u8);
            let mask_len = avx_to_u128(unsafe { _mm_cmplt_epi8(u128_to_avx(indices), len_u8) });
            let mask_range = mask_range(keys, lower, upper);
            len += unsafe {
                compress_store_47(
                    &mut out.0.entries[len as usize..],
                    mask_len & mask_range,
                    indices,
                    keys,
                )
            };
        }
    } else {
        for (i, indices) in indices
            .iter()
            .map(|indices| indices.load(Ordering::Relaxed))
            .enumerate()
        {
            let keys = keys(i as u8);
            let mask_len = avx_to_u128(unsafe { _mm_cmplt_epi8(u128_to_avx(indices), len_u8) });
            len += unsafe {
                compress_store_47(&mut out.0.entries[len as usize..], mask_len, indices, keys)
            }
        }
    }

    out.0.head = 0;
    out.0.tail = len;
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

#[inline]
fn compress_15(mask: u128, lo: u128, hi: u128) -> (__m256i, u8) {
    let mask_bit = mask_byte_to_bit(mask);
    let len = mask_bit.count_ones() as u8;

    cfg_select! {
        all(target_feature = "avx512vbmi2", target_feature = "avx512vl") => unsafe {
            let out = core::arch::x86_64::_mm256_mask_compress_epi16(
                _mm256_set1_epi8(0xFFu8 as i8),
                mask_bit,
                interleave(lo, hi),
            );
        }
        // https://stackoverflow.com/a/36951611
        // https://stackoverflow.com/a/61431303
        _ => {
            validate!(len < 16);

            // Expand each bit to a nibble
            let mask_nibble = unsafe { _pdep_u64(mask_bit as u64, 0x1111_1111_1111_1111) } * 0xF;

            // Select and compress masked nibbles
            let shuffle = unsafe { _pext_u64(U4_SEQ, mask_nibble) };

            // Ensure non-mask nibbles shuffle to 0xF
            let shuffle = shuffle | (u64::MAX << ((len as u64) * 4));

            // Expand shuffle to low u8 of each u16 lane
            let shuffle = unsafe { _mm_cvtepu8_epi16(_mm_cvtsi64_si128(shuffle as i64)) };

            // Shift high nibble of low u8 to low nibble of high u8
            let shuffle = unsafe {
                _mm_and_si128(_mm_or_si128(shuffle, _mm_slli_epi16::<4>(shuffle)), _mm_set1_epi8(0x0F))
            };

            // Shuffle, ensuring index 0xF contains byte 0xFF for bitonic sort
            // Since we know len < 16, this does not clobber
            const HIGH: u128 = 0xFFu128.rotate_right(8);
            let lo = avx_to_u128(unsafe { _mm_shuffle_epi8(u128_to_avx(lo | HIGH), shuffle)});
            let hi = avx_to_u128(unsafe { _mm_shuffle_epi8(u128_to_avx(hi | HIGH), shuffle)});
            let out = interleave(lo, hi);
        }
    }

    (out, len)
}

/// # Safety
///
/// Caller must guarantee `out` has length >= 16.
#[inline]
unsafe fn compress_store_47(out: &mut [KeyIndex], mask: u128, lo: u128, hi: u128) -> u8 {
    validate!(out.len() >= 16);

    let mask_bit = mask_byte_to_bit(mask);
    let len = mask_bit.count_ones() as u8;

    cfg_select! {
        // TODO: https://lemire.me/blog/2025/02/14/avx-512-gotcha-avoid-compressing-words-to-memory-with-amd-zen-4-processors/
        all(target_feature = "avx512vbmi2", target_feature = "avx512vl") => {
            unsafe {
                core::arch::x86_64::_mm256_mask_compressstoreu_epi16(
                    out.as_mut_ptr().cast::<i16>(),
                    mask_bit,
                    interleave(lo, hi),
                );
            }
        }
        // https://stackoverflow.com/a/36951611
        // https://stackoverflow.com/a/61431303
        _ => {
            // Expand each bit to a nibble
            let mask_nibble = unsafe { _pdep_u64(mask_bit as u64, 0x1111_1111_1111_1111) } * 0xF;

            // Select and compress masked nibbles
            let shuffle = unsafe { _pext_u64(U4_SEQ, mask_nibble) };

            // Expand shuffle to low u8 of each u16 lane
            let shuffle = unsafe { _mm_cvtepu8_epi16(_mm_cvtsi64_si128(shuffle as i64)) };

            // Shift high nibble of low u8 to low nibble of high u8
            let shuffle = unsafe {
                _mm_and_si128(_mm_or_si128(shuffle, _mm_slli_epi16::<4>(shuffle)), _mm_set1_epi8(0x0F))
            };

            let lo = avx_to_u128(unsafe { _mm_shuffle_epi8(u128_to_avx(lo), shuffle)});
            let hi = avx_to_u128(unsafe { _mm_shuffle_epi8(u128_to_avx(hi), shuffle)});
            let data = interleave(lo, hi);

            cfg_select! {
                all(target_feature = "avx512bw", target_feature = "avx512vl") => {
                    unsafe {
                        core::arch::x86_64::_mm256_mask_storeu_epi16(
                            out.as_mut_ptr().cast::<i16>(),
                            mask_bit,
                            data,
                        );
                    }
                }
                _ => {
                    // NOTE: this forces KeyIter63 to take up an extra 32 bytes to
                    // avoid out-of-bound stores. Is there a better alternative for AVX2?
                    unsafe {
                        _mm256_storeu_si256(
                            out.as_mut_ptr().cast(),
                            data,
                        );
                    }
                }
            }
        }
    }

    len
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

const U4_SEQ: u64 = 0xFEDC_BA98_7654_3210;
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
    use core::hash::Hasher as _;

    use ribbit::traits::Integer as _;
    use ribbit::u2;
    use ribbit::u4;

    use crate::raw::node::linear::KeyIter15;
    use crate::raw::node::simd;
    use crate::raw::node::simd::avx2::bitonic_sort_16;

    #[test]
    fn get_3() {
        const COUNT: usize = 100_000;

        let mut hasher = rapidhash::fast::RapidHasher::default_const();

        for i in 0..COUNT {
            hasher.write_usize(i);
            let hash = hasher.finish();
            let array = hash & 0x00FF_00FF_00FF;
            let key = (hash >> 8) as u8;

            let swar = super::get_3(array, key);
            let fallback = simd::get_3_fallback(array, key);

            assert_eq!(
                swar, fallback,
                "SWAR {swar} does not match fallback {fallback} for array {array:x?} and key {key:x?}",
            );
        }
    }

    #[test]
    fn get_15() {
        const COUNT: usize = 100_000;

        let mut hasher = rapidhash::fast::RapidHasher::default_const();

        for i in 0..COUNT {
            hasher.write_usize(i);
            let low = hasher.finish();

            hasher.write_usize(i);
            let high = hasher.finish();

            hasher.write_usize(i);
            let key = hasher.finish() as u8;

            let array = (high as u128) << 64 | (low as u128);
            let simd = super::get_15(array, key);
            let fallback = simd::get_15_fallback(array, key);

            assert_eq!(
                simd, fallback,
                "SIMD does not match fallback for array {array:#x?} and key {key:#x?}",
            );
        }
    }

    #[test]
    fn keys_3() {
        const COUNT: usize = 100_000;

        let mut hasher = rapidhash::fast::RapidHasher::default_const();

        for i in 0..COUNT {
            hasher.write_usize(i);
            let data = hasher.finish();

            let keys = data & 0x00FF_00FF_00FF;
            let len = u2::masked_new(data >> 8);
            let mut low = (data >> 24) as u8;
            let mut high = (data >> 40) as u8;
            if low > high {
                core::mem::swap(&mut low, &mut high);
            }

            let simd = super::keys_3(keys, len, Some(low), Some(high));
            let fallback = simd::keys_3_fallback(keys, len, Some(low), Some(high));

            assert_eq!(
                simd, fallback,
                "SIMD does not match fallback for keys {keys:#x?}, len {len}, low {low:#x?}, high {high:#x?}",
            );
        }
    }

    #[test]
    fn keys_15() {
        const COUNT: usize = 100_000;

        let mut hasher = rapidhash::fast::RapidHasher::default_const();

        for i in 0..COUNT {
            hasher.write_usize(i);
            let low = hasher.finish();
            hasher.write_usize(i);
            let high = hasher.finish();

            let keys = (low as u128) | (high as u128) << 64;

            hasher.write_usize(i);
            let data = hasher.finish();

            let len = u4::masked_new(data);
            let mut low = (data >> 8) as u8;
            let mut high = (data >> 16) as u8;
            if low > high {
                core::mem::swap(&mut low, &mut high);
            }

            let mut simd = KeyIter15::default();
            super::keys_15(keys, len, Some(low), Some(high), &mut simd);
            for (index, entry) in simd.0.entries.iter_mut().enumerate() {
                if entry.key == 0xFF && entry.index == 0xFF {
                    assert!(index >= simd.0.tail as usize);
                    entry.key = 0;
                    entry.index = 0;
                }
            }

            let mut fallback = KeyIter15::default();
            simd::keys_15_fallback(keys, len, Some(low), Some(high), &mut fallback);

            assert_eq!(
                simd, fallback,
                "SIMD does not match fallback for {i}: keys {keys:#x?}, len {len}, low {low:#x?}, high {high:#x?}",
            );
        }
    }

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
