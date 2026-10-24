//! A library that declares the boundary types itself.
//!
//! This crate does not depend on `xt_stable_abi`. Every type below is written
//! out by hand from the documented layout, so a successful exchange through it
//! shows that two independent declarations agree, not that one crate was linked
//! twice.

#![deny(unsafe_op_in_unsafe_fn)]
// MSVC reports the import library it creates for a cdylib; nothing is wrong
// with the artifact.
#![allow(linker_messages)]

use core::ffi::c_void;
use core::mem::{align_of, size_of};
use core::ptr;

/// A major/minor version pair.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Version {
    major: u32,
    minor: u32,
}

/// The allocation functions the host offers.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Allocator {
    context: *mut c_void,
    allocate: unsafe extern "C" fn(*mut c_void, u64, u64) -> *mut u8,
    deallocate: unsafe extern "C" fn(*mut c_void, *mut u8, u64, u64),
    reallocate: unsafe extern "C" fn(*mut c_void, *mut u8, u64, u64, u64) -> *mut u8,
}

impl Allocator {
    unsafe fn allocate(&self, size: u64, align: u64) -> *mut u8 {
        // SAFETY: the host supplied the function with this context.
        unsafe { (self.allocate)(self.context, size, align) }
    }
}

/// The header of the descriptor.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Header {
    capacity: u64,
    written: u64,
    version: Version,
    /// The boundary layout the fields below were written against. A reader
    /// compares this before it reads, releases or drops any of them.
    layout_revision: u64,
}

/// A byte sequence with an explicit length and capacity.
#[repr(C)]
pub struct Bytes {
    pointer: *mut u8,
    length: u64,
    capacity: u64,
    allocator: Allocator,
}

/// The name field of the descriptor.
pub type Name = Bytes;

/// The header every table starts with.
#[repr(C)]
pub struct TablePrefix {
    type_id: u64,
    signature: u64,
    layout_revision: u64,
    size: u64,
    align: u64,
    slots: u64,
    drop: Option<unsafe extern "C" fn(*mut u8)>,
}

/// One entry of the table.
#[repr(C)]
pub struct Slot {
    signature: u64,
    call: *const (),
}

/// An owned object.
#[repr(C)]
pub struct Object {
    data: *mut u8,
    vtable: *const TablePrefix,
    allocator: Allocator,
}

/// What the library reports about itself.
#[repr(C)]
pub struct Descriptor {
    header: Header,
    name: Name,
    plugin_version: Version,
    api: Object,
    allocator: Allocator,
    shutdown: unsafe extern "C" fn(),
}

/// A borrowed byte sequence, spelled out the way the host declares it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Text {
    pointer: *const u8,
    length: u64,
}

/// The log sink the host offers.
pub type LogFn = unsafe extern "C" fn(*mut c_void, u32, Text);

/// The services the host offers.
#[repr(C)]
pub struct Host {
    /// The number of bytes this value occupies, written by the host.
    size: u64,
    version: Version,
    allocator: Allocator,
    /// The sink, or `None` when the host offers none.
    ///
    /// The host writes a word, and the host implements it as an `Option` of a
    /// function pointer, whose `None` is the zero word. Reading the word as a
    /// bare pointer would make the zero value invalid before it could be
    /// refused.
    log: Option<LogFn>,
    context: *mut c_void,
}

/// The status codes this library reports, spelled out from the documented set.
const OK: u32 = 0;
const NULL_POINTER: u32 = 1;
const OUT_OF_MEMORY: u32 = 5;
const VERSION_MISMATCH: u32 = 9;
const INVALID_DESCRIPTOR: u32 = 10;

/// The boundary layout revision this library was written against.
const LAYOUT_REVISION: u64 = 5;

/// The identifier of a piece of text, transcribed from the documented rule.
///
/// This library has no access to the crate it talks to, so it carries its own
/// statement of the rule: the text enters character for character, and the layout
/// revision is mixed in.
const fn declaration_id(text: &str) -> u64 {
    let bytes = text.as_bytes();
    let mut hash: u64 = LAYOUT_REVISION ^ 0xcbf2_9ce4_8422_2325;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    if hash == 0 { 1 } else { hash }
}

// SAFETY: the slot releases a `u64`, which the table says it holds.
unsafe extern "C" fn drop_u64(data: *mut u8) {
    // SAFETY: the table describes a `u64`.
    unsafe { ptr::drop_in_place(data.cast::<u64>()) }
}

unsafe extern "C" fn read_value(data: *mut u8) -> u64 {
    // SAFETY: the table describes a `u64`.
    unsafe { *data.cast::<u64>() }
}

/// Folds one word into `hash`, byte by byte, so the order of the words counts.
const fn mix(hash: u64, value: u64) -> u64 {
    let mut hash = hash;
    let mut index = 0;
    while index < 8 {
        hash ^= (value >> (index * 8)) & 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// The number that names a type with this kind and these children.
///
/// A build that does not use the crate still has to name the types it mentions,
/// and the names are numbers: this is the rule, transcribed for the one type
/// this library publishes (`u64`), named by kind 4.
const fn identity(kind: u64, children: &[u64]) -> u64 {
    let mut hash: u64 = LAYOUT_REVISION ^ 0xcbf2_9ce4_8422_2325;
    hash = mix(hash, kind);
    let mut index = 0;
    while index < children.len() {
        hash = mix(hash, children[index]);
        index += 1;
    }
    hash
}

/// The number that names `u64`.
const U64_IDENTITY: u64 = identity(4, &[]);

/// The receiver forms, as the host numbers them.
const RECEIVER_SHARED: u64 = 1;

/// The identifier of one slot: the method, the receiver form and the types the
/// method mentions.
const fn slot_id(name: &str, receiver: u64, arguments: &[u64], returned: u64) -> u64 {
    let mut hash = declaration_id(name) ^ receiver.rotate_left(32);
    let mut index = 0;
    while index < arguments.len() {
        hash = mix(hash, arguments[index]);
        index += 1;
    }
    hash = mix(hash, returned);
    if hash == 0 { 1 } else { hash }
}

/// The single method this library publishes.
const VALUE: u64 = slot_id("value", RECEIVER_SHARED, &[], U64_IDENTITY);

/// The whole method list, for the header.
///
/// The number is folded from the per-method identifiers the same way the host
/// does it, so a build that adds a method to the list answers with a different
/// number.
const METHODS: u64 = combined_signature(&[VALUE]);

/// Folds one identifier per method into the identifier of the whole list.
///
/// The fold is commutative: the order the methods are listed in is not part of a
/// declaration, because a call finds its entry by comparing numbers.
const fn combined_signature(signatures: &[u64]) -> u64 {
    let mut sum: u64 = 0;
    let mut twist: u64 = 0;
    let mut index = 0;
    while index < signatures.len() {
        let mixed = mix_identifier(signatures[index]);
        sum = sum.wrapping_add(mixed);
        twist ^= mixed.rotate_left(17);
        index += 1;
    }
    (LAYOUT_REVISION ^ 0xcbf2_9ce4_8422_2325)
        ^ (signatures.len() as u64)
        ^ sum
        ^ twist
}

/// Spreads the bits of one identifier so that adding several keeps them apart.
const fn mix_identifier(identifier: u64) -> u64 {
    let mut hash = identifier ^ 0xcbf2_9ce4_8422_2325;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash ^ (hash >> 29)
}

/// The table this library publishes.
#[repr(C)]
struct ServiceTable {
    prefix: TablePrefix,
    value: Slot,
    end: Slot,
}

static TABLE: ServiceTable = ServiceTable {
    prefix: TablePrefix {
        type_id: 99,
        signature: METHODS,
        layout_revision: LAYOUT_REVISION,
        size: 8,
        align: 8,
        slots: 1,
        drop: Some(drop_u64),
    },
    value: Slot {
        signature: VALUE,
        call: read_value as *const (),
    },
    end: Slot {
        signature: 0,
        call: ptr::null(),
    },
};

// SAFETY: the table is immutable for the life of the process, and every entry
// calls a function that only reads the value it is given.
unsafe impl Sync for ServiceTable {}

unsafe extern "C" fn shutdown() {}

/// Writes `text` into a block from the host allocator.
unsafe fn make_name(allocator: Allocator, text: &[u8]) -> Option<Name> {
    if text.is_empty() {
        // An empty value names an aligned, non-null address: a slice or string
        // with a null pointer is not a value the host accepts, even when its
        // length is zero.
        return Some(Name {
            pointer: core::ptr::NonNull::<u8>::dangling().as_ptr(),
            length: 0,
            capacity: 0,
            allocator,
        });
    }
    // SAFETY: the allocator is the host's and the size is non-zero.
    let block = unsafe { allocator.allocate(text.len() as u64, 1) };
    if block.is_null() {
        return None;
    }
    // SAFETY: the block holds `text.len()` bytes.
    unsafe { ptr::copy_nonoverlapping(text.as_ptr(), block, text.len()) };
    Some(Name {
        pointer: block,
        length: text.len() as u64,
        capacity: text.len() as u64,
        allocator,
    })
}

/// The entry point the host looks up by name.
///
/// # Safety
///
/// `host` must be null or point at a valid [`Host`], and `out` must be null or
/// point at a descriptor buffer the host provided.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xt_legacy_plugin_entry(
    host: *const Host,
    required: Version,
    out: *mut Descriptor,
) -> u32 {
    if host.is_null() || out.is_null() {
        return NULL_POINTER;
    }
    // SAFETY: the host guarantees a readable value for the call.
    let host = unsafe { &*host };
    if host.size < size_of::<Host>() as u64 {
        return INVALID_DESCRIPTOR;
    }
    if host.version.major != required.major || host.version.minor < required.minor {
        return VERSION_MISMATCH;
    }
    // SAFETY: the same guarantee covers the buffer.
    let capacity = unsafe { (*out).header.capacity };
    if capacity < size_of::<Descriptor>() as u64 {
        // Writing the descriptor would run past the buffer the host provided.
        return INVALID_DESCRIPTOR;
    }

    // SAFETY: the host allocator is valid for the call.
    let name = match unsafe { make_name(host.allocator, b"legacy_plugin") } {
        Some(name) => name,
        None => return OUT_OF_MEMORY,
    };
    // SAFETY: the host allocator is valid for the call.
    let data = unsafe { host.allocator.allocate(size_of::<u64>() as u64, align_of::<u64>() as u64) };
    if data.is_null() {
        // The name was allocated above, and nothing will read it: releasing it
        // here is the only place that can.
        if !name.pointer.is_null() && name.capacity > 0 {
            // SAFETY: the block came from this allocator with this layout, and
            // no descriptor naming it is ever written.
            unsafe {
                (host.allocator.deallocate)(
                    host.allocator.context,
                    name.pointer,
                    name.capacity,
                    1,
                );
            }
        }
        return OUT_OF_MEMORY;
    }
    // SAFETY: the block holds one `u64`.
    unsafe { data.cast::<u64>().write(7) };

    let descriptor = Descriptor {
        header: Header {
            capacity,
            written: size_of::<Descriptor>() as u64,
            version: Version { major: 1, minor: 0 },
            layout_revision: LAYOUT_REVISION,
        },
        name,
        plugin_version: Version { major: 0, minor: 9 },
        api: Object {
            data,
            vtable: &TABLE.prefix,
            allocator: host.allocator,
        },
        allocator: host.allocator,
        shutdown,
    };
    // SAFETY: the host guarantees the buffer is at least `capacity` bytes, and
    // `capacity` was checked against the layout this library writes.
    unsafe { out.write(descriptor) };
    OK
}
