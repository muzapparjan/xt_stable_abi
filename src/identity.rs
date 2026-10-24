//! The stable number that names a boundary type.
//!
//! A slot is found by comparing one number, and that number has to be the same
//! in every build that agrees about the type. Deriving it from the text a
//! declaration was written as cannot promise that: `type Text = StableString;`
//! is different text for the same type, and so is `StableSlice<'_, u8>` for
//! `StableSlice<u8>`. Deriving it from the type itself removes the question,
//! because the compiler resolves an alias before the number is asked for.

use crate::object::LAYOUT_REVISION;

/// A type that names itself across the boundary.
///
/// The number is what a slot records and what another build compares, so it has
/// to be a property of the type rather than of the spelling: two builds that
/// agree about the layout produce the same number, and two types that differ in
/// layout produce different ones.
///
/// Every type of this crate implements it. A type of your own does too, with
/// [`unsafe_stable_identity!`](crate::unsafe_stable_identity), which names it by
/// its written name: a number it derives from that name.
///
/// # Safety
///
/// `IDENTIFIER` must be this type's own number: the same for the type in every
/// build, and different from the number of any type whose layout is not the
/// same. A wrong number lets a receiver call a trampoline written for another
/// type.
///
/// The number is a 64-bit hash, not a credential: it narrows a match rather than
/// proving one, and two declarations that agree on it are still compared by
/// their slots before a call is made. A number derived from a written name
/// (`unsafe_stable_identity!`) does not include the module path, so two types of
/// the same name in different modules produce the same number; a crate that
/// hands values to another crate under a name that is not unique across both
/// should spell a name it knows to be unique.
pub unsafe trait AbiIdentity {
    /// The number that names this type.
    const IDENTIFIER: u64;
}

/// Folds one word into `hash`.
const fn mix(hash: u64, value: u64) -> u64 {
    let mut hash = hash;
    let mut index = 0;
    while index < 8 {
        hash ^= (value >> (index * 8)) & 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

/// The identifier of a type with this kind and these children.
///
/// The kind separates two types that hold the same children (`StableVec<u8>` and
/// `StableBox<u8>`, say), and the children are folded in order so a container
/// names its element.
///
/// The numbers are part of the protocol: a table built without this crate has
/// to fold the same kind for a type as this crate does, and the kinds in use are
/// the numbers written at the calls below.
pub const fn fold(kind: u64, children: &[u64]) -> u64 {
    let mut hash = LAYOUT_REVISION ^ 0xcbf2_9ce4_8422_2325;
    hash = mix(hash, kind);
    let mut index = 0;
    while index < children.len() {
        hash = mix(hash, children[index]);
        index += 1;
    }
    hash
}

/// The identifier of a type named by `name`.
///
/// This is how a type outside this crate names itself: the number is a function
/// of the name its author wrote, so two sides that write the same name agree.
///
/// The name is read where the implementation is written, not where the type is
/// mentioned, so an alias of that type names the same number: `type Frame = MyFrame;`
/// has no implementation of its own, and every use of `Frame` resolves to the one
/// `MyFrame` carries.
pub const fn named(name: &str) -> u64 {
    fold(0, &[crate::object::declaration_id(name)])
}

/// The identifier of one variant: its discriminant and the shape it carries.
///
/// The discriminant is part of the layout, so two declarations that name the
/// same variant under different numbers are different types even when the
/// payloads agree.
pub const fn of_variant(tag: u32, payload: u64) -> u64 {
    fold(90, &[tag as u64, payload])
}

/// The identifier of a set of variants, in any order.
///
/// The order variants are written in is not part of the layout - the
/// discriminant of each one is written out - so two declarations that list the
/// same variants in a different order describe the same type.
pub const fn of_variants(kind: u64, variants: &[u64]) -> u64 {
    let mut sum: u64 = 0;
    let mut twist: u64 = 0;
    let mut index = 0;
    while index < variants.len() {
        let mixed = mix(variants[index] ^ 0x9e37_79b9_7f4a_7c15, kind);
        sum = sum.wrapping_add(mixed);
        twist ^= mixed.rotate_left(23);
        index += 1;
    }
    fold(kind, &[variants.len() as u64]) ^ sum ^ twist
}

/// The shape a variant's payload describes.
///
/// A payload is written as a tuple and stored in the form
/// [`PayloadShape`](crate::enumeration::PayloadShape) names: one field stays a
/// one-tuple, and two or more become a `#[repr(C)]` struct of the same fields.
/// The number folds the stored shape and the identity of each field, so two
/// declarations whose variants carry different types answer differently, and a
/// declaration that changed how a payload is laid out would too.
pub trait PayloadIdentity {
    /// The number that names this payload shape.
    const IDENTIFIER: u64;
}

macro_rules! payload_identity {
    ($(($($name:ident),*)),* $(,)?) => {
        $(
            // SAFETY: the number counts the fields and folds the identity of
            // each one, so it names the shape rather than a value.
            impl<$($name: AbiIdentity),*> PayloadIdentity for ($($name,)*) {
                const IDENTIFIER: u64 = fold(92, &[$($name::IDENTIFIER),*]);
            }
        )*
    };
}

payload_identity!(
    (),
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
    (A, B, C, D, E, F, G),
    (A, B, C, D, E, F, G, H),
    (A, B, C, D, E, F, G, H, I),
    (A, B, C, D, E, F, G, H, I, J),
    (A, B, C, D, E, F, G, H, I, J, K),
    (A, B, C, D, E, F, G, H, I, J, K, L)
);

macro_rules! identity_of_leaves {
    ($($kind:expr => $type:ty),* $(,)?) => {
        $(
            // SAFETY: the number folds the kind of the type and nothing else, so
            // it is a property of this type alone.
            unsafe impl AbiIdentity for $type {
                const IDENTIFIER: u64 = fold($kind, &[]);
            }
        )*
    };
}

identity_of_leaves! {
    1u64 => u8,
    2u64 => u16,
    3u64 => u32,
    4u64 => u64,
    5u64 => u128,
    6u64 => i8,
    7u64 => i16,
    8u64 => i32,
    9u64 => i64,
    10u64 => i128,
    11u64 => f32,
    12u64 => f64,
    13u64 => (),
    14u64 => crate::primitive::StableBool,
    15u64 => crate::primitive::StableChar,
    16u64 => crate::string::StableString,
    17u64 => crate::object::StableDyn,
    18u64 => crate::duration::StableDuration,
    23u64 => crate::version::AbiVersion,
    24u64 => crate::handle::Handle,
    25u64 => crate::error::AbiStatus,
    26u64 => crate::error::AbiError,
}

#[cfg(feature = "std")]
identity_of_leaves! {
    19u64 => crate::path::StableOsString,
    20u64 => crate::path::StablePathBuf,
    21u64 => crate::time::StableSystemTime,
    22u64 => crate::time::StableInstant,
}

macro_rules! identity_of_lifetime {
    ($($kind:expr => $type:ty),* $(,)?) => {
        $(
            // SAFETY: a lifetime names no part of the layout, so the number is a
            // property of the type alone.
            unsafe impl<'a> AbiIdentity for $type {
                const IDENTIFIER: u64 = fold($kind, &[]);
            }
        )*
    };
}

identity_of_lifetime! {
    30u64 => crate::string::StableStr<'a>,
    33u64 => crate::cow::StableCowStr<'a>,
}

#[cfg(feature = "std")]
identity_of_lifetime! {
    31u64 => crate::path::StableOsStr<'a>,
    32u64 => crate::path::StablePath<'a>,
}

/// The identifier of a container that holds one child.
const fn of_one(kind: u64, child: u64) -> u64 {
    fold(kind, &[child])
}

/// The identifier of a container that holds two children.
const fn of_two(kind: u64, first: u64, second: u64) -> u64 {
    fold(kind, &[first, second])
}

// SAFETY: the kind of the container and the number of its element are both
// folded in, so the number names the shape the container holds.
unsafe impl<T: AbiIdentity + crate::vec::StableElement> AbiIdentity
    for crate::vec::StableVec<T>
{
    const IDENTIFIER: u64 = of_one(40, T::IDENTIFIER);
}

// SAFETY: the kind and the element are both folded in.
unsafe impl<T: AbiIdentity + crate::vec::StableElement> AbiIdentity for crate::boxed::StableBox<T> {
    const IDENTIFIER: u64 = of_one(41, T::IDENTIFIER);
}

// SAFETY: the kind and the payload are both folded in.
unsafe impl<T: AbiIdentity> AbiIdentity for crate::option::StableOption<T> {
    const IDENTIFIER: u64 = of_one(42, T::IDENTIFIER);
}

// SAFETY: the kind and the element are both folded in.
unsafe impl<T: AbiIdentity + crate::vec::StableElement> AbiIdentity for crate::set::StableSet<T> {
    const IDENTIFIER: u64 = of_one(43, T::IDENTIFIER);
}

// SAFETY: the kind and the element are both folded in.
unsafe impl<T: AbiIdentity + crate::vec::StableElement + Ord> AbiIdentity
    for crate::hash::StableHashSet<T>
{
    const IDENTIFIER: u64 = of_one(44, T::IDENTIFIER);
}

// SAFETY: a lifetime names no part of the layout, and the element is folded in.
unsafe impl<'a, T: AbiIdentity + crate::vec::StableElement> AbiIdentity
    for crate::vec::StableSlice<'a, T>
{
    const IDENTIFIER: u64 = of_one(50, T::IDENTIFIER);
}

// SAFETY: a lifetime names no part of the layout, and the element is folded in.
unsafe impl<'a, T: AbiIdentity + crate::vec::StableElement> AbiIdentity
    for crate::vec::StableSliceMut<'a, T>
{
    const IDENTIFIER: u64 = of_one(51, T::IDENTIFIER);
}

// SAFETY: a lifetime names no part of the layout, and the element is folded in.
unsafe impl<'a, T: AbiIdentity + crate::vec::StableElement> AbiIdentity
    for crate::cow::StableCowSlice<'a, T>
{
    const IDENTIFIER: u64 = of_one(52, T::IDENTIFIER);
}

// SAFETY: both children are folded in.
unsafe impl<A: AbiIdentity, B: AbiIdentity> AbiIdentity for crate::tuple::StablePair<A, B> {
    const IDENTIFIER: u64 = of_two(60, A::IDENTIFIER, B::IDENTIFIER);
}

// SAFETY: both children are folded in.
unsafe impl<T: AbiIdentity, E: AbiIdentity> AbiIdentity for crate::result::StableResult<T, E> {
    const IDENTIFIER: u64 = of_two(61, T::IDENTIFIER, E::IDENTIFIER);
}

// SAFETY: both children are folded in.
unsafe impl<K: AbiIdentity + crate::vec::StableElement, V: AbiIdentity + crate::vec::StableElement>
    AbiIdentity for crate::map::StableMap<K, V>
{
    const IDENTIFIER: u64 = of_two(62, K::IDENTIFIER, V::IDENTIFIER);
}

// SAFETY: both children are folded in.
unsafe impl<K: AbiIdentity + crate::vec::StableElement, V: AbiIdentity + crate::vec::StableElement>
    AbiIdentity for crate::hash::StableHashMap<K, V>
{
    const IDENTIFIER: u64 = of_two(63, K::IDENTIFIER, V::IDENTIFIER);
}

// SAFETY: both children are folded in.
unsafe impl<K: AbiIdentity, V: AbiIdentity> AbiIdentity for crate::hash::StableHashEntry<K, V> {
    const IDENTIFIER: u64 = of_two(64, K::IDENTIFIER, V::IDENTIFIER);
}

// SAFETY: the kind and all three children are folded in.
unsafe impl<A: AbiIdentity, B: AbiIdentity, C: AbiIdentity> AbiIdentity
    for crate::tuple::StableTriple<A, B, C>
{
    const IDENTIFIER: u64 = fold(70, &[A::IDENTIFIER, B::IDENTIFIER, C::IDENTIFIER]);
}

// SAFETY: the kind and all four children are folded in.
unsafe impl<A: AbiIdentity, B: AbiIdentity, C: AbiIdentity, D: AbiIdentity> AbiIdentity
    for crate::tuple::StableQuad<A, B, C, D>
{
    const IDENTIFIER: u64 = fold(
        71,
        &[A::IDENTIFIER, B::IDENTIFIER, C::IDENTIFIER, D::IDENTIFIER],
    );
}

// SAFETY: the element and the length are both part of the type, so both are
// folded in.
unsafe impl<T: AbiIdentity, const N: usize> AbiIdentity for crate::array::StableArray<T, N> {
    const IDENTIFIER: u64 = fold(80, &[T::IDENTIFIER, N as u64]);
}
