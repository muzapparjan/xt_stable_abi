//! What the enumeration macro, the mutable slice, the quadruple, the shrinking
//! vector and the hash map promise.

use xt_stable_abi::{
    stable_enum, stable_enum_unwiped, AbiResult, AbiStatus, AbiType, StableElement, StableHash,
    StableHashMap, StableHashSet, StableQuad, StableSliceMut, StableStr, StableString, StableVec,
    Wipe,
};

stable_enum! {
    /// A value that crosses the boundary.
    pub enum Sample {
        /// The union the payloads are laid over.
        payload = SamplePayload;
        /// The form the payloads are read through.
        view = SampleView;

        /// Carries nothing.
        Empty = 0,
        /// One number.
        Count(u32) = 1,
        /// One string and one number.
        Named(StableString, u32) = 2,
    }
}

#[test]
fn a_variant_reports_its_own_tag_and_no_other() {
    let empty = Sample::Empty();
    let count = Sample::Count((7,));
    let named = Sample::Named((StableString::from_str("x").unwrap(), 3));

    assert_eq!(empty.tag(), 0);
    assert_eq!(count.tag(), 1);
    assert_eq!(named.tag(), 2);

    assert!(matches!(empty.view(), SampleView::Empty(())));
    assert!(matches!(count.view(), SampleView::Count((7,))));
    match named.view() {
        SampleView::Named(payload) => {
            assert_eq!(payload.0.as_str(), "x");
            assert_eq!(payload.1, 3);
        }
        _ => panic!("the view named the wrong variant"),
    }

    <Sample as AbiType>::validate(&named).unwrap();
}

#[test]
fn a_payload_of_several_fields_keeps_the_order_the_declaration_wrote() {
    use core::mem::{offset_of, size_of};

    use xt_stable_abi::enumeration::Payload3;

    // `Payload3<u8, u32, u8>` is the form a declaration of `(u8, u32, u8)`
    // stores. The C rules put each field after the previous one, padded to its
    // own alignment, and finish the struct on the largest alignment.
    assert_eq!(offset_of!(Payload3<u8, u32, u8>, 0), 0);
    assert_eq!(offset_of!(Payload3<u8, u32, u8>, 1), 4);
    assert_eq!(offset_of!(Payload3<u8, u32, u8>, 2), 8);
    assert_eq!(size_of::<Payload3<u8, u32, u8>>(), 12);
}

#[test]
fn a_variant_attribute_that_only_a_function_accepts_reaches_the_constructor() {
    // `cfg_attr` may expand to something a field does not accept, so a variant's
    // `cfg_attr` reaches the function that builds it and not the storage or the
    // view. Gating a variant is `#[cfg]`, which every place receives.
    stable_enum! {
        /// An enumeration whose variant carries such an attribute.
        pub enum Decorated {
            /// The union the payloads are laid over.
            payload = DecoratedPayload;
            /// The form the payloads are read through.
            view = DecoratedView;

            /// Carries nothing.
            Empty = 0,
            /// One number.
            #[cfg_attr(all(), inline)]
            Count(u32) = 1,
        }
    }

    let value = Decorated::Count((4,));
    assert_eq!(value.tag(), 1);
    match value.view() {
        DecoratedView::Count(payload) => assert_eq!(payload.0, 4),
        DecoratedView::Empty(_) => panic!("the view named the wrong variant"),
    }
}

#[test]
fn a_tag_the_declaration_did_not_write_is_refused() {
    use core::mem::ManuallyDrop;

    let mut value = ManuallyDrop::new(Sample::Empty());
    let pointer = &mut *value as *mut Sample as *mut u32;
    // SAFETY: the tag is the first field of the declared layout, and the value is
    // in a `ManuallyDrop` so no destructor runs on the damaged tag.
    unsafe { pointer.write(99) };
    let error = <Sample as AbiType>::validate(&value).unwrap_err();
    assert_eq!(error.status(), AbiStatus::InvalidTag);
}

/// A payload that reports when it is released and when it is overwritten.
struct Tracker;

static RELEASED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static WIPED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

impl Drop for Tracker {
    fn drop(&mut self) {
        RELEASED.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    }
}

// SAFETY: the payload carries no content a peer reads, so nothing can be invalid.
unsafe impl AbiType for Tracker {
    fn validate(&self) -> AbiResult<()> {
        Ok(())
    }
}

// SAFETY: the payload is its own storage.
unsafe impl StableElement for Tracker {
    unsafe fn drop_in_place(pointer: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(pointer) }
    }
}

impl Wipe for Tracker {
    fn wipe_in_place(&mut self) {
        WIPED.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    }
}

xt_stable_abi::unsafe_stable_identity!(Tracker);

/// A payload the boundary must refuse.
struct Bad;

// SAFETY: `validate` refuses the value, which is what the trait's contract wants.
unsafe impl AbiType for Bad {
    fn validate(&self) -> AbiResult<()> {
        Err(xt_stable_abi::AbiError::new(AbiStatus::InvalidDescriptor))
    }
}

// SAFETY: the payload is its own storage.
unsafe impl StableElement for Bad {
    unsafe fn drop_in_place(pointer: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(pointer) }
    }
}

impl Wipe for Bad {
    fn wipe_in_place(&mut self) {}
}

xt_stable_abi::unsafe_stable_identity!(Bad);

stable_enum! {
    /// An enumeration whose payloads report what happens to them.
    enum Payloads {
        /// The union.
        payload = PayloadsPayload;
        /// The view.
        view = PayloadsView;

        /// Nothing.
        Blank = 0,
        /// One reporter.
        One(Tracker) = 1,
        /// Two reporters.
        Two(Tracker, Tracker) = 2,
        /// A payload the boundary refuses.
        Broken(Bad) = 3,
    }
}

#[test]
fn only_the_active_payload_is_released_and_wiped() {
    RELEASED.store(0, core::sync::atomic::Ordering::SeqCst);
    WIPED.store(0, core::sync::atomic::Ordering::SeqCst);

    {
        let mut value = Payloads::Two((Tracker, Tracker));
        assert_eq!(WIPED.load(core::sync::atomic::Ordering::SeqCst), 0);
        value.wipe_in_place();
        assert_eq!(WIPED.load(core::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(RELEASED.load(core::sync::atomic::Ordering::SeqCst), 0);
        // A second wipe reaches the payload again rather than being spent.
        value.wipe_in_place();
        assert_eq!(WIPED.load(core::sync::atomic::Ordering::SeqCst), 4);
    }
    assert_eq!(RELEASED.load(core::sync::atomic::Ordering::SeqCst), 2);

    RELEASED.store(0, core::sync::atomic::Ordering::SeqCst);
    {
        let value = Payloads::Blank();
        drop(value);
    }
    assert_eq!(RELEASED.load(core::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn the_check_of_an_enumeration_reaches_its_payload() {
    let value = Payloads::Broken((Bad,));
    assert_eq!(
        <Payloads as AbiType>::validate(&value).unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );

    let good = Payloads::One((Tracker,));
    <Payloads as AbiType>::validate(&good).unwrap();
}

/// Three declarations that share a name and differ in what they describe.
mod twins {
    /// The first shape.
    pub mod numbers {
        xt_stable_abi::stable_enum! {
            /// A shape.
            pub enum Shape {
                /// The union.
                payload = NumbersPayload;
                /// The view.
                view = NumbersView;

                /// Nothing.
                Empty = 0,
                /// A radius.
                Circle(f64) = 1,
            }
        }
    }

    /// The same name, a payload of another type.
    pub mod counts {
        xt_stable_abi::stable_enum! {
            /// A shape.
            pub enum Shape {
                /// The union.
                payload = CountsPayload;
                /// The view.
                view = CountsView;

                /// Nothing.
                Empty = 0,
                /// A count.
                Circle(u64) = 1,
            }
        }
    }

    /// The same name, other discriminants and another variant count.
    pub mod squares {
        xt_stable_abi::stable_enum! {
            /// A shape.
            pub enum Shape {
                /// The union.
                payload = SquaresPayload;
                /// The view.
                view = SquaresView;

                /// Nothing.
                Empty = 7,
                /// A radius.
                Circle(f64) = 8,
                /// An extent.
                Square(f64, f64) = 9,
            }
        }
    }
}

#[test]
fn a_name_alone_does_not_name_an_enumeration() {
    use xt_stable_abi::AbiIdentity;

    let numbers = <twins::numbers::Shape as AbiIdentity>::IDENTIFIER;
    let counts = <twins::counts::Shape as AbiIdentity>::IDENTIFIER;
    let squares = <twins::squares::Shape as AbiIdentity>::IDENTIFIER;

    assert_ne!(
        numbers, counts,
        "the same name with a payload of another type names one type"
    );
    assert_ne!(
        numbers, squares,
        "the same name with other discriminants names one type"
    );
    assert_ne!(counts, squares);
}

stable_enum! {
    /// A declaration whose dropped variant would repeat a discriminant.
    enum RepeatedTag {
        /// The union.
        payload = RepeatedTagPayload;
        /// The view.
        view = RepeatedTagView;

        /// A variant no build of this declaration contains.
        #[cfg(any())]
        Hidden(f64) = 0,
        /// The one this build contains, under the same discriminant.
        Visible(f64) = 0,
    }
}

#[test]
fn a_variant_this_build_dropped_is_not_part_of_the_discriminants() {
    assert_eq!(RepeatedTag::Visible((1.0,)).tag(), 0);
}

#[test]
fn a_mutable_slice_writes_into_the_buffer_it_was_given() {
    let mut buffer = [1u32, 2, 3];
    {
        let mut slice = StableSliceMut::from_mut_slice(&mut buffer);
        assert_eq!(slice.len(), 3);
        assert_eq!(slice.get(1), Some(&2));

        slice.set(1, 20).unwrap();
        assert_eq!(slice.as_slice(), &[1, 20, 3]);

        slice.copy_from_slice(&[7, 8]).unwrap();
        assert_eq!(slice.as_slice(), &[7, 8, 3]);

        let error = slice.set(3, 0).unwrap_err();
        assert_eq!(error.status(), AbiStatus::InvalidArgument);
        let error = slice.copy_from_slice(&[1, 2, 3, 4]).unwrap_err();
        assert_eq!(error.status(), AbiStatus::InvalidArgument);

        for value in slice.iter_mut() {
            *value += 1;
        }
        assert_eq!(slice.as_slice(), &[8, 9, 4]);

        <StableSliceMut<'_, u32> as AbiType>::validate(&slice).unwrap();
    }
    assert_eq!(buffer, [8, 9, 4]);
}

#[test]
fn a_quadruple_keeps_its_parts_and_orders_them() {
    let quad = StableQuad::new(1u8, 2u16, 3u32, 4u64);
    assert_eq!(quad.as_ref(), (&1, &2, &3, &4));
    assert!(StableQuad::new(1u8, 2u16, 3u32, 4u64) < StableQuad::new(1u8, 2u16, 3u32, 5u64));
    assert_eq!(StableQuad::from((1u8, 2u16, 3u32, 4u64)), quad);
    <StableQuad<u8, u16, u32, u64> as AbiType>::validate(&quad).unwrap();

    let (a, b, c, d) = quad.into_tuple();
    assert_eq!((a, b, c, d), (1, 2, 3, 4));
}

#[test]
fn shrinking_the_vector_gives_the_spare_storage_back() {
    let mut vector = StableVec::new();
    for value in 0..8u64 {
        vector.try_push(value).unwrap();
    }
    vector.try_reserve(24).unwrap();
    assert!(vector.capacity() >= 32);

    vector.try_shrink_to(24).unwrap();
    assert_eq!(vector.capacity(), 24);

    vector.try_shrink_to_fit().unwrap();
    assert_eq!(vector.capacity(), 8);
    assert_eq!(vector.as_slice(), &[0, 1, 2, 3, 4, 5, 6, 7]);

    // A request below the length stops at the length: the values are still
    // there, and the capacity is what holds them.
    vector.try_shrink_to(2).unwrap();
    assert_eq!(vector.capacity(), 8);
    assert_eq!(vector.as_slice(), &[0, 1, 2, 3, 4, 5, 6, 7]);

    while !vector.is_empty() {
        vector.pop();
    }
    vector.shrink_to_fit();
    assert_eq!(vector.capacity(), 0);
    assert_eq!(vector.len(), 0);
}

fn hash_of(text: &StableString) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.as_str().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn key(text: &str) -> StableString {
    StableString::from_str(text).unwrap()
}

fn value(number: u32) -> StableVec<u32> {
    StableVec::from_slice(&[number]).unwrap()
}

#[test]
fn a_hash_map_finds_by_hash_and_holds_one_value_per_key() {
    let mut map = StableHashMap::new();
    assert!(map.is_empty());

    for (text, number) in [("b", 2u32), ("a", 1), ("c", 3), ("a", 9)] {
        map.insert(hash_of, key(text), value(number)).unwrap();
    }
    assert_eq!(map.len(), 3);

    let found = map.get(hash_of, &key("a")).unwrap();
    assert_eq!(found.as_slice(), &[9]);
    assert!(map.get(hash_of, &key("z")).is_none());

    let removed = map.remove(hash_of, &key("b")).unwrap();
    assert_eq!(removed.as_slice(), &[2]);
    assert_eq!(map.len(), 2);
    assert!(map.remove(hash_of, &key("b")).is_none());

    <StableHashMap<StableString, StableVec<u32>> as AbiType>::validate(&map).unwrap();
}

#[test]
fn a_hash_map_is_the_same_bytes_whatever_order_it_was_filled_in() {
    let pairs = [("c", 3u32), ("a", 1), ("b", 2)];
    let mut forwards = StableHashMap::new();
    for (text, number) in pairs {
        forwards.insert(hash_of, key(text), value(number)).unwrap();
    }

    let mut backwards = StableHashMap::new();
    for (text, number) in pairs.iter().rev() {
        backwards.insert(hash_of, key(text), value(*number)).unwrap();
    }

    let left = forwards.as_entries();
    let right = backwards.as_entries();
    assert_eq!(left.len(), right.len());
    for (one, other) in left.iter().zip(right.iter()) {
        assert_eq!(one.hash, other.hash);
        assert_eq!(one.key.as_str(), other.key.as_str());
        assert_eq!(one.value.as_slice(), other.value.as_slice());
    }
}

#[test]
fn a_key_stored_under_two_numbers_is_two_entries() {
    // A second function that answers with another number for every key.
    fn other(text: &StableString) -> u64 {
        hash_of(text) ^ 0x0000_0000_0001_0000
    }

    let mut map: StableHashMap<StableString, u32> = StableHashMap::new();
    map.insert(hash_of, key("a"), 100).unwrap();
    // An entry is identified by `(hash, key)`, so this is a second entry: the map
    // does not hold the functions that produced the numbers, so it cannot tell
    // that these two name one key.
    map.insert(other, key("a"), 200).unwrap();

    assert_eq!(map.len(), 2);
    assert_eq!(*map.get(hash_of, &key("a")).unwrap(), 100);
    assert_eq!(*map.get(other, &key("a")).unwrap(), 200);
    assert_eq!(map.remove(hash_of, &key("a")), Some(100));
    assert_eq!(map.len(), 1);
}

#[test]
fn a_map_that_lists_one_entry_twice_is_refused() {
    use xt_stable_abi::StableHashEntry;

    // The same pair twice, which the order the map stores entries in forbids.
    // Only a build that did not insert them through this map can produce it, and
    // the receiver has to refuse it.
    let mut block: StableVec<StableHashEntry<StableString, u32>> = StableVec::new();
    block.try_push(StableHashEntry::new(1, key("a"), 1)).unwrap();
    block.try_push(StableHashEntry::new(1, key("a"), 2)).unwrap();
    // SAFETY: the parts are handed straight to the map below, which owns them.
    let (pointer, _, capacity, allocator) = unsafe { block.into_raw_parts() };
    let map = unsafe {
        StableHashMap::<StableString, u32>::from_raw_parts(pointer, 2, capacity, allocator)
    }
    .unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(
        map.validate().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
}

#[test]
fn a_map_built_from_entries_keeps_the_last_value_written_for_a_key() {
    let entries = [(key("a"), 1u32), (key("b"), 2), (key("a"), 7)];
    let map = StableHashMap::from_entries(hash_of, &entries).unwrap();
    assert_eq!(map.len(), 2);
    assert_eq!(*map.get(hash_of, &key("a")).unwrap(), 7);
    assert_eq!(*map.get(hash_of, &key("b")).unwrap(), 2);
}

#[test]
fn a_hash_map_entry_looks_the_key_up_once() {
    let calls = core::cell::Cell::new(0u32);
    let hash = |key: &StableString| {
        calls.set(calls.get() + 1);
        hash_of(key)
    };
    let mut map: StableHashMap<StableString, u32> = StableHashMap::new();

    assert!(!map.entry(hash, key("a")).is_occupied());
    *map.entry(hash, key("a")).or_insert(1).unwrap() += 1;
    assert_eq!(calls.get(), 2, "one hash per entry call, none for the insert");
    assert_eq!(*map.get(hash_of, &key("a")).unwrap(), 2);

    map.entry(hash, key("b")).or_default().unwrap();
    *map.entry(hash, key("b")).and_modify(|| 0, |value| *value += 5).unwrap() += 1;
    assert_eq!(*map.get(hash_of, &key("b")).unwrap(), 6);
    assert_eq!(map.len(), 2);
}

#[test]
fn wiping_a_hash_map_reaches_the_slots_past_its_length() {
    let mut map: StableHashMap<u64, u64> = StableHashMap::with_capacity(4).unwrap();
    for key in 0..3u64 {
        map.insert(|key: &u64| *key, key, key * 10).unwrap();
    }
    map.remove(|key: &u64| *key, &1).unwrap();
    map.remove(|key: &u64| *key, &2).unwrap();
    assert_eq!(map.len(), 1);

    map.wipe_in_place();
    // SAFETY: the map still owns its block, and this handle gives it up so the
    // slots past the length can be read.
    let (pointer, length, capacity, allocator) = unsafe { map.into_raw_parts() };
    assert_eq!(length, 1);
    let mut index = length;
    while index < capacity {
        // SAFETY: the slots past the length are inside the block.
        let entry = unsafe { &*pointer.add(index as usize) };
        assert_eq!(entry.hash, 0);
        assert_eq!(entry.key, 0);
        assert_eq!(entry.value, 0);
        index += 1;
    }
    // SAFETY: the block is the one the map gave up, with the same header.
    let block = unsafe { StableVec::from_raw_parts(pointer, length, capacity, allocator) }.unwrap();
    drop(block);
}

#[test]
fn a_hash_set_holds_one_of_each_value() {    let mut set = StableHashSet::new();
    assert!(set.is_empty());

    assert!(set.insert(hash_of, key("a")).unwrap());
    assert!(!set.insert(hash_of, key("a")).unwrap());
    assert!(set.insert(hash_of, key("b")).unwrap());
    assert_eq!(set.len(), 2);
    assert!(set.contains(hash_of, &key("a")));
    assert!(!set.contains(hash_of, &key("z")));

    assert!(set.remove(hash_of, &key("a")));
    assert!(!set.remove(hash_of, &key("a")));
    assert_eq!(set.len(), 1);

    let built = StableHashSet::from_values(hash_of, &[key("b"), key("a"), key("b")]).unwrap();
    assert_eq!(built.len(), 2);
    <StableHashSet<StableString> as AbiType>::validate(&built).unwrap();
}

stable_enum! {
    /// An enumeration with a variant a build may not have.
    pub enum Gated {
        /// The union the payloads are laid over.
        payload = GatedPayload;
        /// The form the payloads are read through.
        view = GatedView;
        /// Always present.
        Kept = 0,
        /// Removed by the attribute.
        #[cfg(any())]
        Gone(u64) = 1,
    }
}

stable_enum! {
    /// An enumeration whose payload has more fields than a Rust tuple implements
    /// the element traits for by hand.
    pub enum Wide {
        /// The union the payloads are laid over.
        payload = WidePayload;
        /// The form the payloads are read through.
        view = WideView;
        /// Nine numbers.
        Many(u64, u64, u64, u64, u64, u64, u64, u64, u64) = 1,
    }
}

stable_enum! {
    /// The answer of a guarded call.
    pub enum Answer {
        /// The union the payloads are laid over.
        payload = AnswerPayload;
        /// The form the payloads are read through.
        view = AnswerView;
        /// No answer.
        Nothing = 0,
        /// A number.
        Number(u64) = 1,
    }
}

xt_stable_abi::stable_enum_fallback!(Answer => Nothing());

#[test]
fn an_enumeration_is_a_type_a_declaration_can_name() {
    xt_stable_abi::stable_object! {
        /// A handle whose methods mention an enumeration.
        pub struct StoreObject;
        /// The table behind [`StoreObject`].
        pub struct StoreVTable;
        /// The declaration.
        pub trait Store {
            /// Hands back a value of the enumeration.
            fn read(&self) -> Sample;
            /// Takes one and answers with its tag.
            fn write(&self, value: Sample) -> u32;
        }
    }

    struct Impl;

    impl Store for Impl {
        fn read(&self) -> Sample {
            Sample::Count((4,))
        }
        fn write(&self, value: Sample) -> u32 {
            value.tag()
        }
    }

    let store = StoreObject::abi_from_value(Impl);
    assert_eq!(unsafe { store.read() }.tag(), 1);
    assert_eq!(unsafe { store.write(Sample::Named((StableString::from_str("x").unwrap(), 2))) }, 2);
}

stable_enum_unwiped! {
    /// An enumeration whose payload type cannot be wiped.
    ///
    /// `StableStr` borrows its text, so there is nothing of its own to wipe. The
    /// declaration says so by asking for no implementation at all; `stable_enum!`
    /// would refuse it, because it writes a `Wipe` that has to cover every
    /// payload.
    pub enum Borrowed {
        /// The union the payloads are laid over.
        payload = BorrowedPayload;
        /// The form the payloads are read through.
        view = BorrowedView;
        /// Carries nothing.
        Empty = 0,
        /// Carries borrowed text.
        Text(StableStr<'static>) = 1,
    }
}

#[test]
fn an_enumeration_whose_payload_cannot_be_wiped_is_still_a_boundary_type() {
    let value = Borrowed::Text((StableStr::from_str("borrowed"),));
    assert_eq!(value.tag(), 1);
    match value.view() {
        BorrowedView::Text(text) => assert_eq!(text.0.as_str(), "borrowed"),
        BorrowedView::Empty(()) => panic!("the payload names the other variant"),
    }
    <Borrowed as AbiType>::validate(&value).unwrap();
    // It crosses the boundary like any other declared type.
    assert_ne!(<Borrowed as xt_stable_abi::AbiIdentity>::IDENTIFIER, 0);
}

#[test]
fn a_variant_a_build_does_not_have_leaves_no_trace() {
    let gated = Gated::Kept();
    assert_eq!(gated.tag(), 0);
    assert!(matches!(gated.view(), GatedView::Kept(())));
    <Gated as AbiType>::validate(&gated).unwrap();
}

#[test]
fn a_payload_may_hold_more_fields_than_a_tuple_implements_by_hand() {
    let wide = Wide::Many((1, 2, 3, 4, 5, 6, 7, 8, 9));
    match wide.view() {
        WideView::Many(values) => assert_eq!(values.8, 9),
    }
    <Wide as AbiType>::validate(&wide).unwrap();
}

#[test]
fn a_guarded_call_that_panics_answers_with_the_named_variant() {
    xt_stable_abi::stable_object_guarded! {
        /// A handle whose method answers with the enumeration.
        pub struct AnswerObject;
        /// The table behind [`AnswerObject`].
        pub struct AnswerVTable;
        /// The declaration.
        pub trait Answerer {
            /// Fails.
            fn ask(&self) -> Answer;
        }
    }

    struct Grumpy;

    impl Answerer for Grumpy {
        fn ask(&self) -> Answer {
            panic!("no answer");
        }
    }

    let object = AnswerObject::abi_from_value(Grumpy);
    let error = unsafe { object.ask() }.unwrap_err();
    assert_eq!(error.status(), AbiStatus::Panic);
    // The fallback the declaration named is a value the type defines.
    let fallback = <Answer as xt_stable_abi::PanicFallback>::panic_value();
    assert_eq!(fallback.tag(), 0);
}

stable_enum! {
    /// An enumeration whose values have to cross without carrying what was on
    /// the stack around them.
    pub enum Filled {
        /// The union the payloads are laid over.
        payload = FilledPayload;
        /// The form the payloads are read through.
        view = FilledView;

        /// One byte.
        Small(u8) = 0,
        /// Eight bytes, which is what sizes the union.
        Wide(u64) = 1,
    }
}

#[test]
fn a_value_carries_no_bytes_outside_the_payload_its_tag_names() {
    // The struct is `#[repr(C)]`, so the union starts at the first multiple of
    // its own alignment past the tag. Everything from there to the end of the
    // widest variant is the union, and a value of the shorter variant occupies
    // one byte of it.
    let union_at = core::mem::size_of::<u32>().next_multiple_of(core::mem::align_of::<u64>());
    let union_size = core::mem::size_of::<u64>();

    let value = Filled::Small((0xAB,));
    assert_eq!(value.tag(), 0);
    // SAFETY: the union starts `union_at` bytes into the value, and the bytes of
    // it are the bytes this build wrote when it built the value.
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&value as *const Filled).cast::<u8>().add(union_at),
            union_size,
        )
    };
    assert_eq!(bytes[0], 0xAB);
    // The bytes no variant occupies are zeros rather than leftovers: the caller
    // hands the whole value over, not only its active field.
    assert!(bytes[1..].iter().all(|byte| *byte == 0));
}

stable_enum! {
    /// An enumeration whose values can be copied, compared, ordered and hashed.
    pub enum Suited {
        /// The union the payloads are laid over.
        payload = SuitedPayload;
        /// The form the payloads are read through.
        view = SuitedView;
        derives = [Clone, PartialEq, Eq, PartialOrd, Ord, Hash];
        default = Empty;

        /// Carries nothing, and is the value the type starts as.
        Empty = 0,
        /// One number.
        Count(u32) = 1,
        /// One string and one number.
        Named(StableString, u32) = 2,
    }
}

#[test]
fn comparisons_agree_about_a_value_whose_tag_names_no_variant() {
    use core::cmp::Ordering;
    use core::mem::ManuallyDrop;

    let mut first = ManuallyDrop::new(Suited::Empty());
    let mut second = ManuallyDrop::new(Suited::Empty());
    for value in [&mut first, &mut second] {
        let pointer = &mut **value as *mut Suited as *mut u32;
        // SAFETY: the tag is the first field of the declared layout, and the
        // value is in a `ManuallyDrop` so no destructor sees the damaged tag.
        unsafe { pointer.write(99) };
    }

    // `validate` refuses such a value, but both comparisons are still called on
    // it, and they have to answer the same thing: the order is equal for one
    // tag, so equality is too.
    assert_eq!(first.cmp(&second), Ordering::Equal);
    assert!(first == second);
}

#[test]
fn a_declaration_can_ask_for_the_traits_a_value_has() {
    let count = Suited::Count((7,));
    let copy = count.clone();
    assert_eq!(copy, count);
    assert_eq!(copy.tag(), 1);

    // The fallible copy is written beside the panicking one.
    let named = Suited::Named((StableString::from_str("x").unwrap(), 3));
    let clone = <Suited as xt_stable_abi::TryClone>::try_clone(&named).unwrap();
    assert_eq!(clone, named);

    let default = <Suited as Default>::default();
    assert_eq!(default, Suited::Empty());
    assert_eq!(default, default.clone());
}

#[test]
fn a_copy_of_a_payload_owns_its_own_storage() {
    let named = Suited::Named((StableString::from_str("held").unwrap(), 1));
    let copy = named.clone();
    drop(named);
    match copy.view() {
        SuitedView::Named(payload) => assert_eq!(payload.0.as_str(), "held"),
        _ => panic!("the copy names the other variant"),
    }
}

#[test]
fn values_order_by_their_discriminants_and_then_by_their_payloads() {
    assert!(Suited::Empty() < Suited::Count((0,)));
    assert!(Suited::Count((1,)) < Suited::Count((2,)));
    assert!(Suited::Count((u32::MAX,)) < Suited::Named((StableString::from_str("").unwrap(), 0)));
    assert_eq!(
        Suited::Count((2,)).cmp(&Suited::Count((2,))),
        core::cmp::Ordering::Equal
    );
    assert_eq!(Suited::Empty().partial_cmp(&Suited::Empty()), Some(core::cmp::Ordering::Equal));
}

#[test]
fn equal_values_fold_to_one_number() {
    use core::hash::{BuildHasher, Hash};

    let state = std::collections::hash_map::RandomState::new();
    let folded = |value: &Suited| state.hash_one(value);

    let named = Suited::Named((StableString::from_str("a").unwrap(), 2));
    assert_eq!(folded(&named), folded(&named.clone()));
    // The discriminant is folded too, so a payload cannot stand in for another
    // variant's.
    assert_ne!(folded(&Suited::Count((2,))), folded(&Suited::Empty()));
    // The trait is real: a value can be a key of a set that compares by it.
    let mut seen = std::collections::HashSet::new();
    assert!(seen.insert(folded(&named)));
    assert!(!seen.insert(folded(&named.clone())));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    Hash::hash(&named, &mut hasher);
    assert_ne!(core::hash::Hasher::finish(&hasher), 0);
}

stable_enum! {
    /// An enumeration with a payload a total order cannot cover.
    pub enum Measured {
        /// The union the payloads are laid over.
        payload = MeasuredPayload;
        /// The form the payloads are read through.
        view = MeasuredView;
        derives = [PartialEq, PartialOrd];

        /// A measurement.
        Reading(f64) = 0,
        /// Nothing recorded.
        Missing = 1,
    }
}

#[test]
fn a_partial_order_answers_what_a_total_one_cannot() {
    assert!(Measured::Reading((1.0,)) < Measured::Reading((2.0,)));
    assert!(Measured::Reading((f64::INFINITY,)) < Measured::Missing());
    // NaN is not comparable with anything, so the answer is that there is none.
    assert_eq!(
        Measured::Reading((f64::NAN,)).partial_cmp(&Measured::Reading((1.0,))),
        None
    );
    assert_ne!(Measured::Reading((f64::NAN,)), Measured::Reading((f64::NAN,)));
}

stable_enum! {
    /// A declaration whose derived traits have to see the same `#[cfg]`s the
    /// constructors do.
    pub enum Narrow {
        /// The union the payloads are laid over.
        payload = NarrowPayload;
        /// The form the payloads are read through.
        view = NarrowView;
        derives = [Clone, PartialEq, Eq, PartialOrd, Ord, Hash];

        /// Always here.
        Kept(u32) = 0,
        /// Dropped by this build.
        #[cfg(any())]
        Dropped(u32) = 1,
    }
}

#[test]
fn a_derived_trait_leaves_out_a_variant_this_build_dropped() {
    let kept = Narrow::Kept((3,));
    assert_eq!(kept.clone(), kept);
    assert_ne!(kept, Narrow::Kept((4,)));
}

stable_enum! {
    /// An enumeration a hash map can be keyed by.
    pub enum Key {
        /// The union the payloads are laid over.
        payload = KeyPayload;
        /// The form the payloads are read through.
        view = KeyView;
        derives = [Clone, PartialEq, Eq, PartialOrd, Ord, Hash, StableHash];

        /// A number.
        Ident(u32) = 0,
        /// A name and a number.
        Named(StableString, u32) = 1,
    }
}

#[test]
fn an_enumeration_can_be_the_key_of_a_hash_map() {
    let mut map: StableHashMap<Key, u32> = StableHashMap::new();
    map.insert_stable(Key::Ident((1,)), 10).unwrap();
    map.insert_stable(Key::Named((StableString::from_str("a").unwrap(), 2)), 20)
        .unwrap();

    assert_eq!(map.get_stable(&Key::Ident((1,))), Some(&10));
    assert_eq!(map.get_stable(&Key::Ident((2,))), None);
    // The receiver can check the numbers on its own, which is the whole reason
    // this number is computed here rather than by the caller.
    map.validate_stable().unwrap();

    // The number covers the discriminant and the payload, so neither can stand
    // in for the other.
    let ident = Key::Ident((7,));
    let named = Key::Named((StableString::from_str("").unwrap(), 7));
    assert_eq!(ident.stable_hash(), ident.clone().stable_hash());
    assert_ne!(ident.stable_hash(), Key::Ident((8,)).stable_hash());
    assert_ne!(ident.stable_hash(), named.stable_hash());
}


