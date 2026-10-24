//! Shared instruments for the integration tests.
//!
//! [`Tracker`] counts the calls that reach an allocator, and [`DropCounter`]
//! records that a value was destroyed. Together they let a test prove that a
//! value crossing the boundary was released through the allocator it carries
//! and that no destructor ran twice or not at all.

#![allow(dead_code)]

use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use xt_stable_abi::alloc::SYSTEM_ALLOCATOR;
use xt_stable_abi::AbiAllocator;

/// Counters for one tracked allocator.
#[derive(Debug)]
pub struct Tracker {
    /// Successful-sized allocation calls that reached the allocator.
    pub allocations: AtomicUsize,
    /// Release calls that reached the allocator.
    pub deallocations: AtomicUsize,
    /// Resize calls that reached the allocator.
    pub reallocations: AtomicUsize,
    /// Bytes asked for, summed over every allocation call.
    pub requested_bytes: AtomicUsize,
}

impl Tracker {
    /// A tracker that lives for the rest of the process.
    pub fn new() -> &'static Tracker {
        Box::leak(Box::new(Self {
            allocations: AtomicUsize::new(0),
            deallocations: AtomicUsize::new(0),
            reallocations: AtomicUsize::new(0),
            requested_bytes: AtomicUsize::new(0),
        }))
    }

    /// The number of blocks currently outstanding.
    pub fn outstanding(&self) -> isize {
        self.allocations.load(Ordering::SeqCst) as isize
            - self.deallocations.load(Ordering::SeqCst) as isize
    }

    /// Forgets every counter.
    pub fn reset(&self) {
        self.allocations.store(0, Ordering::SeqCst);
        self.deallocations.store(0, Ordering::SeqCst);
        self.reallocations.store(0, Ordering::SeqCst);
        self.requested_bytes.store(0, Ordering::SeqCst);
    }
}

unsafe extern "C" fn tracked_allocate(context: *mut c_void, size: u64, align: u64) -> *mut u8 {
    // SAFETY: every allocator built by `tracked_allocator` carries a tracker
    // that lives for the process.
    let tracker = unsafe { &*(context as *const Tracker) };
    tracker.allocations.fetch_add(1, Ordering::SeqCst);
    tracker
        .requested_bytes
        .fetch_add(size as usize, Ordering::SeqCst);
    SYSTEM_ALLOCATOR.allocate(size, align)
}

unsafe extern "C" fn tracked_deallocate(
    context: *mut c_void,
    pointer: *mut u8,
    size: u64,
    align: u64,
) {
    // SAFETY: the context is the tracker the block was allocated with.
    let tracker = unsafe { &*(context as *const Tracker) };
    tracker.deallocations.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.deallocate(pointer, size, align) };
}

unsafe extern "C" fn tracked_reallocate(
    context: *mut c_void,
    pointer: *mut u8,
    old_size: u64,
    align: u64,
    new_size: u64,
) -> *mut u8 {
    // SAFETY: the context is the tracker the block was allocated with.
    let tracker = unsafe { &*(context as *const Tracker) };
    tracker.reallocations.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.reallocate(pointer, old_size, align, new_size) }
}

/// An allocator that reports every call to `tracker`.
pub fn tracked_allocator(tracker: &'static Tracker) -> AbiAllocator {
    // SAFETY: the three functions accept the tracker as their context, and the
    // tracker outlives every value built with this allocator.
    unsafe {
        AbiAllocator::from_parts(
            tracker as *const Tracker as *mut c_void,
            tracked_allocate,
            tracked_deallocate,
            tracked_reallocate,
        )
    }
}

unsafe extern "C" fn refusing_allocate(
    _context: *mut c_void,
    _size: u64,
    _align: u64,
) -> *mut u8 {
    core::ptr::null_mut()
}

unsafe extern "C" fn ignoring_deallocate(
    _context: *mut c_void,
    _pointer: *mut u8,
    _size: u64,
    _align: u64,
) {
}

unsafe extern "C" fn refusing_reallocate(
    _context: *mut c_void,
    _pointer: *mut u8,
    _old_size: u64,
    _align: u64,
    _new_size: u64,
) -> *mut u8 {
    core::ptr::null_mut()
}

/// An allocator that never hands out memory.
pub fn refusing_allocator() -> AbiAllocator {
    // SAFETY: the functions implement the contracts and use no context.
    unsafe {
        AbiAllocator::from_parts(
            core::ptr::null_mut(),
            refusing_allocate,
            ignoring_deallocate,
            refusing_reallocate,
        )
    }
}

/// An allocator that can be closed part way through a test.
///
/// While it is open it forwards to the process allocator; once closed every
/// request fails. That is how a test reaches the failure branch of an operation
/// on a value that already exists: the value was built while the allocator
/// answered, and the step under test is the one that asks again.
#[derive(Debug)]
pub struct Gate {
    /// Allocation calls that reached it, whether or not it answered.
    pub allocations: AtomicUsize,
    open: AtomicBool,
}

impl Gate {
    /// An open gate.
    ///
    /// A test keeps it in a `static`, which is what gives the address the
    /// allocator built from it keeps for the rest of the process; a value on the
    /// stack would not outlive the values allocated through it.
    pub const fn new() -> Self {
        Self {
            allocations: AtomicUsize::new(0),
            open: AtomicBool::new(true),
        }
    }

    /// Stops handing out memory.
    pub fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
    }
}

unsafe extern "C" fn gated_allocate(context: *mut c_void, size: u64, align: u64) -> *mut u8 {
    // SAFETY: every allocator built by `gated_allocator` carries a gate that
    // lives for the process.
    let gate = unsafe { &*(context as *const Gate) };
    gate.allocations.fetch_add(1, Ordering::SeqCst);
    if gate.open.load(Ordering::SeqCst) {
        SYSTEM_ALLOCATOR.allocate(size, align)
    } else {
        core::ptr::null_mut()
    }
}

unsafe extern "C" fn gated_deallocate(
    _context: *mut c_void,
    pointer: *mut u8,
    size: u64,
    align: u64,
) {
    // A block that was handed out while the gate was open is released through
    // the same functions wherever the gate stands now.
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.deallocate(pointer, size, align) };
}

unsafe extern "C" fn gated_reallocate(
    context: *mut c_void,
    pointer: *mut u8,
    old_size: u64,
    align: u64,
    new_size: u64,
) -> *mut u8 {
    // SAFETY: every allocator built by `gated_allocator` carries a gate that
    // lives for the process.
    let gate = unsafe { &*(context as *const Gate) };
    if !gate.open.load(Ordering::SeqCst) {
        return core::ptr::null_mut();
    }
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.reallocate(pointer, old_size, align, new_size) }
}

/// An allocator that answers every request while `gate` is open.
pub fn gated_allocator(gate: &'static Gate) -> AbiAllocator {
    // SAFETY: the three functions accept the gate as their context, and the gate
    // outlives every value built with this allocator.
    unsafe {
        AbiAllocator::from_parts(
            gate as *const Gate as *mut c_void,
            gated_allocate,
            gated_deallocate,
            gated_reallocate,
        )
    }
}

/// A value that records its own destruction.
#[derive(Debug)]
pub struct DropCounter {
    /// The counter to bump when this value is dropped.
    pub counter: &'static AtomicUsize,
    /// A payload, so the value is not zero-sized.
    pub payload: u64,
}

impl DropCounter {
    /// Builds a value that reports to `counter`.
    pub fn new(counter: &'static AtomicUsize, payload: u64) -> Self {
        Self { counter, payload }
    }

    /// The number of values that have been dropped so far.
    pub fn dropped(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }
}

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.counter.fetch_add(1, Ordering::SeqCst);
    }
}

xt_stable_abi::unsafe_stable_element!(DropCounter);
