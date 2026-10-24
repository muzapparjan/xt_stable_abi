//! Version pairs used to negotiate the shape of data exchanged across the
//! boundary.
//!
//! Two independently built sides agree on a version before they exchange any
//! value. The pair is deliberately coarse: the major number changes when the
//! layout or the meaning of an existing field changes, the minor number when a
//! purely additive change lands.
//!
//! A host reads a library it loaded through two numbers, and they answer
//! different questions. The **major** number and the layout revision decide
//! whether the bytes can be read at all: a field's offset or meaning may differ
//! under another major, so a library of another major is refused before any
//! value is adopted. The **minor** number decides what the host is entitled to
//! call: [`AbiVersion::serves`] is the floor the host requires of the library,
//! and a library at a newer minor is accepted, because what it added sits past
//! the bytes this build reads.
//!
//! The consequence for a release is that a minor increment alone does not force
//! both sides to be rebuilt in the same step — a host at `1.0` serves a library
//! at `1.1`, and a host at `1.1` serves a library at `1.0` only if it lowers the
//! version it requires (`Plugin::load` requires its own version by default;
//! `load_with_required` states another floor).

use core::fmt;

use crate::vec::StableElement;

/// A major/minor version pair.
///
/// The pair is `#[repr(C)]` and carries no padding, so it can be embedded in
/// other boundary types and read by a side that only knows the first version
/// of the enclosing struct.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbiVersion {
    /// Incremented for changes that break compatibility.
    pub major: u32,
    /// Incremented for additive changes.
    pub minor: u32,
}

impl AbiVersion {
    /// Builds a version pair.
    pub const fn new(major: u32, minor: u32) -> Self {
        Self { major, minor }
    }

    /// The oldest version this crate defines.
    pub const fn zero() -> Self {
        Self::new(0, 0)
    }

    /// Reports whether a provider at `self` can serve a consumer that requires
    /// `required`.
    ///
    /// Compatibility requires an equal major number and a provider minor that
    /// is at least the required one: a provider that knows every field the
    /// consumer knows can answer, the reverse need not hold.
    pub const fn serves(self, required: AbiVersion) -> bool {
        self.major == required.major && self.minor >= required.minor
    }

    /// Packs the pair into a single word, major in the high half.
    pub const fn to_raw(self) -> u64 {
        ((self.major as u64) << 32) | (self.minor as u64)
    }

    /// Rebuilds a pair produced by [`AbiVersion::to_raw`].
    pub const fn from_raw(raw: u64) -> Self {
        Self::new((raw >> 32) as u32, raw as u32)
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl Default for AbiVersion {
    fn default() -> Self {
        Self::zero()
    }
}

impl crate::zeroize::Wipe for AbiVersion {
    fn wipe_in_place(&mut self) {
        // Zero is the oldest version this crate defines, which is a value this
        // type describes.
        crate::zeroize::wipe_value(self);
    }
}

// SAFETY: the pair is two integers with no padding, and it owns nothing.
unsafe impl StableElement for AbiVersion {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}
