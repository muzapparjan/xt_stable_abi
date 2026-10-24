//! A transparent set of permission bits.

use core::fmt;

/// The bits a permission set may hold.
///
/// The type is transparent over a `u64`, which is how a small wrapper over an
/// integer is usually written, and every constructor is a `const fn` so a set of
/// bits can be a constant.
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Flags(u64);

impl Flags {
    /// Nothing is allowed.
    pub const NONE: Flags = Flags(0);
    /// The value may be read.
    pub const READ: Flags = Flags(1 << 0);
    /// The value may be replaced.
    pub const WRITE: Flags = Flags(1 << 1);
    /// The value may be run.
    pub const EXEC: Flags = Flags(1 << 2);
    /// The value may be handed to another holder.
    pub const ADMIN: Flags = Flags(1 << 3);

    /// Every bit this build names.
    pub const ALL: Flags = Flags(0b1111);

    /// Wraps a set of bits, including bits this build does not name.
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// The raw bits.
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Reports whether every bit of `other` is set here.
    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    /// The union of the two sets.
    pub const fn with(self, other: Flags) -> Self {
        Self(self.0 | other.0)
    }

    /// The bits of `self` that `other` does not set.
    pub const fn without(self, other: Flags) -> Self {
        Self(self.0 & !other.0)
    }

    /// The intersection of the two sets.
    pub const fn intersection(self, other: Flags) -> Self {
        Self(self.0 & other.0)
    }

    /// How many bits are set.
    pub const fn count(self) -> u32 {
        self.0.count_ones()
    }

    /// Reports whether no bit is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Names the bits this build knows, in declaration order.
    ///
    /// A bit this build does not name is left out; the raw value is what says
    /// whether it is set.
    pub fn names(self) -> Vec<&'static str> {
        let known = [
            (Self::READ, "read"),
            (Self::WRITE, "write"),
            (Self::EXEC, "exec"),
            (Self::ADMIN, "admin"),
        ];
        known
            .into_iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| name)
            .collect()
    }
}

impl fmt::Debug for Flags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.names()).finish()
    }
}

impl fmt::Display for Flags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names = self.names();
        if names.is_empty() {
            return formatter.write_str("none");
        }
        formatter.write_str(&names.join("+"))
    }
}

impl From<u64> for Flags {
    fn from(bits: u64) -> Self {
        Self::from_bits(bits)
    }
}

impl From<Flags> for u64 {
    fn from(flags: Flags) -> Self {
        flags.bits()
    }
}
