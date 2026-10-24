//! The protocol a dynamically loaded library uses to introduce itself.
//!
//! The host builds a [`HostServices`] value and calls an [`EntryPointFn`]
//! exported by the library. The library fills a [`PluginDescriptor`]: its
//! name, its own version, and an object that implements whatever trait the two
//! sides agreed on.
//!
//! The descriptor is written into a buffer the host owns. The host records the
//! buffer's byte capacity in the descriptor header before the call, and the
//! writer records how many bytes it actually wrote, so both sides can detect a
//! mismatch instead of reading past the buffer.

use core::ffi::c_void;
use core::fmt;
use core::mem::size_of;

use crate::heap::boxed::Box;

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::object::{StableDyn, LAYOUT_REVISION};
use crate::string::{StableStr, StableString};
use crate::version::AbiVersion;

/// The protocol version this crate implements.
pub const ABI_VERSION: AbiVersion = AbiVersion::new(1, 0);

/// How severe a message is.
#[repr(u32)]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AbiLogLevel {
    /// A failure the host should report.
    Error = 0,
    /// A condition worth noticing but not fatal.
    Warning = 1,
    /// Progress information.
    Info = 2,
    /// Detail useful while diagnosing.
    Debug = 3,
}

impl AbiLogLevel {
    /// The discriminant written into the call.
    pub const fn code(self) -> u32 {
        self as u32
    }

    /// Rebuilds a level from its discriminant.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(AbiLogLevel::Error),
            1 => Some(AbiLogLevel::Warning),
            2 => Some(AbiLogLevel::Info),
            3 => Some(AbiLogLevel::Debug),
            _ => None,
        }
    }
}

/// A sink the library may write messages to.
///
/// The level arrives as the `u32` a [`AbiLogLevel`] defines, so a level this
/// build does not know is a number rather than an invalid [`AbiLogLevel`].
pub type LogFn = unsafe extern "C" fn(context: *mut c_void, level: u32, message: StableStr<'_>);

/// Answers with the address of a service the host offers.
///
/// The identifier is chosen by the host and documented by whichever side
/// publishes it. A null answer means the host does not offer that service. The
/// pointed-to value stays alive while the host is loaded.
pub type QueryFn = unsafe extern "C" fn(context: *mut c_void, identifier: u64) -> *const ();

/// The services the host offers to the library.
///
/// The value is built by the host and passed by pointer to the entry point. It
/// stays valid for the duration of the call; a library that keeps the pointer
/// must keep the value alive just as long.
///
/// A callback is an `Option` rather than a bare pointer: a host that fills no
/// sink writes `None`, which occupies the same word and is a value this build
/// defines, while a zero word read as a function pointer would be an invalid
/// value before anything could refuse it.
#[repr(C)]
pub struct HostServices {
    /// The number of bytes this value occupies.
    ///
    /// A library compares it with the size it was built against before reading
    /// any later field, so a host that appends a field stays readable by a
    /// library that only knows the prefix. The comparison is made through
    /// [`HostServices::described_size`], which reads this field alone: forming
    /// `&HostServices` over a description that is shorter than this build's
    /// would claim bytes the host never provided.
    pub size: u64,
    /// The version the host implements.
    pub version: AbiVersion,
    /// An allocator the library may use for memory the host will release.
    pub allocator: AbiAllocator,
    /// A message sink, or `None` when the host has none.
    pub log: Option<LogFn>,
    /// The value passed to `log` and `query` as their first argument.
    pub context: *mut c_void,
    /// Answers with the address of a service the host offers, or `None` when the
    /// host offers none.
    ///
    /// This is the channel for anything beyond allocation and logging: a host
    /// object, a callback table, a clock. The library asks by identifier and
    /// reads the value with the layout the two sides agreed on.
    pub query: Option<QueryFn>,
    /// The boundary layout revision this value was built against.
    ///
    /// `size` says how many bytes the description covers; this says what those
    /// bytes mean. A host that reorders fields of this structure keeps the same
    /// size and the same major version, so neither of those checks would notice;
    /// comparing this number catches it, because a build only produces the one
    /// revision its own headers declare.
    ///
    /// The field sits at the end so that a value built before it existed still
    /// has every earlier field at the offset this build reads: a library that
    /// only knows the prefix reads the fields it knows correctly and, having
    /// compared `size` against the prefix it was built for, never looks at this
    /// field. A library this build produces compares it, because the size check
    /// it makes covers the whole value.
    pub layout_revision: u64,
}

impl HostServices {
    /// The number of bytes the description at `host` says it occupies, or `None`
    /// when `host` is null.
    ///
    /// This is the check a library makes before it reads any other field, and it
    /// is the reason it exists as a function rather than as a method: a host
    /// built against an older revision describes fewer bytes, and forming a
    /// `&HostServices` over those bytes would claim storage the host never
    /// provided. Only the first field is read here.
    ///
    /// ```ignore
    /// // SAFETY: `host` is null or points at a description whose first field is
    /// // a `u64` this side may read.
    /// let described = match unsafe { HostServices::described_size(host) } {
    ///     Some(size) => size,
    ///     None => return AbiStatus::NullPointer.code(),
    /// };
    /// if described < core::mem::size_of::<HostServices>() as u64 {
    ///     return AbiStatus::InvalidDescriptor.code();
    /// }
    /// // SAFETY: the description covers every field, as checked above.
    /// let host = unsafe { &*host };
    /// ```
    ///
    /// # Safety
    ///
    /// `host` must be null, or point at a value whose first field is the `u64`
    /// this type declares there.
    pub unsafe fn described_size(host: *const Self) -> Option<u64> {
        if host.is_null() {
            return None;
        }
        // SAFETY: the caller says the first field is readable as a `u64`, and
        // this reads that field and nothing after it.
        Some(unsafe { core::ptr::read_unaligned(core::ptr::addr_of!((*host).size)) })
    }

    /// Builds a host description with no sink and no query.
    pub const fn new(allocator: AbiAllocator) -> Self {
        Self {
            size: size_of::<HostServices>() as u64,
            version: ABI_VERSION,
            allocator,
            log: None,
            context: core::ptr::null_mut(),
            query: None,
            layout_revision: LAYOUT_REVISION,
        }
    }

    /// Reports whether the description covers `needed` bytes.
    ///
    /// A caller that has not yet formed this value uses
    /// [`HostServices::described_size`] instead; this form is for a host
    /// checking its own description, or for a library that has already compared
    /// the size and found the whole value there.
    pub fn covers(&self, needed: u64) -> bool {
        self.size >= needed
    }

    /// Writes `message` to the sink, and does nothing when the host offered
    /// none.
    pub fn log(&self, level: AbiLogLevel, message: &str) {
        let Some(log) = self.log else {
            return;
        };
        // SAFETY: `log` is a function pointer supplied with this value, and the
        // borrowed text outlives the call.
        unsafe { log(self.context, level.code(), StableStr::from_str(message)) }
    }

    /// The address of the service identified by `identifier`, if the host
    /// offers one.
    ///
    /// The pointer is what the host's query returned; reading it is the
    /// library's responsibility, with the layout the two sides agreed on.
    pub fn service(&self, identifier: u64) -> *const () {
        let Some(query) = self.query else {
            return core::ptr::null();
        };
        // SAFETY: `query` is a function pointer supplied with this value.
        unsafe { query(self.context, identifier) }
    }
}

impl Default for HostServices {
    fn default() -> Self {
        Self::new(AbiAllocator::system())
    }
}

impl fmt::Debug for HostServices {
    // The message sink, the query function and the context are addresses in the
    // host process. Printing them would put the layout of that process into a
    // log, which is the same reason `AbiAllocator` prints nothing.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostServices")
            .field("size", &self.size)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// The first field of any descriptor that may gain fields later.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbiHeader {
    /// How many bytes the receiving side provided.
    ///
    /// The host writes this before the call; it bounds what the writer may
    /// touch.
    pub capacity: u64,
    /// How many bytes the writer produced.
    ///
    /// The writer records this. A reader accepts any value between
    /// [`MIN_DESCRIPTOR_SIZE`] and `capacity`, so a library built against an
    /// earlier revision that knows fewer fields can still answer.
    pub written: u64,
    /// The layout version those bytes follow.
    pub version: AbiVersion,
    /// The revision of the boundary layouts the fields were written against.
    ///
    /// A byte count alone does not say where a field sits: a layout whose
    /// fields were reordered, or whose field types kept their sizes while
    /// changing meaning, produces the same `written` value. The reader compares
    /// this number before it reads, releases or drops any field.
    pub layout_revision: u64,
}

impl AbiHeader {
    /// Builds a header.
    pub const fn new(
        capacity: u64,
        written: u64,
        version: AbiVersion,
        layout_revision: u64,
    ) -> Self {
        Self {
            capacity,
            written,
            version,
            layout_revision,
        }
    }
}

impl Default for AbiHeader {
    fn default() -> Self {
        Self::new(0, 0, AbiVersion::zero(), 0)
    }
}

/// The exact layout of the first descriptor revision.
///
/// This struct is a frozen declaration: it is never edited. [`MIN_DESCRIPTOR_SIZE`]
/// is its size, so the number stays fixed for every target the crate is built
/// for (a 32-bit target occupies fewer bytes than a 64-bit one), while
/// [`PluginDescriptor`] may grow by appending fields.
#[repr(C)]
#[allow(dead_code)]
struct DescriptorV1 {
    header: AbiHeader,
    name: StableString,
    plugin_version: AbiVersion,
    api: StableDyn,
    allocator: AbiAllocator,
    shutdown: Option<ShutdownFn>,
}

/// The number of bytes the first descriptor layout occupies.
///
/// This is the smallest `written` value [`load`] accepts. It is derived from the
/// frozen first-revision layout (`DescriptorV1`) rather than from
/// [`PluginDescriptor`] so that a later revision which appends a field keeps
/// accepting a library built against this one; a constant derived from the
/// growing struct would reject exactly those libraries.
///
/// The buffer a host passes is at least `size_of::<PluginDescriptor>()`, and
/// every byte the reader sees beyond `written` was written by the host's own
/// [`PluginDescriptor::buffer`] call, so reading the whole struct is safe.
pub const MIN_DESCRIPTOR_SIZE: u64 = size_of::<DescriptorV1>() as u64;

// A descriptor written by this revision cannot be smaller than the first one.
const _: () = assert!(MIN_DESCRIPTOR_SIZE <= size_of::<PluginDescriptor>() as u64);

/// A function the host calls while the library is still loaded.
///
/// Whatever the library owns in its own address space is released here, before
/// the code that releases it disappears.
pub type ShutdownFn = unsafe extern "C" fn();

/// What a library reports about itself.
///
/// Every owned field carries the allocator that produced it, so dropping the
/// descriptor on the host side releases the name and the object on the side
/// that allocated them.
///
/// The object's table, its drop entry and the name's free function all live in
/// the library. Dropping the descriptor is enough while the library stays
/// loaded; before unloading, the host must release what the library owns with
/// [`PluginDescriptor::release_owned`] and then call
/// [`PluginDescriptor::shutdown`] (or hold a [`LoadedPlugin`], whose drop does
/// both in that order).
#[repr(C)]
pub struct PluginDescriptor {
    /// The number of bytes written and the layout version they follow.
    pub header: AbiHeader,
    /// The library's name, owned by whichever side allocated it.
    pub name: StableString,
    /// The library's own version, which is informational.
    pub plugin_version: AbiVersion,
    /// The object the library hands to the host.
    pub api: StableDyn,
    /// The allocator the writer uses for the fields it owns.
    pub allocator: AbiAllocator,
    /// Called while the library is loaded, before it goes away, or `None` when
    /// the library recorded no hook.
    ///
    /// This is an `Option` rather than a bare pointer so that a library which
    /// leaves the field untouched writes a value this build defines: a zero word
    /// read as a function pointer would be an invalid value before anything
    /// could refuse it. An `Option` of a function pointer occupies the same
    /// word, with the zero word naming `None`.
    pub shutdown: Option<ShutdownFn>,
}

impl PluginDescriptor {
    /// A buffer whose header records how many bytes a writer may use.
    pub const fn buffer() -> Self {
        Self::buffer_with(AbiAllocator::system())
    }

    /// A buffer whose header records how many bytes a writer may use, and whose
    /// owned fields will be allocated through `allocator`.
    pub const fn buffer_with(allocator: AbiAllocator) -> Self {
        Self {
            header: AbiHeader::new(
                size_of::<PluginDescriptor>() as u64,
                0,
                ABI_VERSION,
                LAYOUT_REVISION,
            ),
            name: StableString::new_with_allocator(allocator),
            plugin_version: AbiVersion::zero(),
            api: StableDyn::empty(),
            allocator,
            shutdown: None,
        }
    }

    /// Runs the shutdown function, once.
    ///
    /// Call [`PluginDescriptor::release_owned`] first: the hook is the library's
    /// chance to tear down state its own drop code may need, so it runs after
    /// the fields the host holds are released.
    pub fn shutdown(&mut self) {
        let shutdown = core::mem::take(&mut self.shutdown);
        if let Some(shutdown) = shutdown {
            // SAFETY: the function was supplied by the library that produced
            // this descriptor, which is still loaded.
            unsafe { shutdown() };
        }
    }

    /// Releases the name and the object, leaving empty fields behind.
    ///
    /// Both were produced by the library: their release calls the function
    /// pointers in their allocators and tables, so it has to run while the
    /// library is still loaded.
    pub fn release_owned(&mut self) {
        let name = core::mem::take(&mut self.name);
        drop(name);
        let api = core::mem::take(&mut self.api);
        drop(api);
    }

    /// The library's name.
    pub fn name_str(&self) -> &str {
        self.name.as_str()
    }

    /// Hands the object to the caller and leaves the field empty.
    ///
    /// The descriptor no longer owns the object, so it must not be dropped
    /// twice: the caller takes over that obligation along with the value.
    pub fn take_api(&mut self) -> StableDyn {
        core::mem::take(&mut self.api)
    }

    /// Writes a descriptor into the buffer the host provided.
    ///
    /// `out` must point at a valid descriptor whose `header.capacity` holds the
    /// number of bytes available; [`PluginDescriptor::buffer_with`] produces
    /// exactly that. Returns [`AbiStatus::InvalidDescriptor`] when the buffer is
    /// smaller than the layout this revision writes, [`AbiStatus::VersionMismatch`]
    /// when the header names another layout revision, [`AbiStatus::NullPointer`]
    /// when `out` is null, and the allocation status when the name cannot be
    /// copied.
    ///
    /// The header records `written` as the number of bytes this revision writes,
    /// which may be more than [`MIN_DESCRIPTOR_SIZE`].
    ///
    /// `shutdown` is called by the host while the library is still loaded. A
    /// library that may be unloaded frees the tables it built, with
    /// [`crate::object::release_cached_tables`]: they come from the process
    /// allocator and nothing can reach them once the module is gone.
    ///
    /// This is the only function that writes a descriptor, and it checks the
    /// buffer's capacity and layout revision before it touches a field. A
    /// library therefore cannot record a field of a buffer that does not cover
    /// it: there is no setter that runs before the check.
    ///
    /// # Safety
    ///
    /// `out` must point at a valid `PluginDescriptor` whose `header.capacity`
    /// covers the layout this revision writes, and the caller must not use the
    /// old contents afterwards.
    pub unsafe fn store(
        out: *mut PluginDescriptor,
        name: &str,
        plugin_version: AbiVersion,
        api: StableDyn,
        shutdown: Option<ShutdownFn>,
    ) -> AbiStatus {
        if out.is_null() {
            return AbiStatus::NullPointer;
        }
        // The capacity is the one field the host is required to have written, and
        // it is the bound on everything after this check. Reading another field
        // first would mean trusting an offset the buffer may not cover.
        // SAFETY: the caller guarantees the pointer names a valid descriptor.
        let capacity = unsafe { (*out).header.capacity };
        let written = size_of::<PluginDescriptor>() as u64;
        if capacity < written {
            return AbiStatus::InvalidDescriptor;
        }
        // The header records which revision its fields follow. A buffer written
        // against another revision may place the fields after it elsewhere, and
        // this function both reads and releases them; the number exists so that
        // the mismatch is refused instead of being read through this build's
        // layout. The header itself is fixed-width, so it can be read before the
        // rest is trusted.
        // SAFETY: `capacity` covers the header, which is the first field.
        let revision = unsafe { (*out).header.layout_revision };
        if revision != LAYOUT_REVISION {
            return AbiStatus::VersionMismatch;
        }
        // SAFETY: the caller guarantees the buffer covers the layout this
        // revision writes, which the check above established.
        let allocator = unsafe { (*out).allocator };
        let owned_name = match StableString::from_str_with_allocator(allocator, name) {
            Ok(owned_name) => owned_name,
            Err(error) => return error.status(),
        };
        // The buffer held valid values, so releasing them before the overwrite
        // keeps a repeated call from leaking the previous object.
        // SAFETY: the caller guarantees a valid descriptor at `out`.
        unsafe { core::ptr::drop_in_place(out) };
        // SAFETY: the buffer holds at least `capacity` bytes, and `capacity` is
        // at least the size of the value written here.
        unsafe {
            out.write(Self {
                header: AbiHeader::new(capacity, written, ABI_VERSION, LAYOUT_REVISION),
                name: owned_name,
                plugin_version,
                api,
                allocator,
                shutdown,
            })
        };
        AbiStatus::Ok
    }
}

impl crate::zeroize::Wipe for AbiHeader {
    /// Overwrites the four words, leaving a header that describes nothing.
    ///
    /// Signed and unsigned fields both have zero as a value, and a header of
    /// zeros names no bytes and no version, which is what a wiped slot should
    /// say.
    fn wipe_in_place(&mut self) {
        crate::zeroize::wipe_value(self);
    }
}

impl crate::zeroize::Wipe for PluginDescriptor {
    /// Wipes the library's name and the object, forgets the hook, and leaves a
    /// header that describes nothing.
    ///
    /// The allocator is left as it is: it names the functions that release the
    /// fields, which is what a later drop of this descriptor calls, and the
    /// fields are empty after the wipe. Running the shutdown hook is not part of
    /// this: the hook belongs to the library, and the host runs it before the
    /// library goes away.
    fn wipe_in_place(&mut self) {
        crate::zeroize::Wipe::wipe_in_place(&mut self.name);
        crate::zeroize::Wipe::wipe_in_place(&mut self.api);
        crate::zeroize::Wipe::wipe_in_place(&mut self.plugin_version);
        self.shutdown = None;
        self.header.wipe_in_place();
    }
}

/// A descriptor together with the obligation to run its shutdown hook.
///
/// Dropping the value releases what the library owns, runs the hook, and then
/// drops the keep-alive value the host supplied, which is the point at which a
/// host may unload the library. Keeping the descriptor without this wrapper
/// leaves those steps to the host.
#[derive(Debug)]
pub struct LoadedPlugin {
    descriptor: PluginDescriptor,
    /// Holds the host's library handle, if the host supplied one.
    ///
    /// Fields are dropped after `Drop::drop` runs, and in declaration order, so
    /// this value goes last: the library is still loaded while its own code
    /// releases the fields and runs the hook.
    keep_alive: Option<Box<dyn core::any::Any>>,
}

impl LoadedPlugin {
    /// Takes ownership of a descriptor that was just loaded.
    pub fn new(descriptor: PluginDescriptor) -> Self {
        Self {
            descriptor,
            keep_alive: None,
        }
    }

    /// Takes ownership of a descriptor and of a value that keeps the library
    /// loaded.
    ///
    /// A host that opened the library itself passes its handle here, so the
    /// order the protocol needs is the order the fields are dropped in: release
    /// what the plugin owns, run its hook, then unload.
    pub fn with_keep_alive<G: core::any::Any>(descriptor: PluginDescriptor, guard: G) -> Self {
        Self {
            descriptor,
            keep_alive: Some(Box::new(guard)),
        }
    }

    /// The descriptor.
    pub const fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    /// The library's name.
    pub fn name(&self) -> &str {
        self.descriptor.name_str()
    }

    /// Reports whether the host supplied a value that keeps the library loaded.
    pub fn has_keep_alive(&self) -> bool {
        self.keep_alive.is_some()
    }

    /// Hands the object to the caller and leaves the descriptor without one.
    pub fn take_api(&mut self) -> StableDyn {
        self.descriptor.take_api()
    }

    /// Releases everything the library owns, runs its hook and hands the
    /// remaining descriptor back.
    ///
    /// The returned value owns no name and no object: both belonged to the
    /// library, which the host may unload after this call.
    pub fn into_descriptor(mut self) -> PluginDescriptor {
        self.descriptor.release_owned();
        self.descriptor.shutdown();
        // The descriptor is moved out; the drop below sees an empty value.
        core::mem::replace(&mut self.descriptor, PluginDescriptor::buffer())
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        // What the descriptor owns is released first: the hook is the library's
        // chance to tear down state its own drop code may still need, so it runs
        // after those fields are gone and before the library is unloaded.
        self.descriptor.release_owned();
        self.descriptor.shutdown();
    }
}

/// The function a library exports for the host to call.
///
/// The return value is the `u32` code of an [`AbiStatus`]. Zero is the only
/// success; every other code is a failure, including one this build has no name
/// for, which the host carries as the code the library wrote rather than
/// replacing with a status of its own.
pub type EntryPointFn = unsafe extern "C" fn(
    host: *const HostServices,
    required: AbiVersion,
    out: *mut PluginDescriptor,
) -> u32;

/// Turns a rejected descriptor into an error.
///
/// The library reported success, so its fields were written; the report says
/// nothing about whether they describe values this build can release. The name
/// is released first and the hook runs afterwards, which is the order
/// [`LoadedPlugin`] uses. The object is released only when
/// [`StableDyn::is_releasable`] accepts it: a handle that names no aligned block,
/// or a table written against another layout revision, is abandoned instead of
/// having its table called and its bytes freed through a size this build cannot
/// establish.
fn reject(descriptor: &mut PluginDescriptor, status: AbiStatus, message: &str) -> AbiError {
    let name = core::mem::take(&mut descriptor.name);
    drop(name);
    let api = core::mem::take(&mut descriptor.api);
    if api.is_releasable() {
        drop(api);
    } else {
        core::mem::forget(api);
    }
    descriptor.shutdown();
    AbiError::with_message(status, message)
}

/// Calls `entry` and validates the descriptor it produced.
///
/// `required` is the protocol version the host requires the library to
/// implement. It is passed to the entry point, which is expected to refuse a
/// library that cannot serve it, and the answer is checked here as well: a
/// library that reports success while implementing an older protocol is
/// released and refused. Both checks are needed because neither side can speak
/// for the other.
///
/// # Safety
///
/// `entry` must be a function pointer obtained from a library built against a
/// compatible version of this crate. Calling into a library that disagrees
/// about the layout of [`HostServices`] or [`PluginDescriptor`] is undefined
/// behaviour, which is why the call is unsafe.
///
/// This function hands over a `&HostServices`, so the value has to be one this
/// build declares in full. A host that wants to offer a shorter description,
/// which is what an older revision declares, cannot use this function: it calls
/// the entry point itself, and the library measures the description through
/// [`HostServices::described_size`] before reading any field past `size`.
pub unsafe fn load(
    entry: EntryPointFn,
    host: &HostServices,
    required: AbiVersion,
) -> AbiResult<PluginDescriptor> {
    let mut descriptor = PluginDescriptor::buffer_with(host.allocator);
    let capacity = descriptor.header.capacity;

    // SAFETY: the caller guarantees the entry point.
    let code = unsafe { entry(host, required, &mut descriptor) };
    match AbiStatus::from_code(code) {
        Some(AbiStatus::Ok) => {}
        // A library that reports failure declares that it left nothing for the
        // host to release, so the buffer is abandoned: its fields are whatever
        // the library wrote before it gave up, and dropping them would call
        // through numbers the library did not stand behind.
        Some(status) => {
            core::mem::forget(descriptor);
            return Err(AbiError::with_message(
                status,
                "the library reported a failure from its entry point",
            ));
        }
        None => {
            core::mem::forget(descriptor);
            // The code is kept as the library wrote it: a failure it named is
            // still a failure, and the number is what says which one.
            return Err(AbiError::with_message(
                AbiStatus::from_raw(code),
                "the library reported a status this host does not define",
            ));
        }
    }

    let written = descriptor.header.written;
    if written < MIN_DESCRIPTOR_SIZE {
        // The library says it wrote fewer bytes than the smallest descriptor
        // layout, so the fields this build would read are not the ones it
        // promised to have written. Neither releasing them nor running the
        // shutdown hook is safe here: both call through numbers the library did
        // not say it had filled in. The buffer is left as it is; the fields the
        // host wrote into it are empty, so nothing of the host's is lost.
        core::mem::forget(descriptor);
        return Err(AbiError::with_message(
            AbiStatus::InvalidDescriptor,
            "the library wrote fewer bytes than the smallest descriptor",
        ));
    }
    if written > capacity {
        // The claim is larger than the buffer the host offered, so the writer's
        // own account of what it did cannot be used at all: a field may sit
        // anywhere inside that claim. The hook is not run and the fields are not
        // released, because both would read offsets from a descriptor whose size
        // is not established.
        core::mem::forget(descriptor);
        return Err(AbiError::with_message(
            AbiStatus::InvalidDescriptor,
            "the library reports more bytes than the buffer it was given",
        ));
    }
    if descriptor.header.layout_revision != LAYOUT_REVISION {
        // A different revision means the fields may be at different offsets, so
        // neither the hook nor the release is safe: reading them would use this
        // build's layout on another build's bytes.
        core::mem::forget(descriptor);
        return Err(AbiError::with_message(
            AbiStatus::VersionMismatch,
            "the descriptor was written against a different boundary layout",
        ));
    }
    if descriptor.header.version.major != ABI_VERSION.major {
        // A different major version may change what an existing field means, so
        // the same caution applies as for the layout revision.
        core::mem::forget(descriptor);
        return Err(AbiError::with_message(
            AbiStatus::VersionMismatch,
            "the descriptor version is not one this host implements",
        ));
    }
    // A newer minor is accepted: the major number matched, so an existing field
    // still means what this build reads it as, and the layout revision matched,
    // so it still sits at the offset this build reads it from. A field added by
    // the newer build is past the bytes read here, and a value whose declaration
    // gained a field is refused by its own signature check rather than by this
    // one. The floor is the other direction, checked next.
    if !descriptor.header.version.serves(required) {
        // The library implements an older protocol than the caller requires. Its
        // own fields are valid, so they are released and its hook runs, but the
        // host cannot use what it published.
        return Err(reject(
            &mut descriptor,
            AbiStatus::VersionMismatch,
            "the library implements an older protocol than this host requires",
        ));
    }
    // The name is read as text by safe code: `LoadedPlugin::name`, the loader in
    // the demo, and everything that logs it. Text that is not UTF-8 has to be
    // refused here, because nothing later in the chain can.
    if let Err(error) = descriptor.name.validate() {
        let status = error.status();
        return Err(reject(
            &mut descriptor,
            status,
            "the library's name is not a valid boundary string",
        ));
    }
    if descriptor.api.is_empty() {
        return Err(reject(
            &mut descriptor,
            AbiStatus::InvalidDescriptor,
            "the library did not provide an object",
        ));
    }
    // The object was checked for emptiness above; its table, address and layout
    // are checked here so a value that names no aligned storage is refused at the
    // adoption point rather than at the first call.
    if let Err(error) = descriptor.api.validate() {
        let status = error.status();
        return Err(reject(
            &mut descriptor,
            status,
            "the library's object does not describe a value",
        ));
    }

    Ok(descriptor)
}

impl fmt::Display for AbiHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} of {} bytes at version {}, layout revision {}",
            self.written, self.capacity, self.version, self.layout_revision
        )
    }
}

impl fmt::Debug for PluginDescriptor {
    // The name and the object are what the other side wrote. Printing the text
    // would put arbitrary content into a log; the length says what a reader
    // normally needs.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PluginDescriptor")
            .field("header", &self.header)
            .field("name_len", &self.name.len())
            .field("plugin_version", &self.plugin_version)
            .field("api", &self.api)
            .finish_non_exhaustive()
    }
}
