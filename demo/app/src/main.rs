//! A program that uses the kernel through the published surface.
//!
//! The program links [`xt_kernel_api`] and nothing else. The kernel itself is
//! not compiled into it: the implementation arrives as a dynamic library, which
//! this program finds next to its own executable, opens, and asks for the
//! object. Everything below therefore exercises the boundary, not a direct
//! function call.
//!
//! This is the first of three programs over the same workload.
//! `xt_kernel_static` runs the same scenario list against a boundary that is
//! linked in, and `xt_kernel_native` runs it against the kernel with no boundary
//! at all, so the three reports can be placed side by side.

use core::mem;
use std::path::PathBuf;
use std::process::ExitCode;

use xt_bench::Report;
use xt_kernel_api::loader::{Library, Plugin};
use xt_kernel_api::{ABI_VERSION, SERVICE_NAME};

fn main() -> ExitCode {
    let path = match library_path() {
        Ok(path) => path,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    println!("loading {}", path.display());

    // A second handle on the same library lets the program read the shutdown
    // counter after the plugin is gone: the library is unloaded only when the
    // last handle drops, which is exactly what the handle order is meant to
    // guarantee.
    let counter_library = match Library::open(&path) {
        Ok(library) => library,
        Err(error) => {
            eprintln!("could not open {}: {error}", path.display());
            return ExitCode::from(2);
        }
    };
    let counter_symbol = counter_library.symbol("xt_kernel_shutdowns");
    if counter_symbol.is_null() {
        eprintln!("the library does not export xt_kernel_shutdowns");
        return ExitCode::from(2);
    }
    // SAFETY: the symbol is the counter the library documents.
    let shutdowns: unsafe extern "C" fn() -> u64 = unsafe { mem::transmute(counter_symbol) };
    if unsafe { shutdowns() } != 0 {
        eprintln!("the library reported a shutdown before it was loaded");
        return ExitCode::from(2);
    }

    let mut report = Report::new();
    {
        let plugin = match Plugin::load(&path) {
            Ok(plugin) => plugin,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::from(2);
            }
        };
        report.is("the descriptor names the service", plugin.name(), SERVICE_NAME);
        report.is(
            "the library version",
            plugin.version(),
            xt_kernel_api::published_version(),
        );
        report.is("the protocol version", plugin.protocol(), ABI_VERSION);

        xt_surface_checks::run(plugin.workspace(), &mut report);
    }

    // SAFETY: the second handle keeps the library loaded, and the counter is
    // still the one this module exports.
    let after = unsafe { shutdowns() };
    report.is(
        "the shutdown hook ran once, while the object was released first",
        after,
        1,
    );
    drop(counter_library);

    if report.finish() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Finds the library: a path given on the command line, or the file next to
/// this executable, which is where a deployment puts it.
fn library_path() -> Result<PathBuf, String> {
    if let Some(argument) = std::env::args().nth(1) {
        return Ok(PathBuf::from(argument));
    }
    let name = if cfg!(windows) {
        "xt_kernel_dynlib.dll"
    } else if cfg!(target_os = "macos") {
        "libxt_kernel_dynlib.dylib"
    } else {
        "libxt_kernel_dynlib.so"
    };
    let executable = std::env::current_exe()
        .map_err(|error| format!("cannot find this executable: {error}"))?;
    let directory = executable
        .parent()
        .ok_or_else(|| "this executable has no directory".to_owned())?;
    let beside = directory.join(name);
    if beside.exists() {
        return Ok(beside);
    }
    Err(format!(
        "{} is not next to this program; build it with `cargo build -p xt_kernel_dynlib`, \
         or pass its path as the first argument",
        name
    ))
}
