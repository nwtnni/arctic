//! Support for dynamically sized keys.

use core::fmt;
use core::fmt::Debug;
use core::fmt::Display;
use core::hash::Hash;

pub(crate) mod boxed_slice;
pub(crate) mod slice;

/// An invariant of `[u8]` that is sufficient to guarantee the
/// prefix property (no key is a prefix of another key).
///
/// # Safety
///
/// Caller must ensure that if `validate` returns `Ok(())`,
/// then `key` satisfies the prefix property.
pub unsafe trait Invariant:
    Debug + Default + Hash + Eq + Ord + Send + Sync + 'static
{
    /// Validation error.
    type Error: core::error::Error;

    /// Implementation detail: some invariants append
    /// a logical terminator byte to the end of each key.
    #[expect(private_bounds)]
    type Terminate: Terminate;

    /// Returns `Ok(())` if and only if `key` satisfies this invariant.
    fn validate(key: &[u8]) -> Result<(), Self::Error>;
}

/// [`Invariant`] ZST indicating this key does not contain any null bytes.
///
/// Allows a null byte to be internally appended to each key,
/// which guarantees the prefix property and does not change
/// the lexicographic ordering of the key.
#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct NonNull;

/// Index at which a null byte was found within a key.
#[derive(Clone, Debug)]
pub struct NonNullError(usize);

impl core::error::Error for NonNullError {}

impl Display for NonNullError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Null byte at index ")?;
        Display::fmt(&self.0, f)
    }
}

// SAFETY: a non-null key that appends a null byte terminator
// satisfies the precondition property.
unsafe impl Invariant for NonNull {
    type Error = NonNullError;
    type Terminate = bool;

    fn validate(key: &[u8]) -> Result<(), Self::Error> {
        if let Some(index) = key.iter().position(|byte| *byte == 0) {
            return Err(NonNullError(index));
        }

        Ok(())
    }
}

/// [`Invariant`] ZST indicating this key contains exactly one
/// `TERMINATOR` byte at the end of the key.
///
/// Implies the prefix property.
#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Terminated<const TERMINATOR: u8>;

/// Information about why a key does not satisfy the [`Terminated`] invariant.
#[derive(Clone, Debug)]
pub enum TerminatedError {
    /// Terminator byte is missing from key.
    Missing,
    /// Terminator byte was found before end of key.
    Internal(usize),
}

impl core::error::Error for TerminatedError {}

impl Display for TerminatedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "Missing terminator byte"),
            Self::Internal(index) => {
                write!(f, "Internal terminator byte at index {index}")
            }
        }
    }
}

// SAFETY: a key that ends in a terminator satisfies the precondition property.
unsafe impl<const TERMINATOR: u8> Invariant for Terminated<TERMINATOR> {
    type Error = TerminatedError;
    type Terminate = ();

    fn validate(key: &[u8]) -> Result<(), Self::Error> {
        match key.iter().position(|byte| *byte == TERMINATOR) {
            None => Err(TerminatedError::Missing),
            Some(index) if index < key.len() - 1 => Err(TerminatedError::Internal(index)),
            Some(_) => Ok(()),
        }
    }
}

// TODO: optimize?
#[inline]
fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    core::iter::zip(left, right)
        .position(|(l, r)| l != r)
        .unwrap_or_else(|| left.len().min(right.len()))
}

#[inline]
fn read_u64(slice: &[u8]) -> u64 {
    if slice.len() >= 8 {
        cfg_select! {
            // Avoid memcpy (just x86 for now):
            any(target_arch = "x86", target_arch = "x86_64") => {
                // - https://rust-lang.github.io/rfcs/1725-unaligned-access.html#detailed-design
                // - https://github.com/llvm/llvm-project/issues/87440
                // - https://github.com/rust-lang/rust/issues/92993
                // - https://github.com/rust-lang/rust/pull/37573
                // - https://lemire.me/blog/2012/05/31/data-alignment-for-speed-myth-or-reality/
                let buffer: u64;
                unsafe {
                    core::arch::asm! {
                        "mov {}, [{}]",
                        out(reg) buffer,
                        in(reg) slice.as_ptr().cast::<u64>(),
                        options(pure, readonly, preserves_flags, nostack)
                    }
                }
                buffer
            }
            _ => unsafe {
                slice.as_ptr().cast::<u64>().read_unaligned()
            }
        }
    } else {
        let mut buffer = [0u8; 8];
        buffer[..slice.len()].copy_from_slice(slice);
        u64::from_le_bytes(buffer)
    }
}

mod seal {
    pub trait Seal {}
}

pub(crate) trait Terminate:
    Debug + Default + Eq + ribbit::Pack<Packed = Self> + Send + Sync + 'static + seal::Seal
{
    const FALSE: Self;
    const TRUE: Self;

    fn new(terminate: bool) -> Self;
    fn get(self) -> bool;

    fn try_compress(byte: u8) -> usize;

    fn trim(slice: &[u8]) -> &[u8];
}

impl seal::Seal for () {}
impl Terminate for () {
    const FALSE: Self = ();
    const TRUE: Self = ();

    #[inline]
    fn new(_: bool) -> Self {}

    #[inline]
    fn get(self) -> bool {
        false
    }

    #[inline]
    fn try_compress(_: u8) -> usize {
        1
    }

    #[inline]
    fn trim(slice: &[u8]) -> &[u8] {
        slice
    }
}

impl seal::Seal for bool {}
impl Terminate for bool {
    const FALSE: Self = false;
    const TRUE: Self = true;

    #[inline]
    fn new(terminate: bool) -> Self {
        terminate
    }

    #[inline]
    fn get(self) -> bool {
        self
    }

    #[inline]
    fn try_compress(byte: u8) -> usize {
        (byte > 0) as usize
    }

    #[inline]
    fn trim(slice: &[u8]) -> &[u8] {
        let (last, slice) = slice.split_last().expect("Non-empty");
        validate_eq!(*last, 0);
        slice
    }
}
