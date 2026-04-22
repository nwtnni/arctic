mod be;
mod le;
mod slice;

pub(crate) use be::Be;
pub(crate) use le::Le;
use ribbit::u6;
pub(crate) use slice::Slice;

use core::fmt::Debug;
use core::ops::Add;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::OptionExt as _;

use crate::raw::key;
use crate::raw::node;
use crate::raw::node::Node as _;
use crate::raw::node::Node3;
use crate::stat;

#[derive(Copy, Clone, Default, ribbit::Pack)]
#[ribbit(size = 128, packed(rename = EdgePacked))]
pub(crate) struct Edge<M> {
    #[ribbit(size = 64)]
    pub(crate) meta: M,

    #[ribbit(get(rename = "child_raw"))]
    child: u64,
}

impl<M: ribbit::Pack<Packed: Meta>> Edge<M> {
    pub(crate) const DEFAULT: ribbit::Packed<Self> =
        ribbit::Packed::<Self>::new(<M::Packed as Meta>::DEFAULT, 0);

    #[inline]
    pub(crate) unsafe fn as_value_unchecked<'g>(edge: NonNull<Atomic<Self>>) -> &'g u64 {
        unsafe {
            if cfg!(target_endian = "little") {
                edge.byte_add(8)
            } else {
                edge
            }
            .cast::<u64>()
            .as_ref()
        }
    }

    #[inline]
    pub(crate) unsafe fn as_value_mut_unchecked<'g>(edge: NonNull<Atomic<Self>>) -> &'g mut u64 {
        unsafe {
            if cfg!(target_endian = "little") {
                edge.byte_add(8)
            } else {
                edge
            }
            .cast::<u64>()
            .as_mut()
        }
    }

    #[inline]
    pub(crate) fn new_path<R>(mut reader: R, value: u64) -> ribbit::Packed<Self>
    where
        R: key::Read<Edge = M>,
    {
        let key = reader.read(<<M::Packed as Meta>::Key as Key>::Len::MAX);
        let Some(byte) = reader.next() else {
            return Self::new_value(key, value);
        };
        Self::new_path_cold(reader, key, byte, value)
    }

    #[cold]
    fn new_path_cold<R>(
        mut reader: R,
        key: <<R::Edge as ribbit::Pack>::Packed as Meta>::Key,
        mut byte: u8,
        value: u64,
    ) -> ribbit::Packed<Self>
    where
        R: key::Read<Edge = M>,
    {
        let mut tail = NonNull::from(Box::leak(Box::new(Node3::default())));
        let head = ribbit::Packed::<Self>::new(
            <M::Packed as Meta>::new(key, false),
            node::Ptr::new_ptr(tail).raw().get(),
        );

        loop {
            let edge = unsafe { tail.as_mut().insert(byte) };
            let edge = if cfg!(feature = "validate") {
                edge.expect("Node3 fits one edge")
            } else {
                unsafe { edge.unwrap_unchecked() }
            };

            let key = reader.read(<<M::Packed as Meta>::Key as Key>::Len::MAX);

            let Some(next_byte) = reader.next() else {
                edge.set_packed(Self::new_value(key, value));
                return head;
            };
            byte = next_byte;

            let next_node = NonNull::from(Box::leak(Box::new(Node3::default())));
            edge.set_packed(ribbit::Packed::<Self>::new(
                <M::Packed as Meta>::new(key, false),
                node::Ptr::new_ptr(next_node).raw().get(),
            ));
            tail = next_node;
        }
    }

    #[cold]
    pub(crate) fn new_node<N, K, E>(
        key: <<M as ribbit::Pack>::Packed as Meta>::Key,
        keys: K,
        edges: E,
    ) -> ribbit::Packed<Self>
    where
        N: node::Node<M>,
        K: IntoIterator<Item = u8>,
        E: IntoIterator<Item = ribbit::Packed<Edge<M>>>,
    {
        unsafe {
            Self::new_node_unchecked::<N, K, E>(<M::Packed as Meta>::new(key, false), keys, edges)
        }
    }

    #[cold]
    pub(crate) unsafe fn new_node_unchecked<N, K, E>(
        meta: ribbit::Packed<M>,
        keys: K,
        edges: E,
    ) -> ribbit::Packed<Self>
    where
        N: node::Node<M>,
        K: IntoIterator<Item = u8>,
        E: IntoIterator<Item = ribbit::Packed<Edge<M>>>,
    {
        validate!(!meta.is_frozen());
        validate!(!meta.is_value());

        let mut node = Box::new(N::default());

        for (key, edge) in keys.into_iter().zip(edges) {
            node.insert(key)
                .expect("Node can fit all edges")
                .set_packed(edge);
        }

        ribbit::Packed::<Self>::new(meta, node::Ptr::new(node).raw().get())
    }

    fn new_value(
        key: <<M as ribbit::Pack>::Packed as Meta>::Key,
        value: u64,
    ) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(<M::Packed as Meta>::new(key, true), value)
    }

    #[inline]
    pub(crate) fn freeze(edge: &Atomic<Self>) {
        let mut old = edge.load_packed(Ordering::Relaxed);

        while !old.meta().is_frozen() {
            match edge.compare_exchange_packed(
                old,
                old.with_meta(old.meta().with_frozen(true)),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
    }
}

impl<M: ribbit::Pack<Packed: Meta>> EdgePacked<M> {
    #[inline]
    pub(crate) fn is_null(self) -> bool {
        !self.meta().is_value() && self.child_raw() == 0
    }

    #[inline]
    pub(crate) fn as_node(self) -> Option<ribbit::Packed<node::Ptr<M>>> {
        if self.meta().is_value() {
            return None;
        }

        unsafe { ribbit::Packed::<Option<node::Ptr<M>>>::new_unchecked(self.child_raw()) }
    }

    #[inline]
    pub(crate) fn as_value(self) -> Option<u64> {
        self.meta().is_value().then(|| self.child_raw())
    }

    #[inline]
    pub(crate) fn child(self) -> Option<Child<M>> {
        let raw = self.child_raw();
        if self.meta().is_value() {
            Some(Child::Value(raw))
        } else {
            unsafe { ribbit::Packed::<Option<node::Ptr<M>>>::new_unchecked(raw) }.map(Child::Node)
        }
    }

    /// # Safety
    ///
    /// Caller must ensure that child is a value.
    #[inline]
    pub(crate) unsafe fn into_value_unchecked(self) -> u64 {
        validate!(self.meta().is_value());
        self.child_raw()
    }

    /// # Safety
    ///
    /// Caller must ensure that child is a value.
    #[inline]
    pub(crate) unsafe fn with_value_unchecked(self, value: u64) -> Self {
        validate!(self.meta().is_value());
        self.with_child(value)
    }

    #[inline]
    pub(super) fn unfreeze(self) -> Self {
        self.with_meta(self.meta().with_frozen(false))
    }

    #[inline]
    pub(crate) unsafe fn deallocate<F>(self, deallocate_value: F, counter: stat::Counter)
    where
        F: FnOnce(u64),
    {
        if self.is_null() {
            return;
        }

        unsafe { self.deallocate_unchecked(deallocate_value, counter) }
    }

    #[inline]
    pub(crate) unsafe fn deallocate_unchecked<F>(self, deallocate_value: F, counter: stat::Counter)
    where
        F: FnOnce(u64),
    {
        match self.child() {
            None if cfg!(feature = "validate") => unreachable!(),
            None => unsafe { core::hint::unreachable_unchecked() },
            Some(Child::Node(node)) => unsafe { node.deallocate(counter) },
            Some(Child::Value(value)) => deallocate_value(value),
        }
    }

    #[inline]
    pub(crate) unsafe fn deallocate_recursive_unchecked(self, counter: stat::Counter) {
        match self.child() {
            None if cfg!(feature = "validate") => unreachable!(),
            None => unsafe { core::hint::unreachable_unchecked() },
            Some(Child::Node(node)) => unsafe { node.deallocate_recursive(counter) },
            Some(Child::Value(_)) => (),
        }
    }
}

impl<M: ribbit::Pack> Debug for EdgePacked<M>
where
    M::Packed: Meta + core::fmt::Debug,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut debug = f.debug_struct("Edge");

        debug.field("meta", &self.meta());
        debug.field("data", &self.child());

        debug.finish()
    }
}

pub(crate) trait Meta: ribbit::Unpack + core::fmt::Debug {
    const DEFAULT: Self;

    type Key: Key;

    fn new(key: Self::Key, value: bool) -> Self;
    fn with_frozen(self, frozen: bool) -> Self;

    fn key(self) -> Self::Key;
    fn is_value(self) -> bool;
    fn is_frozen(self) -> bool;

    fn expand(self, new: Self::Key) -> Result<(Self::Key, u8, Self), ()>;
    fn compress(self, byte: u8, child: Self) -> Option<Self>;
}

pub(crate) trait Key: Copy + Eq + Ord + core::fmt::Debug + IntoIterator<Item = u8> {
    type Len: Len;

    fn len(self) -> Self::Len;
    fn prefix(self, len: Self::Len) -> Self;
}

pub(crate) trait Len: Copy + Eq + Add<Output = Self> {
    const MAX: Self;

    #[cfg_attr(not(test), expect(unused))]
    fn new(bits: usize) -> Self;
    fn bits(self) -> usize;
}

impl Len for u6 {
    const MAX: Self = u6::new(56);

    #[inline]
    fn new(bits: usize) -> Self {
        validate_eq!(bits & 0b111, 0);
        validate!(bits <= u8::MAX as usize);
        u6::new(bits as u8)
    }

    #[inline]
    fn bits(self) -> usize {
        self.value() as usize
    }
}

pub(crate) enum Child<M> {
    Node(ribbit::Packed<node::Ptr<M>>),
    Value(u64),
}

impl<M> Debug for Child<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Node(node) => f.debug_tuple("Node").field(node).finish(),
            Self::Value(value) => f.debug_tuple("Value").field(value).finish(),
        }
    }
}
