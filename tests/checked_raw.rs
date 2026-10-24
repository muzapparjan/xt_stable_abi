//! Checking values that arrive as raw parts before they are trusted.

use xt_stable_abi::{AbiAllocator, AbiStatus, StableSlice, StableStr, StableVec};

#[test]
fn a_slice_length_that_fits_is_accepted() {
    let values = [10u32, 20, 30];
    // SAFETY: the pointer covers the buffer the length is checked against.
    let slice = unsafe {
        StableSlice::from_raw_parts_checked(values.as_ptr(), 3, std::mem::size_of_val(&values) as u64)
    }
    .unwrap();
    assert_eq!(slice.as_slice(), &values[..]);
}

#[test]
fn a_slice_length_that_exceeds_the_buffer_is_rejected() {
    let values = [10u32, 20, 30];
    // SAFETY: the pointer covers the buffer the length is checked against.
    let error = unsafe { StableSlice::from_raw_parts_checked(values.as_ptr(), 4, 12) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_string_length_that_fits_and_is_utf8_is_accepted() {
    let text = "héllo";
    // SAFETY: the pointer covers the buffer the length is checked against.
    let handle = unsafe {
        StableStr::from_raw_parts_checked(text.as_ptr(), text.len() as u64, text.len() as u64)
    }
    .unwrap();
    assert_eq!(handle.as_str(), text);
}

#[test]
fn a_string_length_that_exceeds_the_buffer_is_rejected() {
    let text = "abc";
    // SAFETY: the pointer covers the buffer the length is checked against.
    let error =
        unsafe { StableStr::from_raw_parts_checked(text.as_ptr(), 4, 3) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn bytes_that_are_not_utf8_are_rejected() {
    let bytes = [0x61u8, 0xFF, 0x62];
    // SAFETY: the pointer covers the buffer the length is checked against.
    let error =
        unsafe { StableStr::from_raw_parts_checked(bytes.as_ptr(), 3, 3) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidUtf8);
}

#[test]
fn a_vector_length_beyond_its_capacity_is_rejected() {
    let mut slot = 0u32;
    let pointer = &raw mut slot;
    // SAFETY: the pointer is aligned and names one slot; the length is rejected
    // before anything is read.
    let error = unsafe {
        StableVec::from_raw_parts(pointer, 5, 1, AbiAllocator::system())
    }
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn an_addressable_capacity_limit_is_enforced() {
    let mut slot = 0u32;
    let pointer = &raw mut slot;
    // SAFETY: the pointer is aligned and names one slot; the capacity is
    // rejected before anything is read.
    let error = unsafe {
        StableVec::from_raw_parts(pointer, 0, u64::MAX / 2, AbiAllocator::system())
    }
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::CapacityOverflow);
}

#[test]
fn an_explicit_capacity_is_raised_to_the_minimum_and_not_past_it() {
    for capacity in [1u64, 4, 5, 1000] {
        let vector: StableVec<u32> = StableVec::with_capacity(capacity).unwrap();
        // A vector that has never allocated takes at least `MINIMUM_CAPACITY`
        // slots, and above that number it takes exactly what was asked for: the
        // growth that rounds up belongs to later reservations, not to this one.
        assert_eq!(vector.capacity(), capacity.max(4));
        assert!(vector.is_empty());
    }
}

#[test]
fn later_growth_doubles_from_the_current_capacity() {
    let mut vector: StableVec<u32> = StableVec::with_capacity(1000).unwrap();
    for value in 0..1001u32 {
        vector.push(value);
    }
    assert_eq!(vector.len(), 1001);
    assert_eq!(vector.capacity(), 2000);
}
