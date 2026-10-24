//! The allocator handle: what it forwards, what it refuses, and what it keeps.

mod support;

use core::ffi::c_void;
use std::sync::atomic::Ordering;

use support::{tracked_allocator, Tracker};
use xt_stable_abi::alloc::SYSTEM_ALLOCATOR;
use xt_stable_abi::{AbiAllocator, AbiStatus, StableString, StableVec};

#[test]
fn a_block_survives_a_roundtrip_through_the_process_allocator() {
    let block = SYSTEM_ALLOCATOR.allocate(64, 8);
    assert!(!block.is_null());
    assert_eq!(block as usize % 8, 0);
    // SAFETY: the block holds 64 bytes.
    unsafe { block.write_bytes(0x5A, 64) };
    for index in 0..64 {
        // SAFETY: the index is inside the block.
        assert_eq!(unsafe { *block.add(index) }, 0x5A);
    }
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.deallocate(block, 64, 8) };
}

#[test]
fn a_zero_sized_request_does_not_reach_a_foreign_allocator() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);

    let mut zero_sized: StableVec<()> = StableVec::new_with_allocator(allocator);
    zero_sized.push(());
    zero_sized.push(());

    let empty_text = StableString::from_str_with_allocator(allocator, "").unwrap();
    assert!(empty_text.is_empty());

    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 0);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 0);

    drop(empty_text);
    drop(zero_sized);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 0);
}

#[test]
fn an_invalid_layout_returns_null_without_calling_the_allocator() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);

    assert!(allocator.allocate(16, 3).is_null());
    assert!(allocator.allocate(16, 0).is_null());
    assert!(allocator.allocate(u64::MAX, 8).is_null());
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_rejected_layout_is_distinguished_from_exhaustion() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);

    let error = allocator.try_allocate(16, 3).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidLayout);
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 0);

    let error = support::refusing_allocator().try_allocate(16, 4).unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
}

#[test]
fn the_context_is_preserved() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    assert_eq!(
        allocator.context(),
        tracker as *const Tracker as *mut c_void
    );
}

#[test]
fn the_default_allocator_is_the_process_allocator() {
    let allocator = AbiAllocator::default();
    assert!(allocator.context().is_null());
    let block = allocator.allocate(8, 8);
    assert!(!block.is_null());
    // SAFETY: the block came from this allocator with this layout.
    unsafe { allocator.deallocate(block, 8, 8) };
}

#[test]
fn growth_reuses_the_block_through_reallocation() {
    let tracker = Tracker::new();
    let mut vector = StableVec::new_with_allocator(tracked_allocator(tracker));
    for value in 0..200u32 {
        vector.push(value);
    }
    assert_eq!(vector.len(), 200);
    assert!(tracker.reallocations.load(Ordering::SeqCst) >= 1);
    assert_eq!(tracker.outstanding(), 1);
    drop(vector);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn every_owned_value_records_the_allocator_that_made_it() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let text = StableString::from_str_with_allocator(allocator, "recorded").unwrap();
    assert_eq!(text.allocator().context(), allocator.context());
    assert_eq!(tracker.outstanding(), 1);
    drop(text);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn system_allocate_roundtrip() {
    let allocator = AbiAllocator::system();
    let block = allocator.allocate(64, 8);
    assert!(!block.is_null());
    assert_eq!(block as usize % 8, 0);
    unsafe { block.write_bytes(0xA5, 64) };
    assert_eq!(unsafe { *block }, 0xA5);
    // SAFETY: the block came from this allocator with this layout.
    unsafe { allocator.deallocate(block, 64, 8) };
}

#[test]
fn zero_sized_allocation_is_aligned_and_non_null() {
    let allocator = AbiAllocator::system();
    for align in [1u64, 2, 8, 64] {
        let block = allocator.allocate(0, align);
        assert!(!block.is_null());
        assert_eq!(block as usize % align as usize, 0);
        // SAFETY: the block came from this allocator with this layout.
        unsafe { allocator.deallocate(block, 0, align) };
    }
}

#[test]
fn invalid_alignment_yields_null() {
    let allocator = AbiAllocator::system();
    assert!(allocator.allocate(16, 3).is_null());
    assert!(allocator.allocate(16, 0).is_null());
}

#[test]
fn reallocate_preserves_contents() {
    let allocator = AbiAllocator::system();
    let block = allocator.allocate(16, 4);
    assert!(!block.is_null());
    unsafe { block.write_bytes(0x11, 16) };
    // SAFETY: the block came from this allocator with this layout.
    let grown = unsafe { allocator.reallocate(block, 16, 4, 128) };
    assert!(!grown.is_null());
    for index in 0..16 {
        assert_eq!(unsafe { *grown.add(index) }, 0x11);
    }
    // SAFETY: the grown block came from this allocator with this layout.
    unsafe { allocator.deallocate(grown, 128, 4) };
}

#[test]
fn reallocate_with_zero_new_size_leaves_the_block_alone() {
    let allocator = AbiAllocator::system();
    let block = allocator.allocate(16, 4);
    // SAFETY: the block came from this allocator with this layout.
    assert!(unsafe { allocator.reallocate(block, 16, 4, 0) }.is_null());
    // The block must still be writable; releasing it here proves the original
    // allocation was not disturbed.
    unsafe { block.write_bytes(0x22, 16) };
    // SAFETY: the block came from this allocator with this layout.
    unsafe { allocator.deallocate(block, 16, 4) };
}

#[test]
fn reallocate_with_zero_old_size_allocates() {
    let allocator = AbiAllocator::system();
    // SAFETY: a zero old size makes this an allocation.
    let block = unsafe { allocator.reallocate(core::ptr::null_mut(), 0, 8, 32) };
    assert!(!block.is_null());
    // SAFETY: the block holds 32 bytes.
    unsafe { block.write_bytes(0x33, 32) };
    // SAFETY: the block came from this allocator with this layout.
    unsafe { allocator.deallocate(block, 32, 8) };
}

#[test]
fn a_foreign_context_reaches_every_call() {
    static mut SEEN: [usize; 2] = [0, 0];

    unsafe extern "C" fn allocate(context: *mut c_void, size: u64, align: u64) -> *mut u8 {
        unsafe { SEEN[0] = context as usize };
        SYSTEM_ALLOCATOR.allocate(size, align)
    }

    unsafe extern "C" fn deallocate(context: *mut c_void, ptr: *mut u8, size: u64, align: u64) {
        unsafe { SEEN[1] = context as usize };
        // SAFETY: the pointer came from the process allocator.
        unsafe { SYSTEM_ALLOCATOR.deallocate(ptr, size, align) };
    }

    unsafe extern "C" fn reallocate(
        _context: *mut c_void,
        ptr: *mut u8,
        old_size: u64,
        align: u64,
        new_size: u64,
    ) -> *mut u8 {
        // SAFETY: the pointer came from the process allocator.
        unsafe { SYSTEM_ALLOCATOR.reallocate(ptr, old_size, align, new_size) }
    }

    let marker = 0x5A5Au16;
    let context = &marker as *const u16 as *mut c_void;
    let allocator =
        unsafe { AbiAllocator::from_parts(context, allocate, deallocate, reallocate) };
    assert_eq!(allocator.context(), context);

    let block = allocator.allocate(8, 1);
    assert_eq!(unsafe { SEEN[0] }, context as usize);
    // SAFETY: the block came from this allocator with this layout.
    unsafe { allocator.deallocate(block, 8, 1) };
    assert_eq!(unsafe { SEEN[1] }, context as usize);
}

#[test]
fn removing_text_takes_one_block_and_keeps_the_one_the_value_owns() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let mut text = StableString::from_str_with_allocator(allocator, "abcdef").unwrap();
    let before = tracker.allocations.load(Ordering::SeqCst);

    let removed = text.drain(1, 3).unwrap();
    assert_eq!(removed.as_str(), "bc");
    assert_eq!(text.as_str(), "adef");
    // The removed text is the only block the call asks for: the rest of the
    // value moves down inside the block it already owns.
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), before + 1);
    assert_eq!(tracker.reallocations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_layout_whose_rounding_leaves_the_address_space_is_refused() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);

    // A size that is addressable on its own but cannot be rounded up to this
    // alignment inside `isize::MAX` is not a layout the allocator may be asked
    // for. The reported form names the layout; the plain form answers null.
    let too_large = (isize::MAX as u64) - 3;
    assert_eq!(
        allocator
            .try_allocate(too_large, 8)
            .unwrap_err()
            .status(),
        AbiStatus::InvalidLayout
    );
    assert!(allocator.allocate(too_large, 8).is_null());
    // Neither call reached the allocator behind the handle, so a custom
    // allocator is never handed a layout it may not accept.
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_resize_whose_rounding_leaves_the_address_space_is_refused() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let block = allocator.allocate(8, 8);
    assert!(!block.is_null());

    // A block is described by a layout, and a layout rounds its size up to the
    // alignment. This size is addressable on its own yet cannot be rounded to
    // eight without passing `isize::MAX`, so it is not a request the allocator
    // may be handed.
    let too_large = (isize::MAX as u64) - 3;
    // SAFETY: the block came from this allocator with this layout.
    let resized = unsafe { allocator.reallocate(block, 8, 8, too_large) };
    assert!(resized.is_null());
    // Nothing reached the allocator behind the handle, so the block is the one
    // that was just made.
    assert_eq!(tracker.reallocations.load(Ordering::SeqCst), 0);

    // SAFETY: the block is still the one this allocator produced, untouched.
    unsafe { allocator.deallocate(block, 8, 8) };
}
