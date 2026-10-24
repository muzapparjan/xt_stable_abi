//! Handles as the name two sides exchange for an object one of them owns.

mod support;

use std::sync::atomic::AtomicUsize;

use support::DropCounter;
use xt_stable_abi::{AbiStatus, Handle, HandleTable};

#[test]
fn the_value_a_panicked_accessor_answers_with_names_no_object() {
    use xt_stable_abi::PanicFallback;

    // Nothing can be said about an object whose call did not answer, so the
    // fallback has to be the handle that names none.
    let fallback = <Handle as PanicFallback>::panic_value();
    assert!(fallback.is_none());
    assert_eq!(fallback, Handle::NONE);
}

#[test]
fn a_handle_travels_as_a_single_word() {
    let mut table = HandleTable::new();
    let handle = table.insert(String::from("owned here"));

    // The other side only ever sees the word.
    let raw = handle.to_raw();
    let returned = Handle::from_raw(raw);

    assert_eq!(returned, handle);
    assert_eq!(table.get(returned).map(String::as_str), Some("owned here"));
}

#[test]
fn a_removed_name_never_resolves_again() {
    let mut table = HandleTable::new();
    let retired = table.insert(1u32);
    assert_eq!(table.remove(retired), Some(1));

    let replacement = table.insert(2u32);
    assert_eq!(retired.index(), replacement.index());
    assert!(!table.contains(retired));
    assert_eq!(table.get(replacement), Some(&2));
}

#[test]
fn the_table_owns_the_values_it_names() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    {
        let mut table = HandleTable::new();
        let first = table.insert(DropCounter::new(&DROPS, 1));
        let second = table.insert(DropCounter::new(&DROPS, 2));
        assert_eq!(DropCounter::dropped(&DROPS), 0);

        let removed = table.remove(first).expect("the entry is live");
        assert_eq!(removed.payload, 1);
        assert_eq!(DropCounter::dropped(&DROPS), 0);
        drop(removed);
        assert_eq!(DropCounter::dropped(&DROPS), 1);

        assert!(table.contains(second));
    }
    assert_eq!(DropCounter::dropped(&DROPS), 2);
}

#[test]
fn clearing_retires_every_name() {
    let mut table = HandleTable::new();
    let first = table.insert(1u8);
    let second = table.insert(2u8);
    table.clear();
    assert!(table.is_empty());
    assert!(!table.contains(first));
    assert!(!table.contains(second));

    let reused = table.insert(3u8);
    assert_eq!(table.get(reused), Some(&3));
    assert!(!table.contains(first));
}

#[test]
fn a_missing_name_is_reported_rather_than_guessed() {
    let table: HandleTable<u8> = HandleTable::new();
    assert!(!table.contains(Handle::NONE));
    assert_eq!(table.get(Handle::NONE), None);

    // A word that was never handed out by this table.
    let invented = Handle::from_parts(7, 1);
    assert_eq!(table.get(invented), None);
}

#[test]
fn a_missing_name_is_reported_as_an_invalid_handle() {
    let mut table = HandleTable::new();
    let handle = table.insert(1u8);
    assert_eq!(table.require(handle).copied(), Ok(1));
    table.remove(handle);

    let error = table.require(handle).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidHandle);
    assert_eq!(error.message(), "the handle does not name a live entry");

    let error = table.require_mut(handle).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidHandle);
}

#[test]
fn iteration_reports_only_live_names() {
    let mut table = HandleTable::new();
    let removed = table.insert(1u8);
    let kept = table.insert(2u8);
    table.remove(removed);

    let entries: Vec<(Handle, u8)> = table.iter().map(|(name, value)| (name, *value)).collect();
    assert_eq!(entries, vec![(kept, 2)]);
    assert_eq!(table.values().copied().collect::<Vec<_>>(), vec![2]);
    assert_eq!(table.handles().collect::<Vec<_>>(), vec![kept]);
}

#[test]
fn insert_and_get_roundtrip() {
    let mut table = HandleTable::new();
    let first = table.insert(String::from("first"));
    let second = table.insert(String::from("second"));
    assert_eq!(table.get(first).map(String::as_str), Some("first"));
    assert_eq!(table.get(second).map(String::as_str), Some("second"));
    assert_eq!(table.len(), 2);
}

#[test]
fn removal_invalidates_the_handle() {
    let mut table = HandleTable::new();
    let handle = table.insert(1u32);
    assert_eq!(table.remove(handle), Some(1));
    assert!(!table.contains(handle));
    assert_eq!(table.remove(handle), None);
    assert!(table.is_empty());
}

#[test]
fn a_reused_slot_rejects_the_old_handle() {
    let mut table = HandleTable::new();
    let old = table.insert(1u32);
    table.remove(old);
    let new = table.insert(2u32);
    assert_eq!(old.index(), new.index());
    assert_ne!(old.generation(), new.generation());
    assert!(!table.contains(old));
    assert_eq!(table.get(new), Some(&2));
}

#[test]
fn generations_survive_many_cycles() {
    let mut table = HandleTable::new();
    let mut seen = Vec::new();
    for value in 0..1_000u32 {
        let handle = table.insert(value);
        seen.push(handle);
        assert_eq!(table.get(handle), Some(&value));
        table.remove(handle);
    }
    for handle in seen {
        assert!(!table.contains(handle));
    }
}

#[test]
fn none_never_resolves() {
    let mut table = HandleTable::new();
    table.insert(1u8);
    assert!(!table.contains(Handle::NONE));
    assert_eq!(table.get(Handle::NONE), None);
    assert_eq!(table.remove(Handle::NONE), None);
}

#[test]
fn clear_advances_every_generation() {
    let mut table = HandleTable::new();
    let first = table.insert(1u8);
    let second = table.insert(2u8);
    table.clear();
    assert!(table.is_empty());
    assert!(!table.contains(first));
    assert!(!table.contains(second));
    let reused = table.insert(3u8);
    assert_eq!(table.get(reused), Some(&3));
    assert!(!table.contains(first));
}

#[test]
fn iteration_sees_only_live_entries() {
    let mut table = HandleTable::new();
    let removed = table.insert(1u8);
    let kept = table.insert(2u8);
    table.remove(removed);
    let entries: Vec<_> = table.iter().map(|(handle, value)| (handle, *value)).collect();
    assert_eq!(entries, vec![(kept, 2)]);
    assert_eq!(table.values().copied().collect::<Vec<_>>(), vec![2]);
    assert_eq!(table.handles().collect::<Vec<_>>(), vec![kept]);
}

#[test]
fn raw_words_roundtrip() {
    let handle = Handle::from_parts(7, 3);
    assert_eq!(Handle::from_raw(handle.to_raw()), handle);
    assert_eq!(handle.index(), Some(7));
    assert_eq!(handle.generation(), 3);
    assert!(Handle::NONE.is_none());
    assert_eq!(Handle::NONE.index(), None);
}

#[test]
fn wiping_the_table_wipes_the_values_it_still_holds() {
    use std::sync::atomic::Ordering;

    use xt_stable_abi::Wipe;

    static WIPES: AtomicUsize = AtomicUsize::new(0);

    /// A value that records that it was wiped.
    #[repr(C)]
    struct Counted(u32);

    xt_stable_abi::unsafe_stable_element!(Counted);

    impl Wipe for Counted {
        fn wipe_in_place(&mut self) {
            WIPES.fetch_add(1, Ordering::SeqCst);
            self.0 = 0;
        }
    }

    let mut table = HandleTable::new();
    let removed = table.insert(Counted(1));
    let kept = table.insert(Counted(2));
    table.remove(removed);
    assert_eq!(WIPES.load(Ordering::SeqCst), 0);

    table.wipe_in_place();

    // The entry that was removed had already been released, so only the one the
    // table still held is wiped.
    assert_eq!(WIPES.load(Ordering::SeqCst), 1);
    assert!(table.is_empty());
    assert!(!table.contains(kept));
}

#[test]
fn a_name_taken_before_a_wipe_names_nothing_after_it() {
    use xt_stable_abi::Wipe;

    let mut table = HandleTable::new();
    let before = table.insert(1u32);
    let second = table.insert(2u32);
    table.wipe_in_place();
    assert!(table.is_empty());

    // The table starts again with slots of its own, and the names it hands out
    // afterwards are not the ones it retired.
    let after = table.insert(3u32);
    assert_eq!(table.get(after), Some(&3));
    assert_eq!(table.get(before), None);
    assert_eq!(table.get(second), None);
    assert_ne!(before, after);
    assert!(!table.contains(before));
}

#[test]
fn two_wipes_in_a_row_keep_retiring_names() {
    use xt_stable_abi::Wipe;

    let mut table = HandleTable::new();
    let first = table.insert(1u32);
    table.wipe_in_place();
    let second = table.insert(2u32);
    table.wipe_in_place();
    let third = table.insert(3u32);

    assert_eq!(table.get(third), Some(&3));
    assert_eq!(table.get(first), None);
    assert_eq!(table.get(second), None);
    assert_eq!(table.len(), 1);
}

#[test]
fn a_wipe_retires_a_generation_the_block_climbed_to() {
    use xt_stable_abi::Wipe;

    // The slot's generation climbs before the wipe, so a table that started its
    // next block at the first generation would hand out a word an old handle
    // already carries.
    let mut table = HandleTable::new();
    table.insert(0u32);
    for round in 1..=8u32 {
        let handle = table
            .handles()
            .next()
            .expect("the table holds the value it was given");
        table.remove(handle);
        table.insert(round);
    }
    let before = table
        .handles()
        .next()
        .expect("the table holds the value it was given");
    assert!(before.generation() > 1);

    table.wipe_in_place();
    let after = table.insert(99u32);
    assert_eq!(table.get(after), Some(&99));
    assert_eq!(table.get(before), None);
    assert_ne!(before, after);
}
