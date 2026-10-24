//! Overwriting memory that is about to be released.
//!
//! Freeing a block leaves its bytes readable by whoever allocates it next. The
//! functions here write zeros through a volatile pointer before a value lets go
//! of its storage, so the contents do not outlive the owner.

use core::ptr;

use crate::error::{AbiError, AbiResult, AbiStatus};

/// Overwrites `bytes` with zeros.
///
/// The writes are volatile, so the compiler does not remove them as dead stores
/// to memory that is about to be freed.
pub fn volatile_zero(bytes: &mut [u8]) {
    for byte in bytes.iter_mut() {
        // SAFETY: the pointer comes from a live mutable borrow.
        unsafe { ptr::write_volatile(byte, 0) };
    }
    // A fence keeps the writes from being reordered after the release that
    // follows.
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
}

/// Overwrites the storage of `values` with zeros.
///
/// A range whose byte count cannot be computed is reported rather than skipped:
/// a wipe that quietly does nothing is worse than one that says it could not
/// run.
///
/// # Safety
///
/// `values` must point at live, writable bytes of `length` elements of size
/// `element_size`.
pub unsafe fn volatile_zero_raw(
    values: *mut u8,
    length: u64,
    element_size: u64,
) -> AbiResult<()> {
    let bytes = match length.checked_mul(element_size) {
        Some(bytes) => bytes,
        None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
    };
    let mut offset: u64 = 0;
    // Whole words first, when the block is aligned for them: one volatile store
    // per word instead of one per byte.
    if (values as usize).is_multiple_of(core::mem::align_of::<usize>()) {
        while bytes - offset >= core::mem::size_of::<usize>() as u64 {
            // SAFETY: the caller guarantees the range is writable, and the
            // offset leaves a whole word inside it.
            unsafe { ptr::write_volatile(values.add(offset as usize).cast::<usize>(), 0) };
            offset += core::mem::size_of::<usize>() as u64;
        }
    }
    while offset < bytes {
        // SAFETY: the caller guarantees the range is writable.
        unsafe { ptr::write_volatile(values.add(offset as usize), 0) };
        offset += 1;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// A value whose contents can be overwritten before it is released.
///
/// The contract is about the *content*, not about the bytes: an implementation
/// overwrites what the value carries and leaves the value itself valid and
/// droppable, so wiping a container does not turn its header into garbage.
///
/// A container's own header - the address, the length, the capacity and the
/// allocator of a [`crate::StableString`] or [`crate::StableVec`] - is *not*
/// overwritten, because the value still has to release the block it names after
/// the wipe. What a container can wipe is the content, including the slots it no
/// longer covers; what it cannot wipe is the description of where that content
/// was. A value that must leave no trace of either has to be released by the
/// owner of the storage rather than wiped through this trait.
pub trait Wipe {
    /// Overwrites the content this value carries.
    fn wipe_in_place(&mut self);
}

/// Overwrites the bytes of one value with zeros, through volatile writes.
///
/// This is the body every implementation for plain data shares. A plain store
/// (`*self = false`) is a write the compiler may remove, because nothing reads
/// the value before it is released; the volatile form is the one that survives
/// being provably dead.
///
/// # Panics
///
/// Does not panic. A value whose byte count cannot be computed is left as it
/// was; no type this crate calls this for is large enough for that to happen.
pub fn wipe_value<T>(value: &mut T) {
    // SAFETY: `value` names one live, writable value of `T`.
    let _ = unsafe {
        volatile_zero_raw(
            (value as *mut T).cast::<u8>(),
            1,
            core::mem::size_of::<T>() as u64,
        )
    };
}

macro_rules! impl_wipe_for_plain_data {
    ($($type:ty),* $(,)?) => {
        $(
            impl Wipe for $type {
                fn wipe_in_place(&mut self) {
                    wipe_value(self);
                }
            }
        )*
    };
}

impl_wipe_for_plain_data!(
    u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64, (), bool, char
);
