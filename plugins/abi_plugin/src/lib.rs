//! A library built as its own dynamic artifact, using the crate.
//!
//! The host loads this file, calls the exported entry point and uses the object
//! that comes back. Nothing about the exchange is compiled into the host except
//! the declarations both sides share.

// MSVC reports the import library it creates for a cdylib; nothing is wrong
// with the artifact.
#![allow(linker_messages)]

use std::sync::atomic::{AtomicUsize, Ordering};

use core::ffi::c_void;

use xt_stable_abi::{
    release_cached_tables, stable_object, stable_object_guarded, AbiAllocator, AbiStatus,
    AbiVersion, HostServices, PluginDescriptor, StableOption, StableStr, StableString, StableDyn,
    ABI_VERSION,
};

static SHUTDOWNS: AtomicUsize = AtomicUsize::new(0);

stable_object! {
    /// A handle to the service this library publishes.
    pub struct ServiceObject;
    /// The table behind [`ServiceObject`].
    pub struct ServiceVTable;
    /// The published behaviour.
    pub trait Service {
        /// The value the service was built with.
        fn value(&self) -> u64;
        /// A word describing the service.
        fn label(&self) -> StableString;
        /// Greets `name`.
        fn greet(&self, name: StableStr<'_>) -> StableString;
        /// Hands out a second object, so one entry point can publish more than
        /// one service.
        fn child(&self) -> StableOption<StableDyn>;
    }
}

stable_object! {
    /// A handle to the object the service hands out.
    pub struct ChildObject;
    /// The table behind [`ChildObject`].
    pub struct ChildVTable;
    /// What the child offers.
    pub trait Child {
        /// The child's number.
        fn number(&self) -> u64;
    }
}

/// The object the service hands out.
struct Nested;

impl Child for Nested {
    fn number(&self) -> u64 {
        99
    }
}

/// The implementation this library publishes.
struct Counter {
    value: u64,
    allocator: AbiAllocator,
}

impl Service for Counter {
    fn value(&self) -> u64 {
        self.value
    }

    fn label(&self) -> StableString {
        StableString::from_str("built-with-crate").unwrap()
    }

    fn greet(&self, name: StableStr<'_>) -> StableString {
        StableString::from_str(&format!("hello {}", name.as_str())).unwrap()
    }

    fn child(&self) -> StableOption<StableDyn> {
        let child = ChildObject::abi_try_from_value_with(
            Nested,
            xt_stable_abi::object::type_id_of("Child"),
            self.allocator,
        )
        .expect("build the child object");
        StableOption::some(child.abi_into_stable())
    }
}

stable_object_guarded! {
    /// A handle to the service that may give up.
    pub struct FragileObject;
    /// The table behind [`FragileObject`].
    pub struct FragileVTable;
    /// The published behaviour of the fragile service.
    pub trait Fragile {
        /// The number, unless the implementation gives up.
        fn boom(&self) -> u64;
        /// A number the implementation always produces.
        fn calm(&self) -> u64;
    }
}

/// The implementation of [`Fragile`].
struct Grumpy;

impl Fragile for Grumpy {
    fn boom(&self) -> u64 {
        panic!("the plugin gave up");
    }

    fn calm(&self) -> u64 {
        7
    }
}

unsafe extern "C" fn shutdown() {
    SHUTDOWNS.fetch_add(1, Ordering::SeqCst);
    // The tables this load built were allocated from the process allocator and
    // only this module can reach them, so they have to go back before the host
    // unloads it. Nothing built from them is alive: the host released what it
    // adopted before calling this hook.
    // SAFETY: the host unloads the module only after this returns.
    unsafe { release_cached_tables() };
}

/// The entry point the host looks up by name.
///
/// # Safety
///
/// `host` must be null, or point at a description whose first field is the
/// `u64` size this crate declares there, and whose bytes cover that size when
/// it says they do; `out` must be null or point at a descriptor buffer the host
/// provided.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xt_abi_plugin_entry(
    host: *const HostServices,
    required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    if out.is_null() {
        return AbiStatus::NullPointer.code();
    }
    if !ABI_VERSION.serves(required) {
        return AbiStatus::VersionMismatch.code();
    }
    // The description is read through its size field alone first: a host built
    // against an older revision describes fewer bytes than this build knows,
    // and forming `&HostServices` over those bytes would claim storage the host
    // never provided.
    // SAFETY: the host guarantees a readable size field for the call.
    let described = match unsafe { HostServices::described_size(host) } {
        Some(size) => size,
        None => return AbiStatus::NullPointer.code(),
    };
    if described < core::mem::size_of::<HostServices>() as u64 {
        return AbiStatus::InvalidDescriptor.code();
    }
    // SAFETY: the description covers every field, as checked above.
    let host = unsafe { &*host };
    // The size says the bytes are there; the version says what they mean. A host
    // of another major may give an existing field another meaning, and a field
    // appended to the description does not announce itself any other way.
    if host.version.major != ABI_VERSION.major {
        return AbiStatus::VersionMismatch.code();
    }
    if host.layout_revision != xt_stable_abi::object::LAYOUT_REVISION {
        return AbiStatus::VersionMismatch.code();
    }
    // The buffer has to hold the layout this build writes before any field of it
    // is written.
    // SAFETY: `out` is non-null, and the host guarantees it names a buffer.
    let capacity = unsafe { (*out).header.capacity };
    if capacity < core::mem::size_of::<PluginDescriptor>() as u64 {
        return AbiStatus::InvalidDescriptor.code();
    }
    host.log(xt_stable_abi::AbiLogLevel::Info, "abi_plugin: entering");

    let object = match ServiceObject::abi_try_from_value_with(
        Counter {
            value: 42,
            allocator: host.allocator,
        },
        xt_stable_abi::object::type_id_of("Service"),
        host.allocator,
    ) {
        Ok(object) => object,
        Err(error) => return error.status().code(),
    };

    // SAFETY: the host provides a writable descriptor buffer.
    unsafe {
        PluginDescriptor::store(
            out,
            "abi_plugin",
            AbiVersion::new(0, 2),
            object.abi_into_stable(),
            Some(shutdown),
        )
        .code()
    }
}

/// Publishes an object whose methods are contained against panics.
///
/// # Safety
///
/// `host` must be null, or point at a description whose first field is the
/// `u64` size this crate declares there; `out` must be null or point at a
/// descriptor buffer the host provided.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xt_abi_fragile_entry(
    host: *const HostServices,
    required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    if out.is_null() {
        return AbiStatus::NullPointer.code();
    }
    if !ABI_VERSION.serves(required) {
        return AbiStatus::VersionMismatch.code();
    }
    // SAFETY: the host guarantees a readable size field for the call.
    let described = match unsafe { HostServices::described_size(host) } {
        Some(size) => size,
        None => return AbiStatus::NullPointer.code(),
    };
    if described < core::mem::size_of::<HostServices>() as u64 {
        return AbiStatus::InvalidDescriptor.code();
    }
    // SAFETY: the description covers every field, as checked above.
    let host = unsafe { &*host };
    if host.version.major != ABI_VERSION.major {
        return AbiStatus::VersionMismatch.code();
    }
    if host.layout_revision != xt_stable_abi::object::LAYOUT_REVISION {
        return AbiStatus::VersionMismatch.code();
    }
    let object = match FragileObject::abi_try_from_value_with(
        Grumpy,
        xt_stable_abi::object::type_id_of("Fragile"),
        host.allocator,
    ) {
        Ok(object) => object,
        Err(error) => return error.status().code(),
    };

    // SAFETY: the host provides a writable descriptor buffer.
    unsafe {
        PluginDescriptor::store(
            out,
            "abi_plugin_fragile",
            AbiVersion::new(0, 3),
            object.abi_into_stable(),
            Some(shutdown),
        )
        .code()
    }
}

/// Reports how many times the shutdown hook ran, for the host's assertion.
///
/// # Safety
///
/// The function only reads a static and is safe to call from the host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xt_abi_plugin_shutdowns() -> u64 {
    SHUTDOWNS.load(Ordering::SeqCst) as u64
}

/// The shape of the service the host offers under identifier 1.
#[repr(C)]
pub struct CounterService {
    /// Counts one call and reports the new total.
    bump: unsafe extern "C" fn(*mut c_void) -> u64,
}

/// Calls a service the host offers, through the query channel.
///
/// # Safety
///
/// `host` must be null, or point at a description whose first field is the
/// `u64` size this crate declares there; and the value its query returns for
/// identifier 1 must point at a [`CounterService`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xt_abi_plugin_calls_host(host: *const HostServices) -> u64 {
    // SAFETY: the host guarantees a readable size field for the call.
    let described = match unsafe { HostServices::described_size(host) } {
        Some(size) => size,
        None => return u64::MAX,
    };
    if described < core::mem::size_of::<HostServices>() as u64 {
        return u64::MAX;
    }
    // SAFETY: the description covers every field, as checked above.
    let host = unsafe { &*host };
    // A host of another major may give an existing field another meaning; this
    // export has no status word, so it answers with the value that names no
    // count.
    if host.version.major != ABI_VERSION.major {
        return u64::MAX;
    }
    if host.layout_revision != xt_stable_abi::object::LAYOUT_REVISION {
        return u64::MAX;
    }
    let service = host.service(1) as *const CounterService;
    if service.is_null() {
        return u64::MAX;
    }
    // SAFETY: the host states that identifier 1 is a `CounterService`.
    let service = unsafe { &*service };
    // SAFETY: the function pointer belongs to the host, which is still loaded.
    unsafe { (service.bump)(host.context) }
}
