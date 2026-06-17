//! [`Node3`] is linear and can contain at most 3 key-edge pairs.

use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::u2;
use ribbit::u48;

use crate::Atomic;
use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node;
use crate::raw::node::header;
use crate::raw::node::iter::KeyIter3;
use crate::raw::node::simd;

const CAPACITY: usize = 3;

/// [`Node`][crate::raw::node::Node] representation that contains at most 3 key-edge pairs.
pub(in crate::raw) type Node3 = Node<CAPACITY, Atomic<Header>>;

const_assert_size_align!(Node3, 64, 64);

#[derive(Copy, Clone, Debug, Default, ribbit::Pack)]
#[ribbit(size = 64, derive(Debug))]
pub(in crate::raw) struct Header {
    keys: u48,
    #[ribbit(offset = 48)]
    frozen: bool,
    #[ribbit(offset = 56)]
    len: u2,
}

impl Header {
    const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(u48::new(0), false, u2::new(0));
}

impl Default for HeaderPacked {
    fn default() -> Self {
        Header::DEFAULT
    }
}

unsafe impl header::Header for Atomic<Header> {
    const TYPE: node::Type = node::Type::Node3;
    type KeyIter = KeyIter3;

    unsafe fn initialize_unchecked(&mut self, keys: &[u8]) {
        let mut buffer = 0u64;
        buffer |= keys.first().copied().unwrap_or(0) as u64;
        buffer |= (keys.get(1).copied().unwrap_or(0) as u64) << 16;
        buffer |= (keys.get(2).copied().unwrap_or(0) as u64) << 32;
        buffer |= (keys.len() as u64) << 56;
        *self = Self::new_packed(unsafe { ribbit::Packed::<Header>::from_raw_unchecked(buffer) });
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
        let index = simd::get_3(header.into_raw(), key);
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

    fn keys<L: node::Lower, U: node::Upper>(&self, lower: L, upper: U, iter: &mut Self::KeyIter) {
        let header = self.load_packed(Ordering::Relaxed);
        node::simd::keys_3(header.into_raw(), header.len(), lower, upper, iter)
    }

    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex> {
        let header = self.load_packed(Ordering::Relaxed);
        node::simd::min_3(header.into_raw(), header.len(), lower)
    }

    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex> {
        let header = self.load_packed(Ordering::Relaxed);
        node::simd::max_3(header.into_raw(), header.len(), upper)
    }
}

impl HeaderPacked {
    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let index = simd::get_3(self.into_raw(), key);
        let len = self.len().value();

        if index < len {
            return Ok(index);
        }

        if len >= CAPACITY as u8 || self.frozen() {
            return Err(None);
        }

        // Insert key byte and increment length
        let key = (key as u64) << (len << 4);
        let value = (self.into_raw() | key) + (1u64 << 56);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::from_raw_unchecked(value) }))
    }
}

impl Node<3, Atomic<Header>> {
    pub(crate) fn new_expand<M: ribbit::Pack<Packed: edge::Meta>>(
        meta: ribbit::Packed<M>,
        keys: [u8; 2],
        edges: [ribbit::Packed<Edge<M>>; 2],
    ) -> (ribbit::Packed<Edge<M>>, NonNull<Atomic<Edge<M>>>) {
        let mut node = Box::new(Self::default());

        *node.header.get_mut_packed() = ribbit::Packed::<Header>::new(
            u48::new(keys[0] as u64 | ((keys[1] as u64) << 16)),
            false,
            const { u2::new(2) },
        );
        *node.edges[0].get_mut_packed() = edges[0].erase();
        *node.edges[1].get_mut_packed() = edges[1].erase();

        let tail = NonNull::from(&node.edges[0]);
        let head = Edge::new_node(meta, node::Ptr::new_node_3(node));
        (head, tail.cast())
    }

    pub(crate) fn new_path<R: key::Read<Edge = M>, M: ribbit::Pack<Packed: edge::Meta>>(
        meta: ribbit::Packed<M>,
        byte: u8,
        mut reader: R,
        value: u64,
    ) -> (ribbit::Packed<Edge<M>>, NonNull<Atomic<Edge<M>>>) {
        let mut head = Box::new(Self::default());
        *head.header.get_mut_packed() =
            ribbit::Packed::<Header>::new(u48::new(byte as u64), false, const { u2::new(1) });

        let mut tail = NonNull::from(&head.edges[0]);

        loop {
            let edge = reader.get_edge(<ribbit::Packed<M> as edge::Meta>::Len::MAX);

            let Some(byte) = reader.get_byte(edge.len()) else {
                *unsafe { tail.as_mut() }.get_mut_packed() =
                    Edge::<M>::new_value(edge, value).erase();
                break;
            };

            reader = reader.suffix(R::Len::BYTE + edge.len().into());

            let mut node = Box::new(Self::default());
            *node.header.get_mut_packed() =
                ribbit::Packed::<Header>::new(u48::new(byte as u64), false, const { u2::new(1) });

            let next = NonNull::from(&node.edges[0]);
            *unsafe { tail.as_mut() }.get_mut_packed() =
                Edge::<M>::new_node(edge, node::Ptr::new_node_3(node)).erase();
            tail = next;
        }

        let head = Edge::<M>::new_node(meta, node::Ptr::new_node_3(head));
        (head, tail.cast())
    }
}

impl From<KeyIter3> for node::KeyIter {
    #[inline]
    fn from(iter: KeyIter3) -> Self {
        node::KeyIter::new_3(iter)
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for Header {
    type Parameters = (u2, u2);
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((min_len, max_len): Self::Parameters) -> Self::Strategy {
        use proptest::bits::SampledBitSetStrategy;
        use proptest::strategy::Strategy as _;

        (
            SampledBitSetStrategy::<crate::raw::set::Set256>::new(
                min_len.value() as usize..=max_len.value() as usize,
                u8::MIN as usize..=u8::MAX as usize,
            )
            .prop_map(|set| set.iter().collect::<Vec<_>>())
            .prop_shuffle(),
            bool::arbitrary(),
        )
            .prop_map(|(keys, frozen)| {
                let mut buffer = 0u64;
                buffer |= keys.first().copied().unwrap_or(0) as u64;
                buffer |= (keys.get(1).copied().unwrap_or(0) as u64) << 16;
                buffer |= (keys.get(2).copied().unwrap_or(0) as u64) << 32;
                Self {
                    keys: u48::new(buffer),
                    frozen,
                    len: u2::new(keys.len() as u8),
                }
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "proptest")]
    mod proptest {
        use proptest::arbitrary::any_with;
        use proptest::strategy::Strategy as _;
        use ribbit::Integer as _;
        use ribbit::u2;

        crate::raw::node::header::tests::impl_suite!(
            any_with::<crate::raw::node::node_3::Header>((u2::new(0), u2::MAX))
                .prop_map(crate::sync::Atomic::new)
        );
    }
}
