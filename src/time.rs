//! Points in time with a layout that does not depend on the platform.

use core::fmt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::duration::NANOS_PER_SEC;
use crate::duration::StableDuration;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::string::StableString;
use crate::vec::StableElement;

/// A point on the calendar, counted from the Unix epoch.
///
/// The offset is signed because a time before the epoch is a value too. The
/// fraction is nanoseconds and always below one second, so the same instant has
/// one representation on both sides.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableSystemTime {
    /// Whole seconds from the Unix epoch, negative before it.
    pub seconds: i64,
    /// The fraction of a second, below [`NANOS_PER_SEC`].
    pub nanos: u32,
}

impl StableSystemTime {
    /// Builds a point, rejecting a fraction that is not below one second.
    pub const fn new(seconds: i64, nanos: u32) -> AbiResult<Self> {
        if nanos >= NANOS_PER_SEC {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        Ok(Self { seconds, nanos })
    }

    /// Reports whether the stored fraction is below one second.
    pub const fn is_valid(self) -> bool {
        self.nanos < NANOS_PER_SEC
    }

    /// Checks that the fraction is below one second, which is the only state
    /// this type does not otherwise describe.
    pub const fn validate(&self) -> AbiResult<()> {
        if self.is_valid() {
            Ok(())
        } else {
            Err(AbiError::new(AbiStatus::InvalidArgument))
        }
    }

    /// The point as RFC 3339 text in UTC, for example `2026-10-23T12:34:56Z`.
    ///
    /// The fraction is written only when it is not zero, and trailing zeros are
    /// left out. Years outside `0000`..`9999` are refused: RFC 3339 writes them
    /// only with an agreed expansion, and a reader that does not share the
    /// agreement would read a different date.
    ///
    /// Converting to a local zone needs the zone's rules for the date in
    /// question, which come from a database this crate does not carry; a caller
    /// that has one passes the offset it looked up to
    /// [`StableSystemTime::to_rfc3339_with_offset`].
    pub fn to_rfc3339(&self) -> AbiResult<StableString> {
        self.write_rfc3339(0)
    }

    /// Writes the instant as text in the zone `offset_seconds` east of UTC.
    ///
    /// This is how a caller whose host has a zone database writes local time:
    /// the host knows the offset that applies to this instant, and this method
    /// applies it. An offset that is not a whole number of minutes is refused,
    /// because RFC 3339 spells an offset as hours and minutes, and the text would
    /// then name a different instant from the one this value holds.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::InvalidArgument`] when the offset is not a whole number of
    /// minutes, is not smaller than a day, or moves the result outside the years
    /// this form writes.
    pub fn to_rfc3339_with_offset(&self, offset_seconds: i64) -> AbiResult<StableString> {
        self.validate()?;
        if offset_seconds % 60 != 0 || offset_seconds <= -SECONDS_PER_DAY
            || offset_seconds >= SECONDS_PER_DAY
        {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        self.write_rfc3339(offset_seconds)
    }

    fn write_rfc3339(&self, offset_seconds: i64) -> AbiResult<StableString> {
        self.validate()?;
        let local = self
            .seconds
            .checked_add(offset_seconds)
            .ok_or_else(|| AbiError::new(AbiStatus::InvalidArgument))?;
        let days = local.div_euclid(SECONDS_PER_DAY);
        let second_of_day = local.rem_euclid(SECONDS_PER_DAY);
        let (year, month, day) = civil_from_days(days);
        if !(0..=9999).contains(&year) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let mut text = StableString::new();
        push_digits(&mut text, year as u64, 4)?;
        text.push_str("-")?;
        push_digits(&mut text, month as u64, 2)?;
        text.push_str("-")?;
        push_digits(&mut text, day as u64, 2)?;
        text.push_str("T")?;
        push_digits(&mut text, (second_of_day / 3600) as u64, 2)?;
        text.push_str(":")?;
        push_digits(&mut text, (second_of_day % 3600 / 60) as u64, 2)?;
        text.push_str(":")?;
        push_digits(&mut text, (second_of_day % 60) as u64, 2)?;
        if self.nanos != 0 {
            let mut fraction = self.nanos;
            let mut digits = 9;
            while fraction.is_multiple_of(10) {
                fraction /= 10;
                digits -= 1;
            }
            text.push_str(".")?;
            push_digits(&mut text, fraction as u64, digits)?;
        }
        if offset_seconds == 0 {
            text.push_str("Z")?;
        } else {
            if offset_seconds < 0 {
                text.push_str("-")?;
            } else {
                text.push_str("+")?;
            }
            let magnitude = offset_seconds.unsigned_abs();
            push_digits(&mut text, magnitude / 3600, 2)?;
            text.push_str(":")?;
            push_digits(&mut text, magnitude % 3600 / 60, 2)?;
        }
        Ok(text)
    }

    /// Reads RFC 3339 text, in UTC or with a numeric offset.
    ///
    /// The accepted form is `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`, with
    /// one to nine digits of fraction. `T` and `Z` may be lower case. A leap
    /// second (`:60`) is refused: this type counts seconds, and a minute that
    /// holds sixty-one of them has no representation here.
    pub fn from_rfc3339(text: &str) -> AbiResult<Self> {
        let bytes = text.as_bytes();
        // The shortest accepted form is `YYYY-MM-DDTHH:MM:SSZ`.
        if bytes.len() < 20 {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let year = read_digits(bytes, 0, 4)? as i64;
        expect(bytes, 4, b'-')?;
        let month = read_digits(bytes, 5, 2)?;
        expect(bytes, 7, b'-')?;
        let day = read_digits(bytes, 8, 2)?;
        match bytes[10] {
            b'T' | b't' => {}
            _ => return Err(AbiError::new(AbiStatus::InvalidArgument)),
        }
        let hour = read_digits(bytes, 11, 2)?;
        expect(bytes, 13, b':')?;
        let minute = read_digits(bytes, 14, 2)?;
        expect(bytes, 16, b':')?;
        let second = read_digits(bytes, 17, 2)?;
        let mut index = 19;
        let mut nanos: u32 = 0;
        if bytes[index] == b'.' {
            index += 1;
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            let digits = index - start;
            if digits == 0 || digits > 9 {
                return Err(AbiError::new(AbiStatus::InvalidArgument));
            }
            nanos = read_digits(bytes, start, digits)?;
            let mut scale = digits;
            while scale < 9 {
                nanos *= 10;
                scale += 1;
            }
        }
        let offset = match bytes.get(index) {
            Some(b'Z') | Some(b'z') => {
                index += 1;
                0i64
            }
            Some(sign @ (b'+' | b'-')) => {
                let sign = if *sign == b'+' { 1i64 } else { -1 };
                let hours = read_digits(bytes, index + 1, 2)? as i64;
                expect(bytes, index + 3, b':')?;
                let minutes = read_digits(bytes, index + 4, 2)? as i64;
                index += 6;
                if hours > 23 || minutes > 59 {
                    return Err(AbiError::new(AbiStatus::InvalidArgument));
                }
                sign * (hours * 3600 + minutes * 60)
            }
            _ => return Err(AbiError::new(AbiStatus::InvalidArgument)),
        };
        if index != bytes.len() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        if hour > 23 || minute > 59 || second > 59 {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let days = days_from_civil(year, month, day);
        let seconds = days
            .checked_mul(SECONDS_PER_DAY)
            .and_then(|base| {
                base.checked_add((hour * 3600 + minute * 60 + second) as i64)
            })
            .and_then(|local| local.checked_sub(offset))
            .ok_or_else(|| AbiError::new(AbiStatus::InvalidArgument))?;
        Self::new(seconds, nanos)
    }

    /// Copies a native time.
    ///
    /// A time whose distance from the epoch does not fit the seconds field is
    /// reported rather than clamped: a clamped value names a different instant,
    /// which the receiving side could not tell from the one that was meant.
    pub fn from_system_time(time: SystemTime) -> AbiResult<Self> {
        match time.duration_since(UNIX_EPOCH) {
            Ok(after) => Self::new(
                i64::try_from(after.as_secs())
                    .map_err(|_| AbiError::new(AbiStatus::InvalidArgument))?,
                after.subsec_nanos(),
            ),
            Err(error) => {
                let before = error.duration();
                let seconds = i64::try_from(before.as_secs())
                    .map_err(|_| AbiError::new(AbiStatus::InvalidArgument))?;
                // A time before the epoch is stored as a negative offset; when
                // the fraction is non-zero the whole second moves down so the
                // fraction stays a forward offset from `seconds`.
                if before.subsec_nanos() == 0 {
                    Self::new(-seconds, 0)
                } else {
                    let seconds = seconds.checked_add(1).ok_or_else(|| {
                        AbiError::new(AbiStatus::InvalidArgument)
                    })?;
                    Self::new(
                        -seconds,
                        NANOS_PER_SEC - before.subsec_nanos(),
                    )
                }
            }
        }
    }

    /// Rebuilds a native time.
    pub fn to_system_time(self) -> AbiResult<SystemTime> {
        if !self.is_valid() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let nanos = u64::from(self.nanos);
        if self.seconds >= 0 {
            let seconds = Duration::from_secs(self.seconds as u64);
            Ok(UNIX_EPOCH
                .checked_add(seconds)
                .and_then(|time| time.checked_add(Duration::from_nanos(nanos)))
                .ok_or_else(|| AbiError::new(AbiStatus::InvalidArgument))?)
        } else {
            // `seconds` is the whole second below the instant, so the fraction
            // counts forward from it.
            let magnitude = self.seconds.unsigned_abs();
            let forward = Duration::from_secs(magnitude).checked_sub(Duration::from_nanos(nanos));
            let offset = match forward {
                Some(offset) => offset,
                None => {
                    // The fraction is larger than the whole-second offset; that
                    // happens only for a malformed value.
                    return Err(AbiError::new(AbiStatus::InvalidArgument));
                }
            };
            UNIX_EPOCH
                .checked_sub(offset)
                .ok_or_else(|| AbiError::new(AbiStatus::InvalidArgument))
        }
    }
}

impl fmt::Display for StableSystemTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_system_time() {
            Ok(time) => write!(formatter, "{time:?}"),
            Err(_) => write!(formatter, "{}s (+{}ns)", self.seconds, self.nanos),
        }
    }
}

/// A reading of a monotonic clock, relative to an origin the two sides agree on.
///
/// A native `Instant` has no meaning outside the process that produced it. The
/// value here carries the distance from a base the sender also communicates,
/// plus an identifier for that base; a receiver that does not share the base
/// must not interpret the reading.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableInstant {
    /// Names the base the reading counts from.
    pub origin: u64,
    /// Nanoseconds since that base.
    pub nanos: u64,
}

impl StableInstant {
    /// Builds a reading from its parts.
    pub const fn new(origin: u64, nanos: u64) -> Self {
        Self { origin, nanos }
    }

    /// The distance from `base` to `instant`.
    ///
    /// `origin` must name `base`; the receiving side needs the same base to
    /// rebuild the reading.
    ///
    /// An `instant` earlier than `base` is not a distance this type can carry,
    /// so the reading is recorded as zero: the function saturates rather than
    /// naming a negative span. A sender that needs to tell "no time passed" from
    /// "the clock went backwards" records the sentinel itself.
    pub fn from_instant(origin: u64, base: Instant, instant: Instant) -> Self {
        let offset = instant.saturating_duration_since(base);
        Self {
            origin,
            nanos: offset.as_nanos().min(u64::MAX as u128) as u64,
        }
    }

    /// Rebuilds the reading on top of `base`.
    ///
    /// The result is only meaningful when `base` is the instant the sender used.
    pub fn to_instant(self, base: Instant) -> AbiResult<Instant> {
        match base.checked_add(Duration::from_nanos(self.nanos)) {
            Some(instant) => Ok(instant),
            None => Err(AbiError::new(AbiStatus::InvalidArgument)),
        }
    }

    /// The reading as a span.
    pub const fn as_duration(self) -> Duration {
        Duration::new(
            self.nanos / NANOS_PER_SEC as u64,
            (self.nanos % NANOS_PER_SEC as u64) as u32,
        )
    }

    /// Every pair of fields is a reading, so there is no state to reject; the
    /// method exists so the reading can stand where a validated value is
    /// required.
    pub const fn validate(&self) -> AbiResult<()> {
        Ok(())
    }
}

impl fmt::Display for StableInstant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}ns since origin {}", self.nanos, self.origin)
    }
}

// SAFETY: the point is two integers with a fixed layout and owns nothing.
unsafe impl StableElement for StableSystemTime {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl crate::zeroize::Wipe for StableSystemTime {
    fn wipe_in_place(&mut self) {
        // The epoch with no fraction is a value this type defines.
        crate::zeroize::wipe_value(self);
    }
}

impl crate::zeroize::Wipe for StableInstant {
    fn wipe_in_place(&mut self) {
        // Zero nanoseconds from an origin named zero is a value this type
        // defines.
        crate::zeroize::wipe_value(self);
    }
}

// SAFETY: the reading is two integers with a fixed layout and owns nothing.
unsafe impl StableElement for StableInstant {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl crate::clone::TryClone for StableSystemTime {
    /// A reading owns no storage, so its copy cannot fail.
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(*self)
    }
}

impl crate::clone::TryClone for StableInstant {
    /// A reading owns no storage, so its copy cannot fail.
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(*self)
    }
}

/// The number of seconds in a day.
const SECONDS_PER_DAY: i64 = 86_400;

/// Reports whether `year` is a leap year in the Gregorian calendar.
fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// The number of days in `month` of `year`.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The calendar date a count of days since the Unix epoch names.
///
/// The arithmetic is the standard shift of the epoch to March so that the leap
/// day lands at the end of the year, which makes the month lengths periodic.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// The number of days since the Unix epoch for a calendar date.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 } as i64;
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Reads `count` decimal digits starting at `at`.
fn read_digits(bytes: &[u8], at: usize, count: usize) -> AbiResult<u32> {
    let end = at
        .checked_add(count)
        .ok_or_else(|| AbiError::new(AbiStatus::InvalidArgument))?;
    if end > bytes.len() {
        return Err(AbiError::new(AbiStatus::InvalidArgument));
    }
    let mut value = 0u32;
    for byte in &bytes[at..end] {
        if !byte.is_ascii_digit() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        value = value * 10 + (byte - b'0') as u32;
    }
    Ok(value)
}

/// Requires the byte at `at` to be `expected`.
fn expect(bytes: &[u8], at: usize, expected: u8) -> AbiResult<()> {
    if bytes.get(at) == Some(&expected) {
        Ok(())
    } else {
        Err(AbiError::new(AbiStatus::InvalidArgument))
    }
}

/// Appends `value` as at least `width` decimal digits.
fn push_digits(text: &mut StableString, value: u64, width: usize) -> AbiResult<()> {
    let mut buffer = [0u8; 20];
    let mut index = buffer.len();
    let mut remaining = value;
    loop {
        index -= 1;
        buffer[index] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    let written = buffer.len() - index;
    let mut padding = written;
    while padding < width {
        text.push_str("0")?;
        padding += 1;
    }
    // SAFETY: every byte written above is an ASCII digit.
    text.push_str(unsafe { core::str::from_utf8_unchecked(&buffer[index..]) })
}

impl StableSystemTime {
    /// The point in time the host's clock reads now.
    ///
    /// The answer comes from the clock of the process the call is made in. Two
    /// sides that read it at different moments therefore disagree, which is why
    /// the value is described as "now on the side that built it" rather than as
    /// a shared instant.
    pub fn now() -> AbiResult<Self> {
        let now = std::time::SystemTime::now();
        Self::from_system_time(now)
    }

    /// The length of time from `earlier` to this point.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::InvalidArgument`] when either point is not a value this
    /// type defines, or when `earlier` is later than this point: a length of
    /// time is not negative.
    pub fn duration_since(&self, earlier: Self) -> AbiResult<StableDuration> {
        self.validate()?;
        earlier.validate()?;
        if *self < earlier {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let whole = self
            .seconds
            .checked_sub(earlier.seconds)
            .ok_or(AbiError::new(AbiStatus::CapacityOverflow))?;
        let nanos = (whole as i128) * (NANOS_PER_SEC as i128)
            + (self.nanos as i128 - earlier.nanos as i128);
        let seconds = (nanos / NANOS_PER_SEC as i128) as u64;
        let fraction = (nanos % NANOS_PER_SEC as i128) as u32;
        StableDuration::new(seconds, fraction)
    }

    /// The length of time from `earlier` to this point, as a platform duration.
    pub fn elapsed_since(&self, earlier: Self) -> AbiResult<std::time::Duration> {
        self.duration_since(earlier)?.to_duration()
    }

    /// This point moved later by `length`.
    ///
    /// A length whose seconds do not fit in the signed field this type stores is
    /// refused rather than wrapped: `StableDuration` carries an unsigned count,
    /// so a value that arrived from the other side can name more seconds than a
    /// point in time can hold.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::InvalidArgument`] when this point or `length` is not a
    /// value its type defines, and [`AbiStatus::CapacityOverflow`] when the sum,
    /// or the length itself, is past the range this type describes.
    pub fn checked_add(&self, length: StableDuration) -> AbiResult<Self> {
        self.validate()?;
        length.validate()?;
        let whole = i64::try_from(length.as_secs())
            .map_err(|_| AbiError::new(AbiStatus::CapacityOverflow))?;
        let seconds = self
            .seconds
            .checked_add(whole)
            .ok_or(AbiError::new(AbiStatus::CapacityOverflow))?;
        let mut nanos = u64::from(self.nanos) + u64::from(length.subsec_nanos());
        let mut seconds = seconds;
        if nanos >= u64::from(NANOS_PER_SEC) {
            nanos -= u64::from(NANOS_PER_SEC);
            seconds = seconds
                .checked_add(1)
                .ok_or(AbiError::new(AbiStatus::CapacityOverflow))?;
        }
        Self::new(seconds, nanos as u32)
    }

    /// This point moved earlier by `length`.
    ///
    /// A length whose seconds do not fit in the signed field this type stores is
    /// refused rather than wrapped, for the same reason as
    /// [`StableSystemTime::checked_add`].
    ///
    /// # Errors
    ///
    /// [`AbiStatus::InvalidArgument`] when this point or `length` is not a
    /// value its type defines, and [`AbiStatus::CapacityOverflow`] when the
    /// difference, or the length itself, is past the range this type describes.
    pub fn checked_sub(&self, length: StableDuration) -> AbiResult<Self> {
        self.validate()?;
        length.validate()?;
        let whole = i64::try_from(length.as_secs())
            .map_err(|_| AbiError::new(AbiStatus::CapacityOverflow))?;
        let seconds = self
            .seconds
            .checked_sub(whole)
            .ok_or(AbiError::new(AbiStatus::CapacityOverflow))?;
        let removed = u64::from(length.subsec_nanos());
        let mut seconds = seconds;
        let nanos = if u64::from(self.nanos) >= removed {
            (u64::from(self.nanos) - removed) as u32
        } else {
            // Take one whole second back from the difference.
            seconds = seconds
                .checked_sub(1)
                .ok_or(AbiError::new(AbiStatus::CapacityOverflow))?;
            (u64::from(self.nanos) + u64::from(NANOS_PER_SEC) - removed) as u32
        };
        Self::new(seconds, nanos)
    }
}

impl StableInstant {
    /// The length of time from `earlier` to this instant.
    ///
    /// Both values have to come from the same origin, which is the base the
    /// side that built them measured from:
    /// [`StableInstant::from_instant`] records it.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::InvalidArgument`] when the origins differ, or when `earlier`
    /// is later than this instant.
    pub fn duration_since(&self, earlier: Self) -> AbiResult<StableDuration> {
        if self.origin != earlier.origin || self.nanos < earlier.nanos {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let difference = self.nanos - earlier.nanos;
        StableDuration::new(
            difference / (NANOS_PER_SEC as u64),
            (difference % (NANOS_PER_SEC as u64)) as u32,
        )
    }

    /// This instant moved later by `length`.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::InvalidArgument`] when `length` is not a value its type
    /// defines, and [`AbiStatus::CapacityOverflow`] when the sum is past what
    /// the nanosecond count can hold.
    pub fn checked_add(&self, length: StableDuration) -> AbiResult<Self> {
        length.validate()?;
        let added = u64::from(length.subsec_nanos());
        let seconds = length
            .as_secs()
            .checked_mul(NANOS_PER_SEC as u64)
            .and_then(|whole| whole.checked_add(added))
            .and_then(|duration| self.nanos.checked_add(duration))
            .ok_or(AbiError::new(AbiStatus::CapacityOverflow))?;
        Ok(Self {
            origin: self.origin,
            nanos: seconds,
        })
    }

    /// This instant moved earlier by `length`.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::CapacityOverflow`] when the length is past what the
    /// nanosecond count can hold, and [`AbiStatus::InvalidArgument`] when it is
    /// longer than the time this instant is measured from its origin.
    pub fn checked_sub(&self, length: StableDuration) -> AbiResult<Self> {
        length.validate()?;
        let removed = u64::from(length.subsec_nanos());
        let seconds = length
            .as_secs()
            .checked_mul(NANOS_PER_SEC as u64)
            .and_then(|whole| whole.checked_add(removed))
            .ok_or(AbiError::new(AbiStatus::CapacityOverflow))?;
        let nanos = self
            .nanos
            .checked_sub(seconds)
            .ok_or(AbiError::new(AbiStatus::InvalidArgument))?;
        Ok(Self {
            origin: self.origin,
            nanos,
        })
    }
}

