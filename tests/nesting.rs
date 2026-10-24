//! Containers of containers, and the conversions to and from native types.

mod support;

use std::sync::atomic::Ordering;

use support::{tracked_allocator, Tracker};
use xt_stable_abi::{StableOption, StableSlice, StableString, StableVec};

#[test]
fn a_vector_of_vectors_roundtrips() {
    let mut rows: StableVec<StableVec<u32>> = StableVec::new();
    for row in 0..3u32 {
        let mut values = StableVec::new();
        for column in 0..4u32 {
            values.push(row * 10 + column);
        }
        rows.push(values);
    }

    assert_eq!(rows.len(), 3);
    for (row, values) in rows.iter().enumerate() {
        assert_eq!(values.len(), 4);
        for (column, value) in values.iter().enumerate() {
            assert_eq!(*value, row as u32 * 10 + column as u32);
        }
    }
}

#[test]
fn a_vector_of_optional_vectors_roundtrips() {
    let mut values: StableVec<StableOption<StableVec<u8>>> = StableVec::new();
    let mut filled = StableVec::new();
    filled.push(1u8);
    filled.push(2u8);
    values.push(StableOption::some(filled));
    values.push(StableOption::none());
    let mut other = StableVec::new();
    other.push(3u8);
    values.push(StableOption::some(other));

    assert_eq!(values.len(), 3);
    assert_eq!(
        values.get(0).and_then(|value| value.as_ref()).map(StableVec::as_slice),
        Some(&[1u8, 2u8][..])
    );
    assert!(values.get(1).expect("index 1 is inside").is_none());
    assert_eq!(
        values.get(2).and_then(|value| value.as_ref()).map(StableVec::as_slice),
        Some(&[3u8][..])
    );
}

#[test]
fn a_removed_inner_vector_releases_only_its_own_blocks() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    {
        let mut outer: StableVec<StableVec<u32>> = StableVec::new_with_allocator(allocator);
        for row in 0..3u32 {
            let mut inner = StableVec::new_with_allocator(allocator);
            for column in 0..8u32 {
                inner.push(row * 8 + column);
            }
            outer.push(inner);
        }
        // One block per vector, outer included.
        assert_eq!(tracker.outstanding(), 4);

        let removed = outer.remove(0).expect("index 0 is inside");
        assert_eq!(removed.len(), 8);
        drop(removed);
        assert_eq!(tracker.outstanding(), 3);
        assert_eq!(outer.len(), 2);
    }
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_vector_can_hold_borrowed_slices() {
    let first = [1u16, 2, 3];
    let second = [4u16, 5];
    let mut slices: StableVec<StableSlice<'_, u16>> = StableVec::new();
    slices.push(StableSlice::from_slice(&first));
    slices.push(StableSlice::from_slice(&second));

    assert_eq!(slices.len(), 2);
    assert_eq!(slices.get(0).map(StableSlice::as_slice), Some(&first[..]));
    assert_eq!(slices.get(1).map(StableSlice::as_slice), Some(&second[..]));
}

#[test]
fn native_collections_convert_in_both_directions() {
    let source = vec![3u32, 1, 4, 1, 5];
    let stable = StableVec::try_from(source.clone()).unwrap();
    assert_eq!(stable.as_slice(), &source[..]);

    let back: Vec<u32> = stable.into();
    assert_eq!(back, source);

    let from_slice = StableVec::<u32>::try_from(&source[..]).unwrap();
    assert_eq!(from_slice.as_slice(), &source[..]);

    let text = StableString::try_from("hello").unwrap();
    let native: String = text.into();
    assert_eq!(native, "hello");
}

#[test]
fn the_bulk_copy_constructor_matches_the_element_wise_one() {
    let values = [7u64, 8, 9, 10];
    let bulk = StableVec::from_copy_slice(&values).unwrap();
    let element_wise = StableVec::from_slice(&values).unwrap();
    assert_eq!(bulk.as_slice(), element_wise.as_slice());
    assert_eq!(bulk.as_slice(), &values[..]);

    let empty: [u64; 0] = [];
    let bulk_empty = StableVec::from_copy_slice(&empty).unwrap();
    assert!(bulk_empty.is_empty());
    assert_eq!(bulk_empty.capacity(), 0);
}

#[test]
fn nested_blocks_use_the_allocator_of_the_vector_that_owns_them() {
    let outer_tracker = Tracker::new();
    let inner_tracker = Tracker::new();
    {
        let mut outer: StableVec<StableVec<u8>> =
            StableVec::new_with_allocator(tracked_allocator(outer_tracker));
        for _ in 0..2 {
            outer.push(StableVec::new_with_allocator(tracked_allocator(inner_tracker)));
        }
        outer.get_mut(0).expect("index 0 is inside").push(1u8);
        outer.get_mut(1).expect("index 1 is inside").push(2u8);
        assert_eq!(outer_tracker.allocations.load(Ordering::SeqCst), 1);
        assert_eq!(inner_tracker.allocations.load(Ordering::SeqCst), 2);
    }
    assert_eq!(outer_tracker.outstanding(), 0);
    assert_eq!(inner_tracker.outstanding(), 0);
}
