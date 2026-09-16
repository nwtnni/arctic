use core::fmt::Debug;
use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::raw::edge;
use crate::raw::edge::Meta as _;
use crate::raw::key::Len as _;
use crate::raw::key::Terminate;
use crate::sync::Convert;

type Byte = crate::raw::key::len::Byte<{ (1 << 13) - 1 }>;

// Layout:
// - 0..48: ptr
// - 48: value
// - 49: frozen
// - 50: terminate
// - 51..64 len
#[derive(Copy, Clone)]
pub struct Slice<T> {
    raw: *const u8,
    terminate: PhantomData<T>,
}

unsafe impl<T> Sync for Slice<T> {}
unsafe impl<T> Send for Slice<T> {}

static EMPTY: &[u8] = &[];

#[expect(private_bounds)]
impl<T: Terminate> Slice<T> {
    const MASK_PTR: usize = (1 << 48) - 1;
    const MASK_VALUE: usize = 1 << 48;
    const MASK_FROZEN: usize = 1 << 49;
    const MASK_TERMINATE: usize = 1 << 50;
    const SHIFT_LEN: usize = 51;

    #[inline]
    pub(crate) fn new(ptr: NonNull<u8>, len: Byte) -> Self {
        validate!(len < Byte::MAX);

        Self {
            raw: ptr.as_ptr().map_addr(|addr| {
                validate_eq!(addr & !Self::MASK_PTR, 0);
                addr | (len.bytes() << Self::SHIFT_LEN)
            }),
            terminate: PhantomData,
        }
    }

    #[inline]
    pub(crate) fn with_terminate(self, terminate: T) -> Self {
        Self {
            raw: self.raw.map_addr(|addr| {
                if terminate.get() {
                    addr | Self::MASK_TERMINATE
                } else {
                    addr & !Self::MASK_TERMINATE
                }
            }),
            terminate: PhantomData,
        }
    }

    #[inline]
    pub(crate) unsafe fn as_slice(&self) -> &[u8] {
        if self.raw.is_null() {
            return EMPTY;
        }

        let ptr = self.raw.map_addr(|addr| addr & Self::MASK_PTR);
        let len = self.len_slice();
        unsafe { core::slice::from_raw_parts(ptr, len.bytes()) }
    }

    #[inline]
    pub(crate) fn as_ptr(&self) -> *const u8 {
        self.raw.map_addr(|addr| addr & Self::MASK_PTR)
    }

    #[inline]
    pub(crate) fn len_slice(&self) -> Byte {
        unsafe { Byte::new_unchecked(self.raw.addr() >> Self::SHIFT_LEN) }
    }
}

impl<T: Terminate> Default for Slice<T> {
    fn default() -> Self {
        Self::NULL
    }
}

impl<T: Terminate> IntoIterator for Slice<T> {
    type Item = u8;
    type IntoIter = std::vec::IntoIter<u8>;
    fn into_iter(self) -> Self::IntoIter {
        let mut vec = unsafe { self.as_slice().to_vec() };
        if self.is_terminate() {
            vec.push(0);
        }
        vec.into_iter()
    }
}

impl<T: Terminate> edge::Meta for Slice<T> {
    const NULL: Self = Self {
        raw: core::ptr::null(),
        terminate: PhantomData,
    };

    type Len = Byte;

    #[inline]
    fn is_value(self) -> bool {
        self.raw.addr() & Self::MASK_VALUE > 0
    }

    #[inline]
    fn is_frozen(self) -> bool {
        self.raw.addr() & Self::MASK_FROZEN > 0
    }

    #[inline]
    fn is_terminate(self) -> bool {
        T::new(self.raw.addr() & Self::MASK_TERMINATE > 0).get()
    }

    #[inline]
    fn with_frozen(self, frozen: bool) -> Self {
        Self {
            raw: self.raw.map_addr(|addr| {
                if frozen {
                    addr | Self::MASK_FROZEN
                } else {
                    addr & !Self::MASK_FROZEN
                }
            }),
            terminate: PhantomData,
        }
    }

    #[inline]
    fn len(self) -> Self::Len {
        self.len_slice() + self.is_terminate().into()
    }

    #[inline]
    fn with_value(self, value: bool) -> Self {
        Self {
            raw: self.raw.map_addr(|addr| {
                if value {
                    addr | Self::MASK_VALUE
                } else {
                    addr & !Self::MASK_VALUE
                }
            }),
            terminate: PhantomData,
        }
    }

    fn try_compress(self, byte: u8, child: Self) -> Option<Self> {
        validate!(!self.is_frozen());
        validate!(!self.is_value());
        validate!(!self.is_terminate());

        let len_parent = self.len_slice();
        let len_byte = Byte::from(!T::is_terminator(byte));
        let len_child = child.len_slice();
        let len_total = Byte::try_add(len_parent, len_byte, len_child)?;

        // If we're compressing a terminator byte, then
        // the child must be an empty edge without a terminator
        validate!(
            len_byte == Byte::BYTE
                || !child.is_terminate() && len_child == Byte::ZERO && child.is_value()
        );

        Some(Slice {
            raw: unsafe {
                child
                    .raw
                    // NOTE: requires provenance of original slice
                    .byte_sub((len_parent + len_byte).bytes())
            }
            .map_addr(|addr| {
                let len = len_total.bytes() << Self::SHIFT_LEN;
                let terminate = if T::new(len_byte == Byte::ZERO).get() {
                    Self::MASK_TERMINATE
                } else {
                    0
                };

                addr & (Self::MASK_PTR
                    | Self::MASK_VALUE
                    | Self::MASK_FROZEN
                    | Self::MASK_TERMINATE)
                    | len
                    | terminate
            }),
            terminate: PhantomData,
        })
    }

    #[inline]
    fn try_expand(self, index: Self::Len) -> Option<(Self, u8, Self)> {
        if index >= self.len() {
            return None;
        }

        let len_parent = index;
        let len_slice = self.len_slice();
        let len_middle = (len_parent + Self::Len::BYTE).min(len_slice);
        validate!(len_parent <= len_slice);

        let parent = Slice {
            raw: self
                .raw
                .map_addr(|addr| addr & Self::MASK_PTR | (len_parent.bytes() << Self::SHIFT_LEN)),
            terminate: PhantomData,
        };

        let byte = unsafe { self.as_slice() }
            .get(len_parent.bytes())
            .copied()
            .unwrap_or(0);

        let child = Slice {
            raw: unsafe { self.raw.byte_add(len_middle.bytes()) }.map_addr(|addr| {
                let len = (len_slice - len_middle).bytes() << Self::SHIFT_LEN;
                let terminate = if T::new(len_parent < len_slice).get() {
                    usize::MAX
                } else {
                    !Self::MASK_TERMINATE
                };

                addr & (Self::MASK_PTR
                    | Self::MASK_VALUE
                    | Self::MASK_FROZEN
                    | Self::MASK_TERMINATE)
                    & terminate
                    | len
            }),
            terminate: PhantomData,
        };

        Some((parent, byte, child))
    }
}

impl<T: Terminate> Eq for Slice<T> {}

impl<T: Terminate> PartialEq for Slice<T> {
    fn eq(&self, other: &Self) -> bool {
        unsafe {
            self.as_slice() == other.as_slice() && self.is_terminate() == other.is_terminate()
        }
    }
}

impl<T: Terminate> Ord for Slice<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        unsafe {
            self.as_slice()
                .cmp(other.as_slice())
                .then_with(|| self.is_terminate().cmp(&other.is_terminate()))
        }
    }
}

impl<T: Terminate> PartialOrd for Slice<T> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: Terminate> Convert<u64> for Slice<T> {
    #[inline]
    fn into_raw(self) -> u64 {
        self.raw.expose_provenance() as u64
    }

    #[inline]
    unsafe fn from_raw_unchecked(raw: u64) -> Self {
        Self {
            raw: core::ptr::with_exposed_provenance(raw as usize),
            terminate: PhantomData,
        }
    }
}

// FIXME: remove after debugging,
// can easily cause a use-after-free otherwise
impl<T: Terminate> Debug for Slice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Le")
            .field("value", &self.is_value())
            .field("frozen", &self.is_frozen())
            .field("terminate", &self.is_terminate())
            .field("keys", &unsafe { self.as_slice() })
            .finish()
    }
}

#[cfg(feature = "proptest")]
impl<T: Terminate> proptest::arbitrary::Arbitrary for Slice<T> {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with((): Self::Parameters) -> Self::Strategy {
        use proptest::strategy::Strategy as _;

        use crate::raw::key::BoxedSlice;
        use crate::raw::key::NonNull;

        (
            bool::arbitrary(),
            bool::arbitrary(),
            BoxedSlice::<NonNull, [u8]>::arbitrary_with(((0..=32).into(), ())),
        )
            .prop_map(|(frozen, terminate, buffer)| {
                let len = buffer.as_bytes().len();
                let ptr = core::ptr::NonNull::from(
                    // HACK: need static lifetime
                    Box::leak(buffer.into_boxed_slice()),
                )
                .cast::<u8>();

                Self::new(ptr, Byte::new_clamped(len))
                    .with_value(terminate)
                    .with_frozen(frozen)
                    .with_terminate(T::new(terminate))
            })
            .boxed()
    }
}

#[cfg(test)]
mod tests {
    crate::raw::edge::tests::impl_suite!(crate::raw::edge::Slice<bool>);
}
