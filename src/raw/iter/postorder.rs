use core::sync::atomic::Ordering;

use ribbit::Atomic;

use crate::raw::Edge;
use crate::raw::edge;
use crate::raw::iter::Unbound;
use crate::raw::node;

pub(crate) struct PostorderIter<'g, M: ribbit::Pack> {
    stack: Vec<RepeatIter<'g, M>>,
}

impl<'g, M> PostorderIter<'g, M>
where
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
{
    #[inline]
    pub(crate) unsafe fn new(root: &'g Atomic<Edge<M>>) -> Self {
        // HACK: we're masquerading as a node here--this is okay
        // since this iterator doesn't keep track of the key state,
        // so we can use an arbitrary byte.
        Self {
            stack: vec![RepeatIter::new(unsafe {
                node::EntryIter::new(node::KeyIter::ROOT, core::slice::from_ref(root))
            })],
        }
    }

    #[inline]
    pub(crate) fn for_each_internal<F: FnMut(ribbit::Packed<Edge<M>>, usize)>(
        mut self,
        mut apply: F,
    ) {
        'vertical: loop {
            let depth = self.stack.len().saturating_sub(1);

            let Some(iter) = self.stack.last_mut() else {
                return;
            };

            loop {
                let Some((first, edge)) = iter.next() else {
                    self.stack.pop();
                    continue 'vertical;
                };

                if first {
                    match edge.child() {
                        // Fall through for non-nodes
                        None | Some(edge::Child::Value(_)) => {
                            iter.skip();
                        }
                        // Visit children before node
                        Some(edge::Child::Node(node)) => {
                            self.stack.push(RepeatIter::new(unsafe {
                                node.entries::<_, _>(
                                    Unbound::<()>::default(),
                                    Unbound::<()>::default(),
                                )
                            }));
                            continue 'vertical;
                        }
                    }
                }

                apply(edge, depth);
            }
        }
    }
}

struct RepeatIter<'g, M: ribbit::Pack> {
    first: bool,
    edge: ribbit::Packed<Edge<M>>,
    iter: node::EntryIter<'g, M>,
}

impl<'g, M> RepeatIter<'g, M>
where
    M: ribbit::Pack<Packed: edge::Meta> + 'g,
{
    #[inline]
    fn new(iter: node::EntryIter<'g, M>) -> Self {
        Self {
            first: true,
            edge: Edge::DEFAULT,
            iter,
        }
    }

    #[inline]
    fn next(&mut self) -> Option<(bool, ribbit::Packed<Edge<M>>)> {
        let first = self.first;
        self.first ^= true;

        if first {
            let (_, edge) = self.iter.next()?;
            let edge = unsafe { edge.as_ref() }.load_packed(Ordering::Acquire);
            self.edge = edge;
        }

        Some((first, self.edge))
    }

    #[inline]
    fn skip(&mut self) {
        self.first = true;
    }
}
