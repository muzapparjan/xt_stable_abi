//! Malformed values at the boundary.
//!
//! Every case below is a fixed input, not a search: a descriptor that claims
//! more bytes than the buffer, a tag outside the set the type defines, a length
//! that would read past the storage it describes. The point is that each one
//! produces a status instead of a read the receiver cannot justify.

use std::mem::{size_of, MaybeUninit};
use std::sync::atomic::{AtomicUsize, Ordering};

use xt_stable_abi::object::LAYOUT_REVISION;
use xt_stable_abi::{
    load, AbiHeader, AbiStatus, AbiVersion, EntryPointFn, HostServices, PluginDescriptor,
    StableOption, StableResult, StableSlice, StableStr, StableString, StableVec, ABI_VERSION,
    MIN_DESCRIPTOR_SIZE,
};

const HOST: HostServices = HostServices::new(xt_stable_abi::SYSTEM_ALLOCATOR);

static HOOK_RUNS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn count_hook() {
    HOOK_RUNS.fetch_add(1, Ordering::SeqCst);
}

/// Writes a descriptor that claims more bytes than the buffer holds.
unsafe extern "C" fn claims_too_much(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: `load` provides a valid buffer.
    unsafe {
        let capacity = (*out).header.capacity;
        (*out).header = AbiHeader::new(capacity, capacity + 1, ABI_VERSION, LAYOUT_REVISION);
    }
    AbiStatus::Ok.code()
}

/// Writes a descriptor that claims fewer bytes than the smallest layout.
unsafe extern "C" fn claims_nothing(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: `load` provides a valid buffer.
    unsafe {
        let capacity = (*out).header.capacity;
        (*out).header = AbiHeader::new(capacity, 0, ABI_VERSION, LAYOUT_REVISION);
    }
    AbiStatus::Ok.code()
}

/// Leaves the object field empty.
unsafe extern "C" fn omits_the_object(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: `load` provides a valid buffer.
    unsafe {
        let capacity = (*out).header.capacity;
        (*out).header = AbiHeader::new(capacity, MIN_DESCRIPTOR_SIZE, ABI_VERSION, LAYOUT_REVISION);
    }
    AbiStatus::Ok.code()
}

/// Records a layout revision this host does not implement.
unsafe extern "C" fn newer_layout(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: `load` provides a valid buffer.
    unsafe {
        let capacity = (*out).header.capacity;
        (*out).header = AbiHeader::new(
            capacity,
            MIN_DESCRIPTOR_SIZE,
            AbiVersion::new(2, 0),
            LAYOUT_REVISION,
        );
        (*out).shutdown = Some(count_hook);
    }
    AbiStatus::Ok.code()
}

/// Reports a failure.
unsafe extern "C" fn refuses(
    _host: *const HostServices,
    _required: AbiVersion,
    _out: *mut PluginDescriptor,
) -> u32 {
    AbiStatus::InvalidArgument.code()
}

/// Reports a failure with a code this host does not define.
unsafe extern "C" fn refuses_with_a_foreign_code(
    _host: *const HostServices,
    _required: AbiVersion,
    _out: *mut PluginDescriptor,
) -> u32 {
    4242
}

/// Records a boundary layout revision this host does not implement.
unsafe extern "C" fn foreign_layout(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: `load` provides a valid buffer.
    unsafe {
        let capacity = (*out).header.capacity;
        (*out).header = AbiHeader::new(
            capacity,
            MIN_DESCRIPTOR_SIZE,
            ABI_VERSION,
            LAYOUT_REVISION + 1,
        );
        (*out).shutdown = Some(count_hook);
    }
    AbiStatus::Ok.code()
}

/// Writes a name whose bytes cannot be decoded as text.
unsafe extern "C" fn names_itself_with_bytes_that_are_not_text(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: `load` provides a valid buffer.
    unsafe {
        let capacity = (*out).header.capacity;
        let mut bytes = StableVec::<u8>::new();
        for byte in [0xffu8, 0xfe, 0x41] {
            if bytes.try_push(byte).is_err() {
                return AbiStatus::CapacityOverflow.code();
            }
        }
        (*out).name = StableString::from_utf8_unchecked(bytes);
        (*out).header =
            AbiHeader::new(capacity, MIN_DESCRIPTOR_SIZE, ABI_VERSION, LAYOUT_REVISION);
        (*out).shutdown = Some(count_hook);
    }
    AbiStatus::Ok.code()
}

#[test]
fn a_descriptor_from_another_layout_revision_is_rejected_without_running_its_hook() {
    HOOK_RUNS.store(0, Ordering::SeqCst);
    let error = unsafe { load(foreign_layout, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);
    // The fields may sit at other offsets, so neither the hook nor the release
    // may run: both would read a layout this build does not implement.
    assert_eq!(HOOK_RUNS.load(Ordering::SeqCst), 0);
}

#[test]
fn a_descriptor_from_another_major_version_does_not_run_its_hook() {
    HOOK_RUNS.store(0, Ordering::SeqCst);
    let error = unsafe { load(newer_layout, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);
    assert_eq!(HOOK_RUNS.load(Ordering::SeqCst), 0);
}

#[test]
fn a_name_that_is_not_text_is_rejected() {
    HOOK_RUNS.store(0, Ordering::SeqCst);
    let error = unsafe { load(names_itself_with_bytes_that_are_not_text, &HOST, ABI_VERSION) }
        .unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidUtf8);
    // The descriptor itself is otherwise sound, so the hook runs; the name is
    // released on the side that allocated it when the descriptor is dropped.
    assert_eq!(HOOK_RUNS.load(Ordering::SeqCst), 1);
}

#[test]
fn an_error_whose_message_is_not_text_is_refused() {
    use xt_stable_abi::AbiType;

    let mut bytes = StableVec::<u8>::new();
    for byte in [0xffu8, 0xfe] {
        bytes.try_push(byte).unwrap();
    }
    let error = xt_stable_abi::AbiError::with_owned_message(
        AbiStatus::InvalidArgument,
        // SAFETY: the bytes are the value under test: the point is that nothing
        // checked them before the error was built.
        unsafe { StableString::from_utf8_unchecked(bytes) },
    );
    // The message is read as text by `message()` and by `Display`, so it passes
    // the same check a returned string does.
    assert_eq!(
        error.validate().unwrap_err().status(),
        AbiStatus::InvalidUtf8
    );
}

#[test]
fn a_descriptor_that_overstates_its_length_is_rejected() {
    let error = unsafe { load(claims_too_much, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_descriptor_that_understates_its_length_is_rejected() {
    let error = unsafe { load(claims_nothing, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_descriptor_without_an_object_is_rejected() {
    let error = unsafe { load(omits_the_object, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_descriptor_from_a_newer_layout_is_rejected() {
    let error = unsafe { load(newer_layout, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);
}

#[test]
fn a_status_from_the_library_reaches_the_caller() {
    let error = unsafe { load(refuses, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
}

#[test]
fn a_status_this_host_does_not_define_keeps_its_code() {
    let error = unsafe { load(refuses_with_a_foreign_code, &HOST, ABI_VERSION) }.unwrap_err();
    // The library named a failure this build has no name for. It is still a
    // failure, and the number is what says which one: nothing here replaces it
    // with a status this build happens to know.
    assert_eq!(error.status().code(), 4242);
    assert_eq!(error.raw_status(), 4242);
    assert!(AbiStatus::from_code(4242).is_none());
    assert!(!error.status().is_ok());
}

#[test]
fn a_borrowed_string_longer_than_its_buffer_is_rejected() {
    let bytes = *b"four";
    let result = unsafe { StableStr::from_raw_parts_checked(bytes.as_ptr(), 9, 4) };
    assert_eq!(result.unwrap_err().status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn borrowed_bytes_that_are_not_text_are_rejected() {
    let bytes = [0xffu8, 0xfe, 0xfd];
    let result = unsafe { StableStr::from_raw_parts_checked(bytes.as_ptr(), 3, 3) };
    assert_eq!(result.unwrap_err().status(), AbiStatus::InvalidUtf8);
}

#[test]
fn a_slice_length_whose_byte_count_would_overflow_is_rejected() {
    let values = [0u32, 1, 2];
    let result =
        unsafe { StableSlice::<u32>::from_raw_parts_checked(values.as_ptr(), u64::MAX, 12) };
    assert_eq!(result.unwrap_err().status(), AbiStatus::CapacityOverflow);
}

#[test]
fn a_slice_length_past_the_storage_is_rejected() {
    let values = [0u32, 1, 2];
    let result = unsafe { StableSlice::<u32>::from_raw_parts_checked(values.as_ptr(), 4, 12) };
    assert_eq!(result.unwrap_err().status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn an_option_tag_outside_the_defined_set_is_rejected() {
    let value = MaybeUninit::<u64>::new(5);
    let result = unsafe { StableOption::<u64>::from_parts(7, value) };
    assert_eq!(result.unwrap_err().status(), AbiStatus::InvalidTag);
}

#[test]
fn a_result_tag_outside_the_defined_set_is_rejected() {
    let value = xt_stable_abi::result::StableResultValue {
        ok: std::mem::ManuallyDrop::new(5u64),
    };
    // SAFETY: the union field that is read back is the one that was written;
    // the point of the test is that the tag says otherwise.
    let result = unsafe { StableResult::<u64, u64>::from_parts(9, value) };
    assert_eq!(result.unwrap_err().status(), AbiStatus::InvalidTag);
}

#[test]
fn an_unknown_status_code_is_not_guessed() {
    assert_eq!(AbiStatus::from_code(u32::MAX), None);
    assert_eq!(AbiStatus::from_code(12), None);
    assert_eq!(AbiStatus::from_code(11), Some(AbiStatus::Panic));
}

#[test]
fn a_descriptor_buffer_smaller_than_the_layout_is_refused() {
    let mut buffer = PluginDescriptor::buffer();
    buffer.header.capacity = 8;
    let status = unsafe {
        PluginDescriptor::store(
            &mut buffer,
            "too_small",
            AbiVersion::new(0, 1),
            xt_stable_abi::StableDyn::empty(),
            None,
        )
    };
    assert_eq!(status, AbiStatus::InvalidDescriptor);
    assert_eq!(buffer.name_str(), "");
}

#[test]
fn writing_a_descriptor_into_null_is_refused() {
    let status = unsafe {
        PluginDescriptor::store(
            core::ptr::null_mut(),
            "nowhere",
            AbiVersion::new(0, 1),
            xt_stable_abi::StableDyn::empty(),
            None,
        )
    };
    assert_eq!(status, AbiStatus::NullPointer);
}

#[test]
fn a_library_that_writes_more_than_the_buffer_is_not_trusted() {
    // The same input as the first test, driven through the entry point type
    // rather than the helper, so the check is pinned where a host would use it.
    let entry: EntryPointFn = claims_too_much;
    let error = unsafe { load(entry, &HOST, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_slice_built_from_forged_parts_is_not_read_past_its_declared_length() {
    // A vector handle that claims four elements over storage for two is a
    // forged input; the checked constructor is the place that refuses it.
    let values = [1u64, 2];
    let result = unsafe { StableSlice::<u64>::from_raw_parts_checked(values.as_ptr(), 4, 16) };
    assert_eq!(result.unwrap_err().status(), AbiStatus::InvalidDescriptor);
    assert_eq!(size_of::<u64>(), 8);
}
