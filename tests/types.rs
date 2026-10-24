//! The types that cover ownership, borrowing, names and points in time.

mod support;

use std::path::Path;
use std::time::{Duration, Instant, UNIX_EPOCH};

use support::{gated_allocator, Gate};
use xt_stable_abi::{
    AbiStatus, StableBox, StableCowSlice, StableHashMap, StableInstant, StableMap, StableOsStr,
    StableOsString, StablePath, StablePathBuf, StableSet, StableSystemTime, StableVec,
};

#[test]
fn a_box_hands_the_value_back_and_releases_the_block() {
    let boxed = StableBox::new(StableVec::from_slice(&[1u32, 2, 3]).unwrap());
    assert_eq!(boxed.as_slice(), &[1, 2, 3]);
    let vector = boxed.into_inner().unwrap();
    assert_eq!(vector.as_slice(), &[1, 2, 3]);
}

#[test]
fn an_empty_box_reports_that_it_names_no_value() {
    let empty: StableBox<u32> = StableBox::empty();
    assert!(empty.is_empty());
    assert_eq!(empty.validate().unwrap_err().status(), AbiStatus::NullPointer);

    // A handle that names no value has nothing to hand back, and says so rather
    // than reading through the null address.
    let error = StableBox::<u32>::empty().into_inner().unwrap_err();
    assert_eq!(error.status(), AbiStatus::NullPointer);

    // Two handles that name nothing are equal, and a copy of one names nothing.
    let copy: StableBox<u32> = StableBox::empty();
    assert_eq!(empty, copy);
    assert!(empty.clone().is_empty());
    assert_ne!(StableBox::new(1u32), StableBox::empty());
}

#[test]
#[should_panic(expected = "the boxed handle names no value")]
fn reading_an_empty_box_panics_instead_of_dereferencing_null() {
    let empty: StableBox<u32> = StableBox::empty();
    let _ = *empty;
}

#[test]
fn a_box_checks_the_block_it_adopts() {    let mut value = 7u64;
    // SAFETY: the pointer names one initialised `u64`. The value is on the
    // stack, so the handle is forgotten rather than released.
    let adopted = unsafe {
        StableBox::from_raw_parts_checked(&mut value, 8, xt_stable_abi::SYSTEM_ALLOCATOR)
    };
    let adopted = adopted.unwrap();
    assert_eq!(*adopted, 7);
    core::mem::forget(adopted);

    // SAFETY: the pointer is valid; the advertised size is what is checked.
    let rejected = unsafe {
        StableBox::from_raw_parts_checked(&mut value, 4, xt_stable_abi::SYSTEM_ALLOCATOR)
    };
    assert_eq!(
        rejected.unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
    assert_eq!(value, 7);
}

#[test]
fn a_sequence_borrows_until_it_must_own() {
    let values = [4u32, 5, 6];
    let borrowed = StableCowSlice::borrowed(&values);
    assert!(borrowed.is_borrowed());
    assert_eq!(borrowed.as_slice(), &[4, 5, 6]);

    let owned = StableCowSlice::owned(StableVec::from_slice(&[7u32, 8]).unwrap());
    assert!(!owned.is_borrowed());
    assert_eq!(owned.into_owned().unwrap().as_slice(), &[7, 8]);

    let copied = borrowed.into_owned().unwrap();
    assert_eq!(copied.as_slice(), &[4, 5, 6]);
}

#[test]
fn a_name_keeps_the_encoding_it_was_written_with() {
    let text = StableOsString::from_text("report.txt").unwrap();
    assert_eq!(text.encoding(), xt_stable_abi::path::ENCODING_UTF8);
    assert_eq!(text.to_str(), Some("report.txt"));
    assert_eq!(text.to_os_string().unwrap(), "report.txt");
    let joined = text.on(Path::new("dir"));
    assert_eq!(joined.to_path_buf().unwrap(), Path::new("dir/report.txt"));

    let borrowed = StableOsStr::from_bytes(b"raw-\xff", xt_stable_abi::path::ENCODING_UNIX).unwrap();
    assert_eq!(borrowed.encoding(), xt_stable_abi::path::ENCODING_UNIX);
    assert_eq!(borrowed.as_bytes(), b"raw-\xff");
    // A Unix byte sequence is not a Windows name, so rebuilding it there is
    // refused rather than guessed at.
    let rebuilt = borrowed.to_os_str();
    assert_eq!(rebuilt.is_ok(), cfg!(unix));
}

#[test]
fn a_name_an_encoding_does_not_allow_is_rejected() {
    let status = StableOsString::from_parts_with_allocator(
        xt_stable_abi::SYSTEM_ALLOCATOR,
        b"not utf8: \xff",
        xt_stable_abi::path::ENCODING_UTF8,
    )
    .unwrap_err()
    .status();
    assert_eq!(status, AbiStatus::InvalidUtf8);

    let status = StableOsStr::from_bytes(b"x", 99).unwrap_err().status();
    assert_eq!(status, AbiStatus::InvalidArgument);

    // A WTF-8 sequence that is not well formed is refused.
    let status = StableOsStr::from_bytes(&[0xed, 0xa0], xt_stable_abi::path::ENCODING_WTF8)
        .unwrap_err()
        .status();
    assert_eq!(status, AbiStatus::InvalidUtf8);
}

#[test]
fn ascii_names_are_well_formed_and_survive_the_check() {
    // A name is mostly ASCII, and an ASCII byte is one whole sequence with no
    // continuation byte to look for. Reading one anyway either indexes past the
    // end (a one-byte name) or rejects the next letter (any longer name).
    for name in ["x", "abc", "a/b/c.txt", "with space"] {
        let borrowed =
            StableOsStr::from_bytes(name.as_bytes(), xt_stable_abi::path::ENCODING_WTF8)
                .expect("an ASCII name is well-formed WTF-8");
        assert!(borrowed.validate().is_ok(), "{name:?} was refused");
        let owned = StableOsString::from_parts_with_allocator(
            xt_stable_abi::SYSTEM_ALLOCATOR,
            name.as_bytes(),
            xt_stable_abi::path::ENCODING_WTF8,
        )
        .expect("an ASCII name");
        assert_eq!(owned.to_str(), Some(name));
        assert!(owned.as_stable_os_str().validate().is_ok());
    }
    // An empty name is the degenerate case of the same rule.
    assert!(
        StableOsStr::from_bytes(&[], xt_stable_abi::path::ENCODING_WTF8)
            .expect("empty")
            .validate()
            .is_ok()
    );
}

#[test]
fn a_name_mixing_ascii_and_an_unpaired_surrogate_is_well_formed() {
    // UTF-8 refuses an unpaired surrogate; WTF-8 is the encoding that exists to
    // carry one, and a Windows name is usually ASCII text with such a code unit
    // in the middle.
    let bytes = [b'C', b':', b'\\', b'x', 0xed, 0xa0, 0x80];
    let borrowed = StableOsStr::from_bytes(&bytes, xt_stable_abi::path::ENCODING_WTF8)
        .expect("a mixed WTF-8 name");
    assert!(borrowed.validate().is_ok());

    // The three shapes the rule has to separate: ASCII alone, a surrogate
    // alone, and a truncated sequence.
    assert!(StableOsStr::from_bytes(b"a", xt_stable_abi::path::ENCODING_WTF8)
        .expect("one ASCII byte")
        .validate()
        .is_ok());
    assert!(StableOsStr::from_bytes(&[0xed, 0xa0, 0x80], xt_stable_abi::path::ENCODING_WTF8)
        .expect("a surrogate alone")
        .validate()
        .is_ok());
    assert!(StableOsStr::from_bytes(&[b'a', 0xed], xt_stable_abi::path::ENCODING_WTF8).is_err());
    // A continuation byte where a lead byte belongs is still refused.
    assert!(StableOsStr::from_bytes(&[0x80], xt_stable_abi::path::ENCODING_WTF8).is_err());
}

#[cfg(windows)]
#[test]
fn a_windows_name_with_an_unpaired_surrogate_survives_the_round_trip() {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    // The code unit D800 is not a scalar value, which is exactly why the native
    // type can hold it and a `str` cannot.
    let native = OsString::from_wide(&[0x0043, 0x003A, 0x005C, 0xD800]);
    let owned = StableOsString::from_os_str(&native).expect("a name this platform stores");
    assert!(owned.as_stable_os_str().validate().is_ok());

    let rebuilt = owned.to_os_string().expect("the same platform rebuilds it");
    assert_eq!(rebuilt.as_os_str().encode_wide().collect::<Vec<_>>(), native.encode_wide().collect::<Vec<_>>());
}

#[test]
fn a_path_roundtrips_through_its_borrowed_form() {
    let owned = StablePathBuf::from_path(Path::new("a/b/c.txt")).unwrap();
    let borrowed: StablePath<'_> = owned.as_stable_path();
    assert_eq!(borrowed.to_str(), Some("a/b/c.txt"));
    assert_eq!(borrowed.as_path().unwrap(), Path::new("a/b/c.txt"));
    assert_eq!(owned.to_path_buf().unwrap(), Path::new("a/b/c.txt"));

    let text = StablePath::from_str("x.txt");
    assert_eq!(text.to_path_buf().unwrap().to_path_buf().unwrap(), Path::new("x.txt"));
}

#[test]
fn a_path_and_a_name_convert_from_their_native_forms() {
    let converted = StablePathBuf::try_from(Path::new("a/b")).unwrap();
    assert_eq!(converted.to_path_buf().unwrap(), Path::new("a/b"));

    let name = StableOsString::try_from(std::ffi::OsStr::new("name.txt")).unwrap();
    assert_eq!(name.to_str(), Some("name.txt"));
    // A copy with a named allocator holds what the default one holds.
    let copied =
        StablePathBuf::from_path_with_allocator(xt_stable_abi::AbiAllocator::system(), Path::new("c/d"))
            .unwrap();
    assert_eq!(copied.to_path_buf().unwrap(), Path::new("c/d"));
}

#[test]
fn a_path_can_be_a_key_the_receiver_checks_on_its_own() {
    let mut map: StableHashMap<StablePathBuf, u32> = StableHashMap::new();
    map.insert_stable(StablePathBuf::try_from(Path::new("a/b")).unwrap(), 1)
        .unwrap();
    map.insert_stable(StablePathBuf::try_from(Path::new("a/c")).unwrap(), 2)
        .unwrap();
    let looked_up = StablePathBuf::try_from(Path::new("a/c")).unwrap();
    assert_eq!(map.get_stable(&looked_up), Some(&2));
    // The numbers came from this crate's hash, so the receiver computes them
    // itself rather than needing the function the sender used.
    map.validate_stable().unwrap();

    let mut names: StableHashMap<StableOsString, u32> = StableHashMap::new();
    names
        .insert_stable(StableOsString::try_from(std::ffi::OsStr::new("one")).unwrap(), 1)
        .unwrap();
    names.validate_stable().unwrap();
}

#[test]
fn text_and_groups_answer_one_number_in_both_forms() {
    use xt_stable_abi::{StableArray, StableCowStr, StableHash};

    // A borrowed value and an owned one that say the same thing are one key.
    let borrowed = StableCowStr::borrowed("same");
    let owned = StableCowStr::owned(xt_stable_abi::StableString::from_str("same").unwrap());
    assert_eq!(borrowed.stable_hash(), owned.stable_hash());

    let group = StableArray::new([1u32, 2, 3]);
    assert_eq!(group.stable_hash(), StableArray::new([1u32, 2, 3]).stable_hash());
    // The length is part of the number, so a group and one of another length do
    // not answer the same one.
    assert_ne!(
        StableArray::new([1u32, 2, 3]).stable_hash(),
        StableArray::new([1u32, 2, 3, 4]).stable_hash()
    );
}

#[test]
fn removing_a_part_answers_whether_it_could() {
    // An empty path has no part to remove, which is the answer `false` gives.
    let mut empty = StablePathBuf::new();
    assert!(!empty.pop().unwrap());

    let gate = {
        static GATE: Gate = Gate::new();
        &GATE
    };
    let mut path = StablePathBuf::from_path_with_allocator(
        gated_allocator(gate),
        Path::new("/one/two"),
    )
    .unwrap();
    // Rebuilding the shorter path asks for a block. When that step fails the
    // answer is an error, because `false` would say the path had nothing to
    // remove.
    gate.close();
    assert_eq!(
        path.pop().unwrap_err().status(),
        AbiStatus::OutOfMemory
    );
    assert_eq!(
        path.to_str(),
        Some("/one/two"),
        "a removal that failed leaves the path as it was"
    );
}

#[test]
fn choices_and_readings_answer_one_number_for_equal_values() {
    use xt_stable_abi::{StableHash, StableOption, StableResult};

    let some = StableOption::some(7u32);
    assert_eq!(some.stable_hash(), StableOption::some(7u32).stable_hash());
    assert_ne!(some.stable_hash(), StableOption::<u32>::none().stable_hash());
    // The two choices are different types, and their tags are kept apart as
    // well, so a value of one never answers the number of the other.
    assert_ne!(
        some.stable_hash(),
        StableResult::<u32, u32>::ok(7).stable_hash()
    );

    let reading = StableSystemTime::new(5, 7).unwrap();
    assert_eq!(
        reading.stable_hash(),
        StableSystemTime::new(5, 7).unwrap().stable_hash()
    );
    assert_ne!(
        reading.stable_hash(),
        StableSystemTime::new(5, 8).unwrap().stable_hash()
    );
    // A reading of a monotonic clock means nothing without its base, so the
    // base is part of the number.
    let instant = StableInstant::new(1, 100);
    assert_ne!(
        instant.stable_hash(),
        StableInstant::new(2, 100).stable_hash()
    );
}

#[test]
fn a_group_is_hashed_by_the_rule_the_module_documents() {
    use xt_stable_abi::stable_hash::{fold_word, FNV_OFFSET};
    use xt_stable_abi::{StableHash, StablePair, StableQuad, StableTriple};

    // The rule is: start from the offset and fold each part's own number, in
    // order. A group written by a party that follows the documentation has to
    // answer the number this build computes.
    let expected = fold_word(fold_word(FNV_OFFSET, 1u64.stable_hash()), 2u64.stable_hash());
    assert_eq!(StablePair::new(1u64, 2u64).stable_hash(), expected);

    // A pair and a one-tuple holding that pair are the same numbers in the same
    // order, so the two agree with each other.
    assert_eq!(
        (StablePair::new(1u64, 2u64),).stable_hash(),
        fold_word(FNV_OFFSET, expected)
    );

    let expected = fold_word(
        fold_word(fold_word(FNV_OFFSET, 1u64.stable_hash()), 2u64.stable_hash()),
        3u64.stable_hash(),
    );
    assert_eq!(StableTriple::new(1u64, 2u64, 3u64).stable_hash(), expected);

    let expected = fold_word(
        fold_word(
            fold_word(fold_word(FNV_OFFSET, 1u64.stable_hash()), 2u64.stable_hash()),
            3u64.stable_hash(),
        ),
        4u64.stable_hash(),
    );
    assert_eq!(
        StableQuad::new(1u64, 2u64, 3u64, 4u64).stable_hash(),
        expected
    );
}

#[test]
fn a_calendar_point_roundtrips_including_before_the_epoch() {
    let now = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
    let stored = StableSystemTime::from_system_time(now).unwrap();
    assert_eq!(stored.to_system_time().unwrap(), now);

    let before = UNIX_EPOCH - Duration::from_millis(1_500);
    let stored = StableSystemTime::from_system_time(before).unwrap();
    assert_eq!(stored.to_system_time().unwrap(), before);
}

#[test]
fn a_fraction_that_is_not_a_fraction_is_rejected() {
    assert_eq!(
        StableSystemTime::new(0, 1_000_000_000).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    let malformed = StableSystemTime {
        seconds: 0,
        nanos: 2_000_000_000,
    };
    assert_eq!(
        malformed.to_system_time().unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
}

#[test]
fn a_monotonic_reading_counts_from_the_base_the_two_sides_share() {
    let base = Instant::now();
    let later = base + Duration::from_millis(20);
    let stored = StableInstant::from_instant(7, base, later);
    assert_eq!(stored.origin, 7);
    assert_eq!(stored.as_duration(), Duration::from_millis(20));

    let rebuilt = stored.to_instant(base).unwrap();
    assert_eq!(rebuilt.duration_since(base), Duration::from_millis(20));
}

#[test]
fn a_map_can_be_taken_apart_and_adopted_again() {
    let map = StableMap::from_pairs(&[(1u32, 10u32), (2, 20)]).unwrap();
    // SAFETY: the parts describe the block the map owns, and they are handed
    // straight back to the constructor below.
    let (pointer, length, capacity, allocator) = unsafe { map.into_raw_parts() };
    // SAFETY: the parts came from a map that was just taken apart.
    let adopted = unsafe { StableMap::from_raw_parts(pointer, length, capacity, allocator) };
    let adopted = adopted.unwrap();
    assert_eq!(adopted.len(), 2);
    assert_eq!(adopted.get(&2), Some(&20));
}

#[test]
fn a_map_built_from_pairs_keeps_the_value_written_last() {
    // Inserting the pairs one after another leaves the later value, so building
    // the map from the same pairs has to produce the same map.
    let mut inserted = StableMap::new();
    for (key, value) in [(7u32, 100u32), (7, 200), (7, 300)] {
        inserted.insert(key, value).unwrap();
    }
    let built = StableMap::from_pairs(&[(7u32, 100u32), (7, 200), (7, 300)]).unwrap();
    assert_eq!(built.get(&7), Some(&300));
    assert_eq!(built.as_entries(), inserted.as_entries());

    // A repeated key among others keeps the map ordered by key.
    let mixed = StableMap::from_pairs(&[(2u32, 1u32), (1, 5), (2, 9), (1, 7)]).unwrap();
    assert_eq!(mixed.as_entries().len(), 2);
    assert_eq!(mixed.get(&1), Some(&7));
    assert_eq!(mixed.get(&2), Some(&9));
}

#[test]
fn a_set_can_be_taken_apart_and_adopted_again() {
    let set = StableSet::from_values(&[3u32, 1, 3, 2]).unwrap();
    assert_eq!(set.as_values(), &[1, 2, 3]);
    // SAFETY: the parts describe the block the set owns, and they are handed
    // straight back to the constructor below.
    let (pointer, length, capacity, allocator) = unsafe { set.into_raw_parts() };
    // SAFETY: the parts came from a set that was just taken apart.
    let adopted = unsafe { StableSet::from_raw_parts(pointer, length, capacity, allocator) };
    let adopted = adopted.unwrap();
    assert_eq!(adopted.as_values(), &[1, 2, 3]);
}

#[test]
fn set_operations_answer_the_values_a_set_theory_says() {
    let left = StableSet::from_values(&[1u32, 2, 3, 5]).unwrap();
    let right = StableSet::from_values(&[2u32, 3, 4]).unwrap();

    assert_eq!(left.union(&right).unwrap().as_values(), &[1, 2, 3, 4, 5]);
    assert_eq!(left.intersection(&right).unwrap().as_values(), &[2, 3]);
    assert_eq!(left.difference(&right).unwrap().as_values(), &[1, 5]);
    assert_eq!(right.difference(&left).unwrap().as_values(), &[4]);

    // A value only one side holds is not in the intersection, and the empty set
    // behaves like the identity for union and the absorbing value for
    // intersection.
    let empty = StableSet::<u32>::new();
    assert_eq!(left.union(&empty).unwrap().as_values(), &[1, 2, 3, 5]);
    assert!(empty.intersection(&left).unwrap().is_empty());
    assert_eq!(left.difference(&empty).unwrap().as_values(), &[1, 2, 3, 5]);
}

#[test]
fn a_map_entry_can_be_modified_or_filled_with_a_default() {
    use xt_stable_abi::StableMap;

    let mut counts: StableMap<u32, u32> = StableMap::new();
    counts.entry(1).and_modify(|| 0, |count| *count += 1).unwrap();
    counts.entry(1).and_modify(|| 0, |count| *count += 1).unwrap();
    counts.entry(2).or_default().unwrap();
    assert_eq!(counts.get(&1), Some(&2));
    assert_eq!(counts.get(&2), Some(&0));
}

#[test]
fn a_system_time_can_travel_through_a_vector() {
    let mut times = StableVec::new();
    times.push(StableSystemTime::new(5, 0).unwrap());
    assert_eq!(times.as_slice()[0].seconds, 5);
}

trait On {
    fn on(&self, directory: &Path) -> StablePathBuf;
}

impl On for StableOsString {
    fn on(&self, directory: &Path) -> StablePathBuf {
        let mut path = directory.to_path_buf();
        path.push(self.to_os_string().expect("a name this platform uses"));
        StablePathBuf::from_path(&path).expect("a path this platform stores")
    }
}
