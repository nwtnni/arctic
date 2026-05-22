//! [`Node3`] is linear and can contain at most 3 key-edge pairs.

use core::ptr::NonNull;

use ribbit::u2;
use ribbit::u48;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::edge::Len as _;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Linear;
#[cfg_attr(not(doc), expect(unused_imports))]
use crate::raw::node::Node;
use crate::raw::node::linear;
use crate::raw::node::simd;

/// [`Node`] representation that contains at most 3 key-edge pairs.
pub(crate) type Node3<M> = Linear<3, Header, M>;

const_assert_size_align!(Node3::<()>, 64, 64);

#[derive(Copy, Clone, Debug, ribbit::Pack)]
#[ribbit(size = 64, packed(rename = "HeaderPacked"), debug)]
pub(crate) struct Header {
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

impl linear::Header for ribbit::Packed<Header> {
    const TYPE: node::Type = node::Type::Node3;
    const CAPACITY: usize = 3;

    #[expect(clippy::get_first)]
    unsafe fn new_unchecked(keys: &[u8]) -> Self {
        let mut buffer = 0u64;
        buffer |= keys.get(0).copied().unwrap_or(0) as u64;
        buffer |= (keys.get(1).copied().unwrap_or(0) as u64) << 16;
        buffer |= (keys.get(2).copied().unwrap_or(0) as u64) << 32;
        Self::new(u48::new(buffer), false, u2::new(keys.len() as u8))
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
        let index = simd::get_3(self.value, key);
        (index < self.len().value()).then_some(index)
    }

    #[inline]
    fn get_or_insert(self, key: u8) -> Result<u8, Option<Self>> {
        let index = simd::get_3(self.value, key);
        let len = self.len().value();

        if index < len {
            return Ok(index);
        }

        if len >= Self::CAPACITY as u8 || self.is_frozen() {
            return Err(None);
        }

        // Insert key byte and increment length
        let key = (key as u64) << (len << 4);
        let value = (self.value | key) + (1u64 << 56);

        // SAFETY: `len < Self::LEN`
        Err(Some(unsafe { Self::new_unchecked(value) }))
    }

    fn keys<L: crate::raw::node::Lower, U: crate::raw::node::Upper>(
        self,
        lower: L,
        upper: U,
    ) -> node::KeyIter {
        let len = self.len();
        let iter = node::simd::compress_3(self.value, len, lower, upper);
        node::KeyIter::new_3(iter)
    }
}

impl<M: ribbit::Pack<Packed: edge::Meta>> Linear<3, Header, M> {
    pub(crate) fn new_expand(
        meta: ribbit::Packed<M>,
        keys: [u8; 2],
        edges: [ribbit::Packed<Edge<M>>; 2],
    ) -> (ribbit::Packed<Edge<M>>, NonNull<ribbit::Atomic<Edge<M>>>) {
        let mut node = Box::new(Self::default());

        node.header.set_packed(ribbit::Packed::<Header>::new(
            u48::new(keys[0] as u64 | ((keys[1] as u64) << 16)),
            false,
            const { u2::new(2) },
        ));
        node.edges[0].set_packed(edges[0]);
        node.edges[1].set_packed(edges[1]);

        let tail = NonNull::from(&node.edges[0]);
        let head = Edge::new_node(meta, node::Ptr::new_node_3(node));
        (head, tail)
    }

    pub(crate) fn new_path<R: key::Read<Edge = M>>(
        meta: ribbit::Packed<M>,
        byte: u8,
        mut reader: R,
        value: u64,
    ) -> (ribbit::Packed<Edge<M>>, NonNull<ribbit::Atomic<Edge<M>>>) {
        let mut head = Box::new(Self::default());
        head.header.set_packed(ribbit::Packed::<Header>::new(
            u48::new(byte as u64),
            false,
            const { u2::new(1) },
        ));

        let mut tail = NonNull::from(&head.edges[0]);

        loop {
            let edge = reader.get_edge(<ribbit::Packed<M> as edge::Meta>::Len::MAX);

            let Some(byte) = reader.get_byte(edge.len()) else {
                unsafe { tail.as_mut() }.set_packed(Edge::<M>::new_value(edge, value));
                break;
            };

            reader = reader.suffix(R::Len::BYTE + edge.len().into());

            let mut node = Box::new(Self::default());
            node.header.set_packed(ribbit::Packed::<Header>::new(
                u48::new(byte as u64),
                false,
                const { u2::new(1) },
            ));

            let next = NonNull::from(&node.edges[0]);
            unsafe { tail.as_mut() }
                .set_packed(Edge::<M>::new_node(edge, node::Ptr::new_node_3(node)));
            tail = next;
        }

        let head = Edge::<M>::new_node(meta, node::Ptr::new_node_3(head));
        (head, tail)
    }
}
