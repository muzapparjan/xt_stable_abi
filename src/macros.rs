//! Macros that turn ordinary Rust traits into boundary types.
//!
//! [`stable_object!`] takes three items, the handle type, the table type and
//! the trait, and generates a typed wrapper whose methods call through the
//! table. The generated accessors are `unsafe` because their soundness rests on
//! both sides agreeing on the trait and on the caller not aliasing a mutable
//! receiver.
//!
//! Supported method shapes are `fn name(&self)`, `fn name(&mut self)`,
//! `fn name(self)`, and any of them followed by a comma-separated argument list
//! that may end with a comma, with or without a return type. Arguments and return values
//! must themselves be boundary types: fixed-width integers, floats,
//! [`crate::string::StableStr`], [`crate::string::StableString`],
//! [`crate::vec::StableSlice`], [`crate::vec::StableVec`],
//! [`crate::option::StableOption`], [`crate::result::StableResult`] or another
//! generated handle.
//!
//! A return type that names a lifetime, and `-> Self`, are refused at the
//! declaration: a slot lends its value only for the call, so a returned borrow
//! would outlive the storage, and the generated trampoline has no input lifetime
//! for the borrow to elide to; `Self` names the implementation type, which the
//! caller does not know. `'static` is refused too: a value that points into the
//! library dangles once the library is unloaded, which the type system is not
//! told about. Return an owned value instead (`StableString`, `StableVec`,
//! `StableBox`, ...).

/// Declares a trait object whose virtual table is part of the ABI.
///
/// The three items are the handle type, the table type and the trait:
///
/// ```
/// use xt_stable_abi::{stable_object, StableString};
///
/// stable_object! {
///     /// A handle to a greeting source.
///     pub struct GreeterObject;
///     /// The table behind [`GreeterObject`].
///     pub struct GreeterVTable;
///     /// Produces greetings.
///     pub trait Greeter {
///         /// Returns a greeting for `name`.
///         fn greet(&self, name: xt_stable_abi::StableStr) -> StableString;
///         /// Makes the greetings more formal.
///         fn formalize(&mut self);
///     }
/// }
///
/// struct Plain {
///     exclamation: bool,
/// }
///
/// impl Greeter for Plain {
///     fn greet(&self, name: xt_stable_abi::StableStr<'_>) -> StableString {
///         let name = name.as_str();
///         let suffix = if self.exclamation { "!" } else { "." };
///         StableString::from_str(&format!("Hello, {name}{suffix}")).unwrap()
///     }
///     fn formalize(&mut self) {
///         self.exclamation = false;
///     }
/// }
///
/// let mut object = GreeterObject::abi_from_value(Plain { exclamation: true });
/// let greeting = unsafe { object.greet(xt_stable_abi::StableStr::from_str("Ada")) };
/// assert_eq!(greeting.as_str(), "Hello, Ada!");
/// unsafe { object.formalize() };
/// let greeting = unsafe { object.greet(xt_stable_abi::StableStr::from_str("Ada")) };
/// assert_eq!(greeting.as_str(), "Hello, Ada.");
/// ```
///
/// # Supported method shapes
///
/// A method is `fn name(&self)`, `fn name(&mut self)` or `fn name(self)`,
/// optionally followed by comma-separated `argument: Type` pairs, which may end
/// with a comma, and optionally followed by `-> ReturnType`. A generic method or
/// an associated item does not
/// fit the grammar: the table is a fixed list of slots, one position per declared
/// method, and both sides derive that list from the same declaration.
///
/// A return type that names a lifetime is refused, because the slot lends its
/// value only for the call: the handle would point at storage that dies when the
/// call returns. `'static` is refused under the same rule, because the storage
/// of a dynamic library does not outlive the library either.
///
/// ```compile_fail
/// use xt_stable_abi::{stable_object, StableStr};
///
/// stable_object! {
///     /// A handle to a view.
///     pub struct ViewObject;
///     /// The table behind [`ViewObject`].
///     pub struct ViewVTable;
///     /// Hands out a view of what it holds.
///     pub trait View {
///         /// The slot cannot lend what it points at past the call.
///         fn text(&self) -> StableStr<'_>;
///     }
/// }
/// ```
///
/// `Self` is refused as well, because the caller of a slot does not know the
/// implementation type:
///
/// ```compile_fail
/// use xt_stable_abi::stable_object;
///
/// stable_object! {
///     /// A handle that would hand itself out.
///     pub struct AgainObject;
///     /// The table behind [`AgainObject`].
///     pub struct AgainVTable;
///     /// Names the implementation type in its return position.
///     pub trait Again {
///         /// `Self` here is the implementation, not the handle.
///         fn again(&self) -> Self;
///     }
/// }
/// ```
///
/// Parentheses name the same type, so they do not get around the check:
///
/// ```compile_fail
/// use xt_stable_abi::stable_object;
///
/// stable_object! {
///     /// A handle that would hand itself out.
///     pub struct ParenthesisedObject;
///     /// The table behind [`ParenthesisedObject`].
///     pub struct ParenthesisedVTable;
///     /// Names the implementation type inside parentheses.
///     pub trait Parenthesised {
///         /// The parentheses do not change what the type is.
///         fn again(&self) -> (Self);
///     }
/// }
/// ```
///
/// The handle also receives the constructors and accessors named
/// `abi_from_value`, `abi_try_from_value`, `abi_try_from_value_with`,
/// `abi_from_stable`, `abi_into_stable`, `abi_as_stable`, `abi_type_id` and
/// `abi_is_empty`, so those names are not available to the trait.
///
/// `abi_try_from_value_with` takes the identifier and the allocator to build the
/// table under, so it is the form a plugin uses to state which declaration it
/// publishes. One implementation type has one table, and the table records the
/// identifier it was built under: wrapping the same type under a second
/// identifier in one process panics rather than leaving a table behind per
/// identifier. Wrap each implementation type under one identifier, which is what
/// `abi_try_from_value` does on its own.
///
/// A declaration cannot name a supertrait: the table lists the methods of this
/// trait, and an inherited method has no slot in it.
///
/// ```compile_fail
/// use xt_stable_abi::stable_object;
///
/// stable_object! {
///     /// A handle whose trait inherits another one.
///     pub struct ChildObject;
///     /// The table behind [`ChildObject`].
///     pub struct ChildVTable;
///     /// Names a supertrait, whose methods have no slot here.
///     pub trait Child: Send {
///         /// The only method this table would list.
///         fn child(&self) -> u32;
///     }
/// }
/// ```
///
/// Arguments and return types must implement [`crate::boundary::AbiType`], which
/// rejects a declaration that names a type whose layout is not part of the
/// boundary, such as `String`, `usize` or `bool`.
///
/// An attribute written before `struct` or `trait` belongs to that declaration
/// and to nothing the macro generates beside it, so a `#[cfg]` there is the wrong
/// tool: the table, the impls and the accessors would still be compiled and would
/// name a type the attribute removed. Gate the whole declaration instead, by
/// putting the attribute on the invocation:
///
/// ```ignore
/// #[cfg(feature = "extra")]
/// stable_object! {
///     pub struct ExtraObject;
///     // ...
/// }
/// ```
///
/// A value the declaration returns is checked before the accessor hands it out,
/// so a table that produced a tag or byte the type does not define stops at the
/// accessor. The signature recorded in the table numbers the types a method
/// mentions by [`crate::identity::AbiIdentity`] rather than by the way they were
/// written: `StableStr`, `StableStr<'_>` and `xt_stable_abi::StableStr` are one
/// type and name one identifier.
#[macro_export]
macro_rules! stable_object {
    (
        $(#[$object_meta:meta])*
        $object_vis:vis struct $object:ident;
        $(#[$vtable_meta:meta])*
        $vtable_vis:vis struct $vtable:ident;
        $(#[$trait_meta:meta])*
        $trait_vis:vis trait $trait_name:ident {
            $($body:tt)*
        }
    ) => {
        $(#[$trait_meta])*
        $trait_vis trait $trait_name {
            $($body)*
        }

        $(#[$object_meta])*
        #[repr(transparent)]
        $object_vis struct $object($crate::object::StableDyn);

        impl ::core::fmt::Debug for $object {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter
                    .debug_tuple(::core::stringify!($object))
                    .field(&self.0)
                    .finish()
            }
        }

        // SAFETY: the handle is a `#[repr(transparent)]` wrapper around
        // `StableDyn`, whose contract it inherits: dropping it releases the
        // value through the table and the allocator that value records.
        unsafe impl $crate::vec::StableElement for $object {
            unsafe fn drop_in_place(ptr: *mut Self) {
                // SAFETY: the caller guarantees an initialised value.
                unsafe { $crate::__private::ptr::drop_in_place(ptr) }
            }
        }

        // SAFETY: the handle has the layout of `StableDyn`, so it travels by
        // value exactly as that type does; `validate` checks that the address
        // the table describes is one the value can be read through, and that
        // the table shares this declaration's methods.
        unsafe impl $crate::boundary::AbiType for $object {
            fn validate(&self) -> $crate::error::AbiResult<()> {
                $crate::object::validate_handle(&self.0, Self::ABI_SLOT_SIGNATURES)
            }
        }

        // A guarded call that panicked has no object to hand over: the caller
        // reads the panic from the result the accessor returns, and the value
        // this produces is never handed out.
        impl $crate::boundary::PanicFallback for $object {
            fn panic_value() -> Self {
                Self($crate::object::StableDyn::empty())
            }
        }

        // Wiping the handle is wiping the value it carries: the field does the
        // work, because only the value's own type knows how to overwrite it.
        impl $crate::zeroize::Wipe for $object {
            fn wipe_in_place(&mut self) {
                $crate::zeroize::Wipe::wipe_in_place(&mut self.0);
            }
        }

        // SAFETY: the handle names one declaration, and the number is derived
        // from the name the macro was given, which is the same in every build of
        // that declaration.
        unsafe impl $crate::identity::AbiIdentity for $object {
            const IDENTIFIER: u64 = $crate::identity::named(::core::stringify!($object));
        }

        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$(#[$vtable_meta])*]
            []
            []
            []
            []
            [$($body)*]
            []
        }
    };

    // A declaration with a supertrait names a shape one table cannot carry: a
    // table describes the methods of one trait, and an inherited method belongs
    // to another declaration whose slots this table does not list.
    (
        $(#[$object_meta:meta])*
        $object_vis:vis struct $object:ident;
        $(#[$vtable_meta:meta])*
        $vtable_vis:vis struct $vtable:ident;
        $(#[$trait_meta:meta])*
        $trait_vis:vis trait $trait_name:ident : $($rest:tt)*
    ) => {
        ::core::compile_error!(::core::concat!(
            "a table lists the methods of one trait, so a declaration cannot name a ",
            "supertrait: an inherited method has no slot here. Repeat the inherited ",
            "methods in this trait, or cross the boundary with an object whose own ",
            "declaration lists them.",
        ));
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __stable_object_build {
    // `self: &Self` is the shared receiver written out.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( self : & Self $(, $arg:ident : $ty:ty)* $(,)? ) $(-> $ret:ty)? ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( & self $(, $arg : $ty)* ) $(-> $ret)? ; $($rest)* ]
            [$($signature)*]
        }
    };

    // `self: &mut Self` is the mutable receiver written out.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( self : & mut Self $(, $arg:ident : $ty:ty)* $(,)? ) $(-> $ret:ty)? ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( & mut self $(, $arg : $ty)* ) $(-> $ret)? ; $($rest)* ]
            [$($signature)*]
        }
    };

    // Every method has been collected: emit the table and the handle methods.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        []
        [$($signature:tt)*]
    ) => {
        $($vtable_meta)*
        #[repr(C)]
        $vtable_vis struct $vtable {
            /// The header shared by every generated table.
            ///
            /// The field name is reserved: a method called `prefix` occupies a
            /// slot of its own and does not collide with it.
            pub __abi_prefix: $crate::object::VTablePrefix,
            $($fields)*
            /// The entry that ends the slot list.
            ///
            /// The field name is reserved. The header records how many entries
            /// precede it, so a receiver reads exactly those and the sentinel is
            /// the last field of the table.
            pub __abi_end: $crate::object::VTableSlot,
        }

        impl $object {
            /// The identifier of every method slot this build compiled, in
            /// declaration order.
            ///
            /// A method whose attributes (a `#[cfg]`, for instance) removed it
            /// from this build contributes no entry, so two builds of the same
            /// source with different features describe different tables.
            #[allow(unused_doc_comments)]
            pub const ABI_SLOT_SIGNATURES: &'static [u64] = &[ $($signature)* ];

            // A table records its entry count in one field, and a receiver reads
            // at most `MAX_SLOTS` entries. A declaration past that number would
            // build a table whose later methods no peer can see, so it is refused
            // where it is written rather than at the first call. The check sits
            // inside a function so that it is evaluated when this code is
            // compiled, whether or not anything calls the function.
            #[allow(dead_code)]
            fn __abi_slot_count_fits() {
                const _: () = assert!(
                    $object::ABI_SLOT_SIGNATURES.len() <= $crate::object::MAX_SLOTS as usize,
                    "a declaration has more methods than a table can record",
                );
            }

            /// An identifier derived from the slots this build compiled:
            /// every method name, receiver form, argument type and return type,
            /// together with the crate's layout revision.
            ///
            /// The fold is commutative, so the order the methods are listed in
            /// does not change the number: the order is not part of a
            /// declaration.
            ///
            /// A type is named by its [`crate::identity::AbiIdentity`], so how a
            /// type is written does not change the number: `StableStr`,
            /// `StableStr<'_>` and `xt_stable_abi::StableStr` name one type, and
            /// a local alias names the type it stands for.
            pub const ABI_SIGNATURE: u64 =
                $crate::object::combined_signature(Self::ABI_SLOT_SIGNATURES);

            $($methods)*

            /// Wraps `value` in an object, using the process allocator.
            ///
            /// # Panics
            ///
            /// Panics when the storage for the value cannot be allocated.
            pub fn abi_from_value<T: $trait_name + 'static>(value: T) -> Self {
                match Self::abi_try_from_value(value) {
                    Ok(object) => object,
                    Err(error) => panic!("failed to build an ABI object: {}", error),
                }
            }

            /// Wraps `value` in an object, using the process allocator.
            ///
            /// # Panics
            ///
            /// Panics when this implementation type was already registered under
            /// a different identifier. One type has one method list and one
            /// table, and a table records the identifier it was built under, so
            /// wrapping one type under a second identifier is a mistake in the
            /// declaration rather than a failure a caller can handle; it stops
            /// here instead of leaving a table behind per identifier.
            pub fn abi_try_from_value<T: $trait_name + 'static>(
                value: T,
            ) -> $crate::error::AbiResult<Self> {
                // The fold runs where this code is compiled, so the header holds
                // a constant rather than a number computed on every build. The
                // literal it is folded from is still part of the unoptimized
                // binary: this build removes it when it optimizes, and a debug
                // build keeps it.
                const __ABI_TYPE_ID: u64 =
                    $crate::object::type_id_of(::core::stringify!($trait_name));
                Self::abi_try_from_value_with(
                    value,
                    __ABI_TYPE_ID,
                    $crate::alloc::AbiAllocator::system(),
                )
            }

            /// Wraps `value` in an object with an explicit identifier and
            /// allocator.
            pub fn abi_try_from_value_with<T: $trait_name + 'static>(
                __abi_value: T,
                __abi_type_id: u64,
                __abi_allocator: $crate::alloc::AbiAllocator,
            ) -> $crate::error::AbiResult<Self> {
                $($trampolines)*

                unsafe extern "C" fn __abi_drop_value<U>(data: *mut u8) {
                    // SAFETY: the table is built for `U`, so `data` points at
                    // an initialised `U`.
                    unsafe { $crate::__private::ptr::drop_in_place(data.cast::<U>()) }
                }

                static VTABLE_CACHE: $crate::object::VTableCache =
                    $crate::object::VTableCache::new();

                let __abi_vtable = unsafe {
                    VTABLE_CACHE.get_or_insert(
                        ($crate::__private::TypeId::of::<T>(), __abi_type_id),
                        || {
                            let mut __abi_table = $vtable {
                                __abi_prefix: $crate::object::VTablePrefix {
                                    type_id: __abi_type_id,
                                    signature: Self::ABI_SIGNATURE,
                                    layout_revision: $crate::object::LAYOUT_REVISION,
                                    size: $crate::__private::size_of::<T>() as u64,
                                    align: $crate::__private::align_of::<T>() as u64,
                                    slots: 0,
                                    drop: Some(__abi_drop_value::<T>),
                                },
                                $($values)*
                                __abi_end: $crate::object::VTableSlot {
                                    signature: 0,
                                    call: $crate::__private::ptr::null(),
                                },
                            };
                            // The header says how many entries follow it, so a
                            // receiver never has to scan past the end of a table
                            // it did not build. The number is the length of the
                            // list of compiled signatures, so a method a `#[cfg]`
                            // removed is not counted.
                            __abi_table.__abi_prefix.slots =
                                Self::ABI_SLOT_SIGNATURES.len() as u64;
                            // The entries are ordered by the signature each one
                            // declares, so a call is a binary search rather than a
                            // walk. The order is not part of the protocol: a call
                            // finds its entry by comparing numbers.
                            // SAFETY: the header now counts the entries, which were
                            // just written, and the table is not shared yet.
                            unsafe {
                                $crate::object::sort_table_entries(
                                    ::core::ptr::addr_of_mut!(__abi_table).cast::<u8>(),
                                    Self::ABI_SLOT_SIGNATURES.len() as u64,
                                )
                            };
                            // Frees the table this closure built, with the type
                            // it was allocated as, so that the memory can be given
                            // back when the module that built it is unloaded.
                            unsafe fn __abi_release_table(
                                table: *const $crate::object::VTablePrefix,
                            ) {
                                // SAFETY: the pointer came from `Box::into_raw`
                                // of this table type.
                                ::core::mem::drop(unsafe {
                                    $crate::__private::Box::from_raw(table as *mut $vtable)
                                });
                            }
                            let __abi_storage = $crate::__private::Box::into_raw(
                                $crate::__private::Box::new(__abi_table),
                            );
                            // SAFETY: `__abi_storage` came from `Box::into_raw`
                            // of this table type, and `__abi_release_table` takes
                            // that same allocation back.
                            unsafe {
                                $crate::object::BuiltTable::new(
                                    __abi_storage.cast(),
                                    __abi_release_table,
                                )
                            }
                        },
                    )
                };

                // SAFETY: the storage is allocated for `T` and written once.
                let __abi_data =
                    unsafe { $crate::object::allocate_value(__abi_value, __abi_allocator)? };
                Ok(Self(unsafe {
                    $crate::object::StableDyn::from_raw_parts(
                        __abi_data,
                        __abi_vtable,
                        __abi_allocator,
                    )
                }))
            }

            /// Takes ownership of an existing object after checking that it was
            /// built from the same trait declaration.
            ///
            /// The check is equality of the method sets: the table must declare
            /// exactly the slots this declaration does. The order they are listed
            /// in is not part of it, because a call finds its slot by comparing
            /// identifiers rather than by counting entries. A table that declares
            /// other methods is rejected here, because calling through it would
            /// reach code with a different signature.
            ///
            /// The check is structural, not nominal: the number a table records
            /// as its type identifier is not compared, because two sides that
            /// declare the same methods call the same code whatever name each
            /// wrote its trait under. Two declarations that end up with the same
            /// method set are therefore interchangeable by design.
            pub fn abi_try_from_stable(
                value: $crate::object::StableDyn,
            ) -> $crate::error::AbiResult<Self> {
                if value.is_empty() || value.data_ptr().is_null() {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::NullPointer,
                        "the handle carries no object",
                    ));
                }
                // The address, the alignment and the layout revision are the
                // properties of the value itself, and none of them is implied by
                // a matching declaration. A value that fails these checks cannot
                // be released either: the table does not describe a block this
                // build may hand back, so it is left to the side that allocated
                // it instead of being dropped.
                if let Err(error) = value.validate() {
                    core::mem::forget(value);
                    return Err(error);
                }
                if !value.declarations_match(Self::ABI_SLOT_SIGNATURES) {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::InvalidDescriptor,
                        "the object was built from a different trait declaration",
                    ));
                }
                Ok(Self(value))
            }

            /// Takes ownership of an object that may declare fewer methods than
            /// this handle.
            ///
            /// A method this handle declares but the object does not answers at
            /// the call: the accessor panics, because the table has no entry to
            /// call. This is the entry point a host uses when it has to keep
            /// working with a library built against an older revision of the
            /// trait.
            pub fn abi_try_from_stable_compatible(
                value: $crate::object::StableDyn,
            ) -> $crate::error::AbiResult<Self> {
                if value.is_empty() || value.data_ptr().is_null() {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::NullPointer,
                        "the handle carries no object",
                    ));
                }
                // A value that fails these checks cannot be released either: the
                // table does not describe a block this build may hand back, so it
                // is left to the side that allocated it.
                if let Err(error) = value.validate() {
                    core::mem::forget(value);
                    return Err(error);
                }
                // The layout revision is the same number for every trait in this
                // build, so it says nothing about which trait a table belongs
                // to. The slot lists do: a table that shares no whole
                // declaration with this one belongs to something else, and
                // calling through it would reach code with a different
                // signature.
                if !value.declarations_overlap(Self::ABI_SLOT_SIGNATURES) {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::InvalidDescriptor,
                        "the object's methods are not a revision of this declaration",
                    ));
                }
                Ok(Self(value))
            }

            /// Takes ownership of an existing object without inspecting it.
            ///
            /// # Panics
            ///
            /// Panics when the object is empty or carries a table built from a
            /// different declaration; use [`Self::abi_try_from_stable`] to
            /// handle that case.
            pub fn abi_from_stable(value: $crate::object::StableDyn) -> Self {
                match Self::abi_try_from_stable(value) {
                    Ok(object) => object,
                    Err(error) => panic!("failed to adopt an ABI object: {}", error),
                }
            }

            /// Takes ownership of an existing object without inspecting it.
            ///
            /// # Safety
            ///
            /// The object must have been built from this same trait
            /// declaration; no check is performed.
            pub unsafe fn abi_from_stable_unchecked(value: $crate::object::StableDyn) -> Self {
                Self(value)
            }

            /// Releases the handle and hands back the untyped object.
            pub fn abi_into_stable(self) -> $crate::object::StableDyn {
                self.0
            }

            /// The untyped object behind the handle.
            pub fn abi_as_stable(&self) -> &$crate::object::StableDyn {
                &self.0
            }

            /// The identifier recorded when the object was built.
            pub fn abi_type_id(&self) -> u64 {
                self.0.type_id()
            }

            /// The table signature recorded when the object was built.
            pub fn abi_signature(&self) -> u64 {
                self.0.signature()
            }

            /// Reports whether the handle carries no object.
            pub fn abi_is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }
    };

    // An attribute on a method is read before the method is, and it lands in one
    // of two lists. A field and an array element accept `#[cfg]` but not a
    // function-only attribute such as `#[inline]`, so the list a field receives
    // holds only the attributes that decide whether the method exists at all.
    // The whole list still reaches the trait method, the accessor and the
    // trampoline, where every attribute is legal.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ # [ $($inner:tt)* ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ ] [ ] # [ $($inner)* ] $($rest)* ]
            [$($signature)*]
        }
    };

    // A method without a leading attribute opens the two lists as empty ones.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ fn $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ ] [ ] fn $($rest)* ]
            [$($signature)*]
        }
    };

    // An attribute that decides whether the method exists is part of every place
    // the method is written.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] # [ cfg ( $($inner:tt)* ) ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* #[cfg($($inner)*)] ] [ $(#[$full])* #[cfg($($inner)*)] ] $($rest)* ]
            [$($signature)*]
        }
    };

    // The same for the form whose predicate is evaluated where the attribute
    // lands. Only the functions receive it: what it may expand to is not known
    // here, and a field accepts a narrower set of attributes than a function
    // does, so a `cfg_attr` that expands to `#[inline]` would be refused there.
    // A method that has to disappear under a condition writes `#[cfg]`, which the
    // fields do receive.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] # [ cfg_attr ( $($inner:tt)* ) ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* #[cfg_attr($($inner)*)] ] $($rest)* ]
            [$($signature)*]
        }
    };

    // Any other attribute belongs to the functions the macro writes, and never
    // reaches a field or an array element.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] # [ $($inner:tt)* ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* #[$($inner)*] ] $($rest)* ]
            [$($signature)*]
        }
    };

    // A method with a body: the body stays in the trait, and the declaration is
    // collected as if it ended with a semicolon.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( $($sig:tt)* ) $(-> $ret:ty)? { $($body:tt)* } $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( $($sig)* ) $(-> $ret)? ; $($rest)* ]
            [$($signature)*]
        }
    };

    // A method whose declared return type is the unit type names the same call
    // as one that declares no return type, so both produce one declaration.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( $($sig:tt)* ) -> () ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( $($sig)* ) ; $($rest)* ]
            [$($signature)*]
        }
    };

    // A method that takes a shared receiver.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( & self $(, $arg:ident : $ty:ty)* $(,)? ) $(-> $ret:ty)? ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_SHARED,
                            [$($ty),*]
                            $(, $ret)?
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait.
                pub unsafe fn $name(&self $(, $arg: $ty)*) $(-> $ret)? {
                    const _: () = $crate::object::require_owned_return(
                        ::core::stringify!($($ret)?),
                    );
                    $(
                        if $crate::boundary::check(&$arg).is_err() {
                            $crate::object::abandon($arg);
                            panic!("an argument is not a value its type defines");
                        }
                    )*
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_SHARED,
                        [$($ty),*]
                        $(, $ret)?
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8 $(, $ty)*) $(-> $ret)? =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    let __abi_value = unsafe { __abi_call(self.0.data_ptr() $(, $arg)*) };
                    // A value the type does not define would be undefined
                    // behaviour downstream, so it stops here.
                    $crate::boundary::require(&__abi_value);
                    __abi_value
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(data: *mut u8 $(, $arg: $ty)*) $(-> $ret)? {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // SAFETY: the table is built for `U`, so `data` points at
                    // an initialised `U`.
                    let receiver = unsafe { &*data.cast::<U>() };
                    let __abi_value = <U as $trait_name>::$name(receiver $(, $arg)*);
                    __abi_require(&__abi_value);
                    __abi_value
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_SHARED,
                    [$($ty),*]
                    $(, $ret)?
                ),]
        }
    };

    // A method that takes a mutable receiver.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( & mut self $(, $arg:ident : $ty:ty)* $(,)? ) $(-> $ret:ty)? ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_MUTABLE,
                            [$($ty),*]
                            $(, $ret)?
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait, and no other reference to the value may be
                /// used while the call runs.
                pub unsafe fn $name(&mut self $(, $arg: $ty)*) $(-> $ret)? {
                    const _: () = $crate::object::require_owned_return(
                        ::core::stringify!($($ret)?),
                    );
                    $(
                        if $crate::boundary::check(&$arg).is_err() {
                            $crate::object::abandon($arg);
                            panic!("an argument is not a value its type defines");
                        }
                    )*
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_MUTABLE,
                        [$($ty),*]
                        $(, $ret)?
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8 $(, $ty)*) $(-> $ret)? =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    let __abi_value = unsafe { __abi_call(self.0.data_ptr() $(, $arg)*) };
                    // A value the type does not define would be undefined
                    // behaviour downstream, so it stops here.
                    $crate::boundary::require(&__abi_value);
                    __abi_value
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(data: *mut u8 $(, $arg: $ty)*) $(-> $ret)? {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // SAFETY: the table is built for `U`, so `data` points at
                    // an initialised `U`.
                    let receiver = unsafe { &mut *data.cast::<U>() };
                    let __abi_value = <U as $trait_name>::$name(receiver $(, $arg)*);
                    __abi_require(&__abi_value);
                    __abi_value
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_MUTABLE,
                    [$($ty),*]
                    $(, $ret)?
                ),]
        }
    };

    // A method that takes the receiver by value. The callee moves the value out
    // of the storage, so the handle releases the block without dropping it.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( self $(, $arg:ident : $ty:ty)* $(,)? ) $(-> $ret:ty)? ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_BY_VALUE,
                            [$($ty),*]
                            $(, $ret)?
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object, giving up the
                /// handle.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait.
                pub unsafe fn $name(self $(, $arg: $ty)*) $(-> $ret)? {
                    const _: () = $crate::object::require_owned_return(
                        ::core::stringify!($($ret)?),
                    );
                    $(
                        if $crate::boundary::check(&$arg).is_err() {
                            $crate::object::abandon($arg);
                            panic!("an argument is not a value its type defines");
                        }
                    )*
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_BY_VALUE,
                        [$($ty),*]
                        $(, $ret)?
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8 $(, $ty)*) $(-> $ret)? =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    let __abi_data = self.0.data_ptr();
                    let __abi_value = unsafe { __abi_call(__abi_data $(, $arg)*) };
                    // The callee moved the value out, so nothing inside the
                    // block is left to drop.
                    // SAFETY: see the line above.
                    unsafe { self.0.forget_value() };
                    // A value the type does not define would be undefined
                    // behaviour downstream, so it stops here.
                    $crate::boundary::require(&__abi_value);
                    __abi_value
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(data: *mut u8 $(, $arg: $ty)*) $(-> $ret)? {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // SAFETY: the caller hands over the value and releases the
                    // storage afterwards, so moving it out happens once.
                    let receiver = unsafe { $crate::__private::ptr::read(data.cast::<U>()) };
                    let __abi_value = <U as $trait_name>::$name(receiver $(, $arg)*);
                    __abi_require(&__abi_value);
                    __abi_value
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_BY_VALUE,
                    [$($ty),*]
                    $(, $ret)?
                ),]
        }
    };

    // Anything else names a shape one fixed table slot cannot carry.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ $($unparsed:tt)+ ]
        [$($signature:tt)*]
    ) => {
        ::core::compile_error!(::core::concat!(
            "a table slot has a fixed parameter list: the supported shapes are ",
            "`fn name(&self)`, `fn name(&mut self)` and `fn name(self)`, each ",
            "followed by comma-separated `argument: Type` pairs, which may end ",
            "with a comma, and an optional return type, ended by `;` or a ",
            "default body. An ",
            "associated item, `-> Self` or a borrowed return value cannot be ",
            "one slot, and neither can a generic method: one slot has one ",
            "parameter list. Declare one concrete method per instantiation in ",
            "the trait and forward from the implementation, whose own generic ",
            "method is not part of the boundary.",
        ));
    };
}

/// Declares a trait object whose methods contain panics instead of letting them
/// unwind through an `extern "C"` boundary.
///
/// The declaration has the same grammar as [`stable_object!`], but every
/// generated accessor returns [`crate::error::AbiResult`]. A panic inside the
/// implementation is caught in the trampoline and reported as
/// [`crate::error::AbiStatus::Panic`]; the report travels with the call, so it
/// survives the call when the implementation lives in another dynamic library.
/// The caller never sees the fallback value the trampoline had to produce.
///
/// Return types must implement [`crate::unwind::PanicFallback`]. Arguments and
/// return types must implement [`crate::boundary::AbiType`], which rejects a
/// declaration that names a type whose layout is not part of the boundary.
///
/// ```
/// use xt_stable_abi::{stable_object_guarded, StableString};
///
/// stable_object_guarded! {
///     /// A handle to a fallible producer.
///     pub struct ProducerObject;
///     /// The table behind [`ProducerObject`].
///     pub struct ProducerVTable;
///     /// Produces a name.
///     pub trait Producer {
///         /// Returns a name.
///         fn name(&self) -> StableString;
///     }
/// }
///
/// struct Fragile;
///
/// impl Producer for Fragile {
///     fn name(&self) -> StableString {
///         panic!("the implementation gave up");
///     }
/// }
///
/// let object = ProducerObject::abi_from_value(Fragile);
/// let error = unsafe { object.name() }.unwrap_err();
/// assert_eq!(error.status(), xt_stable_abi::AbiStatus::Panic);
/// ```
#[cfg(feature = "std")]
#[macro_export]
macro_rules! stable_object_guarded {
    (
        $(#[$object_meta:meta])*
        $object_vis:vis struct $object:ident;
        $(#[$vtable_meta:meta])*
        $vtable_vis:vis struct $vtable:ident;
        $(#[$trait_meta:meta])*
        $trait_vis:vis trait $trait_name:ident {
            $($body:tt)*
        }
    ) => {
        $(#[$trait_meta])*
        $trait_vis trait $trait_name {
            $($body)*
        }

        $(#[$object_meta])*
        #[repr(transparent)]
        $object_vis struct $object($crate::object::StableDyn);

        impl ::core::fmt::Debug for $object {
            fn fmt(&self, formatter: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                formatter
                    .debug_tuple(::core::stringify!($object))
                    .field(&self.0)
                    .finish()
            }
        }

        // SAFETY: the handle is a `#[repr(transparent)]` wrapper around
        // `StableDyn`, whose contract it inherits: dropping it releases the
        // value through the table and the allocator that value records.
        unsafe impl $crate::vec::StableElement for $object {
            unsafe fn drop_in_place(ptr: *mut Self) {
                // SAFETY: the caller guarantees an initialised value.
                unsafe { $crate::__private::ptr::drop_in_place(ptr) }
            }
        }

        // SAFETY: the handle has the layout of `StableDyn`, so it travels by
        // value exactly as that type does; `validate` checks that the address
        // the table describes is one the value can be read through, and that
        // the table shares this declaration's methods.
        unsafe impl $crate::boundary::AbiType for $object {
            fn validate(&self) -> $crate::error::AbiResult<()> {
                $crate::object::validate_handle(&self.0, Self::ABI_SLOT_SIGNATURES)
            }
        }

        // A guarded call that panicked has no object to hand over: the caller
        // reads the panic from the result the accessor returns, and the value
        // this produces is never handed out.
        impl $crate::boundary::PanicFallback for $object {
            fn panic_value() -> Self {
                Self($crate::object::StableDyn::empty())
            }
        }

        // Wiping the handle is wiping the value it carries: the field does the
        // work, because only the value's own type knows how to overwrite it.
        impl $crate::zeroize::Wipe for $object {
            fn wipe_in_place(&mut self) {
                $crate::zeroize::Wipe::wipe_in_place(&mut self.0);
            }
        }

        // SAFETY: the handle names one declaration, and the number is derived
        // from the name the macro was given, which is the same in every build of
        // that declaration.
        unsafe impl $crate::identity::AbiIdentity for $object {
            const IDENTIFIER: u64 = $crate::identity::named(::core::stringify!($object));
        }

        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$(#[$vtable_meta])*]
            []
            []
            []
            []
            [$($body)*]
            []
        }
    };

    // The same limit as in `stable_object!`: a table lists the methods of one
    // trait, and an inherited one has no slot here.
    (
        $(#[$object_meta:meta])*
        $object_vis:vis struct $object:ident;
        $(#[$vtable_meta:meta])*
        $vtable_vis:vis struct $vtable:ident;
        $(#[$trait_meta:meta])*
        $trait_vis:vis trait $trait_name:ident : $($rest:tt)*
    ) => {
        ::core::compile_error!(::core::concat!(
            "a table lists the methods of one trait, so a declaration cannot name a ",
            "supertrait: an inherited method has no slot here. Repeat the inherited ",
            "methods in this trait, or cross the boundary with an object whose own ",
            "declaration lists them.",
        ));
    };
}

#[cfg(feature = "std")]
#[doc(hidden)]
#[macro_export]
macro_rules! __stable_object_guarded_build {
    // `self: &Self` is the shared receiver written out.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( self : & Self $(, $arg:ident : $ty:ty)* $(,)? ) $(-> $ret:ty)? ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( & self $(, $arg : $ty)* ) $(-> $ret)? ; $($rest)* ]
            [$($signature)*]
        }
    };

    // `self: &mut Self` is the mutable receiver written out.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( self : & mut Self $(, $arg:ident : $ty:ty)* $(,)? ) $(-> $ret:ty)? ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( & mut self $(, $arg : $ty)* ) $(-> $ret)? ; $($rest)* ]
            [$($signature)*]
        }
    };

    // Every method has been collected.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        []
        [$($signature:tt)*]
    ) => {
        $($vtable_meta)*
        #[repr(C)]
        $vtable_vis struct $vtable {
            /// The header shared by every generated table.
            ///
            /// The field name is reserved: a method called `prefix` occupies a
            /// slot of its own and does not collide with it.
            pub __abi_prefix: $crate::object::VTablePrefix,
            $($fields)*
            /// The entry that ends the slot list.
            ///
            /// The field name is reserved. The header records how many entries
            /// precede it, so a receiver reads exactly those and the sentinel is
            /// the last field of the table.
            pub __abi_end: $crate::object::VTableSlot,
        }

        impl $object {
            /// The identifier of every method slot this build compiled, in
            /// declaration order.
            ///
            /// A method whose attributes (a `#[cfg]`, for instance) removed it
            /// from this build contributes no entry, so two builds of the same
            /// source with different features describe different tables.
            #[allow(unused_doc_comments)]
            pub const ABI_SLOT_SIGNATURES: &'static [u64] = &[ $($signature)* ];

            // A table records its entry count in one field, and a receiver reads
            // at most `MAX_SLOTS` entries. A declaration past that number would
            // build a table whose later methods no peer can see, so it is refused
            // where it is written rather than at the first call. The check sits
            // inside a function so that it is evaluated when this code is
            // compiled, whether or not anything calls the function.
            #[allow(dead_code)]
            fn __abi_slot_count_fits() {
                const _: () = assert!(
                    $object::ABI_SLOT_SIGNATURES.len() <= $crate::object::MAX_SLOTS as usize,
                    "a declaration has more methods than a table can record",
                );
            }

            /// An identifier derived from the slots this build compiled,
            /// including the fact that the calls contain panics, together with
            /// the crate's layout revision. The fold is commutative, so the order
            /// the methods are listed in does not change the number.
            pub const ABI_SIGNATURE: u64 =
                $crate::object::combined_signature(Self::ABI_SLOT_SIGNATURES);

            $($methods)*

            /// Wraps `value` in an object, using the process allocator.
            ///
            /// # Panics
            ///
            /// Panics when the storage for the value cannot be allocated.
            pub fn abi_from_value<T: $trait_name + 'static>(value: T) -> Self {
                match Self::abi_try_from_value(value) {
                    Ok(object) => object,
                    Err(error) => panic!("failed to build an ABI object: {}", error),
                }
            }

            /// Wraps `value` in an object, using the process allocator.
            ///
            /// # Panics
            ///
            /// Panics when this implementation type was already registered under
            /// a different identifier. One type has one method list and one
            /// table, and a table records the identifier it was built under, so
            /// wrapping one type under a second identifier is a mistake in the
            /// declaration rather than a failure a caller can handle; it stops
            /// here instead of leaving a table behind per identifier.
            pub fn abi_try_from_value<T: $trait_name + 'static>(
                value: T,
            ) -> $crate::error::AbiResult<Self> {
                // The fold runs where this code is compiled, so the header holds
                // a constant rather than a number computed on every build. The
                // literal it is folded from is still part of the unoptimized
                // binary: this build removes it when it optimizes, and a debug
                // build keeps it.
                const __ABI_TYPE_ID: u64 =
                    $crate::object::type_id_of(::core::stringify!($trait_name));
                Self::abi_try_from_value_with(
                    value,
                    __ABI_TYPE_ID,
                    $crate::alloc::AbiAllocator::system(),
                )
            }

            /// Wraps `value` in an object with an explicit identifier and
            /// allocator.
            pub fn abi_try_from_value_with<T: $trait_name + 'static>(
                __abi_value: T,
                __abi_type_id: u64,
                __abi_allocator: $crate::alloc::AbiAllocator,
            ) -> $crate::error::AbiResult<Self> {
                $($trampolines)*

                unsafe extern "C" fn __abi_drop_value<U>(data: *mut u8) {
                    // SAFETY: the table is built for `U`, so `data` points at
                    // an initialised `U`.
                    unsafe { $crate::__private::ptr::drop_in_place(data.cast::<U>()) }
                }

                static VTABLE_CACHE: $crate::object::VTableCache =
                    $crate::object::VTableCache::new();

                let __abi_vtable = unsafe {
                    VTABLE_CACHE.get_or_insert(
                        ($crate::__private::TypeId::of::<T>(), __abi_type_id),
                        || {
                            let mut __abi_table = $vtable {
                                __abi_prefix: $crate::object::VTablePrefix {
                                    type_id: __abi_type_id,
                                    signature: Self::ABI_SIGNATURE,
                                    layout_revision: $crate::object::LAYOUT_REVISION,
                                    size: $crate::__private::size_of::<T>() as u64,
                                    align: $crate::__private::align_of::<T>() as u64,
                                    slots: 0,
                                    drop: Some(__abi_drop_value::<T>),
                                },
                                $($values)*
                                __abi_end: $crate::object::VTableSlot {
                                    signature: 0,
                                    call: $crate::__private::ptr::null(),
                                },
                            };
                            // The header says how many entries follow it, so a
                            // receiver never has to scan past the end of a table
                            // it did not build. The number is the length of the
                            // list of compiled signatures, so a method a `#[cfg]`
                            // removed is not counted.
                            __abi_table.__abi_prefix.slots =
                                Self::ABI_SLOT_SIGNATURES.len() as u64;
                            // The entries are ordered by the signature each one
                            // declares, so a call is a binary search rather than a
                            // walk. The order is not part of the protocol: a call
                            // finds its entry by comparing numbers.
                            // SAFETY: the header now counts the entries, which were
                            // just written, and the table is not shared yet.
                            unsafe {
                                $crate::object::sort_table_entries(
                                    ::core::ptr::addr_of_mut!(__abi_table).cast::<u8>(),
                                    Self::ABI_SLOT_SIGNATURES.len() as u64,
                                )
                            };
                            // Frees the table this closure built, with the type
                            // it was allocated as, so that the memory can be given
                            // back when the module that built it is unloaded.
                            unsafe fn __abi_release_table(
                                table: *const $crate::object::VTablePrefix,
                            ) {
                                // SAFETY: the pointer came from `Box::into_raw`
                                // of this table type.
                                ::core::mem::drop(unsafe {
                                    $crate::__private::Box::from_raw(table as *mut $vtable)
                                });
                            }
                            let __abi_storage = $crate::__private::Box::into_raw(
                                $crate::__private::Box::new(__abi_table),
                            );
                            // SAFETY: `__abi_storage` came from `Box::into_raw`
                            // of this table type, and `__abi_release_table` takes
                            // that same allocation back.
                            unsafe {
                                $crate::object::BuiltTable::new(
                                    __abi_storage.cast(),
                                    __abi_release_table,
                                )
                            }
                        },
                    )
                };

                // SAFETY: the storage is allocated for `T` and written once.
                let __abi_data =
                    unsafe { $crate::object::allocate_value(__abi_value, __abi_allocator)? };
                Ok(Self(unsafe {
                    $crate::object::StableDyn::from_raw_parts(
                        __abi_data,
                        __abi_vtable,
                        __abi_allocator,
                    )
                }))
            }

            /// Takes ownership of an existing object after checking that it was
            /// built from the same guarded declaration.
            pub fn abi_try_from_stable(
                value: $crate::object::StableDyn,
            ) -> $crate::error::AbiResult<Self> {
                if value.is_empty() || value.data_ptr().is_null() {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::NullPointer,
                        "the handle carries no object",
                    ));
                }
                // The address, the alignment and the layout revision are the
                // properties of the value itself, and none of them is implied by
                // a matching declaration. A value that fails these checks cannot
                // be released either: the table does not describe a block this
                // build may hand back, so it is left to the side that allocated
                // it instead of being dropped.
                if let Err(error) = value.validate() {
                    core::mem::forget(value);
                    return Err(error);
                }
                if !value.declarations_match(Self::ABI_SLOT_SIGNATURES) {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::InvalidDescriptor,
                        "the object was built from a different trait declaration",
                    ));
                }
                Ok(Self(value))
            }

            /// Takes ownership of an object that may declare fewer methods than
            /// this handle.
            ///
            /// A method this handle declares but the object does not answers at
            /// the call: the accessor panics, because the table has no entry to
            /// call. This is the entry point a host uses when it has to keep
            /// working with a library built against an older revision of the
            /// trait.
            pub fn abi_try_from_stable_compatible(
                value: $crate::object::StableDyn,
            ) -> $crate::error::AbiResult<Self> {
                if value.is_empty() || value.data_ptr().is_null() {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::NullPointer,
                        "the handle carries no object",
                    ));
                }
                // A value that fails these checks cannot be released either: the
                // table does not describe a block this build may hand back, so it
                // is left to the side that allocated it.
                if let Err(error) = value.validate() {
                    core::mem::forget(value);
                    return Err(error);
                }
                // The layout revision is the same number for every trait in this
                // build, so it says nothing about which trait a table belongs
                // to. The slot lists do: a table that shares no whole
                // declaration with this one belongs to something else, and
                // calling through it would reach code with a different
                // signature.
                if !value.declarations_overlap(Self::ABI_SLOT_SIGNATURES) {
                    return Err($crate::error::AbiError::with_message(
                        $crate::error::AbiStatus::InvalidDescriptor,
                        "the object's methods are not a revision of this declaration",
                    ));
                }
                Ok(Self(value))
            }

            /// Takes ownership of an existing object, panicking when it cannot
            /// be adopted.
            pub fn abi_from_stable(value: $crate::object::StableDyn) -> Self {
                match Self::abi_try_from_stable(value) {
                    Ok(object) => object,
                    Err(error) => panic!("failed to adopt an ABI object: {}", error),
                }
            }

            /// Takes ownership of an existing object without inspecting it.
            ///
            /// # Safety
            ///
            /// The object must have been built from this same guarded
            /// declaration; no check is performed.
            pub unsafe fn abi_from_stable_unchecked(value: $crate::object::StableDyn) -> Self {
                Self(value)
            }

            /// Releases the handle and hands back the untyped object.
            pub fn abi_into_stable(self) -> $crate::object::StableDyn {
                self.0
            }

            /// The untyped object behind the handle.
            pub fn abi_as_stable(&self) -> &$crate::object::StableDyn {
                &self.0
            }

            /// The identifier recorded when the object was built.
            pub fn abi_type_id(&self) -> u64 {
                self.0.type_id()
            }

            /// The table signature recorded when the object was built.
            pub fn abi_signature(&self) -> u64 {
                self.0.signature()
            }

            /// Reports whether the handle carries no object.
            pub fn abi_is_empty(&self) -> bool {
                self.0.is_empty()
            }
        }
    };

    // An attribute on a method is read before the method is, and it lands in one
    // of two lists. A field and an array element accept `#[cfg]` but not a
    // function-only attribute such as `#[inline]`, so the list a field receives
    // holds only the attributes that decide whether the method exists at all.
    // The whole list still reaches the trait method, the accessor and the
    // trampoline, where every attribute is legal.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ # [ $($inner:tt)* ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ ] [ ] # [ $($inner)* ] $($rest)* ]
            [$($signature)*]
        }
    };

    // A method without a leading attribute opens the two lists as empty ones.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ fn $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ ] [ ] fn $($rest)* ]
            [$($signature)*]
        }
    };

    // An attribute that decides whether the method exists is part of every place
    // the method is written.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] # [ cfg ( $($inner:tt)* ) ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* #[cfg($($inner)*)] ] [ $(#[$full])* #[cfg($($inner)*)] ] $($rest)* ]
            [$($signature)*]
        }
    };

    // The same for the form whose predicate is evaluated where the attribute
    // lands. Only the functions receive it, for the reason the shared-receiver
    // arm gives: a field accepts a narrower set of attributes than a function.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] # [ cfg_attr ( $($inner:tt)* ) ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* #[cfg_attr($($inner)*)] ] $($rest)* ]
            [$($signature)*]
        }
    };

    // Any other attribute belongs to the functions the macro writes, and never
    // reaches a field or an array element.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] # [ $($inner:tt)* ] $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* #[$($inner)*] ] $($rest)* ]
            [$($signature)*]
        }
    };

    // A method with a body: the body stays in the trait, and the declaration is
    // collected as if it ended with a semicolon.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( $($sig:tt)* ) $(-> $ret:ty)? { $($body:tt)* } $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( $($sig)* ) $(-> $ret)? ; $($rest)* ]
            [$($signature)*]
        }
    };

    // A method whose declared return type is the unit type names the same call
    // as one that declares no return type, so both produce one declaration.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( $($sig:tt)* ) -> () ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*]
            [$($values)*]
            [$($methods)*]
            [$($trampolines)*]
            [ [ $(#[$gated])* ] [ $(#[$full])* ] fn $name ( $($sig)* ) ; $($rest)* ]
            [$($signature)*]
        }
    };

    // A method that takes a shared receiver.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( & self $(, $arg:ident : $ty:ty)* $(,)? ) -> $ret:ty ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_SHARED_GUARDED,
                            [$($ty),*],
                            $ret
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object.
                ///
                /// A panic inside the implementation is reported as
                /// [`crate::error::AbiStatus::Panic`], and an argument the
                /// receiving side refused as
                /// [`crate::error::AbiStatus::InvalidArgument`]: the second says
                /// the implementation was not entered.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait.
                pub unsafe fn $name(&self $(, $arg: $ty)*) -> $crate::error::AbiResult<$ret> {
                    const _: () = $crate::object::require_owned_return(::core::stringify!($ret));
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    // A contained panic is reported through this word. It lives
                    // in the caller's frame and is passed with the call, so the
                    // report crosses the same boundary the call does.
                    let mut __abi_panic: u32 = 0;
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_SHARED_GUARDED,
                        [$($ty),*],
                        $ret
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8, *mut u32 $(, $ty)*) -> $ret =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    let __abi_value =
                        unsafe { __abi_call(self.0.data_ptr(), &mut __abi_panic $(, $arg)*) };
                    if __abi_panic != 0 {
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    $crate::boundary::check(&__abi_value)?;
                    Ok(__abi_value)
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(
                    data: *mut u8,
                    __abi_panic: *mut u32 $(, $arg: $ty)*,
                ) -> $ret {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // Whether the argument check ran to the end. Checking an
                    // argument runs the type's own `validate`, which can panic
                    // before the implementation is entered; a borrowed receiver
                    // leaves no other trace of that, so the status word reads
                    // this flag to tell a refused argument from a panic the
                    // implementation raised.
                    let __abi_checked = ::core::cell::Cell::new(false);
                    let __abi_checked_ref = &__abi_checked;
                    let __abi_outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(
                        move || {
                            // A value the sender described incorrectly cannot be
                            // read, so the call stops before the implementation
                            // sees it, and the status word says the argument was
                            // refused rather than that the implementation gave up.
                            if !$crate::__abi_arguments_are_valid!($($arg),*) {
                                $crate::__abi_refuse_arguments!(__abi_panic, $ret $(, $arg)*);
                            }
                            __abi_checked_ref.set(true);
                            // SAFETY: the table is built for `U`, so `data`
                            // points at an initialised `U`.
                            let receiver = unsafe { &*data.cast::<U>() };
                            <U as $trait_name>::$name(receiver $(, $arg)*)
                        },
                    ));
                    match __abi_outcome {
                        Ok(value) => {
                            __abi_require(&value);
                            value
                        }
                        Err(_) => {
                            // SAFETY: the caller passes a word it owns, which
                            // outlives this call.
                            unsafe {
                                *__abi_panic = if __abi_checked.get() {
                                    $crate::object::PANICKED
                                } else {
                                    $crate::object::ARGUMENT_REFUSED
                                }
                            };
                            <$ret as $crate::unwind::PanicFallback>::panic_value()
                        }
                    }
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_SHARED_GUARDED,
                    [$($ty),*],
                    $ret
                ),]
        }
    };

    // A method that takes a mutable receiver.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( & mut self $(, $arg:ident : $ty:ty)* $(,)? ) -> $ret:ty ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_MUTABLE_GUARDED,
                            [$($ty),*],
                            $ret
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object.
                ///
                /// A panic inside the implementation is reported as
                /// [`crate::error::AbiStatus::Panic`], and an argument the
                /// receiving side refused as
                /// [`crate::error::AbiStatus::InvalidArgument`]: the second says
                /// the implementation was not entered.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait, and no other reference to the value may be
                /// used while the call runs.
                pub unsafe fn $name(&mut self $(, $arg: $ty)*) -> $crate::error::AbiResult<$ret> {
                    const _: () = $crate::object::require_owned_return(::core::stringify!($ret));
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    // A contained panic is reported through this word. It lives
                    // in the caller's frame and is passed with the call, so the
                    // report crosses the same boundary the call does.
                    let mut __abi_panic: u32 = 0;
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_MUTABLE_GUARDED,
                        [$($ty),*],
                        $ret
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8, *mut u32 $(, $ty)*) -> $ret =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    let __abi_value =
                        unsafe { __abi_call(self.0.data_ptr(), &mut __abi_panic $(, $arg)*) };
                    if __abi_panic != 0 {
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    $crate::boundary::check(&__abi_value)?;
                    Ok(__abi_value)
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(
                    data: *mut u8,
                    __abi_panic: *mut u32 $(, $arg: $ty)*,
                ) -> $ret {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // Whether the argument check ran to the end. Checking an
                    // argument runs the type's own `validate`, which can panic
                    // before the implementation is entered; a borrowed receiver
                    // leaves no other trace of that, so the status word reads
                    // this flag to tell a refused argument from a panic the
                    // implementation raised.
                    let __abi_checked = ::core::cell::Cell::new(false);
                    let __abi_checked_ref = &__abi_checked;
                    let __abi_outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(
                        move || {
                            // A value the sender described incorrectly cannot be
                            // read, so the call stops before the implementation
                            // sees it, and the status word says the argument was
                            // refused rather than that the implementation gave up.
                            if !$crate::__abi_arguments_are_valid!($($arg),*) {
                                $crate::__abi_refuse_arguments!(__abi_panic, $ret $(, $arg)*);
                            }
                            __abi_checked_ref.set(true);
                            // SAFETY: the table is built for `U`, so `data`
                            // points at an initialised `U`.
                            let receiver = unsafe { &mut *data.cast::<U>() };
                            <U as $trait_name>::$name(receiver $(, $arg)*)
                        },
                    ));
                    match __abi_outcome {
                        Ok(value) => {
                            __abi_require(&value);
                            value
                        }
                        Err(_) => {
                            // SAFETY: the caller passes a word it owns, which
                            // outlives this call.
                            unsafe {
                                *__abi_panic = if __abi_checked.get() {
                                    $crate::object::PANICKED
                                } else {
                                    $crate::object::ARGUMENT_REFUSED
                                }
                            };
                            <$ret as $crate::unwind::PanicFallback>::panic_value()
                        }
                    }
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_MUTABLE_GUARDED,
                    [$($ty),*],
                    $ret
                ),]
        }
    };

    // A method with a shared receiver and no return value.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( & self $(, $arg:ident : $ty:ty)* $(,)? ) ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_SHARED_GUARDED,
                            [$($ty),*]
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object.
                ///
                /// A panic inside the implementation is reported as
                /// [`crate::error::AbiStatus::Panic`], and an argument the
                /// receiving side refused as
                /// [`crate::error::AbiStatus::InvalidArgument`]: the second says
                /// the implementation was not entered.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait.
                pub unsafe fn $name(&self $(, $arg: $ty)*) -> $crate::error::AbiResult<()> {
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    // A contained panic is reported through this word. It lives
                    // in the caller's frame and is passed with the call, so the
                    // report crosses the same boundary the call does.
                    let mut __abi_panic: u32 = 0;
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_SHARED_GUARDED,
                        [$($ty),*]
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8, *mut u32 $(, $ty)*) =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    unsafe { __abi_call(self.0.data_ptr(), &mut __abi_panic $(, $arg)*) };
                    if __abi_panic != 0 {
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    Ok(())
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(
                    data: *mut u8,
                    __abi_panic: *mut u32 $(, $arg: $ty)*,
                ) {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // Whether the argument check ran to the end; see the arm
                    // that returns a value for why a borrowed receiver needs it.
                    let __abi_checked = ::core::cell::Cell::new(false);
                    let __abi_checked_ref = &__abi_checked;
                    let __abi_outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(
                        move || {
                            // A value the sender described incorrectly cannot be
                            // read, so the call stops before the implementation
                            // sees it, and the status word says the argument was
                            // refused rather than that the implementation gave up.
                            if !$crate::__abi_arguments_are_valid!($($arg),*) {
                                $crate::__abi_refuse_arguments!(__abi_panic, () $(, $arg)*);
                            }
                            __abi_checked_ref.set(true);
                            // SAFETY: the table is built for `U`, so `data`
                            // points at an initialised `U`.
                            let receiver = unsafe { &*data.cast::<U>() };
                            <U as $trait_name>::$name(receiver $(, $arg)*)
                        },
                    ));
                    if __abi_outcome.is_err() {
                        // SAFETY: the caller passes a word it owns, which
                        // outlives this call.
                        unsafe {
                            *__abi_panic = if __abi_checked.get() {
                                $crate::object::PANICKED
                            } else {
                                $crate::object::ARGUMENT_REFUSED
                            }
                        };
                    }
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_SHARED_GUARDED,
                    [$($ty),*]
                ),]
        }
    };

    // A method with a mutable receiver and no return value.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( & mut self $(, $arg:ident : $ty:ty)* $(,)? ) ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_MUTABLE_GUARDED,
                            [$($ty),*]
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object.
                ///
                /// A panic inside the implementation is reported as
                /// [`crate::error::AbiStatus::Panic`], and an argument the
                /// receiving side refused as
                /// [`crate::error::AbiStatus::InvalidArgument`]: the second says
                /// the implementation was not entered.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait, and no other reference to the value may be
                /// used while the call runs.
                pub unsafe fn $name(&mut self $(, $arg: $ty)*) -> $crate::error::AbiResult<()> {
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    // A contained panic is reported through this word. It lives
                    // in the caller's frame and is passed with the call, so the
                    // report crosses the same boundary the call does.
                    let mut __abi_panic: u32 = 0;
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_MUTABLE_GUARDED,
                        [$($ty),*]
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8, *mut u32 $(, $ty)*) =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    unsafe { __abi_call(self.0.data_ptr(), &mut __abi_panic $(, $arg)*) };
                    if __abi_panic != 0 {
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    Ok(())
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(
                    data: *mut u8,
                    __abi_panic: *mut u32 $(, $arg: $ty)*,
                ) {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // Whether the argument check ran to the end; see the arm
                    // that returns a value for why a borrowed receiver needs it.
                    let __abi_checked = ::core::cell::Cell::new(false);
                    let __abi_checked_ref = &__abi_checked;
                    let __abi_outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(
                        move || {
                            // A value the sender described incorrectly cannot be
                            // read, so the call stops before the implementation
                            // sees it, and the status word says the argument was
                            // refused rather than that the implementation gave up.
                            if !$crate::__abi_arguments_are_valid!($($arg),*) {
                                $crate::__abi_refuse_arguments!(__abi_panic, () $(, $arg)*);
                            }
                            __abi_checked_ref.set(true);
                            // SAFETY: the table is built for `U`, so `data`
                            // points at an initialised `U`.
                            let receiver = unsafe { &mut *data.cast::<U>() };
                            <U as $trait_name>::$name(receiver $(, $arg)*)
                        },
                    ));
                    if __abi_outcome.is_err() {
                        // SAFETY: the caller passes a word it owns, which
                        // outlives this call.
                        unsafe {
                            *__abi_panic = if __abi_checked.get() {
                                $crate::object::PANICKED
                            } else {
                                $crate::object::ARGUMENT_REFUSED
                            }
                        };
                    }
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_MUTABLE_GUARDED,
                    [$($ty),*]
                ),]
        }
    };

    // A method that takes the receiver by value and returns a value.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( self $(, $arg:ident : $ty:ty)* $(,)? ) -> $ret:ty ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_BY_VALUE_GUARDED,
                            [$($ty),*],
                            $ret
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object, giving up the
                /// handle. A panic inside the implementation is reported as
                /// [`crate::error::AbiStatus::Panic`], and an argument the
                /// receiving side refused as
                /// [`crate::error::AbiStatus::InvalidArgument`]: the second says
                /// the implementation was not entered.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait.
                pub unsafe fn $name(self $(, $arg: $ty)*) -> $crate::error::AbiResult<$ret> {
                    const _: () = $crate::object::require_owned_return(::core::stringify!($ret));
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    let mut __abi_panic: u32 = 0;
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_BY_VALUE_GUARDED,
                        [$($ty),*],
                        $ret
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8, *mut u32 $(, $ty)*) -> $ret =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    let __abi_data = self.0.data_ptr();
                    let __abi_value =
                        unsafe { __abi_call(__abi_data, &mut __abi_panic $(, $arg)*) };
                    if __abi_panic == $crate::object::ARGUMENT_REFUSED {
                        // The callee stopped before it read the value, so the
                        // handle still owns it and its own drop releases it.
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    // The callee moved the value out.
                    // SAFETY: see the line above.
                    unsafe { self.0.forget_value() };
                    if __abi_panic != 0 {
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    $crate::boundary::check(&__abi_value)?;
                    Ok(__abi_value)
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(
                    data: *mut u8,
                    __abi_panic: *mut u32 $(, $arg: $ty)*,
                ) -> $ret {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // Whether the value has left the storage the caller owns.
                    // Checking an argument runs the type's own `validate`, which
                    // can panic; a panic raised there happens before the move,
                    // and the caller's handle still owns the value, so the
                    // status word has to say the argument was refused rather
                    // than report a panic the implementation never raised.
                    let __abi_moved = ::core::cell::Cell::new(false);
                    let __abi_moved_ref = &__abi_moved;
                    let __abi_outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(
                        move || {
                            // A value the sender described incorrectly cannot be
                            // read, so the call stops before the implementation
                            // sees it, and the status word says the argument was
                            // refused rather than that the implementation gave up.
                            if !$crate::__abi_arguments_are_valid!($($arg),*) {
                                $crate::__abi_refuse_arguments!(__abi_panic, $ret $(, $arg)*);
                            }
                            // SAFETY: the caller hands over the value and
                            // releases the storage afterwards.
                            let receiver = unsafe { $crate::__private::ptr::read(data.cast::<U>()) };
                            __abi_moved_ref.set(true);
                            <U as $trait_name>::$name(receiver $(, $arg)*)
                        },
                    ));
                    match __abi_outcome {
                        Ok(value) => {
                            __abi_require(&value);
                            value
                        }
                        Err(_) => {
                            // SAFETY: the caller passes a word it owns, which
                            // outlives this call.
                            unsafe {
                                *__abi_panic = if __abi_moved.get() {
                                    $crate::object::PANICKED
                                } else {
                                    $crate::object::ARGUMENT_REFUSED
                                }
                            };
                            <$ret as $crate::unwind::PanicFallback>::panic_value()
                        }
                    }
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_BY_VALUE_GUARDED,
                    [$($ty),*],
                    $ret
                ),]
        }
    };

    // A method that takes the receiver by value and returns nothing.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ [ $(#[$gated:meta])* ] [ $(#[$full:meta])* ] fn $name:ident ( self $(, $arg:ident : $ty:ty)* $(,)? ) ; $($rest:tt)* ]
        [$($signature:tt)*]
    ) => {
        $crate::__stable_object_guarded_build! {
            [$object_vis $object]
            [$vtable_vis $vtable]
            [$trait_name]
            [$($vtable_meta)*]
            [$($fields)*
                #[doc = "The slot for this method."]
                $(#[$gated])*
                pub $name: $crate::object::VTableSlot,]
            [$($values)*
                $(#[$gated])*
                $name: $crate::object::VTableSlot {
                    signature: const {
                        $crate::__abi_signature!(
                            $name,
                            $crate::object::RECEIVER_BY_VALUE_GUARDED,
                            [$($ty),*]
                        )
                    },
                    call: $name::<T> as *const (),
                },]
            [$($methods)*
                $(#[$full])*
                /// Calls the implementation behind the object, giving up the
                /// handle. A panic inside the implementation is reported as
                /// [`crate::error::AbiStatus::Panic`], and an argument the
                /// receiving side refused as
                /// [`crate::error::AbiStatus::InvalidArgument`]: the second says
                /// the implementation was not entered.
                ///
                /// # Safety
                ///
                /// The object must have been built from an implementation of
                /// the same trait.
                pub unsafe fn $name(self $(, $arg: $ty)*) -> $crate::error::AbiResult<()> {
                    assert!(
                        !self.0.is_empty(),
                        "called {} on an ABI object that carries no value",
                        ::core::stringify!($name),
                    );
                    let mut __abi_panic: u32 = 0;
                    const __ABI_SIGNATURE: u64 = $crate::__abi_signature!(
                        $name,
                        $crate::object::RECEIVER_BY_VALUE_GUARDED,
                        [$($ty),*]
                    );
                    let __abi_signature = __ABI_SIGNATURE;
                    let __abi_call = match self.0.slot(__abi_signature) {
                        Some(__abi_call) => __abi_call,
                        None => panic!(
                            "the object does not declare the method {}",
                            ::core::stringify!($name),
                        ),
                    };
                    // SAFETY: the caller of this method states that the object
                    // was built from this declaration, and `slot` answers only
                    // for the identifier such a declaration produces. That
                    // identifier is a hash, so it narrows the match rather than
                    // proving it; the cast below and the call after it rest on
                    // the caller's contract, not on the comparison.
                    let __abi_call: unsafe extern "C" fn(*mut u8, *mut u32 $(, $ty)*) =
                        unsafe { $crate::__private::transmute(__abi_call) };
                    let __abi_data = self.0.data_ptr();
                    unsafe { __abi_call(__abi_data, &mut __abi_panic $(, $arg)*) };
                    if __abi_panic == $crate::object::ARGUMENT_REFUSED {
                        // The callee stopped before it read the value, so the
                        // handle still owns it and its own drop releases it.
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    // The callee moved the value out.
                    // SAFETY: see the line above.
                    unsafe { self.0.forget_value() };
                    if __abi_panic != 0 {
                        return Err($crate::object::outcome_error(__abi_panic));
                    }
                    Ok(())
                }]
            [$($trampolines)*
                $(#[$full])*
                // The value crosses as the layout this crate declared for it:
                // a boundary type, or the stored form of an enumeration
                // payload, which this crate writes rather than the compiler.
                #[allow(improper_ctypes_definitions)]
                unsafe extern "C" fn $name<U: $trait_name>(
                    data: *mut u8,
                    __abi_panic: *mut u32 $(, $arg: $ty)*,
                ) {
                    /// Requires the boundary-type contract of its argument.
                    fn __abi_require<T: $crate::boundary::AbiType>(_: &T) {}
                    $(__abi_require(&$arg);)*
                    // Whether the value has left the storage the caller owns.
                    // Checking an argument runs the type's own `validate`, which
                    // can panic; a panic raised there happens before the move,
                    // and the caller's handle still owns the value, so the
                    // status word has to say the argument was refused rather
                    // than report a panic the implementation never raised.
                    let __abi_moved = ::core::cell::Cell::new(false);
                    let __abi_moved_ref = &__abi_moved;
                    let __abi_outcome = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(
                        move || {
                            // A value the sender described incorrectly cannot be
                            // read, so the call stops before the implementation
                            // sees it, and the status word says the argument was
                            // refused rather than that the implementation gave up.
                            if !$crate::__abi_arguments_are_valid!($($arg),*) {
                                $crate::__abi_refuse_arguments!(__abi_panic, () $(, $arg)*);
                            }
                            // SAFETY: the caller hands over the value and
                            // releases the storage afterwards.
                            let receiver = unsafe { $crate::__private::ptr::read(data.cast::<U>()) };
                            __abi_moved_ref.set(true);
                            <U as $trait_name>::$name(receiver $(, $arg)*)
                        },
                    ));
                    if __abi_outcome.is_err() {
                        // SAFETY: the caller passes a word it owns, which
                        // outlives this call.
                        unsafe {
                            *__abi_panic = if __abi_moved.get() {
                                $crate::object::PANICKED
                            } else {
                                $crate::object::ARGUMENT_REFUSED
                            }
                        };
                    }
                }]
            [$($rest)*]
            [$($signature)*
                $(#[$gated])*
                $crate::__abi_signature!(
                    $name,
                    $crate::object::RECEIVER_BY_VALUE_GUARDED,
                    [$($ty),*]
                ),]
        }
    };

    // Anything else names a shape one fixed table slot cannot carry.
    (
        [$object_vis:vis $object:ident]
        [$vtable_vis:vis $vtable:ident]
        [$trait_name:ident]
        [$($vtable_meta:tt)*]
        [$($fields:tt)*]
        [$($values:tt)*]
        [$($methods:tt)*]
        [$($trampolines:tt)*]
        [ $($unparsed:tt)+ ]
        [$($signature:tt)*]
    ) => {
        ::core::compile_error!(::core::concat!(
            "a table slot has a fixed parameter list: the supported shapes are ",
            "`fn name(&self)`, `fn name(&mut self)` and `fn name(self)`, each ",
            "followed by comma-separated `argument: Type` pairs, which may end ",
            "with a comma, and an optional return type, ended by `;` or a ",
            "default body. An ",
            "associated item, `-> Self` or a borrowed return value cannot be ",
            "one slot, and neither can a generic method: one slot has one ",
            "parameter list. Declare one concrete method per instantiation in ",
            "the trait and forward from the implementation, whose own generic ",
            "method is not part of the boundary.",
        ));
    };
}

/// Implements [`crate::vec::StableElement`] for types whose layout is already
/// part of the ABI.
///
/// A macro cannot be `unsafe`, so the invocation *is* the `unsafe` assertion -
/// which is why the name says so - and it states, on the caller's behalf, that
/// the type has the same size, alignment and field offsets in every build that
/// exchanges it, and that dropping a value releases exactly what the value owns.
/// Reach for it for a type of your own whose layout is already fixed; it cannot
/// make a `repr(Rust)` type safe to exchange, and nothing checks the claim.
///
/// ```
/// #[repr(C)]
/// struct Pair {
///     left: u32,
///     right: u32,
/// }
///
/// xt_stable_abi::unsafe_stable_element!(Pair);
///
/// let mut values = xt_stable_abi::StableVec::new();
/// values.push(Pair { left: 1, right: 2 });
/// assert_eq!(values.len(), 1);
/// ```
#[macro_export]
macro_rules! unsafe_stable_element {
    ($($type:ty),* $(,)?) => {
        $(
            // SAFETY: the caller names a type whose layout is fixed and whose
            // drop releases exactly what it owns.
            unsafe impl $crate::vec::StableElement for $type {
                unsafe fn drop_in_place(ptr: *mut Self) {
                    // SAFETY: the caller guarantees an initialised value.
                    unsafe { ::core::ptr::drop_in_place(ptr) }
                }
            }
        )*
    };
}

/// The identifier of one slot, built from the method, the receiver form and the
/// types the method names.
///
/// The types enter as their own [`AbiIdentity`](crate::identity::AbiIdentity)
/// numbers, so how a declaration spells them does not matter: an alias, a path
/// prefix and an elided lifetime all name the type they stand for.
#[doc(hidden)]
#[macro_export]
macro_rules! __abi_signature {
    ($name:ident, $receiver:expr, [$($ty:ty),*] $(, $ret:ty)?) => {
        $crate::object::slot_id(
            ::core::stringify!($name),
            $receiver,
            &[$(<$ty as $crate::identity::AbiIdentity>::IDENTIFIER),*],
            $crate::__abi_returned!($($ret)?),
        )
    };
}

/// The identity of a method's return type, where no return names the unit type.
#[doc(hidden)]
#[macro_export]
macro_rules! __abi_returned {
    () => { <() as $crate::identity::AbiIdentity>::IDENTIFIER };
    ($ret:ty) => { <$ret as $crate::identity::AbiIdentity>::IDENTIFIER };
}

/// Names a boundary type of your own.
///
/// The number is derived from the name written here, so the two sides of a
/// declaration agree when they write the same name. The name is read where this
/// macro is invoked, which is where the implementation is written, so a type
/// alias used anywhere else resolves to this one implementation and this one
/// number; and invoking the macro twice for one type is a second implementation
/// of one trait, which the compiler refuses.
///
/// The implementation it writes is `unsafe`: a receiver that trusts the number
/// reads the bytes through the layout this side compiles, so the name has to be
/// one the other side means the same way. That is why the invocation carries the
/// assertion in its name. A name is not a module path: two types of one name in
/// two modules produce one number, so a name that crosses crates has to be
/// unique across both.
///
/// ```ignore
/// #[repr(C)]
/// pub struct Frame { width: u32, height: u32 }
///
/// // SAFETY: the layout is the one both sides compile.
/// unsafe impl xt_stable_abi::AbiType for Frame {
///     fn validate(&self) -> xt_stable_abi::AbiResult<()> { Ok(()) }
/// }
/// xt_stable_abi::unsafe_stable_identity!(Frame);
/// ```
#[macro_export]
macro_rules! unsafe_stable_identity {
    ($($type:ty),* $(,)?) => {
        $(
            // SAFETY: the identifier is derived from the name written here, and
            // both sides of a declaration write the same name.
            unsafe impl $crate::identity::AbiIdentity for $type {
                const IDENTIFIER: u64 = $crate::identity::named(::core::stringify!($type));
            }
        )*
    };
}

/// Reports whether every value a trampoline received is one its type defines.
///
/// This is the run-time half of the argument rule. The compile-time half is the
/// [`AbiType`](crate::boundary::AbiType) bound the trampoline already states; a
/// declaration naming a type that is not a boundary type fails to build either
/// way. The run-time half is what stops an implementation from reading bytes the
/// sender described incorrectly.
///
/// The check runs inside a guarded trampoline, which reports a refusal through
/// its status word. A table generated by [`stable_object!`] has no status word,
/// so the accessor of such a table runs the same check on the sending side and
/// panics on a value its type does not define; this macro is not used there.
#[doc(hidden)]
#[macro_export]
macro_rules! __abi_arguments_are_valid {
    ($($arg:expr),* $(,)?) => {{
        let mut __abi_valid = true;
        $(
            if $crate::boundary::check(&$arg).is_err() {
                __abi_valid = false;
            }
        )*
        __abi_valid
    }};
}

/// Stops a guarded trampoline whose argument was refused.
///
/// An argument that failed its check is abandoned rather than released: its
/// bytes do not describe an owner this side may hand back, so releasing it
/// would read numbers the sender did not stand behind. An argument that passed
/// is released normally, because it does describe one; abandoning it would leak
/// what the sender handed over. The status word records that the call never
/// reached the implementation, and the value returned is the one the method's
/// return type produces when the call has no answer.
///
/// This is a distinct outcome from a panic, which the implementation raises
/// after it has begun: a caller that sees
/// [`AbiStatus::InvalidArgument`](crate::error::AbiStatus::InvalidArgument) knows
/// the implementation was not entered at all.
///
/// The check runs again here instead of reading a result the first pass left
/// behind: abandoning an argument moves it, and a move on a path that continues
/// would leave the call without an argument to pass. This branch returns, so it
/// is the one place a move is allowed. The cost is one more pass over the
/// arguments, and only a call that was already refused pays it.
#[doc(hidden)]
#[macro_export]
macro_rules! __abi_refuse_arguments {
    ($word:expr, $ret:ty $(, $arg:expr)*) => {{
        $(
            if $crate::boundary::check(&$arg).is_err() {
                $crate::object::abandon($arg);
            }
        )*
        // SAFETY: the caller passes a word it owns, which outlives this call.
        unsafe { *$word = $crate::object::ARGUMENT_REFUSED };
        return <$ret as $crate::unwind::PanicFallback>::panic_value();
    }};
}
