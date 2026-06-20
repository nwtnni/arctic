//! [`Node15`] is linear and can contain at most 15 key-edge pairs.

use core::sync::atomic::Ordering;

use ribbit::u4;
use ribbit::u120;

use crate::raw::node;
use crate::raw::node::KeyIter15;
use crate::raw::node::Node;
use crate::raw::node::header;
use crate::sync::Atomic;

const CAPACITY: usize = 15;

/// [`Node`][crate::raw::node::Node] representation that contains at most 15 key-edge pairs.
pub(super) type Node15 = Node<CAPACITY, Atomic<Header>>;

const_assert_size_align!(Node15, 256, 64);

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

    unsafe fn initialize_unchecked(&mut self, keys: &[u8]) {
        let mut buffer = [0u8; 16];
        buffer[..keys.len()].copy_from_slice(keys);
        // Skip frozen bit
        buffer[15] = (keys.len() as u8) << 1;

        *self = Self::new_packed(unsafe {
            ribbit::Packed::<Header>::from_raw_unchecked(u128::from_le_bytes(buffer))
        })
    }

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
        let index = node::simd::get_15(header.into_raw(), key);
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
        node::simd::keys_15(header.into_raw(), header.len(), lower, upper, iter);
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
        let index = node::simd::get_15(self.into_raw(), key);
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
