//! A span of time stored as whole seconds and a fraction.

use core::fmt;
use core::time::Duration;

use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::StableElement;

/// The number of nanoseconds in one second.
pub const NANOS_PER_SEC: u32 = 1_000_000_000;

/// A span of time, with the fraction expressed in nanoseconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StableDuration {
    /// Whole seconds.
    pub secs: u64,
    /// The fraction, below [`NANOS_PER_SEC`].
    pub nanos: u32,
}

impl StableDuration {
    /// Builds a span from its parts, rejecting a fraction that is not below
    /// [`NANOS_PER_SEC`].
    pub const fn new(secs: u64, nanos: u32) -> AbiResult<Self> {
        if nanos >= NANOS_PER_SEC {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        Ok(Self { secs, nanos })
    }

    /// The span as whole nanoseconds.
    ///
    /// This cannot be out of range: a `u64` of seconds times one billion fits
    /// `u128`, which is the widest integer this crate names.
    pub const fn as_nanos(self) -> u128 {
        (self.secs as u128) * (NANOS_PER_SEC as u128) + self.nanos as u128
    }

    /// Copies a native span.
    pub const fn from_duration(duration: Duration) -> Self {
        Self {
            secs: duration.as_secs(),
            nanos: duration.subsec_nanos(),
        }
    }

    /// Converts to a native span, reporting a fraction that is too large.
    pub const fn to_duration(self) -> AbiResult<Duration> {
        if self.nanos >= NANOS_PER_SEC {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        Ok(Duration::new(self.secs, self.nanos))
    }

    /// Reports whether the stored fraction is below one second.
    pub const fn is_valid(self) -> bool {
        self.nanos < NANOS_PER_SEC
    }

    /// A length of exactly `secs` seconds.
    pub const fn from_secs(secs: u64) -> Self {
        Self { secs, nanos: 0 }
    }

    /// A length of exactly `millis` milliseconds.
    pub const fn from_millis(millis: u64) -> AbiResult<Self> {
        let secs = millis / 1_000;
        let nanos = (millis % 1_000) * 1_000_000;
        Self::new(secs, nanos as u32)
    }

    /// A length of exactly `micros` microseconds.
    pub const fn from_micros(micros: u64) -> AbiResult<Self> {
        let secs = micros / 1_000_000;
        let nanos = (micros % 1_000_000) * 1_000;
        Self::new(secs, nanos as u32)
    }

    /// A length of exactly `nanos` nanoseconds.
    pub const fn from_nanos(nanos: u64) -> AbiResult<Self> {
        let secs = nanos / NANOS_PER_SEC as u64;
        let fraction = nanos % NANOS_PER_SEC as u64;
        Self::new(secs, fraction as u32)
    }

    /// The length in seconds, with the fraction.
    pub const fn as_secs_f64(self) -> f64 {
        self.secs as f64 + self.nanos as f64 / 1e9
    }

    /// The length divided by `divisor`.
    ///
    /// Answers `None` when `divisor` is zero, and when this value is not a span
    /// this type defines: a fraction at or past one second is not a length, so
    /// dividing it would answer with a length nobody asked for.
    pub fn checked_div(self, divisor: u32) -> Option<Self> {
        if divisor == 0 || !self.is_valid() {
            return None;
        }
        let whole = self.secs / divisor as u64;
        let remainder = self.secs % divisor as u64;
        let nanos = (remainder * (NANOS_PER_SEC as u64) + (self.nanos as u64)) / divisor as u64;
        Self::new(whole, nanos as u32).ok()
    }

    /// The whole seconds.
    pub const fn as_secs(self) -> u64 {
        self.secs
    }

    /// The fraction, in nanoseconds below one second.
    pub const fn subsec_nanos(self) -> u32 {
        self.nanos
    }

    /// The whole milliseconds.
    ///
    /// This cannot be out of range, for the reason [`StableDuration::as_nanos`]
    /// gives.
    pub const fn as_millis(self) -> u128 {
        (self.secs as u128) * 1000 + (self.nanos / 1_000_000) as u128
    }

    /// Reports whether the span is zero.
    pub const fn is_zero(self) -> bool {
        self.secs == 0 && self.nanos == 0
    }

    /// Multiplies the span by `factor`, or answers `None` when the product does
    /// not fit.
    ///
    /// The product is taken in nanoseconds, so a result is always a whole number
    /// of them; the only way this fails is a total that no `u64` of seconds can
    /// hold.
    ///
    /// A value whose fraction is at or past one second is not a span this type
    /// defines, so it answers `None` rather than being folded into a length.
    pub const fn checked_mul(self, factor: u32) -> Option<Self> {
        if !self.is_valid() {
            return None;
        }
        let total = self.as_nanos();
        let scaled = match total.checked_mul(factor as u128) {
            Some(scaled) => scaled,
            None => return None,
        };
        let secs = scaled / (NANOS_PER_SEC as u128);
        if secs > u64::MAX as u128 {
            return None;
        }
        Some(Self {
            secs: secs as u64,
            nanos: (scaled % (NANOS_PER_SEC as u128)) as u32,
        })
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

    /// Subtracts `other`, answering with `None` when the result would be
    /// negative: a duration has no sign.
    ///
    /// A fraction at or past one second is not a duration this type defines, so
    /// a value that carries one answers `None` instead of borrowing the carry
    /// arithmetic below.
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        if !self.is_valid() || !other.is_valid() {
            return None;
        }
        if self.secs < other.secs {
            return None;
        }
        let seconds = self.secs - other.secs;
        if self.nanos >= other.nanos {
            Some(Self {
                secs: seconds,
                nanos: self.nanos - other.nanos,
            })
        } else if seconds == 0 {
            None
        } else {
            Some(Self {
                secs: seconds - 1,
                nanos: NANOS_PER_SEC - (other.nanos - self.nanos),
            })
        }
    }

    /// Adds `other`, answering with `None` when the seconds would overflow.
    ///
    /// A fraction at or past one second is not a duration this type defines:
    /// adding two of them would overflow the `u32` before the carry below, so
    /// such a value answers `None`.
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        if !self.is_valid() || !other.is_valid() {
            return None;
        }
        let seconds = match self.secs.checked_add(other.secs) {
            Some(seconds) => seconds,
            None => return None,
        };
        let nanos = self.nanos + other.nanos;
        if nanos >= NANOS_PER_SEC {
            match seconds.checked_add(1) {
                Some(seconds) => Some(Self {
                    secs: seconds,
                    nanos: nanos - NANOS_PER_SEC,
                }),
                None => None,
            }
        } else {
            Some(Self { secs: seconds, nanos })
        }
    }
}

impl crate::zeroize::Wipe for StableDuration {
    fn wipe_in_place(&mut self) {
        // Zero seconds and zero nanoseconds is a value this type defines.
        crate::zeroize::wipe_value(self);
    }
}

impl From<Duration> for StableDuration {
    fn from(value: Duration) -> Self {
        Self::from_duration(value)
    }
}

impl TryFrom<StableDuration> for Duration {
    type Error = AbiError;

    fn try_from(value: StableDuration) -> AbiResult<Self> {
        value.to_duration()
    }
}

impl fmt::Display for StableDuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_duration() {
            Ok(duration) => write!(formatter, "{:?}", duration),
            Err(_) => write!(formatter, "{}s (+{}ns)", self.secs, self.nanos),
        }
    }
}

// SAFETY: the struct is two scalars with a fixed layout and owns nothing.
unsafe impl StableElement for StableDuration {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}
