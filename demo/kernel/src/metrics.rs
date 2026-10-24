//! Counting terms, ranking them and measuring the distribution.

use core::cmp::Reverse;
use core::fmt;
use std::collections::hash_map::Entry;
use std::collections::{BinaryHeap, HashMap};

use crate::text::{tokenize, TokenKind};

/// The counts of one text.
///
/// The map is the working form; [`TermStats::top`] produces the ordered form a
/// caller is usually after.
#[derive(Debug, Clone, Default)]
pub struct TermStats {
    counts: HashMap<String, u64>,
    total: u64,
}

impl TermStats {
    /// Counts the words and numbers of `text`, case-folded.
    pub fn collect(text: &str) -> Self {
        let mut stats = Self::default();
        for token in tokenize(text) {
            if token.kind == TokenKind::Punctuation {
                continue;
            }
            stats.total += 1;
            match stats.counts.entry(token.folded()) {
                Entry::Occupied(mut entry) => *entry.get_mut() += 1,
                Entry::Vacant(entry) => {
                    entry.insert(1);
                }
            }
        }
        stats
    }

    /// The number of terms counted, including repeats.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// The number of different terms.
    pub fn distinct(&self) -> usize {
        self.counts.len()
    }

    /// Reports whether nothing was counted.
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// How often `term` was counted, case-folded.
    pub fn count(&self, term: &str) -> u64 {
        self.counts
            .get(&term.to_lowercase())
            .copied()
            .unwrap_or(0)
    }

    /// The `limit` most frequent terms.
    ///
    /// Terms with the same count are ordered by name, so the answer does not
    /// depend on the map's iteration order.
    pub fn top(&self, limit: usize) -> Vec<(String, u64)> {
        let mut ranking: TopK<(u64, Reverse<&str>)> = TopK::new(limit);
        for (term, count) in &self.counts {
            ranking.push((*count, Reverse(term.as_str())));
        }
        ranking
            .into_sorted()
            .into_iter()
            .map(|(count, Reverse(term))| (term.to_owned(), count))
            .collect()
    }

    /// The Shannon entropy of the term distribution, in thousandths of a bit.
    ///
    /// The integer form keeps the answer from depending on how a platform
    /// formats a float.
    pub fn entropy_milli(&self) -> u32 {
        if self.total == 0 {
            return 0;
        }
        let total = self.total as f64;
        let mut entropy = 0.0f64;
        for count in self.counts.values() {
            let probability = *count as f64 / total;
            entropy -= probability * probability.log2();
        }
        (entropy * 1000.0).round() as u32
    }
}

impl fmt::Display for TermStats {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "terms={} distinct={}",
            self.total,
            self.counts.len()
        )
    }
}

/// A bounded ranking that keeps the largest items it was shown.
///
/// The heap holds at most `limit` items, so the memory a ranking needs does not
/// grow with the input.
#[derive(Debug)]
pub struct TopK<T: Ord> {
    limit: usize,
    items: BinaryHeap<Reverse<T>>,
}

impl<T: Ord> TopK<T> {
    /// A ranking that keeps at most `limit` items.
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            items: BinaryHeap::new(),
        }
    }

    /// Offers one item to the ranking.
    pub fn push(&mut self, item: T) {
        if self.limit == 0 {
            return;
        }
        self.items.push(Reverse(item));
        if self.items.len() > self.limit {
            self.items.pop();
        }
    }

    /// The number of items kept so far.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Reports whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The kept items, largest first.
    pub fn into_sorted(self) -> Vec<T> {
        self.items
            .into_sorted_vec()
            .into_iter()
            .map(|Reverse(item)| item)
            .collect()
    }
}

/// Something that scores a text from its counted terms.
///
/// The implementations are used through `dyn Analyzer`, so a caller can extend
/// the set without the engine knowing the concrete types.
pub trait Analyzer: Send + Sync {
    /// The name the analyzer reports.
    fn name(&self) -> &'static str;
    /// A score; larger means more of whatever the analyzer measures.
    fn score(&self, stats: &TermStats) -> i64;
}

/// Scores a text by how many terms it holds.
#[derive(Debug, Default)]
pub struct TermWeight;

impl Analyzer for TermWeight {
    fn name(&self) -> &'static str {
        "term-weight"
    }

    fn score(&self, stats: &TermStats) -> i64 {
        stats.total().min(i64::MAX as u64) as i64
    }
}

/// Scores a text by how varied its vocabulary is.
#[derive(Debug, Default)]
pub struct DistinctRatio;

impl Analyzer for DistinctRatio {
    fn name(&self) -> &'static str {
        "distinct-ratio"
    }

    fn score(&self, stats: &TermStats) -> i64 {
        if stats.total() == 0 {
            return 0;
        }
        (stats.distinct() as i64).saturating_mul(1000) / stats.total() as i64
    }
}

/// Every analyzer this build offers.
pub fn analyzers() -> Vec<Box<dyn Analyzer>> {
    vec![Box::new(TermWeight), Box::new(DistinctRatio)]
}

macro_rules! extreme {
    ($(#[$meta:meta])* $name:ident => $pick:ident) => {
        $(#[$meta])*
        pub fn $name(values: &[u64]) -> u64 {
            values.iter().copied().reduce(u64::$pick).unwrap_or(0)
        }
    };
}

extreme!(
    /// The largest value, or zero for an empty slice.
    maximum => max
);
extreme!(
    /// The smallest value, or zero for an empty slice.
    minimum => min
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counting_folds_case_and_skips_punctuation() {
        let stats = TermStats::collect("One one ONE, two! 3");
        assert_eq!(stats.total(), 5);
        assert_eq!(stats.count("one"), 3);
        assert_eq!(stats.count("two"), 1);
        assert_eq!(stats.count("3"), 1);
        assert_eq!(stats.distinct(), 3);
    }

    #[test]
    fn the_ranking_is_bounded_and_deterministic() {
        let stats = TermStats::collect("b a b c a b");
        assert_eq!(
            stats.top(2),
            vec![("b".to_owned(), 3), ("a".to_owned(), 2)]
        );
        // Equal counts fall back to the name, not to iteration order.
        let tied = TermStats::collect("x y z");
        assert_eq!(
            tied.top(2),
            vec![("x".to_owned(), 1), ("y".to_owned(), 1)]
        );
        assert!(stats.top(0).is_empty());
    }

    #[test]
    fn a_heap_keeps_the_largest_items() {
        let mut top: TopK<u64> = TopK::new(3);
        for value in [5, 1, 9, 3, 7] {
            top.push(value);
        }
        assert_eq!(top.len(), 3);
        assert_eq!(top.into_sorted(), vec![9, 7, 5]);
    }

    #[test]
    fn entropy_grows_with_variety() {
        let repeated = TermStats::collect("a a a a");
        let varied = TermStats::collect("a b c d");
        assert_eq!(repeated.entropy_milli(), 0);
        assert_eq!(varied.entropy_milli(), 2000);
        assert!(TermStats::default().entropy_milli() == 0);
    }

    #[test]
    fn analyzers_answer_without_knowing_their_type() {
        let stats = TermStats::collect("a b b");
        let scores: Vec<(String, i64)> = analyzers()
            .iter()
            .map(|analyzer| (analyzer.name().to_owned(), analyzer.score(&stats)))
            .collect();
        assert_eq!(
            scores,
            vec![
                ("term-weight".to_owned(), 3),
                ("distinct-ratio".to_owned(), 666)
            ]
        );
    }

    #[test]
    fn the_extremes_are_reported() {
        assert_eq!(maximum(&[3, 9, 1]), 9);
        assert_eq!(minimum(&[3, 9, 1]), 1);
        assert_eq!(maximum(&[]), 0);
    }
}
