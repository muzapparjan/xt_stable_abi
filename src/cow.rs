//! Text that is borrowed when it can be and owned when it must be.

use core::fmt;
use core::ops::Deref;

use crate::error::AbiResult;
use crate::string::{StableStr, StableString};
use crate::vec::{StableElement, StableSlice, StableVec};

/// The tag written for a borrowed value.
pub const BORROWED_TAG: u32 = 0;
/// The tag written for an owned value.
pub const OWNED_TAG: u32 = 1;

/// Text that borrows when possible and owns when needed.
///
/// Both fields are present so the layout is fixed. The owned field is held
/// through [`core::mem::ManuallyDrop`], so only the field the tag names is ever
/// read or released; the other one is storage the tag says to ignore.
#[repr(C)]
pub struct StableCowStr<'a> {
    tag: u32,
    borrowed: StableStr<'a>,
    owned: core::mem::ManuallyDrop<StableString>,
}

impl<'a> StableCowStr<'a> {
    /// Borrows `text`.
    pub const fn borrowed(text: &'a str) -> Self {
        Self {
            tag: BORROWED_TAG,
            borrowed: StableStr::from_str(text),
            owned: core::mem::ManuallyDrop::new(StableString::new()),
        }
    }

    /// Owns `text`.
    pub const fn owned(text: StableString) -> Self {
        Self {
            tag: OWNED_TAG,
            borrowed: StableStr::empty(),
            owned: core::mem::ManuallyDrop::new(text),
        }
    }

    /// The raw tag.
    pub const fn tag(&self) -> u32 {
        self.tag
    }

    /// Reports whether the text is borrowed.
    pub const fn is_borrowed(&self) -> bool {
        self.tag == BORROWED_TAG
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        if self.is_borrowed() {
            self.borrowed.as_str()
        } else {
            self.owned.as_str()
        }
    }

    /// The text as a borrowed handle.
    pub fn as_stable_str(&self) -> StableStr<'_> {
        StableStr::from_str(self.as_str())
    }

    /// Takes ownership, copying the text when it was borrowed.
    pub fn into_owned(self) -> AbiResult<StableString> {
        let this = core::mem::ManuallyDrop::new(self);
        if this.is_borrowed() {
            StableString::from_str(this.borrowed.as_str())
        } else {
            // SAFETY: the value is not dropped, so the owned storage moves out
            // exactly once.
            Ok(core::mem::ManuallyDrop::into_inner(unsafe {
                core::ptr::read(&this.owned)
            }))
        }
    }

    /// Takes ownership, copying the text through `allocator` when it was
    /// borrowed.
    ///
    /// An owned value keeps the storage it already has: the allocator is the one
    /// that pair of functions records, and copying it into another block would
    /// gain nothing.
    pub fn into_owned_with_allocator(
        self,
        allocator: crate::alloc::AbiAllocator,
    ) -> AbiResult<StableString> {
        let this = core::mem::ManuallyDrop::new(self);
        if this.is_borrowed() {
            StableString::from_str_with_allocator(allocator, this.borrowed.as_str())
        } else {
            // SAFETY: the value is not dropped, so the owned storage moves out
            // exactly once.
            Ok(core::mem::ManuallyDrop::into_inner(unsafe {
                core::ptr::read(&this.owned)
            }))
        }
    }

    /// Checks that the tag names a mode this type defines, and that the field it
    /// names is a value that field's type defines.
    pub fn validate(&self) -> AbiResult<()> {
        match self.tag {
            BORROWED_TAG => crate::boundary::check(&self.borrowed),
            OWNED_TAG => crate::boundary::check(&*self.owned),
            _ => Err(crate::error::AbiError::new(crate::error::AbiStatus::InvalidTag)),
        }
    }
}

impl<'a> Clone for StableCowStr<'a> {
    fn clone(&self) -> Self {
        if self.is_borrowed() {
            Self {
                tag: BORROWED_TAG,
                borrowed: self.borrowed,
                owned: core::mem::ManuallyDrop::new(StableString::new()),
            }
        } else {
            Self {
                tag: OWNED_TAG,
                borrowed: StableStr::empty(),
                owned: core::mem::ManuallyDrop::new((*self.owned).clone()),
            }
        }
    }
}

impl<'a> Drop for StableCowStr<'a> {
    fn drop(&mut self) {
        // Only the field the tag names is released. The other one is storage the
        // tag says to ignore, which is why it is held through `ManuallyDrop`:
        // the compiler's drop glue would otherwise release bytes this type was
        // never told were a valid value.
        if self.tag == OWNED_TAG {
            // SAFETY: the tag says the owned field is the initialised one, and
            // it is not released again.
            unsafe { core::mem::ManuallyDrop::drop(&mut self.owned) };
        }
        self.tag = BORROWED_TAG;
    }
}

impl<'a> Deref for StableCowStr<'a> {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl<'a> From<&'a str> for StableCowStr<'a> {
    fn from(value: &'a str) -> Self {
        Self::borrowed(value)
    }
}

impl<'a> From<StableString> for StableCowStr<'a> {
    fn from(value: StableString) -> Self {
        Self::owned(value)
    }
}

impl<'a> PartialEq for StableCowStr<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl<'a> Eq for StableCowStr<'a> {}

impl<'a> PartialEq<str> for StableCowStr<'a> {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl<'a> fmt::Debug for StableCowStr<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableCowStr")
            .field("borrowed", &self.is_borrowed())
            .field("len", &self.as_str().len())
            .finish()
    }
}

impl<'a> fmt::Display for StableCowStr<'a> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

// SAFETY: the struct is a `#[repr(C)]` group of a tag, a borrow and an owned
// string; dropping it releases exactly the owned storage when the tag names it.
unsafe impl<'a> StableElement for StableCowStr<'a> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

/// A sequence that borrows when possible and owns when needed.
///
/// Both fields are present so the layout is fixed. The owned field is held
/// through [`core::mem::ManuallyDrop`], so only the field the tag names is ever
/// read or released.
#[repr(C)]
pub struct StableCowSlice<'a, T: StableElement> {
    tag: u32,
    borrowed: StableSlice<'a, T>,
    owned: core::mem::ManuallyDrop<StableVec<T>>,
}

impl<'a, T: StableElement> StableCowSlice<'a, T> {
    /// Borrows `values`.
    pub const fn borrowed(values: &'a [T]) -> Self {
        Self {
            tag: BORROWED_TAG,
            borrowed: StableSlice::from_slice(values),
            owned: core::mem::ManuallyDrop::new(StableVec::new()),
        }
    }

    /// Owns `values`.
    pub const fn owned(values: StableVec<T>) -> Self {
        Self {
            tag: OWNED_TAG,
            borrowed: StableSlice::empty(),
            owned: core::mem::ManuallyDrop::new(values),
        }
    }

    /// The raw tag.
    pub const fn tag(&self) -> u32 {
        self.tag
    }

    /// Reports whether the values are borrowed.
    pub const fn is_borrowed(&self) -> bool {
        self.tag == BORROWED_TAG
    }

    /// The values.
    pub fn as_slice(&self) -> &[T] {
        if self.is_borrowed() {
            // SAFETY: a borrowed handle names storage that lives for `'a`, and
            // `self` lives no longer than that.
            unsafe { self.borrowed.as_slice_unbounded() }
        } else {
            self.owned.as_slice()
        }
    }

    /// The values as a borrowed handle.
    pub fn as_stable_slice(&self) -> StableSlice<'_, T> {
        StableSlice::from_slice(self.as_slice())
    }

    /// Takes ownership, copying the values when they were borrowed.
    pub fn into_owned(self) -> AbiResult<StableVec<T>>
    where
        T: Clone,
    {
        let this = core::mem::ManuallyDrop::new(self);
        if this.is_borrowed() {
            // SAFETY: the borrowed handle names storage that lives for `'a`.
            StableVec::from_slice(unsafe { this.borrowed.as_slice_unbounded() })
        } else {
            // SAFETY: the value is not dropped, so the owned storage moves out
            // exactly once.
            Ok(core::mem::ManuallyDrop::into_inner(unsafe {
                core::ptr::read(&this.owned)
            }))
        }
    }

    /// Takes ownership, copying the values through `allocator` when they were
    /// borrowed.
    ///
    /// An owned value keeps the storage it already has, for the reason the text
    /// form gives.
    pub fn into_owned_with_allocator(
        self,
        allocator: crate::alloc::AbiAllocator,
    ) -> AbiResult<StableVec<T>>
    where
        T: Clone,
    {
        let this = core::mem::ManuallyDrop::new(self);
        if this.is_borrowed() {
            // SAFETY: the borrowed handle names storage that lives for `'a`.
            StableVec::from_slice_with_allocator(
                allocator,
                unsafe { this.borrowed.as_slice_unbounded() },
            )
        } else {
            // SAFETY: the value is not dropped, so the owned storage moves out
            // exactly once.
            Ok(core::mem::ManuallyDrop::into_inner(unsafe {
                core::ptr::read(&this.owned)
            }))
        }
    }

    /// Checks that the tag names a mode this type defines, and that the field it
    /// names is a value that field's type defines.
    pub fn validate(&self) -> AbiResult<()>
    where
        T: crate::boundary::AbiType,
    {
        match self.tag {
            BORROWED_TAG => crate::boundary::check(&self.borrowed),
            OWNED_TAG => crate::boundary::check(&*self.owned),
            _ => Err(crate::error::AbiError::new(crate::error::AbiStatus::InvalidTag)),
        }
    }
}

impl<'a, T: StableElement> Deref for StableCowSlice<'a, T> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<'a, T: StableElement + Clone> Clone for StableCowSlice<'a, T> {
    fn clone(&self) -> Self {
        if self.is_borrowed() {
            Self {
                tag: BORROWED_TAG,
                borrowed: self.borrowed,
                owned: core::mem::ManuallyDrop::new(StableVec::new()),
            }
        } else {
            Self {
                tag: OWNED_TAG,
                borrowed: StableSlice::empty(),
                owned: core::mem::ManuallyDrop::new((*self.owned).clone()),
            }
        }
    }
}

impl<'a, T: StableElement> Drop for StableCowSlice<'a, T> {
    fn drop(&mut self) {
        // Only the field the tag names is released; the other one is storage the
        // tag says to ignore.
        if self.tag == OWNED_TAG {
            // SAFETY: the tag says the owned field is the initialised one, and
            // it is not released again.
            unsafe { core::mem::ManuallyDrop::drop(&mut self.owned) };
        }
        self.tag = BORROWED_TAG;
    }
}

impl<'a, T: StableElement + PartialEq> PartialEq for StableCowSlice<'a, T> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl<'a, T: StableElement + Eq> Eq for StableCowSlice<'a, T> {}

impl<'a, T: StableElement> fmt::Debug for StableCowSlice<'a, T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StableCowSlice")
            .field("borrowed", &self.is_borrowed())
            .field("len", &self.as_slice().len())
            .finish()
    }
}

// SAFETY: the struct is a `#[repr(C)]` group of a tag, a borrow and an owned
// vector; dropping it releases exactly the owned block when the tag names it.
unsafe impl<'a, T: StableElement> StableElement for StableCowSlice<'a, T> {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

impl<'a> crate::zeroize::Wipe for StableCowStr<'a> {
    fn wipe_in_place(&mut self) {
        if self.tag == OWNED_TAG {
            self.owned.wipe_in_place();
        }
    }
}

impl<'a, T: StableElement + crate::zeroize::Wipe> crate::zeroize::Wipe
    for StableCowSlice<'a, T>
{
    fn wipe_in_place(&mut self) {
        if self.tag == OWNED_TAG {
            self.owned.wipe_in_place();
        }
    }
}

impl<'a> PartialOrd for StableCowStr<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a> Ord for StableCowStr<'a> {
    /// Compares the text, so a borrowed and an owned value with the same
    /// content compare equal.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl<'a> core::hash::Hash for StableCowStr<'a> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::hash::Hash::hash(self.as_str(), state)
    }
}

impl<'a> crate::clone::TryClone for StableCowStr<'a> {
    fn try_clone(&self) -> AbiResult<Self> {
        if self.is_borrowed() {
            // A borrowed value owns nothing, so its copy names the same storage.
            Ok(Self {
                tag: BORROWED_TAG,
                borrowed: self.borrowed,
                owned: core::mem::ManuallyDrop::new(StableString::new()),
            })
        } else {
            Ok(Self {
                tag: OWNED_TAG,
                borrowed: StableStr::empty(),
                owned: core::mem::ManuallyDrop::new(self.owned.try_clone()?),
            })
        }
    }
}

impl<'a, T: StableElement + PartialOrd> PartialOrd for StableCowSlice<'a, T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        self.as_slice().partial_cmp(other.as_slice())
    }
}

impl<'a, T: StableElement + Ord> Ord for StableCowSlice<'a, T> {
    /// Compares the values, so a borrowed and an owned value with the same
    /// content compare equal.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl<'a, T: StableElement + core::hash::Hash> core::hash::Hash for StableCowSlice<'a, T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        core::hash::Hash::hash(self.as_slice(), state)
    }
}

impl<'a, T: StableElement + crate::clone::TryClone> crate::clone::TryClone
    for StableCowSlice<'a, T>
{
    fn try_clone(&self) -> AbiResult<Self> {
        if self.is_borrowed() {
            // A borrowed value owns nothing, so its copy names the same storage.
            Ok(Self {
                tag: BORROWED_TAG,
                borrowed: self.borrowed,
                owned: core::mem::ManuallyDrop::new(StableVec::new()),
            })
        } else {
            Ok(Self {
                tag: OWNED_TAG,
                borrowed: StableSlice::empty(),
                owned: core::mem::ManuallyDrop::new(self.owned.try_clone()?),
            })
        }
    }
}

