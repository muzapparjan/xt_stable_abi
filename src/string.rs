//! UTF-8 text in borrowed and owned forms.
//!
//! [`StableStr`] is a `(pointer, length)` view that owns nothing;
//! [`StableString`] owns a vector of bytes and carries its allocator. Both are
//! UTF-8 by construction, so a reader never has to validate bytes that were
//! produced by the other side.

use core::fmt;
use core::marker::PhantomData;

use crate::heap::string::String;
use crate::heap::borrow::ToOwned;

use crate::alloc::AbiAllocator;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::{StableElement, StableVec};

/// A borrowed UTF-8 string.
///
/// The handle owns nothing and borrows the bytes for `'a`, so a handle built by
/// a safe constructor cannot outlive the text it points at. Bytes that arrive
/// from another build carry no such tie; they are rebuilt with
/// [`StableStr::from_raw_parts`], which is `unsafe` for exactly that reason.
#[repr(C)]
pub struct StableStr<'a> {
    pointer: *const u8,
    length: u64,
    text: PhantomData<&'a str>,
}

impl<'a> StableStr<'a> {
    /// The empty string.
    pub const fn empty() -> Self {
        Self {
            pointer: core::ptr::NonNull::<u8>::dangling().as_ptr(),
            length: 0,
            text: PhantomData,
        }
    }

    /// Borrows `text` for as long as the text lives.
    pub const fn from_str(text: &'a str) -> Self {
        Self {
            pointer: text.as_ptr(),
            length: text.len() as u64,
            text: PhantomData,
        }
    }

    /// Rebuilds a handle from its parts.
    ///
    /// # Safety
    ///
    /// `pointer` must point at `length` bytes that stay alive, unchanged and
    /// valid UTF-8 for `'a`.
    pub const unsafe fn from_raw_parts(pointer: *const u8, length: u64) -> Self {
        Self {
            pointer,
            length,
            text: PhantomData,
        }
    }

    /// Rebuilds a handle from parts that arrived from another build, checking
    /// that the declared length fits the buffer and that the bytes are UTF-8.
    ///
    /// `available` is the number of bytes the receiver actually holds. A null
    /// pointer, or a length whose byte count is not addressable, is rejected as
    /// well: `str::from_utf8_unchecked` requires an aligned, non-null address
    /// even for an empty value.
    ///
    /// # Safety
    ///
    /// `pointer` must point at `available` valid bytes that stay alive and
    /// unchanged for `'a`.
    pub unsafe fn from_raw_parts_checked(
        pointer: *const u8,
        length: u64,
        available: u64,
    ) -> AbiResult<Self> {
        let handle = Self {
            pointer,
            length,
            text: PhantomData,
        };
        handle.validate_header()?;
        if length > available {
            return Err(AbiError::new(AbiStatus::InvalidDescriptor));
        }
        // SAFETY: the caller guarantees the bytes are readable.
        let bytes = unsafe { core::slice::from_raw_parts(pointer, length as usize) };
        if core::str::from_utf8(bytes).is_err() {
            return Err(AbiError::new(AbiStatus::InvalidUtf8));
        }
        Ok(handle)
    }

    /// Checks the header without reading the bytes.
    ///
    /// A null pointer rejects the handle, and so does a length whose byte count
    /// is not addressable: `length as usize` on a narrower target would silently
    /// cut the text at a byte that may be inside a character.
    pub const fn validate_header(&self) -> AbiResult<()> {
        if self.pointer.is_null() {
            return Err(AbiError::new(AbiStatus::NullPointer));
        }
        if self.length > isize::MAX as u64 {
            return Err(AbiError::new(AbiStatus::CapacityOverflow));
        }
        Ok(())
    }

    /// The address of the first byte.
    pub const fn as_ptr(&self) -> *const u8 {
        self.pointer
    }

    /// Checks that the bytes this handle names are UTF-8.
    ///
    /// This is the check a receiver runs on a handle built by another build:
    /// [`StableStr::as_str`] trusts the handle, so the bytes have to be checked
    /// before it is read.
    pub fn validate_text(&self) -> AbiResult<()> {
        self.validate_header()?;
        match core::str::from_utf8(self.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(AbiError::new(AbiStatus::InvalidUtf8)),
        }
    }

    /// The length in bytes.
    pub const fn len(&self) -> u64 {
        self.length
    }

    /// Reports whether the string is empty.
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// The borrowed text.
    pub fn as_str(&self) -> &str {
        // SAFETY: the constructor guarantees the bytes are valid UTF-8 for as
        // long as this handle exists.
        unsafe {
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                self.pointer,
                self.length as usize,
            ))
        }
    }

    /// The borrowed bytes.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: the constructor guarantees the bytes are alive.
        unsafe { core::slice::from_raw_parts(self.pointer, self.length as usize) }
    }

    /// The borrowed text, with a lifetime chosen by the caller.
    ///
    /// # Safety
    ///
    /// The bytes must still be alive, unchanged and valid UTF-8 for `'b`.
    pub unsafe fn as_str_unbounded<'b>(&self) -> &'b str {
        // SAFETY: the caller guarantees validity.
        unsafe {
            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                self.pointer,
                self.length as usize,
            ))
        }
    }

    /// Copies the text into an owned string.
    pub fn to_string(&self) -> AbiResult<StableString> {
        self.to_string_with_allocator(AbiAllocator::system())
    }

    /// Copies the text into an owned string that uses `allocator`.
    pub fn to_string_with_allocator(&self, allocator: AbiAllocator) -> AbiResult<StableString> {
        StableString::from_str_with_allocator(allocator, self.as_str())
    }
}

impl<'a> Clone for StableStr<'a> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a> Copy for StableStr<'a> {}

// SAFETY: the handle is a shared view of immutable bytes borrowed for `'a`, so
// moving or sharing it moves or shares a `&'a str`.
unsafe impl<'a> Send for StableStr<'a> {}
unsafe impl<'a> Sync for StableStr<'a> {}

// SAFETY: the handle has the layout of a pointer, a length and a marker, and
// owns nothing.
unsafe impl<'a> StableElement for StableStr<'a> {
    unsafe fn drop_in_place(_ptr: *mut Self) {}
}

impl<'a> fmt::Debug for StableStr<'a> {
    // The handle often carries text the other side wrote, so the debug form
    // reports its shape rather than its contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableStr")
            .field("len", &self.length)
            .finish()
    }
}

impl<'a> fmt::Display for StableStr<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'a> core::hash::Hash for StableStr<'a> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl<'a> PartialOrd for StableStr<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> Ord for StableStr<'a> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl<'a> PartialEq for StableStr<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl<'a> Eq for StableStr<'a> {}

impl<'a> PartialEq<str> for StableStr<'a> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl<'a> PartialEq<&str> for StableStr<'a> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// An owned UTF-8 string.
///
/// The storage is a [`StableVec<u8>`], so the value carries the allocator that
/// produced it and stays valid UTF-8 through every mutation this type exposes.
#[repr(C)]
pub struct StableString {
    bytes: StableVec<u8>,
}

impl StableString {
    /// The empty string, using the process allocator.
    pub const fn new() -> Self {
        Self {
            bytes: StableVec::new(),
        }
    }

    /// The empty string, using `allocator`.
    pub const fn new_with_allocator(allocator: AbiAllocator) -> Self {
        Self {
            bytes: StableVec::new_with_allocator(allocator),
        }
    }

    /// Copies `text`, using the process allocator.
    // The trait is implemented below; this constructor exists as an inherent
    // method so callers do not need to import the trait to build a value.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> AbiResult<Self> {
        Self::from_str_with_allocator(AbiAllocator::system(), text)
    }

    /// Copies `text`, using `allocator`.
    pub fn from_str_with_allocator(allocator: AbiAllocator, text: &str) -> AbiResult<Self> {
        let mut bytes = StableVec::with_capacity_and_allocator(allocator, text.len() as u64)?;
        bytes.try_extend_from_slice(text.as_bytes())?;
        Ok(Self { bytes })
    }

    /// Takes ownership of `bytes`, which must be valid UTF-8.
    ///
    /// # Safety
    ///
    /// `bytes` must contain valid UTF-8.
    pub unsafe fn from_utf8_unchecked(bytes: StableVec<u8>) -> Self {
        Self { bytes }
    }

    /// Takes ownership of `bytes` after validating them.
    ///
    /// On failure the input is released and the status is
    /// [`AbiStatus::InvalidUtf8`].
    pub fn from_utf8(bytes: StableVec<u8>) -> AbiResult<Self> {
        match core::str::from_utf8(bytes.as_slice()) {
            Ok(_) => Ok(Self { bytes }),
            Err(_) => Err(AbiError::new(AbiStatus::InvalidUtf8)),
        }
    }

    /// Checks the header and that the bytes are UTF-8.
    ///
    /// This is the check a receiver runs on a value built by another build. The
    /// header is checked first, so the bytes are only read once the length is
    /// known to be addressable.
    pub fn validate(&self) -> AbiResult<()> {
        self.bytes.validate_header()?;
        match core::str::from_utf8(self.as_bytes()) {
            Ok(_) => Ok(()),
            Err(_) => Err(AbiError::new(AbiStatus::InvalidUtf8)),
        }
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        // SAFETY: every constructor and mutation preserves UTF-8.
        unsafe { core::str::from_utf8_unchecked(self.bytes.as_slice()) }
    }

    /// The bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// The text as a borrowed handle, tied to this string.
    pub fn as_stable_str(&self) -> StableStr<'_> {
        StableStr::from_str(self.as_str())
    }

    /// The allocator that owns the storage.
    pub const fn allocator(&self) -> AbiAllocator {
        self.bytes.allocator()
    }

    /// The length in bytes.
    pub const fn len(&self) -> u64 {
        self.bytes.len()
    }

    /// Reports whether the string is empty.
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The number of bytes the storage can hold without reallocating.
    pub const fn capacity(&self) -> u64 {
        self.bytes.capacity()
    }

    /// Appends `text`.
    pub fn push_str(&mut self, text: &str) -> AbiResult<()> {
        self.bytes.try_extend_from_slice(text.as_bytes())
    }

    /// Appends one character.
    pub fn push(&mut self, character: char) -> AbiResult<()> {
        let mut buffer = [0u8; 4];
        self.bytes.try_extend_from_slice(character.encode_utf8(&mut buffer).as_bytes())
    }

    /// Removes every character.
    pub fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Shortens the string to `new_length` bytes.
    ///
    /// Returns [`AbiStatus::InvalidArgument`] when the requested length is not
    /// a character boundary, and leaves the string unchanged in that case.
    pub fn truncate(&mut self, new_length: u64) -> AbiResult<()> {
        if new_length > self.len() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        if !self.as_str().is_char_boundary(new_length as usize) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        self.bytes.truncate(new_length);
        Ok(())
    }

    /// Releases the text and hands back the byte storage.
    pub fn into_bytes(self) -> StableVec<u8> {
        self.bytes
    }

    /// Splits the text at `at`: this value keeps the part before it and the
    /// answer holds the rest, in its own block.
    ///
    /// Returns [`AbiStatus::InvalidArgument`] when `at` is past the end or is not
    /// a character boundary, and leaves the string unchanged in that case. A
    /// block for the tail that cannot be allocated is reported as well, and
    /// leaves the string unchanged too.
    pub fn split_off(&mut self, at: u64) -> AbiResult<Self> {
        if at > self.len() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        if !self.as_str().is_char_boundary(at as usize) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        // A split at either end moves no bytes: the empty side is created with
        // no block, and the side that keeps the text keeps its storage.
        if at == 0 {
            let mut moved = Self {
                bytes: StableVec::new_with_allocator(self.allocator()),
            };
            core::mem::swap(&mut moved, self);
            return Ok(moved);
        }
        if at == self.len() {
            return Ok(Self {
                bytes: StableVec::new_with_allocator(self.allocator()),
            });
        }
        // The tail gets its own block, so this is the one step that can fail;
        // a value that reports a failure keeps the text it had.
        let tail = self.bytes.try_split_off(at)?;
        Ok(Self { bytes: tail })
    }

    /// The text repeated `count` times.
    ///
    /// # Errors
    ///
    /// Reports a length that the storage cannot address.
    pub fn repeat(&self, count: u64) -> AbiResult<Self> {
        // Repeating either nothing or an empty text produces an empty text in
        // no steps: a loop that copies nothing `count` times costs the caller
        // the count and produces the same answer.
        if count == 0 || self.is_empty() {
            return Ok(Self {
                bytes: StableVec::with_capacity_and_allocator(self.allocator(), 0)?,
            });
        }
        let total = match self.len().checked_mul(count) {
            Some(total) => total,
            None => return Err(AbiError::new(AbiStatus::CapacityOverflow)),
        };
        let mut bytes =
            StableVec::with_capacity_and_allocator(self.allocator(), total)?;
        bytes.try_extend_from_slice(self.as_bytes())?;
        // Each pass copies what is already there onto the end of it, so the
        // filled part doubles: `log2(count)` copies instead of `count`.
        let mut filled = self.len();
        while filled < total {
            let take = filled.min(total - filled);
            // SAFETY: the block has room for `total` bytes, the first `filled`
            // are initialised, and `take <= filled`, so the source is inside the
            // initialised part and the destination follows it without overlap.
            unsafe {
                let source = bytes.as_ptr();
                let destination = bytes.spare_capacity_mut().as_mut_ptr().cast::<u8>();
                core::ptr::copy_nonoverlapping(source, destination, take as usize);
                bytes.set_len(filled + take);
            }
            filled += take;
        }
        Ok(Self { bytes })
    }

    /// The text with every occurrence of `from` replaced by `to`.
    ///
    /// An empty `from` is refused: it would match at every boundary and produce a
    /// text with no relation to the one this value holds.
    pub fn replace(&self, from: &str, to: &str) -> AbiResult<Self> {
        if from.is_empty() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let mut bytes = StableVec::with_capacity_and_allocator(self.allocator(), self.len())?;
        let mut rest = self.as_str();
        while let Some(position) = rest.find(from) {
            bytes.try_extend_from_slice(&rest.as_bytes()[..position])?;
            bytes.try_extend_from_slice(to.as_bytes())?;
            rest = &rest[position + from.len()..];
        }
        bytes.try_extend_from_slice(rest.as_bytes())?;
        Ok(Self { bytes })
    }

    /// Copies the text into a `String` on this side of the boundary.
    pub fn into_string(self) -> String {
        // The copy uses the process allocator, which is the only one a `String`
        // can be released through.
        self.as_str().to_owned()
    }

    /// Keeps only the characters for which `keep` answers true.
    ///
    /// The predicate sees the characters in order. The new text is built beside
    /// the old one and replaces it only when every byte has been written, so a
    /// failure to allocate leaves this value as it was.
    pub fn retain<F: FnMut(char) -> bool>(&mut self, mut keep: F) -> AbiResult<()> {
        let mut bytes = StableVec::with_capacity_and_allocator(self.allocator(), self.len())?;
        for character in self.as_str().chars() {
            if keep(character) {
                let mut buffer = [0u8; 4];
                bytes.try_extend_from_slice(character.encode_utf8(&mut buffer).as_bytes())?;
            }
        }
        self.bytes = bytes;
        Ok(())
    }

    /// Removes the bytes in `start..end` and answers with them as text.
    ///
    /// Both ends have to be character boundaries: a range that cuts a character
    /// in half is refused, because the two halves would not be text. The removed
    /// part is copied out first, so a failure leaves this value as it was; once
    /// it is copied, the rest of the text moves down inside the block this value
    /// already owns, which allocates nothing.
    pub fn drain(&mut self, start: u64, end: u64) -> AbiResult<Self> {
        if start > end || end > self.len() {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let text = self.as_str();
        if !text.is_char_boundary(start as usize) || !text.is_char_boundary(end as usize) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let mut removed = Self::new_with_allocator(self.allocator());
        // An empty range removes nothing: no copy is made and this value keeps
        // the storage it already has.
        if start == end {
            return Ok(removed);
        }
        removed
            .bytes
            .try_extend_from_slice(&text.as_bytes()[start as usize..end as usize])?;
        // The removed bytes are out, so the value changes only now.
        self.bytes.drain_range(start, end);
        Ok(removed)
    }

    /// Clones the string, reporting allocation failure.
    pub fn try_clone(&self) -> AbiResult<Self> {
        let mut bytes = StableVec::with_capacity_and_allocator(self.allocator(), self.len())?;
        bytes.try_extend_from_slice(self.as_bytes())?;
        Ok(Self { bytes })
    }

    /// The text, mutably.
    ///
    /// A `&mut str` keeps the value valid UTF-8, because the methods it offers
    /// replace characters rather than bytes.
    pub fn as_mut_str(&mut self) -> &mut str {
        // SAFETY: every mutation through this type keeps the bytes valid UTF-8,
        // and a `&mut str` can only change them in ways that preserve that.
        unsafe { core::str::from_utf8_unchecked_mut(self.bytes.as_mut_slice()) }
    }

    /// The characters of the text.
    pub fn chars(&self) -> core::str::Chars<'_> {
        self.as_str().chars()
    }

    /// The byte offset of the first occurrence of `pattern`.
    pub fn find(&self, pattern: &str) -> Option<u64> {
        self.as_str().find(pattern).map(|index| index as u64)
    }

    /// Inserts `character` before the byte at `index`.
    ///
    /// Returns [`AbiStatus::InvalidArgument`] when `index` falls inside a
    /// character.
    pub fn insert(&mut self, index: u64, character: char) -> AbiResult<()> {
        if index > self.len() || !self.as_str().is_char_boundary(index as usize) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        let mut buffer = [0u8; 4];
        let encoded = character.encode_utf8(&mut buffer);
        // One shift for the whole encoded character, not one per byte.
        self.bytes.try_insert_copy(index, encoded.as_bytes())
    }

    /// Removes the character that starts at `index`.
    ///
    /// Returns [`AbiStatus::InvalidArgument`] when `index` is past the end or
    /// falls inside a character.
    pub fn remove(&mut self, index: u64) -> AbiResult<char> {
        if index >= self.len() || !self.as_str().is_char_boundary(index as usize) {
            return Err(AbiError::new(AbiStatus::InvalidArgument));
        }
        // SAFETY: the index is a character boundary below the length, so a
        // character starts there.
        let character = unsafe { self.as_str().get_unchecked(index as usize..) }
            .chars()
            .next()
            .ok_or_else(|| AbiError::new(AbiStatus::InvalidArgument))?;
        self.remove_bytes(index, character.len_utf8() as u64);
        Ok(character)
    }

    /// Removes and returns the last character.
    pub fn pop(&mut self) -> Option<char> {
        let character = self.as_str().chars().next_back()?;
        let start = self.len() - character.len_utf8() as u64;
        self.remove_bytes(start, character.len_utf8() as u64);
        Some(character)
    }

    /// Removes `count` bytes starting at `start`, which the caller has checked
    /// to be a character boundary pair.
    fn remove_bytes(&mut self, start: u64, count: u64) {
        // One shift for the whole character, not one per byte.
        self.bytes.remove_copy_range(start, start + count);
    }

    /// Overwrites the text with zeros before releasing it.
    ///
    /// The whole block is overwritten, including the bytes past the length that
    /// an earlier `truncate`, `pop` or `remove` left behind. The storage is
    /// still released through the allocator the value carries; only its
    /// contents change.
    ///
    /// # Errors
    ///
    /// [`AbiStatus::CapacityOverflow`] when the byte count of the block cannot
    /// be computed, which a value built by this crate cannot produce.
    pub fn wipe(self) -> AbiResult<()> {
        let capacity = self.bytes.capacity();
        if capacity > 0 {
            // SAFETY: the block holds `capacity` writable bytes for its whole
            // length, whether or not they are inside the current text.
            unsafe {
                crate::zeroize::volatile_zero_raw(self.bytes.storage_ptr(), capacity, 1)?;
            }
        }
        Ok(())
    }
}

impl Default for StableString {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::zeroize::Wipe for StableString {
    fn wipe_in_place(&mut self) {
        let capacity = self.bytes.capacity();
        if capacity > 0 {
            // SAFETY: the block holds `capacity` writable bytes for its whole
            // length, whether or not they are inside the current text.
            let _ = unsafe {
                crate::zeroize::volatile_zero_raw(self.bytes.storage_ptr(), capacity, 1)
            };
        }
    }
}

impl Clone for StableString {
    fn clone(&self) -> Self {
        match self.try_clone() {
            Ok(clone) => clone,
            Err(error) => panic!("failed to clone a stable string: {error}"),
        }
    }
}

impl PartialEq for StableString {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for StableString {}

impl PartialEq<str> for StableString {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for StableString {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl core::str::FromStr for StableString {
    type Err = AbiError;

    fn from_str(text: &str) -> AbiResult<Self> {
        Self::from_str_with_allocator(AbiAllocator::system(), text)
    }
}

impl TryFrom<&str> for StableString {
    type Error = AbiError;

    fn try_from(text: &str) -> AbiResult<Self> {
        Self::from_str(text)
    }
}

impl TryFrom<String> for StableString {
    type Error = AbiError;

    fn try_from(text: String) -> AbiResult<Self> {
        Self::from_str(&text)
    }
}

impl From<StableString> for String {
    fn from(text: StableString) -> Self {
        text.into_string()
    }
}

impl fmt::Debug for StableString {
    // The value often holds text the other side wrote, so the debug form reports
    // its shape rather than its contents.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableString")
            .field("len", &self.len())
            .finish()
    }
}

impl fmt::Display for StableString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl core::ops::Deref for StableString {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl core::hash::Hash for StableString {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl PartialOrd for StableString {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StableString {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

// SAFETY: the string owns a byte vector with the layout of `StableVec<u8>` and
// exposes no way to break the UTF-8 invariant.
unsafe impl StableElement for StableString {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<'a> crate::clone::TryClone for StableStr<'a> {
    /// Names the same storage: a borrowed view owns nothing, so its copy is
    /// another view of what the caller lent.
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(*self)
    }
}
