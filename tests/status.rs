//! Status codes as the values that cross the boundary.

use xt_stable_abi::{AbiStatus, AbiType};

#[test]
fn every_status_roundtrips_through_its_code() {
    for status in AbiStatus::ALL {
        assert_eq!(AbiStatus::from_code(status.code()), Some(status));
    }
}

#[test]
fn discriminant_order_matches_the_declaration() {
    for (index, status) in AbiStatus::ALL.iter().enumerate() {
        assert_eq!(status.code() as usize, index);
    }
}

#[test]
fn unknown_code_is_rejected() {
    assert_eq!(AbiStatus::from_code(12), None);
    assert_eq!(AbiStatus::from_code(u32::MAX), None);
}

#[test]
fn a_code_this_build_does_not_define_is_still_a_status_value() {
    // A status is a `u32` with a name attached, so a code written by a build
    // that knows more statuses is a value of this type rather than one the type
    // refuses. What it means is `from_code`'s question.
    // SAFETY: the type is `#[repr(transparent)]` over `u32`, so every `u32` is a
    // value of it.
    let unknown: AbiStatus = unsafe { core::mem::transmute(12u32) };
    assert!(unknown.validate().is_ok());
    assert_eq!(unknown.code(), 12);
    assert_eq!(AbiStatus::from_code(unknown.code()), None);
    assert!(!unknown.is_ok());
    assert_eq!(unknown.to_string(), "status this build does not define");
    // An error carries the code through unchanged, so a host at an older build
    // reports what the library said rather than a status it substituted.
    let error = xt_stable_abi::AbiError::new(unknown);
    assert_eq!(error.status().code(), 12);
}

#[test]
fn the_debug_form_names_a_status_this_build_knows() {
    assert_eq!(
        format!("{:?}", AbiStatus::OutOfMemory),
        "AbiStatus::OutOfMemory"
    );
    assert_eq!(format!("{:?}", AbiStatus::Ok), "AbiStatus::Ok");
    // Every status the type lists has a name of its own: a code added to the
    // list without one would print as a number here.
    for status in AbiStatus::ALL {
        let printed = format!("{status:?}");
        assert!(printed.starts_with("AbiStatus::"), "{printed}");
        assert!(!printed.contains('('), "{printed}");
    }
    // A code this build does not name is printed as the number it is.
    // SAFETY: the type is `#[repr(transparent)]` over `u32`.
    let unknown: AbiStatus = unsafe { core::mem::transmute(12u32) };
    assert_eq!(format!("{unknown:?}"), "AbiStatus(12)");
}

#[test]
fn every_code_the_lookup_names_is_in_the_list() {
    // The other direction: the list is walked by the test above, and this walks
    // the codes `from_code` answers for. A code added to the lookup and not to
    // the list has no name in `ALL` and no entry in the debug form.
    for code in 0..4096u32 {
        if let Some(status) = AbiStatus::from_code(code) {
            assert!(AbiStatus::ALL.contains(&status), "code {code}");
        }
    }
}

#[test]
fn ok_is_the_only_success() {
    assert!(AbiStatus::Ok.is_ok());
    for status in AbiStatus::ALL {
        if status != AbiStatus::Ok {
            assert!(status.is_err());
        }
    }
}
