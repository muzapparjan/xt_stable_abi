//! One value owned through the allocator that produced it.

use core::fmt;
use core::mem::size_of;
use core::ops::{Deref, DerefMut};

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::StableElement;

/// Reports whether `pointer` names an address a value of `T` may sit at.
/// Reports whether the handle names a value: a non-null, aligned address, and -
/// for a type that occupies bytes - an address a block can have.
///
/// The dangling address belongs to no block, so a handle that carries it for a
/// non-zero-sized type names nothing: reading through it, or releasing it, would
/// reach memory that was never allocated. A zero-sized value allocates nothing
/// and carries exactly that address, which is why the type is exempt.
fn names_a_value<T>(pointer: *const T) -> bool {
    !pointer.is_null()
        && (pointer as usize).is_multiple_of(core::mem::align_of::<T>())
        && (core::mem::size_of::<T>() == 0
            || pointer != core::ptr::NonNull::<T>::dangling().as_ptr())
}

/// A value on the heap, with the allocator that owns the block.
///
/// The handle stores an address and the allocator, so a value handed to another
/// build is released through the functions that produced it. The value itself
/// keeps its layout, which is why `T` has to be a [`StableElement`].
#[repr(C)]
pub struct StableBox<T: StableElement> {
    pointer: *mut T,
    allocator: AbiAllocator,
}

impl<T: StableElement> StableBox<T> {
    /// A handle that names no value.
    ///
    /// It owns no block, so dropping it releases nothing and [`StableBox::validate`]
    /// refuses it. This is the value a call that could not answer returns when
    /// there is no heap to allocate from: building it never allocates and never
    /// panics.
    pub const fn empty() -> Self {
        Self {
            pointer: core::ptr::null_mut(),
            allocator: AbiAllocator::system(),
        }
    }

    /// Reports whether the handle names a value.
    pub fn is_empty(&self) -> bool {
        !names_a_value(self.pointer)
    }

    /// Moves `value` onto the process heap.
    ///
    /// # Panics
    ///
    /// Panics when the allocation fails; use [`StableBox::try_new`] for a
    /// fallible version.
    pub fn new(value: T) -> Self {
        match Self::try_new(value) {
            Ok(boxed) => boxed,
            Err(error) => panic!("failed to box a stable value: {error}"),
        }
    }

    /// Moves `value` onto the process heap, reporting allocation failure.
    pub fn try_new(value: T) -> AbiResult<Self> {
        Self::try_new_with_allocator(value, AbiAllocator::system())
    }

    /// Moves `value` onto a block from `allocator`.
    pub fn try_new_with_allocator(value: T, allocator: AbiAllocator) -> AbiResult<Self> {
        let size = size_of::<T>() as u64;
        let align = core::mem::align_of::<T>() as u64;
        let block = allocator.try_allocate(size, align)?;
        // SAFETY: the block holds one `T`, and a zero-sized `T` accepts the
        // aligned address returned for a zero-sized request.
        unsafe { block.cast::<T>().write(value) };
        Ok(Self {
            pointer: block.cast(),
            allocator,
        })
    }

    /// Adopts a block that arrived from another build.
    ///
    /// # Safety
    ///
    /// `pointer` must be a live block from `allocator` that holds one
    /// initialised `T`.
    pub const unsafe fn from_raw_parts(pointer: *mut T, allocator: AbiAllocator) -> Self {
        Self { pointer, allocator }
    }

    /// Adopts a block after checking that it can hold one `T`.
    ///
    /// `available` is the number of bytes the receiving side actually holds.
    ///
    /// # Safety
    ///
    /// `pointer` must be a live block from `allocator` of `available` bytes that
    /// holds one initialised `T`.
    pub unsafe fn from_raw_parts_checked(
        pointer: *mut T,
        available: u64,
        allocator: AbiAllocator,
    ) -> AbiResult<Self> {
        if !names_a_value(pointer) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        if available < size_of::<T>() as u64 {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        Ok(Self { pointer, allocator })
    }

    /// Checks that the handle names an aligned, non-null block and that the
    /// value inside it is one its type defines.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        if !names_a_value(self.pointer) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        // SAFETY: the block holds one initialised value for the life of the
        // handle.
        crate::boundary::check(unsafe { &*self.pointer })
    }

    /// Overwrites the value with zeros before releasing it.
    ///
    /// The value is wiped through its own contract rather than by zeroing the
    /// block it sits in. Zeroing the block of a container would overwrite its
    /// header, which leaves the storage it owns unreachable: the content would
    /// stay in memory and the block would never be released. Wiping the value
    /// leaves it valid, so the drop that follows releases exactly what it still
    /// owns.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::NullPointer`] when the handle names no value.
    pub fn wipe(mut self) -> AbiResult<()>
    where
        T: crate::zeroize::Wipe,
    {
        if !names_a_value(self.pointer) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        self.wipe_in_place();
        Ok(())
    }

    /// The value inside the handle.
    ///
    /// # Panics
    ///
    /// Panics when the handle names no value; use [`StableBox::is_empty`] or
    /// [`StableBox::validate`] to decide before reading.
    fn value(&self) -> &T {
        if !names_a_value(self.pointer) {
            panic!("the boxed handle names no value");
        }
        // SAFETY: the check above is the one `is_empty` and `Drop` use, so a
        // handle that reaches this point names an aligned, non-null block that
        // holds one initialised `T`.
        unsafe { &*self.pointer }
    }

    /// Takes the value out and releases the block.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::NullPointer`] when the handle names no value.
    pub fn into_inner(self) -> AbiResult<T> {
        if !names_a_value(self.pointer) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        let this = core::mem::ManuallyDrop::new(self);
        // SAFETY: `this` is not dropped, so the value moves out exactly once.
        let value = unsafe { core::ptr::read(this.pointer) };
        // SAFETY: the block came from this allocator with this layout, and the
        // value it held has moved out.
        unsafe {
            this.allocator.deallocate(
                this.pointer.cast(),
                size_of::<T>() as u64,
                core::mem::align_of::<T>() as u64,
            )
        };
        Ok(value)
    }

    /// The allocator that owns the block.
    pub const fn allocator(&self) -> AbiAllocator {
        self.allocator
    }

    /// The value, when the handle names one.
    pub fn as_ref(&self) -> Option<&T> {
        if names_a_value(self.pointer) {
            // SAFETY: the check above is the one `is_empty` and `Drop` use.
            Some(unsafe { &*self.pointer })
        } else {
            None
        }
    }

    /// The value, for writing, when the handle names one.
    pub fn as_mut(&mut self) -> Option<&mut T> {
        if names_a_value(self.pointer) {
            // SAFETY: the check above is the one `is_empty` and `Drop` use, and
            // `&mut self` grants exclusive access.
            Some(unsafe { &mut *self.pointer })
        } else {
            None
        }
    }

    /// Moves the value out and releases the block, leaving the handle empty.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::NullPointer`] when the handle names no value.
    pub fn take(&mut self) -> AbiResult<T> {
        if !names_a_value(self.pointer) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        let previous = core::mem::take(self);
        previous.into_inner()
    }

    /// Takes the handle apart without releasing anything.
    ///
    /// # Safety
    ///
    /// The returned block must be released exactly once through `allocator`,
    /// with the layout of `T`, after the value it holds is released.
    pub unsafe fn into_raw(self) -> (*mut T, AbiAllocator) {
        let this = core::mem::ManuallyDrop::new(self);
        (this.pointer, this.allocator)
    }

    /// Rebuilds a handle from the parts [`StableBox::into_raw`] returned.
    ///
    /// # Safety
    ///
    /// `pointer` must name an aligned block from `allocator` that holds one
    /// initialised `T`, and it must not be reachable through another handle.
    pub unsafe fn from_raw(pointer: *mut T, allocator: AbiAllocator) -> AbiResult<Self> {
        if !names_a_value(pointer) {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        Ok(Self { pointer, allocator })
    }

    /// The address of the value.
    pub const fn as_ptr(&self) -> *const T {
        self.pointer
    }
}

impl<T: StableElement> Default for StableBox<T> {
    /// An empty handle.
    fn default() -> Self {
        Self::empty()
    }
}

/// Reads the value through the handle.
///
/// # Panics
///
/// Panics when the handle names no value, which an empty box, a box that was
/// taken apart by value, and the value [`Default`] and
/// [`PanicFallback`](crate::boundary::PanicFallback) produce all do. A caller
/// that cannot rule out an empty box reads it through [`StableBox::as_ref`],
/// which answers `None` instead.
impl<T: StableElement> Deref for StableBox<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value()
    }
}

/// Writes the value through the handle.
///
/// # Panics
///
/// Panics when the handle names no value; see the implementation of [`Deref`]
/// for the same condition. [`StableBox::as_mut`] is the form that reports it.
impl<T: StableElement> DerefMut for StableBox<T> {
    fn deref_mut(&mut self) -> &mut T {
        if !names_a_value(self.pointer) {
            panic!("the boxed handle names no value");
        }
        // SAFETY: the check above is the one `is_empty` and `Drop` use, and
        // `&mut self` grants exclusive access to the block it names.
        unsafe { &mut *self.pointer }
    }
}

impl<T: StableElement + Clone> Clone for StableBox<T> {
    fn clone(&self) -> Self {
        if !names_a_value(self.pointer) {
            // A handle that names no value has nothing to copy, so its copy is
            // another handle that names no value.
            return Self::empty();
        }
        match Self::try_new_with_allocator(T::clone(self.value()), self.allocator) {
            Ok(clone) => clone,
            Err(error) => panic!("failed to clone a stable box: {error}"),
        }
    }
}

impl<T: StableElement + PartialEq> PartialEq for StableBox<T> {
    fn eq(&self, other: &Self) -> bool {
        match (names_a_value(self.pointer), names_a_value(other.pointer)) {
            (false, false) => true,
            (true, true) => *self.value() == *other.value(),
            _ => false,
        }
    }
}

impl<T: StableElement + Eq> Eq for StableBox<T> {}

impl<T: StableElement> Drop for StableBox<T> {
    fn drop(&mut self) {
        // A handle whose address cannot name a value of `T` says nothing about
        // what sits there, so nothing is released. Such a value is refused by
        // `validate` before it reaches safe code.
        if !names_a_value(self.pointer) {
            return;
        }
        // SAFETY: the block holds one initialised value that is not reachable
        // afterwards, and it came from this allocator with this layout.
        unsafe {
            T::drop_in_place(self.pointer);
            self.allocator.deallocate(
                self.pointer.cast(),
                size_of::<T>() as u64,
                core::mem::align_of::<T>() as u64,
            );
        }
    }
}

impl<T: StableElement + crate::zeroize::Wipe> crate::zeroize::Wipe for StableBox<T> {
    fn wipe_in_place(&mut self) {
        if names_a_value(self.pointer) {
            // SAFETY: the block holds one writable value.
            unsafe { (*self.pointer).wipe_in_place() };
        }
    }
}

impl<T: StableElement> fmt::Debug for StableBox<T> {
    // The value can be content a log must not repeat, so the debug form reports
    // the shape.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableBox")
            .field("size", &size_of::<T>())
            .finish()
    }
}

// SAFETY: the handle is an address and an allocator, and dropping it releases
// exactly the value and the block it owns.
unsafe impl<T: StableElement> StableElement for StableBox<T> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<T: StableElement + PartialOrd> PartialOrd for StableBox<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match (self.is_empty(), other.is_empty()) {
            (true, true) => Some(core::cmp::Ordering::Equal),
            (true, false) => Some(core::cmp::Ordering::Less),
            (false, true) => Some(core::cmp::Ordering::Greater),
            (false, false) => (**self).partial_cmp(&**other),
        }
    }
}

impl<T: StableElement + Ord> Ord for StableBox<T> {
    /// Orders a handle that names no value before one that does, and compares
    /// the values themselves after that.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match (self.is_empty(), other.is_empty()) {
            (true, true) => core::cmp::Ordering::Equal,
            (true, false) => core::cmp::Ordering::Less,
            (false, true) => core::cmp::Ordering::Greater,
            (false, false) => (**self).cmp(&**other),
        }
    }
}

impl<T: StableElement + core::hash::Hash> core::hash::Hash for StableBox<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self.is_empty() {
            true => core::hash::Hash::hash(&0u32, state),
            false => core::hash::Hash::hash(&**self, state),
        }
    }
}
