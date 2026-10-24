//! Text crossing the boundary in both directions.

mod support;

use std::sync::atomic::Ordering;

use support::{gated_allocator, refusing_allocator, tracked_allocator, Gate, Tracker};
use xt_stable_abi::{AbiStatus, StableStr, StableString};

/// A module that only knows the borrowed handle, standing in for the other
/// side of the boundary.
mod consumer {
    use xt_stable_abi::StableStr;

    pub fn read(handle: StableStr<'_>) -> String {
        handle.as_str().to_owned()
    }

    pub fn byte_length(handle: StableStr<'_>) -> u64 {
        handle.len()
    }
}

#[test]
fn a_borrowed_handle_reaches_another_module() {
    let owned = StableString::from_str("Grüße, 世界").unwrap();
    let handle = owned.as_stable_str();
    assert_eq!(consumer::read(handle), "Grüße, 世界");
    assert_eq!(consumer::byte_length(handle), owned.len());
    // The handle is a copy, so reading it does not disturb the owner.
    assert_eq!(owned.as_str(), "Grüße, 世界");
}

#[test]
fn a_static_borrow_outlives_the_function_that_made_the_handle() {
    fn borrowed() -> StableStr<'static> {
        StableStr::from_str("static text")
    }

    let handle = borrowed();
    assert_eq!(consumer::read(handle), "static text");
}

#[test]
fn the_carried_allocator_releases_the_string() {
    let tracker = Tracker::new();
    let text =
        StableString::from_str_with_allocator(tracked_allocator(tracker), "payload").unwrap();
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.outstanding(), 1);

    // Moving the value is the boundary crossing: the new owner only knows the
    // allocator recorded inside the value.
    let moved = text;
    assert_eq!(moved.as_str(), "payload");
    drop(moved);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn an_empty_string_never_touches_the_allocator() {
    let tracker = Tracker::new();
    let text = StableString::from_str_with_allocator(tracked_allocator(tracker), "").unwrap();
    assert!(text.is_empty());
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 0);
    drop(text);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_failing_allocator_is_reported_for_a_non_empty_string() {
    let error = StableString::from_str_with_allocator(refusing_allocator(), "payload").unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
}

#[test]
fn cloning_uses_the_same_allocator_as_the_source() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let original = StableString::from_str_with_allocator(allocator, "shared").unwrap();
    let clone = original.try_clone().unwrap();
    assert_eq!(clone.as_str(), original.as_str());
    assert_eq!(clone.allocator().context(), allocator.context());
    assert_eq!(tracker.allocations.load(Ordering::SeqCst), 2);
    drop(original);
    drop(clone);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn text_survives_repeated_growth() {
    let tracker = Tracker::new();
    let mut text = StableString::new_with_allocator(tracked_allocator(tracker));
    for index in 0..200u32 {
        text.push_str(&index.to_string()).unwrap();
        text.push(';').unwrap();
    }
    assert!(text.len() > 200);
    assert_eq!(text.as_str().chars().filter(|value| *value == ';').count(), 200);
    drop(text);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn truncation_refuses_to_split_a_character() {
    let mut text = StableString::from_str("aé").unwrap();
    assert_eq!(text.len(), 3);
    let error = text.truncate(2).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
    assert_eq!(text.as_str(), "aé");
    text.truncate(1).unwrap();
    assert_eq!(text.as_str(), "a");
}

#[test]
fn text_roundtrips_through_the_owned_form() {
    for text in ["", "ascii", "naïve", "日本語", "emoji 🦀 and combining é"] {
        let owned = StableString::from_str(text).unwrap();
        assert_eq!(owned.as_str(), text);
        assert_eq!(owned.len(), text.len() as u64);
        let borrowed = owned.as_stable_str();
        assert_eq!(borrowed.as_str(), text);
        assert_eq!(borrowed.as_bytes(), text.as_bytes());
    }
}

#[test]
fn push_and_truncate_keep_character_boundaries() {
    let mut text = StableString::from_str("café").unwrap();
    assert_eq!(text.len(), 5);
    text.push('!').unwrap();
    assert_eq!(text.as_str(), "café!");
    // The two-byte character written at index 1 occupies bytes 3..5, so a
    // length of 4 falls inside it.
    assert!(text.truncate(4).is_err());
    assert_eq!(text.as_str(), "café!");
    text.truncate(3).unwrap();
    assert_eq!(text.as_str(), "caf");
    assert!(text.truncate(10).is_err());
}

#[test]
fn push_str_appends() {
    let mut text = StableString::new();
    text.push_str("hello").unwrap();
    text.push_str(", ").unwrap();
    text.push_str("world").unwrap();
    assert_eq!(text.as_str(), "hello, world");
    assert_eq!(text.len(), 12);
}

#[test]
fn valid_bytes_can_become_a_string() {
    let mut bytes = xt_stable_abi::StableVec::new();
    for byte in "hé".as_bytes() {
        bytes.push(*byte);
    }
    let text = StableString::from_utf8(bytes).unwrap();
    assert_eq!(text.as_str(), "hé");
    assert_eq!(text.len(), 3);
}

#[test]
fn invalid_bytes_are_rejected() {
    let mut bytes = xt_stable_abi::StableVec::new();
    bytes.push(0xFFu8);
    let error = StableString::from_utf8(bytes).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidUtf8);
}

#[test]
fn bytes_can_be_taken_back_out() {
    let text = StableString::from_str("payload").unwrap();
    let bytes = text.into_bytes();
    assert_eq!(bytes.as_slice(), b"payload");
}

#[test]
fn borrowed_handle_copies_are_independent() {
    let text = String::from("owned elsewhere");
    let borrowed = StableStr::from_str(&text);
    let copy = borrowed;
    assert_eq!(copy.as_str(), "owned elsewhere");
    assert_eq!(borrowed, copy);
}

#[test]
fn empty_values_have_no_storage() {
    let text = StableString::new();
    assert!(text.is_empty());
    assert_eq!(text.capacity(), 0);
    assert_eq!(text.as_str(), "");
    let borrowed = StableStr::empty();
    assert!(borrowed.is_empty());
    assert_eq!(borrowed.as_str(), "");
}

#[test]
fn a_string_can_be_split_repeated_and_rewritten() {
    let mut text = StableString::from_str("hello world").unwrap();
    let tail = text.split_off(5).unwrap();
    assert_eq!(text.as_str(), "hello");
    assert_eq!(tail.as_str(), " world");

    // Splitting inside a character is refused instead of producing text that is
    // not UTF-8.
    let mut accented = StableString::from_str("é").unwrap();
    assert_eq!(
        accented.split_off(1).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    assert_eq!(accented.as_str(), "é");

    assert_eq!(
        StableString::from_str("ab").unwrap().repeat(3).unwrap().as_str(),
        "ababab"
    );
    assert_eq!(
        StableString::from_str("a-b-c").unwrap().replace("-", "+").unwrap().as_str(),
        "a+b+c"
    );
    assert_eq!(
        StableString::from_str("abc").unwrap().replace("", "x").unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
}

#[test]
fn a_string_can_drop_characters_by_predicate_or_by_range() {
    let mut text = StableString::from_str("a1b2c3").unwrap();
    text.retain(|character| !character.is_ascii_digit()).unwrap();
    assert_eq!(text.as_str(), "abc");

    let mut text = StableString::from_str("hello world").unwrap();
    let removed = text.drain(5, 11).unwrap();
    assert_eq!(removed.as_str(), " world");
    assert_eq!(text.as_str(), "hello");

    // Both ends have to be character boundaries, and the value is unchanged
    // when the range is refused.
    let mut accented = StableString::from_str("é!").unwrap();
    assert_eq!(
        accented.drain(1, 2).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    assert_eq!(accented.as_str(), "é!");
    assert_eq!(
        accented.drain(0, 9).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
}

#[test]
fn a_split_reports_the_block_it_cannot_get() {
    static GATE: Gate = Gate::new();
    let mut text =
        StableString::from_str_with_allocator(gated_allocator(&GATE), "abcdef").unwrap();
    GATE.close();

    let error = text.split_off(3).unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
    // What failed is the step that moves the tail into its own block, so the
    // text is exactly what it was.
    assert_eq!(text.as_str(), "abcdef");
}

#[test]
fn repeating_an_empty_text_does_no_work() {
    let empty = StableString::new();
    // A count this large would take the loop a lifetime to walk; the answer is
    // the empty text either way, so it is answered without the loop.
    let repeated = empty.repeat(1_000_000_000).unwrap();
    assert!(repeated.is_empty());

    let text = StableString::from_str("ab").unwrap();
    assert_eq!(text.repeat(0).unwrap().as_str(), "");
    assert_eq!(text.repeat(1).unwrap().as_str(), "ab");
    assert_eq!(text.repeat(3).unwrap().as_str(), "ababab");
    // The copy doubles the block it already has, so a count that is not a power
    // of two still lands on exactly `count` copies.
    assert_eq!(text.repeat(5).unwrap().as_str(), "ababababab");
    assert_eq!(text.repeat(5).unwrap().len(), 10);

    // A count whose length the storage cannot address is still reported.
    assert_eq!(
        text.repeat(u64::MAX).unwrap_err().status(),
        AbiStatus::CapacityOverflow
    );
}
