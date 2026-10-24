//! File names and paths as bytes with an explicit encoding tag.
//!
//! An `OsStr` is a byte sequence whose meaning depends on the platform: on Unix
//! it is arbitrary bytes, on Windows it is WTF-8. The types here store the bytes
//! together with the encoding, so a receiver knows how to read them and can
//! refuse a value it cannot rebuild on its own platform instead of guessing.

use core::fmt;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::{StableElement, StableSlice, StableVec};

/// The bytes are valid UTF-8.
pub const ENCODING_UTF8: u32 = 0;
/// The bytes are arbitrary; they mean a file name on a Unix-like system.
pub const ENCODING_UNIX: u32 = 1;
/// The bytes are WTF-8, the encoding `OsStr` uses on Windows.
pub const ENCODING_WTF8: u32 = 2;

fn is_encoding(encoding: u32) -> bool {
    matches!(encoding, ENCODING_UTF8 | ENCODING_UNIX | ENCODING_WTF8)
}

/// Reports whether `bytes` is a well-formed WTF-8 sequence.
///
/// WTF-8 is UTF-8 extended with the surrogate code points, which is what a
/// Windows file name may contain. A value produced by another build is checked
/// with this before it is handed to `OsStr::from_encoded_bytes_unchecked`.
fn is_wtf8(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let extra = match first {
            // A single ASCII byte is one whole sequence: it has no continuation
            // bytes, so nothing after it is read or required to be one.
            0x00..=0x7f => {
                index += 1;
                continue;
            }
            0xc2..=0xdf => 1,
            0xe0..=0xef => 2,
            0xf0..=0xf4 => 3,
            _ => return false,
        };
        if index + extra >= bytes.len() {
            return false;
        }
        let second = bytes[index + 1];
        // Continuation bytes all start with `10`.
        if second & 0xc0 != 0x80 {
            return false;
        }
        if extra >= 1 {
            // The first continuation byte carries the high bits of the code
            // point, which narrows the allowed range.
            let lowest = match first {
                0xe0 => 0xa0,
                0xf0 => 0x90,
                0xf4 => 0x80,
                _ => 0x80,
            };
            let highest = match first {
                0xed => 0xbf,
                0xf4 => 0x8f,
                _ => 0xbf,
            };
            if second < lowest || second > highest {
                return false;
            }
        }
        let mut offset = 1;
        while offset <= extra {
            let byte = bytes[index + offset];
            if byte & 0xc0 != 0x80 {
                return false;
            }
            offset += 1;
        }
        index += extra + 1;
    }
    true
}

/// The encoding a native name is stored in, together with its bytes.
fn encoded_parts(name: &OsStr) -> (u32, &[u8]) {
    let bytes = name.as_encoded_bytes();
    if core::str::from_utf8(bytes).is_ok() {
        (ENCODING_UTF8, bytes)
    } else if cfg!(windows) {
        (ENCODING_WTF8, bytes)
    } else {
        (ENCODING_UNIX, bytes)
    }
}

/// A borrowed file name.
#[repr(C)]
pub struct StableOsStr<'a> {
    bytes: StableSlice<'a, u8>,
    encoding: u32,
}

impl<'a> StableOsStr<'a> {
    /// A name that is already text.
    pub const fn from_str(text: &'a str) -> Self {
        Self {
            bytes: StableSlice::from_slice(text.as_bytes()),
            encoding: ENCODING_UTF8,
        }
    }

    /// A name from raw bytes and the encoding they follow.
    pub fn from_bytes(bytes: &'a [u8], encoding: u32) -> AbiResult<Self> {
        if !is_encoding(encoding) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        if encoding == ENCODING_UTF8 && core::str::from_utf8(bytes).is_err() {
            return Err(AbiError::new(AbiStatus::InvalidUtf8));
        }
        if encoding == ENCODING_WTF8 && !is_wtf8(bytes) {
            return Err(AbiError::new(AbiStatus::InvalidUtf8));
        }
        Ok(Self {
            bytes: StableSlice::from_slice(bytes),
            encoding,
        })
    }

    /// A name copied from a native one.
    pub fn from_os_str(name: &'a OsStr) -> Self {
        let (encoding, bytes) = encoded_parts(name);
        Self {
            bytes: StableSlice::from_slice(bytes),
            encoding,
        }
    }

    /// The bytes of the name.
    pub fn as_bytes(&self) -> &'a [u8] {
        // SAFETY: the handle names storage that lives for `'a`.
        unsafe { self.bytes.as_slice_unbounded() }
    }

    /// The encoding the bytes follow.
    pub const fn encoding(&self) -> u32 {
        self.encoding
    }

    /// The number of bytes.
    pub const fn len(&self) -> u64 {
        self.bytes.len()
    }

    /// Reports whether the name is empty.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The name as text, when the bytes are valid UTF-8.
    pub fn to_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.as_bytes()).ok()
    }

    /// Reports whether the value is one this type defines.
    ///
    /// The header is checked before the bytes are read, which is what makes this
    /// answer meaningful for a value that arrived from another build: a length
    /// this side cannot address is refused rather than turned into a slice that
    /// runs past the storage. [`StableOsStr::validate`] is the same answer with
    /// the reason for a refusal.
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }

    /// The native name, when the encoding is the one this platform uses.
    ///
    /// A Unix byte sequence cannot be turned into a Windows `OsStr`, and a
    /// WTF-8 sequence is only meaningful on Windows; both cases answer with
    /// [`AbiStatus::InvalidArgument`] rather than a lossy conversion.
    pub fn to_os_str(&self) -> AbiResult<&'a OsStr> {
        match self.encoding {
            ENCODING_UTF8 => match core::str::from_utf8(self.as_bytes()) {
                Ok(text) => Ok(OsStr::new(text)),
                Err(_) => Err(AbiError::new(AbiStatus::InvalidUtf8)),
            },
            ENCODING_UNIX => {
                if cfg!(unix) {
                    #[cfg(unix)]
                    {
                        use std::os::unix::ffi::OsStrExt;
                        Ok(OsStr::from_bytes(self.as_bytes()))
                    }
                    #[cfg(not(unix))]
                    {
                        Err(AbiError::new(AbiStatus::InvalidArgument))
                    }
                } else {
                    Err(AbiError::new(AbiStatus::InvalidArgument))
                }
            }
            ENCODING_WTF8 => {
                if !is_wtf8(self.as_bytes()) {
                    return Err(AbiError::new(AbiStatus::InvalidUtf8));
                }
                if cfg!(windows) {
                    // SAFETY: the bytes were checked to be well-formed WTF-8,
                    // which is the encoding a Windows `OsStr` holds.
                    Ok(unsafe { OsStr::from_encoded_bytes_unchecked(self.as_bytes()) })
                } else {
                    Err(AbiError::new(AbiStatus::InvalidArgument))
                }
            }
            _ => Err(AbiError::new(AbiStatus::InvalidArgument)),
        }
    }

    /// The name as an owned native value, replacing bytes that this platform
    /// cannot represent.
    ///
    /// Two different names can become the same text here - every byte a name
    /// holds that is not part of a character becomes the replacement character -
    /// so the result is for display. Compare names with equality, which looks at
    /// the bytes.
    pub fn to_os_string_lossy(&self) -> OsString {
        match core::str::from_utf8(self.as_bytes()) {
            Ok(text) => OsString::from(text),
            Err(_) => OsString::from(String::from_utf8_lossy(self.as_bytes()).into_owned()),
        }
    }

    /// Copies the name into an owned value that uses `allocator`.
    pub fn to_os_string_with_allocator(&self, allocator: AbiAllocator) -> AbiResult<StableOsString> {
        StableOsString::from_parts_with_allocator(allocator, self.as_bytes(), self.encoding)
    }
}

impl<'a> Clone for StableOsStr<'a> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a> Copy for StableOsStr<'a> {}

impl<'a> fmt::Debug for StableOsStr<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableOsStr")
            .field("len", &self.len())
            .field("encoding", &self.encoding)
            .finish()
    }
}

impl<'a> PartialEq for StableOsStr<'a> {
    fn eq(&self, other: &Self) -> bool {
        // Two names are the same when the bytes and the encoding they follow
        // are the same. Comparing through `to_os_string_lossy` would make
        // different WTF-8 names equal, because every unpaired surrogate is
        // replaced by the same character there.
        self.encoding == other.encoding && self.as_bytes() == other.as_bytes()
    }
}

impl<'a> Eq for StableOsStr<'a> {}

impl<'a> core::hash::Hash for StableOsStr<'a> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.encoding.hash(state);
        self.as_bytes().hash(state);
    }
}

impl<'a> PartialOrd for StableOsStr<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> Ord for StableOsStr<'a> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.encoding
            .cmp(&other.encoding)
            .then_with(|| self.as_bytes().cmp(other.as_bytes()))
    }
}

impl<'a> StableOsStr<'a> {
    /// Checks the encoding tag and that the bytes follow it.
    ///
    /// A tag this build does not define is rejected, and so are bytes that are
    /// not the encoding the tag names: a name is handed to
    /// `OsStr::from_encoded_bytes_unchecked`, which trusts exactly that.
    pub fn validate(&self) -> AbiResult<()> {
        self.bytes.validate_header()?;
        if !is_encoding(self.encoding) {
            return Err(AbiError::new(AbiStatus::InvalidTag));
        }
        if self.encoding == ENCODING_UTF8 && core::str::from_utf8(self.as_bytes()).is_err() {
            return Err(AbiError::new(AbiStatus::InvalidUtf8));
        }
        if self.encoding == ENCODING_WTF8 && !is_wtf8(self.as_bytes()) {
            return Err(AbiError::new(AbiStatus::InvalidUtf8));
        }
        Ok(())
    }
}

/// An owned file name.
#[repr(C)]
pub struct StableOsString {
    bytes: StableVec<u8>,
    encoding: u32,
}

impl StableOsString {
    /// An empty name on the process allocator.
    pub const fn new() -> Self {
        Self {
            bytes: StableVec::new(),
            encoding: ENCODING_UTF8,
        }
    }

    /// An empty name whose storage uses `allocator`.
    pub const fn new_with_allocator(allocator: AbiAllocator) -> Self {
        Self {
            bytes: StableVec::new_with_allocator(allocator),
            encoding: ENCODING_UTF8,
        }
    }

    /// The allocator that owns the storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.bytes.allocator()
    }

    /// A name copied from text.
    pub fn from_text(text: &str) -> AbiResult<Self> {
        Self::from_str_with_allocator(AbiAllocator::system(), text)
    }

    /// A name copied from text, using `allocator`.
    pub fn from_str_with_allocator(allocator: AbiAllocator, text: &str) -> AbiResult<Self> {
        Self::from_parts_with_allocator(allocator, text.as_bytes(), ENCODING_UTF8)
    }

    /// A name from raw bytes and the encoding they follow.
    pub fn from_parts_with_allocator(
        allocator: AbiAllocator,
        bytes: &[u8],
        encoding: u32,
    ) -> AbiResult<Self> {
        if !is_encoding(encoding) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        if encoding == ENCODING_UTF8 && core::str::from_utf8(bytes).is_err() {
            return Err(AbiError::new(AbiStatus::InvalidUtf8));
        }
        if encoding == ENCODING_WTF8 && !is_wtf8(bytes) {
            return Err(AbiError::new(AbiStatus::InvalidUtf8));
        }
        let mut owned = StableVec::with_capacity_and_allocator(allocator, bytes.len() as u64)?;
        owned.try_extend_from_slice(bytes)?;
        Ok(Self {
            bytes: owned,
            encoding,
        })
    }

    /// A name copied from a native one.
    pub fn from_os_str(name: &OsStr) -> AbiResult<Self> {
        Self::from_os_str_with_allocator(AbiAllocator::system(), name)
    }

    /// A name copied from a native one, using `allocator`.
    pub fn from_os_str_with_allocator(
        allocator: AbiAllocator,
        name: &OsStr,
    ) -> AbiResult<Self> {
        let (encoding, bytes) = encoded_parts(name);
        Self::from_parts_with_allocator(allocator, bytes, encoding)
    }

    /// The bytes of the name.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// The encoding the bytes follow.
    pub const fn encoding(&self) -> u32 {
        self.encoding
    }

    /// The number of bytes.
    pub const fn len(&self) -> u64 {
        self.bytes.len()
    }

    /// Reports whether the name is empty.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The name as a borrowed handle.
    pub fn as_stable_os_str(&self) -> StableOsStr<'_> {
        StableOsStr {
            bytes: StableSlice::from_slice(self.as_bytes()),
            encoding: self.encoding,
        }
    }

    /// The native name, borrowed from this value, when the encoding is the one
    /// this platform uses.
    ///
    /// A caller that only has to look at the name reads it here instead of
    /// copying it into a native value. An encoding this platform cannot use is
    /// refused rather than converted, for the reason
    /// [`StableOsStr::to_os_str`] gives.
    pub fn as_os_str(&self) -> AbiResult<&OsStr> {
        self.as_stable_os_str().to_os_str()
    }

    /// The name as text, when the bytes are valid UTF-8.
    pub fn to_str(&self) -> Option<&str> {
        core::str::from_utf8(self.as_bytes()).ok()
    }

    /// Rebuilds the native name, refusing an encoding this platform cannot use.
    pub fn to_os_string(&self) -> AbiResult<OsString> {
        Ok(self.as_stable_os_str().to_os_str()?.to_os_string())
    }

    /// The name as a native value, replacing bytes this platform cannot use.
    pub fn to_os_string_lossy(&self) -> OsString {
        self.as_stable_os_str().to_os_string_lossy()
    }

    /// Removes every byte.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.encoding = ENCODING_UTF8;
    }

    /// Appends `part` to the name.
    ///
    /// The two names have to follow one encoding: appending a byte sequence to
    /// another would otherwise produce a value whose encoding contradicts its
    /// bytes, and no reader could say which one to trust. An empty name takes the
    /// encoding of what is appended to it, which is how a name built from a
    /// native value keeps that value's encoding.
    pub fn push(&mut self, part: StableOsStr<'_>) -> AbiResult<()> {
        part.validate()?;
        if self.is_empty() {
            self.encoding = part.encoding();
        } else if self.encoding != part.encoding() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        self.bytes.try_extend_from_slice(part.as_bytes())
    }

    /// Copies the name.
    pub fn try_clone(&self) -> AbiResult<Self> {
        Self::from_parts_with_allocator(self.bytes.allocator(), self.as_bytes(), self.encoding)
    }

    /// Overwrites the whole block with zeros before releasing it.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::CapacityOverflow`] when the byte count of the block cannot
    /// be computed, which a value built by this crate cannot produce.
    pub fn wipe(mut self) -> AbiResult<()> {
        let bytes = core::mem::take(&mut self.bytes);
        bytes.wipe()
    }
}

impl Default for StableOsString {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::zeroize::Wipe for StableOsString {
    fn wipe_in_place(&mut self) {
        self.bytes.wipe_in_place();
    }
}

impl Clone for StableOsString {
    fn clone(&self) -> Self {
        self.try_clone().expect("clone a stable file name")
    }
}

impl fmt::Debug for StableOsString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableOsString")
            .field("len", &self.len())
            .field("encoding", &self.encoding)
            .finish()
    }
}

impl PartialEq for StableOsString {
    fn eq(&self, other: &Self) -> bool {
        self.as_stable_os_str() == other.as_stable_os_str()
    }
}

impl Eq for StableOsString {}

impl core::hash::Hash for StableOsString {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_stable_os_str().hash(state)
    }
}

impl PartialOrd for StableOsString {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StableOsString {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_stable_os_str().cmp(&other.as_stable_os_str())
    }
}

impl StableOsString {
    /// Checks that the storage describes a block, then the encoding tag and the
    /// bytes.
    ///
    /// The header comes first: the bytes are read through it, so a length past
    /// the capacity has to be refused before a slice is built over them. The
    /// borrowed form has no capacity to compare against, which is why the check
    /// on the owned value is the one that can make this decision.
    pub fn validate(&self) -> AbiResult<()> {
        self.bytes.validate_header()?;
        self.as_stable_os_str().validate()
    }
}

/// A borrowed path.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StablePath<'a> {
    value: StableOsStr<'a>,
}

impl<'a> StablePath<'a> {
    /// A path from text.
    pub const fn from_str(text: &'a str) -> Self {
        Self {
            value: StableOsStr::from_str(text),
        }
    }

    /// A path copied from a native one.
    pub fn from_path(path: &'a Path) -> Self {
        Self {
            value: StableOsStr::from_os_str(path.as_os_str()),
        }
    }

    /// The path as a file name handle.
    pub const fn as_stable_os_str(&self) -> StableOsStr<'a> {
        self.value
    }

    /// The native path, when every byte can be rebuilt here.
    pub fn as_path(&self) -> AbiResult<&'a Path> {
        Ok(Path::new(self.value.to_os_str()?))
    }

    /// The path as text, when it is valid UTF-8.
    pub fn to_str(&self) -> Option<&'a str> {
        self.value.to_str()
    }

    /// Copies the path into an owned value.
    pub fn to_path_buf(&self) -> AbiResult<StablePathBuf> {
        self.to_path_buf_with_allocator(AbiAllocator::system())
    }

    /// Copies the path into an owned value that uses `allocator`.
    pub fn to_path_buf_with_allocator(&self, allocator: AbiAllocator) -> AbiResult<StablePathBuf> {
        Ok(StablePathBuf {
            value: self.value.to_os_string_with_allocator(allocator)?,
        })
    }

    /// Reports whether the path names a place from its root.
    ///
    /// The answer is the one the platform gives: a path that begins with a
    /// separator on Unix, and one with a prefix on Windows. A path whose bytes
    /// cannot be rebuilt on this side names nothing, so it answers false.
    pub fn is_absolute(&self) -> bool {
        self.as_path().is_ok_and(Path::is_absolute)
    }

    /// The directory this path sits in, as a borrowed handle.
    pub fn parent(&self) -> Option<Self> {
        self.as_path()
            .ok()
            .and_then(Path::parent)
            .map(Self::from_path)
    }

    /// The last part of the path, as a file-name handle.
    pub fn file_name(&self) -> Option<StableOsStr<'a>> {
        self.as_path()
            .ok()
            .and_then(Path::file_name)
            .map(StableOsStr::from_os_str)
    }

    /// The extension of the last part of the path.
    pub fn extension(&self) -> Option<StableOsStr<'a>> {
        self.as_path()
            .ok()
            .and_then(Path::extension)
            .map(StableOsStr::from_os_str)
    }

    /// The path a visit to `part` from here names.
    pub fn join(&self, part: Self) -> AbiResult<StablePathBuf> {
        self.join_with_allocator(part, AbiAllocator::system())
    }

    /// The path a visit to `part` from here names, with the answer's storage
    /// taken from `allocator`.
    ///
    /// A borrowed path carries no allocator, so this is how a caller that works
    /// inside an arena builds the answer without leaving it.
    pub fn join_with_allocator(
        &self,
        part: Self,
        allocator: AbiAllocator,
    ) -> AbiResult<StablePathBuf> {
        let mut path = self.to_path_buf()?.to_path_buf()?;
        path.push(part.as_path()?);
        buffer_from_path(&path, allocator)
    }

    /// The path this one names with its extension replaced.
    pub fn with_extension(&self, extension: StableOsStr<'_>) -> AbiResult<StablePathBuf> {
        self.with_extension_with_allocator(extension, AbiAllocator::system())
    }

    /// The path this one names with its extension replaced, with the answer's
    /// storage taken from `allocator`.
    pub fn with_extension_with_allocator(
        &self,
        extension: StableOsStr<'_>,
        allocator: AbiAllocator,
    ) -> AbiResult<StablePathBuf> {
        let mut path = self.to_path_buf()?.to_path_buf()?;
        path.set_extension(extension.to_os_str()?);
        buffer_from_path(&path, allocator)
    }
}

/// Copies a native path into a buffer whose storage comes from `allocator`.
fn buffer_from_path(path: &Path, allocator: AbiAllocator) -> AbiResult<StablePathBuf> {
    Ok(StablePathBuf {
        value: StableOsString::from_os_str_with_allocator(allocator, path.as_os_str())?,
    })
}

impl<'a> fmt::Debug for StablePath<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("StablePath").field(&self.value).finish()
    }
}

impl<'a> PartialEq for StablePath<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<'a> Eq for StablePath<'a> {}

/// An owned path.
#[repr(C)]
pub struct StablePathBuf {
    value: StableOsString,
}

impl StablePathBuf {
    /// An empty path on the process allocator.
    pub const fn new() -> Self {
        Self {
            value: StableOsString::new(),
        }
    }

    /// An empty path whose storage uses `allocator`.
    pub const fn new_with_allocator(allocator: AbiAllocator) -> Self {
        Self {
            value: StableOsString::new_with_allocator(allocator),
        }
    }
    /// A path copied from text.
    pub fn from_text(text: &str) -> AbiResult<Self> {
        Ok(Self {
            value: StableOsString::from_text(text)?,
        })
    }

    /// The allocator that owns the storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.value.allocator()
    }

    /// A path copied from a native one.
    pub fn from_path(path: &Path) -> AbiResult<Self> {
        Self::from_path_with_allocator(AbiAllocator::system(), path)
    }

    /// A path copied from a native one, using `allocator`.
    pub fn from_path_with_allocator(allocator: AbiAllocator, path: &Path) -> AbiResult<Self> {
        Ok(Self {
            value: StableOsString::from_os_str_with_allocator(allocator, path.as_os_str())?,
        })
    }

    /// The path as a borrowed handle.
    pub fn as_stable_path(&self) -> StablePath<'_> {
        StablePath {
            value: self.value.as_stable_os_str(),
        }
    }

    /// Checks that the storage describes a block, then the encoding tag and the
    /// bytes.
    ///
    /// This is the name's check, reached through the owned value rather than
    /// through the borrowed view: the view has no capacity to compare the length
    /// against, so a check written on it cannot refuse a length past the block.
    pub fn validate(&self) -> AbiResult<()> {
        self.value.validate()
    }

    /// The native path, rebuilding it from the stored bytes.
    pub fn to_path_buf(&self) -> AbiResult<PathBuf> {
        Ok(PathBuf::from(self.value.to_os_string()?))
    }

    /// The path as text, when it is valid UTF-8.
    pub fn to_str(&self) -> Option<&str> {
        self.value.to_str()
    }

    /// Copies the path.
    pub fn try_clone(&self) -> AbiResult<Self> {
        Ok(Self {
            value: self.value.try_clone()?,
        })
    }

    /// Appends `part` to the path, the way a directory and a name meet.
    ///
    /// The storage is rebuilt through the allocator this value carries, so the
    /// result stays inside the allocation strategy it was built with.
    pub fn push(&mut self, part: StablePath<'_>) -> AbiResult<()> {
        let allocator = self.value.allocator();
        let mut path = self.to_path_buf()?;
        path.push(part.as_path()?);
        *self = Self {
            value: StableOsString::from_os_str_with_allocator(allocator, path.as_os_str())?,
        };
        Ok(())
    }

    /// The directory this path sits in, as a path of its own, or `None` when
    /// there is none.
    pub fn parent(&self) -> AbiResult<Option<Self>> {
        let allocator = self.value.allocator();
        let Some(parent) = self.to_path_buf()?.parent().map(Path::to_path_buf) else {
            return Ok(None);
        };
        Ok(Some(Self {
            value: StableOsString::from_os_str_with_allocator(allocator, parent.as_os_str())?,
        }))
    }

    /// The last part of the path.
    pub fn file_name(&self) -> AbiResult<Option<StableOsStr<'_>>> {
        Ok(self
            .as_stable_path()
            .as_path()?
            .file_name()
            .map(StableOsStr::from_os_str))
    }

    /// The extension of the last part of the path.
    pub fn extension(&self) -> AbiResult<Option<StableOsStr<'_>>> {
        Ok(self
            .as_stable_path()
            .as_path()?
            .extension()
            .map(StableOsStr::from_os_str))
    }

    /// The path a visit to `part` from here names, as a path of its own.
    pub fn join(&self, part: StablePath<'_>) -> AbiResult<Self> {
        let mut joined = self.try_clone()?;
        joined.push(part)?;
        Ok(joined)
    }

    /// Replaces the extension of the last part, answering whether there was one
    /// to replace.
    pub fn set_extension(&mut self, extension: StableOsStr<'_>) -> AbiResult<bool> {
        let allocator = self.value.allocator();
        let mut path = self.to_path_buf()?;
        let replaced = path.set_extension(extension.to_os_str()?);
        *self = Self {
            value: StableOsString::from_os_str_with_allocator(allocator, path.as_os_str())?,
        };
        Ok(replaced)
    }

    /// Removes the last part of the path, answering whether there was one.
    ///
    /// `Ok(false)` says the path had no part to remove; the path is unchanged.
    /// A failure to rebuild the shorter path - the bytes cannot be produced for
    /// this platform, or the block cannot be allocated - is reported as an
    /// error, because answering `false` would say the path had nothing to
    /// remove when it still has what it had.
    pub fn pop(&mut self) -> AbiResult<bool> {
        let allocator = self.value.allocator();
        let mut path = self.to_path_buf()?;
        if !path.pop() {
            return Ok(false);
        }
        *self = Self {
            value: StableOsString::from_os_str_with_allocator(allocator, path.as_os_str())?,
        };
        Ok(true)
    }

    /// Overwrites the whole block with zeros before releasing it.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::CapacityOverflow`] when the byte count of the block cannot
    /// be computed, which a value built by this crate cannot produce.
    pub fn wipe(mut self) -> AbiResult<()> {
        let value = core::mem::take(&mut self.value);
        value.wipe()
    }
}

impl Default for StablePathBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for StablePathBuf {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
        }
    }
}

impl fmt::Debug for StablePathBuf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StablePathBuf")
            .field(&self.value)
            .finish()
    }
}

impl PartialEq for StablePathBuf {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for StablePathBuf {}

impl crate::zeroize::Wipe for StablePathBuf {
    fn wipe_in_place(&mut self) {
        self.value.wipe_in_place();
    }
}

// SAFETY: the handle is a borrowed byte slice and a tag, and owns nothing.
unsafe impl<'a> StableElement for StableOsStr<'a> {
    unsafe fn drop_in_place(_ptr: *mut Self) {}
}

// SAFETY: the handle is an owned byte vector and a tag; dropping it releases
// exactly the block the vector owns.
unsafe impl StableElement for StableOsString {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

// SAFETY: the handle is a borrowed name and owns nothing.
unsafe impl<'a> StableElement for StablePath<'a> {
    unsafe fn drop_in_place(_ptr: *mut Self) {}
}

// SAFETY: the handle is an owned name; dropping it releases exactly that name.
unsafe impl StableElement for StablePathBuf {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl core::str::FromStr for StableOsString {
    type Err = AbiError;

    fn from_str(text: &str) -> AbiResult<Self> {
        Self::from_text(text)
    }
}

impl core::str::FromStr for StablePathBuf {
    type Err = AbiError;

    fn from_str(text: &str) -> AbiResult<Self> {
        Self::from_text(text)
    }
}

impl core::hash::Hash for StablePathBuf {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::hash::Hash::hash(&self.as_stable_path(), state)
    }
}

impl PartialOrd for StablePathBuf {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StablePathBuf {
    /// Orders the name the path holds, which is what the borrowed view compares.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_stable_path().cmp(&other.as_stable_path())
    }
}

impl<'a> core::hash::Hash for StablePath<'a> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::hash::Hash::hash(&self.as_stable_os_str(), state)
    }
}

impl<'a> PartialOrd for StablePath<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> Ord for StablePath<'a> {
    /// Orders the name the path holds, which is what the borrowed view compares.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_stable_os_str().cmp(&other.as_stable_os_str())
    }
}

impl<'a> crate::clone::TryClone for StablePath<'a> {
    /// Names the same storage: a borrowed view owns nothing, so its copy is
    /// another view of what the caller lent.
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(*self)
    }
}

impl<'a> crate::clone::TryClone for StableOsStr<'a> {
    /// Names the same storage: a borrowed view owns nothing, so its copy is
    /// another view of what the caller lent.
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(*self)
    }
}

impl TryFrom<&Path> for StablePathBuf {
    type Error = AbiError;

    fn try_from(path: &Path) -> AbiResult<Self> {
        Self::from_path(path)
    }
}

impl TryFrom<&OsStr> for StableOsString {
    type Error = AbiError;

    fn try_from(name: &OsStr) -> AbiResult<Self> {
        Self::from_os_str(name)
    }
}
