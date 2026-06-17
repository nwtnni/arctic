//! SIMD acceleration for node operations.

#[cfg(target_feature = "avx2")]
mod avx2;

use ribbit::u2;
use ribbit::u4;

use crate::raw::node;
use crate::raw::node::KeyIter3;
use crate::raw::node::KeyIter15;
use crate::raw::node::KeyIter47;
use crate::raw::node::iter::KeyIndex;

#[inline]
pub(super) fn get_3(array: u64, key: u8) -> u8 {
    simd!(
        "opt-no-node3-get",
        avx2::get_3(array, key),
        get_3_fallback(array, key)
    )
}

#[inline]
fn get_3_fallback(array: u64, key: u8) -> u8 {
    array
        .to_le_bytes()
        .into_iter()
        .step_by(2)
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(3)
}

#[inline]
pub(crate) fn get_15(array: u128, key: u8) -> u8 {
    simd!(
        "opt-no-node15-get",
        avx2::get_15(array, key),
        get_15_fallback(array, key)
    )
}

#[inline]
fn get_15_fallback(array: u128, key: u8) -> u8 {
    array
        .to_le_bytes()
        .into_iter()
        .position(|byte| byte == key)
        .map(|index| index as u8)
        .unwrap_or(32)
}

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

#[inline]
pub(super) fn sort_3(iter: &mut KeyIter3) {
    simd!(
        "opt-no-node3-keys",
        avx2::sort_3(iter),
        sort_3_fallback(iter),
    )
}

#[inline]
fn sort_3_fallback(iter: &mut KeyIter3) {
    iter.0.entries[..iter.0.tail as usize].sort_unstable();
}

#[inline]
pub(super) fn keys_15<L: node::Lower, U: node::Upper>(
    keys: u128,
    len: u4,
    lower: L,
    upper: U,
    out: &mut KeyIter15,
) {
    simd!(
        "opt-no-node15-keys",
        avx2::keys_15(keys, len, lower, upper, out),
        keys_15_fallback(keys, len, lower, upper, out),
    )
}

#[inline]
pub(super) fn keys_15_fallback<L: node::Lower, U: node::Upper>(
    keys: u128,
    len: u4,
    lower: L,
    upper: U,
    out: &mut KeyIter15,
) {
    let len = core::iter::zip(&mut out.0.entries, iter_15(keys, len, lower, upper))
        .map(|(out, r#in)| *out = r#in)
        .count();
    out.0.head = 0;
    out.0.tail = len as u8;
}

#[inline]
pub(super) fn sort_15(iter: &mut KeyIter15) {
    simd!(
        "opt-no-node15-keys",
        avx2::sort_15(iter),
        sort_15_fallback(iter),
    )
}

#[inline]
fn sort_15_fallback(iter: &mut KeyIter15) {
    iter.0.entries[..iter.0.tail as usize].sort_unstable();
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
        use ribbit::u2;
        use ribbit::u4;

        use crate::raw::node::KeyIter15;
        use crate::raw::node::KeyIter47;
        use crate::raw::node::iter::KeyIter3;
        use crate::raw::node::node_3;
        use crate::raw::node::node_15;
        use crate::raw::node::node_47;
        use crate::raw::node::simd;

        /// `get_3` matches output of fallback.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_3_correct<F: Fn(u64, u8) -> u8>(header: node_3::Header, get_3: F) {
            let raw = ribbit::Pack::pack(header).into_raw();
            for key in u8::MIN..=u8::MAX {
                let simd = get_3(raw, key);
                let fallback = simd::get_3_fallback(raw, key);

                assert_eq!(
                    simd, fallback,
                    "SIMD does not match fallback for header {header:#x?} and key {key:#x?}",
                );
            }
        }

        /// `get_15` matches output of fallback.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn get_15_correct<F: Fn(u128, u8) -> u8>(header: node_15::Header, get_15: F) {
            let raw = ribbit::Pack::pack(header).into_raw();
            for key in u8::MIN..=u8::MAX {
                let simd = get_15(raw, key);
                let fallback = simd::get_15_fallback(raw, key);

                assert_eq!(
                    simd, fallback,
                    "SIMD does not match fallback for header {header:#x?} and key {key:#x?}",
                );
            }
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

        /// `keys_15` matches output of fallback.
        #[cfg_attr(not(feature = "proptest"), expect(unused))]
        pub(crate) fn keys_15_correct<F: Fn(u128, u4, Option<u8>, Option<u8>, &mut KeyIter15)>(
            header: node_15::Header,
            lower: u8,
            upper: u8,
            keys_15: F,
        ) {
            let header = ribbit::Pack::pack(header);
            let raw = header.into_raw();
            let len = header.len();

            let mut simd = KeyIter15::default();
            keys_15(raw, len, Some(lower), Some(upper), &mut simd);

            let mut fallback = KeyIter15::default();
            simd::keys_15_fallback(raw, len, Some(lower), Some(upper), &mut fallback);

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
                use ribbit::u4;

                use crate::raw::node::simd::tests::sequential;
                use crate::raw::node::iter::bound;
                use crate::raw::node::node_3;
                use crate::raw::node::node_15;
                use crate::raw::node::node_47;
                use crate::raw::node::simd::$mod;

                proptest::proptest! {
                    #![proptest_config(proptest::test_runner::Config::with_cases(100_000))]

                    #[test]
                    fn get_3_correct(header in any_with::<node_3::Header>((u2::new(0), u2::MAX))) {
                        sequential::get_3_correct(header, $mod::get_3)
                    }

                    #[test]
                    fn get_15_correct(header in any_with::<node_15::Header>((u4::new(0), u4::MAX))) {
                        sequential::get_15_correct(header, $mod::get_15)
                    }

                    #[test]
                    fn keys_3_correct(
                        header in any_with::<node_3::Header>((u2::new(0), u2::MAX)),
                        (lower, upper) in bound()
                    ) {
                        sequential::keys_3_correct(header, lower, upper, $mod::keys_3)
                    }

                    #[test]
                    fn keys_15(
                        header in any_with::<node_15::Header>((u4::new(0), u4::MAX)),
                        (lower, upper) in bound()
                    ) {
                        sequential::keys_15_correct(header, lower, upper, $mod::keys_15)
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
