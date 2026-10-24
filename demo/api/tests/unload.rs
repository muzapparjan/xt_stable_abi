//! Loading a plugin, using it, unloading it, and loading it again in the same
//! process.
//!
//! A host that swaps mods at run time does exactly this, so the order matters:
//! everything the library produced has to be released while its code is still
//! mapped, and the library is unloaded last.

use std::path::{Path, PathBuf};
use std::process::Command;

use xt_kernel_api::loader::{Library, Plugin};
use xt_stable_abi::StableStr;

/// The directory cargo builds into, taken from where this test binary lives.
fn profile_directory() -> PathBuf {
    std::env::current_exe()
        .expect("locate the test executable")
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the test binary sits in a deps directory")
        .to_path_buf()
}

fn library_path() -> PathBuf {
    #[cfg(windows)]
    let file = "xt_kernel_dynlib.dll";
    #[cfg(target_os = "macos")]
    let file = "libxt_kernel_dynlib.dylib";
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let file = "libxt_kernel_dynlib.so";
    profile_directory().join(file)
}

/// The workspace root, two levels above this crate's manifest.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
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

/// The target this test binary was built for, when it is not the host.
fn build_target() -> Option<std::ffi::OsString> {
    let profile = profile_directory();
    let relative = profile.strip_prefix(target_directory()).ok()?;
    let mut parts = relative.components();
    let target = parts.next()?.as_os_str().to_os_string();
    parts.next().map(|_| target)
}

/// The newest modification time among the sources the library is built from.
fn newest_source() -> std::time::SystemTime {
    fn latest(directory: &Path, newest: &mut Option<std::time::SystemTime>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                latest(&path, newest);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
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
    for directory in ["src", "demo/api/src", "demo/impl/src", "demo/kernel/src"] {
        latest(&root().join(directory), &mut newest);
    }
    newest.expect("the tree holds sources")
}

/// Builds the library when it is missing or older than the sources it is built
/// from.
///
/// Cargo builds a library target only when it is asked for one, so `cargo test`
/// alone leaves the dynamic library behind. Building it here keeps the test from
/// reporting on an artifact that no longer matches the tree, and names the target
/// this test binary was built for so a run for another target does not refresh
/// the host's copy instead.
fn ensure_built() {
    let path = library_path();
    let newest = newest_source();
    let fresh = std::fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified >= newest);
    if fresh {
        return;
    }
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(root())
        .args(["build", "-p", "xt_kernel_dynlib"]);
    if let Some(target) = build_target() {
        command.arg("--target").arg(target);
    }
    let status = command.status().expect("start cargo to build the library");
    assert!(status.success(), "building {} failed", path.display());
}

/// Reports whether the operating system still has the library mapped.
#[cfg(windows)]
fn is_mapped(path: &std::path::Path) -> bool {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut core::ffi::c_void;
    }

    let name: Vec<u16> = path
        .file_name()
        .expect("the path names a file")
        .encode_wide()
        .chain(core::iter::once(0))
        .collect();
    // SAFETY: the name is a NUL-terminated wide string that outlives the call.
    !unsafe { GetModuleHandleW(name.as_ptr()) }.is_null()
}

#[cfg(not(windows))]
fn is_mapped(path: &std::path::Path) -> bool {
    // No portable way to ask, so the cycle is what this build checks.
    let _ = path;
    true
}

#[test]
fn a_path_with_an_interior_nul_is_refused_before_the_loader_sees_it() {
    // A C string ends at the first zero, so a loader handed this path would
    // open a different file than the caller named.
    let error = Library::open(Path::new("a\0b")).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
}

#[test]
fn the_path_a_library_reports_is_the_one_it_was_opened_from() {
    ensure_built();
    let named = library_path();
    let library = Library::open(&named).unwrap();
    // The loader is never given a name it has to resolve itself, so the path the
    // handle reports is the absolute one it opened.
    assert!(library.path().is_absolute());
    assert_eq!(library.path().file_name(), named.file_name());
}

#[test]
fn a_plugin_is_used_and_unloaded_before_the_next_copy_is_loaded() {
    ensure_built();
    let path = library_path();

    for round in 0..3 {
        let plugin = Plugin::load(&path).unwrap_or_else(|error| {
            panic!("round {round}: {error}");
        });

        // A call that returns an owned value: the string is allocated by the
        // library and released through it, so both have to happen before the
        // unload below.
        let band = unsafe { plugin.workspace().classify(150) }.unwrap();
        assert_eq!(band.as_str(), "hot", "round {round}");

        // A second object reached through the first, dropped while the library
        // is still mapped.
        let mut session = unsafe {
            plugin
                .workspace()
                .open_session(StableStr::from_str("mod"))
        }
        .unwrap();
        let total = unsafe { session.add(StableStr::from_str("one two three")) }.unwrap();
        assert_eq!(total, 3, "round {round}");
        drop(session);

        assert!(is_mapped(&path), "round {round}: the library is not mapped");
        // Dropping the plugin releases the object, runs the shutdown hook, and
        // only then unloads the library.
        drop(plugin);
        assert!(
            !is_mapped(&path),
            "round {round}: the library stayed mapped after the plugin was dropped"
        );
    }
}

/// Private bytes the process holds now.
#[cfg(windows)]
fn private_bytes() -> usize {
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
    }

    #[link(name = "psapi")]
    unsafe extern "system" {
        fn GetProcessMemoryInfo(
            process: *mut core::ffi::c_void,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }

    let mut counters = Counters {
        cb: core::mem::size_of::<Counters>() as u32,
        page_faults: 0,
        peak_working_set: 0,
        working_set: 0,
        quota_peak_paged: 0,
        quota_paged: 0,
        quota_peak_nonpaged: 0,
        quota_nonpaged: 0,
        pagefile: 0,
        peak_pagefile: 0,
    };
    // SAFETY: the counters value is writable for the size it reports.
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    counters.pagefile
}

/// Measures what a load/unload cycle leaves behind. Not part of the gate: it
/// takes seconds and its number moves with the allocator.
#[cfg(windows)]
#[test]
#[ignore = "a measurement, not a check"]
fn measure_private_bytes_per_cycle() {
    ensure_built();
    let path = library_path();
    // Warm up, so the first measurement does not include one-time runtime costs.
    for _ in 0..20 {
        let plugin = Plugin::load(&path).unwrap();
        let _ = unsafe { plugin.workspace().classify(1) }.unwrap();
    }

    const BATCH: usize = 2000;
    const BATCHES: usize = 5;
    let mut previous = private_bytes();

    // A cycle that never calls a method.
    for batch in 0..BATCHES {
        for _ in 0..BATCH {
            let plugin = Plugin::load(&path).unwrap();
            drop(plugin);
        }
        let now = private_bytes();
        println!(
            "load/drop  batch {batch}: {:>8} bytes",
            now as i64 - previous as i64
        );
        previous = now;
    }

    // A cycle that calls a method, so the library builds the table it needs.
    for batch in 0..BATCHES {
        for _ in 0..BATCH {
            let plugin = Plugin::load(&path).unwrap();
            let _ = unsafe { plugin.workspace().classify(1) }.unwrap();
        }
        let now = private_bytes();
        println!(
            "load/call  batch {batch}: {:>8} bytes",
            now as i64 - previous as i64
        );
        previous = now;
    }

    // A cycle that also opens a session and reaches the object it hands out, so
    // three caches build a table inside the library.
    for batch in 0..BATCHES {
        for _ in 0..BATCH {
            let plugin = Plugin::load(&path).unwrap();
            let session = unsafe {
                plugin
                    .workspace()
                    .open_session(StableStr::from_str("mod"))
            }
            .unwrap();
            let child = unsafe { session.child() }.unwrap();
            let _ = unsafe { child.value() };
            drop(child);
            drop(session);
            drop(plugin);
        }
        let now = private_bytes();
        println!(
            "load/session batch {batch}: {:>8} bytes",
            now as i64 - previous as i64
        );
        previous = now;
    }

    // A cycle that only maps and unmaps the file, with no code of ours running
    // inside it.
    for batch in 0..BATCHES {
        for _ in 0..BATCH {
            let library = Library::open(&path).unwrap();
            drop(library);
        }
        let now = private_bytes();
        println!(
            "map/unmap  batch {batch}: {:>8} bytes",
            now as i64 - previous as i64
        );
        previous = now;
    }
}
