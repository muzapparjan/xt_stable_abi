//! A spin lock and a one-time slot, for the parts of the crate that keep
//! process-wide state without the standard library.

use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// A mutual-exclusion lock that spins instead of parking.
///
/// The lock is held only while an entry list of a few elements is read or
/// extended, so spinning costs less than parking a thread.
pub struct SpinLock<T> {
    locked: AtomicBool,
    value: UnsafeCell<T>,
}

// SAFETY: the lock hands out one `&mut T` at a time.
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    /// A lock holding `value`.
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }

    /// Takes the lock, waiting until it is free.
    pub fn lock(&self) -> SpinGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.locked.load(Ordering::Relaxed) {
                spin_loop();
            }
        }
        SpinGuard {
            lock: self,
            value: PhantomData,
        }
    }
}

/// The lock is held while this value is alive.
///
/// The guard is a borrow of the locked value, not of the lock: it stands in for
/// `&'a mut T`, so it is `Sync` only where `T` is. A guard that could be shared
/// between threads would hand out two `&T` at once, and `DerefMut` is reachable
/// through `&mut` alone, so the bound belongs on the shared form.
///
/// A value that may be locked but not shared - a `Cell`, for instance - cannot be
/// reached from two threads at once, and the guard keeps that property:
///
/// ```compile_fail
/// use core::cell::Cell;
/// use std::sync::Arc;
/// use xt_stable_abi::cache::SpinLock;
///
/// let lock = Arc::new(SpinLock::new(Cell::new(1u32)));
/// let guard = lock.lock();
/// // Sharing the guard would reach one `&Cell` from both threads.
/// std::thread::scope(|scope| {
///     scope.spawn(|| drop(&guard));
///     scope.spawn(|| drop(&guard));
/// });
/// ```
pub struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
    // Names the borrow the guard stands for. Without it, the only field is a
    // shared reference to the lock, and the guard would be `Sync` whenever the
    // lock is - including for a `T` that is `Send` but not `Sync`.
    value: PhantomData<&'a mut T>,
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the guard proves the lock is held.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard proves the lock is held and is the only holder.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}

impl<T> core::fmt::Debug for SpinLock<T> {
    // The guarded value is not printed: a lock may be held while a formatter
    // runs.
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SpinLock { .. }")
    }
}

impl<T> core::fmt::Debug for SpinGuard<'_, T> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("SpinGuard { .. }")
    }
}

const UNINITIALISED: u8 = 0;
const INITIALISING: u8 = 1;
const READY: u8 = 2;

/// A value built once, on first use.
pub struct Once<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

// SAFETY: the value is written once, before `READY` is published, and only read
// afterwards.
unsafe impl<T: Send + Sync> Sync for Once<T> {}

/// Puts the state back when the initialiser panics, so a later call can retry.
struct Initialising<'a> {
    state: &'a AtomicU8,
    finished: bool,
}

impl Drop for Initialising<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.state.store(UNINITIALISED, Ordering::Release);
        }
    }
}

impl<T> Once<T> {
    /// An uninitialised slot.
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(UNINITIALISED),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// The value, building it with `init` the first time.
    pub fn get_or_init<F: FnOnce() -> T>(&self, init: F) -> &T {
        loop {
            match self.state.load(Ordering::Acquire) {
                READY => {
                    // SAFETY: `READY` is published after the write.
                    return unsafe { (*self.value.get()).assume_init_ref() };
                }
                UNINITIALISED => {
                    if self
                        .state
                        .compare_exchange(
                            UNINITIALISED,
                            INITIALISING,
                            Ordering::Acquire,
                            Ordering::Relaxed,
                        )
                        .is_ok()
                    {
                        let mut guard = Initialising {
                            state: &self.state,
                            finished: false,
                        };
                        // SAFETY: this call owns the slot while it holds the
                        // `INITIALISING` state.
                        unsafe { (*self.value.get()).write(init()) };
                        guard.finished = true;
                        self.state.store(READY, Ordering::Release);
                        return unsafe { (*self.value.get()).assume_init_ref() };
                    }
                }
                _ => spin_loop(),
            }
        }
    }
}

impl<T> Default for Once<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for Once<T> {
    /// Releases the value when one was built.
    ///
    /// The crate uses this type for caches that live in `static` values, where
    /// nothing runs; a caller that keeps one in a local variable gets the value
    /// released with it rather than left behind in the slot.
    fn drop(&mut self) {
        if *self.state.get_mut() == READY {
            // SAFETY: `READY` is the state the write published, so the slot holds
            // an initialised value, and `&mut self` says no other reader is here.
            unsafe { (*self.value.get()).assume_init_drop() };
            *self.state.get_mut() = UNINITIALISED;
        }
    }
}

impl<T> core::fmt::Debug for Once<T> {
    // The value may not exist yet, so the debug form reports only the state.
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let state = match self.state.load(Ordering::Relaxed) {
            READY => "ready",
            INITIALISING => "initialising",
            _ => "empty",
        };
        formatter
            .debug_struct("Once")
            .field("state", &state)
            .finish()
    }
}
