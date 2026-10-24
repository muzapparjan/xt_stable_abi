//! Hashing that answers the same number in every build.
//!
//! A hash container is ordered by the numbers it stores, so two builds that
//! number the same key differently cannot read each other's map: the entries are
//! sorted, and a search for a key whose number differs walks past it. The
//! containers accept any `Fn(&K) -> u64` because a hash over a key of your own
//! is yours to write, and that function is then part of the protocol between the
//! two sides: [`StableHashMap::validate_with`](crate::hash::StableHashMap::validate_with)
//! is the check, and it needs the same function on both sides.
//!
//! This trait is the other way to answer: the number is computed by this crate,
//! so both sides agree by construction, and a map built with it can be checked by
//! the receiver on its own ([`StableHashMap::validate_stable`](crate::hash::StableHashMap::validate_stable)).
//!
//! The number is FNV-1a over an encoding that names the width and the byte order
//! of each value, so it does not change with the target: a 32-bit build and a
//! 64-bit build hash the same key to the same number. `usize` and `isize` are not
//! covered, because their width is the target's.
//!
//! A domain enumeration gets its implementation from the declaration that writes
//! it: `derives = [StableHash];` folds the discriminant and the payload, so a
//! value of that type is a key a receiver can check on its own.
//!
//! # Implementing this trait for a type of your own
//!
//! The contract is that every build of both sides answers the same number for
//! equal values: two values that compare equal must fold to one number, which is
//! the same rule [`Hash`](core::hash::Hash) carries. Writing an implementation
//! that reads the target's width, the address of a value, or the order of memory
//! a compiler happens to choose breaks that, and the failure is a map in which two
//! equal keys answer different numbers — which a receiver cannot detect, because
//! it looks a key up by the number it computes.
//!
//! Numbers that differ for values that differ cannot be promised by any 64-bit
//! fold: collisions exist. They are not a correctness problem here, because an
//! entry is identified by `(hash, key)` and the search compares the key after the
//! number, so a collision costs one comparison rather than merging two entries.
//!
//! # What has an implementation
//!
//! Integers of every width, `bool`, `char`, `()`, the checked wrappers
//! ([`StableBool`](crate::primitive::StableBool),
//! [`StableChar`](crate::primitive::StableChar)), text and names
//! ([`StableStr`](crate::string::StableStr),
//! [`StableString`](crate::string::StableString),
//! [`StableCowStr`](crate::cow::StableCowStr),
//! [`StableOsStr`](crate::path::StableOsStr),
//! [`StableOsString`](crate::path::StableOsString),
//! [`StablePath`](crate::path::StablePath),
//! [`StablePathBuf`](crate::path::StablePathBuf)), readings
//! ([`AbiVersion`](crate::version::AbiVersion),
//! [`AbiStatus`](crate::error::AbiStatus),
//! [`StableDuration`](crate::duration::StableDuration), and, with the `std`
//! feature, `StableSystemTime` and `StableInstant`), groups (a one-tuple, a
//! pair, a triple, a quad, an array, and the stored form of an enumeration
//! payload), choices ([`StableOption`](crate::option::StableOption),
//! [`StableResult`](crate::result::StableResult)), and any enumeration whose
//! declaration asks for it with `derives = [StableHash]`.
//!
//! Deliberately absent: `f32`/`f64`, because a number with two bit patterns for
//! zero and a bit pattern that is not a number cannot answer "same value, same
//! number" without a rule the caller has to agree with; the containers and
//! wrappers that iterate or own storage, because a key that is a collection is
//! unusual enough that the caller should say how two of them are the same value;
//! and everything whose layout depends on the target or the process
//! ([`Handle`](crate::handle::Handle),
//! [`AbiAllocator`](crate::alloc::AbiAllocator)).
//!
//! A key type of your own is hashed by implementing this trait, and the
//! implementation has to agree with that type's `PartialEq` for the same reason
//! `Hash` does: two values that compare equal must answer the same number.

/// The starting state of the fold this crate hashes with.
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Folds `bytes` into `state`.
///
/// This is FNV-1a over a 64-bit word: one multiply per byte, no table and no
/// state beyond the word itself.
pub const fn fold_bytes(state: u64, bytes: &[u8]) -> u64 {
    let mut hash = state;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// Folds one more number into `state`.
///
/// This is how a value made of several parts is hashed: each part is hashed on
/// its own and folded in here, so the order of the parts is part of the number.
pub const fn fold_word(state: u64, word: u64) -> u64 {
    fold_bytes(state, &word.to_le_bytes())
}

/// A value that answers the same hash number in every build.
pub trait StableHash {
    /// The number this value is hashed to.
    fn stable_hash(&self) -> u64;
}

macro_rules! hash_by_bytes {
    ($($type:ty),* $(,)?) => {
        $(
            impl StableHash for $type {
                fn stable_hash(&self) -> u64 {
                    fold_bytes(FNV_OFFSET, &self.to_le_bytes())
                }
            }
        )*
    };
}

hash_by_bytes!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128);

impl StableHash for bool {
    fn stable_hash(&self) -> u64 {
        fold_bytes(FNV_OFFSET, &[u8::from(*self)])
    }
}

impl StableHash for char {
    fn stable_hash(&self) -> u64 {
        // A `char` is a code point, which is a number below `0x11_0000`; the
        // width is written out so the number does not follow the target.
        fold_bytes(FNV_OFFSET, &(*self as u32).to_le_bytes())
    }
}

impl StableHash for () {
    fn stable_hash(&self) -> u64 {
        FNV_OFFSET
    }
}

impl StableHash for crate::primitive::StableBool {
    /// Folds the stored byte, so a value the type does not define still has a
    /// number of its own.
    fn stable_hash(&self) -> u64 {
        fold_bytes(FNV_OFFSET, &[self.raw()])
    }
}

impl StableHash for crate::primitive::StableChar {
    /// Folds the stored code point, valid or not.
    fn stable_hash(&self) -> u64 {
        fold_bytes(FNV_OFFSET, &self.raw().to_le_bytes())
    }
}

impl<A: StableHash> StableHash for (A,) {
    fn stable_hash(&self) -> u64 {
        fold_word(FNV_OFFSET, self.0.stable_hash())
    }
}

impl<'a> StableHash for crate::string::StableStr<'a> {
    /// Folds the bytes of the text.
    fn stable_hash(&self) -> u64 {
        fold_bytes(FNV_OFFSET, self.as_bytes())
    }
}

impl StableHash for crate::string::StableString {
    /// Folds the bytes of the text.
    fn stable_hash(&self) -> u64 {
        fold_bytes(FNV_OFFSET, self.as_bytes())
    }
}

impl StableHash for crate::cow::StableCowStr<'_> {
    /// Folds the bytes of the text, whichever form holds them: a borrowed value
    /// and an owned one that say the same thing are the same key.
    fn stable_hash(&self) -> u64 {
        fold_bytes(FNV_OFFSET, self.as_str().as_bytes())
    }
}

impl StableHash for crate::error::AbiStatus {
    /// Folds the code, which is the number the value is.
    fn stable_hash(&self) -> u64 {
        fold_word(FNV_OFFSET, self.code() as u64)
    }
}

impl<T: StableHash, const N: usize> StableHash for crate::array::StableArray<T, N> {
    /// Folds the length and then every element, in order.
    ///
    /// The length is part of the number so an array of three values and one of
    /// four do not answer the same one.
    fn stable_hash(&self) -> u64 {
        let mut state = fold_word(FNV_OFFSET, N as u64);
        for value in self.as_slice() {
            state = fold_word(state, value.stable_hash());
        }
        state
    }
}

#[cfg(feature = "std")]
impl StableHash for crate::path::StableOsStr<'_> {
    /// Folds the encoding and then the bytes of the name.
    ///
    /// Two names are the same value only when both agree, so both are part of
    /// the number here; the bytes alone would make two names the type says
    /// differ answer one number.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.encoding() as u64);
        fold_bytes(state, self.as_bytes())
    }
}

#[cfg(feature = "std")]
impl StableHash for crate::path::StableOsString {
    /// Folds the encoding and then the bytes of the name, as the borrowed form
    /// does, so a name answers the same number whether it arrived owned or
    /// borrowed.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.encoding() as u64);
        fold_bytes(state, self.as_bytes())
    }
}

#[cfg(feature = "std")]
impl StableHash for crate::path::StablePath<'_> {
    fn stable_hash(&self) -> u64 {
        self.as_stable_os_str().stable_hash()
    }
}

#[cfg(feature = "std")]
impl StableHash for crate::path::StablePathBuf {
    fn stable_hash(&self) -> u64 {
        self.as_stable_path().stable_hash()
    }
}

impl StableHash for crate::version::AbiVersion {
    /// Folds the two numbers the version is written as.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.major as u64);
        fold_word(state, self.minor as u64)
    }
}

impl StableHash for crate::duration::StableDuration {
    /// Folds the seconds and then the fraction.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.as_secs());
        fold_word(state, self.subsec_nanos() as u64)
    }
}

impl<T: StableHash> StableHash for crate::option::StableOption<T> {
    /// Folds a zero for the empty value and a one followed by the payload for a
    /// value that holds one, so the two cases do not answer the same number.
    fn stable_hash(&self) -> u64 {
        match self.as_ref() {
            Some(value) => fold_word(fold_word(FNV_OFFSET, 1), value.stable_hash()),
            None => fold_word(FNV_OFFSET, 0),
        }
    }
}

impl<T: StableHash, E: StableHash> StableHash for crate::result::StableResult<T, E> {
    /// Folds the tag and then the payload it names.
    fn stable_hash(&self) -> u64 {
        match self.as_ref() {
            Ok(value) => fold_word(fold_word(FNV_OFFSET, 2), value.stable_hash()),
            Err(error) => fold_word(fold_word(FNV_OFFSET, 3), error.stable_hash()),
        }
    }
}

#[cfg(feature = "std")]
impl StableHash for crate::time::StableSystemTime {
    /// Folds the seconds and then the fraction.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.seconds as u64);
        fold_word(state, self.nanos as u64)
    }
}

#[cfg(feature = "std")]
impl StableHash for crate::time::StableInstant {
    /// Folds the origin and then the reading, because a reading means nothing
    /// without the base it counts from.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.origin);
        fold_word(state, self.nanos)
    }
}

impl<A: StableHash, B: StableHash> StableHash for crate::tuple::StablePair<A, B> {
    /// Folds the two parts in order, each number taken on its own.
    ///
    /// Starting from [`FNV_OFFSET`] rather than from the first part's number is
    /// what makes a group and the stored form of an enumeration payload that
    /// holds the same values answer one number: both follow the rule
    /// [`fold_word`] documents, so a party that writes the rule from the
    /// documentation arrives at the same value.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.left.stable_hash());
        fold_word(state, self.right.stable_hash())
    }
}

impl<A: StableHash, B: StableHash, C: StableHash> StableHash
    for crate::tuple::StableTriple<A, B, C>
{
    /// Folds the three parts in order, each number taken on its own.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.first.stable_hash());
        let state = fold_word(state, self.second.stable_hash());
        fold_word(state, self.third.stable_hash())
    }
}

impl<A: StableHash, B: StableHash, C: StableHash, D: StableHash> StableHash
    for crate::tuple::StableQuad<A, B, C, D>
{
    /// Folds the four parts in order, each number taken on its own.
    fn stable_hash(&self) -> u64 {
        let state = fold_word(FNV_OFFSET, self.first.stable_hash());
        let state = fold_word(state, self.second.stable_hash());
        let state = fold_word(state, self.third.stable_hash());
        fold_word(state, self.fourth.stable_hash())
    }
}
