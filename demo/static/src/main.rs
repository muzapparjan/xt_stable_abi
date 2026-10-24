//! A program that uses the published surface, with the library linked in.
//!
//! Nothing here opens a file or looks up a symbol. The implementation crate is a
//! dependency, so its entry point is an ordinary function this program calls,
//! and the protocol after that call is the one the dynamic program runs too:
//! descriptor, adoption, then the same accessors through the same tables.
//!
//! The comparison this program exists for is therefore exact. It differs from
//! `xt_kernel_app` in one thing only - the library is part of the binary instead
//! of a file that is opened - and from `xt_kernel_native` in one thing only - the
//! calls cross a boundary instead of reaching the kernel directly.

use std::process::ExitCode;

use xt_bench::Report;
use xt_kernel_api::{
    load, AbiAllocator, HostServices, LoadedPlugin, WorkspaceObject, ABI_VERSION,
};

fn main() -> ExitCode {
    let mut report = Report::new();

    let host = HostServices::new(AbiAllocator::system());
    // SAFETY: the entry point belongs to a crate built against this protocol, and
    // the host description outlives the call.
    let descriptor = match unsafe { load(xt_kernel_impl::entry, &host, ABI_VERSION) } {
        Ok(descriptor) => descriptor,
        Err(error) => {
            eprintln!("the library refused the host: {error}");
            return ExitCode::from(2);
        }
    };
    let mut loaded = LoadedPlugin::new(descriptor);
    report.is("the descriptor names the service", loaded.name(), "xt_kernel");
    report.is(
        "the library version",
        loaded.descriptor().plugin_version,
        xt_kernel_api::published_version(),
    );
    report.is(
        "the protocol version",
        loaded.descriptor().header.version,
        ABI_VERSION,
    );

    let workspace = match WorkspaceObject::abi_try_from_stable(loaded.take_api()) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("the object cannot be adopted: {error}");
            return ExitCode::from(2);
        }
    };
    xt_surface_checks::run(&workspace, &mut report);

    // The object is released first, then the descriptor runs its hook: the same
    // order the dynamic program relies on, visible here without a library ever
    // being unloaded.
    drop(workspace);
    drop(loaded);

    // SAFETY: the function only reads a static.
    let shutdowns = unsafe { xt_kernel_impl::shutdowns() };
    report.is(
        "the shutdown hook ran once, while the object was released first",
        shutdowns,
        1,
    );

    if report.finish() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
