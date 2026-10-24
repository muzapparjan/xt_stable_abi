//! Optional and fallible values, including the destructors they carry.

mod support;

use std::sync::atomic::AtomicUsize;

use support::{tracked_allocator, DropCounter, Tracker};
use xt_stable_abi::{AbiError, AbiStatus, StableOption, StableResult, StableString, StableVec};

#[test]
fn an_optional_string_carries_its_storage_across() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    {
        let value = StableOption::some(
            StableString::from_str_with_allocator(allocator, "present").unwrap(),
        );
        assert!(value.is_some());
        assert_eq!(value.as_ref().map(StableString::as_str), Some("present"));
        assert_eq!(tracker.outstanding(), 1);
    }
    assert_eq!(tracker.outstanding(), 0);

    let empty: StableOption<StableString> = StableOption::none();
    assert!(empty.is_none());
    assert_eq!(empty.into_option().map(|text| text.len()), None);
}

#[test]
fn taking_the_value_leaves_the_empty_variant() {
    let tracker = Tracker::new();
    let mut value = StableOption::some(
        StableString::from_str_with_allocator(tracked_allocator(tracker), "taken").unwrap(),
    );
    let taken = value.take().expect("the value was present");
    assert_eq!(taken.as_str(), "taken");
    assert!(value.is_none());
    assert_eq!(tracker.outstanding(), 1);
    drop(taken);
    assert_eq!(tracker.outstanding(), 0);
    drop(value);
}

#[test]
fn an_optional_drop_counter_runs_exactly_once() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    {
        let value = StableOption::some(DropCounter::new(&DROPS, 5));
        assert_eq!(DropCounter::dropped(&DROPS), 0);
        assert_eq!(value.as_ref().map(|counter| counter.payload), Some(5));
    }
    assert_eq!(DropCounter::dropped(&DROPS), 1);

    let empty: StableOption<DropCounter> = StableOption::none();
    drop(empty);
    assert_eq!(DropCounter::dropped(&DROPS), 1);
}

#[test]
fn a_result_carries_either_payload() {
    let success: StableResult<StableString, AbiError> =
        StableResult::ok(StableString::from_str("value").unwrap());
    assert!(success.is_ok());
    assert_eq!(success.ok_ref().map(StableString::as_str), Some("value"));
    assert!(success.err_ref().is_none());
    assert_eq!(success.into_result().unwrap().as_str(), "value");

    let failure: StableResult<StableString, AbiError> = StableResult::err(
        AbiError::with_message(AbiStatus::InvalidArgument, "rejected"),
    );
    assert!(failure.is_err());
    let error = failure.err_value().expect("the failure payload is present");
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
    assert_eq!(error.message(), "rejected");
}

#[test]
fn the_active_payload_is_the_only_one_dropped() {
    static SUCCESS_DROPS: AtomicUsize = AtomicUsize::new(0);
    static FAILURE_DROPS: AtomicUsize = AtomicUsize::new(0);
    {
        let success = StableResult::<DropCounter, DropCounter>::ok(DropCounter::new(
            &SUCCESS_DROPS,
            1,
        ));
        assert_eq!(DropCounter::dropped(&SUCCESS_DROPS), 0);
        drop(success);
        assert_eq!(DropCounter::dropped(&SUCCESS_DROPS), 1);
        assert_eq!(DropCounter::dropped(&FAILURE_DROPS), 0);
    }
    {
        let failure = StableResult::<DropCounter, DropCounter>::err(DropCounter::new(
            &FAILURE_DROPS,
            2,
        ));
        drop(failure);
        assert_eq!(DropCounter::dropped(&FAILURE_DROPS), 1);
        assert_eq!(DropCounter::dropped(&SUCCESS_DROPS), 1);
    }
}

#[test]
fn a_panicking_inspection_leaves_the_value_to_be_released() {
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    let value = StableResult::<DropCounter, DropCounter>::ok(DropCounter::new(&DROPS, 3));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        value.inspect(|_| panic!("the inspector gave up"));
    }));
    assert!(outcome.is_err());
    // The payload was still the value's when the closure gave up, so the unwind
    // released it exactly once.
    assert_eq!(DropCounter::dropped(&DROPS), 1);
}

#[test]
fn a_failed_allocation_is_reported() {
    let error = StableString::from_str_with_allocator(
        support::refusing_allocator(),
        "no room",
    )
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
    assert!(error.message().is_empty());
}

#[test]
fn nesting_survives_a_roundtrip() {
    let tracker = Tracker::new();
    {
        let nested: StableOption<StableResult<StableString, AbiError>> =
            StableOption::some(StableResult::ok(
                StableString::from_str_with_allocator(tracked_allocator(tracker), "deep").unwrap(),
            ));
        match nested.as_ref() {
            Some(result) => match result.as_ref() {
                Ok(text) => assert_eq!(text.as_str(), "deep"),
                Err(error) => panic!("unexpected failure: {error}"),
            },
            None => panic!("unexpected empty option"),
        }
        assert_eq!(nested.into_option().unwrap().into_result().unwrap().as_str(), "deep");
    }
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_vector_of_options_releases_every_payload() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    {
        let mut values: StableVec<StableOption<StableString>> = StableVec::new();
        for index in 0..6u32 {
            let text =
                StableString::from_str_with_allocator(allocator, &index.to_string()).unwrap();
            values.push(StableOption::some(text));
        }
        values.push(StableOption::none());
        assert_eq!(values.len(), 7);
        assert_eq!(tracker.outstanding(), 6);
        assert_eq!(
            values.get(3).and_then(|value| value.as_ref()).map(StableString::as_str),
            Some("3")
        );
    }
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn option_variants_roundtrip() {
    let none: StableOption<u32> = StableOption::none();
    assert!(none.is_none());
    assert_eq!(none.as_ref(), None);
    assert_eq!(none.into_option(), None);

    let some = StableOption::some(7u32);
    assert!(some.is_some());
    assert_eq!(some.as_ref(), Some(&7));
    assert_eq!(some.into_option(), Some(7));
}

#[test]
fn take_leaves_the_empty_variant() {
    let mut value = StableOption::some(String::from("text"));
    assert_eq!(value.take().as_deref(), Some("text"));
    assert!(value.is_none());
    assert_eq!(value.take(), None);
}

#[test]
fn as_mut_reaches_the_value() {
    let mut value = StableOption::some(1u8);
    if let Some(inner) = value.as_mut() {
        *inner = 9;
    }
    assert_eq!(value.as_ref(), Some(&9));
}

#[test]
fn option_tags_are_the_documented_numbers() {
    let none_tag = xt_stable_abi::option::NONE_TAG;
    let some_tag = xt_stable_abi::option::SOME_TAG;
    assert_eq!(StableOption::<u8>::none().tag(), none_tag);
    assert_eq!(StableOption::some(1u8).tag(), some_tag);
    assert!(StableOption::some(1u8).validate().is_ok());
}

#[test]
fn option_foreign_tags_are_rejected() {
    // SAFETY: the payload holds an initialised `u8`; the tag is what is wrong.
    let error = unsafe {
        StableOption::<u8>::from_parts(9, core::mem::MaybeUninit::new(1))
    }
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidTag);
}

#[test]
fn unwrap_or_uses_the_fallback_only_when_empty() {
    assert_eq!(StableOption::some(3u32).unwrap_or(4), 3);
    assert_eq!(StableOption::<u32>::none().unwrap_or(4), 4);
}

#[test]
fn map_touches_only_the_present_value() {
    let mapped = StableOption::some(3u32).map(|value| value.to_string());
    assert_eq!(mapped.as_ref().map(String::as_str), Some("3"));
    let empty = StableOption::<u32>::none().map(|value| value.to_string());
    assert!(empty.is_none());
}

#[test]
fn result_variants_roundtrip() {
    let ok: StableResult<u32, &str> = StableResult::ok(7);
    assert!(ok.is_ok());
    assert_eq!(ok.as_ref(), Ok(&7));
    assert_eq!(ok.into_result(), Ok(7));

    let err: StableResult<u32, &str> = StableResult::err("bad");
    assert!(err.is_err());
    assert_eq!(err.as_ref(), Err(&"bad"));
    assert_eq!(err.into_result(), Err("bad"));
}

#[test]
fn result_tags_are_the_documented_numbers() {
    let ok_tag = xt_stable_abi::result::OK_TAG;
    let err_tag = xt_stable_abi::result::ERR_TAG;
    assert_eq!(StableResult::<u8, u8>::ok(1).tag(), ok_tag);
    assert_eq!(StableResult::<u8, u8>::err(1).tag(), err_tag);
    assert!(StableResult::<u8, u8>::ok(1).validate().is_ok());
    assert!(StableResult::<u8, u8>::err(1).validate().is_ok());
}

#[test]
fn result_foreign_tags_are_rejected() {
    let payload = xt_stable_abi::result::StableResultValue::<u8, u8> {
        ok: core::mem::ManuallyDrop::new(1),
    };
    // SAFETY: the union field that is read back is the one that was written;
    // the point of the test is that the tag says otherwise.
    let error = unsafe { StableResult::<u8, u8>::from_parts(5, payload) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidTag);
}

#[test]
fn payload_accessors_select_by_tag() {
    let ok = StableResult::<u8, u16>::ok(3);
    assert_eq!(ok.ok_ref(), Some(&3));
    assert_eq!(ok.err_ref(), None);
    let err = StableResult::<u8, u16>::err(9);
    assert_eq!(err.ok_ref(), None);
    assert_eq!(err.err_ref(), Some(&9));
}

#[test]
fn consuming_accessors_move_the_payload() {
    let ok = StableResult::<String, String>::ok(String::from("left"));
    assert_eq!(ok.ok_value().as_deref(), Some("left"));
    let err = StableResult::<String, String>::err(String::from("right"));
    assert_eq!(err.err_value().as_deref(), Some("right"));
}

#[test]
fn map_and_map_err_touch_one_variant_each() {
    let success = StableResult::<u32, &str>::ok(2).map(|value| value * 3);
    assert_eq!(success.into_result(), Ok(6));

    let failure = StableResult::<u32, &str>::err("bad").map(|value| value * 3);
    assert_eq!(failure.into_result(), Err("bad"));

    let renamed = StableResult::<u32, &str>::err("bad").map_err(|error| error.len());
    assert_eq!(renamed.into_result(), Err(3));

    let untouched = StableResult::<u32, &str>::ok(2).map_err(|error| error.len());
    assert_eq!(untouched.into_result(), Ok(2));
}
