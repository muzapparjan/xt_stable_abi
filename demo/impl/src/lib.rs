//! The library that keeps the promises of [`xt_kernel_api`] with `xt_kernel`.
//!
//! Every method here forwards to the kernel; nothing is re-implemented. The
//! translation the methods do is the one the boundary requires: kernel values
//! are copied into the published string and sequence types, allocated through
//! the allocator the host offered, and kernel errors become [`AbiError`] with
//! the kernel's message.
//!
//! The workspace and session objects are declared with
//! [`stable_object_guarded`](xt_kernel_api::stable_object_guarded), so a panic
//! inside the kernel is reported to the host as [`AbiStatus::Panic`] instead of
//! unwinding into the host's frame. The child object is plain data and uses the
//! unguarded declaration.
//!
//! This crate is the implementation as a library. The deliverable is the
//! `xt_kernel_dynlib` crate, which is a dynamic library and nothing else: it
//! exports [`entry`] and [`shutdowns`] under the names a host looks up, and
//! contains no code of its own. A program that wants the same protocol without a
//! dynamic library - which the reference programs use to isolate what the
//! dynamic loading itself costs - depends on this crate instead.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use xt_kernel::{Engine, KernelError, Session as KernelSession};
use xt_kernel_api::{
    child_type_id, published_version, release_cached_tables, session_type_id, workspace_type_id,
    AbiAllocator, AbiError, AbiLogLevel, AbiStatus, AbiVersion, Child, ChildObject, HostServices,
    PluginDescriptor, Session, SessionObject, StablePair, StableResult, StableSlice, StableStr,
    StableString, StableSystemTime, StableVec, Workspace, WorkspaceObject, ABI_VERSION,
    SERVICE_NAME,
};

/// How many times the shutdown hook ran.
static SHUTDOWNS: AtomicU64 = AtomicU64::new(0);

/// Copies `value` into a string the host will release.
fn owned(allocator: AbiAllocator, value: &str) -> StableString {
    StableString::from_str_with_allocator(allocator, value)
        .expect("the host allocator can hold a name")
}

/// Copies a list of strings into a sequence the host will release.
fn strings(
    allocator: AbiAllocator,
    values: impl IntoIterator<Item = String>,
) -> StableVec<StableString> {
    let mut out = StableVec::new_with_allocator(allocator);
    for value in values {
        out.try_push(owned(allocator, &value))
            .expect("the host allocator can hold a list");
    }
    out
}

/// Copies a list of numbers into a sequence the host will release.
fn numbers(allocator: AbiAllocator, values: impl IntoIterator<Item = u64>) -> StableVec<u64> {
    let mut out = StableVec::new_with_allocator(allocator);
    for value in values {
        out.try_push(value)
            .expect("the host allocator can hold a list");
    }
    out
}

/// Reads the edge list a host passed in.
fn edges(values: &StableVec<StablePair<u64, u64>>) -> Vec<(u64, u64)> {
    values
        .as_slice()
        .iter()
        .map(|pair| (pair.left, pair.right))
        .collect()
}

/// Turns a kernel failure into the published error shape.
///
/// The kind of failure crosses; the kernel's own sentence does not. Its text can
/// carry counts and offsets from inside the library, and an error is the one
/// value that is written to a log by whoever caught it.
fn failed(error: KernelError) -> AbiError {
    let message = match error {
        KernelError::GraphCycle { .. } => "the graph has a cycle",
        KernelError::UnknownNode(_) => "an edge names a node that does not exist",
        KernelError::EmptyInput => "the input is empty",
        _ => "the request could not be completed",
    };
    AbiError::with_message(AbiStatus::InvalidArgument, message)
}

/// The workspace object's implementation.
struct KernelWorkspace {
    engine: Arc<Engine>,
    allocator: AbiAllocator,
}

impl Workspace for KernelWorkspace {
    fn analyze(&self, text: StableStr<'_>) -> StableString {
        owned(self.allocator, &self.engine.analyze(text.as_str()).render())
    }

    fn tokens(&self, text: StableStr<'_>) -> StableVec<StableString> {
        strings(self.allocator, self.engine.tokens(text.as_str()))
    }

    fn top_terms(
        &self,
        text: StableStr<'_>,
        limit: u32,
    ) -> StableVec<StablePair<StableString, u64>> {
        let mut out = StableVec::new_with_allocator(self.allocator);
        for (term, count) in self.engine.top_terms(text.as_str(), limit as usize) {
            out.try_push(StablePair::new(owned(self.allocator, &term), count))
                .expect("the host allocator can hold a ranking");
        }
        out
    }

    fn digest(&self, bytes: StableSlice<'_, u8>) -> StableString {
        owned(self.allocator, &self.engine.digest(bytes.as_slice()))
    }

    fn normalize(&self, text: StableStr<'_>) -> StableString {
        owned(self.allocator, &self.engine.normalize(text.as_str()))
    }

    fn reachable(&self, pairs: StableVec<StablePair<u64, u64>>, start: u64) -> StableVec<u64> {
        numbers(self.allocator, self.engine.reachable(&edges(&pairs), start))
    }

    fn topological(
        &self,
        pairs: StableVec<StablePair<u64, u64>>,
    ) -> StableResult<StableVec<u64>, AbiError> {
        match self.engine.topological(&edges(&pairs)) {
            Ok(order) => StableResult::ok(numbers(self.allocator, order)),
            Err(error) => StableResult::err(failed(error)),
        }
    }

    fn classify(&self, value: i64) -> StableString {
        owned(self.allocator, &self.engine.classify(value))
    }

    fn split_fixed(&self, micros: i64) -> StablePair<i64, i64> {
        let (whole, fraction) = self.engine.split_fixed(micros);
        StablePair::new(whole, fraction)
    }

    fn scan(&self, text: StableStr<'_>) -> StableString {
        owned(self.allocator, &self.engine.scan_line(text.as_str()))
    }

    fn scores(&self, text: StableStr<'_>) -> StableString {
        owned(self.allocator, &self.engine.analysis_line(text.as_str()))
    }

    fn stats(&self) -> StableString {
        owned(self.allocator, &self.engine.stats())
    }

    fn capabilities(&self) -> StableVec<StableString> {
        strings(self.allocator, self.engine.capabilities())
    }

    fn now(&self) -> StableSystemTime {
        let (seconds, nanos) = self.engine.now().expect("the clock is readable");
        StableSystemTime::new(seconds, nanos).expect("the clock produces a valid point")
    }

    fn open_session(&self, name: StableStr<'_>) -> SessionObject {
        SessionObject::abi_try_from_value_with(
            KernelSessionImpl {
                inner: self.engine.session(name.as_str()),
                allocator: self.allocator,
            },
            session_type_id(),
            self.allocator,
        )
        .expect("the session object can be allocated")
    }
}

/// The session object's implementation.
struct KernelSessionImpl {
    inner: KernelSession,
    allocator: AbiAllocator,
}

impl Session for KernelSessionImpl {
    fn add(&mut self, text: StableStr<'_>) -> u64 {
        self.inner.add(text.as_str())
    }

    fn total(&self) -> u64 {
        self.inner.total()
    }

    fn name(&self) -> StableString {
        owned(self.allocator, self.inner.name())
    }

    fn summary(&self) -> StableString {
        owned(self.allocator, &self.inner.summary())
    }

    fn child(&self) -> ChildObject {
        ChildObject::abi_try_from_value_with(
            ChildImpl {
                value: self.inner.total(),
            },
            child_type_id(),
            self.allocator,
        )
        .expect("the child object can be allocated")
    }

    fn merge(&mut self, other: SessionObject) -> u64 {
        // The peer is read through the surface it publishes, not through its
        // internals: the two objects need not have been built by the same
        // revision of this library.
        // SAFETY: the host hands over an object it owns, built from this
        // declaration.
        let total = unsafe { other.total() }.expect("the peer session answers");
        // SAFETY: the same guarantee covers the name.
        let name = unsafe { other.name() }
            .expect("the peer session answers")
            .into_string();
        self.inner.absorb(&name, total)
    }

    fn explode(&self) -> u64 {
        self.inner.explode()
    }

    fn finish(self) -> StableString {
        let allocator = self.allocator;
        let text = self.inner.finish();
        owned(allocator, &text)
    }
}

/// The child object's implementation.
struct ChildImpl {
    value: u64,
}

impl Child for ChildImpl {
    fn value(&self) -> u64 {
        self.value
    }
}

unsafe extern "C" fn shutdown() {
    SHUTDOWNS.fetch_add(1, Ordering::SeqCst);
    // The tables this load built were allocated from the process allocator and
    // only this module can reach them, so they have to go back before the host
    // unloads it. Nothing built from them is alive: the host released the
    // descriptor's object, and every object a method handed out, before calling
    // this hook.
    // SAFETY: the host unloads the module only after this returns, and it
    // released what it adopted before calling it.
    unsafe { release_cached_tables() };
}

/// The entry point a host calls, once the library carries it under a name.
///
/// # Safety
///
/// `host` must be null or point at a valid [`HostServices`], and `out` must be
/// null or point at a descriptor buffer the host provided.
pub unsafe extern "C" fn entry(
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
    // SAFETY: the host guarantees valid values for the duration of the call.
    let host = unsafe { &*host };
    // The description has to cover every field read below. A host built against
    // an older revision describes fewer bytes, and the fields past that point
    // are not there to read.
    if !host.covers(core::mem::size_of::<HostServices>() as u64) {
        return AbiStatus::InvalidDescriptor.code();
    }
    // The buffer has to hold the layout this build writes. `store` checks this
    // too; checking here as well keeps the workspace below from being built only
    // to be refused.
    // SAFETY: `out` is non-null, and the host guarantees it names a buffer.
    let capacity = unsafe { (*out).header.capacity };
    if capacity < core::mem::size_of::<PluginDescriptor>() as u64 {
        return AbiStatus::InvalidDescriptor.code();
    }
    host.log(AbiLogLevel::Info, "xt_kernel: entering");

    let workspace = match WorkspaceObject::abi_try_from_value_with(
        KernelWorkspace {
            engine: Arc::new(Engine::new()),
            allocator: host.allocator,
        },
        workspace_type_id(),
        host.allocator,
    ) {
        Ok(object) => object,
        Err(error) => return error.status().code(),
    };

    // SAFETY: the host provides a writable descriptor buffer.
    unsafe {
        PluginDescriptor::store(
            out,
            SERVICE_NAME,
            published_version(),
            workspace.abi_into_stable(),
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
pub unsafe extern "C" fn shutdowns() -> u64 {
    SHUTDOWNS.load(Ordering::SeqCst)
}
