//! The entry protocol: a library reports itself and hands over an object.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};

use support::{refusing_allocator, tracked_allocator, Tracker};
use xt_stable_abi::object::{allocate_value, type_id_of, VTablePrefix};
use xt_stable_abi::{
    load, stable_object, AbiAllocator, AbiHeader, AbiLogLevel, AbiStatus, AbiVersion, EntryPointFn,
    HostServices, LoadedPlugin, PluginDescriptor, StableDyn, StableStr, StableString, ABI_VERSION,
    MIN_DESCRIPTOR_SIZE,
};

stable_object! {
    /// A handle to the greeting service a library publishes.
    pub struct GreeterObject;
    /// The table behind [`GreeterObject`].
    pub struct GreeterVTable;
    /// Greets callers.
    pub trait Greeter {
        /// Greets `name`.
        fn greet(&self, name: StableStr) -> StableString;
        /// The number of greetings produced so far.
        fn produced(&self) -> u64;
    }
}

/// The implementation published by the sample library.
struct Friendly {
    produced: u64,
}

impl Greeter for Friendly {
    fn greet(&self, name: StableStr<'_>) -> StableString {
        let name = name.as_str();
        StableString::from_str(&format!("hello {name}, welcome")).unwrap()
    }

    fn produced(&self) -> u64 {
        self.produced
    }
}

/// The function the sample library exports.
unsafe extern "C" fn library_entry(
    host: *const HostServices,
    required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    if host.is_null() || out.is_null() {
        return AbiStatus::NullPointer.code();
    }
    if !ABI_VERSION.serves(required) {
        return AbiStatus::VersionMismatch.code();
    }
    // SAFETY: the host guarantees a readable value for the call.
    let host = unsafe { &*host };
    host.log(AbiLogLevel::Info, "greeter: initialising");

    let object = match GreeterObject::abi_try_from_value_with(
        Friendly { produced: 1 },
        type_id_of("Greeter"),
        host.allocator,
    ) {
        Ok(object) => object,
        Err(error) => return error.status().code(),
    };

    // SAFETY: the host provides a writable descriptor buffer.
    unsafe {
        PluginDescriptor::store(
            out,
            "greeter",
            AbiVersion::new(0, 1),
            object.abi_into_stable(),
            None,
        )
        .code()
    }
}

static LOGS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn counting_log(
    _context: *mut core::ffi::c_void,
    level: u32,
    message: StableStr<'_>,
) {
    assert_eq!(AbiLogLevel::from_code(level), Some(AbiLogLevel::Info));
    assert_eq!(message.as_str(), "greeter: initialising");
    LOGS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn the_protocol_delivers_a_working_object() {
    let mut host = HostServices::new(AbiAllocator::system());
    host.log = Some(counting_log);

    let descriptor = unsafe { load(library_entry as EntryPointFn, &host, ABI_VERSION) }.unwrap();
    assert_eq!(descriptor.name_str(), "greeter");
    assert_eq!(descriptor.header.version, ABI_VERSION);
    assert_eq!(descriptor.plugin_version, AbiVersion::new(0, 1));
    assert_eq!(descriptor.api.type_id(), type_id_of("Greeter"));
    assert_eq!(LOGS.load(Ordering::SeqCst), 1);

    let object = GreeterObject::abi_from_stable(descriptor.api);
    let greeting = unsafe { object.greet(StableStr::from_str("Ada")) };
    assert_eq!(greeting.as_str(), "hello Ada, welcome");
    assert_eq!(unsafe { object.produced() }, 1);
}

#[test]
fn a_required_version_the_library_cannot_serve_is_refused() {
    let host = HostServices::new(AbiAllocator::system());
    let error = unsafe {
        load(
            library_entry as EntryPointFn,
            &host,
            AbiVersion::new(ABI_VERSION.major + 1, 0),
        )
    }
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);
}

#[test]
fn the_descriptor_releases_the_object_and_the_name_through_the_host_allocator() {
    let tracker = Tracker::new();
    let host = HostServices::new(tracked_allocator(tracker));
    let descriptor = unsafe { load(library_entry as EntryPointFn, &host, ABI_VERSION) }.unwrap();
    // One block for the object and one for the name, both from the host.
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 2);
    assert_eq!(tracker.outstanding(), 2);

    drop(descriptor);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 2);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_failing_host_allocator_surfaces_as_a_call_failure() {
    let host = HostServices::new(refusing_allocator());
    let error = unsafe { load(library_entry as EntryPointFn, &host, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
}

#[test]
fn a_buffer_written_against_another_revision_is_left_alone() {
    let mut buffer = PluginDescriptor::buffer();
    buffer.header.layout_revision = buffer.header.layout_revision.wrapping_add(1);
    let revision = buffer.header.layout_revision;
    let object = GreeterObject::abi_try_from_value_with(
        Friendly { produced: 2 },
        type_id_of("Greeter"),
        AbiAllocator::system(),
    )
    .unwrap();
    let status = unsafe {
        PluginDescriptor::store(
            &mut buffer,
            "greeter",
            AbiVersion::new(0, 1),
            object.abi_into_stable(),
            None,
        )
    };
    assert_eq!(status, AbiStatus::VersionMismatch);
    // Nothing was released or overwritten: the fields are the ones the buffer
    // was built with, including the revision that caused the refusal.
    assert_eq!(buffer.header.layout_revision, revision);
    assert_eq!(buffer.header.written, 0);
    assert!(buffer.name.is_empty());
}

#[test]
fn the_buffer_size_reported_by_the_library_is_the_one_it_was_given() {
    let host = HostServices::new(AbiAllocator::system());
    let descriptor = unsafe { load(library_entry as EntryPointFn, &host, ABI_VERSION) }.unwrap();
    assert_eq!(
        descriptor.header.written,
        core::mem::size_of::<PluginDescriptor>() as u64
    );
}

unsafe extern "C" fn drop_u32(data: *mut u8) {
    // SAFETY: the table describes a `u32`.
    unsafe { core::ptr::drop_in_place(data.cast::<u32>()) }
}

static SAMPLE_TABLE: VTablePrefix = VTablePrefix {
    type_id: 9,
    signature: 1,
    layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
    size: 4,
    align: 4,
    slots: 0,
    drop: Some(drop_u32),
};

/// Builds a `StableDyn` that carries the number `7`.
fn sample_object() -> StableDyn {
    let allocator = AbiAllocator::system();
    let data = unsafe { allocate_value(7u32, allocator) }.unwrap();
    // SAFETY: the storage holds a `u32` and the table describes one.
    unsafe { StableDyn::from_raw_parts(data, &SAMPLE_TABLE, allocator) }
}

unsafe extern "C" fn good_entry(
    host: *const HostServices,
    required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    if host.is_null() {
        return AbiStatus::NullPointer.code();
    }
    if !ABI_VERSION.serves(required) {
        return AbiStatus::VersionMismatch.code();
    }
    // SAFETY: the host provides a readable value for the call.
    let host = unsafe { &*host };
    host.log(AbiLogLevel::Info, "entering");
    // SAFETY: the host provides a writable buffer.
    unsafe {
        PluginDescriptor::store(
            out,
            "sample",
            AbiVersion::new(0, 4),
            sample_object(),
            None,
        )
        .code()
    }
}

unsafe extern "C" fn null_host_entry(
    _host: *const HostServices,
    _required: AbiVersion,
    _out: *mut PluginDescriptor,
) -> u32 {
    AbiStatus::NullPointer.code()
}

/// A library that reports success without looking at what was required of it.
unsafe extern "C" fn deaf_entry(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: the host provides a writable buffer.
    unsafe {
        PluginDescriptor::store(out, "deaf", AbiVersion::new(1, 0), sample_object(), None).code()
    }
}

#[test]
fn a_library_that_implements_less_than_the_host_requires_is_refused() {
    let host = HostServices::new(AbiAllocator::system());
    // The library reports version 1.0 whatever it is asked for. A host that
    // requires 1.1 has to notice on its own, because the library said it
    // succeeded.
    let required = AbiVersion::new(1, 1);
    let error = unsafe { load(deaf_entry as EntryPointFn, &host, required) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);

    // The same library is accepted when the host requires what it implements.
    let descriptor =
        unsafe { load(deaf_entry as EntryPointFn, &host, ABI_VERSION) }.unwrap();
    assert_eq!(descriptor.name.as_str(), "deaf");
    drop(descriptor);
}

/// A host description as an older revision wrote it: the size field, and
/// nothing after it.
///
/// The type is deliberately shorter than [`HostServices`], so reading it through
/// a `&HostServices` would claim bytes that are not there.
#[repr(C)]
struct ShortHost {
    size: u64,
}

#[test]
fn a_description_is_measured_through_its_size_field_alone() {
    let host = HostServices::new(AbiAllocator::system());
    // SAFETY: the pointer names a live `HostServices`, whose first field is the
    // size.
    let described = unsafe { HostServices::described_size(&host) }.unwrap();
    assert_eq!(described, core::mem::size_of::<HostServices>() as u64);
    assert!(described >= core::mem::size_of::<HostServices>() as u64);

    // A description built against an older revision is measured the same way,
    // and the number it carries is what a library compares: the bytes after it
    // are not there, and nothing in the measurement reads them.
    let short = ShortHost {
        size: core::mem::size_of::<ShortHost>() as u64,
    };
    let pointer = (&short as *const ShortHost).cast::<HostServices>();
    // SAFETY: the pointer names a value whose first field is the size, which is
    // all this reads.
    let described = unsafe { HostServices::described_size(pointer) }.unwrap();
    assert_eq!(described, core::mem::size_of::<ShortHost>() as u64);
    assert!(described < core::mem::size_of::<HostServices>() as u64);

    // A null description describes nothing, which is what a library reports.
    // SAFETY: a null pointer is what the function is written to report.
    assert_eq!(unsafe { HostServices::described_size(core::ptr::null()) }, None);
}

unsafe extern "C" fn no_object_entry(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: the host provides a writable buffer.
    unsafe {
        PluginDescriptor::store(out, "empty", AbiVersion::new(1, 0), StableDyn::empty(), None)
            .code()
    }
}

unsafe extern "C" fn newer_entry(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    if out.is_null() {
        return AbiStatus::NullPointer.code();
    }
    // SAFETY: the host provides a writable buffer.
    unsafe {
        (*out).header.written = MIN_DESCRIPTOR_SIZE;
        (*out).header.version = AbiVersion::new(ABI_VERSION.major, ABI_VERSION.minor + 1);
        (*out).api = sample_object();
    }
    AbiStatus::Ok.code()
}

/// Writes a complete descriptor at this build's version, ignoring what the
/// host requires.
unsafe extern "C" fn indifferent_entry(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: the host provides a writable buffer.
    unsafe { PluginDescriptor::store(out, "indifferent", ABI_VERSION, sample_object(), None).code() }
}

/// Writes a complete descriptor but reports the size of the first layout.
unsafe extern "C" fn older_writer(
    host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    if host.is_null() {
        return AbiStatus::NullPointer.code();
    }
    // SAFETY: the host provides a writable buffer.
    let status = unsafe {
        PluginDescriptor::store(out, "older", AbiVersion::new(0, 0), sample_object(), None)
    };
    if status.is_ok() {
        // SAFETY: the write above succeeded, so the buffer holds a descriptor.
        unsafe { (*out).header.written = MIN_DESCRIPTOR_SIZE };
    }
    status.code()
}

unsafe extern "C" fn silent_entry(
    _host: *const HostServices,
    _required: AbiVersion,
    _out: *mut PluginDescriptor,
) -> u32 {
    AbiStatus::Ok.code()
}

static SAMPLE_LOGS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn sample_log(
    _context: *mut core::ffi::c_void,
    level: u32,
    message: StableStr<'_>,
) {
    assert_eq!(AbiLogLevel::from_code(level), Some(AbiLogLevel::Info));
    assert_eq!(message.as_str(), "entering");
    SAMPLE_LOGS.fetch_add(1, Ordering::SeqCst);
}

fn sample_host() -> HostServices {
    let mut services = HostServices::new(AbiAllocator::system());
    services.log = Some(sample_log);
    services
}

#[test]
fn a_well_formed_library_loads() {
    let services = sample_host();
    let descriptor = unsafe { load(good_entry as EntryPointFn, &services, ABI_VERSION) }.unwrap();
    assert_eq!(
        descriptor.header.written,
        core::mem::size_of::<PluginDescriptor>() as u64
    );
    assert_eq!(
        descriptor.header.capacity,
        core::mem::size_of::<PluginDescriptor>() as u64
    );
    assert_eq!(descriptor.header.version, ABI_VERSION);
    assert_eq!(descriptor.name_str(), "sample");
    assert_eq!(descriptor.plugin_version, AbiVersion::new(0, 4));
    assert_eq!(descriptor.api.type_id(), 9);
    assert_eq!(SAMPLE_LOGS.load(Ordering::SeqCst), 1);
}

#[test]
fn a_reported_failure_is_returned_with_its_code() {
    let services = sample_host();
    let error = unsafe { load(null_host_entry as EntryPointFn, &services, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::NullPointer);
}

#[test]
fn an_entry_point_may_reject_the_required_version() {
    let services = sample_host();
    let error = unsafe {
        load(good_entry as EntryPointFn, &services, AbiVersion::new(2, 0))
    }
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);
}

#[test]
fn a_missing_object_is_rejected() {
    let services = sample_host();
    let error =
        unsafe { load(no_object_entry as EntryPointFn, &services, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_newer_minor_is_served() {
    // The major number and the layout revision are what the host reads the
    // fields through; a field added by the newer build sits past the bytes read
    // here, so a newer minor is not a mismatch.
    let services = sample_host();
    let descriptor = unsafe { load(newer_entry as EntryPointFn, &services, ABI_VERSION) }.unwrap();
    assert_eq!(
        descriptor.header.version,
        AbiVersion::new(ABI_VERSION.major, ABI_VERSION.minor + 1)
    );
    assert_eq!(descriptor.api.type_id(), 9);
}

#[test]
fn a_library_older_than_the_required_version_is_rejected() {
    // The entry point does not check `required` itself, so what refuses this
    // library is the host's floor.
    let services = sample_host();
    let required = AbiVersion::new(ABI_VERSION.major, ABI_VERSION.minor + 1);
    let error =
        unsafe { load(indifferent_entry as EntryPointFn, &services, required) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);
}

#[test]
fn a_writer_that_reports_nothing_is_rejected() {
    let services = sample_host();
    let error =
        unsafe { load(silent_entry as EntryPointFn, &services, ABI_VERSION) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
    // The size check is what refuses this writer; the object check sits behind
    // it and would otherwise answer with a different message.
    assert_eq!(
        error.message(),
        "the library wrote fewer bytes than the smallest descriptor"
    );
}

#[test]
fn store_refuses_a_buffer_that_is_too_small_before_writing_any_field() {
    unsafe extern "C" fn never_runs() {}
    let mut descriptor = PluginDescriptor::buffer();
    descriptor.header.capacity = 8;
    let status = unsafe {
        PluginDescriptor::store(
            &mut descriptor,
            "small",
            AbiVersion::new(1, 0),
            sample_object(),
            Some(never_runs),
        )
    };
    assert_eq!(status, AbiStatus::InvalidDescriptor);
    // The hook the caller offered was not recorded either: the check runs before
    // any field is written, so a buffer that does not cover the layout is left
    // as it was.
    assert!(descriptor.shutdown.is_none());
    assert_eq!(descriptor.header.written, 0);
    assert_eq!(descriptor.name_str(), "");
}

#[test]
fn store_refuses_a_null_buffer() {
    let status = unsafe {
        PluginDescriptor::store(
            core::ptr::null_mut(),
            "",
            AbiVersion::new(1, 0),
            StableDyn::empty(),
            None,
        )
    };
    assert_eq!(status, AbiStatus::NullPointer);
}

#[test]
fn store_replaces_a_previous_descriptor_without_leaking_it() {
    let mut descriptor = PluginDescriptor::buffer();
    // SAFETY: the buffer is valid and large enough for both writes.
    unsafe {
        PluginDescriptor::store(
            &mut descriptor,
            "first",
            AbiVersion::new(1, 0),
            sample_object(),
            None,
        )
    };
    assert_eq!(descriptor.name_str(), "first");
    // SAFETY: the buffer still holds a valid descriptor, which the second write
    // releases before overwriting.
    unsafe {
        PluginDescriptor::store(
            &mut descriptor,
            "second",
            AbiVersion::new(1, 1),
            sample_object(),
            None,
        )
    };
    assert_eq!(descriptor.name_str(), "second");
    assert_eq!(descriptor.plugin_version, AbiVersion::new(1, 1));
}

#[test]
fn log_levels_roundtrip() {
    for level in [
        AbiLogLevel::Error,
        AbiLogLevel::Warning,
        AbiLogLevel::Info,
        AbiLogLevel::Debug,
    ] {
        assert_eq!(AbiLogLevel::from_code(level.code()), Some(level));
    }
    assert_eq!(AbiLogLevel::from_code(4), None);
}

#[test]
fn headers_report_their_size_and_version() {
    let header = AbiHeader::new(32, 24, AbiVersion::new(1, 0), 9);
    assert_eq!(
        header.to_string(),
        "24 of 32 bytes at version 1.0, layout revision 9"
    );
}

/// Objects whose table releases the value have been dropped.
static OBJECT_DROPS: AtomicUsize = AtomicUsize::new(0);
/// What [`OBJECT_DROPS`] read when the shutdown hook ran.
static DROPS_AT_SHUTDOWN: AtomicUsize = AtomicUsize::new(usize::MAX);
/// Shutdown hooks that ran.
static HOOKS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn counting_drop(data: *mut u8) {
    OBJECT_DROPS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the table describes a `u32`.
    unsafe { core::ptr::drop_in_place(data.cast::<u32>()) }
}

static COUNTED_TABLE: VTablePrefix = VTablePrefix {
    type_id: 11,
    signature: 1,
    layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
    size: 4,
    align: 4,
    slots: 0,
    drop: Some(counting_drop),
};

/// Builds a `StableDyn` whose release is observable.
fn counted_object() -> StableDyn {
    let allocator = AbiAllocator::system();
    let data = unsafe { allocate_value(5u32, allocator) }.unwrap();
    // SAFETY: the storage holds a `u32` and the table describes one.
    unsafe { StableDyn::from_raw_parts(data, &COUNTED_TABLE, allocator) }
}

unsafe extern "C" fn observing_shutdown() {
    HOOKS.fetch_add(1, Ordering::SeqCst);
    DROPS_AT_SHUTDOWN.store(OBJECT_DROPS.load(Ordering::SeqCst), Ordering::SeqCst);
}

unsafe extern "C" fn entry_with_shutdown(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: the host provides a writable buffer.
    unsafe {
        PluginDescriptor::store(
            out,
            "counted",
            AbiVersion::new(0, 5),
            counted_object(),
            Some(observing_shutdown),
        )
        .code()
    }
}

unsafe extern "C" fn entry_with_a_hook_and_no_object(
    _host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: the host provides a writable buffer.
    unsafe {
        PluginDescriptor::store(
            out,
            "empty",
            AbiVersion::new(0, 6),
            StableDyn::empty(),
            Some(observing_shutdown),
        )
        .code()
    }
}

fn reset_counters() {
    OBJECT_DROPS.store(0, Ordering::SeqCst);
    DROPS_AT_SHUTDOWN.store(usize::MAX, Ordering::SeqCst);
    HOOKS.store(0, Ordering::SeqCst);
    KEEP_ALIVE.store(usize::MAX, Ordering::SeqCst);
}

/// What [`HOOKS`] read when the keep-alive value was dropped.
static KEEP_ALIVE: AtomicUsize = AtomicUsize::new(usize::MAX);

/// A stand-in for a library handle the host owns.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        KEEP_ALIVE.store(HOOKS.load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

#[test]
fn the_keep_alive_value_outlives_the_shutdown_hook() {
    reset_counters();
    let descriptor =
        unsafe { load(entry_with_shutdown as EntryPointFn, &sample_host(), ABI_VERSION) }.unwrap();
    let plugin = LoadedPlugin::with_keep_alive(descriptor, Guard);
    assert_eq!(KEEP_ALIVE.load(Ordering::SeqCst), usize::MAX);

    drop(plugin);
    // The hook ran before the value the host passed was dropped, which is the
    // order a library handle needs: nothing touches the code after unloading.
    assert_eq!(HOOKS.load(Ordering::SeqCst), 1);
    assert_eq!(KEEP_ALIVE.load(Ordering::SeqCst), 1);
}

#[test]
fn the_object_is_released_before_the_shutdown_hook_runs() {
    reset_counters();
    let plugin = LoadedPlugin::new(
        unsafe { load(entry_with_shutdown as EntryPointFn, &sample_host(), ABI_VERSION) }.unwrap(),
    );
    assert_eq!(OBJECT_DROPS.load(Ordering::SeqCst), 0);

    drop(plugin);
    assert_eq!(OBJECT_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(HOOKS.load(Ordering::SeqCst), 1);
    // The hook ran after the library-owned object was gone, so a hook that
    // tears down the allocator cannot pull it out from under the release.
    assert_eq!(DROPS_AT_SHUTDOWN.load(Ordering::SeqCst), 1);
}

#[test]
fn into_descriptor_returns_a_value_the_library_no_longer_backs() {
    reset_counters();
    let plugin = LoadedPlugin::new(
        unsafe { load(entry_with_shutdown as EntryPointFn, &sample_host(), ABI_VERSION) }.unwrap(),
    );
    assert_eq!(plugin.name(), "counted");

    let descriptor = plugin.into_descriptor();
    assert!(descriptor.api.is_empty());
    assert_eq!(descriptor.name_str(), "");
    assert_eq!(OBJECT_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(HOOKS.load(Ordering::SeqCst), 1);
}

#[test]
fn a_rejected_descriptor_still_runs_its_hook() {
    reset_counters();
    let error = unsafe {
        load(
            entry_with_a_hook_and_no_object as EntryPointFn,
            &sample_host(),
            ABI_VERSION,
        )
    }
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
    assert_eq!(HOOKS.load(Ordering::SeqCst), 1);
}

#[test]
fn a_writer_that_knows_only_the_first_layout_is_accepted() {
    let descriptor = unsafe {
        load(older_writer as EntryPointFn, &sample_host(), ABI_VERSION)
    }
    .unwrap();
    assert_eq!(descriptor.header.written, MIN_DESCRIPTOR_SIZE);
    assert_eq!(descriptor.name_str(), "older");
    assert_eq!(descriptor.api.type_id(), 9);
}

static GIVEN_UP_DROPS: AtomicUsize = AtomicUsize::new(0);

/// An implementation that counts being released.
struct GivenUp;

impl Drop for GivenUp {
    fn drop(&mut self) {
        GIVEN_UP_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

impl Greeter for GivenUp {
    fn greet(&self, name: StableStr<'_>) -> StableString {
        StableString::from_str(name.as_str()).unwrap()
    }

    fn produced(&self) -> u64 {
        0
    }
}

/// Writes its fields and then reports a failure.
unsafe extern "C" fn entry_that_gives_up_after_writing(
    host: *const HostServices,
    _required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    if host.is_null() || out.is_null() {
        return AbiStatus::NullPointer.code();
    }
    let object = GreeterObject::abi_from_value(GivenUp);
    // SAFETY: the host provides a writable descriptor buffer.
    unsafe {
        (*out).name = StableString::from_str("half-written").unwrap();
        (*out).api = object.abi_into_stable();
    }
    AbiStatus::InvalidArgument.code()
}

#[test]
fn a_library_that_reports_failure_leaves_what_it_wrote_to_itself() {
    reset_counters();
    GIVEN_UP_DROPS.store(0, Ordering::SeqCst);
    let error = unsafe {
        load(
            entry_that_gives_up_after_writing as EntryPointFn,
            &sample_host(),
            ABI_VERSION,
        )
    }
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
    // The fields were written, but the library said the call did not succeed.
    // Neither the object nor the name is released: the library declared that it
    // left nothing for the host to release, and calling through the numbers it
    // wrote would be acting on a claim it withdrew.
    assert_eq!(GIVEN_UP_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(HOOKS.load(Ordering::SeqCst), 0);
}

static WRONG_REVISION_DROPS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn counted_drop(_data: *mut u8) {
    WRONG_REVISION_DROPS.fetch_add(1, Ordering::SeqCst);
}

/// A table written against another boundary layout.
static WRONG_REVISION_TABLE: VTablePrefix = VTablePrefix {
    type_id: 1,
    signature: 1,
    layout_revision: 0,
    size: 8,
    align: 8,
    slots: 0,
    drop: Some(counted_drop),
};

#[test]
fn a_table_this_build_cannot_check_is_released_by_nobody() {
    WRONG_REVISION_DROPS.store(0, Ordering::SeqCst);
    let block = Box::into_raw(Box::new(7u64));
    // SAFETY: the block is a live `u64` block, and the table claims a layout this
    // build does not accept.
    let handle = unsafe {
        StableDyn::from_raw_parts(
            block.cast::<u8>(),
            &WRONG_REVISION_TABLE,
            AbiAllocator::system(),
        )
    };
    assert!(!handle.is_releasable());
    drop(handle);
    // Neither the table's `drop` nor the allocator is called with a size the
    // table claims but this build cannot establish.
    assert_eq!(WRONG_REVISION_DROPS.load(Ordering::SeqCst), 0);
    // SAFETY: the block was never released, and it is the one the line above
    // allocated.
    drop(unsafe { Box::from_raw(block) });
}
