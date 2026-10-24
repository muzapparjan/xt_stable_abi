//! Owned and borrowed sequences with a declared layout.
//!
//! [`StableVec`] owns its storage and carries the allocator that produced it;
//! [`StableSlice`] borrows storage that the holder keeps alive. Both store a
//! length as `u64`, so a side compiled for a different pointer width reads the
//! same field.
//!
//! Only types that implement [`StableElement`] may be stored, which is how the
//! crate keeps `repr(Rust)` types (and `usize`) from silently entering the
//! boundary.

use core::fmt;
use core::marker::PhantomData;
use core::mem::{align_of, size_of, MaybeUninit};
use core::ptr::{self, NonNull};

use crate::heap::vec::Vec;

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};

/// A type whose layout and ownership rule are part of the ABI.
///
/// # Safety
///
/// An implementor promises that the type has the same size, alignment and field
/// offsets in every build that exchanges it, and that
/// [`StableElement::drop_in_place`] releases exactly the resources the value
/// owns.
pub unsafe trait StableElement: Sized {
    /// Drops the value behind `ptr` without releasing the storage.
    ///
    /// # Safety
    ///
    /// `ptr` must point at an initialised value of `Self` that is not used
    /// again afterwards.
    unsafe fn drop_in_place(ptr: *mut Self);
}

macro_rules! impl_stable_element_for_scalars {
    ($($type:ty),* $(,)?) => {
        $(
            // SAFETY: the type is a scalar with a fixed layout and owns nothing.
            unsafe impl StableElement for $type {
                unsafe fn drop_in_place(ptr: *mut Self) {
                    // SAFETY: the caller guarantees an initialised value.
                    unsafe { ptr::drop_in_place(ptr) }
                }
            }
        )*
    };
}

impl_stable_element_for_scalars!(
    u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64
);

// SAFETY: the unit type is zero-sized, has no fields and owns nothing. It is
// useful as the element type of a vector whose length is the value being
// exchanged.
unsafe impl StableElement for () {
    unsafe fn drop_in_place(_ptr: *mut Self) {}
}

/// A borrowed sequence of [`StableElement`] values.
///
/// The handle owns nothing: it names storage that lives for `'a`. Borrowing the
/// storage through a safe constructor ties the handle's lifetime to the owner,
/// so a handle cannot outlive the data it points at. A handle rebuilt from raw
/// parts carries no such tie and must be used only while the storage is alive.
#[repr(C)]
pub struct StableSlice<'a, T: StableElement> {
    pointer: *const T,
    length: u64,
    element: PhantomData<&'a T>,
}

impl<'a, T: StableElement> StableSlice<'a, T> {
    /// The empty slice.
    pub const fn empty() -> Self {
        Self {
            pointer: NonNull::<T>::dangling().as_ptr(),
            length: 0,
            element: PhantomData,
        }
    }

    /// Borrows `values` for as long as they live.
    pub const fn from_slice(values: &'a [T]) -> Self {
        Self {
            pointer: values.as_ptr(),
            length: values.len() as u64,
            element: PhantomData,
        }
    }

    /// Rebuilds a handle from its parts.
    ///
    /// # Safety
    ///
    /// `pointer` must point at `length` initialised values of `T` that stay
    /// alive and unchanged for `'a`.
    pub const unsafe fn from_raw_parts(pointer: *const T, length: u64) -> Self {
        Self {
            pointer,
            length,
            element: PhantomData,
        }
    }

    /// Rebuilds a handle from parts that arrived from another build, checking
    /// that the declared length fits the buffer those parts came from.
    ///
    /// `available` is the number of bytes the receiver actually holds. A length
    /// that would read past it is rejected instead of trusted, and so is a
    /// pointer that is null or not aligned for `T`, a null or misaligned
    /// pointer: `slice::from_raw_parts` requires an aligned, non-null address
    /// even for an empty slice.
    ///
    /// # Safety
    ///
    /// `pointer` must point at `available` valid bytes that stay alive and
    /// unchanged for `'a`.
    pub unsafe fn from_raw_parts_checked(
        pointer: *const T,
        length: u64,
        available: u64,
    ) -> AbiResult<Self> {
        let handle = Self {
            pointer,
            length,
            element: PhantomData,
        };
        handle.validate_header()?;
        let required = match length.checked_mul(size_of::<T>() as u64) {
            Some(required) => required,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        if required > available {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        Ok(handle)
    }

    /// Checks the header without reading the elements.
    ///
    /// A pointer that is null or not aligned for `T`, a length whose byte count
    /// is not addressable, or a byte count past `isize::MAX` is reported instead
    /// of being used to build a slice.
    ///
    /// The length is also required to fit a `usize`, which matters for a
    /// zero-sized element: the byte count of such a slice is zero whatever the
    /// length says, so a length past `usize::MAX` would otherwise pass and be
    /// truncated when the elements are indexed.
    pub fn validate_header(&self) -> AbiResult<()> {
        if self.pointer.is_null() || !(self.pointer as usize).is_multiple_of(align_of::<T>()) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        if self.length > isize::MAX as u64 {
            return Err(AbiError::new(AbiStatus::CapacityOverflow));
        }
        match self.length.checked_mul(size_of::<T>() as u64) {
            Some(bytes) if bytes <= isize::MAX as u64 => Ok(()),
            Some(_) => Err(AbiError::new(AbiStatus::CapacityOverflow)),
            None => Err(AbiError::new(AbiStatus::CapacityOverflow)),
        }
    }

    /// The address of the first element.
    pub const fn as_ptr(&self) -> *const T {
        self.pointer
    }

    /// The number of elements.
    pub const fn len(&self) -> u64 {
        self.length
    }

    /// Reports whether the slice is empty.
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Reads one element by index.
    pub fn get(&self, index: u64) -> Option<&T> {
        if index >= self.length {
            return None;
        }
        // SAFETY: the index is inside the declared length, and the constructor
        // required the storage to be valid.
        Some(unsafe { &*self.pointer.add(index as usize) })
    }

    /// The borrowed values, for the lifetime of this handle.
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: the constructor required the storage to be valid for the
        // life of the handle.
        unsafe { core::slice::from_raw_parts(self.pointer, self.length as usize) }
    }

    /// Checks the header and every element the length covers.
    ///
    /// A slice of zero-sized elements is refused a length, for the reason the
    /// vector's own `validate` gives: the length is the whole value such a
    /// container carries, and nothing bounds it.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        self.validate_header()?;
        if size_of::<T>() == 0 && self.length != 0 {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        for value in self.as_slice() {
            crate::boundary::check(value)?;
        }
        Ok(())
    }

    /// Iterates over the elements.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    /// The borrowed values.
    ///
    /// The returned lifetime is not tied to `self`: the handle is usually a
    /// temporary copy while the storage lives elsewhere.
    ///
    /// # Safety
    ///
    /// The storage must still be alive and must not be mutated for `'b`.
    pub unsafe fn as_slice_unbounded<'b>(&self) -> &'b [T] {
        // SAFETY: the caller guarantees the storage is alive.
        unsafe { core::slice::from_raw_parts(self.pointer, self.length as usize) }
    }

    /// Clones the elements into an owned vector.
    ///
    /// The copy goes through [`Clone`], one element at a time. An element type
    /// that is [`Copy`] has a bulk form that copies the block once:
    /// [`StableSlice::to_copy_vec`].
    pub fn to_vec(&self) -> AbiResult<StableVec<T>>
    where
        T: Clone,
    {
        self.to_vec_with_allocator(AbiAllocator::system())
    }

    /// Clones the elements into an owned vector that uses `allocator`.
    pub fn to_vec_with_allocator(&self, allocator: AbiAllocator) -> AbiResult<StableVec<T>>
    where
        T: Clone,
    {
        StableVec::from_slice_with_allocator(allocator, self.as_slice())
    }

    /// Copies the elements into an owned vector with one move of the block.
    ///
    /// A type that is [`Copy`] can be copied as bytes, so this is one allocation
    /// and one bulk copy rather than one call per element; a type that is only
    /// [`Clone`] has no such form and uses [`StableSlice::to_vec`].
    pub fn to_copy_vec(&self) -> AbiResult<StableVec<T>>
    where
        T: Copy,
    {
        self.to_copy_vec_with_allocator(AbiAllocator::system())
    }

    /// Copies the elements into an owned vector that uses `allocator`.
    pub fn to_copy_vec_with_allocator(&self, allocator: AbiAllocator) -> AbiResult<StableVec<T>>
    where
        T: Copy,
    {
        StableVec::from_copy_slice_with_allocator(allocator, self.as_slice())
    }
}

impl<'a, T: StableElement> Clone for StableSlice<'a, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T: StableElement> Copy for StableSlice<'a, T> {}

// SAFETY: the handle is a shared view of `[T]`; sharing it is safe exactly when
// `T` can be shared, and sending it is safe exactly when `T` can be shared.
unsafe impl<'a, T: Sync + StableElement> Send for StableSlice<'a, T> {}
unsafe impl<'a, T: Sync + StableElement> Sync for StableSlice<'a, T> {}

// SAFETY: the handle has the layout of a pointer, a length and a marker, and
// owns nothing.
unsafe impl<'a, T: StableElement> StableElement for StableSlice<'a, T> {
    unsafe fn drop_in_place(_ptr: *mut Self) {}
}

impl<'a, T: StableElement> fmt::Debug for StableSlice<'a, T> {
    // The elements can be content a log must not repeat, so the debug form
    // reports the shape. `as_slice` prints the values when a caller asks for
    // them.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableSlice")
            .field("len", &self.length)
            .finish()
    }
}

impl<'a, T: StableElement + PartialEq> PartialEq for StableSlice<'a, T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<'a, T: StableElement + Eq> Eq for StableSlice<'a, T> {}

/// A borrowed, writable sequence of [`StableElement`] values.
///
/// This is the writable counterpart of [`StableSlice`]: the holder of the
/// storage keeps ownership, and the other side is allowed to overwrite the
/// elements in place. It exists so a library can write into a buffer the caller
/// already owns, without allocating a result and without copying it back.
///
/// The handle owns nothing. Borrowing the storage through
/// [`StableSliceMut::from_mut_slice`] ties the handle to the borrow, so it cannot
/// outlive the data; a handle rebuilt from raw parts carries no such tie and must
/// be used only while the storage is alive and not otherwise reachable.
#[repr(C)]
pub struct StableSliceMut<'a, T: StableElement> {
    pointer: *mut T,
    length: u64,
    element: PhantomData<&'a mut T>,
}

impl<'a, T: StableElement> StableSliceMut<'a, T> {
    /// Borrows `values` mutably for as long as they live.
    pub const fn from_mut_slice(values: &'a mut [T]) -> Self {
        Self {
            pointer: values.as_mut_ptr(),
            length: values.len() as u64,
            element: PhantomData,
        }
    }

    /// Rebuilds a handle from its parts.
    ///
    /// # Safety
    ///
    /// `pointer` must point at `length` initialised values of `T` that stay
    /// alive and are not reachable through another reference for `'a`.
    pub const unsafe fn from_raw_parts(pointer: *mut T, length: u64) -> Self {
        Self {
            pointer,
            length,
            element: PhantomData,
        }
    }

    /// Rebuilds a handle from parts that arrived from another build, checking
    /// that the declared length fits the buffer those parts came from.
    ///
    /// # Safety
    ///
    /// `pointer` must point at `available` valid, writable bytes that stay alive
    /// and are not otherwise reachable for `'a`.
    pub unsafe fn from_raw_parts_checked(
        pointer: *mut T,
        length: u64,
        available: u64,
    ) -> AbiResult<Self> {
        let handle = Self {
            pointer,
            length,
            element: PhantomData,
        };
        handle.validate_header()?;
        let required = match length.checked_mul(size_of::<T>() as u64) {
            Some(required) => required,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        if required > available {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        Ok(handle)
    }

    /// Checks the header without reading the elements.
    ///
    /// The checks are the ones a writable view from another build has to pass:
    /// the address must be non-null and aligned, and the byte count of the
    /// region must be addressable.
    pub fn validate_header(&self) -> AbiResult<()> {
        if self.pointer.is_null() || !(self.pointer as usize).is_multiple_of(align_of::<T>()) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        if self.length > isize::MAX as u64 {
            return Err(AbiError::new(AbiStatus::CapacityOverflow));
        }
        match self.length.checked_mul(size_of::<T>() as u64) {
            Some(bytes) if bytes <= isize::MAX as u64 => Ok(()),
            Some(_) => Err(AbiError::new(AbiStatus::CapacityOverflow)),
            None => Err(AbiError::new(AbiStatus::CapacityOverflow)),
        }
    }

    /// The address of the first element.
    pub const fn as_ptr(&self) -> *mut T {
        self.pointer
    }

    /// The number of elements.
    pub const fn len(&self) -> u64 {
        self.length
    }

    /// Reports whether the slice is empty.
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Reads one element by index.
    pub fn get(&self, index: u64) -> Option<&T> {
        if index >= self.length {
            return None;
        }
        // SAFETY: the index is inside the declared length, and the constructor
        // required the storage to be valid.
        Some(unsafe { &*self.pointer.add(index as usize) })
    }

    /// Reads one element by index, for writing.
    pub fn get_mut(&mut self, index: u64) -> Option<&mut T> {
        if index >= self.length {
            return None;
        }
        // SAFETY: the index is inside the declared length, and this handle is the
        // one view that may write it.
        Some(unsafe { &mut *self.pointer.add(index as usize) })
    }

    /// Overwrites one element by index, reporting an index past the end.
    ///
    /// This is the whole point of the type: it changes the caller's storage in
    /// place and allocates nothing.
    pub fn set(&mut self, index: u64, value: T) -> AbiResult<()> {
        match self.get_mut(index) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(AbiError::new(AbiStatus::InvalidArgument)),
        }
    }

    /// The borrowed values, for the lifetime of this handle.
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: the constructor required the storage to be valid for the life
        // of the handle.
        unsafe { core::slice::from_raw_parts(self.pointer, self.length as usize) }
    }

    /// The borrowed values, for writing.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: the constructor required exclusive access for the life of the
        // handle.
        unsafe { core::slice::from_raw_parts_mut(self.pointer, self.length as usize) }
    }

    /// Checks the header and every element the length covers.
    ///
    /// A slice of zero-sized elements is refused a length, for the reason the
    /// vector's own `validate` gives: the length is the whole value such a
    /// container carries, and nothing bounds it.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        self.validate_header()?;
        if size_of::<T>() == 0 && self.length != 0 {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        for value in self.as_slice() {
            crate::boundary::check(value)?;
        }
        Ok(())
    }

    /// Iterates over the elements.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    /// Iterates over the elements, for writing.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.as_mut_slice().iter_mut()
    }

    /// Writes `values` over the first `values.len()` elements, reporting a
    /// target that is too small.
    pub fn copy_from_slice(&mut self, values: &[T]) -> AbiResult<()>
    where
        T: Clone,
    {
        if values.len() as u64 > self.length {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        for (slot, value) in self.as_mut_slice().iter_mut().zip(values) {
            *slot = value.clone();
        }
        Ok(())
    }
}

// SAFETY: the handle is a mutable view of `[T]`; sending it is safe exactly when
// `T` can be sent.
unsafe impl<'a, T: Send + StableElement> Send for StableSliceMut<'a, T> {}
unsafe impl<'a, T: Sync + StableElement> Sync for StableSliceMut<'a, T> {}

// SAFETY: the handle has the layout of a pointer, a length and a marker, and
// owns nothing.
unsafe impl<'a, T: StableElement> StableElement for StableSliceMut<'a, T> {
    unsafe fn drop_in_place(_ptr: *mut Self) {}
}

impl<'a, T: StableElement> fmt::Debug for StableSliceMut<'a, T> {
    // The elements can be content a log must not repeat, so the debug form
    // reports the shape.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableSliceMut")
            .field("len", &self.length)
            .finish()
    }
}

/// An owned, growable sequence of [`StableElement`] values.
///
/// The vector records the allocator that produced its storage, so dropping it
/// after it has crossed the boundary releases the block on the side that
/// allocated it.
#[repr(C)]
pub struct StableVec<T: StableElement> {
    pointer: *mut T,
    length: u64,
    capacity: u64,
    allocator: AbiAllocator,
    element: PhantomData<T>,
}

/// Finishes a retain pass on both exits.
///
/// The pass moves kept elements down and releases rejected ones in one walk. If
/// the predicate panics, the length has to be brought back in step with what
/// the walk actually did; without that, the vector would describe slots that
/// were already released and drop them a second time.
struct RetainFinish<'a, T: StableElement> {
    vector: &'a mut StableVec<T>,
    processed: u64,
    removed: u64,
    old_length: u64,
}

impl<T: StableElement> Drop for RetainFinish<'_, T> {
    fn drop(&mut self) {
        let write = self.processed - self.removed;
        let remaining = self.old_length - self.processed;
        if remaining > 0 {
            // SAFETY: both ranges are inside the old length, and the destination
            // starts before the source, so a forward copy is the right
            // direction.
            unsafe {
                ptr::copy(
                    self.vector.pointer.add(self.processed as usize),
                    self.vector.slot(write),
                    remaining as usize,
                )
            };
        }
        self.vector.length = self.old_length - self.removed;
    }
}

/// Sorts `length` slots in place, using `scratch` for one merged run at a time.
///
/// The merge is stable: of two equal elements the one from the earlier run is
/// written first, which is what keeps the order a caller sorted by a second key.
/// A merge writes the merged run into `scratch` and then copies it back, so no
/// element is ever released here and a panic from the comparison leaves the
/// block holding every element exactly once.
///
/// # Safety
///
/// Both pointers must name `length` slots of `T`, disjoint from each other, with
/// every slot in `pointer` initialised.
unsafe fn merge_sort<T: StableElement, F: FnMut(&T, &T) -> core::cmp::Ordering>(
    pointer: *mut T,
    length: usize,
    scratch: *mut T,
    compare: &mut F,
) {
    if length <= 1 {
        return;
    }
    let middle = length / 2;
    // SAFETY: the two halves are inside the range the caller described, and they
    // share `scratch`, which is used only by the merge at this level.
    unsafe {
        merge_sort(pointer, middle, scratch, compare);
        merge_sort(pointer.add(middle), length - middle, scratch, compare);
    }

    let mut left = 0;
    let mut right = middle;
    let mut written = 0;
    // SAFETY: every index below is inside `length`, and `scratch` has room for
    // the whole merged run.
    unsafe {
        while left < middle && right < length {
            // An equal pair takes the left element first: the runs were sorted
            // with the same comparison, so taking the earlier run keeps the
            // original order.
            let take_left = compare(&*pointer.add(left), &*pointer.add(right))
                != core::cmp::Ordering::Greater;
            let source = if take_left { left } else { right };
            ptr::copy_nonoverlapping(pointer.add(source), scratch.add(written), 1);
            if take_left {
                left += 1;
            } else {
                right += 1;
            }
            written += 1;
        }
        while left < middle {
            ptr::copy_nonoverlapping(pointer.add(left), scratch.add(written), 1);
            left += 1;
            written += 1;
        }
        while right < length {
            ptr::copy_nonoverlapping(pointer.add(right), scratch.add(written), 1);
            right += 1;
            written += 1;
        }
        // The merged run replaces both, so every slot in `pointer` is covered and
        // no element exists twice afterwards.
        ptr::copy_nonoverlapping(scratch, pointer, length);
    }
}

/// Moves the element at `root` down a max-heap of `slice[..end]` until both of
/// its children are smaller than it.
///
/// The heap is the block itself: the children of `root` are `2 * root + 1` and
/// `2 * root + 2`. Only swaps are used, so a comparison that panics leaves every
/// element in the block exactly once.
fn sift_down<T, F: FnMut(&T, &T) -> core::cmp::Ordering>(
    slice: &mut [T],
    mut root: usize,
    end: usize,
    compare: &mut F,
) {
    loop {
        let mut child = 2 * root + 1;
        if child >= end {
            return;
        }
        if child + 1 < end && compare(&slice[child], &slice[child + 1]).is_lt() {
            child += 1;
        }
        if !compare(&slice[root], &slice[child]).is_lt() {
            return;
        }
        slice.swap(root, child);
        root = child;
    }
}

impl<T: StableElement> StableVec<T> {
    /// An empty vector that uses the process allocator.
    pub const fn new() -> Self {
        Self::new_with_allocator(AbiAllocator::system())
    }

    /// An empty vector that uses `allocator`.
    pub const fn new_with_allocator(allocator: AbiAllocator) -> Self {
        Self {
            pointer: NonNull::<T>::dangling().as_ptr(),
            length: 0,
            capacity: 0,
            allocator,
            element: PhantomData,
        }
    }

    /// The allocator that owns the storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.allocator
    }

    /// The number of elements.
    pub const fn len(&self) -> u64 {
        self.length
    }

    /// Reports whether the vector holds no elements.
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// The number of elements the storage can hold without reallocating.
    ///
    /// A vector that has never reserved reports `0`, including one built by
    /// `with_capacity(0)`. For a zero-sized element type, a vector that has
    /// reserved reports `isize::MAX`, because no block is involved and no
    /// further reservation is needed.
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }

    /// The address of the storage block.
    ///
    /// The block holds `capacity` slots, of which the first `length` are
    /// initialised elements. Operations that cover the whole block, such as
    /// overwriting it before release, need this address rather than the one
    /// [`StableVec::as_mut_slice`] returns: a slice borrows the initialised
    /// elements only, so writing past them through its pointer would reach
    /// memory the borrow does not describe.
    pub const fn storage_ptr(&self) -> *mut T {
        self.pointer
    }

    /// The elements as a slice.
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: the first `length` slots are initialised.
        unsafe { core::slice::from_raw_parts(self.pointer, self.length as usize) }
    }

    /// The elements as a mutable slice.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: the first `length` slots are initialised and `&mut self`
        // grants exclusive access.
        unsafe { core::slice::from_raw_parts_mut(self.pointer, self.length as usize) }
    }

    /// Keeps only the elements for which `keep` returns true.
    ///
    /// The predicate sees the elements in their original order. If it panics,
    /// the vector is left holding every element the predicate had not decided
    /// on yet: the elements it rejected stay released, the ones it accepted stay
    /// moved down, and nothing is released twice.
    pub fn retain<F: FnMut(&T) -> bool>(&mut self, mut keep: F) {
        let old_length = self.length;
        let mut finish = RetainFinish {
            vector: self,
            processed: 0,
            removed: 0,
            old_length,
        };
        while finish.processed < old_length {
            let read = finish.processed;
            // SAFETY: both indices are below the old length.
            let slot = unsafe { finish.vector.pointer.add(read as usize) };
            // SAFETY: the slot holds an initialised value.
            let retained = keep(unsafe { &*slot });
            if retained {
                let write = read - finish.removed;
                if write != read {
                    // SAFETY: the destination is an earlier slot whose value was
                    // already moved forward or released.
                    unsafe { ptr::copy_nonoverlapping(slot, finish.vector.slot(write), 1) };
                }
            } else {
                // The slot stops being part of the vector before its value is
                // released: a destructor that panics must not leave the slot
                // covered by the length, or the guard would copy a value that is
                // already being destroyed and the vector would release it a
                // second time.
                finish.removed += 1;
                finish.processed += 1;
                // SAFETY: the element is being removed, so it is released here
                // and never recorded as part of the length again.
                unsafe { T::drop_in_place(slot) };
                continue;
            }
            finish.processed += 1;
        }
    }

    /// Removes the elements in `start..end`, returning them in a new vector.
    ///
    /// This is the owned form of a drain: the removed elements keep their order
    /// and stay alive in the returned vector.
    pub fn remove_range(&mut self, start: u64, end: u64) -> AbiResult<Self> {
        if start > end || end > self.length {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let count = end - start;
        let mut removed = Self::with_capacity_and_allocator(self.allocator, count)?;
        // SAFETY: the destination has room for `count` elements, and the two
        // ranges do not overlap.
        unsafe {
            ptr::copy_nonoverlapping(
                self.pointer.add(start as usize),
                removed.pointer,
                count as usize,
            );
        }
        removed.length = count;
        let tail = self.length - end;
        // SAFETY: the source range starts at `end` and the destination at
        // `start`, which is no later.
        unsafe { ptr::copy(self.pointer.add(end as usize), self.pointer.add(start as usize), tail as usize) };
        self.length -= count;
        Ok(removed)
    }

    /// Appends every value the iterator yields.
    ///
    /// # Panics
    ///
    /// Panics when the storage cannot grow; use [`StableVec::try_extend`] for a
    /// fallible version.
    pub fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        if let Err(error) = self.try_extend(items) {
            panic!("failed to extend a stable vector: {error}");
        }
    }

    /// Appends every value the iterator yields.
    ///
    /// A failure leaves the vector as it was: the values already appended are
    /// released before the error is returned, so the caller never has to work out
    /// how far the loop got.
    ///
    /// The iterator's lower bound is reserved before the first value is moved, so
    /// an iterator that knows how many values it holds costs one allocation
    /// rather than one per doubling step.
    pub fn try_extend<I: IntoIterator<Item = T>>(&mut self, items: I) -> AbiResult<()> {
        let original = self.length;
        let items = items.into_iter();
        let (lower, _) = items.size_hint();
        if lower > 0 {
            self.try_reserve(lower as u64)?;
        }
        for value in items {
            if let Err(error) = self.try_push(value) {
                self.truncate(original);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Sorts the elements with the default ordering.
    ///
    /// The temporary storage a stable merge needs is taken from the allocator
    /// this vector carries, so the value stays inside the allocation strategy it
    /// was built with. Use [`StableVec::try_sort`] for a fallible version.
    ///
    /// # Panics
    ///
    /// Panics when the temporary storage cannot be allocated.
    pub fn sort(&mut self)
    where
        T: Ord,
    {
        match self.try_sort() {
            Ok(()) => {}
            Err(error) => panic!("failed to sort a stable vector: {error}"),
        }
    }

    /// Sorts the elements with the default ordering, reporting a failure to
    /// allocate the temporary storage.
    pub fn try_sort(&mut self) -> AbiResult<()>
    where
        T: Ord,
    {
        self.try_sort_by(|left, right| left.cmp(right))
    }

    /// Sorts the elements with `compare`.
    ///
    /// The order is stable: elements that compare equal keep the order they had,
    /// which is what lets a second key decide a tie.
    ///
    /// # Panics
    ///
    /// Panics when the temporary storage cannot be allocated.
    pub fn sort_by<F: FnMut(&T, &T) -> core::cmp::Ordering>(&mut self, compare: F) {
        match self.try_sort_by(compare) {
            Ok(()) => {}
            Err(error) => panic!("failed to sort a stable vector: {error}"),
        }
    }

    /// Sorts the elements with `compare`, reporting a failure to allocate the
    /// temporary storage.
    ///
    /// The temporary block comes from [`StableVec::allocator`], not from the
    /// process allocator, so a vector built on an arena keeps its promise that
    /// every block it takes is released with the arena.
    ///
    /// The sort is a stable merge. If `compare` panics, the vector still holds
    /// every element it held before the call; the order is unspecified.
    pub fn try_sort_by<F: FnMut(&T, &T) -> core::cmp::Ordering>(
        &mut self,
        mut compare: F,
    ) -> AbiResult<()> {
        let length = self.length;
        if length <= 1 || size_of::<T>() == 0 {
            return Ok(());
        }
        let bytes = match (length as usize).checked_mul(size_of::<T>()) {
            Some(bytes) => bytes as u64,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        let scratch = self
            .allocator
            .try_allocate(bytes, align_of::<T>() as u64)?
            .cast::<T>();

        /// Releases the temporary block on every exit, including the one where
        /// the comparison panics.
        struct Scratch<T: StableElement> {
            pointer: *mut T,
            bytes: u64,
            allocator: AbiAllocator,
            element: PhantomData<T>,
        }

        impl<T: StableElement> Drop for Scratch<T> {
            fn drop(&mut self) {
                // SAFETY: the block came from this allocator with this layout.
                unsafe {
                    self.allocator.deallocate(
                        self.pointer.cast(),
                        self.bytes,
                        align_of::<T>() as u64,
                    )
                };
            }
        }

        let scratch = Scratch {
            pointer: scratch,
            bytes,
            allocator: self.allocator,
            element: PhantomData,
        };
        // SAFETY: both blocks hold `length` slots, and the comparison reads only
        // elements that are initialised.
        unsafe {
            merge_sort(
                self.pointer,
                length as usize,
                scratch.pointer,
                &mut compare,
            )
        };
        Ok(())
    }

    /// Sorts the elements with `compare`, giving up the order of elements that
    /// compare equal.
    ///
    /// Nothing is allocated: the sort rearranges the elements inside the block
    /// the vector already owns, so it has no fallible form and no temporary
    /// block to give back. Use it when a tie carries no meaning and the `2n`
    /// bytes a stable sort needs are too much; [`StableVec::sort_by`] keeps
    /// equal elements in the order they had.
    ///
    /// # Panics
    ///
    /// Panics when `compare` panics. The vector then holds every element it held
    /// before the call, in an order this call does not describe.
    pub fn sort_unstable_by<F: FnMut(&T, &T) -> core::cmp::Ordering>(&mut self, mut compare: F) {
        let length = self.length as usize;
        if length <= 1 {
            return;
        }
        let slice = self.as_mut_slice();
        // Build a heap over the whole block, then take the largest element out
        // of the front one at a time and let the heap shrink from the back.
        let mut root = length / 2;
        while root > 0 {
            root -= 1;
            sift_down(slice, root, length, &mut compare);
        }
        let mut end = length;
        while end > 1 {
            end -= 1;
            slice.swap(0, end);
            sift_down(slice, 0, end, &mut compare);
        }
    }

    /// Sorts the elements, giving up the order of elements that compare equal.
    ///
    /// # Panics
    ///
    /// Panics when a comparison panics; see [`StableVec::sort_unstable_by`].
    pub fn sort_unstable(&mut self)
    where
        T: Ord,
    {
        self.sort_unstable_by(|left, right| left.cmp(right));
    }

    /// The index of `value` in a sorted vector, or where it would be inserted.
    pub fn binary_search(&self, value: &T) -> core::result::Result<usize, usize>
    where
        T: Ord,
    {
        self.as_slice().binary_search(value)
    }

    /// The storage as a borrowed handle.
    pub fn as_stable_slice(&self) -> StableSlice<'_, T> {
        StableSlice::from_slice(self.as_slice())
    }

    /// Reads one element by index.
    pub fn get(&self, index: u64) -> Option<&T> {
        if index >= self.length || index as usize as u64 != index {
            return None;
        }
        // SAFETY: the index is inside the length, and for a zero-sized element
        // every in-range index names the same aligned address.
        Some(unsafe { &*self.slot(index) })
    }

    /// Reads one element by index, mutably.
    pub fn get_mut(&mut self, index: u64) -> Option<&mut T> {
        if index >= self.length || index as usize as u64 != index {
            return None;
        }
        // SAFETY: the index is inside the length and `&mut self` grants
        // exclusive access.
        Some(unsafe { &mut *self.slot(index) })
    }

    /// The first element.
    pub fn first(&self) -> Option<&T> {
        self.as_slice().first()
    }

    /// The last element.
    pub fn last(&self) -> Option<&T> {
        self.as_slice().last()
    }

    /// The first element, for writing.
    pub fn first_mut(&mut self) -> Option<&mut T> {
        self.as_mut_slice().first_mut()
    }

    /// The last element, for writing.
    pub fn last_mut(&mut self) -> Option<&mut T> {
        self.as_mut_slice().last_mut()
    }

    /// Exchanges two elements, reporting an index past the end.
    pub fn swap(&mut self, left: u64, right: u64) -> AbiResult<()> {
        if left >= self.length || right >= self.length {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        if left != right {
            // SAFETY: both indices are inside the length and name different
            // slots.
            unsafe {
                ptr::swap_nonoverlapping(self.slot(left), self.slot(right), 1);
            }
        }
        Ok(())
    }

    /// Reverses the order of the elements.
    pub fn reverse(&mut self) {
        self.as_mut_slice().reverse();
    }

    /// Appends `value` `count` times.
    ///
    /// # Panics
    ///
    /// Panics when the storage cannot grow, or when copying `value` fails; use
    /// [`StableVec::try_resize`] for a fallible version.
    pub fn resize(&mut self, new_length: u64, value: T)
    where
        T: crate::clone::TryClone,
    {
        if let Err(error) = self.try_resize(new_length, value) {
            panic!("failed to resize a stable vector: {error}");
        }
    }

    /// Appends `value` until the length is `new_length`, or releases the last
    /// elements when it is already longer.
    ///
    /// The copy is made by [`TryClone`](crate::clone::TryClone), so a value whose
    /// copy cannot be allocated is reported rather than raised. A failure leaves
    /// the vector as it was: the copies already made are released before the
    /// error is returned.
    pub fn try_resize(&mut self, new_length: u64, value: T) -> AbiResult<()>
    where
        T: crate::clone::TryClone,
    {
        if new_length <= self.length {
            self.truncate(new_length);
            return Ok(());
        }
        self.try_reserve(new_length - self.length)?;
        let original = self.length;
        while self.length < new_length {
            match crate::clone::TryClone::try_clone(&value) {
                Ok(copy) => {
                    if let Err(error) = self.try_push(copy) {
                        self.truncate(original);
                        return Err(error);
                    }
                }
                Err(error) => {
                    self.truncate(original);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Splits the vector at `at`: the value keeps the elements before it and the
    /// answer holds the rest, with its own block.
    ///
    /// # Panics
    ///
    /// Panics when the second block cannot be allocated; use
    /// [`StableVec::try_split_off`] for a fallible version.
    pub fn split_off(&mut self, at: u64) -> Self {
        match self.try_split_off(at) {
            Ok(tail) => tail,
            Err(error) => panic!("failed to split a stable vector: {error}"),
        }
    }

    /// Splits the vector at `at`, reporting allocation failure or an index past
    /// the length.
    pub fn try_split_off(&mut self, at: u64) -> AbiResult<Self> {
        self.remove_range(at, self.length)
    }

    /// Appends every element of `other`, leaving it empty.
    ///
    /// # Panics
    ///
    /// Panics when the storage cannot grow; use [`StableVec::try_append`] for a
    /// fallible version.
    pub fn append(&mut self, other: &mut Self) {
        if let Err(error) = self.try_append(other) {
            panic!("failed to append to a stable vector: {error}");
        }
    }

    /// Appends every element of `other`, leaving it empty.
    ///
    /// The elements keep the order they had in `other`. On failure the elements
    /// that were moved are in `self` and `other` describes only the ones that
    /// were not, so dropping either releases exactly what it owns.
    pub fn try_append(&mut self, other: &mut Self) -> AbiResult<()> {
        self.try_reserve(other.length)?;
        let count = other.length;
        let mut index = 0;
        while index < count {
            // SAFETY: the index is inside `other`'s length, and the length is
            // cleared below, so no slot is released twice.
            let value = unsafe { ptr::read(other.slot(index)) };
            // The capacity for every element was reserved above, so this cannot
            // fail. A failure would leave the rest of `other` unreleased rather
            // than read twice.
            if let Err(error) = self.try_push(value) {
                // The elements after this one were not moved. Shifting them to
                // the front leaves `other` describing exactly the values it
                // still owns, which is what its drop releases.
                let left = (count - index - 1) as usize;
                if left > 0 && size_of::<T>() != 0 {
                    // SAFETY: both ranges are inside `other`'s block, and the
                    // source begins one past the element read above.
                    unsafe {
                        ptr::copy(other.slot(index + 1), other.slot(0), left)
                    };
                }
                other.length = left as u64;
                return Err(error);
            }
            index += 1;
        }
        other.length = 0;
        Ok(())
    }

    /// Iterates over the elements.
    pub fn iter(&self) -> core::slice::Iter<'_, T> {
        self.as_slice().iter()
    }

    /// Iterates over the elements, mutably.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, T> {
        self.as_mut_slice().iter_mut()
    }
}

impl<T: StableElement> StableVec<T> {
    /// The address of the slot at `index`.
    fn slot(&self, index: u64) -> *mut T {
        if size_of::<T>() == 0 {
            self.pointer
        } else {
            // SAFETY: callers only pass indices below `capacity`.
            unsafe { self.pointer.add(index as usize) }
        }
    }

    /// An empty vector with room for at least `capacity` elements.
    ///
    /// A request below the vector's minimum block is rounded up to that block;
    /// later growth doubles the capacity in hand.
    pub fn with_capacity(capacity: u64) -> AbiResult<Self> {
        Self::with_capacity_and_allocator(AbiAllocator::system(), capacity)
    }

    /// An empty vector with room for `capacity` elements, using `allocator`.
    pub fn with_capacity_and_allocator(
        allocator: AbiAllocator,
        capacity: u64,
    ) -> AbiResult<Self> {
        let mut vector = Self::new_with_allocator(allocator);
        vector.try_reserve(capacity)?;
        Ok(vector)
    }

    /// Clones `values` into a new vector, reporting allocation failure.
    ///
    /// The elements are copied one at a time with [`Clone`]. A type that is
    /// [`Copy`] has a bulk form that copies the block once, which is
    /// [`StableVec::from_copy_slice`].
    pub fn from_slice(values: &[T]) -> AbiResult<Self>
    where
        T: Clone,
    {
        Self::from_slice_with_allocator(AbiAllocator::system(), values)
    }

    /// Clones `values` into a new vector that uses `allocator`.
    ///
    /// The elements are copied one at a time with [`Clone`]; a type that is
    /// [`Copy`] uses [`StableVec::from_copy_slice_with_allocator`], which copies
    /// the block once.
    pub fn from_slice_with_allocator(
        allocator: AbiAllocator,
        values: &[T],
    ) -> AbiResult<Self>
    where
        T: Clone,
    {
        let mut vector = Self::with_capacity_and_allocator(allocator, values.len() as u64)?;
        for value in values {
            // The capacity was reserved above, so this cannot fail.
            vector.try_push(value.clone())?;
        }
        Ok(vector)
    }

    /// Copies `values` into a new vector, reporting a copy that fails as well as
    /// an allocation that does.
    ///
    /// This is the constructor a caller that has an error channel of its own
    /// uses: every copy is made by [`TryClone`](crate::clone::TryClone), so
    /// nothing in it raises.
    pub fn try_from_slice(values: &[T]) -> AbiResult<Self>
    where
        T: crate::clone::TryClone,
    {
        Self::try_from_slice_with_allocator(AbiAllocator::system(), values)
    }

    /// Copies `values` into a new vector that uses `allocator`, reporting a copy
    /// that fails as well as an allocation that does.
    pub fn try_from_slice_with_allocator(
        allocator: AbiAllocator,
        values: &[T],
    ) -> AbiResult<Self>
    where
        T: crate::clone::TryClone,
    {
        let mut vector = Self::with_capacity_and_allocator(allocator, values.len() as u64)?;
        for value in values {
            // The capacity was reserved above, so this cannot fail.
            vector.try_push(crate::clone::TryClone::try_clone(value)?)?;
        }
        Ok(vector)
    }

    /// Makes room for `additional` more elements.
    ///
    /// # Panics
    ///
    /// Panics when the allocation fails; use [`StableVec::try_reserve`] for a
    /// fallible version.
    pub fn reserve(&mut self, additional: u64) {
        if let Err(error) = self.try_reserve(additional) {
            panic!("failed to grow a stable vector: {error}");
        }
    }

    /// Makes room for `additional` more elements, reporting failure.
    pub fn try_reserve(&mut self, additional: u64) -> AbiResult<()> {
        let required = match self.length.checked_add(additional) {
            Some(required) => required,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        if required <= self.capacity {
            return Ok(());
        }
        if required > isize::MAX as u64 {
            return Err(AbiError::new(AbiStatus::CapacityOverflow));
        }
        if size_of::<T>() == 0 {
            self.capacity = isize::MAX as u64;
            return Ok(());
        }

        // A vector that has never allocated reserves at least
        // `MINIMUM_CAPACITY`, so a request below that number yields a larger
        // block; later growth doubles from the capacity in hand. A doubling that
        // would land past the largest addressable count is not an answer: the
        // request itself is addressable, so the block is grown to exactly what it
        // needs instead of failing on a size nobody asked for.
        let mut new_capacity = if self.capacity == 0 {
            required.max(MINIMUM_CAPACITY)
        } else {
            self.capacity.max(MINIMUM_CAPACITY)
        };
        while new_capacity < required {
            new_capacity = match new_capacity.checked_mul(2) {
                Some(doubled) if doubled <= isize::MAX as u64 => doubled,
                _ => required,
            };
        }
        self.grow_to(new_capacity)
    }

    /// Makes room for `additional` more elements without reserving more than
    /// that, reporting failure.
    ///
    /// The block is grown to the number of slots the vector asked for; an
    /// allocator is free to answer with more, and this side records the number it
    /// asked for, because that is the only size the allocator handle reports
    /// back.
    pub fn try_reserve_exact(&mut self, additional: u64) -> AbiResult<()> {
        let required = match self.length.checked_add(additional) {
            Some(required) => required,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        if required <= self.capacity {
            return Ok(());
        }
        if required > isize::MAX as u64 {
            return Err(AbiError::new(AbiStatus::CapacityOverflow));
        }
        if size_of::<T>() == 0 {
            self.capacity = isize::MAX as u64;
            return Ok(());
        }
        self.grow_to(required)
    }

    /// Makes room for `additional` more elements without reserving more than
    /// that.
    ///
    /// # Panics
    ///
    /// Panics when the allocation fails; use [`StableVec::try_reserve_exact`]
    /// for a fallible version.
    pub fn reserve_exact(&mut self, additional: u64) {
        if let Err(error) = self.try_reserve_exact(additional) {
            panic!("failed to grow a stable vector: {error}");
        }
    }

    /// Reduces the capacity to `min_capacity`, keeping at least the length.
    ///
    /// The space is given back through the allocator this vector carries: the
    /// block is reallocated, and released when the capacity it asks for is zero.
    /// A capacity that is already small enough is left alone, which is also what
    /// a request below the length produces: the length is the smallest capacity a
    /// vector can have.
    ///
    /// # Errors
    ///
    /// Reports a failure to reallocate; the vector keeps its storage and its
    /// contents on every error.
    pub fn try_shrink_to(&mut self, min_capacity: u64) -> AbiResult<()> {
        let target = min_capacity.max(self.length);
        if target >= self.capacity || size_of::<T>() == 0 {
            return Ok(());
        }
        let element_size = size_of::<T>() as u64;
        let alignment = align_of::<T>() as u64;
        let old_bytes = match self.capacity.checked_mul(element_size) {
            Some(bytes) => bytes,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        let new_bytes = match target.checked_mul(element_size) {
            Some(bytes) => bytes,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        if target == 0 {
            // An allocator answers a request for no bytes with a null pointer or
            // with a block it does not own, so the space is released instead of
            // reallocated: an empty vector owns nothing.
            // SAFETY: the block came from this allocator with `old_bytes` and
            // this alignment.
            unsafe {
                self.allocator
                    .deallocate(self.pointer.cast(), old_bytes, alignment)
            };
            self.pointer = NonNull::<T>::dangling().as_ptr();
            self.capacity = 0;
            return Ok(());
        }
        // SAFETY: the block came from this allocator with `old_bytes` and this
        // alignment, and the contents are copied to the block it answers with
        // before the old one is released.
        let pointer = unsafe {
            self.allocator
                .reallocate(self.pointer.cast(), old_bytes, alignment, new_bytes)
        };
        if pointer.is_null() {
            return Err(AbiError::new(AbiStatus::OutOfMemory));
        }
        self.pointer = pointer.cast::<T>();
        self.capacity = target;
        Ok(())
    }

    /// Reduces the capacity to the length, giving back the spare space.
    ///
    /// The block is reallocated through the allocator this vector carries, so the
    /// space returns to whoever provided it.
    ///
    /// # Errors
    ///
    /// Reports a failure to reallocate; the vector keeps its storage and its
    /// contents on every error.
    pub fn try_shrink_to_fit(&mut self) -> AbiResult<()> {
        self.try_shrink_to(self.length)
    }

    /// Reduces the capacity to `min_capacity`, keeping at least the length.
    ///
    /// # Panics
    ///
    /// Panics when the reallocation fails; use [`StableVec::try_shrink_to`] for a
    /// fallible version.
    pub fn shrink_to(&mut self, min_capacity: u64) {
        if let Err(error) = self.try_shrink_to(min_capacity) {
            panic!("failed to shrink a stable vector: {error}");
        }
    }

    /// Reduces the capacity to the length, giving back the spare space.
    ///
    /// # Panics
    ///
    /// Panics when the reallocation fails; use
    /// [`StableVec::try_shrink_to_fit`] for a fallible version.
    pub fn shrink_to_fit(&mut self) {
        if let Err(error) = self.try_shrink_to_fit() {
            panic!("failed to shrink a stable vector: {error}");
        }
    }

    /// Grows the block so it holds `new_capacity` elements.
    fn grow_to(&mut self, new_capacity: u64) -> AbiResult<()> {
        let element_size = size_of::<T>() as u64;
        let old_bytes = match self.capacity.checked_mul(element_size) {
            Some(bytes) => bytes,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        let new_bytes = match new_capacity.checked_mul(element_size) {
            Some(bytes) => bytes,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        if new_bytes > isize::MAX as u64 {
            // The byte count, not just the element count, has to be a size an
            // allocator can describe. Both overflows are reported as the same
            // status, so a caller that handles one handles the other.
            return Err(AbiError::new(AbiStatus::CapacityOverflow));
        }
        let alignment = align_of::<T>() as u64;

        let pointer = if self.capacity == 0 {
            self.allocator.try_allocate(new_bytes, alignment)?
        } else {
            // SAFETY: the block was produced by this allocator with `old_bytes`
            // and the same alignment, and it is not used until the call
            // succeeds.
            let pointer = unsafe {
                self.allocator
                    .reallocate(self.pointer.cast(), old_bytes, alignment, new_bytes)
            };
            if pointer.is_null() {
                return Err(AbiError::new(AbiStatus::OutOfMemory));
            }
            pointer
        };

        self.pointer = pointer.cast::<T>();
        self.capacity = new_capacity;
        Ok(())
    }

    /// Appends a value.
    ///
    /// # Panics
    ///
    /// Panics when the allocation fails; use [`StableVec::try_push`] for a
    /// fallible version.
    pub fn push(&mut self, value: T) {
        if let Err(error) = self.try_push(value) {
            panic!("failed to append to a stable vector: {error}");
        }
    }

    /// Appends a value, reporting allocation failure.
    pub fn try_push(&mut self, value: T) -> AbiResult<()> {
        self.try_reserve(1)?;
        let index = self.length;
        // SAFETY: the slot is inside the capacity and uninitialised.
        unsafe { self.slot(index).write(value) };
        self.length += 1;
        Ok(())
    }

    /// Removes and returns the last value.
    pub fn pop(&mut self) -> Option<T> {
        if self.length == 0 {
            return None;
        }
        self.length -= 1;
        let index = self.length;
        // SAFETY: the slot held an initialised value that is not reachable
        // through the length anymore.
        Some(unsafe { self.slot(index).read() })
    }

    /// Inserts `value` at `index`, shifting later elements up by one.
    pub fn insert(&mut self, index: u64, value: T) -> AbiResult<()> {
        if index > self.length {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        self.try_reserve(1)?;
        let moved = (self.length - index) as usize;
        // SAFETY: both ranges are inside the capacity; the source slots stay
        // initialised and the destination slot is uninitialised.
        unsafe {
            ptr::copy(self.slot(index), self.slot(index + 1), moved);
            self.slot(index).write(value);
        }
        self.length += 1;
        Ok(())
    }

    /// Removes and returns the value at `index`, shifting later elements down.
    pub fn remove(&mut self, index: u64) -> Option<T> {
        if index >= self.length {
            return None;
        }
        // SAFETY: the slot holds an initialised value.
        let value = unsafe { self.slot(index).read() };
        let moved = (self.length - index - 1) as usize;
        // SAFETY: the source range is inside the length and the destination
        // range ends at the last initialised slot.
        unsafe { ptr::copy(self.slot(index + 1), self.slot(index), moved) };
        self.length -= 1;
        Some(value)
    }

    /// Inserts a copy of `values` at `index`, shifting later elements up.
    ///
    /// The whole run is moved once, so inserting a multi-byte sequence costs one
    /// shift rather than one per byte.
    pub fn try_insert_copy(&mut self, index: u64, values: &[T]) -> AbiResult<()>
    where
        T: Copy,
    {
        if index > self.length {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let added = values.len() as u64;
        if added == 0 {
            return Ok(());
        }
        self.try_reserve(added)?;
        let moved = (self.length - index) as usize;
        // SAFETY: the destination range ends inside the capacity, and the source
        // range starts inside the old length.
        unsafe {
            let destination = self.slot(index);
            ptr::copy(destination, destination.add(values.len()), moved);
            ptr::copy_nonoverlapping(values.as_ptr(), destination, values.len());
        }
        self.length += added;
        Ok(())
    }

    /// Removes `start..end` with one shift, for elements that own nothing.
    ///
    /// # Panics
    ///
    /// Panics when the range is outside the length.
    pub fn remove_copy_range(&mut self, start: u64, end: u64)
    where
        T: Copy,
    {
        assert!(
            start <= end && end <= self.length,
            "remove_copy_range: {start}..{end} is outside 0..{}",
            self.length,
        );
        let moved = (self.length - end) as usize;
        // SAFETY: the source range starts at `end`, the destination starts
        // before it, and the destination range ends inside the old length.
        unsafe { ptr::copy(self.slot(end), self.slot(start), moved) };
        self.length -= end - start;
    }

    /// Removes and returns the value at `index`, moving the last element into
    /// its place.
    pub fn swap_remove(&mut self, index: u64) -> Option<T> {
        if index >= self.length {
            return None;
        }
        let last = self.length - 1;
        // SAFETY: both slots hold initialised values.
        let value = unsafe { self.slot(index).read() };
        if index != last {
            // SAFETY: reading moves the last value out; the length shrinks so
            // the moved-from slot is never dropped.
            let tail = unsafe { self.slot(last).read() };
            unsafe { self.slot(index).write(tail) };
        }
        self.length -= 1;
        Some(value)
    }

    /// Drops every element beyond `new_length`.
    ///
    /// The length is lowered one element at a time as each is released, so a
    /// destructor that panics leaves a vector that is still correct: it no longer
    /// covers the element that gave up, and it never releases one twice. The
    /// elements the pass had not reached are left alive in the block and are
    /// released with the vector; that is the same outcome the standard library's
    /// `Vec::truncate` gives.
    pub fn truncate(&mut self, new_length: u64) {
        while self.length > new_length {
            self.length -= 1;
            let index = self.length;
            // SAFETY: the slot holds an initialised value that is no longer
            // reachable through the length.
            unsafe { T::drop_in_place(self.slot(index)) };
        }
    }

    /// Drops every element, keeping the storage.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Keeps only the elements for which `keep` returns true, letting the
    /// predicate modify them.
    ///
    /// The predicate sees the elements in order. An element it rejects is
    /// released and the elements after it move down, so a panic from the
    /// predicate leaves a vector that is still consistent: it holds every
    /// element the predicate had not decided on yet.
    ///
    /// The pass is one walk: a kept element is moved at most once and a rejected
    /// one is released where it lies, so the cost is linear in the length
    /// whatever the predicate rejects.
    pub fn retain_mut<F: FnMut(&mut T) -> bool>(&mut self, mut keep: F) {
        let old_length = self.length;
        let mut finish = RetainFinish {
            vector: self,
            processed: 0,
            removed: 0,
            old_length,
        };
        while finish.processed < old_length {
            let read = finish.processed;
            // SAFETY: both indices are below the old length.
            let slot = unsafe { finish.vector.pointer.add(read as usize) };
            // SAFETY: the slot holds an initialised value, and the borrow ends
            // before the value is moved or released.
            let retained = keep(unsafe { &mut *slot });
            if retained {
                let write = read - finish.removed;
                if write != read {
                    // SAFETY: the destination is an earlier slot whose value was
                    // already moved forward or released.
                    unsafe { ptr::copy_nonoverlapping(slot, finish.vector.slot(write), 1) };
                }
            } else {
                // The slot stops being part of the vector before its value is
                // released: a destructor that panics must not leave the slot
                // covered by the length, or the guard would copy a value that is
                // already being destroyed and the vector would release it a
                // second time.
                finish.removed += 1;
                finish.processed += 1;
                // SAFETY: the element is being removed, so it is released here
                // and never recorded as part of the length again.
                unsafe { T::drop_in_place(slot) };
                continue;
            }
            finish.processed += 1;
        }
    }

    /// Removes consecutive elements for which `same` returns true, keeping the
    /// first of each run.
    ///
    /// `same(kept, candidate)` sees the last element that was kept and the one
    /// being decided on, so a sorted container can be deduplicated by comparing
    /// the key only. The pass is linear: each kept element is moved at most
    /// once, and the rest are released where they lie.
    ///
    /// Sorting is the caller's step; without it only adjacent duplicates are
    /// removed.
    pub fn dedup_by<F: FnMut(&T, &T) -> bool>(&mut self, mut same: F) {
        let length = self.length as usize;
        let mut write = 0usize;
        let mut read = 0usize;
        while read < length {
            let duplicate =
                write > 0 && same(&self.as_slice()[write - 1], &self.as_slice()[read]);
            if !duplicate {
                if write != read {
                    // SAFETY: both indices are inside the length, and the value
                    // at `write` is one the pass has already rejected.
                    self.as_mut_slice().swap(write, read);
                }
                write += 1;
            }
            read += 1;
        }
        self.truncate(write as u64);
    }

    /// The slots past the length, for code that fills them before declaring a
    /// length.
    ///
    /// Writing into the slice does not change the length: a value written there
    /// is not part of the vector until [`StableVec::set_len`] covers its slot.
    /// Declare the length before the vector is dropped, or the value written into
    /// the spare slot is never released.
    ///
    /// A zero-sized element has no slots: the answer is an empty slice, whatever
    /// the capacity says. The capacity of such a vector is the address-space
    /// bound rather than a number of block positions, so a slice that long would
    /// be a walk with no end for a caller that visited it.
    pub fn spare_capacity_mut(&mut self) -> &mut [MaybeUninit<T>] {
        if size_of::<T>() == 0 {
            return &mut [];
        }
        let spare = self.capacity - self.length;
        // SAFETY: the slots past the length are inside the block and hold no
        // value.
        unsafe {
            core::slice::from_raw_parts_mut(
                self.slot(self.length).cast::<MaybeUninit<T>>(),
                spare as usize,
            )
        }
    }

    /// Declares that the first `new_length` slots hold initialised values.
    ///
    /// This is the step that makes the values written through
    /// [`StableVec::spare_capacity_mut`] part of the vector: before it, they are
    /// outside the length and are never released.
    ///
    /// # Panics
    ///
    /// Panics when `new_length` is past the capacity.
    ///
    /// # Safety
    ///
    /// Every slot below `new_length` must hold an initialised value, either
    /// moved in with `ptr::write` or filled through
    /// [`StableVec::spare_capacity_mut`], and the length must not go backwards
    /// past slots that still hold values.
    pub unsafe fn set_len(&mut self, new_length: u64) {
        assert!(
            new_length <= self.capacity,
            "set_len: {new_length} is past the capacity {}",
            self.capacity
        );
        self.length = new_length;
    }

    /// Removes every element and yields them in order.
    ///
    /// The vector stops covering the slots at once, so an element the iterator
    /// has not reached is released when the iterator is dropped and never twice;
    /// the storage stays with the vector.
    pub fn drain(&mut self) -> Drain<'_, T> {
        let end = self.length;
        self.drain_range(0, end)
    }

    /// Removes the elements in `start..end` and yields them in order.
    ///
    /// The vector stops covering the range at once, so an element the iterator
    /// has not reached is released when the iterator is dropped and never twice.
    /// The elements after `end` move into the gap when the iterator is dropped,
    /// which keeps their order and needs no allocation.
    ///
    /// # Panics
    ///
    /// Panics when `start` is past `end`, or when `end` is past the length.
    pub fn drain_range(&mut self, start: u64, end: u64) -> Drain<'_, T> {
        assert!(start <= end, "drain_range: {start} is past {end}");
        assert!(
            end <= self.length,
            "drain_range: {end} is past the length {}",
            self.length
        );
        let tail = self.length;
        self.length = start;
        Drain {
            vector: self,
            start,
            index: start,
            end,
            tail_start: end,
            tail,
        }
    }

    /// Appends a copy of `values`.
    pub fn try_extend_from_slice(&mut self, values: &[T]) -> AbiResult<()>
    where
        T: Copy,
    {
        let additional = values.len() as u64;
        self.try_reserve(additional)?;
        let start = self.length;
        // SAFETY: the destination range is inside the capacity and does not
        // overlap `values` unless the caller passed a slice of this vector,
        // which the borrow rules forbid.
        unsafe {
            ptr::copy_nonoverlapping(values.as_ptr(), self.slot(start), values.len());
        }
        self.length += additional;
        Ok(())
    }

    /// Copies the vector, reporting a copy or an allocation that fails.
    ///
    /// The elements are copied by [`TryClone`](crate::clone::TryClone), which is
    /// the same implementation the trait offers: this method names it so that a
    /// caller need not import the trait.
    pub fn try_clone(&self) -> AbiResult<Self>
    where
        T: crate::clone::TryClone,
    {
        <Self as crate::clone::TryClone>::try_clone(self)
    }
}

/// The capacity a vector grows to on its first allocation.
const MINIMUM_CAPACITY: u64 = 4;

impl<T: StableElement + Copy> StableVec<T> {
    /// Copies `values` with one allocation and one bulk move.
    ///
    /// This is the cheap path for element types that are plain data; the
    /// element-wise constructor exists for types that must be cloned.
    pub fn from_copy_slice(values: &[T]) -> AbiResult<Self> {
        Self::from_copy_slice_with_allocator(AbiAllocator::system(), values)
    }

    /// Copies `values` with one allocation and one bulk move, using
    /// `allocator`.
    pub fn from_copy_slice_with_allocator(
        allocator: AbiAllocator,
        values: &[T],
    ) -> AbiResult<Self> {
        let mut vector = Self::with_capacity_and_allocator(allocator, values.len() as u64)?;
        vector.try_extend_from_slice(values)?;
        Ok(vector)
    }
}

impl<T: StableElement + Clone> TryFrom<&[T]> for StableVec<T> {
    type Error = AbiError;

    fn try_from(values: &[T]) -> AbiResult<Self> {
        Self::from_slice(values)
    }
}

impl<T: StableElement> StableVec<T> {
    /// Adopts storage that arrived from another build, checking that the header
    /// describes a block that could exist.
    ///
    /// A length beyond the capacity, a null or misaligned pointer, or a capacity
    /// whose byte size is not addressable is rejected instead of trusted. The
    /// allocator must be the one that produced the block, and the block must
    /// hold `length` initialised values.
    ///
    /// # Safety
    ///
    /// `pointer` must be a live block from `allocator` with at least
    /// `capacity` slots.
    pub unsafe fn from_raw_parts(
        pointer: *mut T,
        length: u64,
        capacity: u64,
        allocator: AbiAllocator,
    ) -> AbiResult<Self> {
        let vector = Self {
            pointer,
            length,
            capacity,
            allocator,
            element: PhantomData,
        };
        vector.validate_header()?;
        Ok(vector)
    }

    /// Checks the header without reading the elements.
    ///
    /// The checks are the ones a value from another build has to pass before
    /// its contents may be read: the length fits the capacity, the pointer
    /// names an aligned, non-null address, neither the initialised region nor
    /// the whole block has a byte count that is not addressable, and the capacity
    /// fits a `usize` even when the element type is zero-sized.
    ///
    /// A header with no capacity describes no block, so its address has to be
    /// the one an empty vector carries. Any other address names a block this
    /// build would neither read nor release: the release of a vector is decided
    /// by its capacity, so a stray address under a zero capacity is a leak that
    /// the receiving side can still refuse.
    ///
    /// The other direction is refused as well: a block larger than nothing has an
    /// address, and the dangling one is the address of no block. A vector of
    /// zero-sized elements is the exception, because it allocates nothing and
    /// carries that address at every capacity.
    ///
    /// Nothing here bounds the length of a zero-sized element type: such a
    /// vector needs no block, so the length is the whole value and no capacity
    /// can contradict it. Walking such a vector costs one step per element, and a
    /// receiver that accepts lengths it did not produce bounds them itself.
    pub fn validate_header(&self) -> AbiResult<()> {
        if self.pointer.is_null() || !(self.pointer as usize).is_multiple_of(align_of::<T>()) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        if self.length > self.capacity {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        if self.capacity == 0 && self.pointer != NonNull::<T>::dangling().as_ptr() {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        if self.capacity > 0
            && size_of::<T>() > 0
            && self.pointer == NonNull::<T>::dangling().as_ptr()
        {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        if self.capacity > isize::MAX as u64 {
            return Err(AbiError::new(AbiStatus::CapacityOverflow));
        }
        match self.capacity.checked_mul(size_of::<T>() as u64) {
            Some(bytes) if bytes <= isize::MAX as u64 => Ok(()),
            Some(_) => Err(AbiError::new(AbiStatus::CapacityOverflow)),
            None => Err(AbiError::new(AbiStatus::CapacityOverflow)),
        }
    }

    /// Reports whether the header describes a block this build may release.
    ///
    /// A header that fails [`StableVec::validate_header`] says nothing about
    /// the address, so releasing it would free whatever the number happens to
    /// name. Such a value is dropped without touching its storage.
    pub fn is_releasable(&self) -> bool {
        self.validate_header().is_ok()
    }

    /// Checks the header and every element the length covers.
    ///
    /// A vector of zero-sized elements is refused a length here rather than in
    /// [`StableVec::validate_header`]: such a vector carries nothing but that
    /// number, so no byte count, alignment or address says whether it is true,
    /// and walking it would run as many steps as the producer named. The header
    /// check stays as it is because it also decides whether this build may
    /// release the storage of a value it already holds, and a vector of
    /// zero-sized elements holds values whose destructors still have to run.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        self.validate_header()?;
        if size_of::<T>() == 0 && self.length != 0 {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        for value in self.as_slice() {
            crate::boundary::check(value)?;
        }
        Ok(())
    }

    /// Takes the value apart without releasing the storage.
    ///
    /// The caller takes over the obligation to release the block through the
    /// returned allocator, after dropping `length` elements.
    ///
    /// # Safety
    ///
    /// Every element stays alive and unreachable through this value; the
    /// returned block must be released exactly once.
    pub unsafe fn into_raw_parts(self) -> (*mut T, u64, u64, AbiAllocator) {
        let this = core::mem::ManuallyDrop::new(self);
        (this.pointer, this.length, this.capacity, this.allocator)
    }
}

impl<T: StableElement + Clone> TryFrom<Vec<T>> for StableVec<T> {
    type Error = AbiError;

    fn try_from(values: Vec<T>) -> AbiResult<Self> {
        Self::from_slice(&values)
    }
}

impl<T: StableElement + Clone> From<StableVec<T>> for Vec<T> {
    fn from(vector: StableVec<T>) -> Self {
        // The clone lands in the process allocator, and the boundary storage is
        // released with the vector.
        vector.as_slice().to_vec()
    }
}

impl<T: StableElement + crate::zeroize::Wipe> StableVec<T> {
    /// Overwrites the whole block, including spare capacity, with zeros before
    /// it is released.
    ///
    /// Every element is wiped in place first, so a container of containers is
    /// wiped down to its elements; the spare slots hold no value and are
    /// overwritten directly. The vector is then dropped as usual, which releases
    /// the blocks the values still own.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::CapacityOverflow`] when the byte count of the block cannot
    /// be computed, which a value built by this crate cannot produce.
    pub fn wipe(mut self) -> AbiResult<()> {
        if size_of::<T>() == 0 {
            return Ok(());
        }
        for value in self.as_mut_slice() {
            value.wipe_in_place();
        }
        if self.capacity > self.length {
            let spare = match (self.capacity - self.length).checked_mul(size_of::<T>() as u64) {
                Some(spare) => spare,
                None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
            };
            // SAFETY: the slots past the length hold no value, and the block is
            // writable for its whole capacity.
            unsafe {
                crate::zeroize::volatile_zero_raw(
                    self.pointer.add(self.length as usize).cast(),
                    spare,
                    1,
                )?
            };
        }
        Ok(())
    }
}

impl<T: StableElement> core::ops::Deref for StableVec<T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: StableElement> core::ops::DerefMut for StableVec<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T: StableElement + core::hash::Hash> core::hash::Hash for StableVec<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl<'a, T: StableElement + core::hash::Hash> core::hash::Hash for StableSlice<'a, T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl<'a, T: StableElement + core::hash::Hash> core::hash::Hash for StableSliceMut<'a, T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl<'a, T: StableElement> crate::clone::TryClone for StableSlice<'a, T> {
    /// Names the same storage: a borrowed view owns nothing, so its copy is
    /// another view of what the caller lent.
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(*self)
    }
}

impl<T: StableElement + PartialOrd> PartialOrd for StableVec<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.as_slice().partial_cmp(other.as_slice())
    }
}

impl<T: StableElement + Ord> Ord for StableVec<T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl<'a, T: StableElement> IntoIterator for &'a StableVec<T> {
    type Item = &'a T;
    type IntoIter = core::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'a, T: StableElement> IntoIterator for &'a mut StableVec<T> {
    type Item = &'a mut T;
    type IntoIter = core::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}

/// An iterator that moves values out of a [`StableVec`].
///
/// The vector stops covering the slots as soon as the iterator is created, so an
/// element the iterator has not yielded is released when the iterator is
/// dropped, and never twice.
pub struct Drain<'a, T: StableElement> {
    vector: &'a mut StableVec<T>,
    start: u64,
    index: u64,
    end: u64,
    /// Where the elements after the drained range begin. It stays put when the
    /// back of the range is taken, so the tail is still moved from initialised
    /// slots.
    tail_start: u64,
    tail: u64,
}

impl<T: StableElement> Iterator for Drain<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.index >= self.end {
            return None;
        }
        let slot = self.vector.slot(self.index);
        self.index += 1;
        // SAFETY: the slot held an initialised value that the vector no longer
        // covers.
        Some(unsafe { slot.read() })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.end - self.index) as usize;
        (remaining, Some(remaining))
    }
}

impl<T: StableElement> DoubleEndedIterator for Drain<'_, T> {
    fn next_back(&mut self) -> Option<T> {
        if self.index >= self.end {
            return None;
        }
        self.end -= 1;
        let slot = self.vector.slot(self.end);
        // SAFETY: the slot held an initialised value that the vector no longer
        // covers, and the cursor no longer names it.
        Some(unsafe { slot.read() })
    }
}

impl<T: StableElement> ExactSizeIterator for Drain<'_, T> {}

impl<T: StableElement> Drop for Drain<'_, T> {
    fn drop(&mut self) {
        while self.index < self.end {
            let slot = self.vector.slot(self.index);
            self.index += 1;
            // SAFETY: the slot holds an initialised value that has not been
            // yielded.
            unsafe { T::drop_in_place(slot) };
        }
        // The elements after the range move into the hole, which keeps their
        // order and needs no allocation. Nothing was yielded from them, so they
        // are moved rather than released.
        let moved = self.tail - self.tail_start;
        if moved > 0 {
            // SAFETY: both ranges are inside the block, and `copy` handles the
            // overlap that moving a tail down creates.
            unsafe {
                ptr::copy(
                    self.vector.slot(self.tail_start),
                    self.vector.slot(self.start),
                    moved as usize,
                );
            }
        }
        self.vector.length = self.start + moved;
    }
}

impl<T: StableElement> fmt::Debug for Drain<'_, T> {
    // The values can be content a log must not repeat, so the debug form reports
    // how many are left.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Drain")
            .field("len", &(self.end - self.index))
            .finish()
    }
}

/// An owning iterator over a [`StableVec`].
///
/// The vector's block stays with the iterator until it is exhausted or dropped;
/// the remaining elements are released at that point.
pub struct StableIntoIter<T: StableElement> {
    pointer: *mut T,
    length: u64,
    capacity: u64,
    index: u64,
    allocator: AbiAllocator,
    element: PhantomData<T>,
}

impl<T: StableElement> fmt::Debug for StableIntoIter<T> {
    // The address is inside the block the vector allocated; printing it would
    // put the layout of the allocating side into a log.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableIntoIter")
            .field("remaining", &(self.length - self.index))
            .finish()
    }
}

impl<T: StableElement> Iterator for StableIntoIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.index >= self.length {
            return None;
        }
        let pointer = if size_of::<T>() == 0 {
            self.pointer
        } else {
            // SAFETY: the index is below the length.
            unsafe { self.pointer.add(self.index as usize) }
        };
        self.index += 1;
        // SAFETY: the slot holds an initialised value that the vector will not
        // release again, because its length was taken by this iterator.
        Some(unsafe { pointer.read() })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.length - self.index) as usize;
        (remaining, Some(remaining))
    }
}

impl<T: StableElement> ExactSizeIterator for StableIntoIter<T> {}

impl<T: StableElement> Drop for StableIntoIter<T> {
    fn drop(&mut self) {
        // The block is guarded for the same reason the vector guards its own:
        // an element's destructor is the caller's code and may panic, and the
        // allocation must not go with it.
        let block = BlockRelease {
            pointer: self.pointer,
            capacity: self.capacity,
            allocator: self.allocator,
        };
        while self.index < self.length {
            let pointer = if size_of::<T>() == 0 {
                self.pointer
            } else {
                // SAFETY: the index is below the length.
                unsafe { self.pointer.add(self.index as usize) }
            };
            // SAFETY: the slot holds an initialised value that has not been
            // handed out.
            unsafe { T::drop_in_place(pointer) };
            self.index += 1;
        }
        drop(block);
        self.capacity = 0;
    }
}

impl<T: StableElement> IntoIterator for StableVec<T> {
    type Item = T;
    type IntoIter = StableIntoIter<T>;

    fn into_iter(mut self) -> Self::IntoIter {
        let iterator = StableIntoIter {
            pointer: self.pointer,
            length: self.length,
            capacity: self.capacity,
            index: 0,
            allocator: self.allocator,
            element: PhantomData,
        };
        // The iterator now owns the block and the elements.
        self.length = 0;
        self.capacity = 0;
        core::mem::forget(self);
        iterator
    }
}

impl<T: StableElement> Drop for StableVec<T> {
    fn drop(&mut self) {
        // A header this build does not accept says nothing about the block: the
        // address, the length and the capacity cannot be trusted, so nothing is
        // released. The storage is left to whoever produced the value.
        if !self.is_releasable() {
            return;
        }
        // The block is given to a guard before the elements are released, so it
        // is released even when an element's destructor panics. The elements
        // after that one are then left alive, which is what the standard
        // library's `Vec` does as well: a panic in a destructor stops the
        // sequence, and the block is not allowed to leak with it.
        let block = BlockRelease {
            pointer: self.pointer,
            capacity: self.capacity,
            allocator: self.allocator,
        };
        while self.length > 0 {
            self.length -= 1;
            let index = self.length;
            // `slot` handles the zero-sized case, and the element contract is
            // what releases the value, exactly as `truncate` does.
            // SAFETY: the slot holds an initialised value that is not reachable
            // through the length anymore.
            unsafe { T::drop_in_place(self.slot(index)) };
        }
        drop(block);
        self.capacity = 0;
    }
}

/// Releases a block when the value holding it is dropped, however that happens.
///
/// A vector's drop releases its elements one by one, and an element's destructor
/// is the caller's code: it may panic. A guard is what keeps the allocation from
/// leaking when that happens, because a local is dropped during unwinding as
/// well as on the way out.
struct BlockRelease<T: StableElement> {
    pointer: *mut T,
    capacity: u64,
    allocator: AbiAllocator,
}

impl<T: StableElement> Drop for BlockRelease<T> {
    fn drop(&mut self) {
        if self.capacity > 0 && size_of::<T>() > 0 {
            let bytes = self.capacity * size_of::<T>() as u64;
            // SAFETY: the block came from this allocator with this layout, and
            // the values it held have been released or taken by the iterator.
            unsafe {
                self.allocator
                    .deallocate(self.pointer.cast(), bytes, align_of::<T>() as u64)
            };
        }
    }
}

// SAFETY: the vector owns a fixed-layout header, its drop releases every
// element through the element contract and then releases the block through the
// recorded allocator.
unsafe impl<T: StableElement> StableElement for StableVec<T> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { ptr::drop_in_place(ptr) }
    }
}

impl<T: StableElement> core::iter::Extend<T> for StableVec<T> {
    /// Appends every value, panicking when the storage cannot grow; use
    /// [`StableVec::try_extend`] where a failure has to be reported.
    fn extend<I: IntoIterator<Item = T>>(&mut self, items: I) {
        if let Err(error) = self.try_extend(items) {
            panic!("failed to extend a stable vector: {error}");
        }
    }
}

impl<T: StableElement> core::iter::FromIterator<T> for StableVec<T> {
    /// Builds a vector from the values, panicking when the storage cannot grow.
    fn from_iter<I: IntoIterator<Item = T>>(items: I) -> Self {
        let mut values = Self::new();
        for value in items {
            values.push(value);
        }
        values
    }
}

impl<T: StableElement> Default for StableVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: StableElement + crate::zeroize::Wipe> crate::zeroize::Wipe for StableVec<T> {
    fn wipe_in_place(&mut self) {
        for value in self.as_mut_slice() {
            value.wipe_in_place();
        }
        // The slots past the length hold no value, but they can still hold what
        // a removed element left behind: `truncate`, `clear`, `remove` and `pop`
        // release their element without writing over it. The address comes from
        // `storage_ptr` and not from a slice, because a slice covers the
        // initialised elements only and writing past it would describe memory
        // the borrow does not name.
        if size_of::<T>() > 0 && self.capacity > self.length {
            // SAFETY: the slots past the length are inside the block and hold no
            // value.
            let _ = unsafe {
                crate::zeroize::volatile_zero_raw(
                    self.storage_ptr().add(self.length as usize).cast(),
                    self.capacity - self.length,
                    size_of::<T>() as u64,
                )
            };
        }
    }
}

impl<T: StableElement + Clone> Clone for StableVec<T> {
    /// Copies the elements through [`Clone`], which raises a copy or an
    /// allocation that fails; [`StableVec::try_clone`] reports it
    /// instead.
    fn clone(&self) -> Self {
        match Self::from_slice_with_allocator(self.allocator, self.as_slice()) {
            Ok(clone) => clone,
            Err(error) => panic!("failed to clone a stable vector: {error}"),
        }
    }
}

impl<T: StableElement + PartialEq> PartialEq for StableVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<T: StableElement + Eq> Eq for StableVec<T> {}

impl<T: StableElement> fmt::Debug for StableVec<T> {
    // The elements can be content a log must not repeat, so the debug form
    // reports the shape. `as_slice` prints the values when a caller asks for
    // them.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableVec")
            .field("len", &self.length)
            .field("capacity", &self.capacity)
            .finish()
    }
}

/// Takes the element the first sequence holds at this position, and moves both
/// sides past nothing else.
const PLAN_LEFT: u8 = 0;
/// Takes the element the second sequence holds.
const PLAN_RIGHT: u8 = 1;
/// Takes the element the second sequence holds and releases the one the first
/// sequence held at the same position.
const PLAN_REPLACE: u8 = 2;
/// Takes the element the first sequence holds and releases the one the second
/// sequence held at the same position.
const PLAN_KEEP: u8 = 3;

/// Merges two ascending sequences into `target` and leaves `incoming` empty.
///
/// `keep_existing_on_equal` decides which of two equal elements the result
/// holds: the one `target` already had, or the one `incoming` brings.
///
/// The comparisons run before any value moves, and the moves compare nothing.
/// That ordering is what makes an interrupted call harmless: `compare` is the
/// caller's own code and may panic, and a panic there leaves `target` holding
/// exactly the elements it held. Both blocks - the record of how the two sides
/// interleave, and the merged block - are taken first for the same reason, so an
/// allocator that refuses also leaves both sequences as they were.
///
/// The record costs one byte per element. It exists to keep "decide the order"
/// and "apply the order" apart, which is the only way a merge of values that
/// cannot be copied stays whole when a comparison stops it.
pub(crate) fn try_merge_sorted<E: StableElement>(
    target: &mut StableVec<E>,
    incoming: &mut StableVec<E>,
    keep_existing_on_equal: bool,
    compare: impl Fn(&E, &E) -> core::cmp::Ordering,
) -> AbiResult<()> {
    let allocator = target.allocator();
    let total = match target.len().checked_add(incoming.len()) {
        Some(total) => total,
        None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
    };
    let mut plan = StableVec::<u8>::with_capacity_and_allocator(allocator, total)?;
    let mut merged = StableVec::with_capacity_and_allocator(allocator, total)?;
    {
        let left = target.as_slice();
        let right = incoming.as_slice();
        let mut at_left = 0usize;
        let mut at_right = 0usize;
        loop {
            let step = match (left.get(at_left), right.get(at_right)) {
                (Some(left_value), Some(right_value)) => match compare(left_value, right_value) {
                    core::cmp::Ordering::Less => PLAN_LEFT,
                    core::cmp::Ordering::Greater => PLAN_RIGHT,
                    core::cmp::Ordering::Equal => {
                        if keep_existing_on_equal {
                            PLAN_KEEP
                        } else {
                            // The arriving element takes the position, and the
                            // one it replaces is released with it.
                            PLAN_REPLACE
                        }
                    }
                },
                (Some(_), None) => PLAN_LEFT,
                (None, Some(_)) => PLAN_RIGHT,
                (None, None) => break,
            };
            match step {
                PLAN_LEFT => at_left += 1,
                PLAN_RIGHT => at_right += 1,
                // Both sides move past the position: one element is emitted and
                // the other is released by the move below.
                _ => {
                    at_left += 1;
                    at_right += 1;
                }
            }
            // The block was reserved for exactly this many steps.
            plan.push(step);
        }
    }
    let incoming_allocator = incoming.allocator();
    let left = core::mem::replace(target, StableVec::new_with_allocator(allocator));
    let right = core::mem::replace(incoming, StableVec::new_with_allocator(incoming_allocator));
    let mut left_values = left.into_iter();
    let mut right_values = right.into_iter();
    for step in plan.as_slice() {
        match *step {
            PLAN_LEFT => merged.push(
                left_values
                    .next()
                    .expect("the plan names an element the first sequence holds"),
            ),
            PLAN_RIGHT => merged.push(
                right_values
                    .next()
                    .expect("the plan names an element the second sequence holds"),
            ),
            PLAN_REPLACE => {
                let _ = left_values.next();
                merged.push(
                    right_values
                        .next()
                        .expect("the plan names an element the second sequence holds"),
                );
            }
            _ => {
                merged.push(
                    left_values
                        .next()
                        .expect("the plan names an element the first sequence holds"),
                );
                let _ = right_values.next();
            }
        }
    }
    *target = merged;
    Ok(())
}
