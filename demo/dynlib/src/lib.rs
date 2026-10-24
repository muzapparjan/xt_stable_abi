//! The dynamic library a host loads.
//!
//! This crate exists to be one thing: a file with two exported names. Every
//! promise it keeps is kept by [`xt_kernel_impl`], which is a library; this
//! crate calls into it and does nothing else.
//!
//! The split is deliberate. A dynamic library is the only artifact that may be
//! published: an `rlib` carries metadata that holds source-level information,
//! so a crate that is meant to be delivered must not also produce one. Keeping
//! the implementation in a separate library crate means the deliverable can
//! declare `crate-type = ["cdylib"]` and never have an `rlib` to give away by
//! accident.

// MSVC reports the import library it creates for a cdylib; nothing is wrong
// with the artifact.
#![allow(linker_messages)]

use xt_kernel_api::{AbiVersion, HostServices, PluginDescriptor};

/// The entry point the host looks up by name.
///
/// # Safety
///
/// `host` must be null or point at a valid [`HostServices`], and `out` must be
/// null or point at a descriptor buffer the host provided.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xt_kernel_entry(
    host: *const HostServices,
    required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32 {
    // SAFETY: the caller states the two pointers are valid or null, which is the
    // contract the implementation checks.
    unsafe { xt_kernel_impl::entry(host, required, out) }
}

/// Reports how many times the shutdown hook ran, for the host's assertion.
///
/// # Safety
///
/// The function only reads a static and is safe to call from the host.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xt_kernel_shutdowns() -> u64 {
    // SAFETY: the implementation only reads a static.
    unsafe { xt_kernel_impl::shutdowns() }
}
