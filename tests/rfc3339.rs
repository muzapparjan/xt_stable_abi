//! RFC 3339 text and the values it names.

use xt_stable_abi::{AbiStatus, StableSystemTime};

fn at(seconds: i64) -> StableSystemTime {
    StableSystemTime::new(seconds, 0).unwrap()
}

fn text(value: &StableSystemTime) -> String {
    value.to_rfc3339().unwrap().into_string()
}

#[test]
fn the_epoch_is_written_in_utc() {
    assert_eq!(text(&at(0)), "1970-01-01T00:00:00Z");
    assert_eq!(text(&at(1_700_000_000)), "2023-11-14T22:13:20Z");
}

#[test]
fn a_time_before_the_epoch_keeps_its_second() {
    assert_eq!(text(&at(-1)), "1969-12-31T23:59:59Z");
    let value = StableSystemTime::new(-1, 250_000_000).unwrap();
    assert_eq!(text(&value), "1969-12-31T23:59:59.25Z");
}

#[test]
fn a_leap_day_is_written_and_read_back() {
    let leap = at(951_782_400);
    assert_eq!(text(&leap), "2000-02-29T00:00:00Z");
    assert_eq!(StableSystemTime::from_rfc3339("2000-02-29T00:00:00Z").unwrap(), leap);
}

#[test]
fn the_fraction_is_written_without_trailing_zeros() {
    let value = StableSystemTime::new(0, 123_000_000).unwrap();
    assert_eq!(text(&value), "1970-01-01T00:00:00.123Z");
    let value = StableSystemTime::new(0, 1).unwrap();
    assert_eq!(text(&value), "1970-01-01T00:00:00.000000001Z");
}

#[test]
fn text_written_here_is_read_back_here() {
    let values = [
        StableSystemTime::new(0, 0).unwrap(),
        StableSystemTime::new(1_700_000_000, 999_999_999).unwrap(),
        StableSystemTime::new(-86_400, 1).unwrap(),
        StableSystemTime::new(253_402_300_799, 500_000_000).unwrap(),
    ];
    for value in values {
        let written = value.to_rfc3339().unwrap();
        let read = StableSystemTime::from_rfc3339(written.as_str()).unwrap();
        assert_eq!(read, value, "round trip of {}", written.as_str());
    }
}

#[test]
fn an_offset_moves_the_point_to_utc() {
    let with_offset = StableSystemTime::from_rfc3339("2024-02-29T12:00:00+02:00").unwrap();
    let without = StableSystemTime::from_rfc3339("2024-02-29T10:00:00Z").unwrap();
    assert_eq!(with_offset, without);
    let west = StableSystemTime::from_rfc3339("2024-02-29T10:00:00-05:30").unwrap();
    let utc = StableSystemTime::from_rfc3339("2024-02-29T15:30:00Z").unwrap();
    assert_eq!(west, utc);
}

#[test]
fn a_lower_case_separator_is_accepted() {
    let lower = StableSystemTime::from_rfc3339("2024-02-29t10:00:00z").unwrap();
    let upper = StableSystemTime::from_rfc3339("2024-02-29T10:00:00Z").unwrap();
    assert_eq!(lower, upper);
}

#[test]
fn text_outside_the_form_is_refused() {
    let refused = [
        "",
        "2024-02-29T10:00:00",
        "2024-02-29 10:00:00Z",
        "2024-13-01T00:00:00Z",
        "2024-02-30T00:00:00Z",
        "2023-02-29T00:00:00Z",
        "2024-02-29T24:00:00Z",
        "2024-02-29T23:60:00Z",
        "2024-02-29T23:59:60Z",
        "2024-02-29T23:59:59.1234567890Z",
        "2024-02-29T23:59:59.",
        "2024-02-29T23:59:59Z ",
        "2024-02-29T23:59:59+24:00",
        "2024-02-29T23:59:59+02:60",
        "2024-02-29T23:59:59Q",
        "20240229T235959Z",
        "2024-02-29T23:59:59.Z",
    ];
    for input in refused {
        assert_eq!(
            StableSystemTime::from_rfc3339(input)
                .unwrap_err()
                .status(),
            AbiStatus::InvalidArgument,
            "{input} was accepted",
        );
    }
}

#[test]
fn a_year_outside_the_written_range_is_refused() {
    let far = StableSystemTime::new(i64::MAX / 2, 0).unwrap();
    assert_eq!(
        far.to_rfc3339().unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
    let early = StableSystemTime::new(i64::MIN / 2, 0).unwrap();
    assert_eq!(
        early.to_rfc3339().unwrap_err().status(),
        AbiStatus::InvalidArgument
    );
}

#[test]
fn an_offset_writes_local_time_and_names_the_offset() {
    assert_eq!(
        at(0)
            .to_rfc3339_with_offset(8 * 3600)
            .unwrap()
            .into_string(),
        "1970-01-01T08:00:00+08:00"
    );
    assert_eq!(
        at(0)
            .to_rfc3339_with_offset(-(5 * 3600 + 30 * 60))
            .unwrap()
            .into_string(),
        "1969-12-31T18:30:00-05:30"
    );
    // The same instant, two zones: the text differs, the point does not.
    let local = at(0).to_rfc3339_with_offset(8 * 3600).unwrap();
    let read_back = StableSystemTime::from_rfc3339(local.as_str()).unwrap();
    assert_eq!(read_back, at(0));
}

#[test]
fn an_offset_the_text_cannot_spell_is_refused() {
    assert_eq!(
        at(0)
            .to_rfc3339_with_offset(30)
            .unwrap_err()
            .status(),
        AbiStatus::InvalidArgument
    );
    assert_eq!(
        at(0)
            .to_rfc3339_with_offset(24 * 3600)
            .unwrap_err()
            .status(),
        AbiStatus::InvalidArgument
    );
}
