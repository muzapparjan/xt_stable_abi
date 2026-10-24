//! Panic containment for generated trait objects.

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};

use support::{tracked_allocator, Tracker};
use xt_stable_abi::{
    stable_object, stable_object_guarded, unsafe_stable_identity, AbiResult, AbiStatus, AbiType,
    StableElement, StableSlice, StableString, StableVec,
};

/// How many times the implementation of the parameterised method ran.
static CALLS: AtomicUsize = AtomicUsize::new(0);

stable_object_guarded! {
    /// A handle to an implementation that may fail.
    pub struct FragileObject;
    /// The table behind [`FragileObject`].
    pub struct FragileVTable;
    /// Produces values, possibly by panicking.
    pub trait Fragile {
        /// Returns a number.
        fn number(&self) -> u32;
        /// Returns a name.
        fn name(&self) -> StableString;
        /// Touches the receiver.
        fn touch(&mut self);
        /// Reads the counter.
        fn touches(&self) -> u32;
        /// Counts the bytes it was given.
        fn count(&self, bytes: StableVec<u8>) -> u32;
    }
}

struct Exploding {
    touches: u32,
}

impl Fragile for Exploding {
    fn number(&self) -> u32 {
        panic!("no number today")
    }

    fn name(&self) -> StableString {
        StableString::from_str("fine").unwrap()
    }

    fn touch(&mut self) {
        self.touches += 1;
    }

    fn touches(&self) -> u32 {
        self.touches
    }

    fn count(&self, bytes: StableVec<u8>) -> u32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        bytes.len() as u32
    }
}

struct ExplodingTouch {
    touches: u32,
}

impl Fragile for ExplodingTouch {
    fn number(&self) -> u32 {
        7
    }

    fn name(&self) -> StableString {
        StableString::from_str("fine").unwrap()
    }

    fn touch(&mut self) {
        panic!("cannot touch")
    }

    fn touches(&self) -> u32 {
        self.touches
    }

    fn count(&self, bytes: StableVec<u8>) -> u32 {
        CALLS.fetch_add(1, Ordering::SeqCst);
        bytes.len() as u32
    }
}

#[test]
fn a_panicking_method_is_reported_instead_of_aborting() {
    let object = FragileObject::abi_from_value(Exploding { touches: 0 });
    let error = unsafe { object.number() }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::Panic);
    assert_eq!(error.message(), "the implementation panicked");
}

#[test]
fn a_method_that_does_not_panic_still_answers() {
    let object = FragileObject::abi_from_value(Exploding { touches: 0 });
    assert_eq!(unsafe { object.name() }.unwrap().as_str(), "fine");
    assert_eq!(unsafe { object.touches() }.unwrap(), 0);
}

#[test]
fn a_panicking_unit_method_is_reported() {
    let mut object = FragileObject::abi_from_value(ExplodingTouch { touches: 0 });
    let error = unsafe { object.touch() }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::Panic);
    // A later call is unaffected: the record describes one call.
    assert_eq!(unsafe { object.number() }.unwrap(), 7);
}

#[test]
fn a_working_unit_method_reports_success() {
    let mut object = FragileObject::abi_from_value(Exploding { touches: 0 });
    assert_eq!(unsafe { object.touch() }.unwrap(), ());
    assert_eq!(unsafe { object.touches() }.unwrap(), 1);
}

stable_object_guarded! {
    /// A handle whose only method takes a mutable receiver and returns a value.
    pub struct MutableReturnObject;
    /// The table behind [`MutableReturnObject`].
    pub struct MutableReturnVTable;
    /// Returns a number and may change the value.
    pub trait MutableReturn {
        /// Advances the value and reports it.
        fn bump(&mut self) -> u32;
    }
}

stable_object_guarded! {
    /// A handle whose only method takes a shared receiver and returns a value.
    pub struct SharedReturnObject;
    /// The table behind [`SharedReturnObject`].
    pub struct SharedReturnVTable;
    /// Returns a number without changing the value.
    pub trait SharedReturn {
        /// Reports the value.
        fn bump(&self) -> u32;
    }
}

#[test]
fn the_receiver_form_is_part_of_the_signature() {
    // A table whose slot writes through the receiver is not interchangeable
    // with one whose slot only reads it, so the two declarations cannot share a
    // signature.
    assert_ne!(
        MutableReturnObject::ABI_SIGNATURE,
        SharedReturnObject::ABI_SIGNATURE
    );
}

stable_object_guarded! {
    /// A handle to a vending machine.
    pub struct VendorObject;
    /// The table behind [`VendorObject`].
    pub struct VendorVTable;
    /// Selling, with panics contained.
    pub trait Vendor {
        /// The price.
        fn price(&self) -> u32;
        /// The label; an implementation may rely on this body.
        fn label(&self) -> StableString {
            StableString::from_str("vendor").unwrap()
        }
        /// Records a sale; the unit return is spelled out.
        fn sell(&mut self) -> ();
        /// Takes the value and reports the total.
        fn finish(self) -> u32;
    }
}

struct Machine;

impl Vendor for Machine {
    fn price(&self) -> u32 {
        3
    }

    fn sell(&mut self) {}

    fn finish(self) -> u32 {
        3
    }
}

#[test]
fn the_guarded_macro_accepts_the_same_method_shapes() {
    let mut machine = VendorObject::abi_from_value(Machine);
    assert_eq!(unsafe { machine.price() }.unwrap(), 3);
    // The default body answers through the slot.
    assert_eq!(unsafe { machine.label() }.unwrap().as_str(), "vendor");
    assert_eq!(unsafe { machine.sell() }.unwrap(), ());
    assert_eq!(unsafe { machine.finish() }.unwrap(), 3);
}

stable_object! {
    /// A handle declared without containment.
    pub struct PlainObject;
    /// The table behind [`PlainObject`].
    pub struct PlainVTable;
    /// The same method list, without containment.
    pub trait Plain {
        /// Returns a number.
        fn number(&self) -> u32;
        /// Returns a name.
        fn name(&self) -> StableString;
        /// Touches the receiver.
        fn touch(&mut self);
        /// Reads the counter.
        fn touches(&self) -> u32;
    }
}

struct Calm {
    touches: u32,
}

impl Plain for Calm {
    fn number(&self) -> u32 {
        1
    }

    fn name(&self) -> StableString {
        StableString::from_str("calm").unwrap()
    }

    fn touch(&mut self) {
        self.touches += 1;
    }

    fn touches(&self) -> u32 {
        self.touches
    }
}

#[test]
fn containment_is_part_of_the_declaration() {
    // The same method list compiled without containment produces a different
    // signature, so the two handle types do not adopt each other's objects.
    assert_ne!(FragileObject::ABI_SIGNATURE, PlainObject::ABI_SIGNATURE);

    let plain = PlainObject::abi_from_value(Calm { touches: 0 });
    let error = FragileObject::abi_try_from_stable(plain.abi_into_stable()).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);
}

#[test]
fn an_argument_the_type_does_not_define_stops_the_call_before_the_implementation() {
    CALLS.store(0, Ordering::SeqCst);
    let object = FragileObject::abi_from_value(Exploding { touches: 0 });

    // A vector whose declared length is past the block behind it. A sender that
    // writes the bytes itself can say any length it likes; this is what that
    // looks like from the receiving side.
    let mut values = StableVec::from_slice(&[1u8, 2, 3]).unwrap();
    // SAFETY: the layout is the one this crate declares: an address, then a
    // length, a capacity and an allocator, each a word wide.
    unsafe {
        let header = &mut values as *mut StableVec<u8> as *mut u64;
        header.add(1).write(9);
    }
    let error = unsafe { object.count(values) }.unwrap_err();

    // The call did not complete, and the status says which side refused it: the
    // implementation was never entered, so blaming it would be wrong.
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
    assert_eq!(
        error.message(),
        "an argument is not a value its type defines"
    );
    // The implementation never saw the value, so it never read the nine bytes it
    // was told were there.
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
}

#[test]
fn a_valid_argument_reaches_the_implementation() {
    CALLS.store(0, Ordering::SeqCst);
    let object = FragileObject::abi_from_value(Exploding { touches: 0 });
    let bytes = StableVec::from_slice(&[1u8, 2, 3]).unwrap();
    assert_eq!(unsafe { object.count(bytes) }.unwrap(), 3);
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
}

/// Counts how often a consumed receiver was released.
static CONSUMED: AtomicUsize = AtomicUsize::new(0);
/// Counts how often the implementation of the consuming method ran.
static CONSULTS: AtomicUsize = AtomicUsize::new(0);

stable_object_guarded! {
    /// A handle whose method gives up the value.
    pub struct ConsumerObject;
    /// The table behind [`ConsumerObject`].
    pub struct ConsumerVTable;
    /// Consumes the receiver.
    pub trait Consumer {
        /// Reads the argument and gives the receiver up.
        fn consume(self, bytes: StableVec<u8>) -> u32;
    }
}

struct Consumed {
    marker: u64,
}

impl Drop for Consumed {
    fn drop(&mut self) {
        CONSUMED.fetch_add(1, Ordering::SeqCst);
        let _ = self.marker;
    }
}

impl Consumer for Consumed {
    fn consume(self, bytes: StableVec<u8>) -> u32 {
        CONSULTS.fetch_add(1, Ordering::SeqCst);
        bytes.len() as u32
    }
}

#[test]
fn a_refused_argument_leaves_the_receiver_to_its_own_release() {
    CONSUMED.store(0, Ordering::SeqCst);
    CONSULTS.store(0, Ordering::SeqCst);
    let object = ConsumerObject::abi_from_value(Consumed { marker: 1 });

    let mut values = StableVec::from_slice(&[1u8, 2, 3]).unwrap();
    // SAFETY: the header layout is the one this crate declares.
    unsafe {
        let header = &mut values as *mut StableVec<u8> as *mut u64;
        header.add(1).write(9);
    }

    let error = unsafe { object.consume(values) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
    // The callee stopped before it read the receiver, so the handle still owned
    // it and released it exactly once.
    assert_eq!(CONSUMED.load(Ordering::SeqCst), 1);
    assert_eq!(CONSULTS.load(Ordering::SeqCst), 0);
}

#[test]
fn a_consumed_receiver_is_released_once_after_the_call() {
    CONSUMED.store(0, Ordering::SeqCst);
    CONSULTS.store(0, Ordering::SeqCst);
    let object = ConsumerObject::abi_from_value(Consumed { marker: 2 });
    let bytes = StableVec::from_slice(&[1u8, 2, 3, 4]).unwrap();
    assert_eq!(unsafe { object.consume(bytes) }.unwrap(), 4);
    assert_eq!(CONSUMED.load(Ordering::SeqCst), 1);
    assert_eq!(CONSULTS.load(Ordering::SeqCst), 1);
}

stable_object_guarded! {
    /// A handle whose method takes two owned arguments.
    pub struct PairObject;
    /// The table behind [`PairObject`].
    pub struct PairVTable;
    /// The declaration.
    pub trait Pair {
        /// Adds the lengths, when both arguments are values their types define.
        fn take(&self, good: StableVec<u8>, bad: StableSlice<u8>) -> u32;
    }
}

struct Adder;

impl Pair for Adder {
    fn take(&self, good: StableVec<u8>, bad: StableSlice<u8>) -> u32 {
        good.len() as u32 + bad.len() as u32
    }
}

#[test]
fn a_refused_argument_does_not_take_the_valid_one_with_it() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let mut good = StableVec::with_capacity_and_allocator(allocator, 4).unwrap();
    good.push(1u8);
    // SAFETY: the handle is built from parts that describe no value this build
    // may read, which is what the receiving side refuses.
    let bad = unsafe { StableSlice::from_raw_parts(core::ptr::null(), 4) };

    let object = PairObject::abi_from_value(Adder);
    let error = unsafe { object.take(good, bad) }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
    // The argument that passed is released rather than abandoned: abandoning it
    // would leave the block the caller handed over allocated for ever.
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 1);
}

/// An argument type whose own check gives up instead of judging the value.
#[derive(Clone, Copy)]
pub struct Restive {
    marker: u64,
}

unsafe_stable_identity!(Restive);

// SAFETY: the type is one word wide, owns nothing and has the same layout in
// every build.
unsafe impl StableElement for Restive {
    unsafe fn drop_in_place(ptr: *mut Self) {
        // SAFETY: the caller passes an initialised value it does not use again.
        unsafe { core::ptr::drop_in_place(ptr) }
    }
}

// SAFETY: the type is one word wide; the check below only gives up, and it does
// so before any caller can read the value it was given.
unsafe impl AbiType for Restive {
    fn validate(&self) -> AbiResult<()> {
        panic!("this type cannot judge a value")
    }
}

stable_object_guarded! {
    /// A handle whose consuming methods take an argument that gives up.
    pub struct RestiveObject;
    /// The table behind [`RestiveObject`].
    pub struct RestiveVTable;
    /// The declaration.
    pub trait RestiveTrait {
        /// Gives up the receiver and returns the argument's marker.
        fn swallow(self, value: Restive) -> u32;
        /// Gives up the receiver and returns nothing.
        fn swallow_quietly(self, value: Restive);
    }
}

impl RestiveTrait for Consumed {
    fn swallow(self, value: Restive) -> u32 {
        CONSULTS.fetch_add(1, Ordering::SeqCst);
        value.marker as u32
    }

    fn swallow_quietly(self, value: Restive) {
        CONSULTS.fetch_add(1, Ordering::SeqCst);
        let _ = value.marker;
    }
}

#[test]
fn an_argument_whose_check_gives_up_leaves_the_receiver_to_its_own_release() {
    CONSUMED.store(0, Ordering::SeqCst);
    CONSULTS.store(0, Ordering::SeqCst);
    let object = RestiveObject::abi_from_value(Consumed { marker: 3 });

    let error = unsafe { object.swallow(Restive { marker: 5 }) }.unwrap_err();
    // The check gave up before the value left the storage the handle owns, so
    // the call never reached the implementation and the handle still releases
    // the value it holds.
    assert_eq!(CONSUMED.load(Ordering::SeqCst), 1);
    assert_eq!(CONSULTS.load(Ordering::SeqCst), 0);
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
}

#[test]
fn a_unit_consuming_method_whose_argument_check_gives_up_behaves_the_same() {
    CONSUMED.store(0, Ordering::SeqCst);
    CONSULTS.store(0, Ordering::SeqCst);
    let object = RestiveObject::abi_from_value(Consumed { marker: 4 });

    let error = unsafe { object.swallow_quietly(Restive { marker: 6 }) }.unwrap_err();
    assert_eq!(CONSUMED.load(Ordering::SeqCst), 1);
    assert_eq!(CONSULTS.load(Ordering::SeqCst), 0);
    assert_eq!(error.status(), AbiStatus::InvalidArgument);
}

/// Counts how often an implementation was entered.
static ENTRIES: AtomicUsize = AtomicUsize::new(0);

stable_object_guarded! {
    /// A handle whose borrowing methods take an argument that gives up.
    pub struct RestiveReaderObject;
    /// The table behind [`RestiveReaderObject`].
    pub struct RestiveReaderVTable;
    /// Every receiver form that borrows.
    pub trait RestiveReader {
        /// Reads through a shared borrow and returns a value.
        fn read(&self, value: Restive) -> u32;
        /// Reads through a shared borrow and returns nothing.
        fn read_quietly(&self, value: Restive);
        /// Writes through a mutable borrow and returns a value.
        fn bump(&mut self, value: Restive) -> u32;
        /// Writes through a mutable borrow and returns nothing.
        fn bump_quietly(&mut self, value: Restive);
    }
}

struct Reader {
    total: u32,
}

impl RestiveReader for Reader {
    fn read(&self, value: Restive) -> u32 {
        ENTRIES.fetch_add(1, Ordering::SeqCst);
        value.marker as u32
    }

    fn read_quietly(&self, value: Restive) {
        ENTRIES.fetch_add(1, Ordering::SeqCst);
        let _ = value.marker;
    }

    fn bump(&mut self, value: Restive) -> u32 {
        ENTRIES.fetch_add(1, Ordering::SeqCst);
        self.total += value.marker as u32;
        self.total
    }

    fn bump_quietly(&mut self, value: Restive) {
        ENTRIES.fetch_add(1, Ordering::SeqCst);
        self.total += value.marker as u32;
    }
}

#[test]
fn a_borrowing_method_whose_argument_check_gives_up_reports_a_refused_argument() {
    // The check runs before the implementation is entered, whatever the
    // receiver form. A borrowed receiver leaves no "was the value moved" trace,
    // so the trampoline has to record the check itself; without that it would
    // report a panic the implementation never raised.
    ENTRIES.store(0, Ordering::SeqCst);

    let mut object = RestiveReaderObject::abi_from_value(Reader { total: 0 });
    let refused = [
        // SAFETY: the handle was built by this declaration.
        unsafe { object.read(Restive { marker: 1 }) }.unwrap_err(),
        unsafe { object.bump(Restive { marker: 2 }) }.unwrap_err(),
    ];
    for error in refused {
        assert_eq!(error.status(), AbiStatus::InvalidArgument);
    }
    assert_eq!(ENTRIES.load(Ordering::SeqCst), 0);
}

#[test]
fn a_unit_borrowing_method_whose_argument_check_gives_up_reports_the_same() {
    ENTRIES.store(0, Ordering::SeqCst);

    let mut object = RestiveReaderObject::abi_from_value(Reader { total: 0 });
    let refused = [
        // SAFETY: the handle was built by this declaration.
        unsafe { object.read_quietly(Restive { marker: 3 }) }.unwrap_err(),
        unsafe { object.bump_quietly(Restive { marker: 4 }) }.unwrap_err(),
    ];
    for error in refused {
        assert_eq!(error.status(), AbiStatus::InvalidArgument);
    }
    assert_eq!(ENTRIES.load(Ordering::SeqCst), 0);
}
