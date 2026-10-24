//! The types a boundary call may pass or return.
//!
//! [`stable_object!`](crate::stable_object) and
//! [`stable_object_guarded!`](crate::stable_object_guarded) accept only these
//! types in a method signature. The check happens where the trampoline is
//! generated, so a method that names `String`, `usize` or `bool` fails to
//! compile instead of quietly crossing the boundary with a layout the other side
//! does not share.
//!
//! An implementor also says which states the type does not define, so a value
//! produced by the other side can be rejected before it reaches safe code:
//! [`StableBool`] checks its byte, [`StableChar`] checks the code point, and the
//! tagged containers check their tag.

use crate::array::StableArray;
use crate::boxed::StableBox;
use crate::cow::{StableCowSlice, StableCowStr};
use crate::duration::StableDuration;
use crate::error::{AbiError, AbiResult, AbiStatus};
use crate::handle::Handle;
use crate::hash::{StableHashEntry, StableHashMap, StableHashSet};
use crate::map::StableMap;
use crate::object::StableDyn;
use crate::option::StableOption;
#[cfg(feature = "std")]
use crate::path::{StableOsStr, StableOsString, StablePath, StablePathBuf};
use crate::primitive::{StableBool, StableChar};
use crate::result::StableResult;
use crate::set::StableSet;
use crate::string::{StableStr, StableString};
#[cfg(feature = "std")]
use crate::time::{StableInstant, StableSystemTime};
use crate::tuple::{StablePair, StableQuad, StableTriple};
use crate::vec::{StableElement, StableSlice, StableSliceMut, StableVec};
use crate::version::AbiVersion;

/// A type whose layout and ownership rule are part of the boundary.
///
/// Implementors must be [`StableElement`]s, so the same type can also be stored
/// in a [`StableVec`] and released through its element rule.
///
/// # Safety
///
/// An implementor promises that the type has the same size, alignment and field
/// offsets in every build that exchanges it, and that [`AbiType::validate`]
/// reports every state the type does not define.
///
/// A receiver calls `validate` on values that arrived from the other side before
/// reading them. Which side runs the check depends on the direction and on the
/// kind of table:
///
/// - a **return value** is checked by the generated accessor, whatever kind of
///   table produced it;
/// - an **argument** of a table built by [`stable_object!`](crate::stable_object!)
///   is checked by the accessor on the sending side, before the call: a value
///   that fails is abandoned and the call panics, because that table has no
///   channel to report a refusal through;
/// - an **argument** of a table built by
///   [`stable_object_guarded!`](crate::stable_object_guarded) is checked by the
///   trampoline on the receiving side, which stops the call and reports
///   [`AbiStatus::InvalidArgument`](crate::AbiStatus::InvalidArgument) rather
///   than panicking.
///
/// What `validate` can decide is bounded by what the value itself describes: a
/// length past a capacity, a tag outside the set the type defines, an alignment
/// that cannot hold the element type. A producer that reports a length inside
/// the capacity for slots it never initialised tells the type nothing is wrong
/// with it; that producer breaks its own contract, and no check on the value can
/// see it. A borrowed handle describes no capacity at all, so its length is
/// trusted as written.
///
/// A check allocates nothing and reads the value where it lies, so it can run on
/// a value the receiver has not taken ownership of and costs nothing per call
/// beyond the pass over the value.
///
/// One shape is outside what a check can decide: a hash container keeps the
/// number the sender's hash function produced beside each entry, and this
/// side does not hold that function. `validate` therefore checks the order of
/// the pairs and the elements, not the numbers, so a container whose entries were
/// numbered by another function passes and then answers nothing. A receiver that
/// knows the function checks the value with
/// [`StableHashMap::validate_with`](crate::StableHashMap::validate_with) before
/// it reads it; the alternative is a protocol that names the function, which this
/// crate leaves to the sides exchanging the value.
pub unsafe trait AbiType: StableElement {
    /// Reports whether the value is one the type defines.
    ///
    /// The default accepts every value, which is correct for types whose bytes
    /// are all valid.
    fn validate(&self) -> AbiResult<()> {
        Ok(())
    }
}

/// Reports whether `value` is one the type defines.
///
/// This is the form the generated accessors use: it names no type, so it works
/// for arguments and returns whose declaration elides a lifetime.
pub fn check<T: AbiType>(value: &T) -> AbiResult<()> {
    AbiType::validate(value)
}

/// Reports whether `value` is one the type defines, panicking when it is not.
///
/// An accessor that returns a value directly has no status to report through, so
/// it stops here rather than hand safe code a value it cannot read.
///
/// # Panics
///
/// Panics when `value` is not one the type defines.
pub fn require<T: AbiType>(value: &T) {
    if let Err(error) = AbiType::validate(value) {
        panic!("the implementation returned a value the type does not define: {error}");
    }
}

/// A value a call that cannot report a failure returns in its place.
///
/// A call whose whole channel is its return value still has to return
/// something when the implementation gave up; this trait supplies that value.
/// The caller learns what happened from the status that travels beside the
/// call, and must not treat the fallback as an answer.
///
/// The trait lives here rather than with the panic containment that consumes it
/// because the generated tables refer to it in every configuration, including
/// the one without the standard library.
pub trait PanicFallback {
    /// The value returned in place of a call that did not answer.
    fn panic_value() -> Self;
}

macro_rules! impl_abi_type_for_scalars {
    ($($type:ty),* $(,)?) => {
        $(
            // SAFETY: the type is a scalar with a fixed layout, owns nothing,
            // and every bit pattern is a value of the type.
            unsafe impl AbiType for $type {}
        )*
    };
}

impl_abi_type_for_scalars!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

// SAFETY: the unit type has one value, no fields and no ownership.
unsafe impl AbiType for () {}

// SAFETY: the wrapper is a byte with the same layout everywhere; the only
// invalid states are the bytes above one, which `validate` rejects.
unsafe impl AbiType for StableBool {
    fn validate(&self) -> AbiResult<()> {
        if self.is_valid() {
            Ok(())
        } else {
            Err(AbiError::new(AbiStatus::InvalidArgument))
        }
    }
}

// SAFETY: the wrapper is a `u32` with the same layout everywhere; surrogates and
// code points past the Unicode range are rejected by `validate`.
unsafe impl AbiType for StableChar {
    fn validate(&self) -> AbiResult<()> {
        if self.is_valid() {
            Ok(())
        } else {
            Err(AbiError::new(AbiStatus::InvalidArgument))
        }
    }
}

// SAFETY: the handle is a pointer and a length; reading it is bounded by the
// length the producer recorded, and `validate` rejects a length that would not
// be addressable on this target as well as any bytes that are not UTF-8.
unsafe impl<'a> AbiType for StableStr<'a> {
    fn validate(&self) -> AbiResult<()> {
        self.validate_text()
    }
}

// SAFETY: the handle is a pointer and a length; reading it is bounded by the
// length the producer recorded, and `validate` checks both the header and every
// element.
unsafe impl<'a, T: AbiType> AbiType for StableSlice<'a, T> {
    fn validate(&self) -> AbiResult<()> {
        <crate::vec::StableSlice<'a, T>>::validate(self)
    }
}

// SAFETY: the handle is a pointer and a length; writing through it is bounded by
// the length the producer recorded, and `validate` checks both the header and
// every element.
unsafe impl<'a, T: AbiType> AbiType for StableSliceMut<'a, T> {
    fn validate(&self) -> AbiResult<()> {
        <crate::vec::StableSliceMut<'a, T>>::validate(self)
    }
}

// SAFETY: the owned string carries its allocator and length, so every field is
// readable regardless of the contents; `validate` checks the header and that the
// bytes are UTF-8.
unsafe impl AbiType for StableString {
    fn validate(&self) -> AbiResult<()> {
        StableString::validate(self)
    }
}

// SAFETY: the vector carries its allocator, length and capacity; `validate`
// checks that the header describes a block that could exist and that every
// element is a value its type defines.
unsafe impl<T: AbiType> AbiType for StableVec<T> {
    fn validate(&self) -> AbiResult<()> {
        <StableVec<T>>::validate(self)
    }
}

// SAFETY: the array is `N` values of `T` with no allocation, and `validate`
// checks every one of them.
unsafe impl<T: AbiType, const N: usize> AbiType for StableArray<T, N> {
    fn validate(&self) -> AbiResult<()> {
        StableArray::validate(self)
    }
}

// SAFETY: the pair is two values with the same layout on both sides, and
// `validate` checks both of them.
unsafe impl<A: AbiType, B: AbiType> AbiType for StablePair<A, B> {
    fn validate(&self) -> AbiResult<()> {
        check(&self.left)?;
        check(&self.right)
    }
}

// SAFETY: the triple is three values with the same layout on both sides, and
// `validate` checks all of them.
unsafe impl<A: AbiType, B: AbiType, C: AbiType> AbiType for StableTriple<A, B, C> {
    fn validate(&self) -> AbiResult<()> {
        check(&self.first)?;
        check(&self.second)?;
        check(&self.third)
    }
}

// SAFETY: the group is four values with the same layout on both sides, and
// `validate` checks all of them.
unsafe impl<A: AbiType, B: AbiType, C: AbiType, D: AbiType> AbiType for StableQuad<A, B, C, D> {
    fn validate(&self) -> AbiResult<()> {
        check(&self.first)?;
        check(&self.second)?;
        check(&self.third)?;
        check(&self.fourth)
    }
}

// SAFETY: the map carries its allocator, length and capacity; `validate` checks
// the header, that the entries are sorted by key, and that every key and value
// is a value its type defines.
unsafe impl<K: AbiType + Ord, V: AbiType> AbiType for StableMap<K, V> {
    fn validate(&self) -> AbiResult<()> {
        StableMap::validate(self)
    }
}

// SAFETY: the set carries its allocator, length and capacity; `validate` checks
// the header, that the values are strictly ascending, and that every value is a
// value its type defines.
unsafe impl<T: AbiType + Ord> AbiType for StableSet<T> {
    fn validate(&self) -> AbiResult<()> {
        StableSet::validate(self)
    }
}

// SAFETY: the entry is a hash and two values with the same layout on both sides,
// and `validate` checks both of them.
unsafe impl<K: AbiType, V: AbiType> AbiType for StableHashEntry<K, V> {
    fn validate(&self) -> AbiResult<()> {
        check(&self.key)?;
        check(&self.value)
    }
}

// SAFETY: the map carries its allocator, length and capacity; `validate` checks
// the header, that the entries are ordered by hash and key, and that every key
// and value is a value its type defines. The numbers themselves are not checked:
// they are the sender's hash function's answer, and this side does not hold that
// function - see the note on this trait and `validate_with`.
unsafe impl<K: AbiType + Ord, V: AbiType> AbiType for StableHashMap<K, V> {
    fn validate(&self) -> AbiResult<()> {
        StableHashMap::validate(self)
    }
}

// SAFETY: the set carries one map; `validate` checks that map, which checks the
// ordering of the hashes and every key.
unsafe impl<T: AbiType + Ord> AbiType for StableHashSet<T> {
    fn validate(&self) -> AbiResult<()> {
        StableHashSet::validate(self)
    }
}

// SAFETY: the object carries its allocator and table; `validate` checks that the
// two halves describe the same thing, and a value with no object is empty in
// both halves.
unsafe impl AbiType for StableDyn {
    fn validate(&self) -> AbiResult<()> {
        StableDyn::validate(self)
    }
}

// SAFETY: the copy-on-write string carries a tag this type defines and one of
// two fields; `validate` checks the tag and the field it names.
unsafe impl<'a> AbiType for StableCowStr<'a> {
    fn validate(&self) -> AbiResult<()> {
        StableCowStr::validate(self)
    }
}

// SAFETY: the duration is two integers with the same layout on both sides;
// `validate` rejects a nanosecond field outside the range a duration defines.
unsafe impl AbiType for StableDuration {
    fn validate(&self) -> AbiResult<()> {
        StableDuration::validate(self)
    }
}

// SAFETY: the box is an address and an allocator; `validate` checks that the
// address can name a value of `T` and that the value is one its type defines.
unsafe impl<T: AbiType> AbiType for StableBox<T> {
    fn validate(&self) -> AbiResult<()> {
        StableBox::validate(self)
    }
}

// SAFETY: the sequence is a tag, a borrow and an owned vector; `validate`
// checks the tag and the field it names.
unsafe impl<'a, T: AbiType> AbiType for StableCowSlice<'a, T> {
    fn validate(&self) -> AbiResult<()> {
        StableCowSlice::validate(self)
    }
}

// SAFETY: the name is a borrowed byte slice and an encoding tag; `validate`
// checks the header, the tag and that the bytes follow the encoding it names.
#[cfg(feature = "std")]
unsafe impl<'a> AbiType for StableOsStr<'a> {
    fn validate(&self) -> AbiResult<()> {
        StableOsStr::validate(self)
    }
}

// SAFETY: the name is an owned byte vector and an encoding tag; `validate`
// checks the same things a borrowed handle is checked for.
#[cfg(feature = "std")]
unsafe impl AbiType for StableOsString {
    fn validate(&self) -> AbiResult<()> {
        StableOsString::validate(self)
    }
}

// SAFETY: the path is a borrowed name.
#[cfg(feature = "std")]
unsafe impl<'a> AbiType for StablePath<'a> {
    fn validate(&self) -> AbiResult<()> {
        self.as_stable_os_str().validate()
    }
}

// SAFETY: the path is an owned name; `validate` checks its storage before it
// reads the bytes, which the borrowed form cannot do on its own.
#[cfg(feature = "std")]
unsafe impl AbiType for StablePathBuf {
    fn validate(&self) -> AbiResult<()> {
        StablePathBuf::validate(self)
    }
}

// SAFETY: the point is two integers with the same layout on both sides;
// `validate` rejects a nanosecond field that is not below one second.
#[cfg(feature = "std")]
unsafe impl AbiType for StableSystemTime {
    fn validate(&self) -> AbiResult<()> {
        StableSystemTime::validate(self)
    }
}

// SAFETY: the reading is two integers with the same layout on both sides, and
// every pair of them is a reading.
#[cfg(feature = "std")]
unsafe impl AbiType for StableInstant {
    fn validate(&self) -> AbiResult<()> {
        StableInstant::validate(self)
    }
}

// SAFETY: the version is two integers with the same layout on both sides.
unsafe impl AbiType for AbiVersion {}

// SAFETY: the handle is one word with the same layout on both sides.
unsafe impl AbiType for Handle {}

// SAFETY: the status is a `u32` with the same layout on both sides. A code this
// build has no name for is still a value: the type is a number with names for
// some of them, and [`AbiStatus::from_code`] is what decides how to read it.
// Refusing it here would lose a failure a library built against a later revision
// reported, which is the opposite of what the type is for.
unsafe impl AbiType for AbiStatus {}

// SAFETY: the option stores a tag followed by the payload, and the tag is what
// says whether the payload may be read.
unsafe impl<T: AbiType> AbiType for StableOption<T> {
    fn validate(&self) -> AbiResult<()> {
        StableOption::validate(self)
    }
}

// SAFETY: the result stores a tag followed by the payload, and the tag is what
// says which payload may be read.
unsafe impl<T: AbiType, E: AbiType> AbiType for StableResult<T, E> {
    fn validate(&self) -> AbiResult<()> {
        StableResult::validate(self)
    }
}
