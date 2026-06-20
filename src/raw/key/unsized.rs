use core::fmt;
use core::fmt::Debug;
use core::fmt::Display;
use core::hash::Hash;

pub mod boxed_slice;
pub mod slice;

pub use boxed_slice::BoxedSlice;
pub use slice::Slice;

pub unsafe trait Invariant:
    Debug + Default + Hash + Eq + Ord + Send + Sync + 'static
{
    type Error: core::error::Error;
    #[doc(hidden)]
    #[expect(private_bounds)]
    type Terminate: Terminate;

    fn validate(key: &[u8]) -> Result<(), Self::Error>;
}

#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct NonNull;

#[derive(Clone, Debug)]
pub struct NonNullError(usize);

impl core::error::Error for NonNullError {}

impl Display for NonNullError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Null byte at index ")?;
        Display::fmt(&self.0, f)
    }
}

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

#[derive(Copy, Clone, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Terminated<const TERMINATOR: u8>;

#[derive(Clone, Debug)]
pub enum TerminatedError {
    Missing,
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

// TODO: optimize?
#[inline]
fn read_u64(slice: &[u8]) -> u64 {
    if slice.len() >= 8 {
        return unsafe { slice.as_ptr().cast::<u64>().read_unaligned() };
    }

    // FIXME: try to avoid memcpy?
    // https://github.com/llvm/llvm-project/issues/87440
    // https://github.com/rust-lang/rust/issues/92993
    // https://github.com/rust-lang/rust/pull/37573
    let mut buffer = [0u8; 8];
    buffer[..slice.len()].copy_from_slice(slice);

    u64::from_le_bytes(buffer)
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
