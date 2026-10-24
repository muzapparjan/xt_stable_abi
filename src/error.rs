//! Status codes and the rich error value that crosses the boundary.
//!
//! Boundary functions return a `u32` code, the value of an [`AbiStatus`]. A code
//! this build does not name is still a value of the type: [`AbiStatus::from_raw`]
//! keeps it, [`AbiStatus::from_code`] answers `None` for it, and an [`AbiError`]
//! carries it unchanged, so a failure reported by a build that knows more
//! statuses survives the trip. When a caller wants a human-readable reason as
//! well, it reads the [`AbiError`], which pairs a code with an owned, UTF-8
//! message. The message is allocated through the allocator carried by the
//! message itself, so whichever side drops it frees it correctly.

use core::fmt;

use crate::string::StableString;

/// The outcome of a boundary call.
///
/// The value is a `u32` code with a name attached, not an enum: a code written
/// by a build that knows more statuses than this one is still a `u32` here, so
/// reading it is never undefined behaviour and every code is a value of this
/// type ([`AbiStatus::from_raw`]). What a code means is the next question, and
/// [`AbiStatus::from_code`] answers it with `None` when this build has no name
/// for the code. The codes themselves are part of the ABI and are never
/// renumbered.
#[repr(transparent)]
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AbiStatus(u32);

impl AbiStatus {
    // The names mirror the variants the type had when it was an enum, so a
    // caller reads `AbiStatus::InvalidTag` at every use site.
    #![allow(non_upper_case_globals)]

    /// The call succeeded.
    pub const Ok: AbiStatus = AbiStatus(0);
    /// A pointer argument was null where a value was required.
    pub const NullPointer: AbiStatus = AbiStatus(1);
    /// A byte sequence was not valid UTF-8.
    pub const InvalidUtf8: AbiStatus = AbiStatus(2);
    /// A tag byte held a value outside the set the type defines.
    pub const InvalidTag: AbiStatus = AbiStatus(3);
    /// A size/alignment pair could not describe a real allocation.
    pub const InvalidLayout: AbiStatus = AbiStatus(4);
    /// The allocator could not satisfy a request.
    pub const OutOfMemory: AbiStatus = AbiStatus(5);
    /// A length computation would exceed the addressable range.
    pub const CapacityOverflow: AbiStatus = AbiStatus(6);
    /// An argument was outside the domain the call accepts.
    pub const InvalidArgument: AbiStatus = AbiStatus(7);
    /// A handle did not name a live entry.
    pub const InvalidHandle: AbiStatus = AbiStatus(8);
    /// The two sides do not implement a common version.
    pub const VersionMismatch: AbiStatus = AbiStatus(9);
    /// A descriptor was malformed or did not fit the provided buffer.
    pub const InvalidDescriptor: AbiStatus = AbiStatus(10);
    /// A Rust panic was caught at the boundary.
    pub const Panic: AbiStatus = AbiStatus(11);

    /// Every status the crate defines, in code order.
    ///
    /// The list, the codes and the lookup are checked against each other: the
    /// build asserts that each entry sits at its own index and that the lookup
    /// answers it, and a test walks the codes the lookup names and looks each one
    /// up in the list. A code added to one and not the others therefore fails
    /// instead of going unnoticed.
    pub const ALL: [AbiStatus; 12] = [
        AbiStatus::Ok,
        AbiStatus::NullPointer,
        AbiStatus::InvalidUtf8,
        AbiStatus::InvalidTag,
        AbiStatus::InvalidLayout,
        AbiStatus::OutOfMemory,
        AbiStatus::CapacityOverflow,
        AbiStatus::InvalidArgument,
        AbiStatus::InvalidHandle,
        AbiStatus::VersionMismatch,
        AbiStatus::InvalidDescriptor,
        AbiStatus::Panic,
    ];

    /// Reports whether this is [`AbiStatus::Ok`].
    pub const fn is_ok(self) -> bool {
        matches!(self, AbiStatus::Ok)
    }

    /// Reports whether this is not [`AbiStatus::Ok`].
    pub const fn is_err(self) -> bool {
        !self.is_ok()
    }

    /// The code written into memory.
    pub const fn code(self) -> u32 {
        self.0
    }

    /// The status a code names, whether or not this build defines it.
    ///
    /// [`AbiStatus::from_code`] answers `None` for a code a later build added;
    /// this form keeps the code, so a value that arrived from that build is
    /// still a status this one can carry, log and hand back. The two differ only
    /// in the question they answer: whether the name is known here, not whether
    /// the value is one of this type.
    pub const fn from_raw(code: u32) -> Self {
        Self(code)
    }

    /// Rebuilds a status from its discriminant.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(AbiStatus::Ok),
            1 => Some(AbiStatus::NullPointer),
            2 => Some(AbiStatus::InvalidUtf8),
            3 => Some(AbiStatus::InvalidTag),
            4 => Some(AbiStatus::InvalidLayout),
            5 => Some(AbiStatus::OutOfMemory),
            6 => Some(AbiStatus::CapacityOverflow),
            7 => Some(AbiStatus::InvalidArgument),
            8 => Some(AbiStatus::InvalidHandle),
            9 => Some(AbiStatus::VersionMismatch),
            10 => Some(AbiStatus::InvalidDescriptor),
            11 => Some(AbiStatus::Panic),
            _ => None,
        }
    }

    /// A short, stable description of the code.
    pub const fn message(self) -> &'static str {
        match self.0 {
            0 => "ok",
            1 => "null pointer",
            2 => "invalid UTF-8",
            3 => "invalid tag",
            4 => "invalid layout",
            5 => "out of memory",
            6 => "capacity overflow",
            7 => "invalid argument",
            8 => "invalid handle",
            9 => "version mismatch",
            10 => "invalid descriptor",
            11 => "panic",
            _ => "status this build does not define",
        }
    }
}

impl fmt::Display for AbiStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

// The list, the codes and the lookup have to agree; a code added to one and not
// the others fails the build here rather than in a caller of `from_code`.
const _: () = {
    let mut index = 0;
    while index < AbiStatus::ALL.len() {
        assert!(AbiStatus::ALL[index].code() == index as u32);
        index += 1;
    }
    let mut index = 0;
    while index < AbiStatus::ALL.len() {
        match AbiStatus::from_code(AbiStatus::ALL[index].code()) {
            Some(found) => assert!(found.code() == index as u32),
            None => panic!("a code in AbiStatus::ALL is not known to from_code"),
        }
        index += 1;
    }
};

impl fmt::Debug for AbiStatus {
    // The name is the useful part; a code this build does not name is printed
    // as a number rather than guessed at. The arms are the names themselves
    // rather than the messages, because a message is prose for a reader and
    // this form is what a log or a test matches on.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.0 {
            0 => "Ok",
            1 => "NullPointer",
            2 => "InvalidUtf8",
            3 => "InvalidTag",
            4 => "InvalidLayout",
            5 => "OutOfMemory",
            6 => "CapacityOverflow",
            7 => "InvalidArgument",
            8 => "InvalidHandle",
            9 => "VersionMismatch",
            10 => "InvalidDescriptor",
            11 => "Panic",
            _ => return write!(formatter, "AbiStatus({})", self.0),
        };
        write!(formatter, "AbiStatus::{name}")
    }
}

#[cfg(feature = "std")]
impl std::error::Error for AbiStatus {}

/// A status code together with an owned explanation.
///
/// The explanation is empty when the producer only had a code to report, which
/// is the normal shape of an allocation failure: the failure path itself must
/// not allocate.
///
/// The code is stored as the `u32` it is written as, so a value produced by a
/// build with a code this one does not know is readable rather than undefined.
/// [`AbiError::status`] answers a status carrying that same code - it is a number
/// with names for some of them - and [`AbiError::raw_status`] answers the number
/// without the type, which is what a caller that logs or forwards it uses.
///
/// An error is not a key of a container: it has no `Hash` and no order, because
/// the text beside the code is prose, and two errors that report the same
/// failure may word it differently. Callers that need to group failures use the
/// code.
#[repr(C)]
pub struct AbiError {
    status: u32,
    message: StableString,
}

impl AbiError {
    /// Builds an error that carries only a code.
    pub const fn new(status: AbiStatus) -> Self {
        Self {
            status: status.code(),
            message: StableString::new(),
        }
    }

    /// Builds an error with a message.
    ///
    /// When the message cannot be allocated the code alone is returned: the
    /// code is the part callers dispatch on, the text is advisory.
    pub fn with_message(status: AbiStatus, message: &str) -> Self {
        match StableString::from_str(message) {
            Ok(text) => Self {
                status: status.code(),
                message: text,
            },
            Err(_) => Self::new(status),
        }
    }

    /// Overwrites the message with zeros before releasing it.
    ///
    /// Returns the status code, which is what a caller still has after the
    /// text is gone. A message that could not be wiped completely reports
    /// [`AbiStatus::CapacityOverflow`] instead of silently skipping the
    /// overwrite.
    pub fn wipe(self) -> AbiStatus {
        let status = self.status();
        match self.message.wipe() {
            Ok(()) => status,
            Err(_) => AbiStatus::CapacityOverflow,
        }
    }

    /// Builds an error with a message that was already copied.
    ///
    /// This is the form a caller uses when the text is produced as a boundary
    /// string in the first place, so it is not copied a second time.
    pub const fn with_owned_message(status: AbiStatus, message: StableString) -> Self {
        Self {
            status: status.code(),
            message,
        }
    }

    /// The status the library reported.
    ///
    /// A code this build does not name is kept as it was stored rather than
    /// replaced by one it knows: the failure happened in a build that has more
    /// to say about it, and inventing a different status here would lose what
    /// the library said. [`.code()`](AbiStatus::code) answers the number either
    /// way, and [`AbiError::raw_status`] is the same number without the type.
    pub const fn status(&self) -> AbiStatus {
        AbiStatus::from_raw(self.status)
    }

    /// The code as it was stored, whether or not this build defines it.
    ///
    /// This is the number [`AbiError::status`] carries, for a caller that logs
    /// or forwards the code as a `u32` rather than as a status value.
    pub const fn raw_status(&self) -> u32 {
        self.status
    }

    /// The explanation, empty when none was recorded.
    pub fn message(&self) -> &str {
        self.message.as_str()
    }

    /// Consumes the error, returning its message.
    pub fn into_message(self) -> StableString {
        self.message
    }

    /// Reports whether the code equals `status`.
    pub fn is(&self, status: AbiStatus) -> bool {
        self.status() == status
    }

    /// Copies the error, reporting allocation failure.
    pub fn try_clone(&self) -> crate::error::AbiResult<Self> {
        Ok(Self {
            status: self.status,
            message: self.message.try_clone()?,
        })
    }
}

impl Clone for AbiError {
    fn clone(&self) -> Self {
        match self.try_clone() {
            Ok(clone) => clone,
            Err(error) => panic!("failed to clone a stable error: {error}"),
        }
    }
}

impl fmt::Display for AbiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.message.is_empty() {
            self.status().fmt(formatter)
        } else {
            write!(formatter, "{}: {}", self.status(), self.message())
        }
    }
}

impl fmt::Debug for AbiError {
    // The message can carry text the producing side wrote, which may be content
    // a log must not repeat; the length and the code say what a reader usually
    // needs. `Display` still prints the message, because that is the form a
    // caller asks for on purpose.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AbiError")
            .field("status", &self.status)
            .field("message_len", &self.message.len())
            .finish()
    }
}

#[cfg(feature = "std")]
impl std::error::Error for AbiError {
    // The failure itself is the code this value carries; there is no nested
    // error to point at, which is why `source` keeps the default answer.
}

impl PartialEq for AbiError {
    fn eq(&self, other: &Self) -> bool {
        self.status == other.status && self.message() == other.message()
    }
}

impl Eq for AbiError {}

impl From<AbiStatus> for AbiError {
    fn from(status: AbiStatus) -> Self {
        Self::new(status)
    }
}

impl From<AbiError> for AbiStatus {
    fn from(error: AbiError) -> Self {
        error.status()
    }
}

impl crate::zeroize::Wipe for AbiError {
    fn wipe_in_place(&mut self) {
        // The code is what a caller dispatches on and carries no content; the
        // message is the part that may hold text worth overwriting.
        self.message.wipe_in_place();
    }
}

impl crate::zeroize::Wipe for AbiStatus {
    /// Overwrites the code with the one that reports success.
    ///
    /// A status carries no content of its own, so a container of statuses can be
    /// wiped; the value it is left with is the one this type defines for "no
    /// failure", which is what a wiped slot should say.
    fn wipe_in_place(&mut self) {
        // The code for success is zero, and the write is volatile for the same
        // reason every other wipe here is: a plain store to memory that is about
        // to be released is the compiler's to remove.
        crate::zeroize::wipe_value(self);
    }
}

// SAFETY: the status is a `u32` with the same layout on both sides, and it owns
// nothing. Every bit pattern is a value of the type, so a value read from the
// sending side needs no conversion before it is used.
unsafe impl crate::vec::StableElement for AbiStatus {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

// SAFETY: the error is a `u32` status followed by a `StableString`, both with a
// layout this crate declares, and its drop releases the message through the
// allocator the message carries.
unsafe impl crate::vec::StableElement for AbiError {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

// SAFETY: the layout is declared by this crate. A code this build does not
// define is carried as the number it is rather than refused: the text beside it
// is what the receivers read, and refusing the value would lose both the code
// and the explanation when a library built against a later revision reports a
// failure this build has no name for.
unsafe impl crate::boundary::AbiType for AbiError {
    fn validate(&self) -> AbiResult<()> {
        // The message is read as text by `message()` and `Display`, so it has to
        // satisfy the same contract a returned `StableString` does. Checking
        // only the code would let text that is not UTF-8 reach safe code.
        self.message.validate()
    }
}

/// The result type used by fallible operations in this crate.
pub type AbiResult<T> = core::result::Result<T, AbiError>;
