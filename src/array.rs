//! A fixed-length sequence whose length is part of its type.

use core::fmt;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};

use crate::vec::StableElement;

/// Exactly `N` values, stored one after another.
///
/// Unlike [`crate::vec::StableVec`] the value owns no allocation, so its size is
/// the size of its elements. The wrapper adds no padding of its own; the padding
/// an element type carries inside itself is still there, so a value of
/// `StableArray<(u8, u32), 4>` holds the same gaps a plain `[(u8, u32); 4]` does.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StableArray<T, const N: usize> {
    values: [T; N],
}

impl<T, const N: usize> fmt::Debug for StableArray<T, N> {
    // The values can be content a log must not repeat, so the debug form reports
    // the shape. `as_slice` prints the values when a caller asks for them.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableArray")
            .field("len", &N)
            .finish()
    }
}

impl<T, const N: usize> StableArray<T, N> {
    /// Wraps an array.
    pub const fn new(values: [T; N]) -> Self {
        Self { values }
    }

    /// The number of values.
    pub const fn len(&self) -> usize {
        N
    }

    /// Reports whether the array holds no values.
    pub const fn is_empty(&self) -> bool {
        N == 0
    }

    /// The values as a slice.
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }

    /// The values as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.values
    }

    /// Reads one value by index.
    pub fn get(&self, index: usize) -> Option<&T> {
        self.values.get(index)
    }

    /// Iterates over the values.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.values.iter()
    }

    /// Takes the array out.
    pub fn into_array(self) -> [T; N] {
        let this = ManuallyDrop::new(self);
        // SAFETY: `this` is not dropped, so the values move out exactly once.
        unsafe { core::ptr::read(&this.values) }
    }

    /// Checks that every element is a value its type defines.
    pub fn validate(&self) -> crate::error::AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        for value in &self.values {
            crate::boundary::check(value)?;
        }
        Ok(())
    }
}

impl<T: crate::zeroize::Wipe, const N: usize> crate::zeroize::Wipe for StableArray<T, N> {
    fn wipe_in_place(&mut self) {
        for value in &mut self.values {
            value.wipe_in_place();
        }
    }
}

impl<T, const N: usize> Deref for StableArray<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        &self.values
    }
}

impl<T, const N: usize> DerefMut for StableArray<T, N> {
    fn deref_mut(&mut self) -> &mut [T] {
        &mut self.values
    }
}

impl<T, const N: usize> From<[T; N]> for StableArray<T, N> {
    fn from(values: [T; N]) -> Self {
        Self::new(values)
    }
}

impl<T, const N: usize> From<StableArray<T, N>> for [T; N] {
    fn from(value: StableArray<T, N>) -> Self {
        value.into_array()
    }
}

impl<T, const N: usize> AsRef<[T]> for StableArray<T, N> {
    fn as_ref(&self) -> &[T] {
        &self.values
    }
}

impl<'a, T, const N: usize> IntoIterator for &'a StableArray<T, N> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

// SAFETY: the array is a `#[repr(C)]` struct holding `N` values of `T`, and
// dropping it drops each of them.
unsafe impl<T: StableElement, const N: usize> StableElement for StableArray<T, N> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

/// An array of bytes, which is the common shape of a fixed-size blob.
pub type StableBytes<const N: usize> = StableArray<u8, N>;

impl<T: PartialOrd, const N: usize> PartialOrd for StableArray<T, N> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.as_slice().partial_cmp(other.as_slice())
    }
}

impl<T: Ord, const N: usize> Ord for StableArray<T, N> {
    /// Compares the elements in order, as a slice would.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl<T: StableElement + crate::clone::TryClone, const N: usize> crate::clone::TryClone
    for StableArray<T, N>
{
    fn try_clone(&self) -> crate::error::AbiResult<Self> {
        let mut values: [core::mem::MaybeUninit<T>; N] =
            [const { core::mem::MaybeUninit::uninit() }; N];
        let mut written = 0;
        while written < N {
            match crate::clone::TryClone::try_clone(&self.as_slice()[written]) {
                Ok(value) => {
                    values[written] = core::mem::MaybeUninit::new(value);
                    written += 1;
                }
                Err(error) => {
                    // The elements already copied are released before the report:
                    // a failed copy must not leave half an array behind.
                    while written > 0 {
                        written -= 1;
                        // SAFETY: this slot was written, and it is dropped once.
                        unsafe { values[written].assume_init_drop() };
                    }
                    return Err(error);
                }
            }
        }
        // SAFETY: every slot was written above, and the two types have the same
        // layout because they have the same length and element type.
        Ok(Self::new(unsafe { core::mem::transmute_copy(&values) }))
    }
}

impl<T: fmt::Display, const N: usize> fmt::Display for StableArray<T, N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[")?;
        for (index, value) in self.values.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            value.fmt(formatter)?;
        }
        formatter.write_str("]")
    }
}
