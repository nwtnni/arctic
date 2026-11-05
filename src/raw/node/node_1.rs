use core::fmt::Debug;
use core::ops::Deref;
use core::ops::DerefMut;
use core::ptr::NonNull;

use crate::key;
use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node;
use crate::raw::node::iter::KeyIter3;
use crate::sync::Atomic128;

pub(super) const CAPACITY: usize = 1;

#[derive(Debug, Default)]
#[repr(C, align(16))]
pub(in crate::raw) struct Node1(node::Node<CAPACITY, Header>);

const_assert_size_align!(Node1, 16, 16);

impl Node1 {
    pub(crate) fn new_path<R: key::Read<Edge = M>, M: edge::Meta>(
        meta: M,
        byte: u8,
        mut reader: R,
        value: u64,
    ) -> (Edge<M>, NonNull<Atomic128<Edge<M>>>) {
        let head = Box::new(Self::default());
        let mut tail = NonNull::from(&head.0.edges[0]);

        loop {
            let edge = reader.get_edge(<M as edge::Meta>::Len::MAX);

            // Base case
            let Some(byte) = reader.get_byte(edge.len()) else {
                unsafe { tail.as_mut() }.set(Edge::<M>::new_value(edge, value).erase());
                break;
            };

            // Inductive case
            let node = Box::new(Self::default());
            let next_tail = NonNull::from(&node.0.edges[0]);
            unsafe { tail.as_mut() }
                .set(Edge::<M>::new_node(edge, node::Ptr::new_node_1(node, byte)).erase());

            reader = reader.suffix(R::Len::BYTE + edge.len().into());
            tail = next_tail;
        }

        let head = Edge::<M>::new_node(meta, node::Ptr::new_node_1(head, byte));
        (head, tail.cast())
    }

    // pub(crate) fn get(&self, key: u8) -> Option<u8> {
    //     self.0.edges[0].load(Ordering::Relaxed)
    // }
}

impl Deref for Node1 {
    type Target = Node<CAPACITY, Header>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Node1 {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Header;

unsafe impl node::Header for Header {
    const TYPE: node::Type = node::Type::Node1;
    type KeyIter = KeyIter3;

    #[inline]
    fn keys<L: node::iter::Lower, U: node::iter::Upper>(&self, _: L, _: U, _: &mut Self::KeyIter) {
        unreachable!()
    }

    #[inline]
    fn get(&self, _: u8) -> Option<u8> {
        unreachable!()
    }

    #[inline]
    fn get_or_insert(&self, _: u8) -> Option<u8> {
        unreachable!()
    }

    #[inline]
    fn freeze(&self) -> usize {
        CAPACITY
    }

    #[inline]
    fn min<L: node::Lower>(&self, _lower: L) -> Option<node::KeyIndex> {
        todo!()
    }

    #[inline]
    fn max<U: node::Upper>(&self, _upper: U) -> Option<node::KeyIndex> {
        todo!()
    }

    #[inline]
    fn len(&self) -> usize {
        CAPACITY
    }

    #[inline]
    fn is_frozen(&self) -> bool {
        false
    }
}
