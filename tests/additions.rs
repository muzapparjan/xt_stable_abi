//! The operations the collection types grew, and the shapes a guarded call can
//! answer with.

use xt_stable_abi::{
    cache::Once, stable_object_guarded, AbiError, AbiStatus, HandleTable, StableBox, StableDuration,
    StableHash, StableHashMap, StableHashEntry, StableHashSet, StableMap, StableSet, StableVec,
};

#[test]
fn an_entry_fills_a_vacant_place_and_keeps_an_occupied_one() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    map.insert(1, 10).unwrap();
    map.insert(3, 30).unwrap();

    let filled = map.entry(2).or_insert(20).copied().unwrap();
    assert_eq!(filled, 20);
    let kept = map.entry(1).or_insert(99).copied().unwrap();
    assert_eq!(kept, 10);
    // The lookup that found nothing is the one the insertion used, so the map
    // stays ordered without a second search.
    assert_eq!(map.keys().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
    assert!(!map.entry(4).is_occupied());
    assert_eq!(map.entry(1).key(), &1);
}

#[test]
fn a_map_can_be_given_room_and_a_set_too() {
    let map: StableMap<u32, u32> = StableMap::with_capacity(8).unwrap();
    assert!(map.capacity() >= 8);
    let set: StableSet<u32> = StableSet::with_capacity(8).unwrap();
    assert!(set.capacity() >= 8);
    assert!(set.is_empty());
}

#[test]
fn a_vector_drains_its_values_and_releases_the_ones_it_did_not_yield() {
    let mut values: StableVec<u32> = StableVec::new();
    values.try_extend([1, 2, 3, 4]).unwrap();
    {
        let mut drain = values.drain();
        assert_eq!(drain.next(), Some(1));
        assert_eq!(drain.len(), 3);
        // The rest is released when the iterator goes out of scope.
    }
    assert!(values.is_empty());
    assert!(values.capacity() >= 4);

    values.try_extend([5, 6]).unwrap();
    let collected: Vec<u32> = values.drain().collect();
    assert_eq!(collected, vec![5, 6]);
    assert!(values.is_empty());
}

#[test]
fn a_vector_drains_a_range_and_moves_the_tail_into_the_gap() {
    let mut values: StableVec<u32> = StableVec::new();
    values.try_extend([1, 2, 3, 4, 5, 6]).unwrap();
    {
        let mut drain = values.drain_range(1, 4);
        assert_eq!(drain.len(), 3);
        assert_eq!(drain.next(), Some(2));
        assert_eq!(drain.next(), Some(3));
        // The rest is released when the iterator goes out of scope.
    }
    assert_eq!(values.as_slice(), &[1, 5, 6]);

    // A range that yields nothing still leaves the vector as it was.
    let empty: Vec<u32> = values.drain_range(1, 1).collect();
    assert!(empty.is_empty());
    assert_eq!(values.as_slice(), &[1, 5, 6]);

    // A range at the end needs no move.
    let tail: Vec<u32> = values.drain_range(2, 3).collect();
    assert_eq!(tail, vec![6]);
    assert_eq!(values.as_slice(), &[1, 5]);
}

#[test]
fn retain_mut_sees_the_values_it_decides_on() {
    let mut values: StableVec<u32> = StableVec::new();
    values.try_extend([1, 2, 3, 4]).unwrap();
    values.retain_mut(|value| {
        *value *= 10;
        *value % 20 == 0
    });
    assert_eq!(values.as_slice(), &[20, 40]);
}

#[test]
fn spare_capacity_covers_the_slots_the_length_does_not() {
    let mut values: StableVec<u32> = StableVec::with_capacity(4).unwrap();
    values.try_push(1).unwrap();
    let spare = values.spare_capacity_mut();
    assert_eq!(spare.len() as u64, values.capacity() - 1);
}

#[test]
fn reserving_exactly_gives_at_least_the_requested_room() {
    let mut values: StableVec<u32> = StableVec::new();
    values.try_reserve_exact(3).unwrap();
    assert!(values.capacity() >= 3);
    values.try_push(1).unwrap();
    values.try_reserve_exact(1).unwrap();
    assert!(values.capacity() >= 2);
    assert_eq!(values.as_slice(), &[1]);
}

#[test]
fn a_duration_can_be_added_and_subtracted() {
    let hour = StableDuration::new(3600, 0).unwrap();
    let half = StableDuration::new(0, 500_000_000).unwrap();
    let total = hour.checked_add(half).unwrap();
    assert_eq!(total.secs, 3600);
    assert_eq!(total.nanos, 500_000_000);

    let back = total.checked_sub(half).unwrap();
    assert_eq!(back, hour);

    // A duration has no sign, so a subtraction that would go below zero has no
    // answer.
    assert_eq!(half.checked_sub(hour), None);
    assert_eq!(StableDuration::new(0, 0).unwrap().checked_sub(half), None);
}

#[test]
fn an_error_can_be_copied_and_wiped() {
    let error = AbiError::with_message(AbiStatus::InvalidTag, "why");
    let copy = error.clone();
    assert_eq!(copy, error);
    assert_eq!(error.wipe(), AbiStatus::InvalidTag);
}

#[test]
fn a_boxed_value_can_be_wiped_and_still_released() {
    let boxed = StableBox::try_new(7u64).unwrap();
    boxed.wipe().unwrap();
}

#[test]
fn a_handle_table_can_retain_and_drain() {
    let mut table: HandleTable<u32> = HandleTable::new();
    let first = table.insert(1);
    let second = table.insert(2);
    let third = table.insert(3);

    table.retain(|_, value| *value != 2);
    assert!(table.contains(first));
    assert!(!table.contains(second));
    assert!(table.contains(third));
    assert_eq!(table.len(), 2);

    let drained: Vec<(u32, u32)> = table.drain().map(|(handle, value)| (handle.to_raw() as u32, value)).collect();
    assert_eq!(drained.len(), 2);
    assert!(table.is_empty());
    assert!(!table.contains(first));
    assert!(!table.contains(third));

    // The slots the drain freed come back with a new generation, so a handle
    // taken before it never names the value that arrives next.
    let fourth = table.insert(4);
    assert!(!table.contains(first));
    assert!(!table.contains(third));
    assert_eq!(table.get(fourth), Some(&4));
    assert_ne!(fourth.generation(), first.generation());
}

#[test]
fn a_drain_that_stops_early_leaves_the_entries_after_the_cursor() {
    let mut table: HandleTable<u32> = HandleTable::new();
    let first = table.insert(1);
    let second = table.insert(2);
    let third = table.insert(3);

    {
        let mut drain = table.drain();
        assert_eq!(drain.next().map(|(_, value)| value), Some(1));
        assert_eq!(drain.len(), 2);
    }

    // The first slot was freed as it was yielded; the others were not reached.
    assert!(!table.contains(first));
    assert_eq!(table.get(second), Some(&2));
    assert_eq!(table.get(third), Some(&3));
    assert_eq!(table.len(), 2);

    // A slot freed by the drain is reused with another generation.
    let fourth = table.insert(4);
    assert_eq!(fourth.index(), first.index());
    assert_ne!(fourth.generation(), first.generation());
    assert!(!table.contains(first));
}

stable_object_guarded! {
    /// A handle to an implementation that answers with collections.
    pub struct InventoryObject;
    /// The table behind [`InventoryObject`].
    pub struct InventoryVTable;
    /// Answers with the shapes a guarded call has to be able to carry.
    pub trait Inventory {
        /// A map keyed by identifier.
        fn counts(&self) -> StableMap<u32, u32>;
        /// An error value, which is the natural failure shape.
        fn failure(&self) -> AbiError;
        /// A boxed number.
        fn boxed(&self) -> StableBox<u64>;
        /// A span of time.
        fn span(&self) -> StableDuration;
    }
}

struct Stock;

impl Inventory for Stock {
    fn counts(&self) -> StableMap<u32, u32> {
        let mut map: StableMap<u32, u32> = StableMap::new();
        map.insert(1, 2).unwrap();
        map
    }

    fn failure(&self) -> AbiError {
        AbiError::with_message(AbiStatus::InvalidHandle, "gone")
    }

    fn boxed(&self) -> StableBox<u64> {
        StableBox::try_new(9).unwrap()
    }

    fn span(&self) -> StableDuration {
        StableDuration::new(2, 0).unwrap()
    }
}

#[test]
fn a_guarded_call_can_answer_with_every_boundary_type() {
    let object = InventoryObject::abi_from_value(Stock);
    // SAFETY: the object was built by this trait's generated constructor.
    let counts = unsafe { object.counts() }.unwrap();
    assert_eq!(counts.get(&1), Some(&2));
    // SAFETY: the object was built by this trait's generated constructor.
    let boxed = unsafe { object.boxed() }.unwrap();
    assert_eq!(*boxed, 9);
    // SAFETY: the object was built by this trait's generated constructor.
    let span = unsafe { object.span() }.unwrap();
    assert_eq!(span.secs, 2);
    // SAFETY: the object was built by this trait's generated constructor.
    let failure = unsafe { object.failure() }.unwrap();
    assert_eq!(failure.status(), AbiStatus::InvalidHandle);
}

#[test]
#[should_panic(expected = "excludes the same key at both ends")]
fn a_map_range_that_excludes_one_key_twice_is_refused_when_the_key_is_absent() {
    // The range names nothing whether or not the key has an entry, so the same
    // input is not empty for one map and fatal for another.
    let mut map: StableMap<u32, u32> = StableMap::new();
    map.insert(1, 10).unwrap();
    let _ = map
        .range((
            core::ops::Bound::Excluded(9),
            core::ops::Bound::Excluded(9),
        ))
        .count();
}

#[test]
#[should_panic(expected = "excludes the same value at both ends")]
fn a_set_range_that_excludes_one_value_twice_is_refused() {
    let mut set: StableSet<u32> = StableSet::new();
    set.insert(1).unwrap();
    let _ = set
        .range((
            core::ops::Bound::Excluded(1),
            core::ops::Bound::Excluded(1),
        ))
        .count();
}

#[test]
fn the_ends_of_an_ordered_collection_can_be_taken() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    for key in [3u32, 1, 2] {
        map.insert(key, key * 10).unwrap();
    }
    assert_eq!(map.pop_first(), Some((1, 10)));
    assert_eq!(map.pop_last(), Some((3, 30)));
    assert_eq!(map.pop_first(), Some((2, 20)));
    assert_eq!(map.pop_first(), None);
    assert_eq!(map.pop_last(), None);

    let mut set: StableSet<u32> = StableSet::new();
    for value in [5u32, 2, 9] {
        set.insert(value).unwrap();
    }
    assert_eq!(set.pop_first(), Some(2));
    assert_eq!(set.pop_last(), Some(9));
    assert_eq!(set.pop_last(), Some(5));
    assert_eq!(set.pop_last(), None);
}

#[test]
fn a_map_built_with_another_hash_function_passes_the_layout_check_and_misses() {
    // The layout check does not recompute the numbers: the receiver does not
    // hold the sender's function, so it can only see that the entries are
    // ordered and that every value is one its type defines.
    let mut map: StableHashMap<u32, u32> = StableHashMap::new();
    map.insert(|value: &u32| u64::from(*value) * 3, 5, 50)
        .unwrap();
    assert!(map.validate().is_ok());
    // The search recomputes with the function it is given, so a different one
    // looks in another place and finds nothing.
    assert_eq!(map.get_stable(&5), None);
    assert!(map.validate_stable().is_err());
}

#[test]
fn a_hash_set_can_be_checked_against_a_hash_function() {
    let mut set: StableHashSet<u32> = StableHashSet::new();
    set.insert(|value: &u32| u64::from(*value) * 3, 5).unwrap();
    set.insert(|value: &u32| u64::from(*value) * 3, 7).unwrap();
    set.validate_with(|value: &u32| u64::from(*value) * 3).unwrap();
    // A different function names other places, so the receiver can tell that
    // the set was built with one it does not hold.
    assert_eq!(
        set.validate_with(|value: &u32| u64::from(*value) * 5)
            .unwrap_err()
            .status(),
        AbiStatus::InvalidDescriptor
    );
}

#[test]
fn a_once_in_a_local_variable_releases_its_value() {
    struct Counted(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    let drops = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let slot: Once<Counted> = Once::new();
        let _held = slot.get_or_init(|| Counted(drops.clone()));
        assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
    // The slot owned the value, so leaving its scope releases it once.
    assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn a_hash_map_merge_keeps_the_value_that_arrives_last() {
    let mut map: StableHashMap<u32, u32> = StableHashMap::new();
    map.insert_stable(1, 10).unwrap();
    map.insert_stable(2, 20).unwrap();
    map.try_extend(StableHash::stable_hash, [(2, 99), (3, 30), (3, 31)])
        .unwrap();
    assert_eq!(map.get_stable(&1), Some(&10));
    // The key the map already held takes the value from the incoming entry.
    assert_eq!(map.get_stable(&2), Some(&99));
    // Two writes for one key in the same batch leave the last one behind, which
    // is what a sequence of inserts would have left.
    assert_eq!(map.get_stable(&3), Some(&31));
    assert_eq!(map.len(), 3);
}

#[test]
fn a_map_merge_keeps_the_value_that_arrives_last() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    map.insert(1, 10).unwrap();
    map.insert(2, 20).unwrap();
    map.try_extend([(2, 99), (3, 30), (3, 31)]).unwrap();
    assert_eq!(
        map.iter().map(|(key, value)| (*key, *value)).collect::<Vec<_>>(),
        vec![(1, 10), (2, 99), (3, 31)]
    );
}

#[test]
fn a_set_hands_back_the_value_it_holds_in_a_range() {
    let mut set: StableSet<u32> = StableSet::new();
    for value in [5, 1, 3, 7] {
        set.insert(value).unwrap();
    }
    assert_eq!(set.get(&3), Some(&3));
    assert_eq!(set.get(&4), None);
    assert_eq!(set.range(2..=5).copied().collect::<Vec<_>>(), vec![3, 5]);
    assert_eq!(set.range(3..).copied().collect::<Vec<_>>(), vec![3, 5, 7]);
    assert_eq!(set.range(..3).copied().collect::<Vec<_>>(), vec![1]);
    assert_eq!(set.range(..).count(), 4);
}

#[test]
fn sorting_a_hash_map_that_repeats_a_key_is_refused() {
    let mut map: StableHashMap<u32, u32> = StableHashMap::new();
    map.insert_stable(1, 10).unwrap();
    map.insert_stable(2, 20).unwrap();
    {
        let entries = map.as_entries_mut();
        let repeat = StableHashEntry::new(entries[0].hash, entries[0].key, entries[0].value);
        entries[1] = repeat;
    }
    assert_eq!(
        map.sort_entries().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
}

#[test]
fn sorting_a_hash_set_that_repeats_a_value_is_refused() {
    let mut set: StableHashSet<u32> = StableHashSet::new();
    set.insert_stable(1).unwrap();
    set.insert_stable(2).unwrap();
    {
        let entries = set.as_entries_mut();
        let repeat = StableHashEntry::new(entries[0].hash, entries[0].key, ());
        entries[1] = repeat;
    }
    assert_eq!(
        set.sort_entries().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
}

#[test]
fn sorting_a_map_that_repeats_a_key_is_refused() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    map.insert(1, 10).unwrap();
    map.insert(2, 20).unwrap();
    {
        let entries = map.as_entries_mut();
        entries[1].left = entries[0].left;
    }
    assert_eq!(
        map.sort_entries().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
}
