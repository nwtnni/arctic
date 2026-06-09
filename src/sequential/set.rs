use crate::raw;
use crate::raw::Key;
use crate::raw::key::Read as _;
use crate::sequential::Map;

#[repr(transparent)]
pub struct Set<K: Key> {
    map: Map<K, raw::Set>,
}

impl<K> Default for Set<K>
where
    K: Key,
{
    fn default() -> Self {
        Self {
            map: Default::default(),
        }
    }
}

impl<K> Set<K>
where
    K: Key,
{
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains_key(&self, key: &K::Borrowed) -> bool {
        let (reader, byte) = K::Read::from(key).split_last().expect("Key is non-empty");

        self.map
            .get_impl(reader)
            .is_some_and(|set| set.contains(byte))
    }

    pub fn insert(&mut self, key: K::Insert<'_>) -> bool {
        let (reader, byte) = K::insert_as_read(key)
            .split_last()
            .expect("Key is non-empty");

        self.map.entry_impl(reader).or_default().insert_mut(byte)
    }
}

#[cfg(test)]
mod tests {
    use crate::sequential::Set;

    #[test]
    fn smoke_insert() {
        let mut set = Set::<u64>::default();
        assert!(set.insert(5));
        assert!(set.insert(0xdeadbeef));
        assert!(!set.insert(5));
    }

    #[test]
    fn smoke_contains() {
        let mut set = Set::<u64>::default();
        assert!(set.insert(0xdeadbeef));
        assert!(set.contains_key(&0xdeadbeef));
    }
}
