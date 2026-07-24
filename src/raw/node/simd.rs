//! SIMD acceleration for node operations.

#[cfg(target_feature = "avx2")]
mod avx2;

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::{__m128i, __m256i, __mmask16};

use fearless_simd::Select;
use fearless_simd::Simd;
use fearless_simd::SimdBase as _;
use fearless_simd::SimdInt;
use fearless_simd::SimdInto as _;
use fearless_simd::SimdMask;
use fearless_simd::mask8x16;
use fearless_simd::mask16x16;
use fearless_simd::u8x16;
use fearless_simd::u8x32;
use fearless_simd::u16x16;
use ribbit::u2;
use ribbit::u4;

use crate::raw::node;
use crate::raw::node::KeyIter3;
use crate::raw::node::KeyIter47;
use crate::raw::node::iter::KeyIndex;

#[inline]
pub(super) fn min_3<L: node::Lower>(keys: u64, len: u2, lower: L) -> Option<KeyIndex> {
    simd!(
        "opt-no-node3-keys",
        avx2::min_3(keys, len, lower),
        min_3_fallback(keys, len, lower),
    )
}

#[inline]
fn min_3_fallback<L: node::Lower>(keys: u64, len: u2, lower: L) -> Option<KeyIndex> {
    iter_3(keys, len, lower, node::Unbound::<()>::default()).min()
}

#[inline]
pub(super) fn max_3<U: node::Upper>(keys: u64, len: u2, upper: U) -> Option<KeyIndex> {
    simd!(
        "opt-no-node3-keys",
        avx2::max_3(keys, len, upper),
        max_3_fallback(keys, len, upper),
    )
}

#[inline]
fn max_3_fallback<U: node::Upper>(keys: u64, len: u2, upper: U) -> Option<KeyIndex> {
    iter_3(keys, len, node::Unbound::<()>::default(), upper).max()
}

#[inline]
pub(super) fn min_15<L: node::Lower>(keys: u128, len: u4, lower: L) -> Option<KeyIndex> {
    simd!(
        "opt-no-node15-keys",
        avx2::min_15(keys, len, lower),
        min_15_fallback(keys, len, lower),
    )
}

#[inline]
fn min_15_fallback<L: node::Lower>(keys: u128, len: u4, lower: L) -> Option<KeyIndex> {
    iter_15(keys, len, lower, node::Unbound::<()>::default()).min()
}

#[inline]
pub(super) fn max_15<U: node::Upper>(keys: u128, len: u4, upper: U) -> Option<KeyIndex> {
    simd!(
        "opt-no-node15-keys",
        avx2::max_15(keys, len, upper),
        max_15_fallback(keys, len, upper),
    )
}

#[inline]
fn max_15_fallback<U: node::Upper>(keys: u128, len: u4, upper: U) -> Option<KeyIndex> {
    iter_15(keys, len, node::Unbound::<()>::default(), upper).max()
}

#[inline]
pub(super) fn keys_3<L: node::Lower, U: node::Upper>(
    keys: u64,
    len: u2,
    lower: L,
    upper: U,
    iter: &mut KeyIter3,
) {
    simd!(
        "opt-no-node3-keys",
        avx2::keys_3(keys, len, lower, upper, iter),
        keys_3_fallback(keys, len, lower, upper, iter),
    )
}

#[inline]
fn keys_3_fallback<L: node::Lower, U: node::Upper>(
    keys: u64,
    len: u2,
    lower: L,
    upper: U,
    iter: &mut KeyIter3,
) {
    let len = core::iter::zip(&mut iter.0.entries, iter_3(keys, len, lower, upper))
        .map(|(out, r#in)| *out = r#in)
        .count();
    iter.0.head = 0;
    iter.0.tail = len as u8;
}

/// https://en.wikipedia.org/wiki/Bitonic_sorter
/// https://github.com/Geolm/simd_bitonic
/// https://hal.inria.fr/hal-01512970v1/document
#[inline(always)]
pub(super) fn sort_u16x16<S: Simd>(simd: S, mut input: u16x16<S>, len: u8) -> u16x16<S> {
    const RECOMBINE_1: u64 = 0x6745_2301;
    const SORT_1: u64 = RECOMBINE_1;
    const SELECT_1: u64 = 0b1010_1010_1010_1010;

    const RECOMBINE_2: u64 = 0x4567_0123;
    const SORT_2: u64 = 0x5476_1032;
    const SELECT_2: u64 = 0b1100_1100_1100_1100;

    const RECOMBINE_4: u64 = 0x0123_4567;
    const SORT_4: u64 = 0x3210_7654;
    const SELECT_4: u64 = 0b1111_0000_1111_0000;

    const RECOMBINE_8: u64 = 0x0123_4567;
    const SELECT_8: u64 = 0b1111_1111_0000_0000;

    const fn decode(pattern: u64, index: u8) -> u8 {
        // `% 16` to repeat across lanes, `/ 2` for u16 granularity, `* 4` for bit width
        let shift = (index % 16 / 2) * 4;
        let select = (pattern >> shift) & 0b1111;
        // Mix bit from top/bottom u16 back in
        ((select << 1) | (index as u64 & 1)) as u8
    }

    #[inline(always)]
    fn bitonic_step<const SWIZZLE: u64, const SELECT: u64, S: Simd>(
        simd: S,
        input: u16x16<S>,
    ) -> u16x16<S> {
        // Lane-crossing comparison requires different code path
        let swap = if SELECT == SELECT_8 {
            // NOTE: didn't see a lane swap method
            let (lower, upper) = simd.split_u16x16(input);
            simd.combine_u16x8(upper, lower)
        } else {
            input
        };

        let swizzle = simd.swizzle_dyn_within_blocks_u16x16(
            swap,
            u8x32::from_fn(simd, |index| decode(SWIZZLE, index as u8)),
        );
        let min = input.min(swizzle);
        let max = input.max(swizzle);
        mask16x16::from_bitmask(simd, SELECT).select(max, min)
    }

    // NOTE: is there a better way to go from bitmask to u16x16?
    let fill = simd.as_array_mask16x16(mask16x16::from_bitmask(simd, !((1 << (len as u64)) - 1)));
    let fill = core::array::from_fn(|index| fill[index] as u16);
    input |= simd.load_array_u16x16(fill);

    input = bitonic_step::<RECOMBINE_1, SELECT_1, _>(simd, input);

    input = bitonic_step::<RECOMBINE_2, SELECT_2, _>(simd, input);
    input = bitonic_step::<SORT_1, SELECT_1, _>(simd, input);

    input = bitonic_step::<RECOMBINE_4, SELECT_4, _>(simd, input);
    input = bitonic_step::<SORT_2, SELECT_2, _>(simd, input);
    input = bitonic_step::<SORT_1, SELECT_1, _>(simd, input);

    if len <= 8 {
        return input;
    }

    input = bitonic_step::<RECOMBINE_8, SELECT_8, _>(simd, input);
    input = bitonic_step::<SORT_4, SELECT_4, _>(simd, input);
    input = bitonic_step::<SORT_2, SELECT_2, _>(simd, input);
    bitonic_step::<SORT_1, SELECT_1, _>(simd, input)
}

/// Compress and interleave bytes from `lo` and `hi` specified by `mask` into
/// the lowest positions in the output register. The value of higher positions
/// is arbitrary.
#[inline(always)]
pub(super) fn compress_u8x16<S: Simd>(
    simd: S,
    mask: mask8x16<S>,
    lower: u8x16<S>,
    upper: u8x16<S>,
) -> u16x16<S> {
    #[cfg(target_arch = "x86_64")]
    if let Some(avx512) = simd.level().as_avx512() {
        return compress_u8x16_avx512(avx512, mask.into(), lower.into(), upper.into())
            .simd_into(simd);
    }

    #[cfg(target_arch = "x86_64")]
    if let Some(avx2) = simd.level().as_avx2() {
        return compress_u8x16_avx2(avx2, mask.into(), lower.into(), upper.into()).simd_into(simd);
    }

    todo!()
}

fearless_simd::kernel! {
    fn compress_u8x16_avx512(
        avx512: Avx512,
        mask: __mmask16,
        lower: __m128i,
        upper: __m128i,
    ) -> __m256i {
        core::arch::x86_64::_mm256_mask_compress_epi16(
            avx512.splat_u8x32(0).into(),
            mask,
            interleave(
                avx512,
                lower.simd_into(avx512),
                upper.simd_into(avx512),
            ).into()
        )
    }
}

fearless_simd::kernel! {
    // https://stackoverflow.com/a/36951611
    // https://stackoverflow.com/a/61431303
    fn compress_u8x16_avx2(
        avx2: Avx2,
        mask: __mmask16,
        lower: __m128i,
        upper: __m128i,
    ) -> __m256i {
        use core::arch::x86_64::_pdep_u64;
        use core::arch::x86_64::_pext_u64;
        use core::arch::x86_64::_mm_cvtepu8_epi16;
        use core::arch::x86_64::_mm_cvtsi64_si128;

        let mask: mask8x16<_> = mask.simd_into(avx2);

        // Expand each bit to a nibble
        let mask = _pdep_u64(mask.to_bitmask(), 0x1111_1111_1111_1111)  * 0xF;

        // Select and compress masked nibbles
        let swizzle = _pext_u64(0xFEDC_BA98_7654_3210, mask);

        // Expand swizzle to low u8 of each u16 lane
        let swizzle: u8x16<_> = _mm_cvtepu8_epi16(_mm_cvtsi64_si128(swizzle as i64)).simd_into(avx2);

        // Shift high nibble of low u8 to low nibble of high u8
        let swizzle = (swizzle | avx2.cvt_to_bytes_u16x8(avx2.cvt_from_bytes_u16x8(swizzle) << 4))
            & avx2.splat_u8x16(0x0F);

        let lower = avx2.swizzle_dyn_within_blocks_u8x16(lower.simd_into(avx2), swizzle);
        let upper = avx2.swizzle_dyn_within_blocks_u8x16(upper.simd_into(avx2), swizzle);
        interleave(avx2, lower, upper).into()
    }
}

#[inline(always)]
pub(super) fn interleave<S: Simd>(simd: S, lower: u8x16<S>, upper: u8x16<S>) -> u16x16<S> {
    let (lower, upper) = simd.interleave_u8x16(lower, upper);
    let combined = simd.combine_u8x16(lower, upper);
    simd.cvt_from_bytes_u16x16(combined)
}

#[inline]
pub(super) fn keys_47<L: node::Lower, U: node::Upper>(
    indices: [u128; 16],
    len: u8,
    lower: L,
    upper: U,
    out: &mut KeyIter47,
) {
    simd!(
        "opt-no-node47-keys",
        avx2::keys_47(indices, len, lower, upper, out),
        keys_47_fallback(indices, len, lower, upper, out),
    )
}

#[inline]
pub(super) fn keys_47_fallback<L: node::Lower, U: node::Upper>(
    indices: [u128; 16],
    len: u8,
    lower: L,
    upper: U,
    out: &mut KeyIter47,
) {
    let i = lower.get() / 16;
    let j = upper.get() / 16;

    let len = indices[i as usize..=j as usize]
        .iter()
        .flat_map(|chunk| chunk.to_le_bytes())
        // HACK: using `i: u8` here causes integer overflow in debug mode
        // when all 256 bytes are loaded
        .zip((i as u16 * 16)..)
        .map(|(index, key)| (index, key as u8))
        .filter(|(index, key)| *index < len && *key >= lower.get() && *key <= upper.get())
        .zip(&mut out.0.entries)
        .map(|((index, key), out)| {
            out.index = index;
            out.key = key;
        })
        .count();

    out.0.head = 0;
    out.0.tail = len as u8;
}

fn iter_3<L: node::Lower, U: node::Upper>(
    keys: u64,
    len: u2,
    lower: L,
    upper: U,
) -> impl Iterator<Item = KeyIndex> {
    keys.to_le_bytes()
        .into_iter()
        .step_by(2)
        .take(len.value() as usize)
        .enumerate()
        .filter(move |(_, key)| *key >= lower.get())
        .filter(move |(_, key)| *key <= upper.get())
        .map(|(index, key)| KeyIndex {
            index: index as u8,
            key,
        })
}

fn iter_15<L: node::Lower, U: node::Upper>(
    keys: u128,
    len: u4,
    lower: L,
    upper: U,
) -> impl Iterator<Item = KeyIndex> {
    keys.to_le_bytes()
        .into_iter()
        .take(len.value() as usize)
        .enumerate()
        .filter(move |(_, key)| *key >= lower.get())
        .filter(move |(_, key)| *key <= upper.get())
        .map(|(index, key)| KeyIndex {
            index: index as u8,
            key,
        })
}

#[cfg(test)]
mod tests {
    /// Correctness properties that hold for sequential executions.
    pub(crate) mod sequential {
        use fearless_simd::Simd;
        use fearless_simd::SimdFrom as _;
        use fearless_simd::u16x16;
        use ribbit::u2;

        use crate::raw::node::KeyIter47;
        use crate::raw::node::iter::KeyIter3;
        use crate::raw::node::node_3;
        use crate::raw::node::node_47;
        use crate::raw::node::simd;
        use crate::raw::node::simd::sort_u16x16;

        #[cfg(feature = "proptest")]
        proptest::proptest! {
            #![proptest_config(proptest::test_runner::Config::with_cases(100_000))]

            #[test]
            fn sort_u16x16_correct(input in proptest::collection::vec(u16::MIN..=u16::MAX, 0..=16)) {
                use fearless_simd::SimdBase as _;

                let actual = fearless_simd::dispatch!(*crate::raw::SIMD, simd => {
                    let len = input.len() as u8;
                    let input = u16x16::from_fn(simd, |index| input.get(index).copied().unwrap_or(0));
                    let output = sort_u16x16(simd, input, len);
                    simd.as_array_u16x16(output)
                });

                let mut expected = input.clone();
                expected.sort_unstable();

                assert_eq!(&actual[..expected.len()], expected);
            }
        }

        // https://en.wikipedia.org/wiki/Sorting_network#Zero-one_principle
        #[test]
        fn sort_u16x16_zero_one() {
            fearless_simd::dispatch!(*crate::raw::SIMD, simd =>{
                let mut buffer = [0u16; 16];

                for i in 0..=u16::MAX {
                    for (j, value) in buffer.iter_mut().enumerate() {
                        *value = (i >> j) & 1;
                    }

                    let actual = simd.as_array_u16x16(sort_u16x16(simd, u16x16::simd_from(simd, buffer), 16));
                    buffer.sort_unstable();
                    assert_eq!(actual, buffer)
                }
            });
        }

        /// `keys_3` matches output of fallback.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn keys_3_correct<F: Fn(u64, u2, Option<u8>, Option<u8>, &mut KeyIter3)>(
            header: node_3::Header,
            lower: u8,
            upper: u8,
            keys_3: F,
        ) {
            let header = ribbit::Pack::pack(header);
            let raw = header.into_raw();
            let len = header.len();

            let mut simd = KeyIter3::default();
            keys_3(raw, len, Some(lower), Some(upper), &mut simd);

            let mut fallback = KeyIter3::default();
            simd::keys_3_fallback(raw, len, Some(lower), Some(upper), &mut fallback);

            assert_eq!(
                simd, fallback,
                "SIMD does not match fallback for keys {header:#x?}, lower {lower:#x?}, upper {upper:#x?}",
            );
        }

        /// `keys_47` matches output of fallback.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn keys_47_correct<
            F: Fn([u128; 16], u8, Option<u8>, Option<u8>, &mut KeyIter47),
        >(
            header: node_47::Header,
            lower: u8,
            upper: u8,
            keys_47: F,
        ) {
            let indices = header.indices();
            let len = header.len();

            let mut simd = KeyIter47::default();
            keys_47(indices, len, Some(lower), Some(upper), &mut simd);

            let mut fallback = KeyIter47::default();
            simd::keys_47_fallback(indices, len, Some(lower), Some(upper), &mut fallback);

            assert_eq!(
                simd, fallback,
                "SIMD does not match fallback for keys {header:#x?}, lower {lower:#x?}, upper {upper:#x?}",
            );
        }
    }

    macro_rules! impl_suite {
        ($mod:ident) => {
            #[cfg(feature = "proptest")]
            mod sequential {
                use proptest::arbitrary::any_with;
                use ribbit::Integer as _;
                use ribbit::u2;

                use crate::raw::node::iter::bound;
                use crate::raw::node::node_3;
                use crate::raw::node::node_47;
                use crate::raw::node::simd::tests::sequential;
                use crate::raw::node::simd::$mod;

                proptest::proptest! {
                    #![proptest_config(proptest::test_runner::Config::with_cases(100_000))]

                    #[test]
                    fn keys_3_correct(
                        header in any_with::<node_3::Header>((u2::new(0), u2::MAX)),
                        (lower, upper) in bound()
                    ) {
                        sequential::keys_3_correct(header, lower, upper, $mod::keys_3)
                    }

                    #[test]
                    fn keys_47(
                        header in any_with::<node_47::Header>((1, 47)),
                        (lower, upper) in bound()
                    ) {
                        sequential::keys_47_correct(header, lower, upper, $mod::keys_47)
                    }
                }
            }
        };
    }
    pub(crate) use impl_suite;
}
