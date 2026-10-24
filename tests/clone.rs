//! A copy that reports a failure instead of panicking.

use xt_stable_abi::{
    StableBox, StableOption, StablePair, StableQuad, StableResult, StableString, StableTriple,
    StableVec, TryClone,
};

fn text(value: &str) -> StableString {
    StableString::from_str(value).unwrap()
}

#[test]
fn a_tagged_value_copies_the_variant_it_holds() {
    let present: StableOption<StableString> = StableOption::some(text("here"));
    let copy = present.try_clone().unwrap();
    assert_eq!(copy.as_ref().map(StableString::as_str), Some("here"));

    let absent: StableOption<StableString> = StableOption::none();
    assert!(absent.try_clone().unwrap().is_none());

    let ok: StableResult<StableVec<u32>, StableString> =
        StableResult::ok(StableVec::from_slice(&[1, 2, 3]).unwrap());
    let copy = ok.try_clone().unwrap();
    assert_eq!(copy.as_ref().unwrap().as_slice(), &[1, 2, 3]);

    let err: StableResult<StableVec<u32>, StableString> = StableResult::err(text("no"));
    let copy = err.try_clone().unwrap();
    assert_eq!(copy.as_ref().unwrap_err().as_str(), "no");
}

#[test]
fn a_container_of_containers_copies_all_the_way_down() {
    let mut outer: StableVec<StableOption<StableString>> = StableVec::new();
    outer.push(StableOption::some(text("one")));
    outer.push(StableOption::none());

    let copy = outer.try_clone().unwrap();
    assert_eq!(copy.len(), 2);
    assert_eq!(copy.as_slice()[0].as_ref().unwrap().as_str(), "one");
    assert!(copy.as_slice()[1].is_none());

    // The copy is a separate value: writing through one leaves the other alone.
    let mut copy = copy;
    copy.as_mut_slice()[0] = StableOption::some(text("changed"));
    assert_eq!(outer.as_slice()[0].as_ref().unwrap().as_str(), "one");
}

#[test]
fn a_group_copies_each_part() {
    let pair = StablePair::new(text("a"), StableVec::from_slice(&[1u8]).unwrap());
    let copy = pair.try_clone().unwrap();
    assert_eq!(copy.left.as_str(), "a");
    assert_eq!(copy.right.as_slice(), &[1]);

    let triple = StableTriple::new(1u32, text("b"), StableOption::some(2u64));
    let copy = triple.try_clone().unwrap();
    assert_eq!(copy.as_ref(), (&1, &text("b"), &StableOption::some(2)));

    let quad = StableQuad::new(1u8, 2u16, 3u32, text("d"));
    let copy = quad.try_clone().unwrap();
    assert_eq!(copy.as_ref().3.as_str(), "d");
}

#[test]
fn a_box_that_names_no_value_copies_to_one_that_names_none() {
    let empty: StableBox<StableString> = StableBox::empty();
    let copy = empty.try_clone().unwrap();
    assert!(copy.is_empty());

    let boxed = StableBox::new(text("inside"));
    let copy = boxed.try_clone().unwrap();
    assert_eq!(copy.as_str(), "inside");
}

#[test]
fn a_copy_of_a_value_with_no_storage_is_the_value() {
    let duration = xt_stable_abi::StableDuration::new(4, 5).unwrap();
    assert_eq!(duration.try_clone().unwrap(), duration);
    let handle = xt_stable_abi::Handle::from_parts(9, 1);
    assert_eq!(handle.try_clone().unwrap(), handle);
    assert_eq!(7u32.try_clone().unwrap(), 7);
}

#[test]
fn an_ordered_container_copies_every_key_and_value() {
    let mut map: xt_stable_abi::StableMap<u32, StableString> = xt_stable_abi::StableMap::new();
    map.insert(1, text("one")).unwrap();
    map.insert(2, text("two")).unwrap();

    let copy = map.try_clone().unwrap();
    assert_eq!(copy.len(), 2);
    assert_eq!(copy.get(&1).map(StableString::as_str), Some("one"));
    assert_eq!(copy.get(&2).map(StableString::as_str), Some("two"));

    let set = xt_stable_abi::StableSet::from_values(&[3u32, 1, 2]).unwrap();
    let copy = set.try_clone().unwrap();
    assert_eq!(copy.as_values(), &[1, 2, 3]);
}

/// Reports whether a copy of [`Flagged`] fails.
static COPY_FAILS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A boundary type that is copied only through [`TryClone`], never through
/// [`Clone`].
#[repr(transparent)]
#[derive(Clone, Copy)]
struct Flagged(u64);

// SAFETY: the wrapper has the layout of a `u64` and owns nothing.
unsafe impl xt_stable_abi::StableElement for Flagged {
    unsafe fn drop_in_place(pointer: *mut Self) {
        // SAFETY: the caller guarantees an initialised value.
        unsafe { core::ptr::drop_in_place(pointer) }
    }
}

impl TryClone for Flagged {
    fn try_clone(&self) -> xt_stable_abi::AbiResult<Self> {
        if COPY_FAILS.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(xt_stable_abi::AbiError::new(
                xt_stable_abi::AbiStatus::OutOfMemory,
            ));
        }
        Ok(*self)
    }
}

#[test]
fn a_vector_copies_through_the_trait_that_reports_a_failure() {
    // The inherent `try_clone` used to require `Clone`, which made it a second
    // path that panicked while the trait reported. It now names the trait, so a
    // type that has no `Clone` is still copyable, and a copy that fails is
    // reported rather than raised.
    let vector = StableVec::from_slice(&[Flagged(1), Flagged(2)]).unwrap();
    let copy = vector.try_clone().unwrap();
    assert_eq!(copy.len(), 2);

    COPY_FAILS.store(true, std::sync::atomic::Ordering::SeqCst);
    let error = vector.try_clone().unwrap_err();
    COPY_FAILS.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(error.status(), xt_stable_abi::AbiStatus::OutOfMemory);
}

#[test]
fn a_hash_map_and_a_hash_set_copy_through_the_trait() {
    let mut map = xt_stable_abi::StableHashMap::<u32, StableString>::new();
    map.insert(|key: &u32| u64::from(*key), 1, text("one"))
        .unwrap();
    let copy = TryClone::try_clone(&map).unwrap();
    assert_eq!(copy.len(), 1);
    assert_eq!(copy.get(|key| u64::from(*key), &1).map(StableString::as_str), Some("one"));

    let mut set = xt_stable_abi::StableHashSet::<u32>::new();
    set.insert(|value: &u32| u64::from(*value), 7).unwrap();
    let copy = TryClone::try_clone(&set).unwrap();
    assert_eq!(copy.len(), 1);
    assert!(copy.contains(|value| u64::from(*value), &7));
}

#[test]
fn the_borrowed_views_copy_by_naming_the_same_storage() {
    use xt_stable_abi::{StableCowStr, StableSlice, StableStr};

    let text: &str = "borrowed";
    let view = StableStr::from_str(text);
    assert_eq!(TryClone::try_clone(&view).unwrap().as_str(), "borrowed");

    let values = [1u8, 2, 3];
    let slice = StableSlice::from_slice(&values);
    assert_eq!(TryClone::try_clone(&slice).unwrap().as_slice(), &[1, 2, 3]);

    let cow = StableCowStr::borrowed(text);
    assert_eq!(TryClone::try_clone(&cow).unwrap().as_str(), "borrowed");
}
