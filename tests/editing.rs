//! Editing operations on text, sequences and the tag types.

use core::ffi::c_void;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};

use xt_stable_abi::{
    AbiStatus, AbiAllocator, StableOption, StableResult, StableString, StableVec, Wipe,
    SYSTEM_ALLOCATOR,
};

fn hash_of(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Release calls that reached the checking allocator.
static RELEASES: AtomicUsize = AtomicUsize::new(0);
/// Release calls whose block was entirely zero.
static ZEROED_RELEASES: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn checking_allocate(_context: *mut c_void, size: u64, align: u64) -> *mut u8 {
    SYSTEM_ALLOCATOR.allocate(size, align)
}

unsafe extern "C" fn checking_deallocate(
    _context: *mut c_void,
    pointer: *mut u8,
    size: u64,
    align: u64,
) {
    // SAFETY: the caller hands back a block of `size` bytes that this
    // allocator produced.
    let bytes = unsafe { core::slice::from_raw_parts(pointer, size as usize) };
    RELEASES.fetch_add(1, Ordering::SeqCst);
    if bytes.iter().all(|byte| *byte == 0) {
        ZEROED_RELEASES.fetch_add(1, Ordering::SeqCst);
    }
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.deallocate(pointer, size, align) };
}

unsafe extern "C" fn checking_reallocate(
    _context: *mut c_void,
    pointer: *mut u8,
    old_size: u64,
    align: u64,
    new_size: u64,
) -> *mut u8 {
    // SAFETY: the block came from the process allocator with this layout.
    unsafe { SYSTEM_ALLOCATOR.reallocate(pointer, old_size, align, new_size) }
}

fn checking_allocator() -> AbiAllocator {
    // SAFETY: the three functions accept any context, and this one is unused.
    unsafe {
        AbiAllocator::from_parts(
            core::ptr::null_mut(),
            checking_allocate,
            checking_deallocate,
            checking_reallocate,
        )
    }
}

#[test]
fn text_can_be_inserted_and_removed_by_character() {
    let mut text = StableString::from_str("ac").unwrap();
    text.insert(1, 'b').unwrap();
    assert_eq!(text.as_str(), "abc");
    text.insert(3, 'é').unwrap();
    assert_eq!(text.as_str(), "abcé");
    assert_eq!(text.remove(3).unwrap(), 'é');
    assert_eq!(text.as_str(), "abc");
    // Byte 1 is a boundary, byte 0 is too; index 4 is past the end.
    assert_eq!(text.remove(0).unwrap(), 'a');
    assert_eq!(text.remove(4).unwrap_err().status(), AbiStatus::InvalidArgument);
}

#[test]
fn insertion_refuses_to_split_a_character() {
    let mut text = StableString::from_str("aé").unwrap();
    // Byte 2 falls inside the two-byte character.
    assert_eq!(text.insert(2, 'x').unwrap_err().status(), AbiStatus::InvalidArgument);
    assert_eq!(text.as_str(), "aé");
}

#[test]
fn the_last_character_can_be_popped() {
    let mut text = StableString::from_str("ab🦀").unwrap();
    assert_eq!(text.pop(), Some('🦀'));
    assert_eq!(text.as_str(), "ab");
    assert_eq!(text.pop(), Some('b'));
    assert_eq!(text.pop(), Some('a'));
    assert_eq!(text.pop(), None);
}

#[test]
fn characters_and_searches_are_visible() {
    let text = StableString::from_str("héllo").unwrap();
    assert_eq!(text.chars().count(), 5);
    // The two-byte character occupies bytes 1..3, so "llo" starts at byte 3.
    assert_eq!(text.find("llo"), Some(3));
    assert_eq!(text.find("zzz"), None);
    assert_eq!(&*text, "héllo");
}

#[test]
fn text_can_be_mutated_as_a_str() {
    let mut text = StableString::from_str("abc").unwrap();
    let upper = text.as_mut_str().to_ascii_uppercase();
    text.as_mut_str().make_ascii_uppercase();
    assert_eq!(text.as_str(), upper);
    assert_eq!(text.as_str(), "ABC");
}

#[test]
fn text_compares_and_hashes_by_content() {
    let left = StableString::from_str("same").unwrap();
    let right = StableString::from_str("same").unwrap();
    assert_eq!(left, right);
    assert_eq!(hash_of(&left), hash_of(&right));
    assert_eq!(left.cmp(&StableString::from_str("other").unwrap()), std::cmp::Ordering::Greater);

    let borrowed = left.as_stable_str();
    assert_eq!(borrowed, right.as_stable_str());
    assert_eq!(hash_of(&borrowed), hash_of(&right));
    assert!(borrowed > StableString::from_str("aaaa").unwrap().as_stable_str());
}

#[test]
fn wiping_a_string_overwrites_its_bytes_before_releasing_them() {
    RELEASES.store(0, Ordering::SeqCst);
    ZEROED_RELEASES.store(0, Ordering::SeqCst);
    let text = StableString::from_str_with_allocator(checking_allocator(), "secret").unwrap();
    assert_eq!(text.len(), 6);
    assert_eq!(text.capacity(), 6);

    // Wiping consumes the value, so the block reaches the allocator during the
    // call; the allocator is what observes its contents at that moment.
    text.wipe().unwrap();

    assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
    assert_eq!(ZEROED_RELEASES.load(Ordering::SeqCst), 1);
}

#[test]
fn wiping_a_string_overwrites_the_bytes_past_its_end() {
    RELEASES.store(0, Ordering::SeqCst);
    ZEROED_RELEASES.store(0, Ordering::SeqCst);
    let mut text =
        StableString::from_str_with_allocator(checking_allocator(), "secret-token").unwrap();
    // Shortening the text leaves the later bytes in the block; they are outside
    // the value but still part of the memory that is handed back.
    text.truncate(6).unwrap();
    assert_eq!(text.as_str(), "secret");

    text.wipe().unwrap();

    assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
    assert_eq!(ZEROED_RELEASES.load(Ordering::SeqCst), 1);
}

#[test]
fn a_vector_can_keep_only_matching_values() {
    let mut values = StableVec::from_slice(&[1u32, 2, 3, 4, 5, 6]).unwrap();
    values.retain(|value| value % 2 == 0);
    assert_eq!(values.as_slice(), &[2, 4, 6]);
    values.retain(|_| false);
    assert!(values.is_empty());
}

#[test]
fn a_vector_can_keep_only_matching_values_while_editing_them() {
    let mut values = StableVec::from_slice(&[1u32, 2, 3, 4, 5, 6]).unwrap();
    values.retain_mut(|value| {
        *value *= 10;
        *value % 20 == 0
    });
    // Every element the predicate keeps is the one it edited, in order.
    assert_eq!(values.as_slice(), &[20, 40, 60]);
    values.retain_mut(|_| false);
    assert!(values.is_empty());
}

#[test]
fn wiping_a_vector_in_place_also_overwrites_the_slots_it_dropped() {
    RELEASES.store(0, Ordering::SeqCst);
    ZEROED_RELEASES.store(0, Ordering::SeqCst);
    let mut values = StableVec::<u64>::with_capacity_and_allocator(checking_allocator(), 4).unwrap();
    for value in 0..4u64 {
        values.try_push(value).unwrap();
    }
    // Three values leave the length but stay in the block; a wipe that covers
    // the initialised region only leaves them readable.
    values.truncate(1);
    values.wipe_in_place();
    drop(values);

    assert_eq!(RELEASES.load(Ordering::SeqCst), 1);
    assert_eq!(ZEROED_RELEASES.load(Ordering::SeqCst), 1);
}

#[test]
fn wiping_a_boxed_value_releases_what_the_value_owned() {
    use xt_stable_abi::StableBox;

    RELEASES.store(0, Ordering::SeqCst);
    ZEROED_RELEASES.store(0, Ordering::SeqCst);
    let inner = StableString::from_str_with_allocator(checking_allocator(), "secret").unwrap();
    let boxed = StableBox::try_new_with_allocator(inner, checking_allocator()).unwrap();
    boxed.wipe().unwrap();

    // Two blocks were allocated: the text and the box. Both are released, and
    // the text's bytes are zero when they are. A wipe that zeroed the box
    // instead of the value would leave the text's block allocated and readable,
    // so the count of releases is what separates the two behaviours.
    assert_eq!(RELEASES.load(Ordering::SeqCst), 2);
    // The box's own block holds the string's handle, which is not content and is
    // not overwritten; the text's block is the one that is zeroed.
    assert_eq!(ZEROED_RELEASES.load(Ordering::SeqCst), 1);
}

#[test]
fn retain_releases_the_elements_it_drops() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Noisy;
    impl Drop for Noisy {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    xt_stable_abi::unsafe_stable_element!(Noisy);

    let mut values = StableVec::new();
    values.push(Noisy);
    values.push(Noisy);
    values.push(Noisy);
    values.retain(|_| false);
    assert_eq!(DROPS.load(Ordering::SeqCst), 3);
    assert!(values.is_empty());
}

#[test]
fn a_range_can_be_removed_and_taken_out() {
    let mut values = StableVec::from_slice(&[0u32, 1, 2, 3, 4, 5]).unwrap();
    let removed = values.remove_range(2, 4).unwrap();
    assert_eq!(removed.as_slice(), &[2, 3]);
    assert_eq!(values.as_slice(), &[0, 1, 4, 5]);

    assert_eq!(values.remove_range(4, 2).unwrap_err().status(), AbiStatus::InvalidArgument);
    assert_eq!(values.remove_range(0, 9).unwrap_err().status(), AbiStatus::InvalidArgument);
    assert_eq!(values.as_slice(), &[0, 1, 4, 5]);
}

#[test]
fn sorting_and_binary_search_agree() {
    let mut values = StableVec::from_slice(&[5u32, 1, 4, 2, 3]).unwrap();
    values.sort();
    assert_eq!(values.as_slice(), &[1, 2, 3, 4, 5]);
    assert_eq!(values.binary_search(&3), Ok(2));
    assert_eq!(values.binary_search(&6), Err(5));

    let mut words = StableVec::from_slice(&[
        StableString::from_str("bbb").unwrap(),
        StableString::from_str("aaa").unwrap(),
    ])
    .unwrap();
    words.sort();
    assert_eq!(words.get(0).map(StableString::as_str), Some("aaa"));
    words.sort_by(|left, right| right.cmp(left));
    assert_eq!(words.get(0).map(StableString::as_str), Some("bbb"));
}

#[test]
fn a_vector_accepts_an_iterator() {
    let mut values: StableVec<u32> = StableVec::new();
    values.try_extend(0..4u32).unwrap();
    assert_eq!(values.as_slice(), &[0, 1, 2, 3]);
}

#[test]
fn an_owned_iterator_hands_out_and_releases_the_values() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Counted(u32);
    impl Drop for Counted {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    xt_stable_abi::unsafe_stable_element!(Counted);

    let mut values = StableVec::new();
    for index in 0..4u32 {
        values.push(Counted(index));
    }
    let mut seen = Vec::new();
    for value in values {
        seen.push(value.0);
        drop(value);
    }
    assert_eq!(seen, vec![0, 1, 2, 3]);
    assert_eq!(DROPS.load(Ordering::SeqCst), 4);
}

#[test]
fn an_abandoned_iterator_releases_the_rest() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Counted;
    impl Drop for Counted {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    xt_stable_abi::unsafe_stable_element!(Counted);

    let mut values = StableVec::new();
    for _ in 0..3 {
        values.push(Counted);
    }
    let mut iterator = values.into_iter();
    iterator.next();
    drop(iterator);
    assert_eq!(DROPS.load(Ordering::SeqCst), 3);
}

#[test]
fn a_vector_can_be_wiped_before_release() {
    let mut values: StableVec<u8> = StableVec::new();
    values.try_extend(b"secret".iter().copied()).unwrap();
    assert_eq!(values.as_slice(), b"secret");
    values.wipe().unwrap();
}

#[test]
fn option_combinators_chain_without_extra_allocation() {
    let doubled = StableOption::some(3u32).map(|value| value * 2);
    assert_eq!(doubled.unwrap(), 6);
    assert!(StableOption::<u32>::none().and_then(StableOption::some).is_none());
    assert_eq!(StableOption::<u32>::none().or_else(|| StableOption::some(9)).unwrap(), 9);
    assert_eq!(StableOption::some(4u32).filter(|value| value % 2 == 0).unwrap(), 4);
    assert!(StableOption::some(3u32).filter(|value| value % 2 == 0).is_none());
    assert_eq!(
        StableOption::some(1u8).zip(StableOption::some(2u16)).unwrap(),
        (1, 2)
    );
    assert!(StableOption::some(1u8).zip(StableOption::<u16>::none()).is_none());
    assert_eq!(StableOption::some(5u8).expect("present"), 5);
}

#[test]
fn result_combinators_chain() {
    assert_eq!(StableResult::<u32, &str>::ok(2).and_then(|value| StableResult::ok(value + 1)).unwrap(), 3);
    assert_eq!(
        StableResult::<u32, &str>::err("bad")
            .or_else(|error| StableResult::<u32, usize>::ok(error.len() as u32))
            .unwrap(),
        3
    );
    assert_eq!(StableResult::<u32, &str>::err("bad").unwrap_or(7), 7);
    assert_eq!(StableResult::<u32, &str>::err("bad").unwrap_or_else(|error| error.len() as u32), 3);
    assert_eq!(StableResult::<u32, &str>::ok(4).expect("present"), 4);
    assert!(StableResult::<u32, &str>::err("bad").ok_value().is_none());
}

#[test]
fn an_empty_optional_value_panics_when_unwrapped() {
    let outcome = std::panic::catch_unwind(|| StableOption::<u8>::none().unwrap());
    assert!(outcome.is_err());
    let outcome = std::panic::catch_unwind(|| StableResult::<u8, u8>::err(1).unwrap());
    assert!(outcome.is_err());
}

#[cfg(not(miri))]
#[test]
fn rejecting_every_element_costs_one_pass() {
    use std::time::{Duration, Instant};

    const COUNT: usize = 20_000;

    fn filled() -> StableVec<u64> {
        let mut values = StableVec::new();
        for value in 0..COUNT as u64 {
            values.push(value);
        }
        values
    }

    // The same outcome by both entries: nothing is kept. They run the same
    // algorithm, so the two durations are of the same order; a pass that shifts
    // the tail once per rejected element is quadratic in this length and takes
    // far longer.
    let mut values = filled();
    let started = Instant::now();
    values.retain(|_| false);
    let by_reference = started.elapsed();

    let mut values = filled();
    let started = Instant::now();
    values.retain_mut(|_| false);
    let by_mutation = started.elapsed();

    assert!(values.is_empty());
    assert!(
        by_mutation < by_reference * 10 + Duration::from_millis(2),
        "retain took {by_reference:?}, retain_mut took {by_mutation:?}"
    );
}

#[test]
fn sorting_takes_its_temporary_block_from_the_allocator_the_value_carries() {
    RELEASES.store(0, Ordering::SeqCst);
    ZEROED_RELEASES.store(0, Ordering::SeqCst);

    let mut values =
        StableVec::<u64>::with_capacity_and_allocator(checking_allocator(), 64).unwrap();
    for value in (0..64u64).rev() {
        values.try_push(value).unwrap();
    }
    let before = RELEASES.load(Ordering::SeqCst);
    values.sort();

    assert_eq!(values.as_slice()[0], 0);
    assert_eq!(values.as_slice()[63], 63);
    // The temporary block a stable merge needs is released during the call, and
    // the only allocator that can have released it is the one the vector carries:
    // a sort that allocated on the process heap would leave this count unchanged.
    assert!(
        RELEASES.load(Ordering::SeqCst) > before,
        "sort() did not allocate through the allocator the vector carries"
    );
}

#[test]
fn sorting_is_stable() {
    use xt_stable_abi::StablePair;

    let mut values = StableVec::from_slice(&[
        StablePair::new(1u32, 10u32),
        StablePair::new(0, 20),
        StablePair::new(1, 30),
        StablePair::new(0, 40),
    ])
    .unwrap();
    values.sort_by(|left, right| left.left.cmp(&right.left));
    // Equal keys keep the order they had, which is what makes a second sort key
    // meaningful.
    let keys: Vec<(u32, u32)> = values
        .as_slice()
        .iter()
        .map(|pair| (pair.left, pair.right))
        .collect();
    assert_eq!(keys, [(0, 20), (0, 40), (1, 10), (1, 30)]);
}

#[test]
fn a_panicking_comparison_leaves_every_element_in_the_vector() {
    use std::sync::atomic::AtomicUsize;

    static DROPS: AtomicUsize = AtomicUsize::new(0);

    struct Noisy(u32);
    impl Drop for Noisy {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    // SAFETY: `Noisy` is a `u32` with a drop that only counts, which is the same
    // layout and the same ownership rule.
    unsafe impl xt_stable_abi::StableElement for Noisy {
        unsafe fn drop_in_place(pointer: *mut Self) {
            // SAFETY: the caller guarantees an initialised value.
            unsafe { core::ptr::drop_in_place(pointer) }
        }
    }

    DROPS.store(0, Ordering::SeqCst);
    let mut values = StableVec::new();
    for value in 0..8u32 {
        values.push(Noisy(value));
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        values.try_sort_by(|left, right| {
            if left.0 == 5 || right.0 == 5 {
                panic!("comparison gave up");
            }
            left.0.cmp(&right.0)
        })
    }));
    assert!(outcome.is_err());
    // Nothing was released by the failed sort and the length still covers every
    // element, so dropping the vector releases exactly the eight values it was
    // given.
    assert_eq!(values.len(), 8);
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    drop(values);
    assert_eq!(DROPS.load(Ordering::SeqCst), 8);
}

#[test]
fn values_written_into_spare_capacity_become_part_of_the_vector() {
    let mut values =
        StableVec::<u64>::with_capacity_and_allocator(checking_allocator(), 4).unwrap();
    {
        let spare = values.spare_capacity_mut();
        assert_eq!(spare.len(), 4);
        spare[0].write(7);
        spare[1].write(9);
    }
    assert!(values.is_empty());
    // SAFETY: the two slots below the new length were written above.
    unsafe { values.set_len(2) };
    assert_eq!(values.as_slice(), &[7, 9]);
    // The vector now owns both values, so dropping it releases them once.
    values.try_push(11).unwrap();
    assert_eq!(values.as_slice(), &[7, 9, 11]);
}

#[test]
fn wiping_an_absent_option_overwrites_what_the_value_left_behind() {
    use xt_stable_abi::StableOption;

    // The payload of a `u8` follows the `u32` tag with no padding of its own, so
    // the byte this test reads is the byte the value occupied.
    fn byte_at(option: &StableOption<u8>) -> u8 {
        let payload = core::mem::size_of::<u32>();
        // SAFETY: the payload starts one tag-width into the value, and that byte
        // is the one the payload occupies.
        unsafe { *((option as *const StableOption<u8>).cast::<u8>().add(payload)) }
    }

    let mut option = StableOption::some(0xABu8);
    assert_eq!(byte_at(&option), 0xAB);
    assert_eq!(option.take(), Some(0xAB));
    // The value is gone but its byte is still there, which is the case the wipe
    // is for.
    assert_eq!(byte_at(&option), 0xAB);

    option.wipe_in_place();
    assert_eq!(byte_at(&option), 0);
}

#[test]
fn wiping_a_hash_entry_overwrites_the_number_it_was_stored_under() {
    use xt_stable_abi::{StableHashEntry, StableString};

    let mut entry = StableHashEntry::new(
        0x1234_5678_9ABC_DEF0,
        StableString::from_str("key").unwrap(),
        StableString::from_str("value").unwrap(),
    );
    entry.wipe_in_place();
    // The number is a function of the key, so it goes with it: leaving it would
    // keep a fingerprint of the content that was just erased. The values keep
    // their length, so each is still a value this build may read and release;
    // what they carried is gone.
    assert_eq!(entry.hash, 0);
    assert_eq!(entry.key.as_bytes(), &[0u8; 3]);
    assert_eq!(entry.value.as_bytes(), &[0u8; 5]);
}
