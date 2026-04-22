macro_rules! const_assert_size_align {
    ($ty:ty, $size:expr, $align:expr) => {
        const _: [(); $size] = [(); core::mem::size_of::<$ty>()];
        const _: [(); $align] = [(); core::mem::align_of::<$ty>()];
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
pub mod raw;
pub mod sequential;
pub mod stat;

pub use concurrent::Key;
pub use concurrent::Value;

#[expect(private_bounds)]
pub trait Order: seal::Seal {}

pub struct Ascend;
pub struct Descend;

impl Order for Ascend {}
impl Order for Descend {}

mod seal {
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

/// https://users.rust-lang.org/t/compiler-hint-for-unlikely-likely-for-if-branches/62102/4
#[inline]
#[cold]
pub(crate) fn cold() {}

#[cfg(test)]
mod tests {
    use crate::Ascend;
    use crate::Descend;
    use crate::concurrent::Map;
    use crate::raw::key::Read as _;
    use crate::sequential;

    // https://users.rust-lang.org/t/testing-if-a-type-is-implementing-an-auto-trait/90871/6
    #[test]
    const fn assert_not_sync() {
        #[allow(dead_code)]
        trait AmbiguousIfSync<T> {
            const ASSERT_NOT_SYNC: () = ();
        }

        impl<T: ?Sized> AmbiguousIfSync<((), ())> for T {}
        impl<T: ?Sized + Sync> AmbiguousIfSync<()> for T {}

        const _: () = <sequential::Map<u64, u64>>::ASSERT_NOT_SYNC;
    }

    #[test]
    fn smoke() {
        let map = Map::<Vec<u8>, _>::default();
        map.upsert(b"abcd", 1u64);
        assert_eq!(map.get(b"abcd").as_deref().copied(), Some(1));
    }

    #[test]
    fn smoke_u64_key() {
        let map = Map::<Vec<u8>, _>::default();
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
            let value = map.get(key as u64).as_deref().copied().unwrap();
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
                        let value = map.get(key as u64).as_deref().copied().unwrap();
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
            let value = map.get(key as u64).as_deref().copied().unwrap();
            assert_eq!(key, value as usize);
        }
    }

    #[test]
    fn scan_value() {
        let map = Map::<u64, _>::default();
        let key = 1u64;
        map.upsert(key, 2u64);
        let range = map.range(1u64..=1u64).unwrap();
        assert_eq!(range.entries::<Ascend>().collect::<Vec<_>>(), vec![(1, 2)]);
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
        let range = map.range(256u64..=511u64).unwrap();
        assert_eq!(
            range.entries::<Ascend>().collect::<Vec<_>>(),
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
            assert_eq!(map.get(1).as_deref().copied(), Some(value));
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
            assert_eq!(map.get(key).as_deref().copied(), Some(key));
        }
        let range = map.range(2..=4).unwrap();

        assert_eq!(
            range.entries::<Descend>().collect::<Vec<_>>(),
            vec![(4, 4), (3, 3), (2, 2)]
        );
    }

    #[test]
    fn split_edges() {
        let mut key = (0..100).collect::<Vec<_>>();
        insert_all(core::iter::from_fn(|| {
            if key.is_empty() {
                None
            } else {
                let mut next = key.clone();
                next.push(0);
                key.pop();
                Some(next)
            }
        }));
    }

    #[test]
    fn one_long_key() {
        insert_all(["a".repeat(1000)]);
    }

    #[test]
    fn two_long_keys() {
        insert_all(["a".repeat(1000), "b".repeat(1000)]);
    }

    #[test]
    fn regression_u128() {
        const fn key(low: i64) -> u128 {
            const FLIP: u64 = 1u64.rotate_right(1);
            let high = (-1i64) as u64 ^ FLIP;
            ((high as u128) << 64) | (((low as u64) ^ FLIP) as u128)
        }

        let map = insert_all((0..10i64).map(key));

        let prefix = map.range(key(5)..=key(i64::MAX)).unwrap();

        let values = prefix.values::<Ascend>().collect::<Vec<_>>();
        assert_eq!(values, (5..10).collect::<Vec<u64>>());
    }

    #[test]
    fn regression_insert() {
        let map = crate::concurrent::Map::<u64, u64>::new();
        map.insert(0u64, 0u64).unwrap();
        map.insert(0u64, 1u64).unwrap_err();
    }

    #[test]
    fn smoke_key_slice() {
        let keys = [b"ad".as_slice(), b"abc".as_slice()];
        let map = crate::concurrent::Map::<&[u8], u64>::new();
        map.insert(keys[0], 0).unwrap_or_else(|(_, _)| panic!());
        map.insert(keys[1], 1).unwrap_or_else(|(_, _)| panic!());
        let temp = b"adabc".to_vec();
        assert_eq!(map.get(&temp[..2]).as_deref().copied(), Some(0));
        assert_eq!(map.get(&temp[2..]).as_deref().copied(), Some(1));
    }

    fn insert_all<I, K>(iter: I) -> Map<K, u64>
    where
        I: IntoIterator<Item = K>,
        K: crate::Key + Clone + Ord + core::fmt::Debug,
    {
        let mut keys = iter
            .into_iter()
            .enumerate()
            .map(|(index, key)| (key, index as u64))
            .collect::<Vec<_>>();

        let mut map = Map::default();

        for (key, value) in &keys {
            map.upsert(key.borrow_insert(), *value);
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

        // Concurrent prefix iteration, non-linearizable
        let prefix = map
            .prefix(K::Read::from(first.borrow()).common_prefix(K::Read::from(last.borrow())))
            .unwrap();
        prefix
            .entries::<Descend>()
            .zip(keys.iter().rev())
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });
        drop(prefix);

        // Concurrent range iteration, non-linearizable
        let range = map.range(first.borrow()..=last.borrow()).unwrap();
        range
            .entries::<Ascend>()
            .zip(&keys)
            .for_each(|((lk, lv), (rk, rv))| {
                assert_eq!(lk, *rk);
                assert_eq!(lv, *rv);
            });
        drop(range);

        map
    }
}
