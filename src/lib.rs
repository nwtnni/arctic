//! This crate contains the primary implementation of
//! [Arctic: a practical lock-free adaptive radix tree](https://www.usenix.org/conference/osdi26/presentation/ni).
//! The main contribution is [`concurrent::Map`], which implements
//! a thread-safe map interface, and also supports
//! **[non-linearizable](https://en.wikipedia.org/wiki/Linearizability)**
//! iteration over key ranges and prefixes,
//! somewhat like a [`std::collections::BTreeMap`] wrapped in a
//! [`std::sync::Mutex`].
//!
//! # Why use this crate?
//!
//! As far as we know (corrections welcome!), out of all index data structures that (a) are [lock-free](https://en.wikipedia.org/wiki/Non-blocking_algorithm)
//! and (b) support scan operations, [`concurrent::Map`] provides the highest scalability and throughput.
//! In fact, under various conditions (integer keys, skewed requests, update-heavy),
//! we even out-perform data structures without properties (a) and/or (b).
//! Our benchmarking infrastructure is in [this repository](https://github.com/nwtnni/index-bench);
//! users are encouraged to measure performance on their own workloads.
//!
//! Briefly comparing against some alternative data structures:
//!
//! - Concurrent hash maps (e.g., [DashMap](https://github.com/xacrimon/dashmap), [papaya](https://github.com/ibraheemdev/papaya))
//!   have excellent performance, but do not support scan operations.
//! - Concurrent B+-trees (e.g., [scc::TreeIndex](https://codeberg.org/wvwwvwwv/scalable-concurrent-containers))
//!   have good performance, but are typically not lock-free.
//! - Concurrent skip lists (e.g., [crossbeam_skiplist](https://docs.rs/crossbeam-skiplist/latest/crossbeam_skiplist/))
//!   have poor performance on modern hardware (low cache locality),
//!   although there are lock-free implementations.
//!
//! # Limitations
//!
//! - 128-bit atomic support required for good performance (currently using [portable-atomic](https://github.com/taiki-e/portable-atomic) crate)
//! - SIMD acceleration is hand-written and currently restricted to AVX2
//! - Theoretically supports big-endian targets, but untested

macro_rules! const_assert_size_align {
    ($ty:ty, $size:expr, $align:expr) => {
        #[cfg(not(feature = "shuttle"))]
        const _: [(); $size] = [(); core::mem::size_of::<$ty>()];
        #[cfg(not(feature = "shuttle"))]
        const _: [(); $align] = [(); core::mem::align_of::<$ty>()];
    };
}

macro_rules! if_validate {
    ($if:expr $(, $else:expr)?) => {
        if cfg!(any(feature = "validate", debug_assertions, test)) {
            $if
        }
        $(else { $else })?
    };
}

macro_rules! validate {
    ($($tt:tt)*) => {
        if cfg!(any(feature = "validate", debug_assertions, test)) {
            assert!($($tt)*);
        }
    };
}

macro_rules! validate_eq {
    ($($tt:tt)*) => {
        if cfg!(any(feature = "validate", debug_assertions, test)) {
            assert_eq!($($tt)*);
        }
    };
}

macro_rules! simd {
    ($flag:expr, $avx2:expr, $fallback:expr $(, $fmt:expr)* $(,)?) => {{
        #[cfg(all(not(feature = $flag), target_feature = "avx2"))]
        {
            let avx2 = $avx2;
            validate_eq!(avx2, $fallback $(, $fmt)*);
            return $avx2;
        }

        #[allow(unreachable_code)]
        $fallback
    }};
}

pub mod concurrent;
pub(crate) mod raw;
pub mod sequential;
pub mod stat;
pub mod sync;

#[doc(inline)]
pub use raw::Key;
#[doc(inline)]
pub use raw::iter::Range;
#[doc(inline)]
pub use raw::key;

pub(crate) use sync::Atomic;

/// Key order for scan operations (e.g., [`concurrent::Shard::entries`]).
///
/// We take a compile-time argument rather than implementing [`core::iter::DoubleEndedIterator`]
/// because the latter would require maintaining two stacks at runtime (for the lower and
/// upper bound).
#[expect(private_bounds)]
pub trait Order: seal::Seal {}

/// Ascending [lexicographic order](https://en.wikipedia.org/wiki/Lexicographic_order).
///
/// Also see [`Order`].
pub struct Ascend;

/// Descending [lexicographic order](https://en.wikipedia.org/wiki/Lexicographic_order).
///
/// Also see [`Order`].
pub struct Descend;

impl Order for Ascend {}
impl Order for Descend {}

mod seal {
    //! [Seal](https://predr.ag/blog/definitive-guide-to-sealed-traits-in-rust/) for [`crate::Order`].

    pub(crate) trait Seal {
        const ASCEND: bool;
    }

    impl Seal for super::Ascend {
        const ASCEND: bool = true;
    }

    impl Seal for super::Descend {
        const ASCEND: bool = false;
    }
}

/// <https://users.rust-lang.org/t/compiler-hint-for-unlikely-likely-for-if-branches/62102/4>
#[inline]
#[cold]
pub(crate) fn cold() {}

#[cfg(test)]
mod tests {
    use crate::Ascend;
    use crate::Descend;
    use crate::concurrent::Map;
    use crate::key::BoxedSlice;
    use crate::key::BoxedStr;
    use crate::key::NonNull;
    use crate::key::Slice;
    use crate::key::Str;
    use crate::key::Terminated;
    use crate::raw::key::Read as _;

    #[test]
    fn smoke() {
        let map = Map::<BoxedStr<NonNull>, _>::default();
        map.upsert(unsafe { Slice::new_unchecked("abcd") }, 1u64);
        assert_eq!(
            map.get(unsafe { Slice::new_unchecked("abcd") })
                .as_deref()
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn smoke_u64_key() {
        let map = Map::<[u8; 8], _>::default();
        let key = 0xdeadbeefu64.to_be_bytes();
        map.upsert(&key, 1u64);
        assert_eq!(map.get(&key).as_deref().copied(), Some(1));
    }

    #[test]
    fn smoke_value_ref() {
        let values = [0, 1, 2, 3, 4, 5];
        let map = Map::<u64, &u64>::default();

        for (key, value) in values.iter().enumerate() {
            map.upsert(key as u64, value);
        }

        #[expect(clippy::needless_range_loop)]
        for key in 0..values.len() {
            let value = map.get(&(key as u64)).as_deref().copied().unwrap();
            assert!(core::ptr::eq(value, &values[key]));
        }
    }

    #[test]
    fn smoke_value_box() {
        let values = [0, 1, 2, 3, 4, 5];
        let map = Map::<u64, Box<u64>>::default();

        for (key, value) in values.iter().enumerate() {
            map.upsert(key as u64, Box::new(*value));
        }

        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for key in (0..values.len()).cycle().take(100_000) {
                        let value = map.get(&(key as u64)).as_deref().copied().unwrap();
                        assert_eq!(key, value as usize);
                    }
                });
            }
        });

        // TODO: multiple hazards?
        // let a = map.get(3);
        // let b = map.get(5);
        // assert_ne!(a.as_deref(), b.as_deref());

        for key in 0..values.len() {
            let value = map.get(&(key as u64)).as_deref().copied().unwrap();
            assert_eq!(key, value as usize);
        }
    }

    #[test]
    fn scan_value() {
        let map = Map::<u64, _>::default();
        let key = 1u64;
        map.upsert(key, 2u64);
        assert_eq!(
            map.range(1u64..=1u64)
                .entries::<Ascend>()
                .collect::<Vec<_>>(),
            vec![(1, 2)]
        );
    }

    #[test]
    fn scan_node3() {
        insert_all(0u64..3);
    }

    #[test]
    fn scan_node256() {
        insert_all(0u64..256);
    }

    #[test]
    fn scan_gap() {
        let map = insert_all((0u64..512).step_by(2));
        assert_eq!(
            map.range(256u64..=511u64)
                .entries::<Ascend>()
                .collect::<Vec<_>>(),
            (256..512)
                .step_by(2)
                .map(|key| (key, key / 2))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn node3_overwrite() {
        let mut map = Map::<u64, _>::default();

        for value in [1u64, 2, 3] {
            map.upsert(1, value);
            assert_eq!(map.get(&1).as_deref().copied(), Some(value));
        }

        assert_eq!(map.as_sequential().all().entries::<Ascend>().count(), 1);

        map.as_sequential()
            .all()
            .entries::<Ascend>()
            .for_each_internal(|(key, value)| {
                assert_eq!(key, 1);
                assert_eq!(*value, 3);
                core::ops::ControlFlow::Continue(())
            });
    }

    #[test]
    fn node3_reverse() {
        insert_all((0u16..3).rev());
    }

    #[test]
    fn node3_full() {
        insert_all(0u16..3);
    }

    #[test]
    fn node3_expand() {
        insert_all(0u16..4);
    }

    #[test]
    fn node15_full() {
        insert_all(0u16..15);
    }

    #[test]
    fn node15_expand() {
        insert_all(0u16..16);
    }

    #[test]
    fn node47_full() {
        insert_all(0u16..47);
    }

    #[test]
    fn node47_expand() {
        insert_all(0u16..61);
    }

    #[test]
    fn node256_full() {
        insert_all(0u16..=255);
    }

    #[test]
    fn range_reverse() {
        let map = Map::<u64, _>::default();

        for key in [5, 1, 4, 3, 2] {
            map.upsert(key, key);
            assert_eq!(map.get(&key).as_deref().copied(), Some(key));
        }

        assert_eq!(
            map.range(2..=4).entries::<Descend>().collect::<Vec<_>>(),
            vec![(4, 4), (3, 3), (2, 2)]
        );
    }

    #[test]
    fn split_edges() {
        let mut key = (1..100).collect::<Vec<_>>();
        insert_all(core::iter::from_fn(|| {
            if key.is_empty() {
                None
            } else {
                let mut next = key.clone();
                next.push(0);
                key.pop();
                let next = next.into_boxed_slice();
                Some(BoxedSlice::<Terminated<0>>::new(next).unwrap())
            }
        }));
    }

    #[test]
    fn one_long_key() {
        insert_all([BoxedStr::<NonNull>::new("a".repeat(1000)).unwrap()]);
    }

    #[test]
    fn short_key() {
        insert_all([BoxedStr::<NonNull>::new("\n".to_string()).unwrap()]);
    }

    #[test]
    fn two_long_keys() {
        insert_all([
            BoxedStr::<NonNull>::new("a".repeat(1000)).unwrap(),
            BoxedStr::<NonNull>::new("b".repeat(1000)).unwrap(),
        ]);
    }

    #[test]
    fn smoke_key_slice() {
        let keys = ["ad", "abc"];
        let map = crate::concurrent::Map::<&Str<NonNull>, u64>::new();
        map.insert(Str::new(keys[0]).unwrap(), 0)
            .unwrap_or_else(|(_, _)| panic!());
        map.insert(Str::new(keys[1]).unwrap(), 1)
            .unwrap_or_else(|(_, _)| panic!());

        let temp = "adabc";
        assert_eq!(
            map.get(Str::new(&temp[..2]).unwrap()).as_deref().copied(),
            Some(0)
        );
        assert_eq!(
            map.get(Str::new(&temp[2..]).unwrap()).as_deref().copied(),
            Some(1)
        );
    }

    #[test]
    fn key_slice_long_prefix() {
        let keys = (0..10)
            .map(|i| "a".repeat(100) + &i.to_string())
            .collect::<Vec<_>>();
        let map = crate::concurrent::Map::<&Slice<NonNull>, u64>::new();
        for (i, key) in keys.iter().enumerate() {
            map.insert(Slice::new(key.as_bytes()).unwrap(), i as u64)
                .unwrap();
        }
        for (i, key) in keys.iter().enumerate() {
            assert_eq!(
                map.get(Slice::new(key.as_bytes()).unwrap())
                    .as_deref()
                    .copied(),
                Some(i as u64)
            );
        }
    }

    fn insert_all<I, K>(iter: I) -> Map<K, u64>
    where
        I: IntoIterator<Item = K>,
        K: crate::concurrent::smr::hazard::Key + Clone + Ord + core::fmt::Debug,
    {
        let mut keys = iter
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u64))
            .collect::<Vec<_>>();

        let mut map = Map::default();

        for (key, value) in &keys {
            map.upsert(key.as_insert(), *value);
            assert_eq!(map.get(key.borrow()).as_deref().copied(), Some(*value));
        }

        for (key, value) in &keys {
            assert_eq!(map.get(key.borrow()).as_deref().copied(), Some(*value));
        }

        let mut iter = map.as_sequential().all().entries::<Ascend>();
        let mut count = 0;
        while iter.lend().is_some() {
            count += 1;
        }
        drop(iter);

        assert_eq!(count, keys.len());

        keys.sort_by(|(l, _), (r, _)| l.cmp(r));

        // Sequential iteration
        map.as_sequential()
            .all()
            .entries::<Ascend>()
            .zip(&keys)
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(*lv, *rv);
            });

        let Some(((first, _), (last, _))) = keys.first().zip(keys.last()) else {
            return map;
        };

        // Concurrent prefix scan, non-linearizable
        map.prefix(K::Read::from(first.borrow()).common_prefix(K::Read::from(last.borrow())))
            .entries::<Descend>()
            .zip(keys.iter().rev())
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });

        // Concurrent range scan, non-linearizable
        let mut i = 0;
        map.range(first.borrow()..=last.borrow())
            .entries::<Descend>()
            .zip(keys.iter().rev())
            .for_each(|((lk, lv), (rk, rv))| {
                i += 1;
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });
        assert_eq!(i, keys.len());

        map
    }
}
