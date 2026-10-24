//! Fixed-shape groups: pairs, triples and arrays.

use std::mem::size_of;

use xt_stable_abi::{
    StableArray, StableBytes, StablePair, StableString, StableTriple, StableVec,
};

#[test]
fn a_pair_exposes_its_two_values_in_order() {
    let pair = StablePair::new(1u32, 2u64);
    assert_eq!(size_of::<StablePair<u32, u64>>(), 16);
    assert_eq!(pair.as_ref(), (&1, &2));
    assert_eq!(pair.into_tuple(), (1, 2));
    assert_eq!(StablePair::from((3u32, 4u64)), StablePair::new(3, 4));
}

#[test]
fn a_triple_exposes_its_three_values_in_order() {
    let triple = StableTriple::new(1u8, 2u16, 3u32);
    assert_eq!(size_of::<StableTriple<u8, u16, u32>>(), 8);
    assert_eq!(triple.as_ref(), (&1, &2, &3));
    assert_eq!(triple.into_tuple(), (1, 2, 3));
}

#[test]
fn pairs_order_by_their_first_field_then_the_second() {
    let mut pairs = [
        StablePair::new(2u8, 1u8),
        StablePair::new(1, 9),
        StablePair::new(1, 2),
    ];
    pairs.sort();
    assert_eq!(
        pairs.iter().map(|pair| pair.into_tuple()).collect::<Vec<_>>(),
        vec![(1, 2), (1, 9), (2, 1)]
    );
}

#[test]
fn a_vector_of_pairs_holds_owned_values() {
    let mut pairs: StableVec<StablePair<u32, StableString>> = StableVec::new();
    pairs.push(StablePair::new(1, StableString::from_str("one").unwrap()));
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs.get(0).map(|pair| pair.right.as_str()), Some("one"));
}

#[test]
fn an_array_has_no_header() {
    let array = StableArray::new([1u16, 2, 3, 4]);
    assert_eq!(size_of::<StableArray<u16, 4>>(), 8);
    assert_eq!(array.len(), 4);
    assert_eq!(array.as_slice(), &[1, 2, 3, 4]);
    assert_eq!(array.into_array(), [1, 2, 3, 4]);
}

#[test]
fn an_array_of_bytes_carries_fixed_blobs() {
    let blob: StableBytes<4> = StableBytes::from([0xDEu8, 0xAD, 0xBE, 0xEF]);
    assert_eq!(blob.as_slice(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(blob.len(), 4);
    assert_eq!(blob.iter().count(), 4);
}

#[test]
fn an_array_behaves_like_a_slice() {
    let mut array = StableArray::new([3u8, 1, 2]);
    array.sort();
    assert_eq!(array.as_slice(), &[1, 2, 3]);
    assert_eq!(array.get(1), Some(&2));
    assert_eq!(array.get(3), None);
}

#[test]
fn an_array_of_owned_values_releases_them() {
    let array = StableArray::new([
        StableString::from_str("a").unwrap(),
        StableString::from_str("b").unwrap(),
    ]);
    assert_eq!(array.as_slice()[1].as_str(), "b");
    drop(array);
}
