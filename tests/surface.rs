//! The parts of the surface that exist for a caller to reach a value: the
//! accessors on the owned types, the iterators over the collections, and the
//! pieces of a path or a point in time.
//!
//! Every case here is about what a value says about itself, not about the bytes
//! crossing the boundary; the layout is pinned in `tests/layout.rs`.

use core::mem::{offset_of, size_of, MaybeUninit};

mod support;

use xt_stable_abi::{
    AbiStatus, StableArray, StableBox, StableHashMap, StableHashSet, StableMap, StableOsStr,
    StableOsString, StablePath, StablePathBuf, StableQuad, StableResult, StableSet, StableString,
    StableSystemTime, StableTriple, StableVec, StableDuration, StableOption, Wipe,
};

/// The layout of an optional value, for reading the bytes a `take` left behind.
#[repr(C)]
struct OptionMirror<T> {
    tag: u32,
    value: MaybeUninit<T>,
}

fn text(value: &str) -> StableString {
    StableString::from_str(value).unwrap()
}

#[test]
fn an_optional_value_can_be_filled_and_replaced() {
    let mut value: StableOption<u32> = StableOption::none();
    assert_eq!(value.get_or_insert(7), &7);
    assert_eq!(value.get_or_insert(9), &7, "a present value is kept");

    assert_eq!(value.replace(11), Some(7));
    assert_eq!(value.as_ref(), Some(&11));
    assert_eq!(value.insert(13), &13);
    assert_eq!(value.as_ref(), Some(&13));

    assert_eq!(StableOption::none().unwrap_or_else(|| 5), 5);
    assert_eq!(StableOption::some(1).or(StableOption::some(2)), StableOption::some(1));
    assert_eq!(StableOption::none().or(StableOption::some(2)), StableOption::some(2));
    assert_eq!(StableOption::some(1).xor(StableOption::some(2)), StableOption::none());
    assert_eq!(StableOption::some(1).xor(StableOption::none()), StableOption::some(1));
}

#[test]
fn wiping_an_optional_value_reaches_the_bytes_a_take_left_behind() {
    let mut value: StableOption<StableArray<u8, 8>> = StableOption::some(StableArray::new([0xA5; 8]));

    let taken = value.take().unwrap();
    assert_eq!(taken.as_slice(), &[0xA5; 8]);
    assert!(value.is_none());

    value.wipe_in_place();

    // The payload storage is where the value sat, and the tag no longer names a
    // value there: the wipe has to reach the bytes rather than the accessor.
    let offset = offset_of!(OptionMirror<StableArray<u8, 8>>, value);
    assert_eq!(size_of::<StableOption<StableArray<u8, 8>>>(), offset + 8);
    // SAFETY: the payload is `offset..offset + 8` of the value, and the mirror
    // states the same layout.
    let payload = unsafe {
        core::slice::from_raw_parts(
            (&value as *const StableOption<StableArray<u8, 8>> as *const u8).add(offset),
            8,
        )
    };
    assert_eq!(payload, &[0u8; 8], "the wipe reached the storage");
}

#[test]
fn a_result_can_be_turned_around_without_matching() {
    let ok: StableResult<u32, AbiStatus> = StableResult::ok(3);
    assert_eq!(ok.and(StableResult::ok(4)), StableResult::ok(4));
    let err: StableResult<u32, AbiStatus> = StableResult::err(AbiStatus::Panic);
    assert_eq!(err.and(StableResult::ok(4)), StableResult::err(AbiStatus::Panic));

    let ok: StableResult<u32, AbiStatus> = StableResult::ok(3);
    let fallback: StableResult<u32, AbiStatus> = StableResult::ok(9);
    assert_eq!(ok.or(fallback), StableResult::ok(3));
    let err: StableResult<u32, AbiStatus> = StableResult::err(AbiStatus::Panic);
    let fallback: StableResult<u32, AbiStatus> = StableResult::ok(9);
    assert_eq!(err.or(fallback), StableResult::ok(9));

    let mut seen = 0;
    let same: StableResult<u32, AbiStatus> = StableResult::ok(5);
    let returned = same.inspect(|value| seen = *value);
    assert_eq!(seen, 5);
    assert_eq!(returned, StableResult::ok(5));

    let default: StableResult<u32, AbiStatus> = StableResult::err(AbiStatus::Panic);
    assert_eq!(default.unwrap_or_default(), 0);
}

#[test]
fn a_boxed_value_can_be_borrowed_and_taken_apart() {
    let mut boxed = StableBox::try_new(7u64).unwrap();
    assert_eq!(boxed.as_ref(), Some(&7));
    *boxed.as_mut().unwrap() = 8;
    assert_eq!(*boxed, 8);

    let taken = boxed.take().unwrap();
    assert_eq!(taken, 8);
    assert!(boxed.is_empty());
    assert_eq!(boxed.as_ref(), None);

    let boxed = StableBox::try_new(3u64).unwrap();
    // SAFETY: the parts are handed straight back.
    let (pointer, allocator) = unsafe { boxed.into_raw() };
    // SAFETY: the parts came from `into_raw` and are not owned twice.
    let restored = unsafe { StableBox::from_raw(pointer, allocator) }.unwrap();
    assert_eq!(*restored, 3);

    let empty: StableBox<u64> = StableBox::default();
    assert!(empty.is_empty());
}

#[test]
fn a_map_can_be_walked_in_three_ways() {
    let mut map = StableMap::new();
    for number in [3u64, 1, 2] {
        map.insert(number, number * 10).unwrap();
    }

    let borrowed: Vec<(u64, u64)> = (&map).into_iter().map(|(k, v)| (*k, *v)).collect();
    assert_eq!(borrowed, [(1, 10), (2, 20), (3, 30)]);

    for (_, value) in &mut map {
        *value += 1;
    }
    let owned: Vec<(u64, u64)> = map.clone().into_iter().collect();
    assert_eq!(owned, [(1, 11), (2, 21), (3, 31)]);

    let values: Vec<u64> = map.values_mut().map(|value| *value).collect();
    assert_eq!(values, [11, 21, 31]);
    assert_eq!(map.first_key_value(), Some((&1, &11)));
    assert_eq!(map.last_key_value(), Some((&3, &31)));
}

#[test]
fn an_optional_value_can_be_turned_into_a_result() {
    let present: StableOption<u32> = StableOption::some(4);
    assert_eq!(present.ok_or(AbiStatus::InvalidArgument).into_result(), Ok(4));

    let empty: StableOption<u32> = StableOption::none();
    assert_eq!(
        empty.ok_or(AbiStatus::InvalidArgument).into_result(),
        Err(AbiStatus::InvalidArgument)
    );

    // The loop form yields the value when there is one and nothing otherwise,
    // which is what a caller needs when only the present case matters.
    let collected: Vec<u32> = StableOption::some(9).into_iter().collect();
    assert_eq!(collected, [9]);
    let kept: StableOption<u32> = StableOption::some(9);
    let borrowed: Vec<u32> = (&kept).into_iter().copied().collect();
    assert_eq!(borrowed, [9]);
    let nothing: Vec<u32> = StableOption::<u32>::none().into_iter().collect();
    assert!(nothing.is_empty());
}

#[test]
fn a_result_can_be_turned_around_and_walked() {
    let success: StableResult<StableOption<u32>, u32> = StableResult::ok(StableOption::some(3));
    let turned = success.transpose();
    assert_eq!(
        turned.as_ref().expect("the success carried a value").ok_ref(),
        Some(&3)
    );

    let empty: StableResult<StableOption<u32>, u32> = StableResult::ok(StableOption::none());
    assert!(empty.transpose().is_none());

    let failed: StableResult<StableOption<u32>, u32> = StableResult::err(8);
    let turned = failed.transpose();
    assert_eq!(turned.as_ref().and_then(StableResult::err_ref), Some(&8));

    let values: Vec<u32> = StableResult::<u32, u32>::ok(5).into_iter().collect();
    assert_eq!(values, [5]);
    let values: Vec<u32> = StableResult::<u32, u32>::err(5).into_iter().collect();
    assert!(values.is_empty());
    let borrowed: Vec<u32> = (&StableResult::<u32, u32>::ok(6)).into_iter().copied().collect();
    assert_eq!(borrowed, [6]);
}

#[test]
fn a_map_can_answer_its_key_and_give_entries_back() {
    let mut map: StableMap<u32, StableString> = StableMap::new();
    map.insert(1, text("one")).unwrap();
    map.insert(2, text("two")).unwrap();

    assert_eq!(
        map.get_key_value(&1).map(|(key, value)| (*key, value.as_str())),
        Some((1, "one"))
    );
    assert_eq!(map.get_key_value(&3), None);

    let removed = map.remove_entry(&2).expect("key 2 is present");
    assert_eq!((removed.0, removed.1.as_str()), (2, "two"));
    assert_eq!(map.remove_entry(&2), None);

    let drained: Vec<(u32, String)> = map
        .drain()
        .map(|(key, value)| (key, value.as_str().to_owned()))
        .collect();
    assert_eq!(drained, vec![(1, String::from("one"))]);
    assert!(map.is_empty());

    let mut again: StableMap<u32, u32> = StableMap::new();
    again.insert(4, 40).unwrap();
    again.insert(5, 50).unwrap();
    assert_eq!(again.clone().into_keys().collect::<Vec<_>>(), [4, 5]);
    assert_eq!(again.into_values().collect::<Vec<_>>(), [40, 50]);
}

#[test]
fn a_hash_map_can_answer_its_key_and_give_entries_back() {
    fn hash(key: &u32) -> u64 {
        u64::from(*key)
    }

    let mut map: StableHashMap<u32, StableString> = StableHashMap::new();
    map.insert(hash, 1, text("one")).unwrap();
    map.insert(hash, 2, text("two")).unwrap();

    assert_eq!(
        map.get_key_value(hash, &1)
            .map(|(key, value)| (*key, value.as_str())),
        Some((1, "one"))
    );
    assert_eq!(map.get_key_value(hash, &3), None);

    let removed = map.remove_entry(hash, &2).expect("key 2 is present");
    assert_eq!((removed.0, removed.1.as_str()), (2, "two"));

    let drained: Vec<String> = map.drain().map(|(_, value)| value.as_str().to_owned()).collect();
    assert_eq!(drained, vec![String::from("one")]);
    assert!(map.is_empty());

    let mut again: StableHashMap<u32, u32> = StableHashMap::new();
    again.insert(hash, 6, 60).unwrap();
    again.insert(hash, 7, 70).unwrap();
    assert_eq!(again.clone().into_keys().collect::<Vec<_>>(), [6, 7]);
    assert_eq!(again.into_values().collect::<Vec<_>>(), [60, 70]);
}

#[test]
fn a_collection_can_be_given_room_and_give_it_back() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    map.try_reserve(8).unwrap();
    assert!(map.capacity() >= 8);
    map.insert(1, 1).unwrap();
    map.try_shrink_to_fit().unwrap();
    assert_eq!(map.capacity(), 1);
    assert_eq!(map.get(&1), Some(&1));

    let mut set: StableSet<u32> = StableSet::new();
    set.try_reserve(8).unwrap();
    assert!(set.capacity() >= 8);
    set.insert(1).unwrap();
    set.try_shrink_to_fit().unwrap();
    assert_eq!(set.capacity(), 1);
    assert!(set.contains(&1));
}

#[test]
fn a_name_can_grow_one_part_at_a_time() {
    let mut name = StableOsString::new();
    name.push(StableOsStr::from_str("fur")).unwrap();
    name.push(StableOsStr::from_str("ther")).unwrap();
    assert_eq!(name.to_str(), Some("further"));
    assert_eq!(name.as_os_str().unwrap(), std::ffi::OsStr::new("further"));

    // Two names that follow different encodings describe bytes under two rules,
    // so appending one to the other would leave a value whose encoding
    // contradicts its bytes.
    let unix_bytes = [0x66u8];
    let mut unix = StableOsString::new();
    unix.push(
        StableOsStr::from_bytes(&unix_bytes, xt_stable_abi::path::ENCODING_UNIX).unwrap(),
    )
    .unwrap();
    assert_eq!(unix.encoding(), xt_stable_abi::path::ENCODING_UNIX);
    assert_eq!(
        unix.push(StableOsStr::from_str("ascii")).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
}

#[test]
fn a_borrowed_value_can_take_ownership_through_a_chosen_allocator() {
    use xt_stable_abi::{StableCowSlice, StableCowStr};

    let tracker = support::Tracker::new();
    let allocator = support::tracked_allocator(tracker);
    let borrowed = StableCowStr::borrowed("text");
    let owned = borrowed.into_owned_with_allocator(allocator).unwrap();
    assert_eq!(owned.as_str(), "text");
    assert_eq!(owned.allocator().context(), allocator.context());

    let values = [1u32, 2, 3];
    let borrowed = StableCowSlice::borrowed(&values);
    let owned = borrowed.into_owned_with_allocator(allocator).unwrap();
    assert_eq!(owned.as_slice(), &values);
    assert_eq!(owned.allocator().context(), allocator.context());
}

#[test]
#[should_panic(expected = "the range starts past the key its end names")]
fn a_range_whose_start_is_past_its_end_is_refused() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    for key in [1u32, 2, 3] {
        map.insert(key, key).unwrap();
    }
    // An empty range that names something in the middle (`2..2`) is a range
    // with no entries; one whose start is past its end names nothing at all.
    assert_eq!(map.range(2..2).count(), 0);
    let start = 3u32;
    let end = 1u32;
    let _ = map.range(start..=end).count();
}

#[test]
fn a_hash_set_gives_its_values_back_and_orders_like_its_map() {
    fn hash(key: &u32) -> u64 {
        u64::from(*key)
    }

    let mut set: StableHashSet<u32> = StableHashSet::new();
    for value in [3u32, 1, 2] {
        set.insert(hash, value).unwrap();
    }
    assert_eq!(set.drain().collect::<Vec<_>>(), [1, 2, 3]);
    assert!(set.is_empty());

    let mut one: StableHashSet<u32> = StableHashSet::new();
    one.insert(hash, 1).unwrap();
    let mut two: StableHashSet<u32> = StableHashSet::new();
    two.insert(hash, 2).unwrap();
    assert!(one < two);
    assert_eq!(one.cmp(&two), core::cmp::Ordering::Less);

    // Equal contents fold to one hash, which is what `Hash` has to promise when
    // `Eq` says two values are the same.
    use core::hash::BuildHasher;
    let state = std::collections::hash_map::RandomState::new();
    let mut copy: StableHashSet<u32> = StableHashSet::new();
    copy.insert(hash, 1).unwrap();
    assert_eq!(state.hash_one(&one), state.hash_one(&copy));
}

#[test]
fn a_drain_can_be_taken_from_either_end() {
    let mut values = StableVec::from_slice(&[1u8, 2, 3, 4, 5]).unwrap();
    {
        let mut drain = values.drain_range(1, 4);
        assert_eq!(drain.next(), Some(2));
        assert_eq!(drain.next_back(), Some(4));
        assert_eq!(drain.len(), 1);
    }
    // The element left in the middle is released, the tail moves down into the
    // hole, and the two ends stay where they were.
    assert_eq!(values.as_slice(), &[1, 5]);
}

#[test]
fn a_drain_taken_only_from_the_back_still_moves_the_tail() {
    let mut values = StableVec::from_slice(&[1u8, 2, 3, 4, 5]).unwrap();
    {
        let mut drain = values.drain_range(1, 4);
        assert_eq!(drain.next_back(), Some(4));
    }
    // What the caller did not take is released when the drain goes away, so the
    // range it named is gone either way and the tail closes the gap.
    assert_eq!(values.as_slice(), &[1, 5]);

    // A drain of the whole vector leaves it empty, however much of the back was
    // taken.
    let mut values = StableVec::from_slice(&[1u8, 2, 3, 4]).unwrap();
    {
        let mut drain = values.drain();
        assert_eq!(drain.next_back(), Some(4));
    }
    assert!(values.is_empty());
}

#[test]
fn a_failed_extend_leaves_the_hash_map_as_it_was() {
    let mut map: StableHashMap<u32, u32> = StableHashMap::new();
    map.insert(|key| u64::from(*key), 9, 9).unwrap();

    /// Yields one pair and then gives up, which is a source that fails after
    /// the first item rather than before it.
    struct Halfway {
        first: Option<(u32, u32)>,
    }

    impl Iterator for Halfway {
        type Item = (u32, u32);

        fn next(&mut self) -> Option<(u32, u32)> {
            match self.first.take() {
                Some(pair) => Some(pair),
                None => panic!("the source gave up"),
            }
        }
    }

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        map.try_extend(
            |key| u64::from(*key),
            Halfway {
                first: Some((1, 1)),
            },
        )
    }));
    assert!(outcome.is_err());
    // The source is read to the end before the map changes, so the pair it did
    // yield is not stored.
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(|key| u64::from(*key), &1), None);
}

#[test]
fn a_duration_can_be_built_from_nanoseconds() {
    let length = StableDuration::from_nanos(1_500_000_000).unwrap();
    assert_eq!(length.as_secs(), 1);
    assert_eq!(length.subsec_nanos(), 500_000_000);
    let short = StableDuration::from_nanos(999).unwrap();
    assert_eq!(short.as_secs(), 0);
    assert_eq!(short.subsec_nanos(), 999);
}

#[test]
fn a_map_can_be_split_and_joined() {
    let mut map = StableMap::new();
    for number in [1u64, 2, 3, 4] {
        map.insert(number, number).unwrap();
    }

    let tail = map.split_off(&3).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(tail.len(), 2);
    assert_eq!(tail.get(&3), Some(&3));

    let mut other = StableMap::new();
    other.insert(2, 20).unwrap();
    other.insert(5, 50).unwrap();
    map.append(&mut other).unwrap();
    assert!(other.is_empty());
    // The value from `other` replaces the one the map held.
    assert_eq!(map.get(&2), Some(&20));
    assert_eq!(map.get(&5), Some(&50));

    let ranged: Vec<u64> = map.range(2..=3).map(|(key, _)| *key).collect();
    assert_eq!(ranged, [2]);
}

#[test]
fn joining_two_maps_leaves_each_map_with_its_own_allocator() {
    use std::sync::atomic::Ordering;

    use support::{tracked_allocator, Tracker};

    let left_tracker = Tracker::new();
    let right_tracker = Tracker::new();
    let left_allocator = tracked_allocator(left_tracker);
    let right_allocator = tracked_allocator(right_tracker);

    let mut left: StableMap<u64, u64> = StableMap::new_with_allocator(left_allocator);
    left.insert(1, 10).unwrap();
    let mut right: StableMap<u64, u64> = StableMap::new_with_allocator(right_allocator);
    right.insert(2, 20).unwrap();

    left.append(&mut right).unwrap();

    assert!(right.is_empty());
    // The emptied map is still one whose storage comes from the allocator it
    // arrived with: a later insert must reach that allocator, not the other
    // one's.
    assert_eq!(right.allocator().context(), right_allocator.context());
    assert_eq!(left.allocator().context(), left_allocator.context());
    let before_left = left_tracker.allocations.load(Ordering::SeqCst);
    let before_right = right_tracker.allocations.load(Ordering::SeqCst);
    right.insert(3, 30).unwrap();
    assert_eq!(right_tracker.allocations.load(Ordering::SeqCst), before_right + 1);
    assert_eq!(left_tracker.allocations.load(Ordering::SeqCst), before_left);
}

#[test]
fn a_set_says_how_two_sets_relate() {
    let small: StableSet<u64> = StableSet::from_values(&[1, 2]).unwrap();
    let large: StableSet<u64> = StableSet::from_values(&[1, 2, 3]).unwrap();
    let other: StableSet<u64> = StableSet::from_values(&[3, 4]).unwrap();

    assert!(small.is_subset(&large));
    assert!(large.is_superset(&small));
    assert!(!large.is_subset(&small));
    assert!(!small.is_disjoint(&StableSet::from_values(&[1]).unwrap()));
    assert!(small.is_disjoint(&other));

    let either = small.symmetric_difference(&other).unwrap();
    assert_eq!(either.as_values(), &[1, 2, 3, 4]);

    let mut extended: StableSet<u64> = StableSet::new();
    extended.try_extend([3u64, 1, 3]).unwrap();
    assert_eq!(extended.as_values(), &[1, 3]);
    assert_eq!(extended.first(), Some(&1));
    assert_eq!(extended.last(), Some(&3));

    let walked: Vec<u64> = (&small).into_iter().copied().collect();
    assert_eq!(walked, [1, 2]);
    assert_eq!(small.into_iter().collect::<Vec<u64>>(), [1, 2]);
}

#[test]
fn a_hash_map_can_be_walked_and_grown() {
    fn hash(key: &u64) -> u64 {
        *key
    }

    let mut map: StableHashMap<u64, u64> = StableHashMap::new();
    map.try_extend(hash, [(1u64, 10u64), (2, 20), (1, 11)]).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(*map.get(hash, &1).unwrap(), 11);

    for (_, value) in &mut map {
        *value += 1;
    }
    let mut pairs: Vec<(u64, u64)> = (&map).into_iter().map(|(k, v)| (*k, *v)).collect();
    pairs.sort();
    assert_eq!(pairs, [(1, 12), (2, 21)]);

    let owned: Vec<(u64, u64)> = map.into_iter().collect();
    assert_eq!(owned, [(1, 12), (2, 21)]);

    let mut set: StableHashSet<u64> = StableHashSet::new();
    set.try_extend(hash, [5u64, 6, 5]).unwrap();
    assert_eq!(set.len(), 2);
    let mut values: Vec<u64> = (&set).into_iter().copied().collect();
    values.sort();
    assert_eq!(values, [5, 6]);
    let mut owned: Vec<u64> = set.into_iter().collect();
    owned.sort();
    assert_eq!(owned, [5, 6]);
}

#[test]
fn an_entry_carries_its_own_copy() {
    use xt_stable_abi::StableHashEntry;
    use xt_stable_abi::TryClone;

    let entry = StableHashEntry::new(7, text("a"), 1u32);
    let copy = entry.try_clone().unwrap();
    assert_eq!(copy.hash, 7);
    assert_eq!(copy.key.as_str(), "a");
    assert_eq!(copy.value, 1);

    // The stored number is a function of the key, so a wipe has to reach it as
    // well as the key and the value.
    let mut entry = StableHashEntry::new(9, text("secret"), 1u32);
    entry.wipe_in_place();
    assert_eq!(entry.hash, 0);

    // The number is a function of the key, so the debug form must not report it:
    // the key prints its shape rather than its contents, and the number would
    // give back the contents the key withholds.
    let entry = StableHashEntry::new(9, text("secret"), 1u32);
    assert_eq!(
        format!("{entry:?}"),
        "StableHashEntry { key: StableString { len: 6 }, value: 1 }"
    );
}

#[test]
fn an_extension_that_fails_leaves_the_vector_as_it_was() {
    use core::ffi::c_void;
    use core::sync::atomic::{AtomicUsize, Ordering};

    /// Allows one block and refuses every later request.
    static REMAINING: AtomicUsize = AtomicUsize::new(1);

    unsafe extern "C" fn allocate(_context: *mut c_void, size: u64, align: u64) -> *mut u8 {
        if REMAINING.swap(0, Ordering::SeqCst) == 0 {
            return core::ptr::null_mut();
        }
        xt_stable_abi::SYSTEM_ALLOCATOR.allocate(size, align)
    }

    unsafe extern "C" fn deallocate(
        _context: *mut c_void,
        pointer: *mut u8,
        size: u64,
        align: u64,
    ) {
        // SAFETY: the block came from the process allocator with this layout.
        unsafe { xt_stable_abi::SYSTEM_ALLOCATOR.deallocate(pointer, size, align) }
    }

    /// Refuses every request to grow a block.
    unsafe extern "C" fn reallocate(
        _context: *mut c_void,
        _pointer: *mut u8,
        _old_size: u64,
        _align: u64,
        _new_size: u64,
    ) -> *mut u8 {
        core::ptr::null_mut()
    }

    // SAFETY: the three functions describe an allocator, and the static above
    // holds the state they read. The reallocation is never called by this test.
    let allocator = unsafe {
        xt_stable_abi::AbiAllocator::from_parts(
            core::ptr::null_mut(),
            allocate,
            deallocate,
            reallocate,
        )
    };

    let mut values: StableVec<u32> = StableVec::new_with_allocator(allocator);
    values.push(1);
    // The first block is larger than one element, so the list is long enough to
    // outgrow it whatever the minimum block is.
    let failed = values.try_extend(2u32..200).is_err();
    assert!(failed, "the allocator refused the growth");
    assert_eq!(
        values.as_slice(),
        &[1],
        "the values added before the failure were released"
    );
}

#[test]
fn a_status_can_be_wiped() {
    let mut statuses: StableVec<AbiStatus> = StableVec::new();
    statuses.push(AbiStatus::Panic);
    statuses.push(AbiStatus::OutOfMemory);
    statuses.wipe_in_place();
    assert_eq!(statuses.as_slice(), &[AbiStatus::Ok, AbiStatus::Ok]);
}

#[test]
fn a_path_can_be_taken_apart_and_put_back_together() {
    let path = StablePath::from_str("/one/two.txt");
    // Whether that names a place from its root is the platform's answer.
    assert_eq!(path.is_absolute(), std::path::Path::new("/one/two.txt").is_absolute());
    assert_eq!(path.file_name().map(|name| name.to_str()), Some(Some("two.txt")));
    assert_eq!(
        path.extension().map(|name| name.to_str()),
        Some(Some("txt"))
    );
    assert_eq!(path.parent().map(|parent| parent.to_str()), Some(Some("/one")));

    let joined = path.parent().unwrap().join(StablePath::from_str("three.txt")).unwrap();
    // The separator is the platform's, which is why the answer is compared with
    // the one the platform builds.
    assert_eq!(
        joined.to_str(),
        std::path::Path::new("/one").join("three.txt").to_str()
    );

    let mut owned = StablePathBuf::from_text("/one").unwrap();
    owned.push(StablePath::from_str("two")).unwrap();
    assert_eq!(
        owned.to_str(),
        std::path::Path::new("/one").join("two").to_str()
    );
    assert!(owned.pop().unwrap());
    assert_eq!(owned.to_str(), Some("/one"));

    assert!(!StablePath::from_str("relative/file").is_absolute());
}

#[test]
fn an_owned_path_answers_the_questions_a_borrowed_one_does() {
    let owned = StablePathBuf::from_text("/one/two.txt").unwrap();
    let borrowed = StablePath::from_str("/one/two.txt");

    assert_eq!(
        owned.file_name().unwrap().map(|name| name.to_str()),
        borrowed.file_name().map(|name| name.to_str())
    );
    assert_eq!(
        owned.extension().unwrap().map(|name| name.to_str()),
        borrowed.extension().map(|name| name.to_str())
    );
    assert_eq!(
        owned
            .parent()
            .unwrap()
            .map(|parent| parent.to_str().map(str::to_owned)),
        borrowed
            .parent()
            .map(|parent| parent.to_str().map(str::to_owned))
    );

    let joined = owned.join(StablePath::from_str("three.txt")).unwrap();
    assert_eq!(
        joined.to_str(),
        std::path::Path::new("/one/two.txt").join("three.txt").to_str()
    );

    let mut renamed = owned.try_clone().unwrap();
    assert!(renamed
        .set_extension(StableOsStr::from_str("md"))
        .unwrap());
    assert_eq!(renamed.to_str(), Some("/one/two.md"));

    let replaced = borrowed
        .with_extension(StableOsStr::from_str("md"))
        .unwrap();
    assert_eq!(replaced.to_str(), renamed.to_str());

    // A borrowed path carries no allocator, so the caller names the one the
    // answer's storage comes from.
    let tracker = support::Tracker::new();
    let allocator = support::tracked_allocator(tracker);
    let before = tracker
        .allocations
        .load(std::sync::atomic::Ordering::SeqCst);
    let joined = borrowed
        .join_with_allocator(StablePath::from_str("three.txt"), allocator)
        .unwrap();
    assert_eq!(joined.allocator().context(), allocator.context());
    assert!(tracker.allocations.load(std::sync::atomic::Ordering::SeqCst) > before);
}

#[test]
fn a_point_in_time_and_a_length_can_be_compared() {
    let early = StableSystemTime::new(100, 750_000_000).unwrap();
    let late = StableSystemTime::new(103, 250_000_000).unwrap();

    let length = late.duration_since(early).unwrap();
    assert_eq!(length.as_secs(), 2);
    assert_eq!(length.subsec_nanos(), 500_000_000);
    assert!(early.duration_since(late).is_err());

    assert_eq!(late.checked_add(length).unwrap(), StableSystemTime::new(105, 750_000_000).unwrap());
    assert_eq!(late.checked_sub(length).unwrap(), StableSystemTime::new(100, 750_000_000).unwrap());

    let half = StableDuration::from_secs(2).checked_div(2).unwrap();
    assert_eq!(half.as_secs(), 1);
    assert_eq!(StableDuration::from_millis(1_500).unwrap().as_secs_f64(), 1.5);
    assert_eq!(StableDuration::from_micros(1_500_000).unwrap().as_secs_f64(), 1.5);
    assert!(StableDuration::from_secs(1).checked_div(0).is_none());
}

#[test]
fn an_operation_refuses_a_value_the_type_does_not_define() {
    let valid_point = StableSystemTime::new(100, 500_000_000).unwrap();
    let valid_span = StableDuration::from_secs(1);

    // The fields are public, so safe code can build a value the type refuses.
    // The fraction is one and a half seconds: the arithmetic folds the extra
    // second into the seconds field and answers with a value that looks valid,
    // which is why the check has to run before it.
    let mut point = valid_point;
    point.nanos = 1_500_000_000;
    assert_eq!(
        point.checked_add(valid_span).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    assert_eq!(
        point.checked_sub(valid_span).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    assert_eq!(
        point.duration_since(valid_point).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    assert_eq!(
        valid_point.duration_since(point).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );

    let mut span = valid_span;
    span.nanos = 1_500_000_000;
    assert_eq!(
        valid_point.checked_add(span).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    assert_eq!(
        valid_point.checked_sub(span).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    // The operations that answer an optional value say so the same way.
    assert!(span.checked_div(8).is_none());
    assert!(span.checked_mul(2).is_none());
}

#[test]
fn a_length_too_large_for_the_signed_field_is_refused() {
    let point = StableSystemTime::new(0, 0).unwrap();
    // `StableDuration` counts seconds as an unsigned number, so a value that
    // arrived from the other side can name more of them than a point in time
    // holds. The conversion must fail rather than wrap into the sign bit.
    let too_long = StableDuration::from_secs(1 << 63);
    assert_eq!(
        point.checked_add(too_long).unwrap_err().status(),
        AbiStatus::CapacityOverflow
    );
    assert_eq!(
        point.checked_sub(too_long).unwrap_err().status(),
        AbiStatus::CapacityOverflow
    );

    // The largest length that does fit still works.
    let longest = StableDuration::from_secs(i64::MAX as u64);
    assert_eq!(
        point.checked_add(longest).unwrap(),
        StableSystemTime::new(i64::MAX, 0).unwrap()
    );
    assert_eq!(
        point.checked_sub(longest).unwrap(),
        StableSystemTime::new(-i64::MAX, 0).unwrap()
    );
}

#[test]
fn an_extension_of_a_set_that_fails_leaves_it_as_it_was() {
    use core::ffi::c_void;
    use core::sync::atomic::{AtomicUsize, Ordering};

    /// Allows one block and refuses every later request.
    static REMAINING: AtomicUsize = AtomicUsize::new(1);

    unsafe extern "C" fn allocate(_context: *mut c_void, size: u64, align: u64) -> *mut u8 {
        if REMAINING.swap(0, Ordering::SeqCst) == 0 {
            return core::ptr::null_mut();
        }
        xt_stable_abi::SYSTEM_ALLOCATOR.allocate(size, align)
    }

    unsafe extern "C" fn deallocate(
        _context: *mut c_void,
        pointer: *mut u8,
        size: u64,
        align: u64,
    ) {
        // SAFETY: the block came from the process allocator with this layout.
        unsafe { xt_stable_abi::SYSTEM_ALLOCATOR.deallocate(pointer, size, align) }
    }

    unsafe extern "C" fn reallocate(
        _context: *mut c_void,
        _pointer: *mut u8,
        _old_size: u64,
        _align: u64,
        _new_size: u64,
    ) -> *mut u8 {
        core::ptr::null_mut()
    }

    // SAFETY: the three functions describe an allocator over a static.
    let allocator = unsafe {
        xt_stable_abi::AbiAllocator::from_parts(
            core::ptr::null_mut(),
            allocate,
            deallocate,
            reallocate,
        )
    };

    let mut set: StableSet<u64> = StableSet::new_with_allocator(allocator);
    set.insert(1).unwrap();
    let failed = set.try_extend([2u64, 3, 4, 5]).is_err();
    assert!(failed, "the allocator refused the block the result needs");
    assert_eq!(set.as_values(), &[1], "the set holds what it held before");
}

#[test]
fn a_tuple_prints_its_parts() {
    let pair: StableTriple<u32, u32, u32> = StableTriple::new(1, 2, 3);
    assert_eq!(pair.to_string(), "(1, 2, 3)");
    let quad: StableQuad<u32, u32, u32, u32> = StableQuad::new(1, 2, 3, 4);
    assert_eq!(quad.to_string(), "(1, 2, 3, 4)");
}
