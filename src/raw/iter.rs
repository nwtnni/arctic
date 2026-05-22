mod postorder;
pub(crate) mod range;

pub(crate) use postorder::PostorderIter;
pub use range::Range;
pub(crate) use range::RangeIter;
pub(crate) use range::Unbound;

use core::ops::ControlFlow;
use core::ptr::NonNull;

use ribbit::Atomic;

use crate::Order;
use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::key;

pub(crate) struct EntryIter<'g, 'k, K: Key, R: Range<K::Read<'k>>, O>(
    pub(super) RangeIter<'g, K::Read<'k>, K::Write, R, O>,
);

impl<'g, 'k, K, R, O> EntryIter<'g, 'k, K, R, O>
where
    K: Key,
    R: Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(K::Insert<'_>, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.0.lend().map(|(writer, value, edge)| {
            (unsafe { K::borrow_writer_unchecked(writer) }, value, edge)
        })
    }

    #[inline]
    pub(crate) fn for_each_internal<
        F: FnMut((K::Insert<'_>, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        self,
        mut apply: F,
    ) {
        self.0.for_each_internal(|(writer, value, edge)| {
            apply((unsafe { K::borrow_writer_unchecked(writer) }, value, edge))
        })
    }
}

/// Iterator over raw values only
pub(crate) struct ValueIter<'g, 'k, K: Key, R: Range<K::Read<'k>>, O>(
    pub(super) RangeIter<'g, K::Read<'k>, key::Discard<K::Read<'k>>, R, O>,
);

impl<'g, 'k, K, R, O> ValueIter<'g, 'k, K, R, O>
where
    K: Key,
    R: Range<K::Read<'k>>,
    O: Order,
{
    #[inline]
    pub(crate) fn lend(&mut self) -> Option<(u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.0.lend().map(|(_, value, edge)| (value, edge))
    }

    #[inline]
    pub(crate) fn for_each_internal<
        F: FnMut((u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        self,
        mut apply: F,
    ) {
        self.0
            .for_each_internal(|(_, value, edge)| apply((value, edge)))
    }
}
