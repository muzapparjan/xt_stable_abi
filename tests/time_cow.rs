//! Spans of time, and text that borrows or owns.

use std::sync::atomic::Ordering;
use std::time::Duration;

use xt_stable_abi::{AbiStatus, StableCowStr, StableDuration, StableString};

mod support;
use support::{tracked_allocator, Tracker};

#[test]
fn a_duration_roundtrips_through_its_parts() {
    for duration in [
        Duration::from_secs(0),
        Duration::from_nanos(1),
        Duration::new(3, 250_000_000),
        Duration::new(u64::MAX, 999_999_999),
    ] {
        let stable = StableDuration::from_duration(duration);
        assert!(stable.is_valid());
        assert_eq!(stable.to_duration().unwrap(), duration);
        assert_eq!(stable.secs, duration.as_secs());
        assert_eq!(stable.nanos, duration.subsec_nanos());
    }
}

#[test]
fn a_fraction_of_a_second_or_more_is_rejected() {
    assert_eq!(
        StableDuration::new(1, 1_000_000_000).unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    let invalid = StableDuration { secs: 1, nanos: u32::MAX };
    assert!(!invalid.is_valid());
    assert_eq!(
        invalid.to_duration().unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
}

#[test]
fn nanoseconds_that_do_not_fit_are_reported() {
    assert_eq!(StableDuration::new(1, 500).unwrap().as_nanos(), 1_000_000_500);
    assert_eq!(StableDuration::new(u64::MAX, 0).unwrap().as_nanos(), (u64::MAX as u128) * 1_000_000_000);
}

#[test]
fn a_span_reports_its_parts_and_its_multiples() {
    let span = StableDuration::new(3, 250_000_000).unwrap();
    assert_eq!(span.as_secs(), 3);
    assert_eq!(span.subsec_nanos(), 250_000_000);
    assert_eq!(span.as_millis(), 3250);
    assert!(!span.is_zero());
    assert!(StableDuration::new(0, 0).unwrap().is_zero());

    // Doubling keeps the fraction below one second.
    let doubled = span.checked_mul(2).unwrap();
    assert_eq!((doubled.secs, doubled.nanos), (6, 500_000_000));

    // A product whose seconds do not fit is reported rather than wrapped.
    assert_eq!(StableDuration::new(u64::MAX, 0).unwrap().checked_mul(2), None);
}

#[test]
fn borrowed_text_needs_no_allocation() {
    let tracker = Tracker::new();
    {
        let text = StableCowStr::borrowed("borrowed");
        assert!(text.is_borrowed());
        assert_eq!(text.as_str(), "borrowed");
        assert_eq!(&*text, "borrowed");
        assert_eq!(tracker.allocations.load(Ordering::SeqCst), 0);
    }
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn owned_text_crosses_with_its_allocator() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    {
        let owned = StableString::from_str_with_allocator(allocator, "owned").unwrap();
        let text = StableCowStr::owned(owned);
        assert!(!text.is_borrowed());
        assert_eq!(text.as_str(), "owned");
        assert_eq!(tracker.outstanding(), 1);
    }
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn taking_ownership_copies_only_when_borrowed() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);

    // A borrowed value is copied into the process allocator, which is the only
    // allocator a fresh owned string can be released through.
    let borrowed = StableCowStr::borrowed("copy me");
    let owned = borrowed.into_owned().unwrap();
    assert_eq!(owned.as_str(), "copy me");
    assert!(owned.allocator().context().is_null());
    assert_eq!(tracker.outstanding(), 0);
    drop(owned);
    assert_eq!(tracker.outstanding(), 0);

    let source = StableString::from_str_with_allocator(allocator, "mine").unwrap();
    let cow = StableCowStr::owned(source);
    let returned = cow.into_owned().unwrap();
    assert_eq!(returned.as_str(), "mine");
    assert_eq!(tracker.outstanding(), 1);
    drop(returned);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_cow_clone_keeps_the_variant() {
    let borrowed = StableCowStr::borrowed("text");
    assert!(borrowed.clone().is_borrowed());

    let owned = StableCowStr::owned(StableString::from_str("text").unwrap());
    let copy = owned.clone();
    assert!(!copy.is_borrowed());
    assert_eq!(copy.as_str(), "text");
}

#[test]
fn a_cow_can_hold_owned_text_inside_a_collection() {
    let mut values: xt_stable_abi::StableVec<StableCowStr<'_>> = xt_stable_abi::StableVec::new();
    values.push(StableCowStr::borrowed("a"));
    values.push(StableCowStr::owned(StableString::from_str("b").unwrap()));
    assert_eq!(
        values.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}
