//! Turning a panic into a status at the boundary.

use core::any::Any;

use xt_stable_abi::unwind::{describe, guarded, guarded_status, MAX_MESSAGE_LEN};
use xt_stable_abi::AbiStatus;

#[test]
fn a_boundary_function_can_report_a_panic() {
    fn boundary() -> AbiStatus {
        guarded_status(|| panic!("the worker gave up"))
    }

    assert_eq!(boundary(), AbiStatus::Panic);
}

#[test]
fn the_panic_message_reaches_the_caller() {
    let error = guarded(|| -> u64 { panic!("cannot {} this", "finish") }).unwrap_err();
    assert_eq!(error.status(), AbiStatus::Panic);
    assert_eq!(error.message(), "cannot finish this");
}

#[test]
fn a_successful_body_is_untouched() {
    assert_eq!(guarded(|| 21u64 * 2).unwrap(), 42);
    assert_eq!(guarded_status(|| {}), AbiStatus::Ok);
}

#[test]
fn a_foreign_payload_gets_a_placeholder_message() {
    let payload: Box<dyn Any + Send> = Box::new(17u8);
    assert_eq!(describe(payload.as_ref()), "panic without a message");
}

#[test]
fn a_very_long_panic_message_is_cut_on_a_character_boundary() {
    let error = guarded(|| -> u8 { panic!("{}", "é".repeat(MAX_MESSAGE_LEN)) }).unwrap_err();
    assert_eq!(error.status(), AbiStatus::Panic);
    assert!(error.message().len() <= MAX_MESSAGE_LEN);
    assert!(error.message().is_char_boundary(error.message().len()));
    // Every retained byte belongs to a complete character.
    assert!(error.message().chars().all(|value| value == 'é'));
}

#[test]
fn a_panic_from_a_nested_call_is_still_caught() {
    fn inner() -> u32 {
        panic!("deep")
    }

    let error = guarded(inner).unwrap_err();
    assert_eq!(error.status(), AbiStatus::Panic);
    assert_eq!(error.message(), "deep");
}

#[test]
fn a_static_message_is_preserved() {
    let error = guarded(|| -> u32 { panic!("static text") }).unwrap_err();
    assert_eq!(error.message(), "static text");
    assert_eq!(guarded_status(|| panic!("status text")), AbiStatus::Panic);
}
