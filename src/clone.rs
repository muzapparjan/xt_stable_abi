//! A copy that reports a failure instead of panicking.

use crate::error::AbiResult;

/// Copies a value, reporting a failure instead of panicking.
///
/// `Clone` panics when a copy cannot be allocated, which is a reasonable
/// default for a value whose copy is a few words. A container copies its
/// elements, so the panic would come from an allocation the caller did not ask
/// for; this trait is the shape that carries the failure back instead. A caller
/// that has an error channel of its own stays in it.
///
/// [`crate::StableHashMap`] and [`crate::StableHashSet`] are implemented as
/// well: an entry is stored under the number the caller's hash function
/// produced, and the copy keeps that number instead of asking a function this
/// side does not have.
///
/// A borrowed view ([`crate::StableStr`], [`crate::StableSlice`],
/// [`crate::StableOsStr`], [`crate::StablePath`]) copies by naming the same
/// storage, because it owns nothing. [`crate::StableSliceMut`] has no
/// implementation: a second writable view of the same storage would let two
/// callers write through the same memory at once, which is the aliasing a `&mut`
/// rules out. A caller that must hand a writable view on moves it instead.
pub trait TryClone: Sized {
    /// Copies the value.
    fn try_clone(&self) -> AbiResult<Self>;
}

macro_rules! impl_try_clone_for_copy {
    ($($type:ty),* $(,)?) => {
        $(
            // A value with no storage behind it is its own copy.
            impl TryClone for $type {
                fn try_clone(&self) -> AbiResult<Self> {
                    Ok(*self)
                }
            }
        )*
    };
}

impl_try_clone_for_copy!(
    (),
    u8,
    u16,
    u32,
    u64,
    u128,
    i8,
    i16,
    i32,
    i64,
    i128,
    f32,
    f64,
    crate::primitive::StableBool,
    crate::primitive::StableChar,
    crate::duration::StableDuration,
    crate::handle::Handle,
    crate::version::AbiVersion,
    crate::error::AbiStatus,
);

/// Copies a type whose own `try_clone` already reports the failure.
macro_rules! impl_try_clone_through_method {
    ($($type:ty),* $(,)?) => {
        $(
            impl TryClone for $type {
                fn try_clone(&self) -> AbiResult<Self> {
                    // The inherent method is the one that knows how the value
                    // is stored; this implementation only names it under the
                    // trait so a container can ask for it without naming the
                    // type.
                    <$type>::try_clone(self)
                }
            }
        )*
    };
}

impl_try_clone_through_method!(
    crate::string::StableString,
    crate::error::AbiError,
);

#[cfg(feature = "std")]
impl TryClone for crate::path::StableOsString {
    fn try_clone(&self) -> AbiResult<Self> {
        crate::path::StableOsString::try_clone(self)
    }
}

#[cfg(feature = "std")]
impl TryClone for crate::path::StablePathBuf {
    fn try_clone(&self) -> AbiResult<Self> {
        crate::path::StablePathBuf::try_clone(self)
    }
}

impl<T: crate::vec::StableElement + TryClone> TryClone for crate::vec::StableVec<T> {
    fn try_clone(&self) -> AbiResult<Self> {
        let mut copy =
            Self::with_capacity_and_allocator(self.allocator(), self.len())?;
        for value in self.as_slice() {
            copy.try_push(TryClone::try_clone(value)?)?;
        }
        Ok(copy)
    }
}

impl<T: crate::vec::StableElement + TryClone> TryClone for crate::boxed::StableBox<T> {
    fn try_clone(&self) -> AbiResult<Self> {
        if self.is_empty() {
            return Ok(Self::empty());
        }
        Self::try_new_with_allocator(TryClone::try_clone(&**self)?, self.allocator())
    }
}

impl<T: TryClone> TryClone for crate::option::StableOption<T> {
    fn try_clone(&self) -> AbiResult<Self> {
        match self.as_ref() {
            Some(value) => Ok(Self::some(TryClone::try_clone(value)?)),
            None => Ok(Self::none()),
        }
    }
}

impl<T: TryClone, E: TryClone> TryClone for crate::result::StableResult<T, E> {
    fn try_clone(&self) -> AbiResult<Self> {
        match self.as_ref() {
            Ok(value) => Ok(Self::ok(TryClone::try_clone(value)?)),
            Err(error) => Ok(Self::err(TryClone::try_clone(error)?)),
        }
    }
}

impl<A: TryClone, B: TryClone> TryClone for crate::tuple::StablePair<A, B> {
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(Self::new(
            TryClone::try_clone(&self.left)?,
            TryClone::try_clone(&self.right)?,
        ))
    }
}

impl<A: TryClone, B: TryClone, C: TryClone> TryClone for crate::tuple::StableTriple<A, B, C> {
    fn try_clone(&self) -> AbiResult<Self> {
        let (first, second, third) = self.as_ref();
        Ok(Self::new(
            TryClone::try_clone(first)?,
            TryClone::try_clone(second)?,
            TryClone::try_clone(third)?,
        ))
    }
}

impl<A: TryClone, B: TryClone, C: TryClone, D: TryClone> TryClone
    for crate::tuple::StableQuad<A, B, C, D>
{
    fn try_clone(&self) -> AbiResult<Self> {
        let (first, second, third, fourth) = self.as_ref();
        Ok(Self::new(
            TryClone::try_clone(first)?,
            TryClone::try_clone(second)?,
            TryClone::try_clone(third)?,
            TryClone::try_clone(fourth)?,
        ))
    }
}

impl<K, V> TryClone for crate::map::StableMap<K, V>
where
    K: crate::vec::StableElement + TryClone + Ord,
    V: crate::vec::StableElement + TryClone,
{
    fn try_clone(&self) -> AbiResult<Self> {
        // The source is in key order, so the copy is one push per entry rather
        // than one search per entry.
        let mut entries =
            crate::vec::StableVec::with_capacity_and_allocator(self.allocator(), self.len())?;
        for (key, value) in self.iter() {
            entries.try_push(crate::tuple::StablePair::new(
                TryClone::try_clone(key)?,
                TryClone::try_clone(value)?,
            ))?;
        }
        Ok(Self::from_sorted_entries(entries))
    }
}

impl<T> TryClone for crate::set::StableSet<T>
where
    T: crate::vec::StableElement + TryClone + Ord,
{
    fn try_clone(&self) -> AbiResult<Self> {
        let mut values =
            crate::vec::StableVec::with_capacity_and_allocator(self.allocator(), self.len())?;
        for value in self.iter() {
            values.try_push(TryClone::try_clone(value)?)?;
        }
        Ok(Self::from_sorted_values(values))
    }
}

// The stored number of an entry is copied with the entry: it names where the
// entry sits, and recomputing it would need the function the caller built the
// map with.
impl<K, V> TryClone for crate::hash::StableHashMap<K, V>
where
    K: crate::vec::StableElement + TryClone + Ord,
    V: crate::vec::StableElement + TryClone,
{
    fn try_clone(&self) -> AbiResult<Self> {
        crate::hash::StableHashMap::try_clone(self)
    }
}

impl<T> TryClone for crate::hash::StableHashSet<T>
where
    T: crate::vec::StableElement + TryClone + Ord,
{
    fn try_clone(&self) -> AbiResult<Self> {
        crate::hash::StableHashSet::try_clone(self)
    }
}

impl<K, V> TryClone for crate::hash::StableHashEntry<K, V>
where
    K: crate::vec::StableElement + TryClone,
    V: crate::vec::StableElement + TryClone,
{
    /// Copies the key, the value and the number the entry is stored under.
    fn try_clone(&self) -> AbiResult<Self> {
        Ok(Self::new(
            self.hash,
            TryClone::try_clone(&self.key)?,
            TryClone::try_clone(&self.value)?,
        ))
    }
}
