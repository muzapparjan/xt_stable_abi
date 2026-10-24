//! The workload the three comparison programs share.
//!
//! Each program - one that calls the kernel directly, one that calls it through
//! a statically linked boundary, and one that loads the boundary at run time -
//! runs the same scenarios over the same inputs and compares the answers with
//! the expectations here. Nothing in this crate can reach the kernel or the
//! boundary, so a program cannot accidentally compare against a different
//! workload than its neighbours.
//!
//! The expectations were taken from a run of the direct program: it is the
//! reference, and the other two have to reproduce it.

#![deny(missing_docs)]

use std::fmt;
use std::time::{Duration, Instant};

/// The text whose analysis every program reports.
pub const TEXT: &str = "one two two";

/// The summary the kernel produces for [`TEXT`].
pub const SUMMARY: &str = "terms=3 distinct=2 bytes=11 entropy=0.918 balanced=true";

/// A text whose brackets do not close.
pub const UNBALANCED_TEXT: &str = "open ( bracket";

/// The text that is tokenised.
pub const TOKENS_TEXT: &str = "Hello, world 42!";

/// The tokens of [`TOKENS_TEXT`], in order.
pub const TOKENS: &[&str] = &["Hello", ",", "world", "42", "!"];

/// The text whose ranking is computed.
pub const RANKING_TEXT: &str = "b a b c a b";

/// How many terms the ranking keeps.
pub const RANKING_LIMIT: u32 = 2;

/// The ranking of [`RANKING_TEXT`].
pub const RANKING: &[(&str, u64)] = &[("b", 3), ("a", 2)];

/// The text normalisation starts from.
pub const NORMALIZE_INPUT: &str = "  Hello   WORLD ";

/// The normalised form of [`NORMALIZE_INPUT`].
pub const NORMALIZE_OUTPUT: &str = "hello world";

/// The text whose analyzer scores are reported.
pub const SCORES_TEXT: &str = "a b";

/// The analyzer scores of [`SCORES_TEXT`].
pub const SCORES: &str = "distinct-ratio=1000 term-weight=2";

/// The text the byte walker reads.
pub const SCAN_TEXT: &str = "call(a, [b])";

/// What the walker reports for [`SCAN_TEXT`].
pub const SCAN: &str = "depth=0 punctuation=4 quoted=false unbalanced=false";

/// A text that leaves a quote open.
pub const SCAN_OPEN_TEXT: &str = "\"open";

/// What the walker reports for [`SCAN_OPEN_TEXT`].
///
/// The walker counts every byte it sees inside a quoted run, not just
/// brackets, so the four letters of `open` are counted as well.
pub const SCAN_OPEN: &str = "depth=0 punctuation=4 quoted=true unbalanced=true";

/// The bytes whose checksums are reported.
pub const DIGEST_INPUT: &[u8] = b"123456789";

/// Both checksums of [`DIGEST_INPUT`], as `fnv:crc`.
pub const DIGEST: &str = "06d5573923c6cdfc:cbf43926";

/// Both checksums of no bytes, as `fnv:crc`.
pub const DIGEST_EMPTY: &str = "cbf29ce484222325:00000000";

/// The bytes a bulk checksum runs over.
pub fn bulk_bytes() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(4096);
    for index in 0..4096u32 {
        bytes.push((index.wrapping_mul(31) % 251) as u8);
    }
    bytes
}

/// The edges the graph scenarios use.
pub const EDGES: &[(u64, u64)] = &[(1, 2), (2, 3), (1, 4)];

/// The node the walk starts from.
pub const START: u64 = 1;

/// Every node reachable from [`START`], in visit order.
pub const REACHABLE: &[u64] = &[1, 2, 4, 3];

/// The order every edge of [`EDGES`] points forward in.
pub const ORDER: &[u64] = &[1, 2, 3, 4];

/// A graph with a cycle, which has no order.
pub const CYCLE: &[(u64, u64)] = &[(1, 2), (2, 3), (3, 1)];

/// Numbers and the bands they fall into.
pub const BANDS: &[(i64, &str)] = &[(-5, "cold"), (10, "warm"), (150, "hot"), (1000, "critical")];

/// A fixed-point value, in millionths.
pub const FIXED: i64 = 1_500_000;

/// The whole and fractional parts of [`FIXED`].
pub const FIXED_PARTS: (i64, i64) = (1, 500_000);

/// How many facts the build reports about itself.
pub const CAPABILITIES: usize = 5;

/// The texts a session accumulates, in order.
pub const SESSION_TEXTS: &[&str] = &["one two", "three"];

/// The total the session reaches after [`SESSION_TEXTS`].
pub const SESSION_TOTAL: u64 = 3;

/// What the session says about itself once it holds [`SESSION_TEXTS`].
pub const SESSION_SUMMARY: &str = "session=first texts=2 notes=0 terms=3 distinct=3 bytes=14 entropy=1.585 balanced=true";

/// What the session says once [`MERGE_TEXT`] has been absorbed.
pub const SESSION_CLOSED: &str = "closed session=first texts=3 words=6";

/// What the session says once [`MERGE_TEXT`]'s total has been absorbed.
///
/// A caller that holds the peer only through a published handle can read the
/// peer's name and total, not its texts: the word count is the same as
/// [`SESSION_CLOSED`], the text count is not.
pub const SESSION_CLOSED_ABSORBED: &str = "closed session=first texts=2 words=6";

/// The text the second session holds.
pub const MERGE_TEXT: &str = "four five six";

/// The total after the second session is merged into the first.
pub const MERGED_TOTAL: u64 = 6;

/// How many rounds each scenario is timed over.
pub mod rounds {
    /// Rounds for a cached analysis.
    pub const ANALYZE: u32 = 2_000;
    /// Rounds for tokenising and ranking.
    pub const TOKENS: u32 = 1_000;
    /// Rounds for a checksum over [`super::bulk_bytes`].
    pub const DIGEST: u32 = 2_000;
    /// Rounds for a graph walk and an order.
    pub const GRAPH: u32 = 2_000;
    /// Rounds for the scalar scenarios.
    pub const SCALARS: u32 = 20_000;
    /// Rounds for a session round trip, which allocates the most.
    pub const SESSION: u32 = 200;
}

/// The outcome of one timed batch.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// How many times the body ran.
    pub rounds: u32,
    /// How long the batch took.
    pub elapsed: Duration,
}

impl Timing {
    /// The time one round took, in nanoseconds.
    pub fn per_call_nanos(&self) -> f64 {
        self.elapsed.as_secs_f64() * 1_000_000_000.0 / f64::from(self.rounds)
    }
}

impl fmt::Display for Timing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} rounds in {:.3} ms, {:.1} ns each",
            self.rounds,
            self.elapsed.as_secs_f64() * 1_000.0,
            self.per_call_nanos()
        )
    }
}

/// Runs `body` `rounds` times and reports how long that took.
///
/// One untimed batch runs first so the first round does not pay for a cold
/// cache, a lazy table or a first allocation on the library's side; the timed
/// batch is what a program reports.
pub fn measure(rounds: u32, mut body: impl FnMut()) -> Timing {
    for _ in 0..rounds.min(100) {
        body();
    }
    let start = Instant::now();
    for _ in 0..rounds {
        body();
    }
    Timing {
        rounds,
        elapsed: start.elapsed(),
    }
}

/// Collects what a program found.
#[derive(Debug)]
pub struct Report {
    checks: usize,
    failures: usize,
    timings: usize,
}

impl Default for Report {
    fn default() -> Self {
        Self::new()
    }
}

impl Report {
    /// An empty report.
    pub fn new() -> Self {
        Self {
            checks: 0,
            failures: 0,
            timings: 0,
        }
    }

    /// Records a check that should hold.
    pub fn that(&mut self, label: &str, condition: bool) {
        self.checks += 1;
        if condition {
            println!("ok   {label}");
        } else {
            self.failures += 1;
            println!("FAIL {label}");
        }
    }

    /// Records a check against an expected value.
    pub fn is<T: PartialEq + fmt::Debug>(&mut self, label: &str, found: T, expected: T) {
        self.checks += 1;
        if found == expected {
            println!("ok   {label}");
        } else {
            self.failures += 1;
            println!("FAIL {label}: found {found:?}, expected {expected:?}");
        }
    }

    /// Records a check on text, comparing the characters rather than the
    /// containers that hold them.
    pub fn text(&mut self, label: &str, found: &str, expected: &str) {
        self.checks += 1;
        if found == expected {
            println!("ok   {label}");
        } else {
            self.failures += 1;
            println!(
                "FAIL {label}: found {found:?}, expected {expected:?}"
            );
        }
    }

    /// Records a timing, which is not a check.
    pub fn perf(&mut self, label: &str, timing: Timing) {
        self.timings += 1;
        println!("perf {label}: {timing}");
    }

    /// Prints a line outside the check list.
    pub fn note(&self, label: &str, detail: impl fmt::Display) {
        println!("info {label}: {detail}");
    }

    /// The number of checks recorded so far.
    pub fn checks(&self) -> usize {
        self.checks
    }

    /// The number of checks that failed.
    pub fn failures(&self) -> usize {
        self.failures
    }

    /// The number of timings recorded.
    pub fn timings(&self) -> usize {
        self.timings
    }

    /// Prints the totals and reports whether every check held.
    pub fn finish(&self) -> bool {
        println!(
            "\n{} checks, {} failed, {} timings",
            self.checks, self.failures, self.timings
        );
        self.failures == 0
    }
}
