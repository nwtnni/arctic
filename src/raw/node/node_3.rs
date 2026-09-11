//! [`Node3`] is linear and can contain at most 3 key-edge pairs.

use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::u2;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node;
use crate::raw::node::header;
use crate::raw::node::iter::KeyIter3;
use crate::sync::Atomic64;
use crate::sync::Atomic128;
use crate::sync::Convert;

const CAPACITY: usize = 3;

/// [`Node`] representation that contains at most 3 key-edge pairs.
pub(in crate::raw) type Node3 = Node<CAPACITY, Atomic64<Header>>;

const_assert_size_align!(Node3, 64, 64);

// Layout:
// - 0..8: keys[0]
// - 8..16: padding
// - 16..24: keys[1]
// - 24..32: padding
// - 32..40: keys[2]
// - 40..48: padding
// - 48: frozen
// - 49..56: padding
// - 56..59: len
// - 59..64: zero
#[derive(Copy, Clone, Debug, Default)]
pub(in crate::raw) struct Header(u64);

impl Header {
    const MASK_FROZEN: u64 = 1 << 48;
    const SHIFT_LEN: usize = 56;

    #[inline]
    const fn new(keys: u64, len: u8) -> Self {
        validate!(len <= 3);
        validate!(keys & !((1 << ((len as usize) << 4)) - 1) == 0);
        Self(keys | ((len as u64) << Self::SHIFT_LEN))
    }

    #[inline]
    const fn is_frozen(self) -> bool {
        self.0 & Self::MASK_FROZEN > 0
    }

    #[inline]
    const fn freeze(self) -> Self {
        Self(self.0 | Self::MASK_FROZEN)
    }

    #[inline]
    const fn len(self) -> u8 {
        let len = self.0 >> Self::SHIFT_LEN;
        validate!(len <= 3);
        len as u8
    }
}

unsafe impl header::Header for Atomic64<Header> {
    const TYPE: node::Type = node::Type::Node3;
    type KeyIter = KeyIter3;

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
        let index = header.get(key);
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

    fn keys<L: node::Lower, U: node::Upper>(&self, lower: L, upper: U, out: &mut Self::KeyIter) {
        let header = self.load(Ordering::Relaxed);
        let keys = header.into_raw();
        let len = header.len();

        let len = if lower.get() > u8::MIN || upper.get() < u8::MAX {
            // TODO: SIMD/SWAR?
            core::iter::zip(
                &mut out.0.entries,
                node::simd::iter_3(keys, u2::new(len), lower, upper),
            )
            .map(|(out, r#in)| *out = r#in)
            .count() as u8
        } else {
            let iter = (keys << 8) | 0x0002_0001_0000;
            let ptr = NonNull::from(&mut *out).cast::<u64>();
            unsafe { ptr.write(iter) };
            len
        };

        out.0.head = 0;
        out.0.tail = len;
    }

    fn min<L: node::Lower>(&self, lower: L) -> Option<node::KeyIndex> {
        let header = self.load(Ordering::Relaxed);
        node::simd::min_3(header.into_raw(), u2::new(header.len()), lower)
    }

    fn max<U: node::Upper>(&self, upper: U) -> Option<node::KeyIndex> {
        let header = self.load(Ordering::Relaxed);
        node::simd::max_3(header.into_raw(), u2::new(header.len()), upper)
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
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let index = self.get(key);
        let len = self.len();

        if index < len {
            return Ok(index);
        }

        if len >= CAPACITY as u8 || self.is_frozen() {
            return Err(None);
        }

        // Insert key byte and increment length
        let key = (key as u64) << (len << 4);
        let value = (self.into_raw() | key) + (1u64 << Self::SHIFT_LEN);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::from_raw_unchecked(value) }))
    }

    #[inline]
    fn get(self, key: u8) -> u8 {
        let key = key as u64;

        // LLVM is smart enough to turn this into an imul
        let broadcast = key | (key << 16) | (key << 32);

        crate::raw::find_zero(
            (self.into_raw() ^ broadcast)
                // Ensure we don't match non-key bytes
                | (0x0000_FF00_FF00_FF00),
        )
        // Convert from u8 index to u16 index
        >> 1
    }
}

impl Node3 {
    pub(super) unsafe fn new_unchecked(keys: &[u8], edges: &[edge::Raw]) -> Box<Self> {
        validate!(crate::raw::is_unique(keys));
        validate!(keys.len() == edges.len());
        validate!(keys.len() <= CAPACITY);

        let mut node = Box::new(Self::default());

        let mut buffer = 0u64;
        buffer |= keys.first().copied().unwrap_or(0) as u64;
        buffer |= (keys.get(1).copied().unwrap_or(0) as u64) << 16;
        buffer |= (keys.get(2).copied().unwrap_or(0) as u64) << 32;
        buffer |= (keys.len() as u64) << Header::SHIFT_LEN;
        node.header = Atomic64::<Header>::new(unsafe { Header::from_raw_unchecked(buffer) });

        for (out, r#in) in node.edges.iter_mut().zip(edges) {
            out.set(*r#in);
        }

        node
    }

    pub(crate) fn new_expand<M: edge::Meta>(
        meta: M,
        keys: [u8; 2],
        edges: [Edge<M>; 2],
    ) -> (Edge<M>, NonNull<Atomic128<Edge<M>>>) {
        let mut node = Box::new(Self::default());

        node.header
            .set(Header::new(keys[0] as u64 | ((keys[1] as u64) << 16), 2));
        node.edges[0].set(edges[0].erase());
        node.edges[1].set(edges[1].erase());

        let tail = NonNull::from(&node.edges[0]);
        let head = Edge::new_node(meta, node::Ptr::new_node_3(node));
        (head, tail.cast())
    }

    pub(crate) fn new_path<R: key::Read<Edge = M>, M: edge::Meta>(
        meta: M,
        byte: u8,
        mut reader: R,
        value: u64,
    ) -> (Edge<M>, NonNull<Atomic128<Edge<M>>>) {
        let mut head = Box::new(Self::default());
        head.header.set(Header::new(byte as u64, 1));

        let mut tail = NonNull::from(&head.edges[0]);

        loop {
            let edge = reader.get_edge(<M as edge::Meta>::Len::MAX);

            let Some(byte) = reader.get_byte(edge.len()) else {
                unsafe { tail.as_mut() }.set(Edge::<M>::new_value(edge, value).erase());
                break;
            };

            reader = reader.suffix(R::Len::BYTE + edge.len().into());

            let mut node = Box::new(Self::default());
            node.header.set(Header::new(byte as u64, 1));

            let next = NonNull::from(&node.edges[0]);
            unsafe { tail.as_mut() }
                .set(Edge::<M>::new_node(edge, node::Ptr::new_node_3(node)).erase());
            tail = next;
        }

        let head = Edge::<M>::new_node(meta, node::Ptr::new_node_3(head));
        (head, tail.cast())
    }
}

impl Convert<u64> for Header {
    #[inline]
    fn into_raw(self) -> u64 {
        self.0
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        Self(raw)
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
    crate::raw::node::header::tests::impl_suite!(
        proptest::arbitrary::any_with::<crate::raw::node::node_3::Header>((
            ribbit::u2::new(0),
            <ribbit::u2 as ribbit::Integer>::MAX,
        ))
        .prop_map(crate::sync::Atomic::new)
    );
}
