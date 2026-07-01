//! Auxiliary types for use with [`SequentialSet`][crate::sequential::Set].

use core::borrow::Borrow as _;

use crate::raw;
use crate::raw::key;
use crate::sequential::Map;

/// Non-concurrent set.
#[repr(transparent)]
pub struct Set<K: key::Split> {
    map: Map<K, raw::Set>,
}

impl<K> Default for Set<K>
where
    K: key::Split,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K> Set<K>
where
    K: key::Split,
{
    /// Constructs a new empty set. Does not allocate.
    #[inline]
    pub const fn new() -> Self {
        Self { map: Map::new() }
    }

    /// Returns `true` if this set contains `key`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential;
    ///
    /// let mut set = sequential::Set::<[u8; 4]>::new();
    /// let key = [8, 2, 3, 255];
    /// assert!(set.insert(&key), "Key is not present");
    /// assert!(set.contains(&key), "Key is present");
    ///
    /// ```
    pub fn contains(&self, key: &K::Borrowed) -> bool {
        let (reader, byte) = K::split_last(key);

        self.map
            .get_raw(reader)
            .map(|value| unsafe { value.cast::<raw::Set>().as_ref() })
            .is_some_and(|set| set.contains(byte))
    }

    /// Insert `key` into this set.
    ///
    /// Returns `true` if successful, i.e., the key was not present and was newly inserted.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use arctic::sequential;
    ///
    /// let mut set = sequential::Set::<u128>::new();
    /// assert!(set.insert(5), "Key is not present");
    /// assert!(!set.insert(5), "Key is present");
    /// ```
    pub fn insert(&mut self, key: K::Insert<'_>) -> bool {
        let (reader, byte) = K::split_last(key.borrow());

        unsafe { self.map.entry_raw(reader) }
            .or_default()
            .insert_mut(byte)
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
        assert!(set.contains(&0xdeadbeef));
    }
}
