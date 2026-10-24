//! Sequences crossing the boundary, including their element destructors.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};

use support::{refusing_allocator, tracked_allocator, DropCounter, Tracker};
use xt_stable_abi::{AbiStatus, StableSlice, StableString, StableVec};

#[test]
fn growth_preserves_order() {
    let mut vector = StableVec::new();
    for value in 0..5_000u64 {
        vector.push(value.wrapping_mul(3));
    }
    assert_eq!(vector.len(), 5_000);
    for (index, value) in vector.iter().enumerate() {
        assert_eq!(*value, index as u64 * 3);
    }
    assert!(vector.capacity() >= vector.len());
}

#[test]
fn the_allocator_that_grew_the_vector_also_releases_it() {
    let tracker = Tracker::new();
    let mut vector = StableVec::new_with_allocator(tracked_allocator(tracker));
    for value in 0..1_000u32 {
        vector.push(value);
    }
    assert!(tracker.allocations.load(Ordering::SeqCst) >= 1);
    assert!(tracker.reallocations.load(Ordering::SeqCst) >= 1);
    assert_eq!(tracker.outstanding(), 1);

    drop(vector);
    assert_eq!(tracker.outstanding(), 0);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 1);
}

#[test]
fn a_refused_allocation_leaves_the_vector_usable() {
    let mut vector = StableVec::new_with_allocator(refusing_allocator());
    let error = vector.try_push(1u64).unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
    assert!(vector.is_empty());
    assert_eq!(vector.capacity(), 0);

    // A vector with spare capacity can still be filled when the allocator
    // starts refusing, because no further block is needed.
    let mut reserved = StableVec::with_capacity(4).unwrap();
    for value in 0..4u32 {
        reserved.push(value);
    }
    assert_eq!(reserved.as_slice(), &[0, 1, 2, 3]);
}

#[test]
fn zero_sized_elements_never_allocate() {
    let mut vector: StableVec<()> = StableVec::new_with_allocator(refusing_allocator());
    for _ in 0..1_000 {
        vector.push(());
    }
    assert_eq!(vector.len(), 1_000);
    assert_eq!(vector.capacity(), isize::MAX as u64);
    vector.insert(500, ()).unwrap();
    assert_eq!(vector.len(), 1_001);
    vector.truncate(10);
    assert_eq!(vector.len(), 10);
    vector.clear();
    assert!(vector.is_empty());
}

#[test]
fn removal_moves_values_without_dropping_them() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    {
        let mut vector = StableVec::new();
        for index in 0..4 {
            vector.push(DropCounter::new(&DROPS, index));
        }
        assert_eq!(DropCounter::dropped(&DROPS), 0);

        let removed = vector.remove(1).expect("index 1 is inside the vector");
        assert_eq!(removed.payload, 1);
        assert_eq!(DropCounter::dropped(&DROPS), 0);
        drop(removed);
        assert_eq!(DropCounter::dropped(&DROPS), 1);

        let swapped = vector.swap_remove(0).expect("index 0 is inside the vector");
        assert_eq!(swapped.payload, 0);
        drop(swapped);
        assert_eq!(DropCounter::dropped(&DROPS), 2);

        assert_eq!(vector.len(), 2);
        assert_eq!(vector.as_slice()[0].payload, 3);
        assert_eq!(vector.as_slice()[1].payload, 2);
    }
    // The two remaining values are released when the vector dies.
    assert_eq!(DropCounter::dropped(&DROPS), 4);
}

#[test]
fn a_panicking_predicate_releases_each_element_once() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let mut vector = StableVec::new();
    for index in 0..4 {
        vector.push(DropCounter::new(&DROPS, index));
    }

    let mut visited = 0;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vector.retain(|_| {
            visited += 1;
            if visited == 3 {
                panic!("the predicate gave up");
            }
            // The first element is rejected, so it is released before the
            // panic; the rest of the walk never happens.
            visited != 1
        });
    }));
    assert!(outcome.is_err());
    assert_eq!(DropCounter::dropped(&DROPS), 1);

    // The elements the walk had not reached are still the vector's, and the one
    // that was rejected is not released a second time.
    assert_eq!(vector.len(), 3);
    assert_eq!(vector.as_slice()[0].payload, 1);
    assert_eq!(vector.as_slice()[2].payload, 3);
    drop(vector);
    assert_eq!(DropCounter::dropped(&DROPS), 4);
}

#[test]
fn truncation_and_clearing_release_the_tail_only() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let mut vector = StableVec::new();
    for index in 0..6 {
        vector.push(DropCounter::new(&DROPS, index));
    }
    vector.truncate(4);
    assert_eq!(DropCounter::dropped(&DROPS), 2);
    vector.truncate(9);
    assert_eq!(DropCounter::dropped(&DROPS), 2);
    vector.clear();
    assert_eq!(DropCounter::dropped(&DROPS), 6);
    assert!(vector.is_empty());
}

#[test]
fn insertion_shifts_without_duplicating_drop_counters() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let mut vector = StableVec::new();
    vector.push(DropCounter::new(&DROPS, 0));
    vector.push(DropCounter::new(&DROPS, 2));
    vector.insert(1, DropCounter::new(&DROPS, 1)).unwrap();
    let payloads: Vec<u64> = vector.iter().map(|value| value.payload).collect();
    assert_eq!(payloads, vec![0, 1, 2]);
    assert_eq!(DropCounter::dropped(&DROPS), 0);
    drop(vector);
    assert_eq!(DropCounter::dropped(&DROPS), 3);
}

#[test]
fn nested_owned_values_are_released_with_the_outer_vector() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    {
        let mut vector = StableVec::new();
        for index in 0..20u32 {
            let text =
                StableString::from_str_with_allocator(allocator, &index.to_string()).unwrap();
            vector.push(text);
        }
        assert_eq!(vector.len(), 20);
        assert_eq!(tracker.outstanding(), 20);
        assert_eq!(vector.as_slice()[7].as_str(), "7");
    }
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_clone_keeps_the_allocator_and_the_contents() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let mut original = StableVec::new_with_allocator(allocator);
    original.push(10u32);
    original.push(20u32);

    let clone = original.try_clone().unwrap();
    assert_eq!(clone.as_slice(), &[10, 20]);
    assert_eq!(clone.allocator().context(), allocator.context());
    drop(original);
    assert_eq!(clone.as_slice(), &[10, 20]);
    drop(clone);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_borrowed_slice_can_be_copied_back_into_an_owned_vector() {
    let vector = StableVec::from_slice(&[1u16, 2, 3, 4]).unwrap();
    let borrowed = vector.as_stable_slice();
    assert_eq!(borrowed.len(), 4);
    let copy = borrowed.to_vec().unwrap();
    assert_eq!(copy.as_slice(), vector.as_slice());
    assert_eq!(borrowed.get(2), Some(&3));
    assert_eq!(borrowed.get(4), None);
}

#[test]
fn empty_vector_has_no_storage() {
    let vector: StableVec<u32> = StableVec::new();
    assert!(vector.is_empty());
    assert_eq!(vector.len(), 0);
    assert_eq!(vector.capacity(), 0);
    assert_eq!(vector.as_slice(), &[] as &[u32]);
}

#[test]
fn pop_returns_values_in_reverse() {
    let mut vector = StableVec::new();
    vector.push(1u8);
    vector.push(2u8);
    assert_eq!(vector.pop(), Some(2));
    assert_eq!(vector.pop(), Some(1));
    assert_eq!(vector.pop(), None);
    assert!(vector.is_empty());
}

#[test]
fn insert_shifts_later_elements() {
    let mut vector = StableVec::from_slice(&[1u32, 2, 4]).unwrap();
    vector.insert(2, 3).unwrap();
    assert_eq!(vector.as_slice(), &[1, 2, 3, 4]);
    vector.insert(0, 0).unwrap();
    assert_eq!(vector.as_slice(), &[0, 1, 2, 3, 4]);
    vector.insert(5, 5).unwrap();
    assert_eq!(vector.as_slice(), &[0, 1, 2, 3, 4, 5]);
    assert!(vector.insert(7, 9).is_err());
}

#[test]
fn remove_shifts_later_elements_back() {
    let mut vector = StableVec::from_slice(&[0u32, 1, 2, 3]).unwrap();
    assert_eq!(vector.remove(1), Some(1));
    assert_eq!(vector.as_slice(), &[0, 2, 3]);
    assert_eq!(vector.remove(2), Some(3));
    assert_eq!(vector.as_slice(), &[0, 2]);
    assert_eq!(vector.remove(2), None);
}

#[test]
fn swap_remove_moves_the_tail() {
    let mut vector = StableVec::from_slice(&[0u32, 1, 2, 3]).unwrap();
    assert_eq!(vector.swap_remove(1), Some(1));
    assert_eq!(vector.as_slice(), &[0, 3, 2]);
    assert_eq!(vector.swap_remove(2), Some(2));
    assert_eq!(vector.as_slice(), &[0, 3]);
}

#[test]
fn truncate_and_clear_drop_the_tail() {
    let mut vector = StableVec::from_slice(&[0u32, 1, 2, 3]).unwrap();
    vector.truncate(2);
    assert_eq!(vector.as_slice(), &[0, 1]);
    vector.clear();
    assert!(vector.is_empty());
    // Storage survives a clear.
    assert!(vector.capacity() >= 4);
}

#[test]
fn zero_sized_elements_hold_no_storage() {
    let mut vector: StableVec<()> = StableVec::new();
    for _ in 0..10 {
        vector.push(());
    }
    assert_eq!(vector.len(), 10);
    assert_eq!(vector.capacity(), isize::MAX as u64);
    assert_eq!(vector.as_slice().len(), 10);
    assert_eq!(vector.pop(), Some(()));
    assert_eq!(vector.remove(0), Some(()));
    vector.clear();
    assert!(vector.is_empty());
}

#[test]
fn slice_handle_exposes_the_same_elements() {
    let vector = StableVec::from_slice(&[7u16, 8, 9]).unwrap();
    let slice = vector.as_stable_slice();
    assert_eq!(slice.len(), 3);
    assert_eq!(slice.get(1), Some(&8));
    assert_eq!(slice.get(3), None);
    assert_eq!(slice.as_slice(), &[7, 8, 9]);
    assert_eq!(unsafe { slice.as_slice_unbounded() }, &[7, 8, 9]);
}

#[test]
fn empty_slice_is_aligned_and_empty() {
    let slice: xt_stable_abi::StableSlice<u64> = xt_stable_abi::StableSlice::empty();
    assert!(slice.is_empty());
    assert_eq!(slice.as_ptr() as usize % core::mem::align_of::<u64>(), 0);
    assert_eq!(slice.iter().count(), 0);
}

/// An element whose destructor gives up once.
///
/// The counter is the caller's rather than a global one, so two tests that run
/// at the same time count their own releases instead of each other's.
struct Bomb {
    /// Whether this destructor panics. Cleared before it does, so a second
    /// release would be silent and only the count would show it.
    armed: bool,
    /// How many times a value of this kind has been released.
    drops: &'static AtomicUsize,
}

impl Drop for Bomb {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
        if self.armed {
            self.armed = false;
            panic!("the element gave up while being released");
        }
    }
}

xt_stable_abi::unsafe_stable_element!(Bomb);

#[test]
fn an_element_that_panics_while_being_removed_is_released_once() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let mut vector = StableVec::new();
    vector.push(Bomb {
        armed: false,
        drops: &DROPS,
    });
    vector.push(Bomb {
        armed: true,
        drops: &DROPS,
    });
    vector.push(Bomb {
        armed: false,
        drops: &DROPS,
    });

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vector.retain(|element| !element.armed);
    }));
    assert!(outcome.is_err());

    // The element being released is no longer part of the vector, and the one
    // behind it moved down into its place.
    assert_eq!(vector.len(), 2);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);

    drop(vector);
    assert_eq!(DROPS.load(Ordering::SeqCst), 3);
}

#[test]
fn an_element_that_panics_while_being_removed_is_released_once_by_retain_mut() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let mut vector = StableVec::new();
    vector.push(Bomb {
        armed: false,
        drops: &DROPS,
    });
    vector.push(Bomb {
        armed: true,
        drops: &DROPS,
    });
    vector.push(Bomb {
        armed: false,
        drops: &DROPS,
    });

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        vector.retain_mut(|element| !element.armed);
    }));
    assert!(outcome.is_err());
    assert_eq!(vector.len(), 2);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);

    drop(vector);
    assert_eq!(DROPS.load(Ordering::SeqCst), 3);
}

#[test]
fn the_editing_helpers_keep_the_order_they_promise() {
    let mut vector = StableVec::from_slice(&[1u32, 2, 3, 4]).unwrap();

    vector.reverse();
    assert_eq!(vector.as_slice(), &[4, 3, 2, 1]);

    vector.swap(0, 3).unwrap();
    assert_eq!(vector.as_slice(), &[1, 3, 2, 4]);
    assert_eq!(
        vector.swap(0, 4).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );

    *vector.first_mut().unwrap() = 10;
    *vector.last_mut().unwrap() = 40;
    assert_eq!(vector.as_slice(), &[10, 3, 2, 40]);

    vector.resize(6, 7);
    assert_eq!(vector.as_slice(), &[10, 3, 2, 40, 7, 7]);
    vector.resize(2, 0);
    assert_eq!(vector.as_slice(), &[10, 3]);

    let mut tail = vector.split_off(1);
    assert_eq!(vector.as_slice(), &[10]);
    assert_eq!(tail.as_slice(), &[3]);
    assert_eq!(
        vector.try_split_off(5).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );

    vector.append(&mut tail);
    assert!(tail.is_empty());
    assert_eq!(vector.as_slice(), &[10, 3]);
}

#[test]
fn an_unstable_sort_rearranges_the_block_it_already_owns() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let mut values = StableVec::with_capacity_and_allocator(allocator, 64).unwrap();
    // A shape a merge would keep and a heap must reorder: many equal keys.
    for index in 0..64u64 {
        values.push((index % 4) * 1000 + index);
    }

    let before = tracker.allocations.load(Ordering::SeqCst);
    values.sort_unstable_by(|left, right| left.cmp(right));
    // The sort takes no block of its own, so nothing was asked of the allocator
    // and there is no failure for a fallible form to report.
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), before);

    let sorted = values.as_slice();
    for pair in sorted.windows(2) {
        assert!(pair[0] <= pair[1], "the elements are in order");
    }
    assert_eq!(sorted.len(), 64);
    assert_eq!(
        sorted.iter().filter(|value| **value / 1000 % 4 == 0).count(),
        16
    );
}

#[test]
fn an_unstable_sort_on_a_reversed_range_keeps_every_element_once() {
    let mut values = StableVec::new();
    for value in (0..1000u64).rev() {
        values.push(value);
    }
    values.sort_unstable();
    for (index, value) in values.as_slice().iter().enumerate() {
        assert_eq!(*value, index as u64);
    }
}

#[test]
fn a_zero_sized_element_has_no_spare_slots() {
    // The capacity of such a vector is the address-space bound, not a number of
    // block positions, so a slice that long would be a walk with no end.
    let mut values: StableVec<()> = StableVec::with_capacity(8).unwrap();
    values.push(());
    assert!(values.spare_capacity_mut().is_empty());
    assert!(values.capacity() > 1);

    // A sized element still gets the slots it has room for.
    let mut numbers: StableVec<u64> = StableVec::with_capacity(4).unwrap();
    numbers.push(1);
    assert_eq!(numbers.spare_capacity_mut().len(), (numbers.capacity() - 1) as usize);
}

#[test]
fn a_destructor_that_gives_up_still_releases_the_block() {
    static RELEASES: AtomicUsize = AtomicUsize::new(0);

    struct Explodes {
        marker: u64,
    }

    impl Drop for Explodes {
        fn drop(&mut self) {
            RELEASES.fetch_add(1, Ordering::SeqCst);
            let _ = self.marker;
            panic!("this destructor gives up");
        }
    }

    // SAFETY: the type owns nothing; its destructor only reports and gives up.
    unsafe impl xt_stable_abi::StableElement for Explodes {
        unsafe fn drop_in_place(pointer: *mut Self) {
            // SAFETY: the caller passes an initialised value it does not use
            // again, and the destructor this runs is the one under test.
            unsafe { core::ptr::drop_in_place(pointer) }
        }
    }

    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let mut values: StableVec<Explodes> = StableVec::with_capacity_and_allocator(allocator, 4).unwrap();
    values.push(Explodes { marker: 1 });
    values.push(Explodes { marker: 2 });

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || drop(values)));
    assert!(outcome.is_err(), "the destructor did not give up");
    // One element was released before the panic and the other was left alive,
    // which is what the standard library's `Vec` does; the block itself is not
    // allowed to leak with it.
    assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_slice_copies_into_a_vector_by_either_form() {
    let source = [4u64, 5, 6];
    let slice = StableSlice::from_slice(&source);
    let cloned = slice.to_vec().unwrap();
    let copied = slice.to_copy_vec().unwrap();
    assert_eq!(cloned.as_slice(), copied.as_slice());
    assert_eq!(copied.as_slice(), &[4, 5, 6]);
    // The copy owns its block: changing it leaves the slice it came from alone.
    let mut copied = copied;
    copied.as_mut_slice()[0] = 9;
    assert_eq!(source, [4, 5, 6]);
}
