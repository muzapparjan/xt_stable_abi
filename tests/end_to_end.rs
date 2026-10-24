//! The whole protocol against libraries that are separate build artifacts.
//!
//! Two dynamic libraries are loaded: one built with this crate, and one that
//! declares every boundary type by hand without depending on it. The host side
//! of this test is compiled into the test binary, so an exchange that works here
//! crossed a real dynamic boundary rather than a function call inside one crate.
//!
//! The interpreter cannot load a dynamic library, so this file is compiled out
//! when the test binary runs under it.

#![cfg(not(miri))]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use xt_stable_abi::{
    load, stable_object, stable_object_guarded, AbiAllocator, AbiLogLevel, AbiStatus, AbiVersion,
    EntryPointFn, HostServices, LoadedPlugin, PluginDescriptor, StableDyn, StableOption, StableStr,
    StableString, ABI_VERSION,
};

/// The lock a test holds for as long as it touches the loaded libraries.
///
/// A dynamic library is loaded once per process however many handles name it, so
/// every test of this file shares one module: its counters, its statics and the
/// file the loader maps. Two tests that ran together would read each other's
/// counts, and a test that rebuilds the artifact while another has it mapped
/// would fail to link.
static LIBRARIES: Mutex<()> = Mutex::new(());

/// Holds [`LIBRARIES`] for the life of the value.
///
/// A test that panics while holding the lock leaves the mutex poisoned, and a
/// poisoned lock still has to keep the next test out, so the guard is recovered
/// rather than unwrapped.
fn hold_libraries() -> std::sync::MutexGuard<'static, ()> {
    match LIBRARIES.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

stable_object! {
    /// The service the plugin publishes.
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
        /// Hands out a second object.
        fn child(&self) -> StableOption<StableDyn>;
    }
}

stable_object! {
    /// The object the service hands out.
    pub struct ChildObject;
    /// The table behind [`ChildObject`].
    pub struct ChildVTable;
    /// What the child offers.
    pub trait Child {
        /// The child's number.
        fn number(&self) -> u64;
    }
}

stable_object! {
    /// The one value the hand-written library publishes.
    pub struct LegacyObject;
    /// The table behind [`LegacyObject`].
    pub struct LegacyVTable;
    /// The single method the hand-written library offers.
    pub trait Legacy {
        /// The published number.
        fn value(&self) -> u64;
    }
}

stable_object_guarded! {
    /// The service whose method gives up.
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

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The directory the workspace builds into.
///
/// The test binary lives in `<target>/<profile>/deps`, so its own path names the
/// profile directory whatever `CARGO_TARGET_DIR` and the chosen profile are.
fn profile_directory() -> PathBuf {
    let executable = std::env::current_exe().expect("locate the test executable");
    executable
        .parent()
        .and_then(Path::parent)
        .expect("the test binary sits in a deps directory")
        .to_path_buf()
}

fn library_path(name: &str) -> PathBuf {
    #[cfg(windows)]
    let file = format!("{name}.dll");
    #[cfg(target_os = "macos")]
    let file = format!("lib{name}.dylib");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let file = format!("lib{name}.so");
    profile_directory().join(file)
}

/// The directory cargo builds into, found by the marker it writes there.
fn target_directory() -> PathBuf {
    let mut directory = profile_directory();
    while let Some(parent) = directory.parent() {
        if !parent.join("CACHEDIR.TAG").exists() {
            break;
        }
        directory = parent.to_path_buf();
    }
    directory
}

/// The target the test binary was built for, when it is not the host.
///
/// A build that names a target keeps its artifacts one level deeper, under the
/// target's own name, so the distance from the target directory says which
/// target that was. Building the plugins without it would refresh the host's
/// artifacts and leave this target's in place.
fn build_target() -> Option<std::ffi::OsString> {
    let profile = profile_directory();
    let relative = profile.strip_prefix(target_directory()).ok()?;
    let mut parts = relative.components();
    let target = parts.next()?.as_os_str().to_os_string();
    parts.next().map(|_| target)
}

/// The profile directory the test binary was built into, as cargo named it.
///
/// The path is `<target>/<profile>` or `<target>/<triple>/<profile>`, so the
/// last part names the profile. `debug` is the directory cargo uses when no
/// profile is asked for.
fn build_profile() -> Option<std::ffi::OsString> {
    profile_directory()
        .file_name()
        .map(|name| name.to_os_string())
}

/// The newest modification time in the sources of this crate and the plugins.
///
/// A manifest is part of what decides the artifact: changing a dependency or a
/// feature changes what cargo builds without touching a `.rs` file, so the
/// files that say what to build are read here too.
fn newest_source() -> std::time::SystemTime {
    fn latest(directory: &Path, newest: &mut Option<std::time::SystemTime>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                latest(&path, newest);
            } else {
                let is_source = path.extension().is_some_and(|extension| extension == "rs");
                let is_manifest = path
                    .file_name()
                    .is_some_and(|name| name == "Cargo.toml" || name == "build.rs");
                if !is_source && !is_manifest {
                    continue;
                }
                let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified()) else {
                    continue;
                };
                if newest.is_none_or(|current| modified > current) {
                    *newest = Some(modified);
                }
            }
        }
    }

    let mut newest = None;
    latest(&root().join("src"), &mut newest);
    latest(&root().join("plugins"), &mut newest);
    // The lock file decides which dependency versions the build uses, and it sits
    // beside the manifest rather than in a directory the walk above reaches.
    for manifest in ["Cargo.toml", "Cargo.lock"] {
        let path = root().join(manifest);
        if let Ok(modified) = std::fs::metadata(&path).and_then(|metadata| metadata.modified())
            && newest.is_none_or(|current| modified > current)
        {
            newest = Some(modified);
        }
    }
    newest.expect("the tree holds sources")
}

/// Makes sure every artifact exists and was built from the current sources.
///
/// Cargo builds a library target only when it is asked for one, so `cargo test`
/// alone leaves the dynamic libraries behind. Building them here keeps the tests
/// from reporting on an artifact that no longer matches the tree. A missing
/// artifact and an artifact older than the sources both lead to a build.
///
/// The build names the target this test binary was built for. Without that, a
/// run for another target would refresh the host's artifacts and leave this
/// target's in place, so the tests would report on a library built for a
/// different target.
///
/// The profile is named the same way. A `cargo test --release` run reads the
/// artifacts under `release`, and a build with no flag writes under `debug`; the
/// stale check above looks at the release file, so a debug build leaves it
/// untouched and the test would load a library older than the tree it reports
/// on - or, when the declaration changed in between, load one that no longer
/// matches.
fn ensure_built(names: &[&str]) {
    let newest = newest_source();
    let stale: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| match std::fs::metadata(library_path(name)).and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified < newest,
            Err(_) => true,
        })
        .collect();
    if stale.is_empty() {
        return;
    }

    let mut command = Command::new(env!("CARGO"));
    command.current_dir(root()).arg("build");
    if let Some(target) = build_target() {
        command.arg("--target").arg(target);
    }
    if let Some(profile) = build_profile() {
        if profile == std::ffi::OsStr::new("release") {
            command.arg("--release");
        } else if profile != std::ffi::OsStr::new("debug") {
            // A named profile keeps its own directory, which this test binary
            // was built into.
            command.arg("--profile").arg(profile);
        }
    }
    let status = command
        .args(stale.iter().map(|name| format!("-p={name}")))
        .status()
        .expect("start cargo to build the plugin artifacts");
    assert!(status.success(), "building the plugin artifacts failed");

    for name in names {
        let path = library_path(name);
        assert!(
            path.exists(),
            "the build left no artifact at {}",
            path.display()
        );
    }
}

/// Opens a dynamic library and looks up a symbol.
struct Library {
    handle: *mut c_void,
}

impl Library {
    fn open(path: &Path) -> Self {
        assert!(path.exists(), "missing artifact at {}", path.display());
        // SAFETY: the path points at a file this test built.
        let handle = unsafe { sys::open(path) };
        assert!(!handle.is_null(), "could not open {}", path.display());
        Self { handle }
    }

    fn symbol(&self, name: &str) -> *mut c_void {
        // SAFETY: the handle is live until this value is dropped.
        let symbol = unsafe { sys::lookup(self.handle, name) };
        assert!(!symbol.is_null(), "missing symbol {name}");
        symbol
    }
}

impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: the handle came from `sys::open` and is closed once.
        unsafe { sys::close(self.handle) };
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> i32;
    }

    pub unsafe fn open(path: &Path) -> *mut c_void {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        // SAFETY: the caller passes a path to a file that exists.
        unsafe { LoadLibraryW(wide.as_ptr()) }
    }

    pub unsafe fn lookup(module: *mut c_void, name: &str) -> *mut c_void {
        let mut text: Vec<u8> = name.as_bytes().to_vec();
        text.push(0);
        // SAFETY: the module is live and the name is NUL-terminated.
        unsafe { GetProcAddress(module, text.as_ptr()) }
    }

    pub unsafe fn close(module: *mut c_void) {
        // SAFETY: the module came from `open`.
        unsafe { FreeLibrary(module) };
    }
}

#[cfg(not(windows))]
mod sys {
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::path::Path;

    const RTLD_NOW: c_int = 2;

    unsafe extern "C" {
        fn dlopen(name: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
    }

    pub unsafe fn open(path: &Path) -> *mut c_void {
        let text = CString::new(path.to_string_lossy().as_bytes()).expect("path has no NUL");
        // SAFETY: the caller passes a path to a file that exists.
        unsafe { dlopen(text.as_ptr(), RTLD_NOW) }
    }

    pub unsafe fn lookup(module: *mut c_void, name: &str) -> *mut c_void {
        let text = CString::new(name).expect("symbol has no NUL");
        // SAFETY: the module is live and the name is NUL-terminated.
        unsafe { dlsym(module, text.as_ptr()) }
    }

    pub unsafe fn close(module: *mut c_void) {
        // SAFETY: the module came from `open`.
        unsafe { dlclose(module) };
    }
}

fn host() -> HostServices {
    HostServices::new(AbiAllocator::system())
}

#[test]
fn a_library_built_with_the_crate_reaches_the_host() {
    let _libraries = hold_libraries();
    ensure_built(&["abi_plugin", "legacy_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let entry: EntryPointFn = unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_entry")) };

    let services = host();
    let mut plugin = LoadedPlugin::new(
        unsafe { load(entry, &services, ABI_VERSION) }.expect("the plugin reports a usable object"),
    );
    assert_eq!(plugin.name(), "abi_plugin");
    assert_eq!(plugin.descriptor().plugin_version, AbiVersion::new(0, 2));

    let object = ServiceObject::abi_try_from_stable(plugin.take_api()).expect("adopt");
    assert_eq!(unsafe { object.value() }, 42);
    assert_eq!(unsafe { object.label() }.as_str(), "built-with-crate");
    assert_eq!(
        unsafe { object.greet(StableStr::from_str("host")) }.as_str(),
        "hello host"
    );

    // The host renders the object; the block goes back to the host allocator,
    // and the table's drop entry lives in the library, which is still loaded.
    drop(object);
}

#[test]
fn a_library_that_declares_the_layout_by_hand_is_understood() {
    let _libraries = hold_libraries();
    ensure_built(&["legacy_plugin"]);
    let library = Library::open(&library_path("legacy_plugin"));
    let entry: EntryPointFn =
        unsafe { std::mem::transmute(library.symbol("xt_legacy_plugin_entry")) };

    let services = host();
    let mut plugin = LoadedPlugin::new(
        unsafe { load(entry, &services, ABI_VERSION) }.expect("the library reports a usable object"),
    );
    assert_eq!(plugin.name(), "legacy_plugin");
    assert_eq!(plugin.descriptor().plugin_version, AbiVersion::new(0, 9));

    // The hand-written library has no way to reproduce the declaration
    // signature, so the host states the agreement itself.
    // SAFETY: the library declares the same single slot this host declares.
    let object = unsafe { LegacyObject::abi_from_stable_unchecked(plugin.take_api()) };
    assert_eq!(unsafe { object.value() }, 7);
    drop(object);
}

#[test]
fn the_shutdown_hook_runs_while_the_library_is_loaded() {
    let _libraries = hold_libraries();
    ensure_built(&["abi_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let entry: EntryPointFn = unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_entry")) };
    let counter: unsafe extern "C" fn() -> u64 =
        unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_shutdowns")) };

    let before = unsafe { counter() };
    let services = host();
    let mut plugin = LoadedPlugin::new(unsafe { load(entry, &services, ABI_VERSION) }.expect("load"));
    let object = ServiceObject::abi_try_from_stable(plugin.take_api()).expect("adopt");
    drop(object);
    drop(plugin);
    assert_eq!(unsafe { counter() }, before + 1);
}

#[test]
fn a_null_host_is_refused_instead_of_dereferenced() {
    let _libraries = hold_libraries();
    ensure_built(&["abi_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let entry: EntryPointFn = unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_entry")) };
    let mut buffer = PluginDescriptor::buffer();
    // SAFETY: a null host is exactly what the library is meant to reject, and
    // the buffer is a valid descriptor.
    let status = unsafe { entry(core::ptr::null(), ABI_VERSION, &mut buffer) };
    assert_ne!(status, 0, "a null host must not be accepted");
    assert_eq!(buffer.header.written, 0);
}

#[test]
fn a_library_that_cannot_serve_the_required_version_says_so() {
    let _libraries = hold_libraries();
    ensure_built(&["legacy_plugin"]);
    let library = Library::open(&library_path("legacy_plugin"));
    let entry: EntryPointFn =
        unsafe { std::mem::transmute(library.symbol("xt_legacy_plugin_entry")) };
    let error = unsafe { load(entry, &host(), AbiVersion::new(2, 0)) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::VersionMismatch);
}

#[test]
fn the_host_log_sink_receives_the_library_message() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static MESSAGES: AtomicUsize = AtomicUsize::new(0);

    let _libraries = hold_libraries();

    unsafe extern "C" fn sink(
        _context: *mut c_void,
        level: u32,
        message: StableStr<'_>,
    ) {
        assert_eq!(AbiLogLevel::from_code(level), Some(AbiLogLevel::Info));
        assert_eq!(message.as_str(), "abi_plugin: entering");
        MESSAGES.fetch_add(1, Ordering::SeqCst);
    }

    ensure_built(&["abi_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let entry: EntryPointFn = unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_entry")) };
    let mut services = host();
    services.log = Some(sink);
    let plugin = LoadedPlugin::new(unsafe { load(entry, &services, ABI_VERSION) }.expect("load"));
    assert_eq!(MESSAGES.load(Ordering::SeqCst), 1);
    drop(plugin);
}

#[test]
fn a_panic_inside_the_library_reaches_the_host() {
    let _libraries = hold_libraries();
    ensure_built(&["abi_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let entry: EntryPointFn =
        unsafe { std::mem::transmute(library.symbol("xt_abi_fragile_entry")) };
    let mut plugin = LoadedPlugin::new(unsafe { load(entry, &host(), ABI_VERSION) }.expect("load"));
    assert_eq!(plugin.name(), "abi_plugin_fragile");

    let object = FragileObject::abi_try_from_stable(plugin.take_api()).expect("adopt");
    // The call that does not panic answers normally.
    assert_eq!(unsafe { object.calm() }.unwrap(), 7);
    // The call that panics was caught inside the trampoline, which lives in the
    // library. The report has to travel back with the call, because each binary
    // has its own copy of every thread-local.
    let error = unsafe { object.boom() }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::Panic);
    // The contained panic does not leave the object unusable.
    assert_eq!(unsafe { object.calm() }.unwrap(), 7);
}

#[test]
fn the_hand_written_library_refuses_a_buffer_it_cannot_fill() {
    let _libraries = hold_libraries();
    ensure_built(&["legacy_plugin"]);
    let library = Library::open(&library_path("legacy_plugin"));
    let entry: EntryPointFn =
        unsafe { std::mem::transmute(library.symbol("xt_legacy_plugin_entry")) };
    // The header is the first field on both sides, so shrinking the capacity is
    // enough to describe a buffer that cannot hold the descriptor.
    let mut buffer = PluginDescriptor::buffer();
    buffer.header.capacity = 16;
    let status = unsafe { entry(&host(), ABI_VERSION, &mut buffer) };
    assert_eq!(
        AbiStatus::from_code(status),
        Some(AbiStatus::InvalidDescriptor)
    );
    assert_eq!(buffer.header.written, 0);
}

#[test]
fn the_published_object_hands_out_a_second_object() {
    let _libraries = hold_libraries();
    ensure_built(&["abi_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let entry: EntryPointFn = unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_entry")) };
    let mut plugin = LoadedPlugin::new(unsafe { load(entry, &host(), ABI_VERSION) }.expect("load"));
    let object = ServiceObject::abi_try_from_stable(plugin.take_api()).expect("adopt");

    let child = unsafe { object.child() }.expect("the service hands out one object");
    let child = ChildObject::abi_try_from_stable(child).expect("adopt the child");
    assert_eq!(unsafe { child.number() }, 99);
    drop(child);
    drop(object);
}

#[test]
fn the_library_calls_a_service_the_host_offers() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static BUMPS: AtomicUsize = AtomicUsize::new(0);

    /// The host's own statement of the service the library asks for.
    #[repr(C)]
    struct Counter {
        bump: unsafe extern "C" fn(*mut std::ffi::c_void) -> u64,
    }

    unsafe extern "C" fn bump(_context: *mut std::ffi::c_void) -> u64 {
        (BUMPS.fetch_add(1, Ordering::SeqCst) + 1) as u64
    }

    unsafe extern "C" fn lookup(
        _context: *mut std::ffi::c_void,
        identifier: u64,
    ) -> *const () {
        if identifier == 1 {
            // SAFETY: the value is a `Counter`, which is what identifier 1
            // names on this side.
            &SERVICE as *const Counter as *const ()
        } else {
            core::ptr::null()
        }
    }

    static SERVICE: Counter = Counter { bump };

    let _libraries = hold_libraries();
    ensure_built(&["abi_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let call: unsafe extern "C" fn(*const HostServices) -> u64 =
        unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_calls_host")) };
    let mut services = host();
    services.query = Some(lookup);

    assert_eq!(unsafe { call(&services) }, 1);
    assert_eq!(unsafe { call(&services) }, 2);
    assert_eq!(BUMPS.load(Ordering::SeqCst), 2);
}

#[test]
fn a_host_description_of_another_layout_is_refused_by_the_library() {
    /// The host's own statement of the service the library asks for.
    #[repr(C)]
    struct Counter {
        bump: unsafe extern "C" fn(*mut std::ffi::c_void) -> u64,
    }

    static BUMPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    unsafe extern "C" fn bump(_context: *mut std::ffi::c_void) -> u64 {
        (BUMPS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1) as u64
    }

    unsafe extern "C" fn lookup(
        _context: *mut std::ffi::c_void,
        identifier: u64,
    ) -> *const () {
        if identifier == 1 {
            // SAFETY: the value is a `Counter`, which is what identifier 1
            // names on this side.
            &SERVICE as *const Counter as *const ()
        } else {
            core::ptr::null()
        }
    }

    static SERVICE: Counter = Counter { bump };

    ensure_built(&["abi_plugin"]);
    let library = Library::open(&library_path("abi_plugin"));
    let call: unsafe extern "C" fn(*const HostServices) -> u64 =
        unsafe { std::mem::transmute(library.symbol("xt_abi_plugin_calls_host")) };

    // A host this build accepts answers with the service's count; that is what
    // makes the refusals below visible.
    let mut services = host();
    services.query = Some(lookup);
    assert_eq!(unsafe { call(&services) }, 1);

    // The size is unchanged, so the byte-count check passes; only the revision
    // says that the fields are not the ones this build reads.
    let mut services = host();
    services.query = Some(lookup);
    services.layout_revision = xt_stable_abi::object::LAYOUT_REVISION.wrapping_add(1);
    assert_eq!(unsafe { call(&services) }, u64::MAX);

    // A host of another major version is refused for the same reason.
    let mut services = host();
    services.query = Some(lookup);
    services.version.major = ABI_VERSION.major + 1;
    assert_eq!(unsafe { call(&services) }, u64::MAX);
}
