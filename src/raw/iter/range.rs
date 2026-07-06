use core::cmp;
use core::fmt::Debug;
use core::marker::PhantomData;
use core::ops::ControlFlow;
use core::ops::RangeFrom;
use core::ops::RangeFull;
use core::ops::RangeInclusive;
use core::ops::RangeToInclusive;
use core::ptr::NonNull;
use core::sync::atomic::Ordering;

#[cfg_attr(not(doc), expect(unused))]
use crate::ConcurrentMap;
#[cfg_attr(not(doc), expect(unused))]
use crate::SequentialMap;
use crate::raw;
use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::iter::Order;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node::Lower as _;
use crate::raw::node::Upper as _;
use crate::sync::Atomic;

pub(crate) enum RangeIter<'g, K: key::Read, W: key::Write<K>, R: Range<K>> {
    Root {
        writer: W,
        #[expect(clippy::type_complexity)]
        next: Option<(u64, NonNull<Atomic<Edge<K::Edge>>>)>,
    },
    Node(NodeIter<'g, K, W, R>),
}

impl<'g, K, W, R> Default for RangeIter<'g, K, W, R>
where
    K: key::Read,
    W: key::Write<K>,
    R: Range<K>,
{
    fn default() -> Self {
        Self::Root {
            writer: W::default(),
            next: None,
        }
    }
}

impl<'g, K, W, R> RangeIter<'g, K, W, R>
where
    K: key::Read,
    W: key::Write<K>,
    R: Range<K>,
{
    pub(crate) unsafe fn new_unchecked(
        root: *mut Atomic<Edge<K::Edge>>,
        edge: ribbit::Packed<Edge<K::Edge>>,
        prefix: K,
        order: Option<Order>,
        range: &R,
    ) -> Self {
        let Some((root, child)) = NonNull::new(root).zip(edge.child()) else {
            return Self::default();
        };

        let meta = edge.meta();
        let len = prefix.len();
        let mut lower = range.lower(len);
        let mut upper = range.upper(len);

        let Some((lower_byte, upper_byte)) = lower.check(meta).zip(upper.check(meta)) else {
            return Self::default();
        };

        let (writer, len) = W::new(prefix, meta);

        match child {
            edge::Child::Value(value) => Self::Root {
                writer,
                next: Some((value, root)),
            },
            edge::Child::Node(node) => {
                let mut stack = Vec::with_capacity(7);
                stack.push((len, lower_byte, upper_byte, unsafe {
                    node.entries(order.is_some(), lower_byte, upper_byte)
                }));

                Self::Node(NodeIter {
                    order,
                    lower,
                    upper,
                    writer,
                    stack,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn try_fold<F, B, C>(self, init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, (&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<B, C>,
    {
        match self {
            RangeIter::Root { writer, mut next } => {
                if let Some((value, edge)) = next.take() {
                    apply(init, (&writer, value, edge))
                } else {
                    ControlFlow::Continue(init)
                }
            }
            RangeIter::Node(mut iter) => iter.try_fold(init, apply),
        }
    }

    #[inline]
    #[expect(clippy::type_complexity)]
    pub(crate) fn lend(&mut self) -> Option<(&W, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        match self {
            RangeIter::Root { writer, next } => {
                crate::cold();
                let (value, edge) = next.take()?;
                Some((writer, value, edge))
            }
            RangeIter::Node(iter) => iter.lend(),
        }
    }
}

pub(crate) struct NodeIter<'g, K, W, R>
where
    K: key::Read,
    W: key::Write<K>,
    R: Range<K>,
{
    order: Option<Order>,
    lower: R::Lower,
    upper: R::Upper,
    writer: W,
    #[expect(clippy::type_complexity)]
    stack: Vec<(
        W::Len,
        <R::Lower as Lower<K::Edge>>::Bound,
        <R::Upper as Upper<K::Edge>>::Bound,
        raw::node::EntryIter<'g>,
    )>,
}

impl<'g, K, W, R> NodeIter<'g, K, W, R>
where
    K: key::Read,
    R: Range<K>,
    W: key::Write<K>,
{
    #[inline]
    #[expect(clippy::type_complexity)]
    fn lend(&mut self) -> Option<(&W, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        let next = match self.try_fold(None, |init, (_, value, edge)| {
            validate!(init.is_none());
            ControlFlow::Break((value, edge))
        }) {
            ControlFlow::Break((value, edge)) => Some((value, edge)),
            ControlFlow::Continue(init) => {
                validate!(init.is_none());
                init
            }
        };

        next.map(|(value, edge)| (&self.writer, value, edge))
    }

    // Imagine `self.lower` and `self.upper` defining a triangular subtree of
    // the entire tree:
    //
    // ```text
    //        /\
    //       // \
    //      / \  \
    //     /  /   \
    //    /  /\    \
    //   /  /xx\    \
    //  /  /xxxx\    \
    // /  /xxxxxx\    \
    // ```
    //
    // We only need to compare against the bounds on the exterior edges of this
    // triangle; everything in the interior is included in the range, and everything
    // in the exterior is excluded.
    fn try_fold<F, B, C>(&mut self, mut init: C, mut apply: F) -> ControlFlow<B, C>
    where
        F: FnMut(C, (&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<B, C>,
    {
        'vertical: loop {
            let Some((len, lower, upper, iter)) = self.stack.last_mut() else {
                return ControlFlow::Continue(init);
            };

            'horizontal: loop {
                let next = match self.order {
                    None | Some(Order::Ascend) => iter.next(),
                    Some(Order::Descend) => iter.next_back(),
                };

                let Some((mut byte, mut edge)) = next else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let mut len = *len;
                let mut lower = *lower;
                let mut upper = *upper;

                'flatten: loop {
                    let (meta, child) = {
                        let edge = unsafe { edge.cast::<Atomic<Edge<K::Edge>>>().as_ref() }
                            .load_packed(Ordering::Acquire);
                        let Some(child) = edge.child() else {
                            continue 'horizontal;
                        };
                        let meta = edge.meta();
                        (meta, child)
                    };

                    lower = if lower.check(byte) {
                        // Exterior edge, check against bound
                        match self.lower.check(meta) {
                            Some(lower) => lower,
                            // Below lower bound, descending order
                            None if matches!(self.order, Some(Order::Descend)) => {
                                self.stack.clear();
                                return ControlFlow::Continue(init);
                            }
                            // Below lower bound, ascending order
                            None => continue 'horizontal,
                        }
                    } else {
                        // Interior edge
                        Default::default()
                    };

                    upper = if upper.check(byte) {
                        // Exterior edge, check against bound
                        match self.upper.check(meta) {
                            Some(upper) => upper,
                            // Above upper bound, descending order
                            None if matches!(self.order, Some(Order::Descend)) => {
                                continue 'horizontal;
                            }
                            // Above upper bound, ascending order
                            None => {
                                self.stack.clear();
                                return ControlFlow::Continue(init);
                            }
                        }
                    } else {
                        // Interior edge
                        Default::default()
                    };

                    len = self.writer.replace(len, byte, meta);

                    match child {
                        edge::Child::Value(value) => {
                            init = apply(init, (&self.writer, value, edge.cast()))?;
                            continue 'horizontal;
                        }
                        edge::Child::Node(node) => {
                            // Avoid pushing and popping node iterators with only one child
                            match unsafe {
                                node.entry_or_entries(self.order.is_some(), lower, upper)
                            } {
                                Ok((next_byte, next_edge)) => {
                                    byte = next_byte;
                                    edge = next_edge;
                                    continue 'flatten;
                                }
                                Err(iter) => {
                                    self.stack.push((len, lower, upper, iter));
                                    continue 'vertical;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Include<T>(pub(crate) T);

pub struct Unbound<T = ()>(PhantomData<T>);

impl<T> Copy for Unbound<T> {}

impl<T> Clone for Unbound<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Default for Unbound<T> {
    #[inline]
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T> Debug for Unbound<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unbound")
    }
}

/// Native range types (`..`, `lower..`, `..=upper`, `lower..=upper`) that can be passed as bounds to
/// [`SequentialMap::range`] and [`ConcurrentMap::range`].
///
/// Currently, only [`RangeFull`], [`RangeFrom`], [`RangeToInclusive`], and [`RangeInclusive`] are supported.
/// [`Range`][std::ops::Range] (with an exclusive upper bound) is not supported.
/// The following types can be used as range bounds:
/// - Borrowed keys ([`&'_ Key::Borrowed`][crate::Key::Borrowed]),
/// - Slices (`&'_ [u8]`)
/// - Array references (`&'_ [u8; 5]`)
/// - For integer keys, owned integers (`u64`)
#[expect(private_bounds)]
pub trait Range<R>
where
    R: key::Read,
{
    #[doc(hidden)]
    #[expect(private_bounds)]
    type Lower: Lower<R::Edge>;

    #[doc(hidden)]
    #[expect(private_bounds)]
    type Upper: Upper<R::Edge>;

    #[doc(hidden)]
    #[expect(private_interfaces)]
    fn lower(&self, start: R::Len) -> Self::Lower;

    #[doc(hidden)]
    #[expect(private_interfaces)]
    fn upper(&self, start: R::Len) -> Self::Upper;

    #[doc(hidden)]
    #[inline]
    fn common_prefix(&self) -> R {
        R::default()
    }
}

impl<R: key::Read, T: Into<R> + Copy> Range<R> for RangeInclusive<T> {
    type Lower = Include<R>;
    type Upper = Include<R>;

    #[inline]
    #[expect(private_interfaces)]
    fn lower(&self, start: R::Len) -> Self::Lower {
        Include((*self.start()).into().suffix(start))
    }

    #[inline]
    #[expect(private_interfaces)]
    fn upper(&self, start: R::Len) -> Self::Upper {
        Include((*self.end()).into().suffix(start))
    }

    #[inline]
    fn common_prefix(&self) -> R {
        let lower = (*self.start()).into();
        let upper = (*self.end()).into();
        lower.common_prefix(upper)
    }
}

impl<R: key::Read, T: Into<R> + Copy> Range<R> for RangeFrom<T> {
    type Lower = Include<R>;
    type Upper = Unbound<R>;

    #[inline]
    #[expect(private_interfaces)]
    fn lower(&self, start: R::Len) -> Self::Lower {
        Include(self.start.into().suffix(start))
    }

    #[inline]
    #[expect(private_interfaces)]
    fn upper(&self, _start: R::Len) -> Self::Upper {
        Unbound::default()
    }
}

impl<R: key::Read, T: Into<R> + Copy> Range<R> for RangeToInclusive<T> {
    type Lower = Unbound<R>;
    type Upper = Include<R>;

    #[inline]
    #[expect(private_interfaces)]
    fn lower(&self, _start: R::Len) -> Self::Lower {
        Unbound::default()
    }

    #[inline]
    #[expect(private_interfaces)]
    fn upper(&self, start: R::Len) -> Self::Upper {
        Include(self.end.into().suffix(start))
    }
}

impl<R> Range<R> for RangeFull
where
    R: key::Read,
{
    type Lower = Unbound<R>;
    type Upper = Unbound<R>;

    #[inline]
    #[expect(private_interfaces)]
    fn lower(&self, _: R::Len) -> Self::Lower {
        Unbound::default()
    }

    #[inline]
    #[expect(private_interfaces)]
    fn upper(&self, _: R::Len) -> Self::Upper {
        Unbound::default()
    }
}

trait Lower<M>: Debug
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    type Bound: raw::node::Lower;

    fn check(&mut self, edge: ribbit::Packed<M>) -> Option<Self::Bound>;
}

trait Upper<M>: Debug
where
    M: ribbit::Pack<Packed: edge::Meta>,
{
    type Bound: raw::node::Upper;

    fn check(&mut self, edge: ribbit::Packed<M>) -> Option<Self::Bound>;
}

#[expect(private_bounds)]
impl<R: key::Read> Include<R> {
    #[inline]
    fn check_eq(&mut self, len: <ribbit::Packed<R::Edge> as edge::Meta>::Len) -> Option<u8> {
        let next = self.0.get_byte(len);
        let skip = match next {
            None => R::Len::ZERO,
            Some(_) => R::Len::BYTE,
        };
        self.0 = self.0.suffix(skip + len.into());
        next
    }
}

impl<R: key::Read> Lower<R::Edge> for Include<R> {
    type Bound = Option<u8>;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        let len = edge.len();
        match edge.cmp(&self.0.get_edge(len)) {
            cmp::Ordering::Less => None,
            cmp::Ordering::Equal => Some(self.check_eq(len)),
            cmp::Ordering::Greater => Some(None),
        }
    }
}

impl<R: key::Read> Upper<R::Edge> for Include<R> {
    type Bound = Option<u8>;

    fn check(&mut self, edge: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        let len = edge.len();
        match edge.cmp(&self.0.get_edge(len)) {
            cmp::Ordering::Less => Some(None),
            cmp::Ordering::Equal => Some(self.check_eq(len)),
            cmp::Ordering::Greater => None,
        }
    }
}

impl<R: key::Read> Lower<R::Edge> for Unbound<R> {
    type Bound = Unbound<R>;

    #[inline]
    fn check(&mut self, _: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound::default())
    }
}

impl<R: key::Read> Upper<R::Edge> for Unbound<R> {
    type Bound = Unbound<R>;

    #[inline]
    fn check(&mut self, _: ribbit::Packed<R::Edge>) -> Option<Self::Bound> {
        Some(Unbound::default())
    }
}
