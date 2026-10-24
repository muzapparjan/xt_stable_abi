//! Trait objects crossing the boundary, declared with the public macro.
//!
//! This file is its own crate, so everything it uses arrives through the
//! exported API: the macro's generated code, the table layout, and the
//! allocator-carrying object.

mod support;

use core::mem::size_of;
use std::sync::atomic::{AtomicUsize, Ordering};

use support::{refusing_allocator, tracked_allocator, Tracker};
use xt_stable_abi::object::type_id_of;
use xt_stable_abi::{
    stable_object, stable_object_guarded, AbiError, AbiStatus, StableBool, StableDyn, StableOption,
    StableResult, StableStr, StableString, StableVec, Wipe,
};

stable_object! {
    /// A handle to a calculator.
    pub struct CalculatorObject;
    /// The table behind [`CalculatorObject`].
    pub struct CalculatorVTable;
    /// Arithmetic with a running total.
    pub trait Calculator {
        /// Adds two numbers without touching the receiver.
        fn add(&self, left: i64, right: i64) -> i64;
        /// The implementation's name.
        fn name(&self) -> StableString;
        /// Increments the running total.
        fn bump(&mut self);
        /// The running total.
        fn value(&self) -> i64;
        /// Divides, reporting a zero denominator.
        fn divide(&self, numerator: i64, denominator: i64) -> StableResult<i64, AbiError>;
        /// The name, when `present` is true.
        fn maybe_name(&self, present: StableBool) -> StableOption<StableString>;
        /// The values the implementation recorded.
        fn history(&self) -> StableVec<i64>;
        /// Clears the running total.
        fn reset(&mut self);
        /// Builds a label from two borrowed pieces.
        fn describe(&self, prefix: StableStr, suffix: StableStr) -> StableString;
    }
}

/// Adds the operands and keeps a running total.
struct Additive {
    total: i64,
    seen: Vec<i64>,
}

impl Additive {
    fn new() -> Self {
        Self {
            total: 0,
            seen: Vec::new(),
        }
    }
}

impl Calculator for Additive {
    fn add(&self, left: i64, right: i64) -> i64 {
        left + right
    }

    fn name(&self) -> StableString {
        StableString::from_str("additive").unwrap()
    }

    fn bump(&mut self) {
        self.total += 1;
        self.seen.push(self.total);
    }

    fn value(&self) -> i64 {
        self.total
    }

    fn divide(&self, numerator: i64, denominator: i64) -> StableResult<i64, AbiError> {
        if denominator == 0 {
            StableResult::err(AbiError::with_message(
                AbiStatus::InvalidArgument,
                "division by zero",
            ))
        } else {
            StableResult::ok(numerator / denominator)
        }
    }

    fn maybe_name(&self, present: StableBool) -> StableOption<StableString> {
        match present.to_bool() {
            Ok(true) => StableOption::some(self.name()),
            _ => StableOption::none(),
        }
    }

    fn history(&self) -> StableVec<i64> {
        StableVec::from_slice(&self.seen).unwrap()
    }

    fn reset(&mut self) {
        self.total = 0;
        self.seen.clear();
    }

    fn describe(&self, prefix: StableStr<'_>, suffix: StableStr<'_>) -> StableString {
        let prefix = prefix.as_str();
        let suffix = suffix.as_str();
        StableString::from_str(&format!("{prefix}{}{suffix}", self.total)).unwrap()
    }
}

/// Multiplies the operands; a second implementation of the same trait.
struct Multiplicative {
    total: i64,
}

impl Calculator for Multiplicative {
    fn add(&self, left: i64, right: i64) -> i64 {
        left * right
    }

    fn name(&self) -> StableString {
        StableString::from_str("multiplicative").unwrap()
    }

    fn bump(&mut self) {
        self.total += 2;
    }

    fn value(&self) -> i64 {
        self.total
    }

    fn divide(&self, numerator: i64, denominator: i64) -> StableResult<i64, AbiError> {
        if denominator == 0 {
            StableResult::err(AbiError::with_message(
                AbiStatus::InvalidArgument,
                "division by zero",
            ))
        } else {
            StableResult::ok(numerator % denominator)
        }
    }

    fn maybe_name(&self, present: StableBool) -> StableOption<StableString> {
        match present.to_bool() {
            Ok(true) => StableOption::some(self.name()),
            _ => StableOption::none(),
        }
    }

    fn history(&self) -> StableVec<i64> {
        StableVec::new()
    }

    fn reset(&mut self) {
        self.total = 0;
    }

    fn describe(&self, prefix: StableStr<'_>, suffix: StableStr<'_>) -> StableString {
        let prefix = prefix.as_str();
        let suffix = suffix.as_str();
        StableString::from_str(&format!("{prefix}{}{suffix}", self.total)).unwrap()
    }
}

stable_object! {
    /// A handle to an object that reports when it is destroyed.
    pub struct TrackedObject;
    /// The table behind [`TrackedObject`].
    pub struct TrackedVTable;
    /// Reports one number.
    pub trait Tracked {
        /// The payload recorded at construction.
        fn payload(&self) -> u64;
    }
}

static TRACKED_DROPS: AtomicUsize = AtomicUsize::new(0);
static REFUSED_DROPS: AtomicUsize = AtomicUsize::new(0);
static POOL_DROPS: AtomicUsize = AtomicUsize::new(0);

/// A value that counts its own destruction.
struct Guarded {
    payload: u64,
}

impl Tracked for Guarded {
    fn payload(&self) -> u64 {
        self.payload
    }
}

impl Drop for Guarded {
    fn drop(&mut self) {
        TRACKED_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

/// A value used to observe what happens when allocation fails.
struct Refused {
    payload: u64,
}

impl Tracked for Refused {
    fn payload(&self) -> u64 {
        self.payload
    }
}

impl Drop for Refused {
    fn drop(&mut self) {
        REFUSED_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

stable_object! {
    /// A handle to an object with no methods at all.
    pub struct MarkerObject;
    /// The table behind [`MarkerObject`].
    pub struct MarkerViewTable;
    /// Carries no payload of its own; the handle still names a table.
    pub trait Marker {
        /// Returns a fixed number, so the slot is observable.
        fn prefix(&self) -> u32;
    }
}

struct Mark;

impl Marker for Mark {
    fn prefix(&self) -> u32 {
        7
    }
}

#[test]
fn every_method_shape_crosses_the_boundary() {
    let mut object = CalculatorObject::abi_from_value(Additive::new());

    assert_eq!(unsafe { object.add(2, 3) }, 5);
    assert_eq!(unsafe { object.name() }.as_str(), "additive");
    assert_eq!(unsafe { object.value() }, 0);

    unsafe { object.bump() };
    unsafe { object.bump() };
    assert_eq!(unsafe { object.value() }, 2);

    assert_eq!(unsafe { object.history() }.as_slice(), &[1, 2]);
    assert!(unsafe { object.history() }.allocator().context().is_null());

    let label = unsafe {
        object.describe(
            StableStr::from_str("total="),
            StableStr::from_str(" units"),
        )
    };
    assert_eq!(label.as_str(), "total=2 units");

    let quotient = unsafe { object.divide(9, 3) };
    assert_eq!(quotient.into_result(), Ok(3));

    let failure = unsafe { object.divide(9, 0) };
    let error = failure.into_result().unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
    assert_eq!(error.message(), "division by zero");

    let present = unsafe { object.maybe_name(StableBool::TRUE) };
    assert_eq!(present.into_option().unwrap().as_str(), "additive");
    let absent = unsafe { object.maybe_name(StableBool::FALSE) };
    assert!(absent.into_option().is_none());

    unsafe { object.reset() };
    assert_eq!(unsafe { object.value() }, 0);
    assert!(unsafe { object.history() }.is_empty());
}

#[test]
fn an_argument_the_type_does_not_define_is_refused_before_the_call() {
    /// Bytes that are not UTF-8, kept in a static so their address is stable.
    static NOT_UTF8: [u8; 2] = [0xff, 0xfe];

    let object = CalculatorObject::abi_from_value(Additive::new());
    // SAFETY: the caller promises the bytes are UTF-8, and the test is about a
    // sender that broke that promise.
    let forged = unsafe { StableStr::from_raw_parts(NOT_UTF8.as_ptr(), NOT_UTF8.len() as u64) };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        object.describe(forged, StableStr::from_str(" units"))
    }));
    // The table this object was built from has no status word, so the refusal is
    // reported by panicking on the sending side, before the call is made.
    let payload = outcome.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("an argument is not a value its type defines")
    );
}

stable_object! {
    /// A handle to a counter.
    pub struct CounterObject;
    /// The table behind [`CounterObject`].
    pub struct CounterVTable;
    /// What a counter can be asked.
    pub trait Counter {
        /// The count it holds.
        fn count(&self) -> u64;
    }
}

struct One;

impl Counter for One {
    fn count(&self) -> u64 {
        1
    }
}

#[test]
fn values_of_one_type_can_be_built_from_several_threads() {
    // The first construction registers the type: it takes the cache's lock,
    // builds the table and publishes the copy. The ones that follow read that
    // copy without locking, so this exercises both paths at once.
    let mut threads = Vec::new();
    for _ in 0..2 {
        threads.push(std::thread::spawn(|| {
            let mut built = 0;
            while built < 8 {
                let object = CounterObject::abi_from_value(One);
                // SAFETY: the value was built by this crate and is alive.
                assert_eq!(unsafe { object.count() }, 1);
                built += 1;
            }
            built
        }));
    }
    for thread in threads {
        assert_eq!(thread.join().unwrap(), 8);
    }
}

#[test]
fn a_generated_handle_can_be_wiped() {
    use xt_stable_abi::Wipe;

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Counted;

    impl Counter for Counted {
        fn count(&self) -> u64 {
            7
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    let mut object = CounterObject::abi_from_value(Counted);
    assert_eq!(unsafe { object.count() }, 7);

    object.wipe_in_place();
    // The value was released once, and the handle is empty afterwards, so
    // dropping it does not release it again.
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
    assert!(object.abi_as_stable().is_empty());
    drop(object);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}

#[test]
fn an_attribute_that_expands_to_a_function_attribute_reaches_only_functions() {
    // `cfg_attr` may expand to something a field does not accept, so the macro
    // puts it on the functions it writes - the accessor and the trampoline - and
    // leaves the table's fields alone. Gating a method is `#[cfg]`, which the
    // fields do receive.
    stable_object! {
        /// A handle to a counter declared here.
        pub struct FastObject;
        /// The table behind [`FastObject`].
        pub struct FastVTable;
        /// What it can be asked.
        pub trait Fast {
            /// A number.
            ///
            /// The default body makes this a provided method, which is a shape
            /// `#[inline]` is legal on; it stays illegal on a struct field, and
            /// the table is built from fields.
            #[cfg_attr(all(), inline)]
            fn number(&self) -> u32 {
                7
            }
        }
    }

    stable_object_guarded! {
        /// A guarded handle to a counter declared here.
        pub struct GuardedFastObject;
        /// The table behind [`GuardedFastObject`].
        pub struct GuardedFastVTable;
        /// What it can be asked.
        pub trait GuardedFast {
            /// A number.
            #[cfg_attr(all(), inline)]
            fn number(&self) -> u32 {
                7
            }
        }
    }

    struct Seven;

    impl Fast for Seven {}

    impl GuardedFast for Seven {}

    let object = FastObject::abi_from_value(Seven);
    assert_eq!(unsafe { object.number() }, 7);
    let object = GuardedFastObject::abi_from_value(Seven);
    assert_eq!(unsafe { object.number() }.unwrap(), 7);
}

#[test]
fn two_implementations_share_one_handle_type() {
    let additive = CalculatorObject::abi_from_value(Additive::new());
    let multiplicative = CalculatorObject::abi_from_value(Multiplicative { total: 10 });

    assert_eq!(unsafe { additive.add(3, 4) }, 7);
    assert_eq!(unsafe { multiplicative.add(3, 4) }, 12);
    assert_eq!(unsafe { additive.name() }.as_str(), "additive");
    assert_eq!(unsafe { multiplicative.name() }.as_str(), "multiplicative");
    assert_eq!(unsafe { multiplicative.value() }, 10);
}

#[test]
fn a_default_identifier_is_derived_from_the_trait_name() {
    let object = CalculatorObject::abi_from_value(Additive::new());
    assert_eq!(object.abi_type_id(), type_id_of("Calculator"));
    assert!(object.abi_as_stable().has_type_id(type_id_of("Calculator")));
    assert!(!object.abi_as_stable().has_type_id(type_id_of("Marker")));
}

struct Tagged;

impl Marker for Tagged {
    fn prefix(&self) -> u32 {
        123
    }
}

#[test]
fn an_explicit_identifier_is_recorded() {
    let object = MarkerObject::abi_try_from_value_with(
        Tagged,
        0x1234_5678,
        xt_stable_abi::AbiAllocator::system(),
    )
    .unwrap();
    assert_eq!(object.abi_type_id(), 0x1234_5678);
}

#[test]
fn an_implementation_type_keeps_the_identifier_it_was_registered_with() {
    // The table belongs to the implementation type, so a second identifier for
    // it is refused instead of producing another table.
    let outcome = std::panic::catch_unwind(|| {
        MarkerObject::abi_try_from_value_with(
            Tagged,
            0x1234_5678,
            xt_stable_abi::AbiAllocator::system(),
        )
        .unwrap();
        MarkerObject::abi_try_from_value_with(
            Tagged,
            0x55,
            xt_stable_abi::AbiAllocator::system(),
        )
        .unwrap()
    });
    assert!(outcome.is_err());
}

#[test]
fn the_table_is_shared_between_instances_of_one_type() {
    let first = CalculatorObject::abi_from_value(Additive::new());
    let second = CalculatorObject::abi_from_value(Additive::new());
    assert_eq!(
        first.abi_as_stable().vtable_ptr(),
        second.abi_as_stable().vtable_ptr()
    );
}

#[test]
fn the_recorded_size_matches_the_value() {    let object = CalculatorObject::abi_from_value(Additive::new());
    let untyped = object.abi_as_stable();
    assert_eq!(untyped.size(), size_of::<Additive>() as u64);
    assert_eq!(untyped.align(), core::mem::align_of::<Additive>() as u64);
    assert!(!untyped.is_empty());
}

#[test]
fn the_entries_of_a_table_are_ordered_by_signature() {
    let object = CalculatorObject::abi_from_value(Additive::new());
    let untyped = object.abi_as_stable();
    let count = untyped.slots();
    assert_eq!(count, CalculatorObject::ABI_SLOT_SIGNATURES.len() as u64);

    let mut previous = 0u64;
    for index in 0..count {
        let signature = untyped.slot_signature(index).unwrap();
        assert!(
            signature > previous,
            "entry {index} does not follow the one before it"
        );
        previous = signature;
        // Every entry the header counts answers a lookup.
        assert!(untyped.slot(signature).is_some());
    }

    // A signature the declaration never wrote is not found.
    assert!(untyped.slot(u64::MAX - 1).is_none());
}

#[test]
fn a_table_written_by_hand_is_still_found_when_it_is_not_ordered() {
    use xt_stable_abi::object::{VTablePrefix, VTableSlot};
    use xt_stable_abi::AbiAllocator;

    unsafe extern "C" fn drop_nothing(_data: *mut u8) {}
    unsafe extern "C" fn no_op() {}

    /// Two entries, written in the order a hand-written table happens to use.
    #[repr(C)]
    struct BackwardsTable {
        prefix: VTablePrefix,
        first: VTableSlot,
        second: VTableSlot,
    }

    static TABLE: BackwardsTable = BackwardsTable {
        prefix: VTablePrefix {
            type_id: 1,
            signature: 2,
            layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
            size: 8,
            align: 8,
            slots: 2,
            drop: Some(drop_nothing),
        },
        first: VTableSlot {
            signature: 900,
            call: no_op as unsafe extern "C" fn() as *const (),
        },
        second: VTableSlot {
            signature: 100,
            call: no_op as unsafe extern "C" fn() as *const (),
        },
    };

    // SAFETY: the table is immutable for the length of the test.
    unsafe impl Sync for BackwardsTable {}

    let data = Box::into_raw(Box::new(0u64));
    // SAFETY: the address is a live block and the table stands for the value.
    let value = unsafe {
        StableDyn::from_raw_parts(
            data.cast::<u8>(),
            &TABLE as *const BackwardsTable as *const VTablePrefix,
            AbiAllocator::system(),
        )
    };

    // The ordered search looks at the middle entry and finds nothing; the scan
    // that follows answers for a table this build did not order.
    assert!(value.slot(100).is_some());
    assert!(value.slot(900).is_some());
    assert!(value.slot(500).is_none());

    // SAFETY: the block came from `Box` with the layout the table records.
    drop(value);
}

#[test]
fn the_object_is_released_through_its_allocator_and_dropped_once() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    {
        let object =
            TrackedObject::abi_try_from_value_with(Guarded { payload: 7 }, 0xABCD, allocator)
                .unwrap();
        assert_eq!(unsafe { object.payload() }, 7);
        assert_eq!(object.abi_type_id(), 0xABCD);
        assert_eq!(tracker.allocations.load(Ordering::SeqCst), 1);
        assert_eq!(tracker.outstanding(), 1);
        assert_eq!(TRACKED_DROPS.load(Ordering::SeqCst), 0);
    }
    assert_eq!(TRACKED_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn wiping_an_object_releases_it_once_and_leaves_nothing_behind() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let before = TRACKED_DROPS.load(Ordering::SeqCst);
    let mut untyped =
        TrackedObject::abi_try_from_value_with(Guarded { payload: 7 }, 0xABCD, allocator)
            .unwrap()
            .abi_into_stable();

    untyped.wipe_in_place();
    assert!(untyped.is_empty());
    assert_eq!(TRACKED_DROPS.load(Ordering::SeqCst), before + 1);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 1);
    assert_eq!(tracker.outstanding(), 0);

    // The emptied handle owns nothing, so dropping it touches neither the value
    // nor the block a second time.
    drop(untyped);
    assert_eq!(TRACKED_DROPS.load(Ordering::SeqCst), before + 1);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 1);
}

#[test]
fn a_refused_allocation_drops_the_value() {
    let error = TrackedObject::abi_try_from_value_with(
        Refused { payload: 1 },
        0x99,
        refusing_allocator(),
    )
    .unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
    assert_eq!(REFUSED_DROPS.load(Ordering::SeqCst), 1);
}

#[test]
fn a_handle_roundtrips_through_the_untyped_form() {
    let object = CalculatorObject::abi_from_value(Additive::new());
    let untyped: StableDyn = object.abi_into_stable();
    let mut restored = CalculatorObject::abi_from_stable(untyped);
    assert_eq!(unsafe { restored.add(1, 1) }, 2);
    unsafe { restored.bump() };
    assert_eq!(unsafe { restored.value() }, 1);
}

#[test]
fn an_empty_handle_is_refused_by_the_checked_entry() {
    let error = CalculatorObject::abi_try_from_stable(StableDyn::empty()).unwrap_err();
    assert_eq!(error.status(), AbiStatus::NullPointer);
    assert_eq!(error.message(), "the handle carries no object");
}

#[test]
#[should_panic(expected = "the handle carries no object")]
fn the_unchecked_adoption_panics_on_an_empty_handle() {
    let _ = CalculatorObject::abi_from_stable(StableDyn::empty());
}

#[test]
fn a_table_from_another_declaration_is_refused() {
    // `Marker` and `Calculator` declare different method lists, so their tables
    // carry different signatures even though both are handles.
    let marker = MarkerObject::abi_from_value(Mark);
    let error = CalculatorObject::abi_try_from_stable(marker.abi_into_stable()).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
    assert_eq!(
        error.message(),
        "the object was built from a different trait declaration"
    );
}

stable_object! {
    /// A handle whose single method takes a shared receiver.
    pub struct SharedObject;
    /// The table behind [`SharedObject`].
    pub struct SharedVTable;
    /// A trait whose only method takes a shared receiver.
    pub trait Shared {
        /// Returns a fixed number.
        fn touch(&self) -> u32;
    }
}

stable_object! {
    /// A handle whose single method takes a mutable receiver.
    pub struct MutableObject;
    /// The table behind [`MutableObject`].
    pub struct MutableVTable;
    /// A trait whose only method takes a mutable receiver.
    pub trait Mutable {
        /// Returns a fixed number.
        fn touch(&mut self) -> u32;
    }
}

struct SharedImpl;

impl Shared for SharedImpl {
    fn touch(&self) -> u32 {
        1
    }
}

struct MutableImpl;

impl Mutable for MutableImpl {
    fn touch(&mut self) -> u32 {
        2
    }
}

#[test]
fn the_signature_records_whether_the_receiver_is_mutable() {
    // The two declarations have the same method name and argument list; the
    // receiver is what decides whether a call may write through the pointer, so
    // it is part of the signature.
    assert_ne!(SharedObject::ABI_SIGNATURE, MutableObject::ABI_SIGNATURE);

    let mutable = MutableObject::abi_from_value(MutableImpl);
    let error = SharedObject::abi_try_from_stable(mutable.abi_into_stable()).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);

    let shared = SharedObject::abi_from_value(SharedImpl);
    assert_eq!(unsafe { shared.touch() }, 1);
}

#[test]
fn an_alias_names_the_type_it_stands_for() {
    use xt_stable_abi::{AbiIdentity, StableBox, StableSlice};

    // The same two methods, one written with the types spelled out and one
    // written through aliases. The slot identifier is built from the types, so
    // the alias names the type it abbreviates.
    type Text = StableString;
    type Bytes<'a> = StableSlice<'a, u8>;

    stable_object! {
        /// A handle whose declaration spells its types out.
        pub struct SpelledObject;
        /// The table behind [`SpelledObject`].
        pub struct SpelledVTable;
        /// Two methods, spelled out.
        pub trait Spelled {
            /// The text.
            fn label(&self) -> StableString;
            /// The number of bytes.
            fn measure(&self, bytes: StableSlice<u8>) -> u64;
        }
    }

    stable_object! {
        /// A handle whose declaration names the same types through aliases.
        pub struct AliasedObject;
        /// The table behind [`AliasedObject`].
        pub struct AliasedVTable;
        /// The same two methods, through aliases.
        pub trait Aliased {
            /// The text, through an alias.
            fn label(&self) -> Text;
            /// The number of bytes, through an alias.
            fn measure(&self, bytes: Bytes<'_>) -> u64;
        }
    }

    struct Both;

    impl Spelled for Both {
        fn label(&self) -> StableString {
            StableString::from_str("spelled").unwrap()
        }
        fn measure(&self, bytes: StableSlice<u8>) -> u64 {
            bytes.len()
        }
    }

    impl Aliased for Both {
        fn label(&self) -> Text {
            StableString::from_str("aliased").unwrap()
        }
        fn measure(&self, bytes: Bytes<'_>) -> u64 {
            bytes.len()
        }
    }

    assert_eq!(SpelledObject::ABI_SIGNATURE, AliasedObject::ABI_SIGNATURE);

    // An object built from one declaration is adopted by the other, and the
    // method it calls is the one the implementation wrote.
    let spelled = SpelledObject::abi_from_value(Both);
    let adopted = AliasedObject::abi_try_from_stable(spelled.abi_into_stable()).unwrap();
    assert_eq!(unsafe { adopted.label() }.as_str(), "spelled");
    assert_eq!(
        unsafe { adopted.measure(StableSlice::from_slice(&[1u8, 2])) },
        2
    );

    // Two types with the same children are still two types.
    assert_ne!(
        <StableVec<u8> as AbiIdentity>::IDENTIFIER,
        <StableBox<u8> as AbiIdentity>::IDENTIFIER
    );
    // And a container names its element.
    assert_ne!(
        <StableVec<u8> as AbiIdentity>::IDENTIFIER,
        <StableVec<u16> as AbiIdentity>::IDENTIFIER
    );
}

#[test]
fn a_type_from_another_crate_joins_by_declaring_its_identity() {
    use xt_stable_abi::{AbiIdentity, AbiType, StableElement, StablePair};

    /// A boundary type written the way a crate outside this one writes it.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Frame {
        width: u32,
        height: u32,
    }

    // SAFETY: `#[repr(C)]` over two `u32` fields, so the layout is fixed, and
    // the value owns nothing.
    unsafe impl StableElement for Frame {
        unsafe fn drop_in_place(ptr: *mut Self) {
            // SAFETY: the caller guarantees an initialised value.
            unsafe { core::ptr::drop_in_place(ptr) }
        }
    }

    // SAFETY: every bit pattern of two `u32` fields is a value of the type.
    unsafe impl AbiType for Frame {}

    xt_stable_abi::unsafe_stable_identity!(Frame);

    stable_object! {
        /// A handle whose method takes a type declared outside this crate.
        struct PictureObject;
        /// The table behind [`PictureObject`].
        struct PictureVTable;
        /// The declaration.
        trait Picture {
            /// The number of pixels the frame holds.
            fn area(&self, frame: Frame) -> u64;
        }
    }

    struct Screen;

    impl Picture for Screen {
        fn area(&self, frame: Frame) -> u64 {
            u64::from(frame.width) * u64::from(frame.height)
        }
    }

    let picture = PictureObject::abi_from_value(Screen);
    assert_eq!(unsafe { picture.area(Frame { width: 3, height: 4 }) }, 12);

    // The number comes from the implementation the type declares, and it is not
    // the number of another type that happens to be layout compatible.
    assert_eq!(
        <Frame as AbiIdentity>::IDENTIFIER,
        xt_stable_abi::identity::named("Frame")
    );
    assert_ne!(
        <Frame as AbiIdentity>::IDENTIFIER,
        <StablePair<u64, u64> as AbiIdentity>::IDENTIFIER
    );
}

#[test]
#[should_panic(expected = "a method cannot return a borrowed type")]
fn a_qualified_borrowed_return_type_is_refused_at_the_declaration() {
    // The name the rule looks at is the last segment, so writing the path out
    // gives the same diagnostic as the short form.
    xt_stable_abi::object::require_owned_return("xt_stable_abi :: StableStr");
}

#[test]
#[should_panic(expected = "a method cannot return a borrowed type")]
fn a_borrowed_return_type_nested_in_another_type_is_refused_at_the_declaration() {
    // The outer name is an owned container, so the outermost name alone would
    // pass; the borrow inside it is what the slot cannot lend.
    xt_stable_abi::object::require_owned_return("StableOption < StableStr >");
}

#[test]
#[should_panic(expected = "a method cannot return a borrowed type")]
fn a_qualified_borrowed_return_type_nested_in_another_type_is_refused_too() {
    xt_stable_abi::object::require_owned_return("StableVec < xt_stable_abi :: StableStr >");
}

#[test]
fn every_borrowed_type_is_refused_as_a_return() {
    // The boundary types that borrow name storage they do not own, so a slot
    // cannot hand one back. The spellings are written out here so that a guard
    // which loses an entry fails this test instead of letting that spelling
    // through to an error inside generated code.
    const BORROWED: [&str; 10] = [
        "StableStr",
        "StableSlice<u8>",
        "StableSlice < u8 >",
        "StableSliceMut<u8>",
        "StableSliceMut < u8 >",
        "StableCowStr",
        "StableCowSlice<u8>",
        "StableCowSlice < u8 >",
        "StableOsStr",
        "StablePath",
    ];
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let accepted: Vec<&str> = BORROWED
        .into_iter()
        .filter(|declaration| {
            std::panic::catch_unwind(|| xt_stable_abi::object::require_owned_return(declaration))
                .is_ok()
        })
        .collect();
    std::panic::set_hook(previous);
    assert!(
        accepted.is_empty(),
        "a borrowed return type was accepted: {accepted:?}"
    );
}

#[test]
fn a_method_may_carry_an_attribute_that_only_a_function_accepts() {
    // `#[inline]` cannot sit on a struct field or on an array element, so the
    // declaration keeps it for the functions it writes and leaves it off the
    // table. The attribute is the caller's, and the declaration is the one that
    // has to place it where it is legal.
    stable_object! {
        /// A handle whose method asks to be inlined.
        pub struct InlineObject;
        /// The table behind [`InlineObject`].
        pub struct InlineVTable;
        /// The declaration.
        pub trait Inline {
            /// The number.
            #[inline]
            fn fast(&self) -> u32 {
                5
            }
        }
    }

    struct Immediate;

    impl Inline for Immediate {}

    let object = InlineObject::abi_from_value(Immediate);
    assert_eq!(unsafe { object.fast() }, 5);
}

#[test]
fn a_method_can_write_through_the_storage_it_was_given() {
    use xt_stable_abi::StableSliceMut;

    // A writable view is a boundary value like any other: the caller lends the
    // storage for the call, and the implementation writes through the handle.
    stable_object! {
        /// A handle with a method that edits the caller's storage.
        pub struct EditorObject;
        /// The table behind [`EditorObject`].
        pub struct EditorVTable;
        /// The declaration.
        pub trait Editor {
            /// Multiplies every element by `factor`, in place.
            fn scale(&self, values: StableSliceMut<u32>, factor: u32);
        }
    }

    struct Doubler;

    impl Editor for Doubler {
        fn scale(&self, mut values: StableSliceMut<u32>, factor: u32) {
            for value in values.as_mut_slice() {
                *value *= factor;
            }
        }
    }

    let editor = EditorObject::abi_from_value(Doubler);
    let mut buffer = [1u32, 2, 3];
    // SAFETY: the buffer lives for the call and nothing else borrows it.
    unsafe { editor.scale(StableSliceMut::from_mut_slice(&mut buffer), 2) };
    assert_eq!(buffer, [2, 4, 6]);
}

#[test]
fn the_order_a_declaration_lists_its_methods_in_is_not_part_of_it() {
    // The same two methods, written in the other order. A call finds its slot by
    // signature, so the order changes nothing a caller can observe.
    stable_object! {
        /// A handle that lists one order.
        pub struct ForwardObject;
        /// The table behind [`ForwardObject`].
        pub struct ForwardVTable;
        /// Two methods, in the order the structure reads in.
        pub trait Forward {
            /// The first method.
            fn first(&self, value: u32) -> u32;
            /// The second method.
            fn second(&self) -> u32;
        }
    }

    stable_object! {
        /// A handle that lists the other order.
        pub struct BackwardObject;
        /// The table behind [`BackwardObject`].
        pub struct BackwardVTable;
        /// The same two methods, in the other order.
        pub trait Backward {
            /// The second method.
            fn second(&self) -> u32;
            /// The first method.
            fn first(&self, value: u32) -> u32;
        }
    }

    struct Both;

    impl Forward for Both {
        fn first(&self, value: u32) -> u32 {
            value
        }
        fn second(&self) -> u32 {
            2
        }
    }

    impl Backward for Both {
        fn second(&self) -> u32 {
            2
        }
        fn first(&self, value: u32) -> u32 {
            value
        }
    }

    assert_eq!(ForwardObject::ABI_SIGNATURE, BackwardObject::ABI_SIGNATURE);

    let forward = ForwardObject::abi_from_value(Both);
    let adopted = BackwardObject::abi_try_from_stable(forward.abi_into_stable()).unwrap();
    assert_eq!(unsafe { adopted.first(5) }, 5);
    assert_eq!(unsafe { adopted.second() }, 2);

    // A list with a different method set is still refused.
    let forward = ForwardObject::abi_from_value(Both);
    let error = MarkerObject::abi_try_from_stable(forward.abi_into_stable()).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

stable_object! {
    /// A handle to a source of a tagged value.
    pub struct TaggedSourceObject;
    /// The table behind [`TaggedSourceObject`].
    pub struct TaggedSourceVTable;
    /// Returns a tagged value.
    pub trait TaggedSource {
        /// The value.
        fn value(&self) -> StableOption<u32>;
    }
}

struct Forging;

impl TaggedSource for Forging {
    fn value(&self) -> StableOption<u32> {
        // The bytes a library built against a different definition of the tag
        // could produce: tag 9, which names neither variant.
        let forged = [9u8, 0, 0, 0, 1, 0, 0, 0];
        // SAFETY: the layout is a `u32` tag followed by a `u32` payload, so the
        // bytes describe a value of this size.
        unsafe { core::mem::transmute::<[u8; 8], StableOption<u32>>(forged) }
    }
}

#[test]
#[should_panic(expected = "the implementation returned a value the type does not define")]
fn a_value_with_a_tag_the_type_does_not_define_stops_at_the_accessor() {
    let object = TaggedSourceObject::abi_from_value(Forging);
    let _ = unsafe { object.value() };
}

#[test]
fn calling_through_an_empty_handle_reports() {
    // Only the unchecked adoption can produce such a handle; calling a method on
    // it must stop with a message rather than dereference a null table.
    let empty = unsafe { CalculatorObject::abi_from_stable_unchecked(StableDyn::empty()) };
    let outcome = std::panic::catch_unwind(|| unsafe { empty.value() });
    assert!(outcome.is_err());
}

#[test]
fn a_method_named_like_the_table_header_has_its_own_slot() {
    let object = MarkerObject::abi_from_value(Mark);
    assert!(!object.abi_is_empty());
    assert_eq!(object.abi_type_id(), type_id_of("Marker"));
    assert_eq!(object.abi_as_stable().size(), 0);
    assert_eq!(unsafe { object.prefix() }, 7);
    drop(object);
}

#[test]
fn a_mutable_receiver_sees_the_same_value_across_calls() {
    let mut object = CalculatorObject::abi_from_value(Multiplicative { total: 0 });
    for _ in 0..5 {
        unsafe { object.bump() };
    }
    assert_eq!(unsafe { object.value() }, 10);
    unsafe { object.reset() };
    assert_eq!(unsafe { object.value() }, 0);
}

stable_object! {
    /// A handle to one entry of an object pool.
    pub struct PoolObject;
    /// The table behind [`PoolObject`].
    pub struct PoolVTable;
    /// Reports the entry's identifier.
    pub trait Pooled {
        /// The identifier given at construction.
        fn identifier(&self) -> u64;
    }
}

/// A pool entry that counts its own destruction.
struct PoolItem {
    identifier: u64,
}

impl Pooled for PoolItem {
    fn identifier(&self) -> u64 {
        self.identifier
    }
}

impl Drop for PoolItem {
    fn drop(&mut self) {
        POOL_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_vector_of_objects_releases_each_one_through_its_own_allocator() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    // The table belongs to the implementation type; identity of an individual
    // entry belongs to `Handle`.
    let implementation = type_id_of("Pooled");
    {
        let mut pool: StableVec<StableDyn> = StableVec::new();
        for identifier in 0..4u64 {
            let object = PoolObject::abi_try_from_value_with(
                PoolItem { identifier },
                implementation,
                allocator,
            )
            .unwrap();
            pool.push(object.abi_into_stable());
        }
        assert_eq!(pool.len(), 4);
        assert_eq!(tracker.outstanding(), 4);
        let second = pool.get(2).expect("index 2 is inside the vector");
        assert!(second.has_type_id(implementation));
        assert_eq!(second.size(), size_of::<PoolItem>() as u64);
    }
    assert_eq!(POOL_DROPS.load(Ordering::SeqCst), 4);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn empty_value_is_inert() {
    let empty = StableDyn::empty();
    assert!(empty.is_empty());
    assert_eq!(empty.type_id(), 0);
    assert_eq!(empty.size(), 0);
    assert_eq!(empty.align(), 0);
    assert!(!empty.has_type_id(0));
}

#[test]
fn identifiers_are_stable_per_name() {
    assert_eq!(type_id_of("Calculator"), type_id_of("Calculator"));
    assert_ne!(type_id_of("Calculator"), type_id_of("Calculator2"));
    assert_ne!(type_id_of(""), type_id_of("a"));
    // A hand-computed FNV-1a value for "a".
    assert_eq!(type_id_of("a"), 0xaf63_dc4c_8601_ec8c);
}

#[test]
fn cache_returns_the_same_table_per_key() {
    use core::any::TypeId;
    use xt_stable_abi::object::{BuiltTable, VTableCache, VTablePrefix};

    static CACHE: VTableCache = VTableCache::new();

    unsafe extern "C" fn drop_nothing(_data: *mut u8) {}

    /// Frees the table the closure below builds.
    ///
    /// # Safety
    ///
    /// `table` must come from `Box::into_raw` of a `VTablePrefix`.
    unsafe fn release(table: *const VTablePrefix) {
        // SAFETY: the caller states the pointer came from `Box::into_raw`.
        ::core::mem::drop(unsafe { Box::from_raw(table as *mut VTablePrefix) });
    }

    let first = unsafe {
        CACHE.get_or_insert((TypeId::of::<u8>(), 1), || {
            BuiltTable::new(
                Box::into_raw(Box::new(VTablePrefix {
                    type_id: 1,
                    signature: 0,
                    layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
                    size: 1,
                    align: 1,
                    slots: 0,
                    drop: Some(drop_nothing),
                })),
                release,
            )
        })
    };
    let second = unsafe {
        CACHE.get_or_insert((TypeId::of::<u8>(), 1), || unreachable!())
    };
    assert_eq!(first, second);
    assert_eq!(unsafe { (*first).type_id }, 1);
}

stable_object! {
    /// A handle to a stall.
    pub struct ShopObject;
    /// The table behind [`ShopObject`].
    pub struct ShopVTable;
    /// Selling things.
    pub trait Shop {
        /// The price of one item.
        fn price(&self) -> u32;
        /// The label; an implementation may rely on this body.
        fn label(&self) -> StableString {
            StableString::from_str("shop").unwrap()
        }
        /// Records a sale. The unit return is spelled out.
        fn sell(&mut self) -> ();
        /// Consumes the handle and reports what was paid.
        fn checkout(self) -> u32;
        /// Compiled out of the trait and the table together.
        #[cfg(any())]
        fn absent(&self) -> u32;
    }
}

struct Stall {
    sales: u32,
}

impl Shop for Stall {
    fn price(&self) -> u32 {
        5
    }

    fn sell(&mut self) {
        self.sales += 1;
    }

    fn checkout(self) -> u32 {
        5
    }
}

#[test]
fn a_default_body_answers_through_the_table() {
    let mut shop = ShopObject::abi_from_value(Stall { sales: 0 });
    assert_eq!(unsafe { shop.price() }, 5);
    // The implementation does not define `label`, so the slot calls the body
    // declared in the trait.
    assert_eq!(unsafe { shop.label() }.as_str(), "shop");
    unsafe { shop.sell() };
    assert_eq!(unsafe { shop.price() }, 5);
}

#[test]
fn a_receiver_taken_by_value_releases_the_storage_once() {
    let shop = ShopObject::abi_from_value(Stall { sales: 0 });
    assert_eq!(unsafe { shop.checkout() }, 5);
}

stable_object! {
    /// The first revision of a declaration.
    pub struct VersionOneObject;
    /// The table behind [`VersionOneObject`].
    pub struct VersionOneVTable;
    /// Revision one.
    pub trait VersionOne {
        /// The shared method.
        fn one(&self) -> u32;
    }
}

stable_object! {
    /// The second revision of the same declaration.
    pub struct VersionTwoObject;
    /// The table behind [`VersionTwoObject`].
    pub struct VersionTwoVTable;
    /// Revision two, with one more method.
    pub trait VersionTwo {
        /// The shared method.
        fn one(&self) -> u32;
        /// The added method.
        fn two(&self) -> u32;
    }
}

struct RevisionOne;

impl VersionOne for RevisionOne {
    fn one(&self) -> u32 {
        1
    }
}

struct RevisionTwo;

impl VersionTwo for RevisionTwo {
    fn one(&self) -> u32 {
        1
    }

    fn two(&self) -> u32 {
        2
    }
}

#[test]
fn an_older_declaration_reads_a_newer_object() {
    let newer = VersionTwoObject::abi_from_value(RevisionTwo);
    // The strict entry refuses the object, because the declarations differ.
    let rejected = VersionOneObject::abi_try_from_stable(newer.abi_into_stable());
    assert_eq!(
        rejected.unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );

    let newer = VersionTwoObject::abi_from_value(RevisionTwo);
    let object = VersionOneObject::abi_try_from_stable_compatible(newer.abi_into_stable()).unwrap();
    assert_eq!(unsafe { object.one() }, 1);
}

#[test]
fn a_newer_declaration_calls_what_the_older_object_provides() {
    let older = VersionOneObject::abi_from_value(RevisionOne);
    let object = VersionTwoObject::abi_try_from_stable_compatible(older.abi_into_stable()).unwrap();
    assert_eq!(unsafe { object.one() }, 1);
    // The added method has no entry in the older table, so calling it stops
    // where the absence is visible rather than through a wrong slot.
    let outcome = std::panic::catch_unwind(|| unsafe { object.two() });
    assert!(outcome.is_err());
}

stable_object! {
    /// A handle to a converter.
    pub struct ConverterObject;
    /// The table behind [`ConverterObject`].
    pub struct ConverterVTable;
    /// One concrete method per instantiation of the implementation's generic
    /// method.
    pub trait Converter {
        /// The instantiation for `u32`.
        fn identity_u32(&self, value: u32) -> u32;
        /// The instantiation for `u64`.
        fn identity_u64(&self, value: u64) -> u64;
        /// The instantiation for `u32`.
        fn add_u32(&self, left: u32, right: u32) -> u32;
    }
}

struct PassThrough;

impl PassThrough {
    /// The generic operation; it is not a boundary method on its own.
    fn identity<T>(&self, value: T) -> T {
        value
    }

    /// The generic operation with a bound.
    fn add<T: core::ops::Add<Output = T>>(&self, left: T, right: T) -> T {
        left + right
    }
}

impl Converter for PassThrough {
    fn identity_u32(&self, value: u32) -> u32 {
        self.identity(value)
    }

    fn identity_u64(&self, value: u64) -> u64 {
        self.identity(value)
    }

    fn add_u32(&self, left: u32, right: u32) -> u32 {
        self.add(left, right)
    }
}

#[test]
fn a_generic_operation_is_reached_through_one_concrete_method_per_type() {
    let object = ConverterObject::abi_from_value(PassThrough);
    assert_eq!(unsafe { object.identity_u32(9) }, 9);
    assert_eq!(unsafe { object.identity_u64(9) }, 9);
    assert_eq!(unsafe { object.add_u32(2, 3) }, 5);

    // The table records the three concrete methods and nothing else; the
    // generic operation stays in the implementation.
    assert!(object.abi_as_stable().slot(u64::MAX).is_none());
    assert!(object.abi_as_stable().slots() >= 3);
}

#[test]
fn raw_parts_can_be_taken_and_returned() {
    use xt_stable_abi::object::{allocate_value, VTablePrefix};

    unsafe extern "C" fn drop_number(data: *mut u8) {
        // SAFETY: the table describes a `u32`.
        unsafe { core::ptr::drop_in_place(data.cast::<u32>()) }
    }

    static TABLE: VTablePrefix = VTablePrefix {
        type_id: 3,
        signature: 4,
        layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
        size: 4,
        align: 4,
        slots: 0,
        drop: Some(drop_number),
    };

    let allocator = xt_stable_abi::AbiAllocator::system();
    let data = unsafe { allocate_value(9u32, allocator) }.unwrap();
    let value = unsafe { StableDyn::from_raw_parts(data, &TABLE, allocator) };
    assert_eq!(value.type_id(), 3);
    assert_eq!(value.size(), 4);
    assert_eq!(value.align(), 4);

    // SAFETY: the parts are handed straight back.
    let (data, table, allocator) = unsafe { value.into_raw_parts() };
    let restored = unsafe { StableDyn::from_raw_parts(data, table, allocator) };
    assert_eq!(restored.type_id(), 3);
    drop(restored);
}

#[test]
fn allocation_failure_is_reported() {
    use xt_stable_abi::object::allocate_value;

    unsafe extern "C" fn refuse(
        _context: *mut core::ffi::c_void,
        _size: u64,
        _align: u64,
    ) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe extern "C" fn release(
        _context: *mut core::ffi::c_void,
        _ptr: *mut u8,
        _size: u64,
        _align: u64,
    ) {
    }
    unsafe extern "C" fn resize(
        _context: *mut core::ffi::c_void,
        _ptr: *mut u8,
        _old_size: u64,
        _align: u64,
        _new_size: u64,
    ) -> *mut u8 {
        core::ptr::null_mut()
    }

    let allocator =
        unsafe { xt_stable_abi::AbiAllocator::from_parts(core::ptr::null_mut(), refuse, release, resize) };
    let error = unsafe { allocate_value(7u32, allocator) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::OutOfMemory);
}

stable_object! {
    /// A declaration that shares nothing with the others in this file.
    pub struct StoreObject;
    /// The table behind [`StoreObject`].
    pub struct StoreVTable;
    /// A list of names.
    pub trait Store {
        /// The names it holds.
        fn items(&self) -> StableVec<StableString>;
    }
}

struct Items;

impl Store for Items {
    fn items(&self) -> StableVec<StableString> {
        StableVec::from_slice(&[StableString::from_str("a").unwrap()]).unwrap()
    }
}

#[test]
fn an_object_from_another_declaration_is_refused_by_the_compatible_entry() {
    // The compatible entry accepts a peer that declares a different number of
    // methods, but not a peer that declares unrelated ones: adopting it would
    // let a call reach code whose signature the table does not describe.
    let unrelated = StoreObject::abi_from_value(Items);
    let refused =
        CalculatorObject::abi_try_from_stable_compatible(unrelated.abi_into_stable());
    assert_eq!(
        refused.unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
}

stable_object! {
    /// A declaration written with the receiver spelled out.
    pub struct ShapesObject;
    /// The table behind [`ShapesObject`].
    pub struct ShapesVTable;
    /// Three ways of writing one receiver.
    ///
    /// The spelled-out receivers are what this declaration is for, so the lint
    /// that would shorten them is turned off here.
    #[allow(clippy::needless_arbitrary_self_type)]
    pub trait Shapes {
        /// The shared receiver written as `self: &Self`.
        fn value(self: &Self) -> u64;
        /// The mutable receiver written out.
        fn bump(self: &mut Self, amount: u64) -> u64;
        /// An ordinary receiver.
        fn plain(&self, amount: u64) -> u64;
    }
}

struct Counted(u64);

impl Shapes for Counted {
    #[allow(clippy::needless_arbitrary_self_type)]
    fn value(self: &Self) -> u64 {
        self.0
    }

    #[allow(clippy::needless_arbitrary_self_type)]
    fn bump(self: &mut Self, amount: u64) -> u64 {
        self.0 += amount;
        self.0
    }

    fn plain(&self, amount: u64) -> u64 {
        self.0 + amount
    }
}

#[test]
fn the_receiver_and_the_argument_list_may_be_written_out() {
    let mut object = ShapesObject::abi_from_value(Counted(4));
    // SAFETY: the value was built by this crate and is alive.
    assert_eq!(unsafe { object.value() }, 4);
    assert_eq!(unsafe { object.bump(3) }, 7);
    assert_eq!(unsafe { object.plain(1) }, 8);
}

stable_object! {
    /// A declaration of exactly two methods.
    pub struct PairObject;
    /// The table behind [`PairObject`].
    pub struct PairVTable;
    /// Two numbers.
    pub trait Pair {
        /// The first.
        fn left(&self) -> u32;
        /// The second.
        fn right(&self) -> u32;
    }
}

#[test]
fn a_table_that_names_one_method_twice_is_refused() {
    use xt_stable_abi::object::{VTablePrefix, VTableSlot, LAYOUT_REVISION};

    /// A table whose two entries name the same method.
    #[repr(C)]
    struct DuplicateTable {
        prefix: VTablePrefix,
        first: VTableSlot,
        second: VTableSlot,
        end: VTableSlot,
    }

    unsafe extern "C" fn drop_number(data: *mut u8) {
        // SAFETY: the block holds one `u32`, which is what the table records.
        unsafe { core::ptr::drop_in_place(data.cast::<u32>()) };
    }

    unsafe extern "C" fn left(_data: *mut u8) -> u32 {
        0
    }

    assert_eq!(PairObject::ABI_SLOT_SIGNATURES.len(), 2);
    let repeated = PairObject::ABI_SLOT_SIGNATURES[0];

    let table = Box::new(DuplicateTable {
        prefix: VTablePrefix {
            type_id: 5,
            // The header a table built from this declaration would carry, so
            // only the entries themselves are wrong.
            signature: xt_stable_abi::object::combined_signature(
                PairObject::ABI_SLOT_SIGNATURES,
            ),
            layout_revision: LAYOUT_REVISION,
            size: 4,
            align: 4,
            slots: 2,
            drop: Some(drop_number),
        },
        first: VTableSlot {
            signature: repeated,
            call: left as *const (),
        },
        second: VTableSlot {
            signature: repeated,
            call: left as *const (),
        },
        end: VTableSlot {
            signature: 0,
            call: core::ptr::null(),
        },
    });
    // The value keeps using the table when it is dropped, so the block has to
    // outlive this call; a test leaks it rather than naming a lifetime.
    let table: &'static DuplicateTable = Box::leak(table);

    let data = Box::into_raw(Box::new(0u32));
    // SAFETY: the block holds one `u32`, which is what the table records, and
    // the pointer names the whole table rather than only its header.
    let object = unsafe {
        StableDyn::from_raw_parts(
            data.cast::<u8>(),
            core::mem::transmute::<*const DuplicateTable, *const VTablePrefix>(table),
            xt_stable_abi::AbiAllocator::system(),
        )
    };

    // One method is missing from the table and another is named twice, so this
    // is not the declaration that built the object. Container-wise the table
    // covers the declaration, which is why the subset rule has to refuse it as
    // well: the method that lost its entry would have no slot to call.
    assert!(!object.declarations_match(PairObject::ABI_SLOT_SIGNATURES));
    assert!(!object.declarations_overlap(PairObject::ABI_SLOT_SIGNATURES));
    let refused = PairObject::abi_try_from_stable_compatible(object);
    assert_eq!(refused.unwrap_err().status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_table_whose_header_fold_matches_but_names_other_methods_is_refused() {
    use xt_stable_abi::object::{VTablePrefix, VTableSlot, LAYOUT_REVISION};

    /// A table with as many entries as the declaration has methods.
    #[repr(C)]
    struct ForgedTable {
        prefix: VTablePrefix,
        first: VTableSlot,
        second: VTableSlot,
    }

    unsafe extern "C" fn drop_number(data: *mut u8) {
        // SAFETY: the block holds one `u32`, which is what the table records.
        unsafe { core::ptr::drop_in_place(data.cast::<u32>()) };
    }

    unsafe extern "C" fn answers(_data: *mut u8) -> u32 {
        0
    }

    let own = PairObject::ABI_SLOT_SIGNATURES;
    // A method this declaration does not name, numbered so it cannot be one of
    // the declaration's own.
    let mut foreign = own[0].wrapping_add(1_000_003);
    while own.contains(&foreign) {
        foreign = foreign.wrapping_add(1);
    }

    let table = Box::new(ForgedTable {
        prefix: VTablePrefix {
            type_id: 5,
            // The header a table built from this declaration carries. The fold
            // is a 64-bit summary of the list, so a table can repeat it while
            // holding other methods; the entries are what decide.
            signature: xt_stable_abi::object::combined_signature(own),
            layout_revision: LAYOUT_REVISION,
            size: 4,
            align: 4,
            slots: 2,
            drop: Some(drop_number),
        },
        first: VTableSlot {
            signature: own[0],
            call: answers as *const (),
        },
        second: VTableSlot {
            signature: foreign,
            call: answers as *const (),
        },
    });
    // The value keeps using the table when it is dropped, so the block has to
    // outlive this call.
    let table: &'static ForgedTable = Box::leak(table);

    let data = Box::into_raw(Box::new(0u32));
    // SAFETY: the block holds one `u32`, which is what the table records, and
    // the pointer names the whole table rather than only its header.
    let object = unsafe {
        StableDyn::from_raw_parts(
            data.cast::<u8>(),
            core::mem::transmute::<*const ForgedTable, *const VTablePrefix>(table),
            xt_stable_abi::AbiAllocator::system(),
        )
    };

    // The counts agree and the header agrees, so only the entries can tell that
    // one method of the declaration has no slot.
    assert!(!object.declarations_match(own));
    assert!(!object.declarations_overlap(own));
    let refused = PairObject::abi_try_from_stable_compatible(object);
    assert_eq!(refused.unwrap_err().status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn a_table_that_declares_no_methods_is_not_an_older_revision() {
    use xt_stable_abi::object::{VTablePrefix, LAYOUT_REVISION};

    unsafe extern "C" fn drop_number(data: *mut u8) {
        // SAFETY: the block holds one `u64`, which is what the table records.
        unsafe { core::ptr::drop_in_place(data.cast::<u64>()) };
    }

    static EMPTY_TABLE: VTablePrefix = VTablePrefix {
        type_id: 1,
        signature: 2,
        layout_revision: LAYOUT_REVISION,
        size: 8,
        align: 8,
        slots: 0,
        drop: Some(drop_number),
    };

    let data = Box::into_raw(Box::new(0u64));
    // SAFETY: the block holds one `u64`, which is what the table records.
    let empty = unsafe {
        StableDyn::from_raw_parts(
            data.cast::<u8>(),
            &EMPTY_TABLE,
            xt_stable_abi::AbiAllocator::system(),
        )
    };
    // Adopting it would let every call report an absent slot, which is not what
    // "an older declaration" means.
    let refused = CalculatorObject::abi_try_from_stable_compatible(empty);
    assert_eq!(
        refused.unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
}

#[test]
#[should_panic(expected = "a method cannot return `Self`")]
fn a_parenthesised_return_type_names_the_same_type() {
    // The declaration check runs when the method list is compiled, so the
    // spelling with parentheses has to be refused by the same rule and with the
    // same message as the bare one.
    xt_stable_abi::object::require_owned_return("(Self)");
}

#[test]
fn an_object_whose_address_is_not_aligned_is_refused_at_adoption() {
    let untyped = CalculatorObject::abi_from_value(Additive::new()).abi_into_stable();
    let table = untyped.vtable_ptr();
    let allocator = untyped.allocator();
    let data = untyped.data_ptr();
    // SAFETY: the same table and the same block, named one byte past the
    // alignment the table records. The handle is only asked to validate; the
    // block itself is still owned by `untyped`.
    let moved = unsafe { StableDyn::from_raw_parts(data.add(1), table, allocator) };
    let refused = CalculatorObject::abi_try_from_stable(moved);
    assert_eq!(refused.unwrap_err().status(), AbiStatus::InvalidLayout);
    // The original still owns the block, and dropping it releases it once.
    drop(untyped);
}

stable_object_guarded! {
    /// A declaration whose method answers with a handle.
    pub struct OuterObject;
    /// The table behind [`OuterObject`].
    pub struct OuterVTable;
    /// Asks for another object.
    pub trait Outer {
        /// The handle it answers with.
        fn inner(&self) -> InnerObject;
    }
}

stable_object! {
    /// The handle [`Outer`] answers with.
    pub struct InnerObject;
    /// The table behind [`InnerObject`].
    pub struct InnerVTable;
    /// One number.
    pub trait Inner {
        /// The number.
        fn value(&self) -> u64;
    }
}

/// An implementation that answers with a handle built from another table.
///
/// A plugin that was built from another declaration can do exactly this: it
/// hands its host a value whose bytes say "an object" while the table behind it
/// names methods the host's declaration never wrote.
struct Liar;

/// A table that names one method, and not the method of [`Inner`].
#[repr(C)]
struct ForeignTable {
    prefix: xt_stable_abi::object::VTablePrefix,
    entry: xt_stable_abi::object::VTableSlot,
    end: xt_stable_abi::object::VTableSlot,
}

// SAFETY: the table is immutable and every entry is a function that takes no
// arguments; the addresses in it are read, never written.
unsafe impl Sync for ForeignTable {}

unsafe extern "C" fn drop_number(data: *mut u8) {
    // SAFETY: the block holds one `u64`, which is what the table records.
    unsafe { core::ptr::drop_in_place(data.cast::<u64>()) };
}

unsafe extern "C" fn foreign_value(_data: *mut u8) -> u64 {
    9
}

static FOREIGN: ForeignTable = ForeignTable {
    prefix: xt_stable_abi::object::VTablePrefix {
        type_id: 11,
        signature: 12,
        layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
        size: 8,
        align: 8,
        slots: 1,
        drop: Some(drop_number),
    },
    entry: xt_stable_abi::object::VTableSlot {
        signature: 13,
        call: foreign_value as *const (),
    },
    end: xt_stable_abi::object::VTableSlot {
        signature: 0,
        call: core::ptr::null(),
    },
};

impl Outer for Liar {
    fn inner(&self) -> InnerObject {
        let data = Box::into_raw(Box::new(0u64));
        // SAFETY: the block holds one `u64`, which is what the table records.
        // The pointer comes from the whole table, not from its first field, so
        // it covers the entries after the header as well.
        let value = unsafe {
            StableDyn::from_raw_parts(
                data.cast::<u8>(),
                core::mem::transmute::<*const ForeignTable, *const xt_stable_abi::object::VTablePrefix>(
                    &FOREIGN,
                ),
                xt_stable_abi::AbiAllocator::system(),
            )
        };
        InnerObject(value)
    }
}

#[test]
fn a_handle_a_method_answers_with_is_checked_against_the_declaration() {
    let outer = OuterObject::abi_from_value(Liar);
    // The accessor checks the value it is about to hand out, and the value names
    // methods this declaration does not have.
    let error = unsafe { outer.inner() }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);

    // The same value, checked on its own, is refused for the same reason.
    let value = unsafe { StableDyn::from_raw_parts(
        Box::into_raw(Box::new(0u64)).cast::<u8>(),
        core::mem::transmute::<*const ForeignTable, *const xt_stable_abi::object::VTablePrefix>(&FOREIGN),
        xt_stable_abi::AbiAllocator::system(),
    ) };
    let error = xt_stable_abi::object::validate_handle(&value, InnerObject::ABI_SLOT_SIGNATURES)
        .unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

stable_object! {
    /// A declaration whose method lists end with a comma, as Rust allows.
    pub struct TailingObject;
    /// The table behind [`TailingObject`].
    pub struct TailingVTable;
    /// One method per receiver form.
    pub trait Tailing {
        /// Reads through a shared borrow.
        fn read(&self, amount: u32,) -> u32;
        /// Writes through a mutable borrow.
        fn write(&mut self, amount: u32,);
        /// Gives the receiver up.
        fn take(self, amount: u32,) -> u32;
    }
}

struct Tally {
    total: u32,
}

impl Tailing for Tally {
    fn read(&self, amount: u32) -> u32 {
        self.total + amount
    }

    fn write(&mut self, amount: u32) {
        self.total += amount;
    }

    fn take(self, amount: u32) -> u32 {
        self.total + amount
    }
}

#[test]
fn a_method_list_may_end_with_a_comma() {
    let mut object = TailingObject::abi_from_value(Tally { total: 1 });
    assert_eq!(unsafe { object.read(2) }, 3);
    unsafe { object.write(4) };
    assert_eq!(unsafe { object.read(0) }, 5);
    assert_eq!(unsafe { object.take(6) }, 11);
}

stable_object_guarded! {
    /// The same declaration, with the status word the guarded table has.
    pub struct TailingGuardedObject;
    /// The table behind [`TailingGuardedObject`].
    pub struct TailingGuardedVTable;
    /// One method per receiver form.
    pub trait TailingGuarded {
        /// Reads through a shared borrow.
        fn read(&self, amount: u32,) -> u32;
        /// Writes through a mutable borrow.
        fn write(&mut self, amount: u32,);
        /// Gives the receiver up.
        fn take(self, amount: u32,) -> u32;
    }
}

impl TailingGuarded for Tally {
    fn read(&self, amount: u32) -> u32 {
        self.total + amount
    }

    fn write(&mut self, amount: u32) {
        self.total += amount;
    }

    fn take(self, amount: u32) -> u32 {
        self.total + amount
    }
}

#[test]
fn a_guarded_method_list_may_end_with_a_comma() {
    let mut object = TailingGuardedObject::abi_from_value(Tally { total: 1 });
    assert_eq!(unsafe { object.read(2) }.unwrap(), 3);
    unsafe { object.write(4) }.unwrap();
    assert_eq!(unsafe { object.take(6) }.unwrap(), 11);
}
