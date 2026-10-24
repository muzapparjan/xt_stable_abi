//! A fallible value with an explicit tag.
//!
//! `Result<T, E>` uses a niche in `T` or `E`, and which niche it finds depends
//! on the compiler. [`StableResult`] stores a `u32` tag followed by a union of
//! the two payloads, so the bytes are fixed.

use core::fmt;
use core::mem::{ManuallyDrop, MaybeUninit};

use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::vec::StableElement;

/// The tag written for the success variant.
pub const OK_TAG: u32 = 0;
/// The tag written for the failure variant.
pub const ERR_TAG: u32 = 1;

/// The payload storage shared by both variants.
///
/// Exactly one field is active, and which one is recorded by the tag beside it.
#[repr(C)]
pub union StableResultValue<T, E> {
    /// The success payload, active when the tag is [`OK_TAG`].
    pub ok: ManuallyDrop<T>,
    /// The failure payload, active when the tag is [`ERR_TAG`].
    pub err: ManuallyDrop<E>,
}

impl<T, E> fmt::Debug for StableResultValue<T, E> {
    // The active field is not known from the payload alone, so the payload is
    // not printed; the tag next to it says which variant is meant.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StableResultValue { .. }")
    }
}

/// A fallible value.
///
/// Both variants occupy the same storage, so the size never depends on which
/// variant is active.
#[repr(C)]
pub struct StableResult<T, E> {
    tag: u32,
    value: StableResultValue<T, E>,
}

impl<T, E> StableResult<T, E> {
    /// The success variant.
    ///
    /// The storage is zeroed before the payload is written, so bytes that no
    /// variant occupies do not carry stack contents across the boundary and two
    /// equal values have equal bytes.
    pub fn ok(value: T) -> Self {
        let mut storage = MaybeUninit::<StableResultValue<T, E>>::zeroed();
        // SAFETY: `storage` is writable and the success field is the only one
        // that will be read for this tag.
        unsafe { core::ptr::addr_of_mut!((*storage.as_mut_ptr()).ok).write(ManuallyDrop::new(value)) };
        Self {
            tag: OK_TAG,
            // SAFETY: the active field was just written.
            value: unsafe { storage.assume_init() },
        }
    }

    /// The failure variant.
    pub fn err(value: E) -> Self {
        let mut storage = MaybeUninit::<StableResultValue<T, E>>::zeroed();
        // SAFETY: `storage` is writable and the failure field is the only one
        // that will be read for this tag.
        unsafe { core::ptr::addr_of_mut!((*storage.as_mut_ptr()).err).write(ManuallyDrop::new(value)) };
        Self {
            tag: ERR_TAG,
            // SAFETY: the active field was just written.
            value: unsafe { storage.assume_init() },
        }
    }

    /// Rebuilds a value from a tag and a payload that arrived as bytes.
    ///
    /// This is the entry point a receiver uses: a tag outside the two this type
    /// defines is rejected instead of being read as one of the variants.
    ///
    /// When the tag is rejected the payload is not released, because which
    /// variant it holds cannot be determined from a tag this type never wrote.
    /// It is also not handed back: the value moved into this function. A
    /// receiver that rejects foreign bytes releases those bytes as raw storage,
    /// not through this type.
    ///
    /// # Safety
    ///
    /// The field `tag` names must hold an initialised value: `value.ok` when
    /// `tag` is [`OK_TAG`] and `value.err` when it is [`ERR_TAG`]. Reading the
    /// other field is undefined behaviour, and so is the drop this type runs.
    pub unsafe fn from_parts(tag: u32, value: StableResultValue<T, E>) -> AbiResult<Self> {
        match tag {
            OK_TAG | ERR_TAG => Ok(Self { tag, value }),
            // The fields hold `ManuallyDrop`, so dropping the parameter touches
            // nothing: no value is released on a tag this type never wrote.
            _ => Err(AbiError::new(AbiStatus::InvalidTag)),
        }
    }

    /// The raw tag, as written into memory.
    pub const fn tag(&self) -> u32 {
        self.tag
    }

    /// Reports whether the success variant is active.
    pub const fn is_ok(&self) -> bool {
        self.tag == OK_TAG
    }

    /// Reports whether the failure variant is active.
    pub const fn is_err(&self) -> bool {
        self.tag == ERR_TAG
    }

    /// Checks that the tag names a variant this type defines, and that the
    /// payload is a value its type defines.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
        E: crate::boundary::AbiType,
    {
        match self.tag {
            // SAFETY: the tag says which payload was initialised.
            OK_TAG => crate::boundary::check(unsafe { &*self.value.ok }),
            // SAFETY: the tag says which payload was initialised.
            ERR_TAG => crate::boundary::check(unsafe { &*self.value.err }),
            _ => Err(AbiError::new(AbiStatus::InvalidTag)),
        }
    }

    /// Borrows the active variant.
    ///
    /// # Panics
    ///
    /// Panics when the tag names no variant this type defines. Only a value
    /// rebuilt through [`StableResult::from_parts`] can be in that state.
    pub fn as_ref(&self) -> core::result::Result<&T, &E> {
        match self.tag {
            // SAFETY: the tag says the success payload was initialised.
            OK_TAG => Ok(unsafe { &*self.value.ok }),
            // SAFETY: the tag says the failure payload was initialised.
            ERR_TAG => Err(unsafe { &*self.value.err }),
            _ => panic!("the tag does not name a variant this type defines"),
        }
    }

    /// Borrows the success payload when present.
    pub fn ok_ref(&self) -> Option<&T> {
        self.as_ref().ok()
    }

    /// Borrows the failure payload when present.
    pub fn err_ref(&self) -> Option<&E> {
        self.as_ref().err()
    }

    /// Borrows the active variant, for writing.
    ///
    /// # Panics
    ///
    /// Panics when the tag names no variant this type defines. Only a value
    /// rebuilt through [`StableResult::from_parts`] can be in that state.
    pub fn as_mut(&mut self) -> core::result::Result<&mut T, &mut E> {
        match self.tag {
            // SAFETY: the tag says the success payload was initialised.
            OK_TAG => Ok(unsafe { &mut *self.value.ok }),
            // SAFETY: the tag says the failure payload was initialised.
            ERR_TAG => Err(unsafe { &mut *self.value.err }),
            _ => panic!("the tag does not name a variant this type defines"),
        }
    }

    /// Iterates over the success payload, which is at most one value.
    pub fn iter(&self) -> core::option::IntoIter<&T> {
        self.ok_ref().into_iter()
    }

    /// The success payload, or `fallback` when the variant is a failure.
    pub fn map_or<U, F: FnOnce(T) -> U>(self, fallback: U, transform: F) -> U {
        match self.into_result() {
            Ok(value) => transform(value),
            Err(_) => fallback,
        }
    }

    /// The success payload transformed by `transform`, or the failure payload
    /// transformed by `fallback`.
    pub fn map_or_else<U, D: FnOnce(E) -> U, F: FnOnce(T) -> U>(
        self,
        fallback: D,
        transform: F,
    ) -> U {
        match self.into_result() {
            Ok(value) => transform(value),
            Err(error) => fallback(error),
        }
    }

    /// The failure payload, panicking with a generic message when the variant
    /// is a success.
    ///
    /// # Panics
    ///
    /// Panics when the variant is a success.
    pub fn unwrap_err(self) -> E {
        match self.into_result() {
            Err(error) => error,
            Ok(_) => panic!("called `unwrap_err` on a successful stable result"),
        }
    }

    /// The failure payload, panicking with `message` when the variant is a
    /// success.
    ///
    /// # Panics
    ///
    /// Panics when the variant is a success.
    pub fn expect_err(self, message: &str) -> E {
        match self.into_result() {
            Err(error) => error,
            Ok(_) => panic!("{}: the stable result is a success", message),
        }
    }

    /// Consumes the value, returning the success payload when present.
    ///
    /// The other payload, if any, is released: this is a transfer, not a peek.
    ///
    /// # Panics
    ///
    /// Panics when the tag names no variant this type defines.
    pub fn ok_value(self) -> Option<T> {
        let mut this = ManuallyDrop::new(self);
        match this.tag {
            // SAFETY: the tag says the payload was initialised, and `this` is
            // not dropped.
            OK_TAG => Some(unsafe { ManuallyDrop::take(&mut this.value.ok) }),
            // SAFETY: the tag says the failure payload is the initialised one.
            ERR_TAG => {
                unsafe { ManuallyDrop::drop(&mut this.value.err) };
                None
            }
            _ => panic!("the tag does not name a variant this type defines"),
        }
    }

    /// Consumes the value, returning the failure payload when present.
    ///
    /// The other payload, if any, is released: this is a transfer, not a peek.
    ///
    /// # Panics
    ///
    /// Panics when the tag names no variant this type defines.
    pub fn err_value(self) -> Option<E> {
        let mut this = ManuallyDrop::new(self);
        match this.tag {
            // SAFETY: the tag says the payload was initialised, and `this` is
            // not dropped.
            ERR_TAG => Some(unsafe { ManuallyDrop::take(&mut this.value.err) }),
            // SAFETY: the tag says the success payload is the initialised one.
            OK_TAG => {
                unsafe { ManuallyDrop::drop(&mut this.value.ok) };
                None
            }
            _ => panic!("the tag does not name a variant this type defines"),
        }
    }

    /// Consumes the value, returning the standard result.
    ///
    /// # Panics
    ///
    /// Panics when the tag names no variant this type defines.
    pub fn into_result(self) -> core::result::Result<T, E> {
        let mut this = ManuallyDrop::new(self);
        match this.tag {
            // SAFETY: the tag says the payload was initialised, and `this` is
            // not dropped.
            OK_TAG => Ok(unsafe { ManuallyDrop::take(&mut this.value.ok) }),
            // SAFETY: the tag says the payload was initialised, and `this` is
            // not dropped.
            ERR_TAG => Err(unsafe { ManuallyDrop::take(&mut this.value.err) }),
            _ => panic!("the tag does not name a variant this type defines"),
        }
    }

    /// Applies `transform` to the success payload.
    pub fn map<U, F: FnOnce(T) -> U>(self, transform: F) -> StableResult<U, E> {
        match self.into_result() {
            Ok(value) => StableResult::ok(transform(value)),
            Err(error) => StableResult::err(error),
        }
    }

    /// Applies `transform` to the failure payload.
    pub fn map_err<F2, F: FnOnce(E) -> F2>(self, transform: F) -> StableResult<T, F2> {
        match self.into_result() {
            Ok(value) => StableResult::ok(value),
            Err(error) => StableResult::err(transform(error)),
        }
    }

    /// Applies `transform` to the success payload when present.
    pub fn and_then<U, F: FnOnce(T) -> StableResult<U, E>>(self, transform: F) -> StableResult<U, E> {
        match self.into_result() {
            Ok(value) => transform(value),
            Err(error) => StableResult::err(error),
        }
    }

    /// Returns the success payload, or calls `fallback` on failure.
    pub fn or_else<F2, F: FnOnce(E) -> StableResult<T, F2>>(self, fallback: F) -> StableResult<T, F2> {
        match self.into_result() {
            Ok(value) => StableResult::ok(value),
            Err(error) => fallback(error),
        }
    }

    /// The success payload.
    ///
    /// # Panics
    ///
    /// Panics with a generic message when the variant is a failure.
    pub fn unwrap(self) -> T {
        match self.into_result() {
            Ok(value) => value,
            Err(_) => panic!("called `unwrap` on a failed stable result"),
        }
    }

    /// The success payload, panicking with `message` when the variant is a
    /// failure.
    ///
    /// # Panics
    ///
    /// Panics when the variant is a failure.
    pub fn expect(self, message: &str) -> T {
        match self.into_result() {
            Ok(value) => value,
            Err(_) => panic!("{}: the stable result is a failure", message),
        }
    }

    /// The success payload, or `fallback` on failure.
    pub fn unwrap_or(self, fallback: T) -> T {
        match self.into_result() {
            Ok(value) => value,
            Err(_) => fallback,
        }
    }

    /// The success payload, or the value `fallback` produces on failure.
    pub fn unwrap_or_else<F: FnOnce(E) -> T>(self, fallback: F) -> T {
        match self.into_result() {
            Ok(value) => value,
            Err(error) => fallback(error),
        }
    }

    /// The success payload, or the default value of `T` on failure.
    pub fn unwrap_or_default(self) -> T
    where
        T: Default,
    {
        self.into_result().unwrap_or_default()
    }

    /// The success payload of `other` when this value succeeded, and this
    /// value's failure otherwise.
    pub fn and<U>(self, other: StableResult<U, E>) -> StableResult<U, E> {
        match self.into_result() {
            Ok(_) => other,
            Err(error) => StableResult::err(error),
        }
    }

    /// This value when it succeeded, and `other` otherwise.
    pub fn or<E2>(self, other: StableResult<T, E2>) -> StableResult<T, E2> {
        match self.into_result() {
            Ok(value) => StableResult::ok(value),
            Err(_) => other,
        }
    }

    /// Applies `inspect` to the success payload and answers the same value.
    ///
    /// A closure that panics leaves the value to be released by the unwind: the
    /// payload moves out exactly once either way.
    pub fn inspect<F: FnOnce(&T)>(self, inspect: F) -> Self {
        if self.tag == OK_TAG {
            // SAFETY: the tag says the success payload was initialised, and the
            // borrow ends with the call.
            inspect(unsafe { &*self.value.ok });
        }
        self
    }
}

impl<T, E> StableResult<crate::option::StableOption<T>, E> {
    /// Turns a result of an optional value into an optional result.
    ///
    /// A failure stays a failure, an empty success becomes an empty optional, and
    /// a success carrying a value becomes an optional carrying a success.
    pub fn transpose(self) -> crate::option::StableOption<StableResult<T, E>> {
        match self.into_result() {
            Ok(optional) => match optional.into_option() {
                Some(value) => crate::option::StableOption::some(StableResult::ok(value)),
                None => crate::option::StableOption::none(),
            },
            Err(error) => crate::option::StableOption::some(StableResult::err(error)),
        }
    }
}

impl<T, E> Drop for StableResult<T, E> {
    fn drop(&mut self) {
        // A tag this type never wrote says nothing about which payload is
        // initialised, so nothing is released. The bytes are left to whoever
        // produced them, which is what `from_parts` documents.
        match self.tag {
            // SAFETY: the tag says the payload was initialised.
            OK_TAG => unsafe { ManuallyDrop::drop(&mut self.value.ok) },
            // SAFETY: the tag says the payload was initialised.
            ERR_TAG => unsafe { ManuallyDrop::drop(&mut self.value.err) },
            _ => return,
        }
        self.tag = OK_TAG;
    }
}

impl<T, E> From<core::result::Result<T, E>> for StableResult<T, E> {
    fn from(value: core::result::Result<T, E>) -> Self {
        match value {
            Ok(value) => Self::ok(value),
            Err(value) => Self::err(value),
        }
    }
}

impl<T, E> From<StableResult<T, E>> for core::result::Result<T, E> {
    fn from(value: StableResult<T, E>) -> Self {
        value.into_result()
    }
}

impl<T, E> IntoIterator for StableResult<T, E> {
    type Item = T;
    type IntoIter = core::option::IntoIter<T>;

    /// Yields the success payload and nothing on failure, so a loop over a value
    /// that may have failed needs no match.
    fn into_iter(self) -> Self::IntoIter {
        let this = ManuallyDrop::new(self);
        let value = if this.tag == OK_TAG {
            // SAFETY: the tag says the success payload was initialised, and this
            // value is not dropped: the payload moved into the iterator.
            Some(ManuallyDrop::into_inner(unsafe {
                core::ptr::read(&this.value.ok)
            }))
        } else {
            None
        };
        value.into_iter()
    }
}

impl<'a, T, E> IntoIterator for &'a StableResult<T, E> {
    type Item = &'a T;
    type IntoIter = core::option::IntoIter<&'a T>;

    fn into_iter(self) -> Self::IntoIter {
        self.ok_ref().into_iter()
    }
}

impl<T: PartialEq, E: PartialEq> PartialEq for StableResult<T, E> {    fn eq(&self, other: &Self) -> bool {
        match (self.as_ref(), other.as_ref()) {
            (Ok(left), Ok(right)) => left == right,
            (Err(left), Err(right)) => left == right,
            _ => false,
        }
    }
}

impl<T: Eq, E: Eq> Eq for StableResult<T, E> {}

impl<T: Clone, E: Clone> Clone for StableResult<T, E> {
    /// Copies the value, putting the payload in the same variant.
    ///
    /// The copy allocates nothing of its own: the value is a tag and one
    /// payload. A payload whose own copy allocates reports a failure through its
    /// own `Clone`, which panics, as every fallible copy in this crate does.
    fn clone(&self) -> Self {
        match self.as_ref() {
            Ok(value) => Self::ok(value.clone()),
            Err(error) => Self::err(error.clone()),
        }
    }
}

impl<T: fmt::Debug, E: fmt::Debug> fmt::Debug for StableResult<T, E> {
    // The payloads can be content a log must not repeat, so the debug form
    // reports which variant holds the bytes. `as_ref` prints the value itself
    // when a caller asks for it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tag {
            OK_TAG => formatter.write_str("StableResult::Ok { .. }"),
            ERR_TAG => formatter.write_str("StableResult::Err { .. }"),
            _ => formatter.write_str("StableResult { undefined tag }"),
        }
    }
}

impl<T: crate::zeroize::Wipe, E: crate::zeroize::Wipe> crate::zeroize::Wipe
    for StableResult<T, E>
{
    fn wipe_in_place(&mut self) {
        match self.tag {
            // SAFETY: the tag says the success payload was initialised.
            OK_TAG => unsafe { (*self.value.ok).wipe_in_place() },
            // SAFETY: the tag says the failure payload was initialised.
            ERR_TAG => unsafe { (*self.value.err).wipe_in_place() },
            _ => {}
        }
    }
}

// SAFETY: the type stores a tag and one payload inline, and its drop releases
// exactly the payload that was initialised.
unsafe impl<T: StableElement, E: StableElement> StableElement for StableResult<T, E> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<T: PartialOrd, E: PartialOrd> PartialOrd for StableResult<T, E> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        match (self.as_ref(), other.as_ref()) {
            (Ok(left), Ok(right)) => left.partial_cmp(right),
            (Ok(_), Err(_)) => Some(core::cmp::Ordering::Less),
            (Err(_), Ok(_)) => Some(core::cmp::Ordering::Greater),
            (Err(left), Err(right)) => left.partial_cmp(right),
        }
    }
}

impl<T: Ord, E: Ord> Ord for StableResult<T, E> {
    /// Orders a value carrying `Ok` before one carrying `Err`, as `Result`
    /// does.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match (self.as_ref(), other.as_ref()) {
            (Ok(left), Ok(right)) => left.cmp(right),
            (Ok(_), Err(_)) => core::cmp::Ordering::Less,
            (Err(_), Ok(_)) => core::cmp::Ordering::Greater,
            (Err(left), Err(right)) => left.cmp(right),
        }
    }
}

impl<T: core::hash::Hash, E: core::hash::Hash> core::hash::Hash for StableResult<T, E> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self.as_ref() {
            Ok(value) => {
                core::hash::Hash::hash(&0u32, state);
                core::hash::Hash::hash(value, state);
            }
            Err(error) => {
                core::hash::Hash::hash(&1u32, state);
                core::hash::Hash::hash(error, state);
            }
        }
    }
}
