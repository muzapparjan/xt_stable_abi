//! The surface `xt_kernel` is published through.
//!
//! This crate was written after the kernel, not before it: every method here is
//! a promise the kernel already keeps, and the list below is what the kernel's
//! own review said a foreign caller should be able to ask for.
//!
//! # What is promised
//!
//! [`Workspace`] is the entry object a library hands over. It answers about
//! text (`analyze`, `tokens`, `top_terms`, `scores`, `scan`, `normalize`), about
//! bytes (`digest`), about graphs (`reachable`, `topological`), about a number
//! (`classify`, `split_fixed`), about the build that produced it
//! (`capabilities`, `stats`, `now`), and it opens a [`Session`] that accumulates
//! texts. A session hands out a [`Child`] so a host can reach a second object
//! through the first.
//!
//! # What is kept inside
//!
//! The kernel's `Token<'a>`, `TermStats`, `TopK`, `Graph`, `Cursor`, `Fixed`,
//! `Flags`, `Level`, `Analyzer` and `KernelError` never cross the boundary. A
//! borrowed token would have to outlive the call that produced it, a `TopK` is a
//! heap with no layout promise, and a `Graph` owns a map the other side cannot
//! release. What a caller gets instead is text, numbers, tuples of them, and an
//! [`AbiError`] for the one operation that can fail on purpose.
//!
//! The kernel's own `Engine` is also not published: a host asks for results,
//! not for the machine that computes them, which is what lets the library behind
//! this surface change without changing this crate.
//!
//! # Loading a library
//!
//! [`loader::Plugin`] opens a dynamic library, calls its entry point and adopts
//! the object it publishes. A program that links this crate and nothing else can
//! therefore use the kernel without containing a line of it.

#![deny(missing_docs)]

pub mod loader;

pub use xt_stable_abi::{
    load, stable_object, stable_object_guarded, release_cached_tables, AbiAllocator, AbiError,
    AbiLogLevel, AbiResult, AbiStatus, AbiType, AbiVersion, EntryPointFn, HostServices,
    LoadedPlugin, PluginDescriptor, StableDyn, StableElement, StableOption, StablePair,
    StableResult, StableSlice, StableStr, StableString, StableSystemTime, StableVec, ABI_VERSION,
};

/// The name a library stores in its descriptor.
pub const SERVICE_NAME: &str = "xt_kernel";

/// The symbol a library exports its entry point under.
pub const ENTRY_SYMBOL: &str = "xt_kernel_entry";

/// The identifier the workspace object is registered under.
pub const WORKSPACE_TYPE: &str = "Workspace";

/// The identifier a session object is registered under.
pub const SESSION_TYPE: &str = "Session";

/// The identifier a child object is registered under.
pub const CHILD_TYPE: &str = "Child";

/// The version this surface is published at.
pub fn published_version() -> AbiVersion {
    AbiVersion::new(0, 1)
}

/// The identifier the workspace object is registered under.
pub fn workspace_type_id() -> u64 {
    xt_stable_abi::object::type_id_of(WORKSPACE_TYPE)
}

/// The identifier a session object is registered under.
pub fn session_type_id() -> u64 {
    xt_stable_abi::object::type_id_of(SESSION_TYPE)
}

/// The identifier a child object is registered under.
pub fn child_type_id() -> u64 {
    xt_stable_abi::object::type_id_of(CHILD_TYPE)
}

stable_object_guarded! {
    /// The object a library publishes.
    pub struct WorkspaceObject;
    /// The table behind [`WorkspaceObject`].
    pub struct WorkspaceVTable;
    /// Everything a host may ask the kernel about.
    pub trait Workspace {
        /// A one-line summary of `text`.
        fn analyze(&self, text: StableStr<'_>) -> StableString;
        /// The tokens of `text`, in order.
        fn tokens(&self, text: StableStr<'_>) -> StableVec<StableString>;
        /// The `limit` most frequent terms of `text`, most frequent first.
        fn top_terms(&self, text: StableStr<'_>, limit: u32) -> StableVec<StablePair<StableString, u64>>;
        /// Both checksums of `bytes`, as `fnv:crc` in hexadecimal.
        fn digest(&self, bytes: StableSlice<'_, u8>) -> StableString;
        /// `text` with case and whitespace folded.
        fn normalize(&self, text: StableStr<'_>) -> StableString;
        /// The nodes reachable from `start`, in visit order.
        fn reachable(&self, edges: StableVec<StablePair<u64, u64>>, start: u64) -> StableVec<u64>;
        /// An order in which every edge points forward.
        fn topological(&self, edges: StableVec<StablePair<u64, u64>>) -> StableResult<StableVec<u64>, AbiError>;
        /// The band a number falls into.
        fn classify(&self, value: i64) -> StableString;
        /// The whole and fractional parts of a fixed-point number.
        fn split_fixed(&self, micros: i64) -> StablePair<i64, i64>;
        /// A description of the byte walker's result.
        fn scan(&self, text: StableStr<'_>) -> StableString;
        /// Every analyzer's score for `text`.
        fn scores(&self, text: StableStr<'_>) -> StableString;
        /// What the library has done since it was loaded.
        fn stats(&self) -> StableString;
        /// What the loaded build offers.
        fn capabilities(&self) -> StableVec<StableString>;
        /// The current time, in UTC.
        fn now(&self) -> StableSystemTime;
        /// Opens a session that accumulates texts.
        fn open_session(&self, name: StableStr<'_>) -> SessionObject;
    }
}

stable_object_guarded! {
    /// A handle to a session opened by a workspace.
    pub struct SessionObject;
    /// The table behind [`SessionObject`].
    pub struct SessionVTable;
    /// A collection of texts analysed together.
    pub trait Session {
        /// Adds `text` and answers with the running total of terms.
        fn add(&mut self, text: StableStr<'_>) -> u64;
        /// The number of terms in every text added so far.
        fn total(&self) -> u64;
        /// The name the session was opened with.
        fn name(&self) -> StableString;
        /// A line describing the session and everything it holds.
        fn summary(&self) -> StableString;
        /// A second object, reached through this one.
        fn child(&self) -> ChildObject;
        /// Takes every text of `other` into this session.
        fn merge(&mut self, other: SessionObject) -> u64;
        /// A deliberate failure, so a host can see a panic stay on this side.
        fn explode(&self) -> u64;
        /// Closes the session and describes it.
        fn finish(self) -> StableString;
    }
}

stable_object! {
    /// A handle to the object a session hands out.
    pub struct ChildObject;
    /// The table behind [`ChildObject`].
    pub struct ChildVTable;
    /// What the child offers.
    pub trait Child {
        /// The number the child was built with.
        fn value(&self) -> u64;
    }
}
