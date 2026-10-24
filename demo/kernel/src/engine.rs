//! The facade the rest of the kernel is used through.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::codec::{self, Fixed};
use crate::error::KernelResult;
use crate::graph::Graph;
use crate::metrics::{self, TermStats};
use crate::state::{self, Level, ScanReport};
use crate::text;

/// What an analysis found, in a form that is cheap to compare and print.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    /// Words and numbers counted.
    pub terms: u64,
    /// Different terms.
    pub distinct: u64,
    /// Bytes analysed.
    pub bytes: u64,
    /// Shannon entropy of the term distribution, in thousandths of a bit.
    pub entropy_milli: u32,
    /// Whether the brackets and quotes balance.
    pub balanced: bool,
}

impl Summary {
    /// The summary as one line of text.
    pub fn render(&self) -> String {
        format!(
            "terms={} distinct={} bytes={} entropy={}.{:03} balanced={}",
            self.terms,
            self.distinct,
            self.bytes,
            self.entropy_milli / 1000,
            self.entropy_milli % 1000,
            self.balanced
        )
    }
}

/// What the engine has done since it was built.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    /// Analyses that were computed.
    pub analyses: u64,
    /// Analyses answered from the cache.
    pub cache_hits: u64,
    /// Sessions opened.
    pub sessions: u64,
    /// Terms counted by the analyses.
    pub tokens: u64,
    /// Checksums computed.
    pub checksums: u64,
}

/// A text-analysis engine.
///
/// The engine is shared: a session holds it in an [`Arc`], so a session keeps
/// working after the handle that opened it is gone. The cache and the counters
/// sit behind mutexes, which is what lets the same engine serve several callers.
#[derive(Debug)]
pub struct Engine {
    cache: Mutex<HashMap<String, Summary>>,
    counters: Mutex<Counters>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An engine with an empty cache.
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            counters: Mutex::new(Counters::default()),
        }
    }

    /// The counters so far.
    pub fn counters(&self) -> Counters {
        *self.counters.lock().expect("the counters were not poisoned")
    }

    /// Analyses `text`, answering from the cache when it was seen before.
    pub fn analyze(&self, text: &str) -> Summary {
        if let Some(cached) = self
            .cache
            .lock()
            .expect("the cache was not poisoned")
            .get(text)
            .copied()
        {
            self.counters
                .lock()
                .expect("the counters were not poisoned")
                .cache_hits += 1;
            return cached;
        }
        let stats = TermStats::collect(text);
        let summary = Summary {
            terms: stats.total(),
            distinct: stats.distinct() as u64,
            bytes: text.len() as u64,
            entropy_milli: stats.entropy_milli(),
            balanced: !state::scan(text).unbalanced,
        };
        self.cache
            .lock()
            .expect("the cache was not poisoned")
            .insert(text.to_owned(), summary);
        let mut counters = self.counters.lock().expect("the counters were not poisoned");
        counters.analyses += 1;
        counters.tokens += summary.terms;
        summary
    }

    /// The tokens of `text`, as owned strings.
    pub fn tokens(&self, text: &str) -> Vec<String> {
        text::tokenize(text)
            .map(|token| token.text.to_owned())
            .collect()
    }

    /// The most frequent terms of `text`.
    pub fn top_terms(&self, text: &str, limit: usize) -> Vec<(String, u64)> {
        TermStats::collect(text).top(limit)
    }

    /// Every analyzer's score for `text`.
    pub fn analysis_line(&self, text: &str) -> String {
        let stats = TermStats::collect(text);
        let mut scores = BTreeMap::new();
        for analyzer in metrics::analyzers() {
            scores.insert(analyzer.name(), analyzer.score(&stats));
        }
        scores
            .iter()
            .map(|(name, score)| format!("{name}={score}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The FNV-1a hash of `bytes`.
    pub fn checksum(&self, bytes: &[u8]) -> u64 {
        self.counters
            .lock()
            .expect("the counters were not poisoned")
            .checksums += 1;
        codec::fnv1a(bytes)
    }

    /// The CRC-32 checksum of `bytes`.
    pub fn crc(&self, bytes: &[u8]) -> u32 {
        codec::crc32(bytes)
    }

    /// Both checksums, written as `fnv:crc` in hexadecimal.
    pub fn digest(&self, bytes: &[u8]) -> String {
        format!(
            "{:016x}:{:08x}",
            self.checksum(bytes),
            codec::crc32(bytes)
        )
    }

    /// Folds case and whitespace.
    pub fn normalize(&self, text: &str) -> String {
        text::normalize(text)
    }

    /// The nodes reachable from `start`.
    pub fn reachable(&self, edges: &[(u64, u64)], start: u64) -> Vec<u64> {
        Graph::from_edges(edges).reachable(start)
    }

    /// An order in which every edge points forward.
    pub fn topological(&self, edges: &[(u64, u64)]) -> KernelResult<Vec<u64>> {
        Graph::from_edges(edges).topological()
    }

    /// The number of weakly connected pieces.
    pub fn components(&self, edges: &[(u64, u64)]) -> usize {
        Graph::from_edges(edges).components()
    }

    /// The band a number falls into, as its name.
    pub fn classify(&self, value: i64) -> String {
        Level::of(value).name().to_owned()
    }

    /// The whole and fractional parts of a fixed-point number.
    pub fn split_fixed(&self, micros: i64) -> (i64, i64) {
        Fixed::from_micros(micros).split()
    }

    /// Reads a fixed-point number into millionths.
    pub fn parse_fixed(&self, text: &str) -> KernelResult<i64> {
        Ok(Fixed::parse(text)?.micros())
    }

    /// What the byte walker finds.
    pub fn scan(&self, text: &str) -> ScanReport {
        state::scan(text)
    }

    /// What the byte walker finds, as one line of text.
    ///
    /// The form is the one both the native and the published paths report, so a
    /// comparison between them compares the walker and not two spellings of its
    /// result.
    pub fn scan_line(&self, text: &str) -> String {
        let report = self.scan(text);
        format!(
            "depth={} punctuation={} quoted={} unbalanced={}",
            report.depth, report.punctuation, report.quoted, report.unbalanced
        )
    }

    /// A line describing what the engine has done.
    pub fn stats(&self) -> String {
        let counters = self.counters();
        format!(
            "analyses={} cache_hits={} sessions={} tokens={} checksums={}",
            counters.analyses,
            counters.cache_hits,
            counters.sessions,
            counters.tokens,
            counters.checksums
        )
    }

    /// What this build offers, so a host can see which binary it loaded.
    pub fn capabilities(&self) -> Vec<String> {
        vec![
            format!(
                "profile={}",
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                }
            ),
            format!("pointer_width={}", usize::BITS),
            format!(
                "endian={}",
                if cfg!(target_endian = "little") {
                    "little"
                } else {
                    "big"
                }
            ),
            format!("analyzers={}", metrics::analyzers().len()),
            format!("version={}", env!("CARGO_PKG_VERSION")),
        ]
    }

    /// The current time, as seconds and nanoseconds since the Unix epoch.
    pub fn now(&self) -> KernelResult<(i64, u32)> {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(elapsed) => Ok((
                elapsed.as_secs().min(i64::MAX as u64) as i64,
                elapsed.subsec_nanos(),
            )),
            Err(error) => {
                let before = error.duration();
                Ok((
                    -(before.as_secs().min(i64::MAX as u64) as i64),
                    before.subsec_nanos(),
                ))
            }
        }
    }

    /// Opens a session that accumulates texts and reports on all of them.
    pub fn session(self: &Arc<Self>, name: &str) -> Session {
        self.counters
            .lock()
            .expect("the counters were not poisoned")
            .sessions += 1;
        Session {
            engine: Arc::clone(self),
            name: name.to_owned(),
            texts: Vec::new(),
            notes: Vec::new(),
            absorbed: 0,
        }
    }
}

/// A collection of texts that are analysed together.
pub struct Session {
    engine: Arc<Engine>,
    name: String,
    texts: Vec<String>,
    notes: Vec<String>,
    absorbed: u64,
}

impl Session {
    /// The name the session was opened with.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Adds `text` and answers with the running total of terms.
    pub fn add(&mut self, text: &str) -> u64 {
        self.texts.push(text.to_owned());
        self.total()
    }

    /// The number of terms in every text added so far, including every total
    /// absorbed from elsewhere.
    pub fn total(&self) -> u64 {
        self.texts
            .iter()
            .map(|text| self.engine.analyze(text).terms)
            .sum::<u64>()
            + self.absorbed
    }

    /// A line describing the session and everything it holds.
    pub fn summary(&self) -> String {
        let mut combined = String::new();
        for text in &self.texts {
            combined.push_str(text);
            combined.push('\n');
        }
        let summary = self.engine.analyze(&combined);
        format!(
            "session={} texts={} notes={} {}",
            self.name,
            self.texts.len(),
            self.notes.len(),
            summary.render()
        )
    }

    /// Takes every text of `other` into this session.
    ///
    /// The other session keeps its own copy; the answer is the combined total.
    pub fn merge(&mut self, other: &Session) -> u64 {
        self.texts.extend(other.texts.iter().cloned());
        self.notes
            .push(format!("{}:{}", other.name, other.texts.len()));
        self.total()
    }

    /// Absorbs a total produced elsewhere, answering with the new total.
    ///
    /// This is what a caller does when the other side is not a session this
    /// process holds: it knows the name and the total and nothing else. The
    /// total is part of this session from then on, so the answer and every later
    /// reading agree.
    pub fn absorb(&mut self, name: &str, total: u64) -> u64 {
        self.notes.push(format!("{name}:{total}"));
        self.absorbed += total;
        self.total()
    }

    /// Closes the session and describes it.
    pub fn finish(self) -> String {
        format!(
            "closed session={} texts={} words={}",
            self.name,
            self.texts.len(),
            self.total()
        )
    }

    /// A deliberate failure, used to show that a panic stays on this side.
    ///
    /// # Panics
    ///
    /// Always, with the session's name.
    pub fn explode(&self) -> u64 {
        panic!("session {} cannot be measured that way", self.name);
    }
}

impl core::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("name", &self.name)
            .field("texts", &self.texts.len())
            .field("notes", &self.notes.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_second_analysis_of_a_text_comes_from_the_cache() {
        let engine = Engine::new();
        let first = engine.analyze("one two two");
        let second = engine.analyze("one two two");
        assert_eq!(first, second);
        let counters = engine.counters();
        assert_eq!(counters.analyses, 1);
        assert_eq!(counters.cache_hits, 1);
        assert_eq!(counters.tokens, 3);
        assert_eq!(first.terms, 3);
        assert_eq!(first.distinct, 2);
        assert!(first.balanced);
        assert_eq!(
            first.render(),
            "terms=3 distinct=2 bytes=11 entropy=0.918 balanced=true"
        );
    }

    #[test]
    fn a_session_accumulates_and_merges() {
        let engine = Arc::new(Engine::new());
        let mut first = engine.session("first");
        assert_eq!(first.add("one two"), 2);
        assert_eq!(first.add("three"), 3);
        let mut second = engine.session("second");
        assert_eq!(second.add("four five six"), 3);
        assert_eq!(first.merge(&second), 6);
        assert!(first.summary().contains("session=first texts=3 notes=1"));
        assert_eq!(
            first.finish(),
            "closed session=first texts=3 words=6"
        );
        assert_eq!(engine.counters().sessions, 2);
    }

    #[test]
    fn an_absorbed_total_stays_with_the_session() {
        let engine = Arc::new(Engine::new());
        let mut session = engine.session("first");
        session.add("one two");
        assert_eq!(session.absorb("peer", 4), 6);
        // The answer and every later reading have to agree: a total that is
        // returned but never kept would leave the session describing itself
        // with a number that is not what the caller was just told.
        assert_eq!(session.total(), 6);
        assert_eq!(session.finish(), "closed session=first texts=1 words=6");
    }

    #[test]
    fn the_facade_answers_from_the_kernel_pieces() {
        let engine = Engine::new();
        assert_eq!(engine.tokens("a b"), vec!["a", "b"]);
        assert_eq!(engine.top_terms("b a b", 1), vec![("b".to_owned(), 2)]);
        assert_eq!(engine.reachable(&[(1, 2)], 1), vec![1, 2]);
        assert_eq!(engine.topological(&[(1, 2)]).unwrap(), vec![1, 2]);
        assert_eq!(engine.components(&[(1, 2), (3, 4)]), 2);
        assert_eq!(engine.classify(150), "hot");
        assert_eq!(engine.split_fixed(1_500_000), (1, 500_000));
        assert_eq!(engine.parse_fixed("-0.25").unwrap(), -250_000);
        assert!(!engine.scan("(a)").unbalanced);
        assert_eq!(engine.checksum(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(engine.digest(b"123456789"), "06d5573923c6cdfc:cbf43926");
        assert!(engine.stats().starts_with("analyses=0"));
        assert_eq!(engine.capabilities().len(), 5);
        // The interpreter does not offer the system clock on every platform, so
        // the clock is checked where a clock exists.
        #[cfg(not(miri))]
        {
            let (seconds, nanos) = engine.now().unwrap();
            assert!(seconds > 1_600_000_000);
            assert!(nanos < 1_000_000_000);
        }
        assert!(engine.analysis_line("a b").contains("term-weight=2"));
    }
}
