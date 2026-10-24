//! An optional value with an explicit tag.
//!
//! `Option<T>` uses a niche in `T` to encode `None`, and which niche it finds
//! depends on the compiler. [`StableOption`] always stores a `u32` tag followed
//! by the value, so the bytes are the same in every build.

use core::fmt;
use core::mem::{ManuallyDrop, MaybeUninit};

use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::StableElement;

/// The tag written for the empty variant.
pub const NONE_TAG: u32 = 0;
/// The tag written for the filled variant.
pub const SOME_TAG: u32 = 1;

/// An optional value.
///
/// The layout is a `u32` tag followed by a suitably aligned `T`. Both variants
/// occupy the same storage, so the size never depends on which variant is
/// active.
#[repr(C)]
pub struct StableOption<T> {
    tag: u32,
    value: MaybeUninit<T>,
}

impl<T> StableOption<T> {
    /// The empty variant.
    ///
    /// The payload storage is zeroed rather than left uninitialised, so an
    /// empty value does not carry the caller's stack contents across the
    /// boundary. The bytes between the tag and the payload are padding, which
    /// the compiler does not write; they are not part of the layout contract and
    /// must not be compared or persisted.
    pub const fn none() -> Self {
        Self {
            tag: NONE_TAG,
            value: MaybeUninit::zeroed(),
        }
    }

    /// The filled variant.
    pub fn some(value: T) -> Self {
        Self {
            tag: SOME_TAG,
            value: MaybeUninit::new(value),
        }
    }

    /// The raw tag, as written into memory.
    pub const fn tag(&self) -> u32 {
        self.tag
    }

    /// Reports whether a value is present.
    pub const fn is_some(&self) -> bool {
        self.tag == SOME_TAG
    }

    /// Reports whether the variant is empty.
    pub const fn is_none(&self) -> bool {
        self.tag == NONE_TAG
    }

    /// Rebuilds a value from a tag and a payload that arrived as bytes.
    ///
    /// This is the entry point a receiver uses: a tag outside the two this type
    /// defines is rejected instead of being read as one of the variants.
    ///
    /// The payload is taken only when the tag is [`SOME_TAG`]. For any other
    /// tag the bytes are not known to hold an initialised value (an empty value
    /// zeroes them), so they are neither read nor released.
    ///
    /// The payload is moved into this function and is not handed back, so a
    /// caller that knows it passed an initialised `T` with a tag other than
    /// [`SOME_TAG`] loses it. Pass an uninitialised or zeroed `MaybeUninit`
    /// unless the tag is [`SOME_TAG`]; a receiver reading foreign bytes owns no
    /// value there in the first place.
    ///
    /// # Safety
    ///
    /// When `tag` is [`SOME_TAG`], `value` must hold an initialised `T`.
    pub unsafe fn from_parts(tag: u32, value: MaybeUninit<T>) -> AbiResult<Self> {
        match tag {
            SOME_TAG => Ok(Self { tag, value }),
            NONE_TAG => Ok(Self::none()),
            _ => Err(AbiError::new(AbiStatus::InvalidTag)),
        }
    }

    /// Checks that the tag names a variant this type defines, and that the
    /// payload is a value the payload type defines when it is present.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        if self.is_none() {
            return Ok(());
        }
        if !self.is_some() {
            return Err(AbiError::new(AbiStatus::InvalidTag));
        }
        // SAFETY: the tag says the value was initialised.
        let value = unsafe { self.value.assume_init_ref() };
        crate::boundary::check(value)
    }

    /// Borrows the value when present.
    pub fn as_ref(&self) -> Option<&T> {
        if self.is_some() {
            // SAFETY: the tag says the value was initialised.
            Some(unsafe { self.value.assume_init_ref() })
        } else {
            None
        }
    }

    /// Borrows the value mutably when present.
    pub fn as_mut(&mut self) -> Option<&mut T> {
        if self.is_some() {
            // SAFETY: the tag says the value was initialised, and `&mut self`
            // grants exclusive access.
            Some(unsafe { self.value.assume_init_mut() })
        } else {
            None
        }
    }

    /// Removes the value, leaving the empty variant behind.
    ///
    /// The bytes the value occupied stay in the slot: the same policy as
    /// `truncate` on a vector or `clear` on a map, which shorten a value without
    /// overwriting it. [`StableOption::wipe_in_place`](crate::zeroize::Wipe)
    /// overwrites a value that should not stay readable, including the bytes a
    /// `take` left behind, and the empty variant built by [`StableOption::none`]
    /// writes zeros over the slot.
    pub fn take(&mut self) -> Option<T> {
        if !self.is_some() {
            return None;
        }
        self.tag = NONE_TAG;
        // SAFETY: the tag said the value was initialised, and it is now
        // unreachable through this value.
        Some(unsafe { self.value.assume_init_read() })
    }

    /// Consumes the option, returning the value when present.
    pub fn into_option(self) -> Option<T> {
        let this = ManuallyDrop::new(self);
        if this.tag != SOME_TAG {
            return None;
        }
        // SAFETY: the tag said the value was initialised, and `this` is not
        // dropped.
        Some(unsafe { this.value.assume_init_read() })
    }

    /// Stores `value` and answers a borrow of it.
    ///
    /// The value this one replaced is released.
    pub fn insert(&mut self, value: T) -> &mut T {
        let previous = self.take();
        drop(previous);
        self.tag = SOME_TAG;
        self.value.write(value)
    }

    /// The value, filling the option with `value` when it is empty.
    pub fn get_or_insert(&mut self, value: T) -> &mut T {
        if !self.is_some() {
            self.tag = SOME_TAG;
            self.value.write(value);
        }
        // SAFETY: the tag says a value is present.
        unsafe { self.value.assume_init_mut() }
    }

    /// Stores `value` and answers the value it replaced.
    pub fn replace(&mut self, value: T) -> Option<T> {
        let previous = self.take();
        self.tag = SOME_TAG;
        self.value.write(value);
        previous
    }

    /// The value when present, or the value `fallback` produces.
    pub fn unwrap_or_else<F: FnOnce() -> T>(self, fallback: F) -> T {
        match self.into_option() {
            Some(value) => value,
            None => fallback(),
        }
    }

    /// This value when it is present, and `other` otherwise.
    pub fn or(self, other: Self) -> Self {
        match self.into_option() {
            Some(value) => Self::some(value),
            None => other,
        }
    }

    /// One of the two values when exactly one is present.
    pub fn xor(self, other: Self) -> Self {
        match (self.into_option(), other.into_option()) {
            (Some(value), None) => Self::some(value),
            (None, Some(value)) => Self::some(value),
            _ => Self::none(),
        }
    }

    /// The value when present, or `fallback`.
    pub fn unwrap_or(self, fallback: T) -> T {
        match self.into_option() {
            Some(value) => value,
            None => fallback,
        }
    }

    /// Applies `transform` to the value when present.
    pub fn map<U, F: FnOnce(T) -> U>(self, transform: F) -> StableOption<U> {
        match self.into_option() {
            Some(value) => StableOption::some(transform(value)),
            None => StableOption::none(),
        }
    }

    /// Applies `transform` when present, or answers with the empty variant.
    pub fn and_then<U, F: FnOnce(T) -> StableOption<U>>(self, transform: F) -> StableOption<U> {
        match self.into_option() {
            Some(value) => transform(value),
            None => StableOption::none(),
        }
    }

    /// Returns the value when present, or `fallback`.
    pub fn or_else<F: FnOnce() -> StableOption<T>>(self, fallback: F) -> StableOption<T> {
        match self.into_option() {
            Some(value) => StableOption::some(value),
            None => fallback(),
        }
    }

    /// Keeps the value only when `predicate` accepts it.
    pub fn filter<P: FnOnce(&T) -> bool>(self, predicate: P) -> StableOption<T> {
        match self.into_option() {
            Some(value) if predicate(&value) => StableOption::some(value),
            _ => StableOption::none(),
        }
    }

    /// Pairs two values when both are present.
    pub fn zip<U>(self, other: StableOption<U>) -> StableOption<(T, U)> {
        match (self.into_option(), other.into_option()) {
            (Some(first), Some(second)) => StableOption::some((first, second)),
            _ => StableOption::none(),
        }
    }

    /// The value when present.
    ///
    /// # Panics
    ///
    /// Panics when the variant is empty.
    pub fn unwrap(self) -> T {
        match self.into_option() {
            Some(value) => value,
            None => panic!("called `unwrap` on an empty optional value"),
        }
    }

    /// The value when present, panicking with `message` when empty.
    ///
    /// # Panics
    ///
    /// Panics when the variant is empty.
    pub fn expect(self, message: &str) -> T {
        match self.into_option() {
            Some(value) => value,
            None => panic!("{}: the optional value is empty", message),
        }
    }

    /// The value as a result, or `error` when the variant is empty.
    ///
    /// This is the crossing between the two boundary types, so a caller that has
    /// to report a failure answers in the form the boundary already carries.
    pub fn ok_or<E>(self, error: E) -> crate::result::StableResult<T, E> {
        match self.into_option() {
            Some(value) => crate::result::StableResult::ok(value),
            None => crate::result::StableResult::err(error),
        }
    }

    /// The value as a result, or the error `error` answers when the variant is
    /// empty.
    pub fn ok_or_else<E, F: FnOnce() -> E>(self, error: F) -> crate::result::StableResult<T, E> {
        match self.into_option() {
            Some(value) => crate::result::StableResult::ok(value),
            None => crate::result::StableResult::err(error()),
        }
    }
}

impl<T> Drop for StableOption<T> {
    fn drop(&mut self) {
        if self.is_some() {
            // SAFETY: the tag says the value was initialised, and the storage
            // is not used after this.
            unsafe { self.value.assume_init_drop() };
        }
        self.tag = NONE_TAG;
    }
}

impl<T> Default for StableOption<T> {
    fn default() -> Self {
        Self::none()
    }
}

impl<T> From<Option<T>> for StableOption<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::some(value),
            None => Self::none(),
        }
    }
}

impl<T> From<StableOption<T>> for Option<T> {
    fn from(value: StableOption<T>) -> Self {
        value.into_option()
    }
}

impl<T> IntoIterator for StableOption<T> {
    type Item = T;
    type IntoIter = core::option::IntoIter<T>;

    /// Yields the value when it is present and nothing when it is empty, so a
    /// loop over a value that may be absent needs no match.
    fn into_iter(self) -> Self::IntoIter {
        let this = core::mem::ManuallyDrop::new(self);
        let value = if this.is_some() {
            // SAFETY: the tag says the slot holds a value, and this value is not
            // dropped: the value moved into the iterator.
            Some(unsafe { core::ptr::read(this.value.as_ptr()) })
        } else {
            None
        };
        value.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a StableOption<T> {
    type Item = &'a T;
    type IntoIter = core::option::IntoIter<&'a T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_ref().into_iter()
    }
}

impl<T: PartialEq> PartialEq for StableOption<T> {
    fn eq(&self, other: &Self) -> bool {
        match (self.as_ref(), other.as_ref()) {
            (Some(left), Some(right)) => left == right,
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: Eq> Eq for StableOption<T> {}

impl<T: Clone> Clone for StableOption<T> {
    /// Copies the value, keeping the variant.
    ///
    /// The copy allocates nothing of its own: the value is a tag and one
    /// payload. A payload whose own copy allocates reports a failure through its
    /// own `Clone`, which panics, as every fallible copy in this crate does.
    fn clone(&self) -> Self {
        match self.as_ref() {
            Some(value) => Self::some(value.clone()),
            None => Self::none(),
        }
    }
}

impl<T: fmt::Debug> fmt::Debug for StableOption<T> {
    // The value can be content a log must not repeat, so the debug form reports
    // which variant holds the bytes. `as_ref` prints the value itself when a
    // caller asks for it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_some() {
            formatter.write_str("StableOption::Some { .. }")
        } else if self.is_none() {
            formatter.write_str("StableOption::None")
        } else {
            // A tag this type does not define is not the absent value: reporting
            // it as `None` would hide a value that `validate` refuses.
            write!(formatter, "StableOption {{ tag: {} }}", self.tag)
        }
    }
}

impl<T: crate::zeroize::Wipe> crate::zeroize::Wipe for StableOption<T> {
    /// Overwrites the value when present, and the bytes it left behind when it
    /// is not.
    ///
    /// A value that was moved out leaves its bytes in the slot, and the tag no
    /// longer names a value there, so the wipe has to reach the storage rather
    /// than the accessor: `take` followed by a wipe is the way a caller clears
    /// something it no longer reads through this value.
    fn wipe_in_place(&mut self) {
        if let Some(value) = self.as_mut() {
            value.wipe_in_place();
            return;
        }
        // The wipe reaches the storage rather than an accessor, and the writes
        // are volatile for the same reason every other content path is: nothing
        // reads these bytes again, so a plain store is the compiler's to remove.
        let _ = unsafe {
            crate::zeroize::volatile_zero_raw(
                self.value.as_mut_ptr().cast::<u8>(),
                1,
                core::mem::size_of::<T>() as u64,
            )
        };
    }
}

// SAFETY: the type stores a tag and the value inline, and its drop releases
// exactly the value that was initialised.
unsafe impl<T: StableElement> StableElement for StableOption<T> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<T: PartialOrd> PartialOrd for StableOption<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match (self.as_ref(), other.as_ref()) {
            (None, None) => Some(core::cmp::Ordering::Equal),
            (None, Some(_)) => Some(core::cmp::Ordering::Less),
            (Some(_), None) => Some(core::cmp::Ordering::Greater),
            (Some(left), Some(right)) => left.partial_cmp(right),
        }
    }
}

impl<T: Ord> Ord for StableOption<T> {
    /// Orders the absent value before every present one, as `Option` does.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match (self.as_ref(), other.as_ref()) {
            (None, None) => core::cmp::Ordering::Equal,
            (None, Some(_)) => core::cmp::Ordering::Less,
            (Some(_), None) => core::cmp::Ordering::Greater,
            (Some(left), Some(right)) => left.cmp(right),
        }
    }
}

impl<T: core::hash::Hash> core::hash::Hash for StableOption<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self.as_ref() {
            None => core::hash::Hash::hash(&0u32, state),
            Some(value) => {
                core::hash::Hash::hash(&1u32, state);
                core::hash::Hash::hash(value, state);
            }
        }
    }
}
