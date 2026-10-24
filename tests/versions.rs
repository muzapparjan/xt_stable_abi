//! Version negotiation between the two sides.

use xt_stable_abi::AbiVersion;

#[test]
fn same_major_minor_at_least_is_served() {
    assert!(AbiVersion::new(1, 3).serves(AbiVersion::new(1, 2)));
    assert!(AbiVersion::new(1, 2).serves(AbiVersion::new(1, 2)));
    assert!(!AbiVersion::new(1, 1).serves(AbiVersion::new(1, 2)));
}

#[test]
fn different_major_is_never_served() {
    assert!(!AbiVersion::new(2, 9).serves(AbiVersion::new(1, 0)));
    assert!(!AbiVersion::new(0, 9).serves(AbiVersion::new(1, 0)));
}

#[test]
fn raw_roundtrip() {
    for major in [0u32, 1, 7, u32::MAX] {
        for minor in [0u32, 2, u32::MAX] {
            let version = AbiVersion::new(major, minor);
            assert_eq!(AbiVersion::from_raw(version.to_raw()), version);
        }
    }
}

#[test]
fn display_uses_both_components() {
    assert_eq!(AbiVersion::new(3, 14).to_string(), "3.14");
}
