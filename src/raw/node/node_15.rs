//! [`Node15`] is linear and can contain at most 15 key-edge pairs.

use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use fearless_simd::Simd;
use fearless_simd::SimdBase as _;
use fearless_simd::SimdFrom as _;
use fearless_simd::SimdInt as _;
use fearless_simd::SimdMask as _;
use fearless_simd::dispatch;
use fearless_simd::mask8x16;
use fearless_simd::u8x16;
use fearless_simd::u16x16;
use ribbit::u4;
use ribbit::u120;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::KeyIter15;
use crate::raw::node::Node;
use crate::raw::node::header;
use crate::sync::Atomic;

const CAPACITY: usize = 15;

/// [`Node`] representation that contains at most 15 key-edge pairs.
pub(super) type Node15 = Node<CAPACITY, Atomic<Header>>;

const_assert_size_align!(Node15, 256, 64);

impl Node15 {
    pub(super) unsafe fn new_unchecked(
        keys: &[u8],
        edges: &[ribbit::Packed<edge::Raw>],
    ) -> Box<Self> {
        if_validate!(assert!(crate::raw::is_unique(keys)));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= CAPACITY);

        let mut node = Box::new(Self::default());

        let mut buffer = [0u8; 16];
        buffer[..keys.len()].copy_from_slice(keys);
        // Skip frozen bit
        buffer[15] = (keys.len() as u8) << 1;

        node.header = Atomic::new_packed(unsafe {
            ribbit::Packed::<Header>::from_raw_unchecked(u128::from_le_bytes(buffer))
        });

        for (out, r#in) in node.edges.iter_mut().zip(edges) {
            *out.get_mut_packed() = *r#in;
        }

        node
    }
}

#[derive(Copy, Clone, Debug, Default, ribbit::Pack)]
#[ribbit(size = 128, derive(Debug))]
pub(super) struct Header {
    keys: u120,
    frozen: bool,
    #[ribbit(get(vis = "pub(in crate::raw)"))]
    len: u4,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u120::new(0), false, u4::new(0));
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

unsafe impl header::Header for Atomic<Header> {
    const TYPE: node::Type = node::Type::Node15;
    type KeyIter = KeyIter15;

    #[inline]
    fn freeze(&self) -> usize {
        let mut header = self.load_packed(Ordering::Relaxed);

        while !header.frozen() {
            match self.compare_exchange_packed(
                header,
                header.with_frozen(true),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => header = conflict,
            }
        }

        header.len().value() as usize
    }

    #[inline]
    fn get(&self, key: u8) -> Option<u8> {
        let header = self.load_packed(Ordering::Relaxed);
        let index = dispatch!(*crate::raw::SIMD, simd => header.get(simd, key));
        (index < header.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(&self, key: u8) -> Option<u8> {
        let mut old = self.load_packed(Ordering::Relaxed);

        loop {
            let new = match old.get_or_insert(key) {
                Ok(index) => return Some(index),
                Err(None) => return None,
                Err(Some(new)) => new,
            };

            match self.compare_exchange_packed(old, new, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break Some(old.len().value()),
                Err(conflict) => old = conflict,
            }
        }
    }

    fn keys<L: node::Lower, U: node::Upper>(&self, lower: L, upper: U, iter: &mut KeyIter15) {
        let header = self.load_packed(Ordering::Relaxed);
        fearless_simd::dispatch!(*crate::raw::SIMD, simd => {
            header.keys_simd(simd, header.len(), lower, upper, iter);
        })
    }

    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex> {
        let header = self.load_packed(Ordering::Relaxed);
        node::simd::min_15(header.into_raw(), header.len(), lower)
    }

    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex> {
        let header = self.load_packed(Ordering::Relaxed);
        node::simd::max_15(header.into_raw(), header.len(), upper)
    }

    #[inline]
    fn len(&self) -> usize {
        self.load_packed(Ordering::Relaxed).len().value() as usize
    }

    #[inline]
    fn is_frozen(&self) -> bool {
        self.load_packed(Ordering::Relaxed).frozen()
    }
}

impl HeaderPacked {
    #[inline]
    fn get_or_insert(&self, key: u8) -> Result<u8, Option<Self>> {
        let index = dispatch!(*crate::raw::SIMD, simd => self.get(simd, key));
        let len = self.len().value();

        if index < len {
            return Ok(index);
        }

        if len >= CAPACITY as u8 || self.frozen() {
            return Err(None);
        }

        let key = (key as u128) << (len << 3);
        let value = (self.into_raw() | key) + (1u128 << 121);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::from_raw_unchecked(value) }))
    }

    #[inline(always)]
    fn get<S: Simd>(&self, simd: S, key: u8) -> u8 {
        let array = u8x16::simd_from(simd, self.into_raw().to_le_bytes());
        let key = u8x16::splat(simd, key);
        array.simd_eq(key).to_bitmask().trailing_zeros() as u8
    }

    #[inline(always)]
    fn keys_simd<S: Simd, L: node::Lower, U: node::Upper>(
        &self,
        simd: S,
        len: u4,
        lower: L,
        upper: U,
        out: &mut KeyIter15,
    ) {
        let keys = u8x16::simd_from(simd, self.into_raw().to_le_bytes());
        let indices = u8x16::from_fn(simd, |index| index as u8);

        let (iter, len) = if lower.get() > u8::MIN || upper.get() < u8::MAX {
            let mask_len = mask8x16::from_bitmask(simd, (1u64 << len.value()) - 1);
            let mask_range = keys
                .max(simd.splat_u8x16(lower.get()))
                .min(simd.splat_u8x16(upper.get()))
                .simd_eq(keys);

            let mask = mask_len & mask_range;
            let len = mask.to_bitmask().count_ones() as u8;
            (node::simd::compress_u8x16(simd, mask, indices, keys), len)
        } else {
            (node::simd::interleave(simd, indices, keys), len.value())
        };

        let ptr = NonNull::from(&mut *out).cast::<u16x16<S>>();
        unsafe { ptr.write(iter) };

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
}

impl From<Box<KeyIter15>> for node::KeyIter {
    #[inline]
    fn from(iter: Box<KeyIter15>) -> Self {
        node::KeyIter::new_15(iter)
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for Header {
    type Parameters = (u4, u4);
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((min_len, max_len): Self::Parameters) -> Self::Strategy {
        use core::sync::atomic::AtomicU64;

        use proptest::bits::SampledBitSetStrategy;
        use proptest::strategy::Strategy as _;

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
                let mut buffer = [0u8; 16];
                buffer[..keys.len()].copy_from_slice(&keys);
                Self {
                    keys: u120::new(u128::from_le_bytes(buffer)),
                    frozen,
                    len: u4::new(keys.len() as u8),
                }
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    crate::raw::node::header::tests::impl_suite!(
        proptest::arbitrary::any_with::<crate::raw::node::node_15::Header>((
            ribbit::u4::new(0),
            <ribbit::u4 as ribbit::Integer>::MAX,
        ))
        .prop_map(crate::sync::Atomic::new)
    );
}
