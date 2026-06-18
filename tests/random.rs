use core::borrow::Borrow as _;
use std::sync::Barrier;

use arctic::raw::Key;
use rand::SeedableRng;
use rand::rngs::Xoshiro256PlusPlus;
use rand::seq::SliceRandom;

mod u64 {
    use arctic::raw::Key;

    use super::Workload;
    use super::test_map;

    #[test]
    fn many() {
        test_map(&U64, 16, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&U64, 2, 10_000_000, true);
    }

    #[test]
    fn one() {
        test_map(&U64, 1, 10_000_000, true);
    }

    struct U64;

    impl Workload for U64 {
        type Key<'k> = u64;

        type Value = u64;

        fn key(&self, index: usize) -> Self::Key<'_> {
            index as u64
        }

        fn value(&self, index: usize) -> Self::Value {
            index as u64
        }

        fn validate(
            &self,
            index: usize,
            key: &<Self::Key<'_> as Key>::Borrowed,
            value: &<Self::Value as arctic::concurrent::Value>::Borrowed,
        ) {
            assert_eq!(index as u64, *key);
            assert_eq!(index as u64, *value);
        }
    }
}

mod arced {
    use arctic::raw::Key;

    use super::Workload;
    use super::test_map;

    struct Arced;

    #[test]
    fn many() {
        test_map(&Arced, 16, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&Arced, 2, 10_000_000, false);
    }

    #[test]
    fn one() {
        test_map(&Arced, 1, 10_000_000, false);
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Entry {
        key: u32,
        value: u64,
    }

    impl Entry {
        fn new(index: usize) -> Self {
            Self {
                key: index as u32,
                value: index as u64 + 1,
            }
        }
    }

    impl Workload for Arced {
        type Key<'k> = u32;

        type Value = arctic::concurrent::value::Arc<Entry>;

        fn key(&self, index: usize) -> Self::Key<'_> {
            index as u32
        }

        fn value(&self, index: usize) -> Self::Value {
            arctic::sync::Arc::new(Entry::new(index)).into()
        }

        fn validate(
            &self,
            index: usize,
            key: &<Self::Key<'_> as Key>::Borrowed,
            value: &arctic::concurrent::value::ArcRef<Entry>,
        ) {
            assert_eq!(*key, index as u32);
            assert_eq!(**value, Entry::new(index));
        }
    }
}

mod boxed {
    use arctic::raw::Key;

    use super::Workload;
    use super::test_map;

    struct Boxed;

    #[test]
    fn many() {
        test_map(&Boxed, 16, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&Boxed, 2, 10_000_000, false);
    }

    #[test]
    fn one() {
        test_map(&Boxed, 1, 10_000_000, false);
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Entry {
        key: u32,
        value: u64,
    }

    impl Entry {
        fn new(index: usize) -> Self {
            Self {
                key: index as u32,
                value: index as u64 + 1,
            }
        }
    }

    impl Workload for Boxed {
        type Key<'k> = u32;

        type Value = Box<Entry>;

        fn key(&self, index: usize) -> Self::Key<'_> {
            index as u32
        }

        fn value(&self, index: usize) -> Self::Value {
            Box::new(Entry::new(index))
        }

        fn validate(&self, index: usize, key: &<Self::Key<'_> as Key>::Borrowed, value: &Entry) {
            assert_eq!(*key, index as u32);
            assert_eq!(*value, Entry::new(index));
        }
    }
}

mod vec {
    use arctic::NonPrefixVec;
    use arctic::raw::Key;
    use rand::SeedableRng as _;
    use rand::distr::Distribution as _;
    use rand::distr::SampleString;
    use rand::distr::StandardUniform;

    use super::Workload;
    use super::test_map;

    struct Bytes;

    #[test]
    fn many() {
        test_map(&Bytes, 16, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&Bytes, 2, 1_000_000, false);
    }

    #[test]
    fn one() {
        test_map(&Bytes, 1, 1_000_000, false);
    }

    impl Workload for Bytes {
        type Key<'k> = NonPrefixVec;
        type Value = u64;

        fn key(&self, index: usize) -> Self::Key<'_> {
            let mut rng = rand::rngs::Xoshiro256PlusPlus::seed_from_u64(index as u64);
            let len = rand::distr::Uniform::new_inclusive(16usize, 32usize)
                .unwrap()
                .sample(&mut rng);

            let mut buffer = StandardUniform.sample_string(&mut rng, len);
            buffer.push('\0');

            unsafe { NonPrefixVec::new_unchecked(buffer.into_bytes()) }
        }

        fn value(&self, index: usize) -> Self::Value {
            index as u64
        }

        fn validate(
            &self,
            index: usize,
            key: &<Self::Key<'_> as Key>::Borrowed,
            value: &<Self::Value as arctic::concurrent::Value>::Borrowed,
        ) {
            assert_eq!(key, self.key(index).as_non_prefix_slice());
            assert_eq!(*value, index as u64);
        }
    }
}

mod non_null_slice {
    use arctic::NonNullSlice;
    use arctic::raw::Key;
    use rand::SeedableRng as _;
    use rand::distr::Distribution as _;
    use rand::distr::SampleString as _;

    use super::Workload;
    use super::test_map;

    struct Slice(Vec<Vec<u8>>);

    #[test]
    fn many() {
        test_map(&Slice::new(10_000_000), 16, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&Slice::new(1_000_000), 2, 1_000_000, false);
    }

    #[test]
    fn one() {
        test_map(&Slice::new(1_000_000), 1, 1_000_000, false);
    }

    impl Slice {
        fn new(key_count: usize) -> Self {
            let mut outer = Vec::new();
            let mut rng = rand::rngs::Xoshiro256PlusPlus::seed_from_u64(key_count as u64);
            let dist_char =
                rand::distr::uniform::Uniform::<char>::new_inclusive(1 as char, char::MAX).unwrap();
            let dist_len = rand::distr::Uniform::new_inclusive(1usize, 8usize).unwrap();
            for _ in 0..key_count {
                let len = dist_len.sample(&mut rng);
                let inner = dist_char.sample_string(&mut rng, len);
                outer.push(inner.into_bytes());
            }
            Self(outer)
        }
    }

    impl Workload for Slice {
        type Key<'k> = &'k NonNullSlice;
        type Value = u64;

        fn key(&self, index: usize) -> Self::Key<'_> {
            unsafe { NonNullSlice::new_unchecked(self.0[index].as_slice()) }
        }

        fn value(&self, index: usize) -> Self::Value {
            index as u64
        }

        fn validate(
            &self,
            index: usize,
            key: &<Self::Key<'_> as Key>::Borrowed,
            value: &<Self::Value as arctic::concurrent::Value>::Borrowed,
        ) {
            assert!(core::ptr::eq(key, self.key(index)));
            assert_eq!(*value, index as u64);
        }
    }
}

mod array {
    use arctic::raw::Key;
    use rand::RngExt as _;
    use rand::SeedableRng as _;
    use rand::rngs::Xoshiro256PlusPlus;

    use super::Workload;
    use super::test_map;

    struct Array<const N: usize>;

    #[test]
    fn many() {
        test_map(&Array::<12>, 16, 10_000_000, false);
    }

    #[test]
    fn two() {
        test_map(&Array::<19>, 2, 1_000_000, false);
    }

    #[test]
    fn one() {
        test_map(&Array::<21>, 1, 1_000_000, false);
    }

    impl<const N: usize> Workload for Array<N> {
        type Key<'k> = [u8; N];
        type Value = u64;

        fn key(&self, index: usize) -> Self::Key<'_> {
            Xoshiro256PlusPlus::seed_from_u64(index as u64).random()
        }

        fn value(&self, index: usize) -> Self::Value {
            index as u64
        }

        fn validate(
            &self,
            index: usize,
            key: &<Self::Key<'_> as Key>::Borrowed,
            value: &<Self::Value as arctic::concurrent::Value>::Borrowed,
        ) {
            assert_eq!(*key, self.key(index));
            assert_eq!(*value, index as u64);
        }
    }
}

trait Workload: Sized + Sync {
    type Key<'k>: arctic::concurrent::smr::hazard::Key + Sync
    where
        Self: 'k;

    type Value: arctic::concurrent::Value + Send + Sync;

    fn key(&self, index: usize) -> Self::Key<'_>;

    fn value(&self, index: usize) -> Self::Value;

    fn validate<'k>(
        &'k self,
        index: usize,
        key: &<Self::Key<'k> as Key>::Borrowed,
        value: &<Self::Value as arctic::concurrent::Value>::Borrowed,
    );
}

fn test_map<'k, K: Workload>(key_set: &'k K, thread_count: usize, key_count: usize, shuffle: bool)
where
    for<'a> &'a <K::Key<'k> as Key>::Borrowed: Sync + core::fmt::Debug,
    <K::Value as arctic::concurrent::Value>::Borrowed: core::fmt::Debug,
    K::Key<'k>: Clone + Ord + core::fmt::Debug,
{
    assert_eq!(key_count % thread_count, 0);

    let barrier = &Barrier::new(thread_count);

    let mut items = (0..key_count)
        .map(|index| (index, key_set.key(index)))
        .collect::<Vec<_>>();
    items.sort_unstable_by(|(_, key_a), (_, key_b)| key_a.cmp(key_b));
    items.dedup_by(|(_, key_a), (_, key_b)| key_a == key_b);

    if shuffle {
        let mut rng = Xoshiro256PlusPlus::seed_from_u64((thread_count * key_count) as u64);
        items.shuffle(&mut rng);
    } else {
        items.sort_unstable_by_key(|(index, _)| *index);
    }

    let map = &arctic::concurrent::Map::<K::Key<'_>, _>::default();

    std::thread::scope(|scope| {
        for chunk in items.chunks(key_count / thread_count) {
            scope.spawn(move || {
                barrier.wait();

                for (index, key) in chunk {
                    let value = key_set.value(*index);
                    map.insert(key.as_insert(), value)
                        .ok()
                        .as_deref()
                        .unwrap_or_else(|| panic!("Key {:?} should not be present", key.borrow()));

                    if map.get(key.borrow()).is_none() {
                        panic!("failed to find {:x?}", key);
                    }
                }

                barrier.wait();

                for (index, key) in chunk.iter().take(chunk.len() / 2) {
                    // FIXME: change to recursive removal after figuring out retiring
                    let value = map
                        .remove(key.borrow())
                        .unwrap_or_else(|| panic!("failed to find {:x?}", key));
                    key_set.validate(*index, key.borrow(), &value);
                }

                barrier.wait();

                for (index, key) in chunk.iter().skip(chunk.len() / 2) {
                    let value = map.get(key.borrow());
                    key_set.validate(*index, key.borrow(), value.as_deref().unwrap());
                }
            });
        }
    });
}
