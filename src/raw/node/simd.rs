//! SIMD acceleration for node operations.

use core::sync::atomic::Ordering;

#[cfg(target_feature = "avx2")]
mod avx2;

use ribbit::Atomic;
use ribbit::u2;
use ribbit::u4;

use crate::raw::node;
use crate::raw::node::iter::KeyIndex;
use crate::raw::node::linear::KeyIter3;
use crate::raw::node::linear::KeyIter15;
use crate::raw::node::linear::KeyIter63;

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
pub(super) fn compress_3<L: node::Lower, U: node::Upper>(
    keys: u64,
    len: u2,
    lower: L,
    upper: U,
) -> KeyIter3 {
    simd!(
        "opt-no-node3-compress",
        avx2::compress_3(keys, len, lower, upper),
        compress_3_fallback(keys, len, lower, upper),
    )
}

#[inline]
fn compress_3_fallback<L: node::Lower, U: node::Upper>(
    keys: u64,
    len: u2,
    lower: L,
    upper: U,
) -> KeyIter3 {
    let mut buffer = [0u16; 3];

    let len = keys
        .to_le_bytes()
        .into_iter()
        .step_by(2)
        .take(len.value() as usize)
        .enumerate()
        .filter(|(_, key)| *key >= lower.get() && *key <= upper.get())
        .zip(&mut buffer)
        .map(|((index, key), out)| {
            *out = (index as u16) | (key as u16) << 8;
        })
        .count();

    buffer[..len].sort_unstable();
    if_validate!(
        // HACK: AVX2 implementation pads with 0xFF bytes
        buffer[len..].fill(0xFF_FF)
    );
    let buffer = unsafe { core::mem::transmute::<[u16; 3], [KeyIndex; 3]>(buffer) };
    KeyIter3::new_3(buffer, len as u8)
}

#[inline]
pub(super) fn compress_15<L: node::Lower, U: node::Upper>(
    keys: u128,
    len: u4,
    lower: L,
    upper: U,
    out: &mut KeyIter15,
) {
    simd!(
        "opt-no-node15-compress",
        avx2::compress_15(keys, len, lower, upper, out),
        compress_15_fallback(keys, len, lower, upper, out),
    )
}

#[inline]
pub(super) fn compress_15_fallback<L: node::Lower, U: node::Upper>(
    keys: u128,
    len: u4,
    lower: L,
    upper: U,
    out: &mut KeyIter15,
) {
    let len = keys
        .to_le_bytes()
        .into_iter()
        .take(len.value() as usize)
        .enumerate()
        .filter(|(_, key)| *key >= lower.get() && *key <= upper.get())
        .zip(&mut out.0.entries)
        .map(|((index, key), out)| {
            out.key = key;
            out.index = index as u8;
        })
        .count();

    out.0.entries[..len].sort_unstable();
    out.0.head = 0;
    out.0.tail = len as u8;
}

#[inline]
pub(super) fn compress_47<L: node::Lower, U: node::Upper>(
    indices: &[Atomic<u128>; 16],
    lower: L,
    upper: U,
    len: u8,
    out: &mut KeyIter63,
) {
    simd!(
        "opt-no-node47-compress",
        avx2::compress_47(indices, lower, upper, len, out),
        compress_47_fallback(indices, lower, upper, len, out),
    )
}

#[inline]
pub(super) fn compress_47_fallback<L: node::Lower, U: node::Upper>(
    indices: &[Atomic<u128>; 16],
    lower: L,
    upper: U,
    len: u8,
    out: &mut KeyIter63,
) {
    let i = lower.get() / 16;
    let j = upper.get() / 16;

    let len = indices[i as usize..=j as usize]
        .iter()
        .flat_map(|chunk| chunk.load(Ordering::Relaxed).to_le_bytes())
        // HACK: using `i: u8` here causes integer overflow in debug mode
        // when all 256 bytes are loaded
        .zip((i as u16 * 16)..)
        .map(|(index, key)| (index, key as u8))
        .filter(|(index, key)| (*index < len && *key >= lower.get() && *key <= upper.get()))
        .zip(&mut out.0.entries)
        .map(|((index, key), out)| {
            out.index = index;
            out.key = key;
        })
        .count();

    out.0.head = 0;
    out.0.tail = len as u8;
}
