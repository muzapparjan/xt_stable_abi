//! The scalar wrappers whose validity rules are narrower than their width.

use xt_stable_abi::{AbiStatus, AbiVersion, Handle, StableBool, StableChar, StableVec};

#[test]
fn booleans_roundtrip() {
    assert_eq!(StableBool::TRUE.to_bool(), Ok(true));
    assert_eq!(StableBool::FALSE.to_bool(), Ok(false));
    assert_eq!(StableBool::from_bool(true).raw(), 1);
    assert!(StableBool::from_raw(0).is_valid());
    assert!(StableBool::from_raw(1).is_valid());
}

#[test]
fn non_boolean_bytes_are_rejected() {
    for raw in [2u8, 3, 0x7F, 0xFF] {
        let value = StableBool::from_raw(raw);
        assert!(!value.is_valid());
        assert_eq!(value.to_bool().unwrap_err().status(), AbiStatus::InvalidArgument);
    }
}

#[test]
fn characters_roundtrip() {
    for character in ['a', 'é', '日', '🦀', '\0', '\u{10FFFF}'] {
        let value = StableChar::from_char(character);
        assert_eq!(value.raw(), character as u32);
        assert_eq!(value.to_char(), Ok(character));
    }
}

#[test]
fn surrogates_and_out_of_range_are_rejected() {
    for code in [0xD800u32, 0xDBFF, 0xDC00, 0xDFFF, 0x110000, u32::MAX] {
        let value = StableChar::from_raw(code);
        assert!(!value.is_valid());
        assert_eq!(value.to_char().unwrap_err().status(), AbiStatus::InvalidArgument);
    }
}

#[test]
fn wiping_a_scalar_leaves_a_value_its_type_defines() {
    use xt_stable_abi::Wipe;

    // The wipe of a scalar is one volatile store of zeros. Zero is a defined
    // value of every type here, which is why the same store serves all of them.
    let mut flag = StableBool::from_raw(1);
    flag.wipe_in_place();
    assert_eq!(flag.raw(), 0);
    assert!(flag.is_valid());

    let mut character = StableChar::from_char('x');
    character.wipe_in_place();
    assert_eq!(character.raw(), 0);
    assert!(character.is_valid());

    let mut plain = true;
    plain.wipe_in_place();
    assert!(!plain);

    let mut letter = 'x';
    letter.wipe_in_place();
    assert_eq!(letter, '\0');

    // A handle keeps the word that names no object rather than the one that
    // names slot zero of generation zero, so `is_none` still answers.
    let mut handle = Handle::from_raw(7);
    handle.wipe_in_place();
    assert!(handle.is_none());

    let mut version = AbiVersion::new(3, 9);
    version.wipe_in_place();
    assert_eq!(version, AbiVersion::zero());
}

#[test]
fn fixed_layout_values_can_be_stored_in_a_vector() {
    // Each of these has a layout this crate declares, so a vector of them is a
    // boundary value rather than a container of `repr(Rust)` types.
    let mut versions = StableVec::new();
    versions.push(AbiVersion::new(1, 2));
    assert_eq!(versions.as_slice(), &[AbiVersion::new(1, 2)]);

    let mut handles = StableVec::new();
    handles.push(Handle::from_parts(3, 4));
    assert_eq!(handles.as_slice()[0].index(), Some(3));

    let mut statuses = StableVec::new();
    statuses.push(AbiStatus::InvalidUtf8);
    assert_eq!(statuses.as_slice()[0], AbiStatus::InvalidUtf8);
}

#[test]
fn scalar_boundaries_are_exact() {
    for code in [0x0000u32, 0xD7FF, 0xE000, 0x10FFFF] {
        assert!(StableChar::from_raw(code).is_valid(), "U+{code:04X} is a scalar value");
    }
    for code in [0xD800u32, 0xDFFF, 0x110000] {
        assert!(!StableChar::from_raw(code).is_valid(), "U+{code:04X} is not");
    }
}
