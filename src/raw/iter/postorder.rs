use core::ptr::NonNull;
use core::sync::atomic::Ordering;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::sync::Atomic;

pub(crate) struct PostorderIter<'g, M: ribbit::Pack> {
    sort: bool,
    stack: Vec<RepeatIter<'g, M>>,
}

impl<'g, M> PostorderIter<'g, M>
where
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
{
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic<Edge<M>>, sort: bool) -> Self {
        // HACK: we're masquerading as a node here--this is okay
        // since this iterator doesn't keep track of the key state,
        // so we can use an arbitrary byte.
        Self {
            sort,
            stack: vec![RepeatIter::new(unsafe {
                node::EntryIter::new(
                    node::KeyIter::ROOT,
                    core::slice::from_ref(core::mem::transmute::<
                        &'g Atomic<Edge<M>>,
                        &'g Atomic<edge::Raw>,
                    >(root)),
                )
            })],
        }
    }

    #[inline]
    pub(crate) fn for_each_internal<F: FnMut(ribbit::Packed<M>, edge::Child)>(
        mut self,
        mut apply: F,
    ) {
        'vertical: loop {
            let Some(iter) = self.stack.last_mut() else {
                return;
            };

            'horizontal: loop {
                let Some((mut first, mut edge)) = iter.next() else {
                    self.stack.pop();
                    continue 'vertical;
                };

                'flatten: loop {
                    let (meta, child) = {
                        let edge = unsafe { edge.as_ref() }.load_packed(Ordering::Acquire);
                        let Some(child) = edge.child() else {
                            continue 'horizontal;
                        };
                        let meta = edge.meta();
                        (meta, child)
                    };

                    match child {
                        // Visit children before node
                        edge::Child::Node(node) if first => {
                            match unsafe {
                                node.entry_or_entries::<_, _>(
                                    self.sort,
                                    Unbound::<()>::default(),
                                    Unbound::<()>::default(),
                                )
                            } {
                                Ok((_, edge_)) => {
                                    first = true;
                                    edge = edge_.cast();
                                    continue 'flatten;
                                }
                                Err(iter) => {
                                    self.stack.push(RepeatIter::new(iter));
                                    continue 'vertical;
                                }
                            }
                        }
                        _ => {
                            iter.skip();
                            apply(meta, child);
                            continue 'horizontal;
                        }
                    }
                }
            }
        }
    }
}

struct RepeatIter<'g, M: ribbit::Pack> {
    first: bool,
    edge: NonNull<Atomic<Edge<M>>>,
    iter: node::EntryIter<'g>,
}

impl<'g, M> RepeatIter<'g, M>
where
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
{
    #[inline]
    fn new(iter: node::EntryIter<'g>) -> Self {
        Self {
            first: true,
            edge: NonNull::dangling(),
            iter,
        }
    }

    #[inline]
    fn next(&mut self) -> Option<(bool, NonNull<Atomic<Edge<M>>>)> {
        let first = self.first;
        self.first ^= true;

        if first {
            let (_, edge) = self.iter.next()?;
            self.edge = edge.cast();
        }

        Some((first, self.edge))
    }

    #[inline]
    fn skip(&mut self) {
        self.first = true;
    }
}
