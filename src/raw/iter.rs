mod postorder;
pub(crate) mod range;

pub(crate) use postorder::PostorderIter;
pub use range::Range;
pub(crate) use range::RangeIter;
pub(crate) use range::Unbound;

use core::ops::ControlFlow;
use core::ptr::NonNull;

use crate::Order;
use crate::raw::Edge;
use crate::raw::Key;
use crate::raw::key;
use crate::sync::Atomic;

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
    #[expect(clippy::type_complexity)]
    pub(crate) fn lend(&mut self) -> Option<(K::Insert<'_>, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.0
            .lend()
            .map(|(writer, value, edge)| (unsafe { K::write_as_insert(writer) }, value, edge))
    }

    #[inline]
    pub(crate) fn try_fold<F, B, C>(self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, (K::Insert<'_>, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<B, C>,
    {
        self.0.try_fold(init, |acc, (writer, value, edge)| {
            apply(acc, (unsafe { K::write_as_insert(writer) }, value, edge))
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
    #[expect(clippy::type_complexity)]
    pub(crate) fn lend(&mut self) -> Option<(u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.0.lend().map(|(_, value, edge)| (value, edge))
    }

    #[inline]
    pub(crate) fn try_fold<F, B, C>(self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, (u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<B, C>,
    {
        self.0
            .try_fold(init, |acc, (_, value, edge)| apply(acc, (value, edge)))
    }
}
