//! Maps and sets kept in sorted order.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};

use support::{tracked_allocator, Tracker};
use xt_stable_abi::{StableHashMap, StableMap, StableSet, StableString};

/// A key whose ordering gives up while the flag is set.
///
/// A comparison is the one step of a merge that runs the caller's own code, and
/// a key whose `Ord` is not total is the shape that stops it part way through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cranky(u32);

static ORDER_GIVES_UP: AtomicBool = AtomicBool::new(false);

impl Ord for Cranky {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        assert!(
            !ORDER_GIVES_UP.load(Ordering::SeqCst),
            "the comparison gave up"
        );
        self.0.cmp(&other.0)
    }
}

impl PartialOrd for Cranky {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

xt_stable_abi::unsafe_stable_element!(Cranky);

#[test]
fn a_comparison_that_gives_up_leaves_the_map_whole() {
    let mut map: StableMap<Cranky, u32> = StableMap::new();
    for key in [1u32, 2, 4, 5] {
        map.insert(Cranky(key), key).unwrap();
    }

    // One arriving entry, so nothing is compared before the merge; the flag then
    // stops the merge's first comparison.
    ORDER_GIVES_UP.store(true, Ordering::SeqCst);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        map.try_extend([(Cranky(3), 30)])
    }));
    ORDER_GIVES_UP.store(false, Ordering::SeqCst);

    assert!(outcome.is_err(), "the comparison did not give up");
    assert_eq!(
        map.keys().map(|key| key.0).collect::<Vec<_>>(),
        vec![1, 2, 4, 5]
    );
}

#[test]
fn a_comparison_that_gives_up_leaves_the_set_whole() {
    let mut set: StableSet<Cranky> = StableSet::new();
    for value in [1u32, 2, 4, 5] {
        set.insert(Cranky(value)).unwrap();
    }

    ORDER_GIVES_UP.store(true, Ordering::SeqCst);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        set.try_extend([Cranky(3)])
    }));
    ORDER_GIVES_UP.store(false, Ordering::SeqCst);

    assert!(outcome.is_err(), "the comparison did not give up");
    assert_eq!(
        set.iter().map(|value| value.0).collect::<Vec<_>>(),
        vec![1, 2, 4, 5]
    );
}

#[test]
fn a_comparison_that_gives_up_leaves_the_hash_map_whole() {
    // Every key answers the same number, so the search and the merge always
    // reach the key comparison - which is the step under test.
    let hash = |_: &Cranky| 0u64;
    let mut map: StableHashMap<Cranky, u32> = StableHashMap::new();
    for key in [1u32, 2, 4, 5] {
        map.insert(hash, Cranky(key), key).unwrap();
    }

    ORDER_GIVES_UP.store(true, Ordering::SeqCst);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        map.try_extend(hash, [(Cranky(3), 30)])
    }));
    ORDER_GIVES_UP.store(false, Ordering::SeqCst);

    assert!(outcome.is_err(), "the comparison did not give up");
    assert_eq!(
        map.keys().map(|key| key.0).collect::<Vec<_>>(),
        vec![1, 2, 4, 5]
    );
}

#[test]
fn a_comparison_that_gives_up_leaves_both_maps_whole() {
    let mut left: StableMap<Cranky, u32> = StableMap::new();
    for key in [1u32, 2] {
        left.insert(Cranky(key), key).unwrap();
    }
    let mut right: StableMap<Cranky, u32> = StableMap::new();
    for key in [4u32, 5] {
        right.insert(Cranky(key), key).unwrap();
    }

    ORDER_GIVES_UP.store(true, Ordering::SeqCst);
    let outcome =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| left.append(&mut right)));
    ORDER_GIVES_UP.store(false, Ordering::SeqCst);

    assert!(outcome.is_err(), "the comparison did not give up");
    assert_eq!(left.keys().map(|key| key.0).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(
        right.keys().map(|key| key.0).collect::<Vec<_>>(),
        vec![4, 5]
    );
}

#[test]
fn a_map_orders_its_entries_and_answers_lookups() {
    let mut map: StableMap<u32, StableString> = StableMap::new();
    for key in [5u32, 1, 3] {
        map.insert(key, StableString::from_str(&format!("v{key}")).unwrap())
            .unwrap();
    }

    let keys: Vec<u32> = map.keys().copied().collect();
    assert_eq!(keys, vec![1, 3, 5]);
    assert_eq!(map.get(&3).map(StableString::as_str), Some("v3"));
    assert_eq!(map.get(&4), None);
    assert!(map.contains_key(&1));
    assert_eq!(map.len(), 3);
}

#[test]
fn inserting_a_present_key_replaces_the_value() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    assert_eq!(map.insert(2, 20).unwrap(), None);
    assert_eq!(map.insert(2, 21).unwrap(), Some(20));
    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&2), Some(&21));
}

#[test]
fn removing_an_entry_returns_its_value() {
    let mut map: StableMap<u32, u32> = StableMap::new();
    map.insert(1, 10).unwrap();
    map.insert(2, 20).unwrap();
    assert_eq!(map.remove(&1), Some(10));
    assert_eq!(map.remove(&1), None);
    assert_eq!(map.len(), 1);
    assert_eq!(map.keys().copied().collect::<Vec<_>>(), vec![2]);
}

#[test]
fn a_map_holds_owned_values_and_releases_them() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    {
        let mut map: StableMap<u32, StableString> = StableMap::new_with_allocator(allocator);
        for key in 0..4u32 {
            map.insert(
                key,
                StableString::from_str_with_allocator(allocator, &key.to_string()).unwrap(),
            )
            .unwrap();
        }
        // Four values plus the entry block.
        assert_eq!(tracker.outstanding(), 5);
        map.remove(&2).expect("key 2 is present");
        assert_eq!(tracker.outstanding(), 4);
    }
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_map_can_be_built_from_pairs() {
    let pairs = [(3u8, "c"), (1, "a"), (2, "b")];
    let map = StableMap::<u8, StableString>::from_pairs(
        &pairs
            .iter()
            .map(|(key, value)| (*key, StableString::from_str(value).unwrap()))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(map.len(), 3);
    assert_eq!(
        map.iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "a"), (2, "b"), (3, "c")]
    );
}

#[test]
fn a_set_keeps_each_value_once_in_order() {
    let mut set: StableSet<u32> = StableSet::new();
    assert!(set.insert(3).unwrap());
    assert!(set.insert(1).unwrap());
    assert!(!set.insert(3).unwrap());
    assert_eq!(set.as_values(), &[1, 3]);
    assert!(set.contains(&1));
    assert!(!set.contains(&2));
    assert!(set.remove(&1));
    assert!(!set.remove(&1));
    assert_eq!(set.as_values(), &[3]);
}

#[test]
fn equal_maps_produce_equal_bytes_in_the_same_order() {
    let left = StableMap::<u8, u16>::from_pairs(&[(2, 20), (1, 10)]).unwrap();
    let right = StableMap::<u8, u16>::from_pairs(&[(1, 10), (2, 20)]).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.as_entries(), right.as_entries());
}

#[test]
fn the_entry_block_uses_the_map_allocator() {
    let tracker = Tracker::new();
    let set: StableSet<u32> = StableSet::new_with_allocator(tracked_allocator(tracker));
    assert_eq!(set.allocator().context(), tracked_allocator(tracker).context());
    assert_eq!(set.len(), 0);
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 0);
}

mod hashed {
    use xt_stable_abi::{AbiType, StableHashMap, StableHashSet, StableString};

    fn identity(value: &u32) -> u64 {
        u64::from(*value)
    }

    fn doubled(value: &u32) -> u64 {
        u64::from(*value) * 2
    }

    fn key(value: &str) -> StableString {
        StableString::from_str(value).unwrap()
    }

    /// A value whose order looks at one field only.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Tagged {
        order: u32,
        label: u64,
    }

    xt_stable_abi::unsafe_stable_element!(Tagged);

    // SAFETY: every bit pattern of a `u32` and a `u64` is a value of the type.
    unsafe impl AbiType for Tagged {}

    impl PartialEq for Tagged {
        fn eq(&self, other: &Self) -> bool {
            self.order == other.order
        }
    }

    impl Eq for Tagged {}

    impl PartialOrd for Tagged {
        fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Tagged {
        fn cmp(&self, other: &Self) -> core::cmp::Ordering {
            self.order.cmp(&other.order)
        }
    }

    /// A key that counts the comparisons a search makes.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Counted(u32);

    xt_stable_abi::unsafe_stable_element!(Counted);

    // SAFETY: every bit pattern of a `u32` is a value of the type.
    unsafe impl AbiType for Counted {}

    static COMPARISONS: core::sync::atomic::AtomicUsize =
        core::sync::atomic::AtomicUsize::new(0);

    impl PartialEq for Counted {
        fn eq(&self, other: &Self) -> bool {
            COMPARISONS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
            self.0 == other.0
        }
    }

    impl Eq for Counted {}

    impl PartialOrd for Counted {
        fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Counted {
        fn cmp(&self, other: &Self) -> core::cmp::Ordering {
            COMPARISONS.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
            self.0.cmp(&other.0)
        }
    }

    #[test]
    fn two_maps_filled_with_one_function_are_equal_whatever_the_order() {
        let mut one: StableHashMap<u32, u32> = StableHashMap::new();
        one.insert(identity, 1, 10).unwrap();
        one.insert(identity, 2, 20).unwrap();

        let mut other: StableHashMap<u32, u32> = StableHashMap::new();
        other.insert(identity, 2, 20).unwrap();
        other.insert(identity, 1, 10).unwrap();

        assert_eq!(one, other);

        // The number is part of what names an entry, so two maps that hold the
        // same keys and values under different numbers are not equal, and a
        // lookup through the function the map was not filled with finds nothing.
        let mut plain: StableHashMap<u32, u32> = StableHashMap::new();
        plain.insert(identity, 1, 10).unwrap();
        let mut doubled_map: StableHashMap<u32, u32> = StableHashMap::new();
        doubled_map.insert(doubled, 1, 10).unwrap();
        assert_ne!(plain, doubled_map);
        assert_eq!(doubled_map.get(identity, &1), None);
        assert_eq!(doubled_map.get(doubled, &1), Some(&10));
    }

    #[test]
    fn a_lookup_that_misses_does_not_walk_the_whole_map() {
        let mut map: StableHashMap<Counted, u32> = StableHashMap::new();
        let count = 1024u32;
        let mut value = 0;
        while value < count {
            map.insert(|key: &Counted| u64::from(key.0), Counted(value), value)
                .unwrap();
            value += 1;
        }
        COMPARISONS.store(0, core::sync::atomic::Ordering::SeqCst);
        let absent = |key: &Counted| u64::from(key.0);
        assert!(map.get(absent, &Counted(count + 1)).is_none());
        let comparisons = COMPARISONS.load(core::sync::atomic::Ordering::SeqCst);
        // A binary search compares once per level; a scan by key compares once
        // per entry, so this bound tells them apart.
        assert!(comparisons <= 24, "{comparisons} comparisons for a miss");
    }

    #[test]
    fn filling_a_map_one_entry_at_a_time_does_not_scan_for_each_insert() {
        let mut map: StableHashMap<Counted, u32> = StableHashMap::new();
        let count = 1024u32;
        COMPARISONS.store(0, core::sync::atomic::Ordering::SeqCst);
        let mut value = 0;
        while value < count {
            map.insert(|key: &Counted| u64::from(key.0), Counted(value), value)
                .unwrap();
            value += 1;
        }
        let comparisons = COMPARISONS.load(core::sync::atomic::Ordering::SeqCst);
        // Each insert searches once per level of the map; a scan for a key the
        // map does not hold compares once per entry, which is what made filling
        // a map quadratic.
        let bound = 20 * count as usize;
        assert!(
            comparisons <= bound,
            "{comparisons} comparisons to fill {count} entries"
        );
    }

    #[test]
    fn checking_a_map_allocates_nothing() {
        use std::sync::atomic::Ordering;

        use super::{tracked_allocator, Tracker};

        let tracker = Tracker::new();
        let allocator = tracked_allocator(tracker);
        let mut map: StableHashMap<u32, u32> = StableHashMap::new_with_allocator(allocator);
        for key in [3u32, 1, 2] {
            map.insert(identity, key, key).unwrap();
        }
        let before = tracker.allocations.load(Ordering::SeqCst);
        <StableHashMap<u32, u32> as AbiType>::validate(&map).unwrap();
        // The check runs on every value that crosses the boundary, so it must not
        // charge the allocator the map was built with.
        assert_eq!(tracker.allocations.load(Ordering::SeqCst), before);
    }

    #[test]
    fn a_map_whose_numbers_were_changed_is_found_by_the_check_that_hashes() {
        let mut map: StableHashMap<u32, u32> = StableHashMap::new();
        map.insert(identity, 1, 10).unwrap();
        // The stored number still names an order, so the plain check accepts it.
        <StableHashMap<u32, u32> as AbiType>::validate(&map).unwrap();
        // The check that knows the function notices that a lookup would miss.
        assert_eq!(
            map.validate_with(doubled).unwrap_err().status(),
            xt_stable_abi::AbiStatus::InvalidDescriptor
        );
        map.validate_with(identity).unwrap();
    }

    #[test]
    fn a_set_keeps_the_last_of_a_repeated_value() {
        // `StableMap` keeps the value of the last insertion, so a set built from
        // the same values keeps the last one too.
        let values = [key("a"), key("b"), key("a")];
        let set = StableHashSet::from_values(|value: &StableString| value.len(), &values).unwrap();
        assert_eq!(set.len(), 2);

        let ordered = xt_stable_abi::StableSet::from_values(&values).unwrap();
        assert_eq!(ordered.len(), 2);
    }

    #[test]
    fn both_sets_keep_the_last_of_two_values_that_compare_equal() {
        // The two values are one as far as the order decides, and differ in what
        // they carry, so which one is kept is visible.
        let values = [
            Tagged {
                order: 1,
                label: 10,
            },
            Tagged {
                order: 1,
                label: 20,
            },
        ];

        let ordered = xt_stable_abi::StableSet::from_values(&values).unwrap();
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered.as_values()[0].label, 20);

        let hashed =
            StableHashSet::from_values(|value: &Tagged| u64::from(value.order), &values).unwrap();
        assert_eq!(hashed.len(), 1);
        assert_eq!(hashed.iter().next().unwrap().label, 20);
    }

    #[test]
    fn a_hash_set_answers_its_values_and_its_set_operations() {
        let mut left: StableHashSet<u32> = StableHashSet::new();
        for value in [1u32, 2, 3] {
            left.insert(identity, value).unwrap();
        }
        let mut right: StableHashSet<u32> = StableHashSet::new();
        for value in [3u32, 4] {
            right.insert(identity, value).unwrap();
        }

        assert_eq!(left.as_entries().len(), 3);
        let mut union: Vec<u32> = left.union_with(identity, &right).unwrap().iter().copied().collect();
        union.sort_unstable();
        assert_eq!(union, vec![1, 2, 3, 4]);

        let mut shared: Vec<u32> = left
            .intersection_with(identity, &right)
            .unwrap()
            .iter()
            .copied()
            .collect();
        shared.sort_unstable();
        assert_eq!(shared, vec![3]);

        let mut only: Vec<u32> = left
            .difference_with(identity, &right)
            .unwrap()
            .iter()
            .copied()
            .collect();
        only.sort_unstable();
        assert_eq!(only, vec![1, 2]);
    }
}

mod stable_hash_maps {
    use xt_stable_abi::{AbiStatus, StableHash, StableHashMap, StableHashSet, StableStr};

    #[test]
    fn a_map_built_with_the_crate_hash_can_be_checked_by_the_receiver() {
        let mut map: StableHashMap<u32, u32> = StableHashMap::new();
        for key in [7u32, 3, 9] {
            map.insert_stable(key, key * 2).unwrap();
        }
        assert_eq!(map.get_stable(&7), Some(&14));
        assert_eq!(map.get_stable(&4), None);
        // The structure holds, and so do the numbers, which is what a receiver
        // that holds nothing but the value can check.
        map.validate().unwrap();
        map.validate_stable().unwrap();
        assert_eq!(map.remove_stable(&3), Some(6));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn a_map_built_with_another_hash_is_valid_in_shape_and_refused_in_meaning() {
        // The container is ordered either way, so the layout check answers that
        // it holds values it may read. Whether the numbers name these keys is a
        // different question, and the answer is no.
        let mut map: StableHashMap<u32, u32> = StableHashMap::new();
        map.insert(|key| u64::from(*key) * 1_000_003, 1, 1).unwrap();
        map.insert(|key| u64::from(*key) * 1_000_003, 2, 2).unwrap();
        map.validate().unwrap();
        assert_eq!(
            map.validate_stable().unwrap_err().status(),
            AbiStatus::InvalidDescriptor
        );
        // The function it was built with is the one that answers yes.
        map.validate_with(|key| u64::from(*key) * 1_000_003).unwrap();
        // And a map with no entries makes no claim about numbers at all.
        map.clear();
        map.validate_stable().unwrap();
    }

    #[test]
    fn a_set_numbers_its_values_the_same_way_its_map_does() {
        let mut set: StableHashSet<u32> = StableHashSet::new();
        for value in [4u32, 6] {
            set.insert_stable(value).unwrap();
        }
        assert!(set.contains_stable(&4));
        assert!(!set.contains_stable(&5));
        set.validate_stable().unwrap();
        assert!(set.remove_stable(&4));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn text_is_hashed_by_its_bytes_in_this_build_and_every_other() {
        let mut map: StableHashMap<StableStr<'static>, u32> = StableHashMap::new();
        map.insert_stable(StableStr::from_str("alpha"), 1).unwrap();
        map.insert_stable(StableStr::from_str("beta"), 2).unwrap();
        assert_eq!(map.get_stable(&StableStr::from_str("beta")), Some(&2));
        map.validate_stable().unwrap();
        // The number names the bytes, not the address: two handles over the same
        // text answer the same number, and a number that followed the address
        // would make the map unreadable to another build.
        let one = StableStr::from_str("beta");
        let other = StableStr::from_str("beta");
        assert_eq!(one.stable_hash(), other.stable_hash());
        assert_ne!(one.stable_hash(), StableStr::from_str("bet").stable_hash());
    }

    #[test]
    fn the_number_is_fnv_1a_over_an_encoding_that_names_width_and_order() {
        use xt_stable_abi::stable_hash::{fold_bytes, fold_word, FNV_OFFSET};
        use xt_stable_abi::StableHash;

        // The published form of FNV-1a over the empty input is the offset basis,
        // and over one "a" it is a number fixed by that specification.
        assert_eq!(fold_bytes(FNV_OFFSET, b""), FNV_OFFSET);
        assert_eq!(fold_bytes(FNV_OFFSET, b"a"), 0xaf63_dc4c_8601_ec8c);

        // The order of the parts is part of the number, which is what makes a
        // pair of two values differ from the same two values swapped.
        assert_ne!(
            fold_word(fold_word(FNV_OFFSET, 1), 2),
            fold_word(fold_word(FNV_OFFSET, 2), 1)
        );

        // The width is part of the number, so the same value at another width is
        // another key rather than the same one read through another layout.
        assert_ne!(1u32.stable_hash(), 1u64.stable_hash());
        assert_ne!(1u8.stable_hash(), 1u16.stable_hash());
    }
}
