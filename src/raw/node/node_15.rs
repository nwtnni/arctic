//! [`Node15`] is linear and can contain at most 15 key-edge pairs.

use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use fearless_simd::Simd;
use fearless_simd::SimdBase as _;
use fearless_simd::SimdFrom as _;
use fearless_simd::SimdInt as _;
use fearless_simd::SimdMask as _;
use fearless_simd::mask8x16;
use fearless_simd::u8x16;
use fearless_simd::u16x16;
use ribbit::u4;

use crate::raw::edge;
use crate::raw::node;
use crate::raw::node::KeyIter15;
use crate::raw::node::Node;
use crate::raw::node::header;
use crate::sync::Atomic128;
use crate::sync::Convert;

const CAPACITY: usize = 15;

/// [`Node`] representation that contains at most 15 key-edge pairs.
pub(super) type Node15 = Node<CAPACITY, Atomic128<Header>>;

const_assert_size_align!(Node15, 256, 64);

impl Node15 {
    pub(super) unsafe fn new_unchecked(keys: &[u8], edges: &[edge::Raw]) -> Box<Self> {
        validate!(crate::raw::is_unique(keys));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= CAPACITY);

        let mut node = Box::new(Self::default());

        let mut buffer = [0u8; 16];
        buffer[..keys.len()].copy_from_slice(keys);

        node.header = Atomic128::new(Header::new(u128::from_le_bytes(buffer), keys.len()));

        for (out, r#in) in node.edges.iter_mut().zip(edges) {
            out.set(*r#in);
        }

        node
    }
}

// Layout:
// - 0..120: keys
// - 120: frozen
// - 121..125: len
#[derive(Copy, Clone, Debug, Default)]
pub(super) struct Header(u128);

unsafe impl header::Header for Atomic128<Header> {
    const TYPE: node::Type = node::Type::Node15;
    type KeyIter = KeyIter15;

    #[inline]
    fn freeze(&self) -> usize {
        let mut header = self.load(Ordering::Relaxed);

        while !header.is_frozen() {
            match self.compare_exchange(
                header,
                header.freeze(),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => header = conflict,
            }
        }

        header.len() as usize
    }

    #[inline]
    fn get(&self, key: u8) -> Option<u8> {
        let header = self.load(Ordering::Relaxed);
        let index = fearless_simd::dispatch!(*crate::raw::SIMD, simd => header.get(simd, key));
        (index < header.len()).then_some(index)
    }

    #[inline]
    fn get_or_insert(&self, key: u8) -> Option<u8> {
        let mut old = self.load(Ordering::Relaxed);

        loop {
            let new = match old.get_or_insert(key) {
                Ok(index) => return Some(index),
                Err(None) => return None,
                Err(Some(new)) => new,
            };

            match self.compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break Some(old.len()),
                Err(conflict) => old = conflict,
            }
        }
    }

    fn keys<L: node::Lower, U: node::Upper>(&self, lower: L, upper: U, iter: &mut KeyIter15) {
        let header = self.load(Ordering::Relaxed);
        fearless_simd::dispatch!(*crate::raw::SIMD, simd => {
            header.keys_simd(simd, u4::new(header.len()), lower, upper, iter);
        })
    }

    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex> {
        let header = self.load(Ordering::Relaxed);
        node::simd::min_15(header.into_raw(), u4::new(header.len()), lower)
    }

    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex> {
        let header = self.load(Ordering::Relaxed);
        node::simd::max_15(header.into_raw(), u4::new(header.len()), upper)
    }

    #[inline]
    fn len(&self) -> usize {
        self.load(Ordering::Relaxed).len() as usize
    }

    #[inline]
    fn is_frozen(&self) -> bool {
        self.load(Ordering::Relaxed).is_frozen()
    }
}

impl Header {
    #[inline]
    fn get_or_insert(&self, key: u8) -> Result<u8, Option<Self>> {
        let index = fearless_simd::dispatch!(*crate::raw::SIMD, simd => self.get(simd, key));
        let len = self.len();

        if index < len {
            return Ok(index);
        }

        if len >= CAPACITY as u8 || self.is_frozen() {
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
            let mask_range = node::simd::mask_range(simd, keys, lower.get(), upper.get());

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
    }
}

impl Header {
    const MASK_FROZEN: u128 = (1 << 120);
    const SHIFT_LEN: usize = 121;

    #[inline]
    const fn new(keys: u128, len: usize) -> Self {
        validate!(len <= 15);
        // Bytes above `len` are zero
        validate!(keys & !((1 << ((len as u32) << 3)) - 1) == 0);
        Self(keys | ((len as u128) << Self::SHIFT_LEN))
    }

    #[inline]
    const fn freeze(self) -> Self {
        validate!(!self.is_frozen());
        Self(self.0 | Self::MASK_FROZEN)
    }

    #[inline]
    const fn is_frozen(self) -> bool {
        self.0 & Self::MASK_FROZEN > 0
    }

    #[inline]
    const fn len(self) -> u8 {
        let len = self.0 >> 121;
        validate!(len <= 15);
        len as u8
    }
}

impl Convert<u128> for Header {
    #[inline]
    fn into_raw(self) -> u128 {
        self.0
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u128) -> Self {
        Self(raw)
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
        use core::sync::atomic::Atomic128U64;

        use proptest::bits::SampledBitSetStrategy;
        use proptest::strategy::Strategy as _;

        (
            SampledBitSetStrategy::<crate::raw::set::Set256<Atomic128U64>>::new(
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
        .prop_map(crate::sync::Atomic128::new)
    );
}
