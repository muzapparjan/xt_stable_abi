//! Giving a cached table back.
//!
//! One table serves every value of an implementation type, so it outlives all of
//! them. That is right while the module that built it stays loaded, and wrong
//! once that module is unloaded: the allocator belongs to the process, so the
//! memory stays allocated while nothing can reach it any more.

// The watch below is installed as the binary's global allocator, and Miri does
// not run a binary that installs one: every deallocation in the process fails
// there, including ones the test harness makes. The tests that do not need it
// still run under Miri.
#[cfg(not(miri))]
use std::alloc::{GlobalAlloc, Layout, System};
#[cfg(not(miri))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use xt_stable_abi::object::release_cached_tables;
use xt_stable_abi::stable_object;

/// The address this build of the allocator watches for a deallocation.
#[cfg(not(miri))]
static WATCHED: AtomicUsize = AtomicUsize::new(0);

/// Whether the watched address was deallocated.
#[cfg(not(miri))]
static WATCHED_WAS_FREED: AtomicBool = AtomicBool::new(false);

/// The blocks this build of the allocator has handed out and not taken back.
#[cfg(not(miri))]
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// Reports whether one particular address came back to the allocator.
///
/// A byte count cannot answer this: a release also frees the list that held the
/// table, and its block can be larger than the table's.
#[cfg(not(miri))]
struct Watching;

// SAFETY: every allocation and deallocation is passed to the system allocator
// unchanged; the wrapper only notes one address on the way through.
#[cfg(not(miri))]
unsafe impl GlobalAlloc for Watching {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        OUTSTANDING.fetch_add(1, Ordering::SeqCst);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if pointer as usize == WATCHED.load(Ordering::SeqCst) {
            WATCHED_WAS_FREED.store(true, Ordering::SeqCst);
        }
        OUTSTANDING.fetch_sub(1, Ordering::SeqCst);
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[cfg(not(miri))]
#[global_allocator]
static ALLOCATOR: Watching = Watching;

stable_object! {
    /// A handle to a press.
    pub struct PressObject;
    /// The table behind [`PressObject`].
    pub struct PressVTable;
    /// What a press can be asked.
    pub trait Press {
        /// The force it applies, in newtons.
        fn force(&self) -> u64;
    }
}

struct Iron;

impl Press for Iron {
    fn force(&self) -> u64 {
        1_200
    }
}

/// One test, because a second test in this binary would share the caches with
/// this one: the default test runner runs them at the same time, and a release
/// is defined only while nothing else is using a table.
#[test]
fn a_release_frees_the_table_and_the_cache_builds_another() {
    // Nothing may be cached from an earlier run of this test.
    unsafe { release_cached_tables() };
    #[cfg(not(miri))]
    frees_the_table_itself();
    #[cfg(not(miri))]
    frees_the_published_copies();
    rebuilds_a_table_after_a_release();
}

/// The table the cache built comes back to the allocator.
#[cfg(not(miri))]
fn frees_the_table_itself() {
    WATCHED_WAS_FREED.store(false, Ordering::SeqCst);
    // Building a value builds the table for its type. The value is dropped
    // before the release, so only the table is left in the cache.
    let object = PressObject::abi_from_value(Iron);
    let value = object.abi_into_stable();
    let table = value.vtable_ptr() as usize;
    drop(value);

    WATCHED.store(table, Ordering::SeqCst);
    let released = unsafe { release_cached_tables() };

    assert_eq!(
        released, 1,
        "the cache holds one table for the one implementation type"
    );
    assert!(
        WATCHED_WAS_FREED.load(Ordering::SeqCst),
        "the table at {table:#x} is given back, not only forgotten"
    );
}

/// The copy of the entries that a registration publishes is given back too.
///
/// A reader finds a table in that copy without taking the cache's lock, so the
/// copy outlives every value of its type and must be freed by the release.
#[cfg(not(miri))]
fn frees_the_published_copies() {
    let before = OUTSTANDING.load(Ordering::SeqCst);
    let object = PressObject::abi_from_value(Iron);
    drop(object);
    assert_eq!(unsafe { release_cached_tables() }, 1);
    let after = OUTSTANDING.load(Ordering::SeqCst);
    assert_eq!(
        after, before,
        "{} block(s) stayed allocated across a build and a release",
        after.saturating_sub(before)
    );
}

/// A second release frees nothing, and a value built afterwards builds a new
/// table rather than reaching a freed one.
fn rebuilds_a_table_after_a_release() {
    let object = PressObject::abi_from_value(Iron);
    // SAFETY: the value was built by this crate and is alive.
    assert_eq!(unsafe { object.force() }, 1_200);
    drop(object);

    assert_eq!(unsafe { release_cached_tables() }, 1);
    assert_eq!(
        unsafe { release_cached_tables() },
        0,
        "the tables a release freed are not freed twice"
    );

    let object = PressObject::abi_from_value(Iron);
    // SAFETY: the value was built by this crate and is alive.
    assert_eq!(unsafe { object.force() }, 1_200);
    drop(object);
    assert_eq!(unsafe { release_cached_tables() }, 1);
}
