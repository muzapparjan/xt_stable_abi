//! The allocator handle that every owned boundary value carries.
//!
//! Memory that crosses the boundary is allocated by one side and released by
//! the other, and the two sides may link different global allocators. The rule
//! that makes this sound is: an allocation is released by the code that made
//! it, reached through function pointers that travel with the value. An
//! [`AbiAllocator`] is exactly that set of function pointers plus the context
//! those functions expect.
//!
//! Every owned type in this crate stores the allocator it was built with, so a
//! value produced by a dynamic library is freed by that library even when the
//! host drops it.

use core::ffi::c_void;
use core::fmt;
use core::ptr;

/// Allocates `size` bytes aligned to `align`.
///
/// Returns null on failure. `size` is non-zero whenever this is called through
/// [`AbiAllocator::allocate`].
pub type AllocateFn = unsafe extern "C" fn(context: *mut c_void, size: u64, align: u64) -> *mut u8;

/// Releases a block previously returned by [`AllocateFn`].
///
/// `ptr`, `size` and `align` are exactly what the allocating call received.
pub type DeallocateFn =
    unsafe extern "C" fn(context: *mut c_void, ptr: *mut u8, size: u64, align: u64);

/// Grows or shrinks a block previously returned by the same allocator.
///
/// On success the old pointer is invalidated and the contents up to
/// `min(old_size, new_size)` are preserved. On failure null is returned and the
/// old block stays valid.
pub type ReallocateFn =
    unsafe extern "C" fn(context: *mut c_void, ptr: *mut u8, old_size: u64, align: u64, new_size: u64) -> *mut u8;

/// A set of allocation functions and the context they operate on.
///
/// The handle is `Copy` and pointer-sized on the fields it holds; it is stored
/// inside every owned value, so passing a value across the boundary also passes
/// the information needed to release it.
///
/// Accepting an owned value from another build means agreeing to call the three
/// functions it carries, with the context it carries, when the value is released
/// or grown. Those functions are code from the other side; that is the point of
/// the design, and it is also why a boundary value is only as trustworthy as the
/// build that produced it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AbiAllocator {
    context: *mut c_void,
    allocate_fn: AllocateFn,
    deallocate_fn: DeallocateFn,
    reallocate_fn: ReallocateFn,
}

// The handle is deliberately neither `Send` nor `Sync`. Its context and its
// functions belong to whoever built it, and nothing in this crate can prove
// that the other side accepts calls from another thread. Owned boundary values
// therefore stay on the thread that built them; a holder that knows its
// allocator is safe across threads can wrap the value in its own type and state
// that guarantee.

impl AbiAllocator {
    /// Builds an allocator from raw function pointers.
    ///
    /// # Safety
    ///
    /// The three functions must implement the contracts on [`AllocateFn`],
    /// [`DeallocateFn`] and [`ReallocateFn`] for the same `context`, and every
    /// value built with this allocator must be released through it before
    /// `context` stops being valid.
    pub const unsafe fn from_parts(
        context: *mut c_void,
        allocate_fn: AllocateFn,
        deallocate_fn: DeallocateFn,
        reallocate_fn: ReallocateFn,
    ) -> Self {
        Self {
            context,
            allocate_fn,
            deallocate_fn,
            reallocate_fn,
        }
    }

    /// The allocator this process uses for boundary values by default.
    pub const fn system() -> Self {
        SYSTEM_ALLOCATOR
    }

    /// The context the functions are called with.
    pub const fn context(self) -> *mut c_void {
        self.context
    }

    /// Allocates `size` bytes with alignment `align`.
    ///
    /// A zero `size` does not call into the allocator: it returns the address
    /// `align` as a dangling but aligned, non-null pointer, which is what a
    /// zero-sized element type needs. An invalid alignment, a size that cannot
    /// be addressed, or a size whose rounded form passes `isize::MAX` returns
    /// null.
    pub fn allocate(self, size: u64, align: u64) -> *mut u8 {
        if !is_valid_align(align) || !is_resizable(size, align) {
            return ptr::null_mut();
        }
        if size == 0 {
            return dangling_for(align);
        }
        // SAFETY: the contract of `allocate_fn` is checked by the caller of
        // `from_parts`, and the arguments were validated above.
        unsafe { (self.allocate_fn)(self.context, size, align) }
    }

    /// Allocates `size` bytes with alignment `align`, distinguishing a rejected
    /// layout from an exhausted allocator.
    pub fn try_allocate(self, size: u64, align: u64) -> crate::error::AbiResult<*mut u8> {
        use crate::error::{AbiError, AbiStatus};

        if !is_valid_align(align) || !is_resizable(size, align) {
            return Err(AbiError::new(AbiStatus::InvalidLayout));
        }
        let pointer = self.allocate(size, align);
        if pointer.is_null() {
            Err(AbiError::new(AbiStatus::OutOfMemory))
        } else {
            Ok(pointer)
        }
    }

    /// Releases a block obtained from [`AbiAllocator::allocate`] or
    /// [`AbiAllocator::reallocate`] on this same allocator.
    ///
    /// A zero `size`, a null `ptr` or an invalid alignment performs no call.
    ///
    /// # Safety
    ///
    /// `ptr` must be a live block produced by this allocator with exactly
    /// `size` and `align`, and it must not be used afterwards.
    pub unsafe fn deallocate(self, ptr: *mut u8, size: u64, align: u64) {
        if ptr.is_null() || size == 0 || !is_valid_align(align) || !is_addressable(size) {
            return;
        }
        // SAFETY: the arguments describe a block this allocator produced.
        unsafe { (self.deallocate_fn)(self.context, ptr, size, align) }
    }

    /// Resizes a block.
    ///
    /// `new_size` must be non-zero; a zero or otherwise invalid request returns
    /// null and leaves the block untouched. When `old_size` is zero this is an
    /// allocation, and the returned block must be released with the same
    /// `align`.
    ///
    /// # Safety
    ///
    /// `ptr` must be a live block produced by this allocator with exactly
    /// `old_size` and `align`. After a successful call the old pointer is
    /// invalid.
    pub unsafe fn reallocate(
        self,
        ptr: *mut u8,
        old_size: u64,
        align: u64,
        new_size: u64,
    ) -> *mut u8 {
        if !is_valid_align(align) || !is_resizable(new_size, align) || new_size == 0 {
            return ptr::null_mut();
        }
        if old_size == 0 {
            return self.allocate(new_size, align);
        }
        if ptr.is_null() || !is_addressable(old_size) {
            return ptr::null_mut();
        }
        // SAFETY: the arguments describe a block this allocator produced.
        unsafe { (self.reallocate_fn)(self.context, ptr, old_size, align, new_size) }
    }
}

impl fmt::Debug for AbiAllocator {
    // The addresses of the functions and the context describe the layout of the
    // process; printing them would put that into logs.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AbiAllocator { .. }")
    }
}

impl Default for AbiAllocator {
    fn default() -> Self {
        Self::system()
    }
}

/// Reports whether `align` can describe a real allocation.
fn is_valid_align(align: u64) -> bool {
    align.is_power_of_two() && align <= isize::MAX as u64
}

/// Reports whether `size` fits in an `isize` on the target.
fn is_addressable(size: u64) -> bool {
    size <= isize::MAX as u64
}

/// Reports whether a block of `size` bytes at `align` can be described at all.
///
/// A size is addressable on its own, but a block is described by a layout, and a
/// layout rounds its size up to the alignment. A size whose rounded form passes
/// `isize::MAX` is not a layout the allocator may be asked for, so the realloc
/// contract - which is the layout contract - refuses it here rather than leaving
/// the caller to guess.
fn is_resizable(size: u64, align: u64) -> bool {
    match size.checked_add(align - 1) {
        Some(rounded) => rounded <= isize::MAX as u64,
        None => false,
    }
}

/// The aligned dangling address used for zero-sized blocks.
fn dangling_for(align: u64) -> *mut u8 {
    // An address with no pointer behind it: a zero-sized request names no
    // block, so there is nothing for the value to be reached through.
    ptr::without_provenance_mut(align as usize)
}

/// The process allocator.
pub const SYSTEM_ALLOCATOR: AbiAllocator = AbiAllocator {
    context: ptr::null_mut(),
    allocate_fn: system_allocate,
    deallocate_fn: system_deallocate,
    reallocate_fn: system_reallocate,
};

unsafe extern "C" fn system_allocate(_context: *mut c_void, size: u64, align: u64) -> *mut u8 {
    if size == 0 || !is_valid_align(align) || !is_addressable(size) {
        return ptr::null_mut();
    }
    match core::alloc::Layout::from_size_align(size as usize, align as usize) {
        // SAFETY: the layout was validated above.
        Ok(layout) => unsafe { crate::heap::alloc::alloc(layout) },
        Err(_) => ptr::null_mut(),
    }
}

unsafe extern "C" fn system_deallocate(
    _context: *mut c_void,
    ptr: *mut u8,
    size: u64,
    align: u64,
) {
    if ptr.is_null() || size == 0 || !is_valid_align(align) || !is_addressable(size) {
        return;
    }
    if let Ok(layout) = core::alloc::Layout::from_size_align(size as usize, align as usize) {
        // SAFETY: the block came from the process allocator with this layout.
        unsafe { crate::heap::alloc::dealloc(ptr, layout) }
    }
}

unsafe extern "C" fn system_reallocate(
    _context: *mut c_void,
    ptr: *mut u8,
    old_size: u64,
    align: u64,
    new_size: u64,
) -> *mut u8 {
    if !is_valid_align(align) || new_size == 0 || !is_resizable(new_size, align) {
        return ptr::null_mut();
    }
    if old_size == 0 {
        // SAFETY: the arguments were validated above.
        return unsafe { system_allocate(ptr::null_mut(), new_size, align) };
    }
    if ptr.is_null() || !is_addressable(old_size) {
        return ptr::null_mut();
    }
    match core::alloc::Layout::from_size_align(old_size as usize, align as usize) {
        // SAFETY: the block came from the process allocator with this layout.
        Ok(layout) => unsafe { crate::heap::alloc::realloc(ptr, layout, new_size as usize) },
        Err(_) => ptr::null_mut(),
    }
}
