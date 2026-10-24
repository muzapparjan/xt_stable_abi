//! Fixed-width replacements for the two scalar types whose validity rules are
//! narrower than their byte size.
//!
//! `bool` is one byte but only `0` and `1` are meaningful, and `char` is a
//! `u32` restricted to Unicode scalar values. Both wrappers carry the raw
//! integer and validate on the way into Rust, so a value produced by another
//! build is checked rather than trusted.

use core::fmt;

use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::StableElement;

/// A `bool` stored as a byte.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct StableBool(u8);

impl StableBool {
    /// The `false` value.
    pub const FALSE: Self = Self(0);
    /// The `true` value.
    pub const TRUE: Self = Self(1);

    /// Stores `value`.
    pub const fn from_bool(value: bool) -> Self {
        Self(value as u8)
    }

    /// Stores the raw byte, which may be invalid.
    pub const fn from_raw(value: u8) -> Self {
        Self(value)
    }

    /// The stored byte.
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// Reports whether the stored byte is `0` or `1`.
    pub const fn is_valid(self) -> bool {
        self.0 <= 1
    }

    /// The boolean, or [`AbiStatus::InvalidArgument`] when the byte is neither
    /// `0` nor `1`.
    pub const fn to_bool(self) -> AbiResult<bool> {
        match self.0 {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(AbiError::new(AbiStatus::InvalidArgument)),
        }
    }
}

impl From<bool> for StableBool {
    fn from(value: bool) -> Self {
        Self::from_bool(value)
    }
}

impl TryFrom<StableBool> for bool {
    type Error = AbiError;

    fn try_from(value: StableBool) -> AbiResult<Self> {
        value.to_bool()
    }
}

impl fmt::Display for StableBool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_bool() {
            Ok(value) => value.fmt(formatter),
            Err(_) => write!(formatter, "<invalid bool: {}>", self.0),
        }
    }
}

// SAFETY: the wrapper has the layout of a byte and owns nothing.
unsafe impl StableElement for StableBool {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl crate::zeroize::Wipe for StableBool {
    fn wipe_in_place(&mut self) {
        // Zero is `false`, which is a value this type defines. The write is
        // volatile, like every other wipe here: a plain store to memory that is
        // about to be released is the compiler's to remove.
        crate::zeroize::wipe_value(self);
    }
}

impl PartialOrd for StableBool {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StableBool {
    /// Orders the stored byte, so a false value sorts before a true one.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

/// A `char` stored as a `u32` code point.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct StableChar(u32);

impl StableChar {
    /// Stores `value`.
    pub const fn from_char(value: char) -> Self {
        Self(value as u32)
    }

    /// Stores the raw code point, which may be invalid.
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// The stored code point.
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Reports whether the stored code point is a Unicode scalar value.
    pub const fn is_valid(self) -> bool {
        is_scalar_value(self.0)
    }

    /// The character, or [`AbiStatus::InvalidArgument`] when the code point is
    /// a surrogate or beyond the Unicode range.
    pub const fn to_char(self) -> AbiResult<char> {
        match char::from_u32(self.0) {
            Some(value) => Ok(value),
            None => Err(AbiError::new(AbiStatus::InvalidArgument)),
        }
    }
}

/// Reports whether `code` is a Unicode scalar value.
const fn is_scalar_value(code: u32) -> bool {
    code <= 0x10FFFF && !(code >= 0xD800 && code <= 0xDFFF)
}

impl From<char> for StableChar {
    fn from(value: char) -> Self {
        Self::from_char(value)
    }
}

impl TryFrom<StableChar> for char {
    type Error = AbiError;

    fn try_from(value: StableChar) -> AbiResult<Self> {
        value.to_char()
    }
}

impl fmt::Display for StableChar {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_char() {
            Ok(value) => value.fmt(formatter),
            Err(_) => write!(formatter, "<invalid char: U+{:04X}>", self.0),
        }
    }
}

impl PartialOrd for StableChar {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StableChar {
    /// Orders the stored code point.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

// SAFETY: the wrapper has the layout of a `u32` and owns nothing.
unsafe impl StableElement for StableChar {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl crate::zeroize::Wipe for StableChar {
    fn wipe_in_place(&mut self) {
        // Zero is U+0000, a Unicode scalar value, so the result is a value this
        // type defines, and the write is volatile for the reason the one above
        // is.
        crate::zeroize::wipe_value(self);
    }
}
