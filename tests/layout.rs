//! The byte layout of the boundary types, checked against independent mirrors.
//!
//! Each mirror is a `#[repr(C)]` struct written from the documented field
//! order. Reading a live value through the mirror proves that the compiler
//! placed the fields where the contract says they are, which is the property the
//! other side of the boundary relies on.

use core::ffi::c_void;
use core::mem::{align_of, size_of};

use xt_stable_abi::{
    AbiAllocator, AbiError, AbiStatus, AbiVersion, Handle, HostServices, PluginDescriptor, ShutdownFn,
    StableBool, StableChar, StableDyn, StableOption, StableResult, StableStr, StableString,
    StableVec, VTablePrefix, ABI_VERSION,
};

/// The documented shape of [`AbiAllocator`]: a context and three pointers.
#[repr(C)]
struct AllocatorMirror {
    context: *mut c_void,
    allocate: usize,
    deallocate: usize,
    reallocate: usize,
}

/// The documented shape of [`StableStr`].
#[repr(C)]
struct StrMirror {
    pointer: *const u8,
    length: u64,
}

/// The documented shape of [`StableVec<u8>`].
#[repr(C)]
struct BytesMirror {
    pointer: *mut u8,
    length: u64,
    capacity: u64,
    allocator: AllocatorMirror,
}

/// The documented shape of [`StableDyn`].
#[repr(C)]
struct DynMirror {
    data: *mut u8,
    vtable: *const c_void,
    allocator: AllocatorMirror,
}

/// The documented shape of [`StableOption<u64>`].
#[repr(C)]
struct TaggedWordMirror {
    tag: u32,
    value: u64,
}

/// The documented shape of [`AbiError`].
#[repr(C)]
struct ErrorMirror {
    status: u32,
    message: BytesMirror,
}

/// The documented shape of [`PluginDescriptor`].
#[repr(C)]
struct HeaderMirror {
    capacity: u64,
    written: u64,
    major: u32,
    minor: u32,
    layout_revision: u64,
}

/// The documented shape of [`PluginDescriptor`].
#[repr(C)]
struct DescriptorMirror {
    header: HeaderMirror,
    name: BytesMirror,
    plugin_major: u32,
    plugin_minor: u32,
    api: DynMirror,
    allocator: AllocatorMirror,
    shutdown: ShutdownFn,
}

/// The documented shape of [`VTablePrefix`].
#[repr(C)]
struct PrefixMirror {
    type_id: u64,
    signature: u64,
    layout_revision: u64,
    size: u64,
    align: u64,
    slots: u64,
    drop: Option<unsafe extern "C" fn(*mut u8)>,
}

#[test]
fn scalar_wrappers_have_explicit_widths() {
    assert_eq!(size_of::<StableBool>(), 1);
    assert_eq!(align_of::<StableBool>(), 1);
    assert_eq!(size_of::<StableChar>(), 4);
    assert_eq!(size_of::<AbiStatus>(), 4);
    assert_eq!(size_of::<AbiVersion>(), 8);
    assert_eq!(size_of::<Handle>(), 8);
}

#[test]
fn the_allocator_is_a_context_and_three_pointers() {
    assert_eq!(size_of::<AbiAllocator>(), size_of::<AllocatorMirror>());
    assert_eq!(align_of::<AbiAllocator>(), align_of::<AllocatorMirror>());
    assert_eq!(size_of::<AbiAllocator>(), 4 * size_of::<usize>());
}

#[test]
fn a_borrowed_string_is_a_pointer_and_a_length() {
    assert_eq!(size_of::<StableStr>(), size_of::<StrMirror>());
    assert_eq!(align_of::<StableStr>(), align_of::<StrMirror>());

    let owned = StableString::from_str("hello").unwrap();
    let handle = owned.as_stable_str();
    // SAFETY: the mirror has the documented layout.
    let mirror = unsafe { &*(&handle as *const StableStr as *const StrMirror) };
    assert_eq!(mirror.length, 5);
    assert_eq!(mirror.pointer, owned.as_bytes().as_ptr());
}

#[test]
fn an_owned_string_is_a_byte_vector() {
    assert_eq!(size_of::<StableString>(), size_of::<StableVec<u8>>());
    assert_eq!(align_of::<StableString>(), align_of::<StableVec<u8>>());
}

#[test]
fn the_vector_marker_field_does_not_change_the_layout() {
    assert_eq!(size_of::<StableVec<u8>>(), size_of::<BytesMirror>());
    assert_eq!(align_of::<StableVec<u8>>(), align_of::<BytesMirror>());

    let mut vector = StableVec::new();
    vector.push(1u8);
    vector.push(2u8);
    // SAFETY: the mirror has the documented layout.
    let mirror = unsafe { &*(&vector as *const StableVec<u8> as *const BytesMirror) };
    assert_eq!(mirror.length, 2);
    assert_eq!(mirror.capacity, vector.capacity());
    assert_eq!(mirror.pointer, vector.as_slice().as_ptr().cast_mut());
    assert_eq!(
        mirror.allocator.context,
        vector.allocator().context()
    );
}

#[test]
fn an_optional_value_carries_an_explicit_word_tag() {
    assert_eq!(size_of::<StableOption<u64>>(), size_of::<TaggedWordMirror>());
    assert_eq!(align_of::<StableOption<u64>>(), align_of::<TaggedWordMirror>());

    let value = StableOption::some(0xDEAD_BEEFu64);
    // SAFETY: the mirror has the documented layout.
    let mirror = unsafe { &*(&value as *const StableOption<u64> as *const TaggedWordMirror) };
    assert_eq!(mirror.tag, 1);
    assert_eq!(mirror.value, 0xDEAD_BEEF);

    // A plain `Option<u8>` fits the empty variant into a niche of the payload,
    // so it is two bytes; the tagged form cannot.
    assert_eq!(size_of::<Option<u8>>(), 2);
    assert!(size_of::<StableOption<u8>>() >= 5);
}

#[test]
fn a_result_shares_one_word_of_storage_between_variants() {
    assert_eq!(
        size_of::<StableResult<u64, u64>>(),
        size_of::<TaggedWordMirror>()
    );
    assert_eq!(
        align_of::<StableResult<u64, u64>>(),
        align_of::<TaggedWordMirror>()
    );

    let success = StableResult::<u64, u64>::ok(11);
    let failure = StableResult::<u64, u64>::err(22);
    // SAFETY: the mirror has the documented layout.
    let success_mirror =
        unsafe { &*(&success as *const StableResult<u64, u64> as *const TaggedWordMirror) };
    // SAFETY: the mirror has the documented layout.
    let failure_mirror =
        unsafe { &*(&failure as *const StableResult<u64, u64> as *const TaggedWordMirror) };
    assert_eq!(success_mirror.tag, 0);
    assert_eq!(success_mirror.value, 11);
    assert_eq!(failure_mirror.tag, 1);
    assert_eq!(failure_mirror.value, 22);
}

#[test]
fn an_error_starts_with_its_code_and_carries_its_message() {
    assert_eq!(size_of::<AbiError>(), size_of::<ErrorMirror>());
    assert_eq!(align_of::<AbiError>(), align_of::<ErrorMirror>());

    let error = AbiError::with_message(AbiStatus::InvalidArgument, "why");
    // SAFETY: the mirror has the documented layout.
    let mirror = unsafe { &*(&error as *const AbiError as *const ErrorMirror) };
    assert_eq!(mirror.status, AbiStatus::InvalidArgument.code());
    assert_eq!(mirror.message.length, 3);
    // SAFETY: the message owns three initialised bytes.
    let bytes = unsafe {
        core::slice::from_raw_parts(mirror.message.pointer, mirror.message.length as usize)
    };
    assert_eq!(bytes, b"why");
}

#[test]
fn a_table_header_records_the_declaration_it_was_built_from() {
    assert_eq!(size_of::<VTablePrefix>(), size_of::<PrefixMirror>());
    assert_eq!(align_of::<VTablePrefix>(), align_of::<PrefixMirror>());

    static DROPS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

    unsafe extern "C" fn drop_nothing(_data: *mut u8) {
        DROPS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    }

    let prefix = VTablePrefix {
        type_id: 5,
        signature: 6,
        layout_revision: 7,
        size: 8,
        align: 9,
        slots: 1,
        drop: Some(drop_nothing),
    };
    // SAFETY: the mirror has the documented layout.
    let mirror = unsafe { &*(&prefix as *const VTablePrefix as *const PrefixMirror) };
    assert_eq!(mirror.type_id, 5);
    assert_eq!(mirror.signature, 6);
    assert_eq!(mirror.layout_revision, 7);
    assert_eq!(mirror.size, 8);
    assert_eq!(mirror.align, 9);
    assert_eq!(mirror.slots, 1);
    // Calling through the mirrored field reaches the function the table was
    // built with, which is what a receiver does with it.
    // SAFETY: the field holds `drop_nothing`, which ignores its argument.
    unsafe { (mirror.drop.unwrap())(core::ptr::null_mut()) };
    assert_eq!(DROPS.load(core::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn the_descriptor_starts_with_its_header() {
    assert_eq!(size_of::<PluginDescriptor>(), size_of::<DescriptorMirror>());
    assert_eq!(
        align_of::<PluginDescriptor>(),
        align_of::<DescriptorMirror>()
    );
    assert_eq!(size_of::<StableDyn>(), size_of::<DynMirror>());

    let descriptor = PluginDescriptor::buffer();
    // SAFETY: the mirror has the documented layout.
    let mirror =
        unsafe { &*(&descriptor as *const PluginDescriptor as *const DescriptorMirror) };
    assert_eq!(mirror.header.capacity, size_of::<PluginDescriptor>() as u64);
    assert_eq!(mirror.header.written, 0);
    assert_eq!(mirror.header.major, 1);
    assert_eq!(mirror.header.minor, 0);
    assert_eq!(mirror.name.length, 0);
    assert!(mirror.api.vtable.is_null());
    assert!(mirror.allocator.context.is_null());
}

/// The documented shape of [`HostServices`].
#[repr(C)]
struct HostMirror {
    size: u64,
    major: u32,
    minor: u32,
    allocator: AllocatorMirror,
    log: usize,
    context: *mut c_void,
    query: usize,
    layout_revision: u64,
}

#[test]
fn the_host_services_start_with_their_size() {
    assert_eq!(size_of::<HostServices>(), size_of::<HostMirror>());
    assert_eq!(align_of::<HostServices>(), align_of::<HostMirror>());

    let services = HostServices::new(AbiAllocator::system());
    // SAFETY: the mirror has the documented layout.
    let mirror = unsafe { &*(&services as *const HostServices as *const HostMirror) };
    // A library reads this field before any other, so it sits first and states
    // how far the value it was given extends.
    assert_eq!(mirror.size, size_of::<HostServices>() as u64);
    assert_eq!(mirror.major, ABI_VERSION.major);
    assert_eq!(mirror.minor, ABI_VERSION.minor);
    // The size says how many bytes there are and the version says what the
    // protocol means; this says where the fields sit. A host that reordered
    // them would keep both of the others.
    assert_eq!(
        mirror.layout_revision,
        xt_stable_abi::object::LAYOUT_REVISION
    );
}

#[test]
fn an_emptied_option_reports_the_empty_variant_and_keeps_its_bytes() {
    let empty = StableOption::<u64>::none();
    let mut filled = StableOption::some(0xAAAA_AAAA_AAAA_AAAAu64);
    assert_eq!(filled.take(), Some(0xAAAA_AAAA_AAAA_AAAA));
    assert!(filled.is_none());

    // SAFETY: both values have the documented layout.
    let empty = unsafe { &*(&empty as *const StableOption<u64> as *const TaggedWordMirror) };
    // SAFETY: the same layout describes the emptied value.
    let filled = unsafe { &*(&filled as *const StableOption<u64> as *const TaggedWordMirror) };
    assert_eq!(empty.tag, filled.tag);
    // A value built by `none` writes zeros over the slot, so a fresh empty value
    // does not carry the caller's stack contents. Emptying a filled one only
    // changes the tag: the bytes stay until `wipe` overwrites them, the same
    // policy `truncate` follows on a vector.
    assert_eq!(empty.value, 0);
    assert_eq!(filled.value, 0xAAAA_AAAA_AAAA_AAAA);
}

