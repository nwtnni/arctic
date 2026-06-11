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

use crate::Atomic;
use crate::Order;
use crate::raw;
use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key;
use crate::raw::key::Len as _;
use crate::raw::node::Lower as _;
use crate::raw::node::Upper as _;

pub(crate) enum RangeIter<'g, K: key::Read, W: key::Write<K>, R: Range<K>, O> {
    Root {
        writer: W,
        next: Option<(u64, NonNull<Atomic<Edge<K::Edge>>>)>,
    },
    Node(NodeIter<'g, K, W, R, O>),
}

impl<'g, K, W, R, O> Default for RangeIter<'g, K, W, R, O>
where
    K: key::Read,
    W: key::Write<K>,
    R: Range<K>,
    O: Order,
{
    fn default() -> Self {
        Self::Root {
            writer: W::default(),
            next: None,
        }
    }
}

impl<'g, K, W, R, O> RangeIter<'g, K, W, R, O>
where
    K: key::Read,
    W: key::Write<K>,
    R: Range<K>,
    O: Order,
{
    pub(crate) unsafe fn new_unchecked(
        root: *mut Atomic<Edge<K::Edge>>,
        edge: ribbit::Packed<Edge<K::Edge>>,
        prefix: K,
        sort: bool,
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
                    node.entries(sort, lower_byte, upper_byte)
                }));

                Self::Node(NodeIter {
                    sort,
                    lower,
                    upper,
                    writer,
                    stack,
                    _order: PhantomData,
                })
            }
        }
    }

    #[inline]
    pub(crate) fn for_each_internal<
        F: FnMut((&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        self,
        mut apply: F,
    ) {
        match self {
            RangeIter::Root { writer, mut next } => {
                crate::cold();
                if let Some((value, edge)) = next.take() {
                    let _ = apply((&writer, value, edge));
                }
            }
            RangeIter::Node(mut iter) => iter.for_each_internal(apply),
        }
    }

    #[inline]
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

pub(crate) struct NodeIter<'g, K, W, R, O>
where
    K: key::Read,
    W: key::Write<K>,
    R: Range<K>,
{
    sort: bool,
    lower: R::Lower,
    upper: R::Upper,
    writer: W,
    stack: Vec<(
        W::Len,
        <R::Lower as Lower<K::Edge>>::Bound,
        <R::Upper as Upper<K::Edge>>::Bound,
        raw::node::EntryIter<'g>,
    )>,
    _order: PhantomData<O>,
}

impl<'g, K, W, R, O> NodeIter<'g, K, W, R, O>
where
    K: key::Read,
    R: Range<K>,
    W: key::Write<K>,
    O: Order,
{
    #[inline]
    fn lend(&mut self) -> Option<(&W, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        self.walk::<true, _>(|(_, _, _)| unreachable!())
    }

    #[inline]
    fn for_each_internal<F: FnMut((&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>>(
        &mut self,
        apply: F,
    ) {
        self.walk::<false, _>(apply);
    }

    fn walk<
        const YIELD: bool,
        F: FnMut((&W, u64, NonNull<Atomic<Edge<K::Edge>>>)) -> ControlFlow<()>,
    >(
        &mut self,
        mut apply: F,
    ) -> Option<(&W, u64, NonNull<Atomic<Edge<K::Edge>>>)> {
        'vertical: loop {
            let (len, lower, upper, iter) = self.stack.last_mut()?;
            let len = *len;

            'horizontal: loop {
                let Some((mut byte, mut edge)) = (if O::ASCEND {
                    iter.next()
                } else {
                    iter.next_back()
                }) else {
                    self.stack.pop();
                    continue 'vertical;
                };

                let mut len = len;
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

                    len = self.writer.replace(len, byte, meta);

                    let lower_next = if lower.check(byte) {
                        match self.lower.check(meta) {
                            Some(lower) => lower,
                            None if O::ASCEND => continue 'horizontal,
                            None => {
                                self.stack.clear();
                                return None;
                            }
                        }
                    } else {
                        Default::default()
                    };

                    let upper_next = if upper.check(byte) {
                        match self.upper.check(meta) {
                            Some(upper) => upper,
                            None if O::ASCEND => {
                                self.stack.clear();
                                return None;
                            }
                            None => continue 'horizontal,
                        }
                    } else {
                        Default::default()
                    };

                    match child {
                        edge::Child::Value(value) if YIELD => {
                            return Some((&self.writer, value, edge.cast()));
                        }
                        edge::Child::Value(value) => {
                            match apply((&self.writer, value, edge.cast())) {
                                ControlFlow::Continue(()) => continue 'horizontal,
                                ControlFlow::Break(()) => {
                                    self.stack.clear();
                                    return None;
                                }
                            }
                        }
                        edge::Child::Node(node) => {
                            lower = lower_next;
                            upper = upper_next;

                            // Avoid pushing and popping iterators with only one child
                            match unsafe { node.entry_or_entries(self.sort, lower, upper) } {
                                Ok((byte_, edge_)) => {
                                    byte = byte_;
                                    edge = edge_;
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
/// [`crate::sequential::Map::range`] and [`crate::concurrent::Map::range`].
///
/// Currently, only [`RangeFull`], [`RangeFrom`], [`RangeToInclusive`], and [`RangeInclusive`] are supported.
/// [`Range`][std::ops::Range] (with an exclusive upper bound) is not supported.
///
/// The endpoints of the ranges can be borrowed keys ([`&'_ Key::Borrowed`][crate::Key::Borrowed]),
/// key prefixes (e.g., [`crate::raw::key::int::Reader`]), or, for integer keys, owned integers.
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
