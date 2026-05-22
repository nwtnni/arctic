mod be;
mod le;

pub(crate) use be::Be;
pub(crate) use le::Le;
use ribbit::u6;

use core::fmt::Debug;
use core::ops::Add;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use ribbit::Atomic;
use ribbit::OptionExt as _;

use crate::raw::edge;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node;
use crate::raw::node::Node3;

/// A fat pointer to a value or a node.
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
    pub(crate) unsafe fn as_value_unchecked(edge: NonNull<Atomic<Self>>) -> NonNull<u64> {
        unsafe {
            validate!(
                edge.as_ref()
                    .load_packed(Ordering::Relaxed)
                    .meta()
                    .is_value()
            );

            if cfg!(target_endian = "little") {
                edge.byte_add(8)
            } else {
                edge
            }
            .cast::<u64>()
        }
    }

    #[inline]
    pub(crate) fn new_path<R>(
        mut reader: R,
        value: u64,
    ) -> (
        ribbit::Packed<Self>,
        Option<NonNull<ribbit::Atomic<Edge<M>>>>,
    )
    where
        R: key::Read<Edge = M>,
    {
        let edge = reader.get_edge(<ribbit::Packed<M> as edge::Meta>::Len::MAX);

        let Some(byte) = reader.get_byte(edge.len()) else {
            // Fast path: remaining bytes fit in one edge
            return (Self::new_value(edge, value), None);
        };

        reader = reader.suffix(R::Len::BYTE + edge.len().into());

        // Key always fits in one edge
        if R::LEN.is_some_and(|len| len <= <ribbit::Packed<M> as edge::Meta>::Len::MAX.into()) {
            validate!(false);
            unsafe { core::hint::unreachable_unchecked() }
        }

        // Key fits in one edge except at root
        if R::LEN.is_some_and(|len| {
            len == R::Len::BYTE + <ribbit::Packed<M> as edge::Meta>::Len::MAX.into()
        }) {
            crate::cold();
        }

        // Slow path: allocate recursive path of Node3s
        let (head, tail) = Node3::new_path(edge, byte, reader, value);
        (head, Some(tail))
    }

    #[inline]
    pub(super) fn new_node(
        meta: ribbit::Packed<M>,
        node: ribbit::Packed<node::Ptr<M>>,
    ) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(meta.with_value(false), node.raw().get())
    }

    #[inline]
    pub(crate) fn new_value(meta: ribbit::Packed<M>, value: u64) -> ribbit::Packed<Self> {
        ribbit::Packed::<Self>::new(meta.with_value(true), value)
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

pub(crate) trait Meta:
    ribbit::Unpack + core::fmt::Debug + Ord + IntoIterator<Item = u8>
{
    const DEFAULT: Self;

    type Len: Len;

    fn with_value(self, value: bool) -> Self;
    fn with_frozen(self, frozen: bool) -> Self;
    fn with_key(self, key: Self) -> Self;
    #[expect(unused)]
    fn with_inline(self, inline: bool) -> Self;

    fn len(self) -> Self::Len;

    fn is_value(self) -> bool;
    fn is_frozen(self) -> bool;

    fn compress(self, byte: u8, child: Self) -> Option<Self>;
}

pub(crate) trait Len: Copy + Eq + Add<Output = Self> {
    const MAX: Self;

    fn bits(self) -> usize;

    #[inline]
    fn bytes(self) -> usize {
        self.bits() >> 3
    }
}

impl Len for u6 {
    const MAX: Self = u6::new(56);

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
