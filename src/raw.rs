//! Weakly typed implementation of adaptive radix tree.
//!
//! The purpose of this module is to re-use as much code as possible between the
//! sequential ([`crate::sequential::Map`]) and concurrent ([`crate::concurrent::Map`])
//! tree implementations, and between instantiations of these trees with different
//! value types.
//!
//! This module contains:
//! - Structural types ([`crate::raw::edge`], [`crate::raw::node`], [`crate::raw::key`])
//! - Traversal for point operations ([`crate::raw::cursor`])
//! - Iteration for scan operations ([`crate::raw::iter`])
//!
//! This module is "raw" with respect to:
//! - Safe memory reclamation ([`crate::concurrent::smr`])
//! - Mutable vs. immutable access
//! - Value types ([`crate::sequential::Value`], [`crate::concurrent::Value`])

pub(crate) mod cursor;
pub(crate) mod edge;
pub(crate) mod iter;
pub mod key;
pub(crate) mod map;
pub(crate) mod node;
pub(crate) mod set;
pub(crate) mod shard;

pub(crate) use cursor::Cursor;
pub(crate) use edge::Edge;
pub use key::Key;
pub(crate) use map::Map;
pub(crate) use set::Set;
pub(crate) use shard::Shard;

/// Structural modification operation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Smo {
    ReplaceNode,
    DeleteNode,
    CompressEdge,
}

impl Smo {
    #[inline]
    pub fn is_allocate(self) -> bool {
        matches!(self, Self::ReplaceNode)
    }
}

fn is_unique(keys: &[u8]) -> bool {
    let mut seen = [0u128; 2];
    for key in keys {
        let row = key / 128;
        let col = key % 128;
        let bit = 1 << col;
        if seen[row as usize] & bit > 0 {
            return false;
        }
        seen[row as usize] |= bit;
    }
    true
}

/// Compute the lowest byte index at which byte 0 appears in `array`.
/// If there is no zero, return 8.
///
/// https://graphics.stanford.edu/~seander/bithacks.html#ZeroInWord
/// https://richardstartin.github.io/posts/finding-bytes
/// https://orlp.net/blog/extracting-depositing-bits/
/// https://lemire.me/blog/2022/01/21/swar-explained-parsing-eight-digits/
/// https://lamport.azurewebsites.net/pubs/multiple-byte.pdf
#[inline]
fn find_zero(array: u64) -> u8 {
    let high_if_zero_or_ge_0x80 = array.wrapping_sub(0x0101_0101_0101_0101);
    let high_if_lt_0x80 = !array;
    let high_if_zero = high_if_zero_or_ge_0x80 & high_if_lt_0x80 & 0x8080_8080_8080_8080;
    (high_if_zero.trailing_zeros() >> 3) as u8
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "proptest")]
    proptest::proptest! {
        #[test]
        fn find_zero_correct(array: u64) {
            let expected = array
                .to_le_bytes()
                .into_iter()
                .position(|byte| byte == 0)
                .unwrap_or(8)
                as u8;

            let actual = super::find_zero(array);

            assert_eq!(
                actual,
                expected,
                "find_zero mismatch for array = {array:#x}",
            );
        }
    }
}
