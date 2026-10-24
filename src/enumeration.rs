//! Enumerations whose variants and payloads have a declared layout.
//!
//! A Rust `enum` has no layout another build can rely on, so a domain
//! enumeration that crosses the boundary is written out as a tag and a payload.
//! [`stable_enum!`](crate::stable_enum) writes that form for you: the tag is the
//! discriminant you wrote, the payload of each variant is a field of the union
//! you named, and the constructors are named after the variants.
//!
//! Reading a payload goes through a second declaration rather than a method named
//! after the variant, because a constructor and an accessor of the same name
//! cannot both exist on one type. The generated view is a Rust `enum` of
//! references; it never crosses the boundary, so its layout is free to be Rust's.
//!
//! Two things are part of the ABI and are therefore written out rather than
//! derived: each variant's discriminant, and the name of the payload union.
//! Counting variants in declaration order would change every later tag the day a
//! variant is inserted, and `macro_rules` cannot build a type name out of another
//! one, so the union is named where it is declared.
//!
//! A payload of one field is held as that field's one-tuple, whose size,
//! alignment and offset have nothing left for a compiler to choose. A payload of
//! two or more fields is held as a `#[repr(C)]` struct of the same fields in the
//! same order ([`Payload2`], [`Payload3`], and so on), so every build places them
//! at the offsets the C rules give. A declaration that names an enumeration in a
//! method signature says where the payload meets an `extern` function that the
//! layout is the one this crate meant: the lint that checks those signatures is
//! told so. (`tests/enumeration.rs` writes that attribute on the methods it
//! declares.)

use crate::error::AbiResult;

/// Validates every element of a payload.
///
/// A payload is a tuple whose arity differs per variant; this trait is what lets
/// one generated `validate` cover all of them.
pub trait ValidateElements {
    /// Reports whether every element is a value its type defines.
    fn validate_elements(&self) -> AbiResult<()>;
}

/// Overwrites every element of a payload.
pub trait WipeElements {
    /// Overwrites the content of every element.
    fn wipe_elements(&mut self);
}

macro_rules! impl_payload_traits {
    ($(($($name:ident),*)),* $(,)?) => {
        $(
            impl<$($name: crate::boundary::AbiType),*> ValidateElements for ($($name,)*) {
                fn validate_elements(&self) -> AbiResult<()> {
                    #[allow(non_snake_case)]
                    let ($($name,)*) = self;
                    $(crate::boundary::check($name)?;)*
                    Ok(())
                }
            }

            impl<$($name: crate::zeroize::Wipe),*> WipeElements for ($($name,)*) {
                fn wipe_elements(&mut self) {
                    #[allow(non_snake_case)]
                    let ($($name,)*) = self;
                    $($name.wipe_in_place();)*
                }
            }
        )*
    };
}

/// Implements [`TryClone`](crate::clone::TryClone) for a payload of one or more
/// elements.
///
/// The empty payload is not here: [`TryClone`](crate::clone::TryClone) is
/// already implemented for `()`.
macro_rules! impl_payload_clone {
    ($(($($name:ident),+)),+ $(,)?) => {
        $(
            impl<$($name: crate::clone::TryClone),+> crate::clone::TryClone
                for ($($name,)+)
            {
                fn try_clone(&self) -> AbiResult<Self> {
                    #[allow(non_snake_case)]
                    let ($($name,)+) = self;
                    Ok(($(crate::clone::TryClone::try_clone($name)?,)+))
                }
            }
        )+
    };
}

/// Reports a payload of no elements as held, and wipes nothing in it.
impl ValidateElements for () {
    fn validate_elements(&self) -> AbiResult<()> {
        Ok(())
    }
}

impl WipeElements for () {
    fn wipe_elements(&mut self) {}
}

impl_payload_clone!(
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

impl_payload_traits!(
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

/// Reports whether every discriminant in `tags` appears once.
///
/// A declaration that names one discriminant twice would give two variants the
/// same tag, and the first arm that matches would read the payload of the other
/// one. A declaration checks the list it writes with a `const` assertion, so the
/// mistake is refused where it is written.
pub const fn tags_are_distinct(tags: &[u32]) -> bool {
    let mut index = 0;
    while index < tags.len() {
        let mut other = index + 1;
        while other < tags.len() {
            if tags[index] == tags[other] {
                return false;
            }
            other += 1;
        }
        index += 1;
    }
    true
}

/// How the fields of a payload are stored.
///
/// One field needs no decision: a one-tuple has that field's size, alignment and
/// offset, so nothing about it is the compiler's to choose. Two or more fields
/// are stored as one of the `#[repr(C)]` structs beside this trait, because the
/// layout of a Rust tuple is the compiler's to choose and two builds may place
/// the same fields at different offsets.
///
/// The view of a variant hands out a reference to the stored form, whose fields
/// are the payload's positions: `.0`, `.1`, and so on.
#[diagnostic::on_unimplemented(
    message = "a variant payload is written as a tuple of at most twelve values",
    label = "this is not a tuple of one to twelve values",
    note = "a payload with more values has no stored form, and a payload written as a single value is written as a one-tuple: `(Value,)`"
)]
pub trait PayloadShape {
    /// The type the payload is stored as.
    type Stored;
    /// Converts the payload from the tuple the declaration writes into the form
    /// it is stored as.
    fn store(self) -> Self::Stored;
}

impl<A> PayloadShape for (A,) {
    type Stored = (A,);

    fn store(self) -> (A,) {
        self
    }
}

/// The type a payload written as `Shape` is stored as.
pub type Stored<Shape> = <Shape as PayloadShape>::Stored;

macro_rules! payload_structs {
    ($(($name:ident, $(($index:tt $field:ident)),+)),+ $(,)?) => {
        $(
            impl<$($field),+> PayloadShape for ($($field,)+) {
                type Stored = $name<$($field),+>;

                fn store(self) -> Self::Stored {
                    #[allow(non_snake_case)]
                    let ($($field,)+) = self;
                    $name($($field),+)
                }
            }

            impl<$($field: crate::boundary::AbiType),+> ValidateElements
                for $name<$($field),+>
            {
                fn validate_elements(&self) -> AbiResult<()> {
                    #[allow(non_snake_case)]
                    let $name($($field,)+) = self;
                    $(crate::boundary::check($field)?;)+
                    Ok(())
                }
            }

            impl<$($field: crate::zeroize::Wipe),+> WipeElements for $name<$($field),+> {
                fn wipe_elements(&mut self) {
                    #[allow(non_snake_case)]
                    let $name($($field,)+) = self;
                    $($field.wipe_in_place();)+
                }
            }

            impl<$($field: crate::clone::TryClone),+> crate::clone::TryClone
                for $name<$($field),+>
            {
                fn try_clone(&self) -> AbiResult<Self> {
                    Ok($name($(crate::clone::TryClone::try_clone(&self.$index)?),+))
                }
            }

            impl<$($field: ::core::cmp::PartialEq),+> ::core::cmp::PartialEq
                for $name<$($field),+>
            {
                fn eq(&self, other: &Self) -> bool {
                    true $(&& self.$index == other.$index)+
                }
            }

            impl<$($field: ::core::cmp::Eq),+> ::core::cmp::Eq for $name<$($field),+> {}

            impl<$($field: ::core::cmp::PartialOrd),+> ::core::cmp::PartialOrd
                for $name<$($field),+>
            {
                fn partial_cmp(&self, other: &Self) -> ::core::option::Option<::core::cmp::Ordering> {
                    let mut order = ::core::option::Option::Some(::core::cmp::Ordering::Equal);
                    $(
                        if order == ::core::option::Option::Some(::core::cmp::Ordering::Equal) {
                            order = ::core::cmp::PartialOrd::partial_cmp(
                                &self.$index,
                                &other.$index,
                            );
                        }
                    )+
                    order
                }
            }

            impl<$($field: ::core::cmp::Ord),+> ::core::cmp::Ord for $name<$($field),+> {
                fn cmp(&self, other: &Self) -> ::core::cmp::Ordering {
                    let mut order = ::core::cmp::Ordering::Equal;
                    $(
                        if order == ::core::cmp::Ordering::Equal {
                            order = ::core::cmp::Ord::cmp(&self.$index, &other.$index);
                        }
                    )+
                    order
                }
            }

            impl<$($field: ::core::hash::Hash),+> ::core::hash::Hash for $name<$($field),+> {
                fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) {
                    $(::core::hash::Hash::hash(&self.$index, state);)+
                }
            }

            impl<$($field: crate::stable_hash::StableHash),+> crate::stable_hash::StableHash
                for $name<$($field),+>
            {
                fn stable_hash(&self) -> u64 {
                    let mut state = crate::stable_hash::FNV_OFFSET;
                    $(
                        state = crate::stable_hash::fold_word(
                            state,
                            crate::stable_hash::StableHash::stable_hash(&self.$index),
                        );
                    )+
                    state
                }
            }

            // SAFETY: the number names the stored shape, which is this struct
            // rather than a tuple, so a declaration that changes how its payload
            // is stored answers with a different number.
            impl<$($field: crate::identity::AbiIdentity),+> crate::identity::PayloadIdentity
                for $name<$($field),+>
            {
                const IDENTIFIER: u64 =
                    crate::identity::fold(94, &[$($field::IDENTIFIER),+]);
            }
        )+
    };
}

/// A payload of two fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload2<A, B>(pub A, pub B);

/// A payload of three fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload3<A, B, C>(pub A, pub B, pub C);

/// A payload of four fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload4<A, B, C, D>(pub A, pub B, pub C, pub D);

/// A payload of five fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload5<A, B, C, D, E>(pub A, pub B, pub C, pub D, pub E);

/// A payload of six fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload6<A, B, C, D, E, F>(pub A, pub B, pub C, pub D, pub E, pub F);

/// A payload of seven fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload7<A, B, C, D, E, F, G>(pub A, pub B, pub C, pub D, pub E, pub F, pub G);

/// A payload of eight fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload8<A, B, C, D, E, F, G, H>(
    pub A,
    pub B,
    pub C,
    pub D,
    pub E,
    pub F,
    pub G,
    pub H,
);

/// A payload of nine fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload9<A, B, C, D, E, F, G, H, I>(
    pub A,
    pub B,
    pub C,
    pub D,
    pub E,
    pub F,
    pub G,
    pub H,
    pub I,
);

/// A payload of ten fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload10<A, B, C, D, E, F, G, H, I, J>(
    pub A,
    pub B,
    pub C,
    pub D,
    pub E,
    pub F,
    pub G,
    pub H,
    pub I,
    pub J,
);

/// A payload of eleven fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload11<A, B, C, D, E, F, G, H, I, J, K>(
    pub A,
    pub B,
    pub C,
    pub D,
    pub E,
    pub F,
    pub G,
    pub H,
    pub I,
    pub J,
    pub K,
);

/// A payload of twelve fields, laid out in the order the declaration writes them.
#[repr(C)]
#[derive(Debug)]
pub struct Payload12<A, B, C, D, E, F, G, H, I, J, K, L>(
    pub A,
    pub B,
    pub C,
    pub D,
    pub E,
    pub F,
    pub G,
    pub H,
    pub I,
    pub J,
    pub K,
    pub L,
);

payload_structs!(
    (Payload2, (0 A), (1 B)),
    (Payload3, (0 A), (1 B), (2 C)),
    (Payload4, (0 A), (1 B), (2 C), (3 D)),
    (Payload5, (0 A), (1 B), (2 C), (3 D), (4 E)),
    (Payload6, (0 A), (1 B), (2 C), (3 D), (4 E), (5 F)),
    (Payload7, (0 A), (1 B), (2 C), (3 D), (4 E), (5 F), (6 G)),
    (Payload8, (0 A), (1 B), (2 C), (3 D), (4 E), (5 F), (6 G), (7 H)),
    (Payload9, (0 A), (1 B), (2 C), (3 D), (4 E), (5 F), (6 G), (7 H), (8 I)),
    (
        Payload10,
        (0 A), (1 B), (2 C), (3 D), (4 E), (5 F), (6 G), (7 H), (8 I), (9 J)
    ),
    (
        Payload11,
        (0 A), (1 B), (2 C), (3 D), (4 E), (5 F), (6 G), (7 H), (8 I), (9 J), (10 K)
    ),
    (
        Payload12,
        (0 A), (1 B), (2 C), (3 D), (4 E), (5 F), (6 G), (7 H), (8 I), (9 J), (10 K),
        (11 L)
    ),
);

/// Declares an enumeration with a tag and one payload per variant.
///
/// ```
/// use xt_stable_abi::stable_enum;
///
/// stable_enum! {
///     /// A shape.
///     pub enum Shape {
///         /// The union the payloads are laid over.
///         payload = ShapePayload;
///         /// The form the payloads are read through.
///         view = ShapeView;
///
///         /// Carries nothing.
///         Empty = 0,
///         /// One number.
///         Circle(f64) = 1,
///     }
/// }
///
/// let shape = Shape::Circle((1.5,));
/// assert_eq!(shape.tag(), 1);
/// match shape.view() {
///     ShapeView::Circle(radius) => assert_eq!(*radius, (1.5,)),
///     ShapeView::Empty(_) => panic!("the payload names the other variant"),
/// }
/// ```
///
/// Each variant gets a constructor of its own name and a field of the same name
/// in the generated view. The payload is written as a tuple at the constructor
/// and read as a reference to the form it is stored in, whose fields are the
/// tuple's positions, so a declaration needs no field names and the macro never
/// has to build an identifier out of another one. The bytes no variant occupies
/// are zeroed when a value is built, so a value carries nothing of the stack it
/// was built on.
///
/// The discriminant is part of the ABI: it is what the other side compares, so it
/// cannot be derived from the order the variants happen to be written in.
///
/// The number that names the type folds the name, every discriminant this build
/// compiled and the shape each payload carries, so two declarations that agree
/// on the name and disagree on what it holds answer with different numbers:
/// a receiver refuses the second instead of reading its bytes through the first
/// one's layout. The name is the one written here, not a module path, so two
/// declarations of one name in two modules answer with one number: a name that
/// crosses a boundary has to be unique on both sides.
///
/// The declaration also writes an implementation of
/// [`Wipe`](crate::zeroize::Wipe), which covers every payload; a payload type
/// that has no wipe of its own therefore stops the declaration from compiling.
/// [`stable_enum_unwiped!`](crate::stable_enum_unwiped) declares the same type
/// without that implementation, which is what a payload such as a borrowed view
/// needs.
///
/// An attribute before a variant lands in two places: the places that store and
/// read the variant accept `#[cfg]`, and the function that builds it accepts any
/// attribute a function does. A `#[cfg_attr]` therefore reaches only the
/// constructor, because what it expands to is not known here and a field accepts
/// a narrower set of attributes than a function: gate a variant with `#[cfg]`.
///
/// An attribute before `enum` belongs to the type and to nothing the macro
/// generates beside it. To gate a whole declaration, put the attribute on the
/// invocation, where it covers everything the macro writes:
///
/// ```ignore
/// #[cfg(feature = "extra")]
/// stable_enum! {
///     pub enum Extra { /* ... */ }
/// }
/// ```
///
/// # Traits the declaration asks for
///
/// A value of this type is a tag and a union, so `#[derive]` is no use: it would
/// read the union rather than the payload the tag selects. The declaration names
/// the traits instead, and the macro writes one implementation per name:
///
/// ```ignore
/// stable_enum! {
///     pub enum Suited {
///         payload = SuitedPayload;
///         view = SuitedView;
///         derives = [Clone, PartialEq, Eq, PartialOrd, Ord, Hash];
///         default = Empty;
///         Empty = 0,
///         Count(u32) = 1,
///     }
/// }
/// ```
///
/// `Clone` writes [`TryClone`](crate::clone::TryClone) as well, because a payload
/// may need to allocate. `Default` needs the variant to build: `default = Empty;`
/// names it, and the variant has to carry no payload. `StableHash` writes
/// [`StableHash`](crate::stable_hash::StableHash), which is what makes a value
/// usable as the key of a map whose numbers this crate computes
/// ([`StableHashMap::insert_stable`](crate::hash::StableHashMap::insert_stable)):
/// it folds the discriminant and the payload, so two values of one variant differ
/// where `Hash` with a caller's hasher would need the caller to agree with the
/// other side. The macros write exactly the names that are listed, as `#[derive]`
/// does, so `Eq` needs `PartialEq` in the list and `Ord` needs `Eq` and
/// `PartialOrd`.
///
/// `Eq` also needs `Ord`, which is the one pairing this macro checks. Equality
/// being a total relation is a property of the payload types, and the ordering is
/// the only property of them the macro can have the compiler verify: the
/// generated `Ord` compares the payloads, so it fails to build unless every one
/// of them is ordered. A declaration that names `Eq` without `Ord` is refused
/// rather than given an `Eq` a float payload could make false:
///
/// ```compile_fail
/// use xt_stable_abi::stable_enum;
///
/// stable_enum! {
///     /// A measurement that asks for equality without an order.
///     pub enum Reading {
///         /// The union.
///         payload = ReadingPayload;
///         /// The view.
///         view = ReadingView;
///         derives = [PartialEq, Eq];
///
///         /// One number, which may be a NaN.
///         Value(f64) = 0,
///     }
/// }
/// ```
///
/// A declaration whose payloads are equal to themselves without being ordered
/// writes the `impl Eq` itself.
#[macro_export]
macro_rules! stable_enum {
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $name:ident {
            $(#[$union_meta:meta])*
            payload = $union:ident;
            $(#[$view_meta:meta])*
            view = $view:ident;
            $($variants:tt)*
        }
    ) => {
        $crate::__stable_enum_split! {
            [$($enum_meta)*]
            [$vis $name]
            [$($union_meta)*]
            [$union]
            [$($view_meta)*]
            [$view]
            [wipe]
            []
            []
            [$($variants)*]
        }
    };
}

/// Declares an enumeration that cannot wipe its payload.
///
/// [`stable_enum!`] writes an implementation of
/// [`Wipe`](crate::zeroize::Wipe) beside the type, which means every payload
/// type has to implement `Wipe` itself. A payload that cannot be wiped - a
/// borrowed view, a handle, a type of your own without an implementation -
/// therefore stops that declaration from compiling.
///
/// This macro declares the same type without that implementation. Everything
/// else is identical: the tag, the payload union, the constructors, the view,
/// the destructor and [`AbiType`](crate::boundary::AbiType). A declaration that
/// does not need to wipe its payload is one that stores no secret in it.
///
/// ```ignore
/// stable_enum_unwiped! {
///     pub enum Shape {
///         payload = ShapePayload;
///         view = ShapeView;
///         Empty = 0,
///         Label(StableStr<'static>) = 1,
///     }
/// }
/// ```
#[macro_export]
macro_rules! stable_enum_unwiped {
    (
        $(#[$enum_meta:meta])*
        $vis:vis enum $name:ident {
            $(#[$union_meta:meta])*
            payload = $union:ident;
            $(#[$view_meta:meta])*
            view = $view:ident;
            $($variants:tt)*
        }
    ) => {
        $crate::__stable_enum_split! {
            [$($enum_meta)*]
            [$vis $name]
            [$($union_meta)*]
            [$union]
            [$($view_meta)*]
            [$view]
            []
            []
            []
            [$($variants)*]
        }
    };
}

/// Implements [`PanicFallback`](crate::boundary::PanicFallback) for an
/// enumeration declared by [`stable_enum!`].
///
/// A guarded table answers with a value of the method's return type when the
/// implementation panics. An enumeration has no value that means "no answer", so
/// the declaration names the one to use instead: one of its own variants, given
/// the payload that marks it.
///
/// ```
/// use xt_stable_abi::{stable_enum, stable_enum_fallback};
///
/// stable_enum! {
///     /// The answer of a call.
///     pub enum Answer {
///         /// The union the payloads are laid over.
///         payload = AnswerPayload;
///         /// The form the payloads are read through.
///         view = AnswerView;
///
///         /// No answer.
///         Nothing = 0,
///         /// A number.
///         Number(u64) = 1,
///     }
/// }
///
/// stable_enum_fallback!(Answer => Nothing());
/// ```
#[macro_export]
macro_rules! stable_enum_fallback {
    ($enum:ty => $variant:ident ()) => {
        impl $crate::boundary::PanicFallback for $enum {
            fn panic_value() -> Self {
                <$enum>::$variant()
            }
        }
    };
    ($enum:ty => $variant:ident ( $($value:expr),* $(,)? )) => {
        impl $crate::boundary::PanicFallback for $enum {
            fn panic_value() -> Self {
                <$enum>::$variant(($($value,)*))
            }
        }
    };
}

/// Reads the variants of a declaration a second time, for the two things that
/// have to see the same `#[cfg]`s the declarations do: the check that no two
/// variants share a discriminant, and the number that names the type.
///
/// It runs beside [`__stable_enum_build!`] rather than inside it because the
/// build walks the variants to emit their fields and arms, and the state it
/// carries is about where each one goes. A second pass keeps that state out of
/// the way: it only needs the discriminant of each variant and the shape its
/// payload carries.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_identity {
    (
        [$name:ident]
        [$($variants:tt)*]
    ) => {
        $crate::__stable_enum_identity_build! {
            [$name]
            []
            []
            []
            [$($variants)*]
        }
    };
}

/// The recursion behind [`__stable_enum_identity!`].
///
/// `$tags` collects the discriminants this build compiles, `$variants` collects
/// one number per variant, and `$cfg` holds the attributes that gate the variant
/// being read.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_identity_build {
    // An attribute that gates a declaration belongs to the variant: a variant
    // this build drops contributes neither a discriminant nor a number.
    (
        [$name:ident]
        [$($tags:tt)*]
        [$($variants:tt)*]
        [$($cfg:tt)*]
        [ #[cfg($($inner:tt)*)] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_identity_build! {
            [$name]
            [$($tags)*]
            [$($variants)*]
            [$($cfg)* #[cfg($($inner)*)]]
            [$($rest)*]
        }
    };

    // Any other attribute names nothing about the layout, which includes a
    // `cfg_attr`: what it expands to is not known here, and it is read by the
    // pass that writes the code rather than by this one. A variant that has to
    // disappear under a condition is gated with `#[cfg]`.
    (
        [$name:ident]
        [$($tags:tt)*]
        [$($variants:tt)*]
        [$($cfg:tt)*]
        [ #[$($other:tt)*] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_identity_build! {
            [$name]
            [$($tags)*]
            [$($variants)*]
            [$($cfg)*]
            [$($rest)*]
        }
    };

    // A variant that carries no payload.
    (
        [$name:ident]
        [$($tags:tt)*]
        [$($variants:tt)*]
        [$($cfg:tt)*]
        [ $variant:ident = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_identity_build! {
            [$name]
            [$($tags)* $($cfg)* $tag,]
            [$($variants)* $($cfg)*
                $crate::identity::of_variant(
                    $tag,
                    <() as $crate::identity::PayloadIdentity>::IDENTIFIER,
                ),]
            []
            [$($rest)*]
        }
    };

    // A variant that carries a payload.
    (
        [$name:ident]
        [$($tags:tt)*]
        [$($variants:tt)*]
        [$($cfg:tt)*]
        [ $variant:ident ( $($field:ty),* $(,)? ) = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_identity_build! {
            [$name]
            [$($tags)* $($cfg)* $tag,]
            [$($variants)* $($cfg)*
                $crate::identity::of_variant(
                    $tag,
                    <$crate::enumeration::Stored<( $($field,)* )> as $crate::identity::PayloadIdentity>::IDENTIFIER,
                ),]
            []
            [$($rest)*]
        }
    };

    // Nothing left: the two numbers are written beside the declarations.
    (
        [$name:ident]
        [$($tags:tt)*]
        [$($variants:tt)*]
        []
        []
    ) => {
        // Two variants that name the same discriminant would answer to the same
        // tag: the first arm that matches would read and release the payload of
        // the other one. The set is fixed by this declaration, so the mistake is
        // refused where it is written.
        const _: () = assert!(
            $crate::enumeration::tags_are_distinct(&[$($tags)*]),
            "a declaration names one discriminant twice",
        );

        // SAFETY: the number folds the name the type was declared under, the
        // discriminant of every variant this build compiled, and the shape each
        // payload carries. Two declarations that disagree about any of those
        // describe values the other cannot read, so they answer with different
        // numbers.
        unsafe impl $crate::identity::AbiIdentity for $name {
            const IDENTIFIER: u64 = $crate::identity::fold(
                91,
                &[
                    $crate::identity::named(::core::stringify!($name)),
                    $crate::identity::of_variants(93, &[$($variants)*]),
                ],
            );
        }
    };
}

/// Reads the optional clauses a declaration may write between the view and the
/// variants, then writes the declarations.
///
/// A declaration may name the traits the type is to have (`derives = [...]`) and
/// the variant a `Default` value uses (`default = Variant;`). Both are read here
/// so that the passes that walk the variants see nothing but variants.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_split {
    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$($union_meta:meta)*]
        [$union:ident]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($wipe_flag:tt)*]
        [$($derives:tt)*]
        [$($default:tt)*]
        [ derives = [ $($trait_name:ident),* $(,)? ] ; $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_split! {
            [$($enum_meta)*]
            [$vis $name]
            [$($union_meta)*]
            [$union]
            [$($view_meta)*]
            [$view]
            [$($wipe_flag)*]
            [$($derives)* $($trait_name)*]
            [$($default)*]
            [$($rest)*]
        }
    };

    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$($union_meta:meta)*]
        [$union:ident]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($wipe_flag:tt)*]
        [$($derives:tt)*]
        [$($default:tt)*]
        [ default = $variant:ident ; $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_split! {
            [$($enum_meta)*]
            [$vis $name]
            [$($union_meta)*]
            [$union]
            [$($view_meta)*]
            [$view]
            [$($wipe_flag)*]
            [$($derives)* Default]
            [$variant]
            [$($rest)*]
        }
    };

    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$($union_meta:meta)*]
        [$union:ident]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($wipe_flag:tt)*]
        [$($derives:tt)*]
        [$($default:tt)*]
        [ $($variants:tt)* ]
    ) => {
        $crate::__stable_enum_build! {
            [$($enum_meta)*]
            [$vis $name]
            [$union]
            [$($union_meta)*]
            [$($view_meta)*]
            [$view]
            [] [] [] [] [] [] []
            [] [] [] []
            [$($wipe_flag)*]
            [$($variants)*]
        }

        $crate::__stable_enum_identity! {
            [$name]
            [$($variants)*]
        }

        $crate::__stable_enum_derives! {
            [$($derives)*]
            [$($default)*]
            [$vis $name]
            [$union]
            [$($variants)*]
        }

        $crate::__stable_enum_stable_hash! {
            [$($derives)*]
            [$vis $name]
            [$($variants)*]
        }
    };
}

/// Writes the [`StableHash`](crate::stable_hash::StableHash) implementation a
/// declaration asked for.
///
/// It reads the variants here rather than in the pass that writes the other
/// traits, because this one asks nothing of the payload but its own number: no
/// comparison, no copy and no union of arms.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_stable_hash {
    (
        [StableHash $($rest:tt)*]
        [$vis:vis $name:ident]
        [$($variants:tt)*]
    ) => {
        $crate::__stable_enum_stable_hash_build! {
            [$vis $name]
            []
            []
            [$($variants)*]
        }
    };

    (
        [$other:ident $($rest:tt)*]
        [$vis:vis $name:ident]
        [$($variants:tt)*]
    ) => {
        $crate::__stable_enum_stable_hash! {
            [$($rest)*]
            [$vis $name]
            [$($variants)*]
        }
    };

    (
        []
        [$vis:vis $name:ident]
        [$($variants:tt)*]
    ) => {};
}

/// The recursion behind [`__stable_enum_stable_hash!`].
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_stable_hash_build {
    // A `#[cfg]` gates the variant: a variant this build drops has no arm.
    (
        [$vis:vis $name:ident]
        [$($arms:tt)*]
        [$($cfg:tt)*]
        [ #[cfg($($inner:tt)*)] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_stable_hash_build! {
            [$vis $name]
            [$($arms)*]
            [$($cfg)* #[cfg($($inner)*)]]
            [$($rest)*]
        }
    };

    // Any other attribute names nothing about a value, and a `cfg_attr` may
    // expand to an attribute no arm accepts: gate a variant with `#[cfg]`.
    (
        [$vis:vis $name:ident]
        [$($arms:tt)*]
        [$($cfg:tt)*]
        [ #[$($other:tt)*] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_stable_hash_build! {
            [$vis $name]
            [$($arms)*]
            [$($cfg)*]
            [$($rest)*]
        }
    };

    // A variant that carries no payload: the discriminant is the whole of it.
    (
        [$vis:vis $name:ident]
        [$($arms:tt)*]
        [$($cfg:tt)*]
        [ $variant:ident = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_stable_hash_build! {
            [$vis $name]
            [$($arms)*
                $($cfg)*
                ($tag, _pointer) => $crate::stable_hash::fold_word(
                    $crate::stable_hash::FNV_OFFSET,
                    $tag as u64,
                ),]
            []
            [$($rest)*]
        }
    };

    // A variant that carries a payload: the discriminant, then the payload's own
    // number, so two values of one variant differ where a value of another
    // variant would answer at all only if the numbers collided.
    (
        [$vis:vis $name:ident]
        [$($arms:tt)*]
        [$($cfg:tt)*]
        [ $variant:ident ( $($field:ty),* $(,)? ) = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_stable_hash_build! {
            [$vis $name]
            [$($arms)*
                $($cfg)*
                ($tag, pointer) => {
                    // SAFETY: the tag names this field, which is initialised,
                    // and the field holds the payload this declaration wrote.
                    let payload = unsafe {
                        &*(&(*pointer).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    $crate::stable_hash::fold_word(
                        $crate::stable_hash::fold_word(
                            $crate::stable_hash::FNV_OFFSET,
                            $tag as u64,
                        ),
                        $crate::stable_hash::StableHash::stable_hash(payload),
                    )
                },]
            []
            [$($rest)*]
        }
    };

    // Nothing left: the implementation is written.
    (
        [$vis:vis $name:ident]
        [$($arms:tt)*]
        []
        []
    ) => {
        impl $crate::stable_hash::StableHash for $name {
            fn stable_hash(&self) -> u64 {
                match (self.tag, self as *const Self) {
                    $($arms)*
                    // A tag this declaration did not write has no payload to
                    // fold, so the discriminant is all it can be numbered by.
                    (tag, _pointer) => $crate::stable_hash::fold_word(
                        $crate::stable_hash::FNV_OFFSET,
                        tag as u64,
                    ),
                }
            }
        }
    };
}

/// Writes the traits a declaration asked for, reading its variants a second
/// time.
///
/// The declared traits are written here rather than by a `derive` because a value
/// of this type is a tag and a union, and a `derive` would read the union rather
/// than the payload the tag selects.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_derives {
    (
        [$($derives:tt)*]
        [$($default:tt)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($variants:tt)*]
    ) => {
        // Equality without an order would be a promise about payload types this
        // macro cannot read, so the pair is checked before anything is written.
        $crate::__stable_enum_order_for_eq! { [$($derives)*] [] }
        $crate::__stable_enum_derive_build! {
            [$($derives)*]
            [$($default)*]
            [$vis $name]
            [$union]
            [] [] [] [] []
            []
            [$($variants)*]
        }
    };
}

/// Reports whether a declaration that names `Eq` also names `Ord`.
///
/// `Eq` says equality is a total relation, and a value of the type is a tag plus
/// a payload; whether that holds is a property of the payload types. The one
/// thing this macro can read about them is the ordering it also derives: the
/// generated `Ord` compares `Stored<(payload…)>`, so asking for `Ord` is a
/// promise the compiler checks that every payload is ordered, and an ordered
/// value is equal to itself. A declaration that names `Eq` without `Ord` is
/// refused, because nothing else here can rule out a payload - a float, say -
/// whose equality is not total.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_order_for_eq {
    ( [ Eq $($rest:tt)* ] [$($seen:tt)*] ) => {
        $crate::__stable_enum_order_for_eq! { [$($rest)*] [$($seen)* eq] }
    };
    ( [ Ord $($rest:tt)* ] [$($seen:tt)*] ) => {
        $crate::__stable_enum_order_for_eq! { [$($rest)*] [$($seen)* ord] }
    };
    ( [ $other:ident $($rest:tt)* ] [$($seen:tt)*] ) => {
        $crate::__stable_enum_order_for_eq! { [$($rest)*] [$($seen)*] }
    };
    ( [] [eq ord] ) => {};
    ( [] [ord eq] ) => {};
    ( [] [eq] ) => {
        ::core::compile_error!(
            "a declaration that derives `Eq` must derive `Ord` as well: the \
             payload types are what make equality total, and the ordering is the \
             only property of them this macro can have the compiler check. Write \
             `impl Eq` yourself for a declaration whose payloads are equal to \
             themselves without being ordered."
        );
    };
    ( [] [$($seen:tt)*] ) => {};
}

/// The recursion behind [`__stable_enum_derives!`].
///
/// It collects one arm per variant for every trait the type may have: five lists
/// that grow together, so the variants are read once however many traits are
/// asked for. What is written at the end is decided by the list of names.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_derive_build {
    // A `#[cfg]` gates the variant: a variant this build drops has no arm.
    (
        [$($derives:tt)*]
        [$($default:tt)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
        [$($cfg:tt)*]
        [ #[cfg($($inner:tt)*)] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_derive_build! {
            [$($derives)*]
            [$($default)*]
            [$vis $name]
            [$union]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
            [$($cfg)* #[cfg($($inner)*)]]
            [$($rest)*]
        }
    };

    // Any other attribute names nothing about a value, and a `cfg_attr` may
    // expand to an attribute no arm accepts: gate a variant with `#[cfg]`.
    (
        [$($derives:tt)*]
        [$($default:tt)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
        [$($cfg:tt)*]
        [ #[$($other:tt)*] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_derive_build! {
            [$($derives)*]
            [$($default)*]
            [$vis $name]
            [$union]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
            [$($cfg)*]
            [$($rest)*]
        }
    };

    // A variant that carries no payload: two values of it are the same value, and
    // nothing is left to compare or to fold.
    (
        [$($derives:tt)*]
        [$($default:tt)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
        [$($cfg:tt)*]
        [ $variant:ident = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_derive_build! {
            [$($derives)*]
            [$($default)*]
            [$vis $name]
            [$union]
            [$($clone)*
                $($cfg)*
                ($tag, pointer) => {
                    let _ = pointer;
                    // The union is zeroed before the active field is written, so
                    // the bytes no variant occupies are zeros rather than what
                    // the sender's stack held: the value crosses whole.
                    let mut value = ::core::mem::MaybeUninit::<$union>::zeroed();
                    // SAFETY: the field is written into a zeroed value, and the
                    // tag names the same field.
                    let value = unsafe {
                        ::core::ptr::addr_of_mut!((*value.as_mut_ptr()).$variant)
                            .write(::core::mem::ManuallyDrop::new(()));
                        value.assume_init()
                    };
                    ::core::result::Result::Ok(Self { tag: $tag, value })
                },]
            [$($eq)*
                $($cfg)*
                ($tag, _left, _right) => true,]
            [$($partial)*
                $($cfg)*
                ($tag, _left, _right) =>
                    ::core::option::Option::Some(::core::cmp::Ordering::Equal),]
            [$($ord)*
                $($cfg)*
                ($tag, _left, _right) => ::core::cmp::Ordering::Equal,]
            [$($hash)*
                $($cfg)*
                ($tag, _pointer, _state) => {},]
            []
            [$($rest)*]
        }
    };

    // A variant that carries a payload written as a tuple of types.
    (
        [$($derives:tt)*]
        [$($default:tt)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
        [$($cfg:tt)*]
        [ $variant:ident ( $($field:ty),* $(,)? ) = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_derive_build! {
            [$($derives)*]
            [$($default)*]
            [$vis $name]
            [$union]
            [$($clone)*
                $($cfg)*
                ($tag, pointer) => {
                    // SAFETY: the tag names this field, which is initialised,
                    // and the field holds the payload this declaration wrote.
                    let payload = unsafe {
                        &*(&(*pointer).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    let copy = $crate::clone::TryClone::try_clone(payload)?;
                    // The union is zeroed before the active field is written,
                    // as in the constructor, so a copy carries nothing of the
                    // bytes no variant occupies.
                    let mut value = ::core::mem::MaybeUninit::<$union>::zeroed();
                    // SAFETY: the field is written into a zeroed value.
                    let value = unsafe {
                        ::core::ptr::addr_of_mut!((*value.as_mut_ptr()).$variant)
                            .write(::core::mem::ManuallyDrop::new(copy));
                        value.assume_init()
                    };
                    ::core::result::Result::Ok(Self { tag: $tag, value })
                },]
            [$($eq)*
                $($cfg)*
                ($tag, left, right) => {
                    // SAFETY: the tag names this field in both values, and both
                    // hold the payload this declaration wrote.
                    let left = unsafe {
                        &*(&(*left).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    // SAFETY: the same field of the other value.
                    let right = unsafe {
                        &*(&(*right).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    *left == *right
                },]
            [$($partial)*
                $($cfg)*
                ($tag, left, right) => {
                    // SAFETY: the tag names this field in both values.
                    let left = unsafe {
                        &*(&(*left).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    // SAFETY: the same field of the other value.
                    let right = unsafe {
                        &*(&(*right).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    ::core::cmp::PartialOrd::partial_cmp(left, right)
                },]
            [$($ord)*
                $($cfg)*
                ($tag, left, right) => {
                    // SAFETY: the tag names this field in both values.
                    let left = unsafe {
                        &*(&(*left).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    // SAFETY: the same field of the other value.
                    let right = unsafe {
                        &*(&(*right).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    ::core::cmp::Ord::cmp(left, right)
                },]
            [$($hash)*
                $($cfg)*
                ($tag, pointer, state) => {
                    // SAFETY: the tag names this field, which is initialised.
                    let payload = unsafe {
                        &*(&(*pointer).value.$variant
                            as *const ::core::mem::ManuallyDrop<
                                $crate::enumeration::Stored<($($field,)*)>,
                            >
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    ::core::hash::Hash::hash(payload, state);
                },]
            []
            [$($rest)*]
        }
    };

    // Nothing left: the arms are written, so the traits the declaration named are
    // written beside the type.
    (
        [$($derives:tt)*]
        [$($default:tt)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
        []
        []
    ) => {
        $crate::__stable_enum_derived! {
            [$($derives)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };
}

/// Writes one implementation per trait the declaration named.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_derived {
    // A copy is made through the fallible form, so a payload that cannot be
    // copied is reported rather than lost silently.
    (
        [Clone $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        impl $crate::clone::TryClone for $name {
            fn try_clone(&self) -> $crate::error::AbiResult<Self> {
                match (self.tag, self as *const Self) {
                    $($clone)*
                    _ => ::core::result::Result::Err(
                        $crate::error::AbiError::new(
                            $crate::error::AbiStatus::InvalidTag,
                        ),
                    ),
                }
            }
        }

        impl ::core::clone::Clone for $name {
            /// Copies the value, panicking when a payload cannot be copied.
            ///
            /// # Panics
            ///
            /// Panics when a payload type reports that its copy could not be
            /// made; use [`TryClone::try_clone`](crate::clone::TryClone) for the
            /// fallible form.
            fn clone(&self) -> Self {
                match $crate::clone::TryClone::try_clone(self) {
                    ::core::result::Result::Ok(value) => value,
                    ::core::result::Result::Err(_) => {
                        panic!("copying this value could not allocate")
                    }
                }
            }
        }

        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    // Two values of different variants are never equal, whatever their payloads
    // are.
    (
        [PartialEq $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        impl ::core::cmp::PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                if self.tag != other.tag {
                    return false;
                }
                match (self.tag, self as *const Self, other as *const Self) {
                    $($eq)*
                    // A tag the declaration does not name is not a value this
                    // type describes, and `validate` refuses it. The two
                    // comparisons still have to agree with each other: `cmp`
                    // answers `Equal` for one tag, so this answers `true`.
                    _ => true,
                }
            }
        }

        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    // Equality as a total relation is a statement about the payload types. It is
    // written only because the declaration also names `Ord`, which the compiler
    // checks against every payload type; see `__stable_enum_order_for_eq`.
    (
        [Eq $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        impl ::core::cmp::Eq for $name {}

        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    (
        [PartialOrd $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        impl ::core::cmp::PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> ::core::option::Option<::core::cmp::Ordering> {
                match ::core::cmp::Ord::cmp(&self.tag, &other.tag) {
                    ::core::cmp::Ordering::Equal => {
                        match (self.tag, self as *const Self, other as *const Self) {
                            $($partial)*
                            _ => ::core::option::Option::Some(::core::cmp::Ordering::Equal),
                        }
                    }
                    order => ::core::option::Option::Some(order),
                }
            }
        }

        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    // The discriminants are part of the ABI, so the order of the values is the
    // order of the numbers the declaration wrote, not the order it wrote them in.
    (
        [Ord $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        impl ::core::cmp::Ord for $name {
            fn cmp(&self, other: &Self) -> ::core::cmp::Ordering {
                match ::core::cmp::Ord::cmp(&self.tag, &other.tag) {
                    ::core::cmp::Ordering::Equal => {
                        match (self.tag, self as *const Self, other as *const Self) {
                            $($ord)*
                            _ => ::core::cmp::Ordering::Equal,
                        }
                    }
                    order => order,
                }
            }
        }

        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    (
        [Hash $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        impl ::core::hash::Hash for $name {
            fn hash<__H: ::core::hash::Hasher>(&self, state: &mut __H) {
                ::core::hash::Hasher::write_u32(state, self.tag);
                match (self.tag, self as *const Self, state) {
                    $($hash)*
                    _ => {}
                }
            }
        }

        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    // A value with no variant named has no payload to build, so the declaration
    // says which variant stands for it.
    (
        [Default $($rest:tt)*]
        []
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        ::core::compile_error!(
            "`Default` needs a variant: write `default = Variant;` before the variants"
        );
    };

    (
        [Default $($rest:tt)*]
        [$variant:ident]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        impl ::core::default::Default for $name {
            fn default() -> Self {
                Self::$variant()
            }
        }

        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$variant]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    // The number this crate hashes with is written by a pass of its own, which
    // reads the variants for nothing but that number.
    (
        [StableHash $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        $crate::__stable_enum_derived! {
            [$($rest)*]
            [$($default)*]
            [$name]
            [$($clone)*]
            [$($eq)*]
            [$($partial)*]
            [$($ord)*]
            [$($hash)*]
        }
    };

    // A name that is not one of these is a mistake worth reporting where it is
    // written.
    (
        [$other:ident $($rest:tt)*]
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {
        ::core::compile_error!(::core::concat!(
            "`",
            ::core::stringify!($other),
            "` is not one of the traits a declaration derives: Clone, PartialEq, ",
            "Eq, PartialOrd, Ord, Hash, StableHash or Default",
        ));
    };

    (
        []
        [$($default:tt)*]
        [$name:ident]
        [$($clone:tt)*]
        [$($eq:tt)*]
        [$($partial:tt)*]
        [$($ord:tt)*]
        [$($hash:tt)*]
    ) => {};
}

/// The recursion behind [`stable_enum!`].
///
/// Each arm leaves a pattern that binds the receiver as `pointer`, and the code
/// that uses it is written in the same arm. A binding created here would not be
/// visible to code written in another arm, because the expansions of two rules
/// keep their local names apart.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_build {
    // An attribute is read before the variant it belongs to, and it is carried
    // in two lists: the whole list goes on the constructor and the view
    // variant, and the attributes that gate a declaration go on the places
    // that hold the variant itself - the union field and the arms that read
    // its payload - because a function-only attribute such as `#[inline]` is
    // not legal there.
    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($union_meta:meta)*]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($fields:tt)*]
        [$($ctors:tt)*]
        [$($view_fields:tt)*]
        [$($view_arms:tt)*]
        [$($drops:tt)*]
        [$($wipes:tt)*]
        [$($checks:tt)*]
        [$($tags:tt)*]
        [$($cfg_meta:tt)*]
        [$($full_meta:tt)*]
        [$($wipe_bounds:tt)*]
        [$($wipe_flag:tt)*]
        [ #[cfg($($inner:tt)*)] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_build! {
            [$($enum_meta)*]
            [$vis $name]
            [$union]
            [$($union_meta)*]
            [$($view_meta)*]
            [$view]
            [$($fields)*]
            [$($ctors)*]
            [$($view_fields)*]
            [$($view_arms)*]
            [$($drops)*]
            [$($wipes)*]
            [$($checks)*]
            [$($tags)*]
            [$($cfg_meta)* #[cfg($($inner)*)]]
            [$($full_meta)* #[cfg($($inner)*)]]
            [$($wipe_bounds)*]
            [$($wipe_flag)*]
            [$($rest)*]
        }
    };

    // The same, for the form whose predicate is evaluated at the use site.
    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($union_meta:meta)*]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($fields:tt)*]
        [$($ctors:tt)*]
        [$($view_fields:tt)*]
        [$($view_arms:tt)*]
        [$($drops:tt)*]
        [$($wipes:tt)*]
        [$($checks:tt)*]
        [$($tags:tt)*]
        [$($cfg_meta:tt)*]
        [$($full_meta:tt)*]
        [$($wipe_bounds:tt)*]
        [$($wipe_flag:tt)*]
        [ #[cfg_attr($($inner:tt)*)] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_build! {
            [$($enum_meta)*]
            [$vis $name]
            [$union]
            [$($union_meta)*]
            [$($view_meta)*]
            [$view]
            [$($fields)*]
            [$($ctors)*]
            [$($view_fields)*]
            [$($view_arms)*]
            [$($drops)*]
            [$($wipes)*]
            [$($checks)*]
            [$($tags)*]
            [$($cfg_meta)*]
            [$($full_meta)* #[cfg_attr($($inner)*)]]
            [$($wipe_bounds)*]
            [$($wipe_flag)*]
            [$($rest)*]
        }
    };

    // Any other attribute belongs to the declaration of the variant, not to the
    // places that hold it.
    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($union_meta:meta)*]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($fields:tt)*]
        [$($ctors:tt)*]
        [$($view_fields:tt)*]
        [$($view_arms:tt)*]
        [$($drops:tt)*]
        [$($wipes:tt)*]
        [$($checks:tt)*]
        [$($tags:tt)*]
        [$($cfg_meta:tt)*]
        [$($full_meta:tt)*]
        [$($wipe_bounds:tt)*]
        [$($wipe_flag:tt)*]
        [ #[$($other:tt)*] $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_build! {
            [$($enum_meta)*]
            [$vis $name]
            [$union]
            [$($union_meta)*]
            [$($view_meta)*]
            [$view]
            [$($fields)*]
            [$($ctors)*]
            [$($view_fields)*]
            [$($view_arms)*]
            [$($drops)*]
            [$($wipes)*]
            [$($checks)*]
            [$($tags)*]
            [$($cfg_meta)*]
            [$($full_meta)* #[$($other)*]]
            [$($wipe_bounds)*]
            [$($wipe_flag)*]
            [$($rest)*]
        }
    };

    // A variant that carries no payload.
    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($union_meta:meta)*]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($fields:tt)*]
        [$($ctors:tt)*]
        [$($view_fields:tt)*]
        [$($view_arms:tt)*]
        [$($drops:tt)*]
        [$($wipes:tt)*]
        [$($checks:tt)*]
        [$($tags:tt)*]
        [$($cfg_meta:tt)*]
        [$($full_meta:tt)*]
        [$($wipe_bounds:tt)*]
        [$($wipe_flag:tt)*]
        [ $variant:ident = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_build! {
            [$($enum_meta)*]
            [$vis $name]
            [$union]
            [$($union_meta)*]
            [$($view_meta)*]
            [$view]
            [$($fields)*
                $($cfg_meta)*
                #[allow(non_camel_case_types)]
                #[doc = "The payload of this variant."]
                $variant: core::mem::ManuallyDrop<()>,]
            [$($ctors)*
                #[allow(non_snake_case)]
                $($full_meta)*
                #[doc = "Builds this variant."]
                pub fn $variant() -> Self {
                    // The union is zeroed before the active field is written, so
                    // the bytes no variant occupies are zeros rather than what
                    // the sender's stack held: the value crosses whole.
                    let mut value = core::mem::MaybeUninit::<$union>::zeroed();
                    // SAFETY: the field is written into a zeroed value, and the
                    // tag names the same field.
                    let value = unsafe {
                        core::ptr::addr_of_mut!((*value.as_mut_ptr()).$variant)
                            .write(core::mem::ManuallyDrop::new(()));
                        value.assume_init()
                    };
                    Self { tag: $tag, value }
                }]
            [$($view_fields)*
                #[allow(non_snake_case)]
                $($cfg_meta)*
                #[doc = "This variant, and its payload."]
                $variant(&'a ()),]
            [$($view_arms)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    #[allow(non_snake_case)]
                    // SAFETY: the tag names this field, which is initialised, and
                    // a unit payload is a valid `()`.
                    let payload = unsafe {
                        &*(&(*pointer).value.$variant as *const core::mem::ManuallyDrop<()>
                            as *const ())
                    };
                    $view::$variant(payload)
                },]
            [$($drops)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    // SAFETY: the tag names this field, which is initialised.
                    unsafe { core::mem::ManuallyDrop::drop(&mut (*pointer).value.$variant) };
                },]
            [$($wipes)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    // A unit payload carries no content, so nothing is written
                    // over. The field is still released when the value is.
                    let _ = pointer;
                },]
            [$($checks)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    // SAFETY: the tag names this field, which is initialised.
                    let payload = unsafe {
                        &*(&(*pointer).value.$variant as *const core::mem::ManuallyDrop<()>
                            as *const ())
                    };
                    <() as $crate::enumeration::ValidateElements>::validate_elements(payload)?;
                },]
            [$($tags)* $tag,]
            []
            []
            [$($wipe_bounds)* (): $crate::enumeration::WipeElements,]
            [$($wipe_flag)*]
            [$($rest)*]
        }
    };

    // A variant that carries a payload, written as a tuple of types.
    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($union_meta:meta)*]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($fields:tt)*]
        [$($ctors:tt)*]
        [$($view_fields:tt)*]
        [$($view_arms:tt)*]
        [$($drops:tt)*]
        [$($wipes:tt)*]
        [$($checks:tt)*]
        [$($tags:tt)*]
        [$($cfg_meta:tt)*]
        [$($full_meta:tt)*]
        [$($wipe_bounds:tt)*]
        [$($wipe_flag:tt)*]
        [ $variant:ident ( $($field:ty),* $(,)? ) = $tag:literal, $($rest:tt)* ]
    ) => {
        $crate::__stable_enum_build! {
            [$($enum_meta)*]
            [$vis $name]
            [$union]
            [$($union_meta)*]
            [$($view_meta)*]
            [$view]
            [$($fields)*
                $($cfg_meta)*
                #[allow(non_camel_case_types)]
                #[doc = "The payload of this variant."]
                $variant: core::mem::ManuallyDrop<$crate::enumeration::Stored<($($field,)*)>>,]
            [$($ctors)*
                #[allow(non_snake_case)]
                $($full_meta)*
                #[doc = "Builds this variant."]
                pub fn $variant(payload: ($($field,)*)) -> Self {
                    // The union is zeroed before the active field is written, so
                    // the bytes no variant occupies are zeros rather than what
                    // the sender's stack held: the value crosses whole.
                    let mut value = core::mem::MaybeUninit::<$union>::zeroed();
                    // SAFETY: the field is written into a zeroed value, and the
                    // tag names the same field.
                    let value = unsafe {
                        core::ptr::addr_of_mut!((*value.as_mut_ptr()).$variant)
                            .write(core::mem::ManuallyDrop::new(
                                <($($field,)*) as $crate::enumeration::PayloadShape>::store(payload),
                            ));
                        value.assume_init()
                    };
                    Self { tag: $tag, value }
                }]
            [$($view_fields)*
                #[allow(non_snake_case)]
                $($cfg_meta)*
                #[doc = "This variant, and its payload."]
                $variant(&'a $crate::enumeration::Stored<($($field,)*)>),]
            [$($view_arms)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    #[allow(non_snake_case)]
                    // SAFETY: the tag names this field, which is initialised, and
                    // the field holds the payload this declaration wrote.
                    let payload = unsafe {
                        &*(&(*pointer).value.$variant
                            as *const core::mem::ManuallyDrop<$crate::enumeration::Stored<($($field,)*)>>
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    $view::$variant(payload)
                },]
            [$($drops)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    // SAFETY: the tag names this field, which is initialised.
                    unsafe { core::mem::ManuallyDrop::drop(&mut (*pointer).value.$variant) };
                },]
            [$($wipes)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    // SAFETY: the tag names this field, which is initialised.
                    unsafe {
                        let payload = &mut *(&mut (*pointer).value.$variant
                            as *mut core::mem::ManuallyDrop<$crate::enumeration::Stored<($($field,)*)>>
                            as *mut $crate::enumeration::Stored<($($field,)*)>);
                        $crate::enumeration::WipeElements::wipe_elements(payload);
                    }
                },]
            [$($checks)*
                $($cfg_meta)*
                ($tag, pointer) => {
                    // SAFETY: the tag names this field, which is initialised.
                    let payload = unsafe {
                        &*(&(*pointer).value.$variant
                            as *const core::mem::ManuallyDrop<$crate::enumeration::Stored<($($field,)*)>>
                            as *const $crate::enumeration::Stored<($($field,)*)>)
                    };
                    $crate::enumeration::ValidateElements::validate_elements(payload)?;
                },]
            [$($tags)* $tag,]
            []
            []
            [$($wipe_bounds)* $crate::enumeration::Stored<($($field,)*)>: $crate::enumeration::WipeElements,]
            [$($wipe_flag)*]
            [$($rest)*]
        }
    };

    // Nothing left: emit the declarations, and let the flag decide whether a
    // wipe is written beside them.
    (
        [$($enum_meta:meta)*]
        [$vis:vis $name:ident]
        [$union:ident]
        [$($union_meta:meta)*]
        [$($view_meta:meta)*]
        [$view:ident]
        [$($fields:tt)*]
        [$($ctors:tt)*]
        [$($view_fields:tt)*]
        [$($view_arms:tt)*]
        [$($drops:tt)*]
        [$($wipes:tt)*]
        [$($checks:tt)*]
        [$($tags:tt)*]
        []
        []
        [$($wipe_bounds:tt)*]
        [$($wipe_flag:tt)*]
        []
    ) => {
        $(#[$enum_meta])*
        #[repr(C)]
        $vis struct $name {
            tag: u32,
            value: $union,
        }

        $(#[$union_meta])*
        #[repr(C)]
        #[allow(non_snake_case)]
        $vis union $union {
            $($fields)*
        }

        $(#[$view_meta])*
        #[allow(non_snake_case)]
        $vis enum $view<'a> {
            $($view_fields)*
        }

        impl $name {
            /// The discriminant of this value.
            pub const fn tag(&self) -> u32 {
                self.tag
            }

            /// The active variant and its payload.
            pub fn view(&self) -> $view<'_> {
                match (self.tag, self as *const Self) {
                    $($view_arms)*
                    _ => {
                        // A value whose tag this declaration did not write has no
                        // variant to report. The constructors refuse to build
                        // one, and `validate` refuses one that arrives.
                        panic!("a tag this declaration did not write")
                    }
                }
            }

            $($ctors)*
        }

        // The discriminant set and the number that names this type are written
        // by the identity pass, which reads the variants again: both have to see
        // the same `#[cfg]`s the arms above do, and the `$tags` this build
        // collected does not carry them.
        impl core::fmt::Debug for $name {
            // The payload can be content a log must not repeat, so the debug form
            // reports the tag.
            fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter
                    .debug_struct(::core::stringify!($name))
                    .field("tag", &self.tag)
                    .finish_non_exhaustive()
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                match (self.tag, self as *mut Self) {
                    $($drops)*
                    _ => {}
                }
            }
        }

        // SAFETY: the layout is declared above, and dropping the value releases
        // exactly the payload of the active variant.
        unsafe impl $crate::vec::StableElement for $name {
            unsafe fn drop_in_place(pointer: *mut Self) {
                // SAFETY: the caller guarantees an initialised value.
                unsafe { core::ptr::drop_in_place(pointer) }
            }
        }

        // SAFETY: a tag this declaration did not write is refused, and the
        // payload of the active variant is validated element by element.
        unsafe impl $crate::boundary::AbiType for $name {
            fn validate(&self) -> $crate::error::AbiResult<()> {
                match (self.tag, self as *const Self) {
                    $($checks)*
                    _ => {
                        return Err($crate::error::AbiError::new(
                            $crate::error::AbiStatus::InvalidTag,
                        ))
                    }
                }
                Ok(())
            }
        }

        $crate::__stable_enum_wipe! {
            [$($wipe_flag)*]
            [$name]
            [$($wipes)*]
        }
    };
}

/// Writes the implementation of [`Wipe`](crate::zeroize::Wipe) for an
/// enumeration, when the declaration asked for one.
///
/// The flag is the declaration's: [`stable_enum!`] turns it on and
/// [`stable_enum_unwiped!`] leaves it off, so a payload that cannot be wiped
/// stops only the declaration that wants the implementation.
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_enum_wipe {
    ([wipe] [$name:ident] [$($wipes:tt)*]) => {
        impl $crate::zeroize::Wipe for $name {
            /// Wipes the payload of the active variant.
            ///
            /// The payload is wiped element by element, so a container inside it
            /// keeps its own header and the value stays one that can be released
            /// afterwards. A payload type that has no wipe of its own stops the
            /// declaration here, because an enumeration written with one cannot
            /// offer a wipe that covers it; [`stable_enum_unwiped!`](crate::stable_enum_unwiped)
            /// declares the same enumeration without the implementation.
            fn wipe_in_place(&mut self) {
                match (self.tag, self as *mut Self) {
                    $($wipes)*
                    _ => {}
                }
            }
        }
    };
    ([$($none:tt)*] [$name:ident] [$($wipes:tt)*]) => {};
}
