//! A graph of values built by one side and released by the other.
//!
//! The two modules below only share the boundary types. The producer owns an
//! allocator, the consumer never learns which one it was, and the blocks still
//! come back to the producer's allocator when the consumer drops the graph.

mod support;

use std::sync::atomic::Ordering;

use support::{tracked_allocator, Tracker};
use xt_stable_abi::{AbiAllocator, StableString, StableVec};

mod producer {
    use xt_stable_abi::{AbiAllocator, StableOption, StableString, StableVec};

    /// Builds a vector of optional strings using `allocator` for every block.
    pub fn build(allocator: AbiAllocator) -> StableVec<StableOption<StableString>> {
        let mut values = StableVec::new_with_allocator(allocator);
        for index in 0..8u32 {
            let text =
                StableString::from_str_with_allocator(allocator, &format!("item-{index}")).unwrap();
            values.push(StableOption::some(text));
        }
        values.push(StableOption::none());
        values
    }
}

mod consumer {
    use xt_stable_abi::{StableOption, StableString, StableVec};

    /// Reads every present entry without taking ownership.
    pub fn labels(values: &StableVec<StableOption<StableString>>) -> Vec<String> {
        values
            .iter()
            .filter_map(|value| value.as_ref())
            .map(|text| text.as_str().to_owned())
            .collect()
    }

    /// Takes ownership, summarises, and releases the graph.
    pub fn summarize(values: StableVec<StableString>) -> (usize, usize) {
        let count = values.len() as usize;
        let bytes: usize = values.iter().map(|text| text.len() as usize).sum();
        (count, bytes)
    }
}

#[test]
fn a_graph_built_on_one_side_is_released_on_the_other() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);

    {
        let values = producer::build(allocator);
        assert_eq!(values.len(), 9);
        assert_eq!(tracker.outstanding(), 9);

        let labels = consumer::labels(&values);
        assert_eq!(labels.len(), 8);
        assert_eq!(labels[0], "item-0");
        assert_eq!(labels[7], "item-7");
        assert_eq!(tracker.outstanding(), 9);
    }

    assert_eq!(tracker.outstanding(), 0);
    assert_eq!(tracker.deallocations.load(Ordering::SeqCst), 9);
}

#[test]
fn a_consumer_that_takes_ownership_releases_everything() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);

    let mut texts = StableVec::new_with_allocator(allocator);
    for index in 0..4u32 {
        texts.push(StableString::from_str_with_allocator(allocator, &index.to_string()).unwrap());
    }
    assert_eq!(tracker.outstanding(), 5);

    let (count, bytes) = consumer::summarize(texts);
    assert_eq!(count, 4);
    assert_eq!(bytes, 4);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_borrowed_slice_crosses_without_transferring_ownership() {
    let tracker = Tracker::new();
    let allocator = tracked_allocator(tracker);
    let values = producer::build(allocator);

    let borrowed = values.as_stable_slice();
    assert_eq!(borrowed.len(), 9);
    assert_eq!(borrowed.get(2).and_then(|value| value.as_ref()).map(|text| text.as_str()), Some("item-2"));

    // The borrow does not extend the owner's life; dropping the vector here is
    // what releases every block.
    drop(values);
    assert_eq!(tracker.outstanding(), 0);
}

#[test]
fn a_value_built_with_the_process_allocator_needs_no_bookkeeping() {
    let values = producer::build(AbiAllocator::system());
    assert_eq!(consumer::labels(&values).len(), 8);
    let texts = StableVec::from_slice(&[
        StableString::from_str("one").unwrap(),
        StableString::from_str("two").unwrap(),
    ])
    .unwrap();
    assert_eq!(consumer::summarize(texts), (2, 6));
}
