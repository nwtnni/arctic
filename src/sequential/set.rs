//! Auxiliary types for use with [`crate::sequential::Set`].

use crate::raw;
use crate::raw::Key;
use crate::sequential::Map;

/// Non-concurrent set. (TODO: support iteration.)
#[repr(transparent)]
pub struct Set<K: Key> {
    map: Map<K, raw::Set>,
}

impl<K> Default for Set<K>
where
    K: Key,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K> Set<K>
where
    K: Key,
{
    /// Constructs a new empty set. Does not allocate.
    #[inline]
    pub const fn new() -> Self {
        Self { map: Map::new() }
    }

    pub fn contains_key(&self, _key: &K::Borrowed) -> bool {
        todo!()
        // let (reader, byte) = K::split_last(key);

        // self.map
        //     .get_impl(reader)
        //     .is_some_and(|set| set.contains(byte))
    }

    pub fn insert(&mut self, _key: K::Insert<'_>) -> bool {
        todo!()
        // let (reader, byte) = K::split_last(key.borrow());
        //
        // self.map.entry_impl(reader).or_default().insert_mut(byte)
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
