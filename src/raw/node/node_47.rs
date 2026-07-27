//! [`Node47`] can contain at most 47 key-edge pairs.
//!
//! Note that, unlike [`crate::raw::node::Node3`] and [`crate::raw::node::Node15`],
//! [`crate::raw::node::Node47`]'s header **cannot** be updated in a single
//! atomic operation, and requires more careful reasoning.

use core::fmt::Debug;
use core::ops::Deref;
use core::ops::Shr;
use core::sync::atomic::Ordering;

use fearless_simd::Simd;
use fearless_simd::SimdBase as _;
use fearless_simd::SimdFrom as _;
use fearless_simd::SimdInt as _;
use fearless_simd::SimdMask as _;
use fearless_simd::u8x16;
use ribbit::u6;

use crate::raw::edge;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::raw::node::Node;
use crate::raw::node::header;
use crate::raw::node::iter::KeyIndex;
use crate::raw::node::iter::KeyIter47;
use crate::stat;
use crate::sync::Atomic;

const CAPACITY: usize = 47;

/// [`Node`] representation that contains at most 47 key-edge pairs.
#[repr(C, align(1024))]
#[derive(Default)]
pub(super) struct Node47(Node<CAPACITY, Header>);

const_assert_size_align!(Node47, 1024, 1024);

impl Node47 {
    pub(super) unsafe fn new_unchecked(
        keys: &[u8],
        edges: &[ribbit::Packed<edge::Raw>],
    ) -> Box<Self> {
        if_validate!(assert!(crate::raw::is_unique(keys)));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= CAPACITY);

        let mut node = Box::new(Self::default());

        for (i, key) in keys.iter().enumerate() {
            let (row, col) = Header::key_to_row_col(*key);
            let row = &mut node.0.header.indices[row as usize];
            let old = row.get();
            let new = old ^ (0x7F ^ i as u128) << col;
            row.set(new);
        }

        *node.0.header.meta.get_mut_packed() = ribbit::Packed::<Meta>::new(
            keys.last().copied().unwrap(),
            false,
            u6::new(keys.len() as u8),
        );

        for (out, r#in) in node.0.edges.iter_mut().zip(edges) {
            *out.get_mut_packed() = *r#in;
        }

        node
    }
}

impl Deref for Node47 {
    type Target = Node<CAPACITY, Header>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[repr(C, align(16))]
#[derive(Clone)]
pub(super) struct Header {
    indices: [Atomic<u128>; 16],
    // Place `meta` after `indices to make sure former
    // is 16-byte aligned for SIMD.
    meta: Atomic<Meta>,
}

// NOTE: we fill in uninitialized indices with 0x7F as opposed to
// - 0x00:
//   - Readers can distinguish an uninitialized index without loading `meta`
//   - Writers don't have to serialize writes to `meta` and `indices`
// - 0xFF:
//   - AVX2 only supports signed byte-wise comparison
const UNINIT: u128 = 0x7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F;

impl Default for Header {
    fn default() -> Self {
        Self {
            indices: core::array::from_fn(|_| Atomic::new_packed(UNINIT)),
            meta: Atomic::new_packed(Meta::DEFAULT),
        }
    }
}

unsafe impl header::Header for Header {
    const TYPE: node::Type = node::Type::Node47;
    type KeyIter = KeyIter47;

    fn freeze(&self) -> usize {
        let mut old = self.meta.load_packed(Ordering::Relaxed);
        while !old.frozen() {
            self.ensure_meta_consistent(old);
            match self.meta.compare_exchange_packed(
                old,
                old.with_frozen(true),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
        old.len().value() as usize
    }

    fn get(&self, key: u8) -> Option<u8> {
        let (row, col) = Self::key_to_row_col(key);
        validate!(col < 128);
        let index = self.indices[row as usize].load(Ordering::Relaxed).shr(col) as u8;
        (index < CAPACITY as u8).then_some(index)
    }

    fn get_or_insert(&self, key: u8) -> Option<u8> {
        loop {
            if let Some(index) = self.get(key) {
                return Some(index);
            }

            let old = self.meta_consistent();
            let len = old.len().value();

            // NOTE: tricky edge case here, where the above `get`
            // call returns `None` between another thread updating
            // the metadata and the data array being updated.
            if key == old.last() {
                let index = len.checked_sub(1);
                validate!(index.is_some());
                return index;
            }

            if len == CAPACITY as u8 || old.frozen() {
                return None;
            }

            let new = old.with_len(u6::new(len + 1)).with_last(key);

            match self
                .meta
                .compare_exchange_packed(old, new, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.ensure_meta_consistent(new);
                    return Some(len);
                }
                Err(_) => continue,
            }
        }
    }

    // fn insert(&mut self, key: u8) -> Option<u8> {
    //     let old_meta = self.meta.get_packed();
    //     let len = old_meta.len().value();
    //
    //     validate!(!old_meta.frozen());
    //     validate!(len <= 47);
    //
    //     if len == 47 {
    //         return None;
    //     }
    //
    //     let new_meta = old_meta.with_len(u6::new(len + 1)).with_last(key);
    //     self.meta.set_packed(new_meta);
    //
    //     let (row, col) = Self::key_to_row_col(key);
    //
    //     let data = unsafe { self.data_unchecked_mut(row) };
    //
    //     let old_data = *data.get_mut();
    //     let hole = !(0xFFu64 << col);
    //     let new_data = old_data & hole | ((len as u64) << col);
    //
    //     *data.get_mut() = new_data;
    //     Some(len)
    // }

    fn keys<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
        iter: &mut KeyIter47,
    ) {
        // NOTE: only writers need to ensure meta consistency
        let len = self.len();
        let indices = self.indices();
        fearless_simd::dispatch!(*crate::raw::SIMD, simd => {
            Header::keys_simd(simd, indices, len, lower, upper, iter);
        })
    }

    fn min<L: node::Lower>(&self, _lower: L) -> Option<KeyIndex> {
        todo!()
    }

    fn max<U: node::Upper>(&self, _upper: U) -> Option<KeyIndex> {
        todo!()
    }

    fn len(&self) -> usize {
        self.len() as usize
    }

    fn is_frozen(&self) -> bool {
        self.meta.load_packed(Ordering::Relaxed).frozen()
    }
}

impl Header {
    fn meta_consistent(&self) -> ribbit::Packed<Meta> {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        self.ensure_meta_consistent(meta);
        meta
    }

    fn ensure_meta_consistent(&self, meta: ribbit::Packed<Meta>) {
        let len = meta.len().value();
        validate!(len <= CAPACITY as u8);
        let index = len - 1;

        let key = meta.last();
        let (row, col) = Self::key_to_row_col(key);

        let row = &self.indices[row as usize];
        let old = row.load(Ordering::Relaxed);

        if (old >> col) as u8 == index {
            stat::increment(stat::Counter::Node47Consistent);
            return;
        }

        let hole = !(0xFFu128 << col);
        let new = old & hole | ((index as u128) << col);

        match row.compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                stat::increment(stat::Counter::Node47CasSuccess);
            }
            Err(_) => stat::increment(stat::Counter::Node47CasFailure),
        }
    }

    fn key_to_row_col(key: u8) -> (u8, u8) {
        let row = key / 16;
        let col = (key % 16) * 8;
        (row, col)
    }

    pub(super) fn len(&self) -> u8 {
        self.meta.load_packed(Ordering::Relaxed).len().value()
    }

    pub(super) fn indices(&self) -> [u128; 16] {
        core::array::from_fn(|i| self.indices[i].load(Ordering::Relaxed))
    }

    #[inline(always)]
    fn keys_simd<S: Simd, L: node::Lower, U: node::Upper>(
        simd: S,
        indices: [u128; 16],
        len: u8,
        lower: L,
        upper: U,
        out: &mut KeyIter47,
    ) {
        validate!(len <= 0x7F, "AVX2 only supports signed byte comparison");

        let len_i8x16 = simd.splat_i8x16(len as i8);
        let mut len = 0;

        if lower.get() > u8::MIN || upper.get() < u8::MAX {
            let i = lower.get() / 16;
            let j = upper.get() / 16;

            let mut keys = u8x16::from_fn(simd, |index| index as u8 + 16 * i);

            for chunk in indices[i as usize..=j as usize].iter().copied() {
                let chunk = simd.cvt_from_bytes_i8x16(u8x16::simd_from(simd, chunk.to_le_bytes()));

                let mask_len = chunk.simd_lt(len_i8x16);
                let mask_range = node::simd::mask_range(simd, keys, lower.get(), upper.get());
                let mask = mask_len & mask_range;

                let compressed =
                    node::simd::compress_u8x16(simd, mask, simd.cvt_to_bytes_i8x16(chunk), keys);

                let slice = unsafe {
                    core::mem::transmute::<&mut [KeyIndex], &mut [u16]>(
                        &mut out.0.entries[len as usize..][..16],
                    )
                };

                compressed.store_slice(slice);
                keys += simd.splat_u8x16(16);
                len += mask.to_bitmask().count_ones() as u8;
            }
        } else {
            let mut keys = u8x16::from_fn(simd, |index| index as u8);

            for chunk in indices.iter().copied() {
                let chunk = simd.cvt_from_bytes_i8x16(u8x16::simd_from(simd, chunk.to_le_bytes()));

                let mask_len = chunk.simd_lt(len_i8x16);
                let compressed = node::simd::compress_u8x16(
                    simd,
                    mask_len,
                    simd.cvt_to_bytes_i8x16(chunk),
                    keys,
                );

                let slice = unsafe {
                    core::mem::transmute::<&mut [KeyIndex], &mut [u16]>(
                        &mut out.0.entries[len as usize..][..16],
                    )
                };

                compressed.store_slice(slice);
                keys += simd.splat_u8x16(16);
                len += mask_len.to_bitmask().count_ones() as u8;
            }
        }

        out.0.head = 0;
        out.0.tail = len;
    }
}

impl Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        let mut iter = KeyIter47::default();
        header::Header::keys(
            self,
            Unbound::<()>::default(),
            Unbound::<()>::default(),
            &mut iter,
        );

        f.debug_struct("Header")
            .field("len", &iter.0.tail)
            .field("frozen", &meta.frozen())
            .field("last", &meta.last())
            .field("keys", &iter)
            .finish()
    }
}

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 16, derive(Debug))]
struct Meta {
    last: u8,
    frozen: bool,
    len: u6,
}

impl Meta {
    const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(0, false, u6::new(0));
}

impl Default for MetaPacked {
    fn default() -> Self {
        Meta::DEFAULT
    }
}

impl From<Box<KeyIter47>> for node::KeyIter {
    #[inline]
    fn from(iter: Box<KeyIter47>) -> Self {
        node::KeyIter::new_47(iter)
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for Header {
    type Parameters = (u8, u8);
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((min_len, max_len): Self::Parameters) -> Self::Strategy {
        use core::sync::atomic::AtomicU64;

        use proptest::bits::SampledBitSetStrategy;
        use proptest::strategy::Strategy as _;
        use ribbit::Integer as _;

        assert!(min_len >= 1);
        assert!(max_len <= 47);

        (
            SampledBitSetStrategy::<crate::raw::set::Set256<AtomicU64>>::new(
                min_len.value() as usize..=max_len.value() as usize,
                u8::MIN as usize..=u8::MAX as usize,
            )
            .prop_map(|set| set.iter().collect::<Vec<_>>())
            .prop_shuffle(),
            bool::arbitrary(),
        )
            .prop_map(|(keys, frozen)| {
                let mut indices = [UNINIT; 16];
                for (i, key) in keys.iter().enumerate() {
                    let (row, col) = Self::key_to_row_col(*key);
                    indices[row as usize] ^= (0x7F ^ i as u128) << col;
                }

                Self {
                    indices: core::array::from_fn(|i| crate::sync::Atomic::new(indices[i])),
                    meta: crate::sync::Atomic::new(Meta {
                        last: keys.last().copied().unwrap(),
                        frozen,
                        len: u6::new(keys.len() as u8),
                    }),
                }
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    crate::raw::node::header::tests::impl_suite!(proptest::prelude::any_with::<
        crate::raw::node::node_47::Header,
    >((1, 47)));
}
