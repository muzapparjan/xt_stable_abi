//! A program that uses the kernel directly.
//!
//! This is the reference: the kernel is linked into the binary and every call is
//! an ordinary function call. It runs the same scenarios as the two boundary
//! programs, over the same inputs, and compares the answers with the same
//! expectations, so the three outputs can be put side by side.

use std::hint::black_box;
use std::panic::AssertUnwindSafe;
use std::process::ExitCode;
use std::sync::Arc;

use xt_bench as bench;
use xt_bench::Report;
use xt_kernel::{Engine, KernelError};

fn main() -> ExitCode {
    let engine = Arc::new(Engine::new());
    let mut report = Report::new();

    text(&engine, &mut report);
    bytes(&engine, &mut report);
    graphs(&engine, &mut report);
    numbers(&engine, &mut report);
    build(&engine, &mut report);
    sessions(&engine, &mut report);
    timings(&engine, &mut report);

    if report.finish() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn text(engine: &Engine, report: &mut Report) {
    report.text(
        "analyze renders the kernel's summary",
        &engine.analyze(bench::TEXT).render(),
        bench::SUMMARY,
    );
    report.that(
        "an unbalanced text is reported as such",
        engine
            .analyze(bench::UNBALANCED_TEXT)
            .render()
            .ends_with("balanced=false"),
    );

    report.is(
        "tokens keep punctuation and numbers",
        engine.tokens(bench::TOKENS_TEXT),
        bench::TOKENS
            .iter()
            .map(|token| (*token).to_owned())
            .collect::<Vec<String>>(),
    );

    report.is(
        "the ranking is bounded and ordered",
        engine.top_terms(bench::RANKING_TEXT, bench::RANKING_LIMIT as usize),
        bench::RANKING
            .iter()
            .map(|(term, count)| ((*term).to_owned(), *count))
            .collect::<Vec<(String, u64)>>(),
    );
    report.that(
        "a limit of zero ranks nothing",
        engine.top_terms(bench::RANKING_TEXT, 0).is_empty(),
    );

    report.text(
        "normalisation folds case and space",
        &engine.normalize(bench::NORMALIZE_INPUT),
        bench::NORMALIZE_OUTPUT,
    );
    report.text(
        "both analyzers answer",
        &engine.analysis_line(bench::SCORES_TEXT),
        bench::SCORES,
    );
    report.text(
        "the walker reports depth and punctuation",
        &engine.scan_line(bench::SCAN_TEXT),
        bench::SCAN,
    );
    report.text(
        "an open quote is reported",
        &engine.scan_line(bench::SCAN_OPEN_TEXT),
        bench::SCAN_OPEN,
    );
}

fn bytes(engine: &Engine, report: &mut Report) {
    report.text(
        "both checksums are computed over the caller's bytes",
        &engine.digest(bench::DIGEST_INPUT),
        bench::DIGEST,
    );
    report.text(
        "an empty input still has a digest",
        &engine.digest(&[]),
        bench::DIGEST_EMPTY,
    );
}

fn graphs(engine: &Engine, report: &mut Report) {
    report.is(
        "a walk visits every node once",
        engine.reachable(bench::EDGES, bench::START),
        bench::REACHABLE.to_vec(),
    );

    match engine.topological(bench::EDGES) {
        Ok(order) => report.is("an acyclic graph is ordered", order, bench::ORDER.to_vec()),
        Err(error) => {
            report.that("an acyclic graph is ordered", false);
            report.note("the kernel refused a valid graph", error);
        }
    }

    match engine.topological(bench::CYCLE) {
        Err(KernelError::GraphCycle { .. }) => {
            report.that("a cycle is refused, not ordered", true);
        }
        Err(other) => {
            report.that("a cycle is refused, not ordered", false);
            report.note("the kernel refused the graph for another reason", other);
            return;
        }
        Ok(_) => {
            report.that("a cycle is refused, not ordered", false);
            return;
        }
    }
    match engine.topological(bench::CYCLE) {
        // The boundary program reads this sentence out of the message the
        // published error carries, so the reference checks that the kernel's own
        // description names the cause.
        Err(error) => report.that(
            "the kernel's reason names the cycle",
            error.to_string().contains("cycle"),
        ),
        Ok(_) => report.that("the kernel's reason names the cycle", false),
    }
}

fn numbers(engine: &Engine, report: &mut Report) {
    for (value, expected) in bench::BANDS {
        report.is("a number is classified", engine.classify(*value), (*expected).to_owned());
    }
    report.is(
        "a fixed-point value splits",
        engine.split_fixed(bench::FIXED),
        bench::FIXED_PARTS,
    );
    let (whole, fraction) = bench::FIXED_PARTS;
    report.is(
        "the sign is carried by both parts",
        engine.split_fixed(-bench::FIXED),
        (-whole, -fraction),
    );
}

fn build(engine: &Engine, report: &mut Report) {
    let capabilities = engine.capabilities();
    report.is(
        "the build reports every fact it publishes",
        capabilities.len(),
        bench::CAPABILITIES,
    );
    report.that(
        "the build reports its own version",
        capabilities.iter().any(|item| item == "version=0.1.0"),
    );
    report.that(
        "the build reports its pointer width",
        capabilities.iter().any(|item| item.starts_with("pointer_width=")),
    );
    let stats = engine.stats();
    report.that(
        "the counters report every figure",
        ["analyses=", "cache_hits=", "sessions=", "tokens=", "checksums="]
            .iter()
            .all(|field| stats.contains(field)),
    );
    // The interpreter does not offer the system clock on every platform.
    #[cfg(not(miri))]
    {
        let (seconds, nanos) = engine.now().expect("the clock is readable");
        report.that(
            "the clock answers with the current time",
            seconds > 1_600_000_000 && nanos < 1_000_000_000,
        );
    }
}

fn sessions(engine: &Arc<Engine>, report: &mut Report) {
    let mut first = engine.session("first");
    report.is("a session reports the name it was opened with", first.name(), "first");
    for text in bench::SESSION_TEXTS {
        first.add(text);
    }
    report.is("a session counts what it holds", first.total(), bench::SESSION_TOTAL);
    report.text(
        "the summary describes the session",
        &first.summary(),
        bench::SESSION_SUMMARY,
    );
    // The published surface hands out a child object; the kernel has no such
    // concept, so the reference program reads the number that object carries.
    report.is(
        "a session hands out a child that carries its total",
        first.total(),
        bench::SESSION_TOTAL,
    );

    let mut second = engine.session("second");
    second.add(bench::MERGE_TEXT);
    report.is(
        "merging adds the peer's total",
        first.merge(&second),
        bench::MERGED_TOTAL,
    );

    // The boundary program receives a status; here the panic is caught in the
    // same process and nothing crosses anything.
    let exploded = std::panic::catch_unwind(AssertUnwindSafe(|| first.explode()));
    report.that("a panicking call does not take the program down", exploded.is_err());

    report.text(
        "a finished session describes itself",
        &first.finish(),
        bench::SESSION_CLOSED,
    );
}

fn timings(engine: &Arc<Engine>, report: &mut Report) {
    let bulk = bench::bulk_bytes();

    report.perf(
        "analyze (cached)",
        bench::measure(bench::rounds::ANALYZE, || {
            black_box(engine.analyze(bench::TEXT).render().len());
        }),
    );
    report.perf(
        "tokens",
        bench::measure(bench::rounds::TOKENS, || {
            black_box(engine.tokens(bench::TOKENS_TEXT).len());
        }),
    );
    report.perf(
        "top_terms",
        bench::measure(bench::rounds::TOKENS, || {
            black_box(
                engine
                    .top_terms(bench::RANKING_TEXT, bench::RANKING_LIMIT as usize)
                    .len(),
            );
        }),
    );
    report.perf(
        "digest of 4 KiB",
        bench::measure(bench::rounds::DIGEST, || {
            black_box(engine.digest(&bulk).len());
        }),
    );
    report.perf(
        "reachable",
        bench::measure(bench::rounds::GRAPH, || {
            black_box(engine.reachable(bench::EDGES, bench::START).len());
        }),
    );
    report.perf(
        "classify",
        bench::measure(bench::rounds::SCALARS, || {
            black_box(engine.classify(150).len());
        }),
    );
    // Returns two numbers: no allocation, so the difference against `classify`
    // is the cost the boundary pays for building a returned owned value. The
    // argument changes every round: with a loop-invariant one, a call that has
    // no boundary to cross is hoisted out of the loop and measures nothing.
    let mut scalar = 0i64;
    report.perf(
        "split_fixed (no allocation)",
        bench::measure(bench::rounds::SCALARS, || {
            scalar = scalar.wrapping_add(1);
            black_box(engine.split_fixed(black_box(scalar)));
        }),
    );
    report.perf(
        "session round trip",
        bench::measure(bench::rounds::SESSION, || {
            let mut session = engine.session("timed");
            for text in bench::SESSION_TEXTS {
                session.add(text);
            }
            black_box(session.finish().len());
        }),
    );
}
