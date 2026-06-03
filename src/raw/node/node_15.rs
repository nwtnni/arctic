//! [`Node15`] is linear and can contain at most 15 key-edge pairs.

use ribbit::u4;
use ribbit::u120;

use crate::raw::node;
use crate::raw::node::Linear;
#[cfg_attr(not(doc), expect(unused_imports))]
use crate::raw::node::Node;
use crate::raw::node::linear;

/// [`Node`] representation that contains at most 15 key-edge pairs.
pub(crate) type Node15<M> = Linear<15, Header, M>;

const_assert_size_align!(Node15::<()>, 256, 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
    keys: u120,
    frozen: bool,
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

impl linear::Header for ribbit::Packed<Header> {
    const TYPE: node::Type = node::Type::Node15;
    const CAPACITY: usize = 15;
    type KeyIter = Box<linear::KeyIter15>;

    unsafe fn new_unchecked(keys: &[u8]) -> Self {
        let mut buffer = [0u8; 16];
        buffer[..keys.len()].copy_from_slice(keys);
        Self::new(
            u120::new(u128::from_le_bytes(buffer)),
            false,
            u4::new(keys.len() as u8),
        )
    }

    #[inline]
    fn freeze(self) -> Self {
        self.with_frozen(true)
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.frozen()
    }

    #[inline]
    fn len(self) -> u8 {
        self.len().value()
    }

    #[inline]
    fn get(self, key: u8) -> Option<u8> {
        let index = node::simd::get_15(self.value, key);
        (index < self.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let index = node::simd::get_15(self.value, key);
        let len = self.len().value();

        if index < len {
            return Ok(index);
        }

        if len >= Self::CAPACITY as u8 || self.is_frozen() {
            return Err(None);
        }

        let key = (key as u128) << (len << 3);
        let value = (self.value | key) + (1u128 << 121);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::new_unchecked(value) }))
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> Self::KeyIter {
        let len = self.len();
        let mut iter = Box::new(linear::KeyIter15::default());
        node::simd::keys_15(self.value, len, lower, upper, &mut iter);
        iter
    }
}

impl From<Box<linear::KeyIter15>> for node::KeyIter {
    #[inline]
    fn from(iter: Box<linear::KeyIter15>) -> Self {
        node::KeyIter::new_15(iter)
    }
}
