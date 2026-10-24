//! Checks on values that arrived from another build.
//!
//! Every case here builds a value whose bytes say something the type does not
//! define, and asserts that the boundary refuses it before safe code reads it.
//! Values that are refused are still dropped, so the checks that a refused value
//! is released without touching the bytes it does not describe are part of the
//! same tests.
//!
//! This file is left out of the leak-checked Miri run: several tests assert that
//! a refused value releases *nothing*, and a table a generated constructor
//! cached lives for the rest of the process by design. The full run uses
//! `-Zmiri-ignore-leaks`, the same as the other targets that keep counters in
//! `'static` values.

use core::sync::atomic::{AtomicUsize, Ordering};

use xt_stable_abi::{
    boundary, stable_object, AbiAllocator, AbiError, AbiStatus, AbiType, StableCowStr, StableDyn,
    StableMap, StableResult, StableSet, StableStr, StableString, StableVec, VTablePrefix,
    SYSTEM_ALLOCATOR,
};

/// Deallocations seen by [`counting_allocator`].
static RELEASES: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn counting_allocate(
    _context: *mut core::ffi::c_void,
    size: u64,
    align: u64,
) -> *mut u8 {
    // The process allocator is the one the counting allocator forwards to.
    SYSTEM_ALLOCATOR.allocate(size, align)
}

unsafe extern "C" fn counting_deallocate(
    _context: *mut core::ffi::c_void,
    pointer: *mut u8,
    size: u64,
    align: u64,
) {
    RELEASES.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.deallocate(pointer, size, align) }
}

unsafe extern "C" fn counting_reallocate(
    _context: *mut core::ffi::c_void,
    pointer: *mut u8,
    old_size: u64,
    align: u64,
    new_size: u64,
) -> *mut u8 {
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.reallocate(pointer, old_size, align, new_size) }
}

fn counting_allocator() -> AbiAllocator {
    // SAFETY: the three functions forward to the process allocator and count
    // calls; they accept a null context.
    unsafe {
        AbiAllocator::from_parts(
            core::ptr::null_mut(),
            counting_allocate,
            counting_deallocate,
            counting_reallocate,
        )
    }
}

#[test]
fn a_result_with_a_tag_the_type_does_not_define_is_refused_and_released_untouched() {
    RELEASES.store(0, Ordering::SeqCst);
    let text = StableString::from_str_with_allocator(counting_allocator(), "secret").unwrap();
    let mut value = StableResult::<StableString, StableString>::ok(text);
    assert!(value.validate().is_ok());

    // SAFETY: the tag is the first field of the value, and the payload stays a
    // valid `StableString` whatever the tag says.
    unsafe { *(&mut value as *mut StableResult<StableString, StableString> as *mut u32) = 9 };

    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::InvalidTag
    );

    let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        boundary::require(&value);
    }));
    assert!(refused.is_err(), "the accessor must stop at the tag");

    // Dropping the value must not release either payload: the tag says nothing
    // about which one is initialised, so the bytes are left to whoever produced
    // them. Releasing the wrong one would free this block.
    drop(value);
    assert_eq!(RELEASES.load(Ordering::SeqCst), 0);
}

#[test]
fn a_copy_on_write_value_with_a_tag_the_type_does_not_define_is_refused() {
    let mut value = StableCowStr::owned(StableString::from_str("text").unwrap());
    // SAFETY: the tag is the first field of the value, and the payload is
    // untouched.
    unsafe { *(&mut value as *mut StableCowStr<'_> as *mut u32) = 2 };

    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::InvalidTag
    );
    drop(value);
}

#[test]
fn a_copy_on_write_value_releases_only_the_field_its_tag_names() {
    RELEASES.store(0, Ordering::SeqCst);
    let text = StableString::from_str_with_allocator(counting_allocator(), "secret").unwrap();
    let mut value = StableCowStr::owned(text);
    // The tag now says the value is borrowed, so the owned block is storage the
    // tag says to ignore. Releasing it anyway would free a block the value is
    // not supposed to own any more.
    // SAFETY: the tag is the first field of the value.
    unsafe { *(&mut value as *mut StableCowStr<'_> as *mut u32) = 0 };

    assert!(value.validate().is_ok());
    drop(value);
    assert_eq!(RELEASES.load(Ordering::SeqCst), 0);
}

#[test]
fn text_that_is_not_utf8_is_refused_at_the_boundary() {
    let mut bytes: StableVec<u8> = StableVec::new();
    bytes.try_extend_from_slice(&[0xff, 0xfe]).unwrap();
    // SAFETY: the caller promises the bytes are UTF-8, and the test is about a
    // producer that broke that promise.
    let text = unsafe { StableString::from_utf8_unchecked(bytes) };
    assert_eq!(text.validate().unwrap_err().status(), AbiStatus::InvalidUtf8);

    let handle = StableStr::from_str("upheld");
    let forged = unsafe {
        StableStr::from_raw_parts(ADDRESS_OF_INVALID_UTF8.as_ptr(), 2)
    };
    assert!(handle.validate_text().is_ok());
    assert_eq!(
        forged.validate_text().unwrap_err().status(),
        AbiStatus::InvalidUtf8
    );

    let checked = unsafe {
        StableStr::from_raw_parts_checked(ADDRESS_OF_INVALID_UTF8.as_ptr(), 2, 2)
    };
    assert_eq!(checked.unwrap_err().status(), AbiStatus::InvalidUtf8);
}

/// Bytes that are not UTF-8, kept in a static so their address is stable.
static ADDRESS_OF_INVALID_UTF8: [u8; 2] = [0xff, 0xfe];

/// The header of a [`StableVec`], spelled out so a test can build one whose
/// numbers disagree with its block.
#[repr(C)]
struct VecHeaderMirror {
    pointer: *mut u8,
    length: u64,
    capacity: u64,
}

#[test]
fn a_vector_header_that_disagrees_with_its_block_is_refused() {
    let allocator = counting_allocator();
    let mut block: StableVec<u8> = StableVec::with_capacity_and_allocator(allocator, 4).unwrap();
    block.try_extend_from_slice(b"abcd").unwrap();
    let pointer = block.storage_ptr();

    // A length past the capacity describes slots that were never initialised.
    let refused = unsafe { StableVec::<u8>::from_raw_parts(pointer, 8, 1, allocator) };
    assert_eq!(
        refused.unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );

    // Forging the same disagreement in place: the header must be refused, and
    // dropping the value must leave the block alone.
    // SAFETY: the header mirror has the documented layout of the vector's first
    // three fields.
    unsafe {
        let header = &mut block as *mut StableVec<u8> as *mut VecHeaderMirror;
        assert_eq!((*header).length, 4);
        assert_eq!((*header).capacity, 4);
        (*header).length = 8;
    }
    assert_eq!(
        block.validate().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
    RELEASES.store(0, Ordering::SeqCst);
    drop(block);
    assert_eq!(RELEASES.load(Ordering::SeqCst), 0);
}

#[test]
fn a_vector_of_strings_with_a_forged_length_is_released_untouched() {
    let allocator = counting_allocator();
    let mut block: StableVec<StableString> =
        StableVec::with_capacity_and_allocator(allocator, 4).unwrap();
    block
        .try_push(StableString::from_str_with_allocator(allocator, "one").unwrap())
        .unwrap();
    let releases_before = RELEASES.load(Ordering::SeqCst);

    // A length past the capacity is the disagreement a header can show; a
    // length inside the capacity is a claim only the producer can settle, so the
    // number is set past the block here.
    // SAFETY: the header mirror has the documented layout of the vector's first
    // two fields.
    unsafe {
        let header = &mut block as *mut StableVec<StableString> as *mut VecHeaderMirror;
        (*header).length = 5;
    }
    assert_eq!(
        block.validate().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
    drop(block);
    // The string's block and the vector's block were both left alone, so the
    // allocator saw no release at all.
    assert_eq!(RELEASES.load(Ordering::SeqCst), releases_before);
}

#[test]
fn a_name_with_a_forged_length_is_refused_before_its_bytes_are_read() {
    use xt_stable_abi::{StableOsStr, StableOsString, StablePathBuf};

    let mut name = StableOsString::from_text("ab").unwrap();
    // A length past the capacity describes bytes the block does not hold. The
    // check has to compare the two numbers before a slice is built over them:
    // building it first would be the out-of-bounds read this test is about.
    // SAFETY: the header mirror has the documented layout of the vector's first
    // three fields, which is where the name's storage begins.
    unsafe {
        let header = &mut name as *mut StableOsString as *mut VecHeaderMirror;
        assert_eq!((*header).length, 2);
        (*header).length = (*header).capacity + 1;
    }
    assert_eq!(
        name.validate().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );

    // The same forgery under a path, which the boundary reaches through its own
    // type rather than through the name's.
    let mut path = StablePathBuf::from_text("ab").unwrap();
    // SAFETY: a path is its stored name, so the header begins at the same place.
    unsafe {
        let header = &mut path as *mut StablePathBuf as *mut VecHeaderMirror;
        assert_eq!((*header).length, 2);
        (*header).length = (*header).capacity + 1;
    }
    assert_eq!(
        <StablePathBuf as xt_stable_abi::AbiType>::validate(&path)
            .unwrap_err()
            .status(),
        AbiStatus::InvalidDescriptor
    );

    // The borrowed form has no capacity to compare against; the owned value is
    // the shape that can answer this question.
    let borrowed = StableOsStr::from_str("ab");
    assert!(borrowed.validate().is_ok());
}

#[test]
fn an_empty_handle_with_an_address_is_refused() {
    let value = unsafe {
        StableDyn::from_raw_parts(
            core::ptr::NonNull::<u8>::dangling().as_ptr(),
            core::ptr::null(),
            AbiAllocator::system(),
        )
    };
    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
    drop(value);
}

#[test]
fn a_handle_with_a_table_but_no_address_is_refused() {
    unsafe extern "C" fn drop_nothing(_data: *mut u8) {}

    static TABLE: VTablePrefix = VTablePrefix {
        type_id: 1,
        signature: 2,
        layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
        size: 8,
        align: 8,
        slots: 0,
        drop: Some(drop_nothing),
    };

    let value = unsafe {
        StableDyn::from_raw_parts(
            core::ptr::null_mut(),
            &TABLE,
            AbiAllocator::system(),
        )
    };
    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::NullPointer
    );
    drop(value);
}

#[test]
fn a_table_whose_size_cannot_be_rounded_to_its_alignment_is_refused() {
    unsafe extern "C" fn drop_nothing(_data: *mut u8) {}

    // A block is described by a layout, and a layout rounds its size up to the
    // alignment. This size is addressable on its own, yet the layout a release
    // would be handed cannot name it.
    static TABLE: VTablePrefix = VTablePrefix {
        type_id: 1,
        signature: 2,
        layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
        size: (isize::MAX as u64) - 3,
        align: 8,
        slots: 0,
        drop: Some(drop_nothing),
    };

    static VALUE: [u64; 1] = [0];

    let value = unsafe {
        StableDyn::from_raw_parts(VALUE.as_ptr() as *mut u8, &TABLE, AbiAllocator::system())
    };
    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::InvalidLayout
    );
    // The same test decides whether the value may be released, so neither the
    // destructor nor the release runs.
    assert!(!value.is_releasable());
    drop(value);
}

#[test]
fn the_slot_scan_stops_at_the_number_the_header_records() {
    use xt_stable_abi::object::VTableSlot;

    unsafe extern "C" fn drop_nothing(_data: *mut u8) {}

    unsafe extern "C" fn no_op() {}

    /// A table that holds one recorded slot and one entry the header does not
    /// count, which is what a table shorter than it claims would look like.
    #[repr(C)]
    struct CountedTable {
        prefix: VTablePrefix,
        recorded: VTableSlot,
        extra: VTableSlot,
    }

    static TABLE: CountedTable = CountedTable {
        prefix: VTablePrefix {
            type_id: 1,
            signature: 2,
            layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
            size: 8,
            align: 8,
            slots: 1,
            drop: Some(drop_nothing),
        },
        recorded: VTableSlot {
            signature: 10,
            call: no_op as unsafe extern "C" fn() as *const (),
        },
        extra: VTableSlot {
            signature: 20,
            call: no_op as unsafe extern "C" fn() as *const (),
        },
    };

    // SAFETY: the table is immutable for the length of the test, and the
    // entries hold no address the test calls through.
    unsafe impl Sync for CountedTable {}

    // The address is a real block this value owns, so dropping it at the end of
    // the test releases exactly what was allocated, whatever the assertions
    // decided.
    let data = Box::into_raw(Box::new(0u64));
    let value = unsafe {
        StableDyn::from_raw_parts(
            data.cast::<u8>(),
            // The pointer names the whole table, which is what a receiver is
            // given: the entries sit after the header inside the same value.
            &TABLE as *const CountedTable as *const VTablePrefix,
            AbiAllocator::system(),
        )
    };
    assert_eq!(value.slots(), 1);
    assert!(value.slot(10).is_some());
    // The second entry is inside the table but outside the recorded count, so it
    // is not part of the method list.
    assert!(value.slot(20).is_none());
    // SAFETY: the block came from `Box` with the layout the table records.
    drop(value);
}

#[test]
fn an_entry_with_no_address_is_not_a_method() {
    use xt_stable_abi::object::VTableSlot;

    unsafe extern "C" fn drop_nothing(_data: *mut u8) {}

    /// A table that names a signature but records no address for it.
    #[repr(C)]
    struct EmptySlotTable {
        prefix: VTablePrefix,
        entry: VTableSlot,
    }

    static TABLE: EmptySlotTable = EmptySlotTable {
        prefix: VTablePrefix {
            type_id: 1,
            signature: 2,
            layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
            size: 8,
            align: 8,
            slots: 1,
            drop: Some(drop_nothing),
        },
        entry: VTableSlot {
            signature: 10,
            call: core::ptr::null(),
        },
    };

    // SAFETY: the table is immutable for the length of the test.
    unsafe impl Sync for EmptySlotTable {}

    let data = Box::into_raw(Box::new(0u64));
    let value = unsafe {
        StableDyn::from_raw_parts(
            data.cast::<u8>(),
            &TABLE as *const EmptySlotTable as *const VTablePrefix,
            AbiAllocator::system(),
        )
    };
    assert_eq!(value.slots(), 1);
    // An entry that names a signature but carries no address has nothing to
    // call, so it answers as absent rather than as a null function pointer.
    assert!(value.slot(10).is_none());
    drop(value);
}

stable_object! {
    /// A handle that hands out another handle.
    pub struct ParentObject;
    /// The table behind [`ParentObject`].
    pub struct ParentVTable;
    /// Produces a child handle.
    pub trait Parent {
        /// Returns a handle the caller can adopt.
        fn child(&self) -> ChildObject;
    }
}

stable_object! {
    /// A handle to a number.
    pub struct ChildObject;
    /// The table behind [`ChildObject`].
    pub struct ChildVTable;
    /// Answers with a number.
    pub trait Child {
        /// The number this value carries.
        fn number(&self) -> u64;
    }
}

struct Seven;

impl Child for Seven {
    fn number(&self) -> u64 {
        7
    }
}

struct Maker;

impl Parent for Maker {
    fn child(&self) -> ChildObject {
        ChildObject::abi_from_value(Seven)
    }
}

#[test]
fn a_generated_handle_can_be_a_return_value() {
    let parent = ParentObject::abi_from_value(Maker);
    // SAFETY: the object was built by this trait's generated constructor.
    let child = unsafe { parent.child() };
    assert!(!child.abi_is_empty());
    // SAFETY: the handle was adopted through the generated return path.
    assert_eq!(unsafe { child.number() }, 7);
}

#[test]
fn a_method_removed_by_a_configuration_is_not_part_of_the_signature() {
    stable_object! {
        /// A handle whose table is built from a configured method list.
        pub struct ConfiguredObject;
        /// The table behind [`ConfiguredObject`].
        pub struct ConfiguredVTable;
        /// Two declared methods, one of which no build compiles.
        pub trait Configured {
            /// A method every build contains.
            fn kept(&self) -> u64;
            /// A method no configuration contains.
            #[cfg(any())]
            fn removed(&self) -> u64;
        }
    }

    struct Only;
    impl Configured for Only {
        fn kept(&self) -> u64 {
            1
        }
    }

    assert_eq!(ConfiguredObject::ABI_SLOT_SIGNATURES.len(), 1);
    assert_eq!(
        ConfiguredObject::ABI_SIGNATURE,
        xt_stable_abi::object::combined_signature(ConfiguredObject::ABI_SLOT_SIGNATURES)
    );
    let object = ConfiguredObject::abi_from_value(Only);
    assert_eq!(object.abi_signature(), ConfiguredObject::ABI_SIGNATURE);
}

#[test]
fn the_status_list_and_the_lookup_agree() {
    assert_eq!(AbiStatus::ALL.len(), 12);
    for status in AbiStatus::ALL {
        assert_eq!(AbiStatus::from_code(status.code()), Some(status));
        assert!(!status.message().is_empty());
    }
    // A code this build has no name for is still a value of the type; what
    // `from_code` answers is whether this build defines it.
    assert_eq!(AbiStatus::from_code(12), None);
    assert_eq!(AbiStatus::from_code(u32::MAX), None);
    assert_eq!(AbiStatus::from_raw(12).code(), 12);
    assert_eq!(AbiStatus::from_raw(u32::MAX).code(), u32::MAX);
}

#[test]
fn two_file_names_that_differ_in_an_unpaired_surrogate_are_not_equal() {
    let first = xt_stable_abi::path::StableOsStr::from_bytes(
        &[0xed, 0xa0, 0x80],
        xt_stable_abi::path::ENCODING_WTF8,
    )
    .unwrap();
    let second = xt_stable_abi::path::StableOsStr::from_bytes(
        &[0xed, 0xa0, 0x81],
        xt_stable_abi::path::ENCODING_WTF8,
    )
    .unwrap();
    assert_ne!(first, second);
    assert!(first.validate().is_ok());
}

#[test]
fn a_table_that_records_no_destructor_is_refused() {
    // A table written by hand may record no destructor at all, and a value
    // nothing can release is not one this build may accept.
    static TABLE: VTablePrefix = VTablePrefix {
        type_id: 1,
        signature: 2,
        layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
        size: 8,
        align: 8,
        slots: 0,
        drop: None,
    };

    let data = Box::into_raw(Box::new(0u64));
    // SAFETY: the block holds one `u64`, which is what the table records.
    let value = unsafe {
        StableDyn::from_raw_parts(data.cast::<u8>(), &TABLE, AbiAllocator::system())
    };
    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
    // The value is not released through the table, so the test releases the
    // block itself rather than leaving it behind.
    assert!(!value.is_releasable());
    core::mem::forget(value);
    // SAFETY: the block came from `Box` with the layout the table records.
    unsafe { drop(Box::from_raw(data)) };
}

#[test]
fn a_table_at_an_address_the_header_cannot_be_read_at_is_refused() {
    unsafe extern "C" fn drop_nothing(_data: *mut u8) {}

    let table = VTablePrefix {
        type_id: 1,
        signature: 2,
        layout_revision: xt_stable_abi::object::LAYOUT_REVISION,
        size: 8,
        align: 8,
        slots: 0,
        drop: Some(drop_nothing),
    };
    // The header is written one byte into a block of whole words, which is an
    // address no header can start at.
    let mut words = vec![0u64; 8];
    let base = words.as_mut_ptr().cast::<u8>();
    // SAFETY: the destination holds the whole header, and the source is one
    // header this test owns.
    unsafe {
        core::ptr::copy_nonoverlapping(
            (&table as *const VTablePrefix).cast::<u8>(),
            base.add(1),
            core::mem::size_of::<VTablePrefix>(),
        );
    }
    let data = Box::into_raw(Box::new(0u64));
    // SAFETY: the block holds one `u64`, and the pointer names the bytes written
    // above.
    let value = unsafe {
        StableDyn::from_raw_parts(
            data.cast::<u8>(),
            base.add(1).cast::<VTablePrefix>(),
            AbiAllocator::system(),
        )
    };
    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::InvalidLayout
    );
    core::mem::forget(value);
    // SAFETY: the block came from `Box` with the layout the table records.
    unsafe { drop(Box::from_raw(data)) };
}

/// The bytes a borrowed name arrives as.
///
/// The handle is a `#[repr(C)]` slice followed by the encoding tag, which is
/// what the mirror states.
#[repr(C)]
struct OsStrMirror {
    pointer: *const u8,
    length: u64,
    encoding: u32,
}

#[test]
fn a_name_whose_length_is_not_addressable_is_not_valid() {
    use xt_stable_abi::path::ENCODING_UNIX;
    use xt_stable_abi::StableOsStr;

    assert_eq!(
        core::mem::size_of::<OsStrMirror>(),
        core::mem::size_of::<StableOsStr<'static>>()
    );

    let mirror = OsStrMirror {
        pointer: core::ptr::NonNull::<u8>::dangling().as_ptr(),
        length: u64::MAX,
        encoding: ENCODING_UNIX,
    };
    // SAFETY: the mirror states the layout the handle declares. The length is
    // one no storage can hold, which is exactly what the check has to refuse
    // before anything reads through the pointer.
    let value: StableOsStr<'static> = unsafe { core::mem::transmute(mirror) };

    assert!(!value.is_valid(), "a length no side can address is refused");
    assert_eq!(
        value.validate().unwrap_err().status(),
        AbiStatus::CapacityOverflow
    );
}

#[test]
fn a_vector_that_reports_no_capacity_but_names_a_block_is_refused() {
    // An aligned address that is not the one an empty vector carries.
    let stray = core::ptr::NonNull::<u32>::dangling()
        .as_ptr()
        .wrapping_add(4);
    // SAFETY: `from_raw_parts` only reads the header it is given; the block is
    // never released because the header is refused.
    let refused = unsafe { StableVec::<u32>::from_raw_parts(stray, 0, 0, AbiAllocator::system()) };
    assert_eq!(
        refused.unwrap_err().status(),
        AbiStatus::InvalidDescriptor
    );
}

#[test]
fn a_vector_that_reports_capacity_but_names_no_block_is_refused() {
    // The dangling address belongs to no block, so a header that claims room for
    // elements over it names storage this side would read and release.
    let no_block = core::ptr::NonNull::<u32>::dangling().as_ptr();
    // SAFETY: `from_raw_parts` only reads the header it is given, and the header
    // is refused before anything reads through the address.
    let refused =
        unsafe { StableVec::<u32>::from_raw_parts(no_block, 1, 1, AbiAllocator::system()) };
    let error = match refused {
        Ok(value) => {
            // Releasing this would call the allocator with an address no block
            // has, so the test lets it go instead of ending the process.
            core::mem::forget(value);
            panic!("a header that names no block is refused");
        }
        Err(error) => error,
    };
    assert_eq!(error.status(), AbiStatus::InvalidDescriptor);

    // A vector of zero-sized elements allocates nothing and carries exactly that
    // address, so its header is still the header of a value this build may hold
    // and release.
    let empty_block = core::ptr::NonNull::<()>::dangling().as_ptr();
    // SAFETY: a zero-sized element needs no block, and the header is the whole
    // value.
    let accepted = unsafe { StableVec::<()>::from_raw_parts(empty_block, 3, 3, AbiAllocator::system()) };
    let accepted = accepted.expect("a zero-sized element needs no block");
    assert_eq!(accepted.len(), 3);

    // What crosses the boundary is refused, though: the length is the whole of
    // what such a container says, nothing backs it, and walking it would run one
    // step per element the producer named.
    let refused = <StableVec<()> as xt_stable_abi::AbiType>::validate(&accepted).unwrap_err();
    assert_eq!(refused.status(), AbiStatus::InvalidDescriptor);
    // The same value with no elements is a value the boundary accepts.
    let empty = unsafe { StableVec::<()>::from_raw_parts(empty_block, 0, 0, AbiAllocator::system()) }
        .expect("an empty vector names no block");
    <StableVec<()> as xt_stable_abi::AbiType>::validate(&empty).unwrap();

    // The ordered collections apply the same rule to the shape they store, which
    // is a pair rather than an element: a pair of two empty types occupies no
    // bytes, so a map of them carries nothing but its length either.
    let pair_block = core::ptr::NonNull::<xt_stable_abi::StablePair<(), ()>>::dangling().as_ptr();
    let map = unsafe { StableMap::<(), ()>::from_raw_parts(pair_block, 1, 1, AbiAllocator::system()) }
        .expect("a zero-sized pair needs no block");
    <StableMap<(), ()> as xt_stable_abi::AbiType>::validate(&map).unwrap_err();
    let set = unsafe { StableSet::<()>::from_raw_parts(empty_block, 1, 1, AbiAllocator::system()) }
        .expect("a zero-sized value needs no block");
    <StableSet<()> as xt_stable_abi::AbiType>::validate(&set).unwrap_err();

    // The same rule covers a boxed value: the handle names nothing when the type
    // occupies bytes and the address belongs to no block.
    // SAFETY: the call only checks the address and the size it is told.
    let refused = unsafe {
        xt_stable_abi::StableBox::<u32>::from_raw_parts_checked(
            core::ptr::NonNull::<u32>::dangling().as_ptr(),
            core::mem::size_of::<u32>() as u64,
            AbiAllocator::system(),
        )
    };
    let error = match refused {
        Ok(value) => {
            core::mem::forget(value);
            panic!("a boxed value that names no block is refused");
        }
        Err(error) => error,
    };
    assert_eq!(error.status(), AbiStatus::NullPointer);
}

#[test]
fn an_error_with_a_code_this_build_has_no_name_for_is_still_a_value() {
    /// The bytes of an `AbiError`, written by a build that knows a code this one
    /// does not.
    #[repr(C)]
    struct RawError {
        status: u32,
        message: StableString,
    }

    let raw = RawError {
        status: 9_999,
        message: StableString::from_str("a failure this build has no name for").unwrap(),
    };
    // SAFETY: `RawError` is declared with the layout of `AbiError`.
    let error: AbiError = unsafe { core::mem::transmute(raw) };

    // The value is one this build may read and release: the message is what a
    // caller displays, and the code is a number it can log or pass on — kept as
    // the library wrote it, because this build has no name to put in its place.
    error.validate().unwrap();
    assert_eq!(error.raw_status(), 9_999);
    assert_eq!(error.status().code(), 9_999);
    assert_eq!(error.message(), "a failure this build has no name for");
    assert_eq!(
        error.clone().status().code(),
        9_999,
        "a copy keeps the number it cannot name"
    );
}
