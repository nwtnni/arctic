//! [`Node47`] can contain at most 47 key-edge pairs.
//!
//! Note that, unlike [`crate::raw::node::Node3`] and [`crate::raw::node::Node15`],
//! [`crate::raw::node::Node47`]'s header **cannot** be updated in a single
//! atomic operation, and requires more careful reasoning.

use core::fmt::Debug;
use core::ops::Shr;
use core::sync::atomic::Ordering;

use ribbit::u6;

use crate::Atomic;
use crate::raw::iter::Unbound;
use crate::raw::node;
use crate::raw::node::Node;
use crate::raw::node::header;
use crate::raw::node::iter::KeyIndex;
use crate::raw::node::iter::KeyIter63;
use crate::stat;

const CAPACITY: usize = 47;

/// [`Node`] representation that contains at most 47 key-edge pairs.
pub(super) type Node47 = Node<CAPACITY, Header>;

// Note: aligning to 1024 would require a newtype wrapper
// and more boilerplate. Just assume a
// reasonable memory allocator will have a dedicated
// size class for 1KiB.
const_assert_size_align!(Node47, 1024, 64);

#[repr(C, align(16))]
pub(super) struct Header {
    indices: [Atomic<u128>; 16],
    meta: Atomic<Meta>,
}

impl Default for Header {
    fn default() -> Self {
        Self {
            // NOTE: fill in uninitialized indices with 0x7F so that
            // readers can call `get` without comparing against the
            // length. Don't use 0xFF because AVX2 only supports
            // signed byte-wise comparison.
            indices: core::array::from_fn(|_| {
                Atomic::new_packed(0x7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F_7F7F)
            }),
            meta: Atomic::new_packed(Meta::DEFAULT),
        }
    }
}

unsafe impl header::Header for Header {
    const TYPE: node::Type = node::Type::Node47;
    type KeyIter = KeyIter63;

    unsafe fn initialize_unchecked(&mut self, keys: &[u8]) {
        for (i, key) in keys.iter().enumerate() {
            let (row, col) = Self::key_to_row_col(*key);
            let row = &mut self.indices[row as usize];
            let old = row.get();
            let new = old ^ (0x7F ^ i as u128) << col;
            row.set(new);
        }

        self.meta.set_packed(ribbit::Packed::<Meta>::new(
            keys.last().copied().unwrap(),
            false,
            u6::new(keys.len() as u8),
        ));
    }

    fn freeze(&self) -> usize {
        let mut old = self.meta.load_packed(Ordering::Relaxed);
        while !old.frozen() {
            self.ensure_meta_consistent(old);
            match self.meta.compare_exchange_packed(
                old,
                old.with_frozen(true),
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(conflict) => old = conflict,
            }
        }
        old.len().value() as usize
    }

    fn get(&self, key: u8) -> Option<u8> {
        let (row, col) = Self::key_to_row_col(key);
        validate!(col < 128);
        let index = self.indices[row as usize].load(Ordering::Relaxed).shr(col) as u8;
        (index < CAPACITY as u8).then_some(index)
    }

    fn get_or_insert(&self, key: u8) -> Option<u8> {
        loop {
            if let Some(index) = self.get(key) {
                return Some(index);
            }

            let old = self.meta_consistent();
            let len = old.len().value();

            // NOTE: tricky edge case here, where the above `get`
            // call returns `None` between another thread updating
            // the metadata and the data array being updated.
            if key == old.last() {
                let index = len.checked_sub(1);
                validate!(index.is_some());
                return index;
            }

            if len == CAPACITY as u8 || old.frozen() {
                return None;
            }

            let new = old.with_len(u6::new(len + 1)).with_last(key);

            match self
                .meta
                .compare_exchange_packed(old, new, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.ensure_meta_consistent(new);
                    return Some(len);
                }
                Err(_) => continue,
            }
        }
    }

    // fn insert(&mut self, key: u8) -> Option<u8> {
    //     let old_meta = self.meta.get_packed();
    //     let len = old_meta.len().value();
    //
    //     validate!(!old_meta.frozen());
    //     validate!(len <= 47);
    //
    //     if len == 47 {
    //         return None;
    //     }
    //
    //     let new_meta = old_meta.with_len(u6::new(len + 1)).with_last(key);
    //     self.meta.set_packed(new_meta);
    //
    //     let (row, col) = Self::key_to_row_col(key);
    //
    //     let data = unsafe { self.data_unchecked_mut(row) };
    //
    //     let old_data = *data.get_mut();
    //     let hole = !(0xFFu64 << col);
    //     let new_data = old_data & hole | ((len as u64) << col);
    //
    //     *data.get_mut() = new_data;
    //     Some(len)
    // }

    fn keys<L: node::iter::Lower, U: node::iter::Upper>(
        &self,
        lower: L,
        upper: U,
        iter: &mut KeyIter63,
    ) {
        let len = self.meta_consistent().len().value();
        let indices = core::array::from_fn(|i| self.indices[i].load(Ordering::Relaxed));
        node::simd::keys_47(indices, lower, upper, len, iter);
    }

    fn min<L: node::Lower>(&self, _lower: L) -> Option<KeyIndex> {
        todo!()
    }

    fn max<U: node::Upper>(&self, _upper: U) -> Option<KeyIndex> {
        todo!()
    }
}

impl Header {
    fn meta_consistent(&self) -> ribbit::Packed<Meta> {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        self.ensure_meta_consistent(meta);
        meta
    }

    fn ensure_meta_consistent(&self, meta: ribbit::Packed<Meta>) {
        let len = meta.len().value();
        validate!(len <= CAPACITY as u8);
        let index = len - 1;

        let key = meta.last();
        let (row, col) = Self::key_to_row_col(key);

        let row = &self.indices[row as usize];
        let old = row.load(Ordering::Relaxed);

        if (old >> col) as u8 == index {
            stat::increment(stat::Counter::Node47Consistent);
            return;
        }

        let hole = !(0xFFu128 << col);
        let new = old & hole | ((index as u128) << col);

        match row.compare_exchange(old, new, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => {
                stat::increment(stat::Counter::Node47CasSuccess);
            }
            Err(_) => stat::increment(stat::Counter::Node47CasFailure),
        }
    }

    fn key_to_row_col(key: u8) -> (u8, u8) {
        let row = key / 16;
        let col = (key % 16) * 8;
        (row, col)
    }
}

impl Debug for Header {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let meta = self.meta.load_packed(Ordering::Relaxed);
        let mut iter = KeyIter63::default();
        header::Header::keys(
            self,
            Unbound::<()>::default(),
            Unbound::<()>::default(),
            &mut iter,
        );

        f.debug_struct("Header")
            .field("len", &iter.0.tail)
            .field("frozen", &meta.frozen())
            .field("last", &meta.last())
            .field("keys", &iter)
            .finish()
    }
}

#[derive(Copy, Clone, ribbit::Pack)]
#[ribbit(size = 16)]
struct Meta {
    last: u8,
    frozen: bool,
    len: u6,
}

impl Meta {
    const DEFAULT: ribbit::Packed<Self> = ribbit::Packed::<Self>::new(0, false, u6::new(0));
}

impl Default for MetaPacked {
    fn default() -> Self {
        Meta::DEFAULT
    }
}

impl From<Box<KeyIter63>> for node::KeyIter {
    #[inline]
    fn from(iter: Box<KeyIter63>) -> Self {
        node::KeyIter::new_47(iter)
    }
}

#[cfg(feature = "proptest")]
impl proptest::arbitrary::Arbitrary for Header {
    type Parameters = (u8, u8);
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((min, max): Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Strategy as _;
        assert!(min >= 15);
        assert!(max <= 47);

        use crate::raw;
        (
            proptest::collection::vec(u8::arbitrary(), min as usize..=max as usize)
                .prop_filter("unique keys", |keys| raw::is_unique(keys)),
            bool::arbitrary(),
        )
            .prop_map(|(keys, frozen)| {
                use crate::raw::node::header::Header as _;
                let mut header = Header::default();
                unsafe { header.initialize_unchecked(&keys) };
                if frozen {
                    header.freeze();
                }
                header
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    use proptest::arbitrary::any_with;

    crate::raw::node::header::tests::impl_suite!(any_with::<crate::raw::node::node_47::Header>((
        15, 47
    )));
}
