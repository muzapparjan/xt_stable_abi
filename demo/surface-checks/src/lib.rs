//! The scenarios both boundary programs run.
//!
//! This crate can reach [`xt_kernel_api`] and the shared workload, and nothing
//! else: not the kernel, not the library that implements the surface. That is
//! what makes it the right place for the scenario list, because the two
//! programs that use it differ in exactly one thing - how they obtained the
//! object - and must agree on everything after that.

#![deny(missing_docs)]

use std::hint::black_box;

use xt_bench as bench;
use xt_bench::Report;
use xt_kernel_api::{
    AbiStatus, StablePair, StableSlice, StableStr, StableVec, WorkspaceObject,
};

/// Runs every scenario against `workspace` and records the outcome.
///
/// The object is called the way any host calls it: through the accessors, with
/// `unsafe` because the object must have been built from this surface's
/// declaration - which the programs that call this guarantee.
pub fn run(workspace: &WorkspaceObject, report: &mut Report) {
    text(workspace, report);
    bytes(workspace, report);
    graphs(workspace, report);
    numbers(workspace, report);
    build(workspace, report);
    sessions(workspace, report);
    timings(workspace, report);
}

fn text(workspace: &WorkspaceObject, report: &mut Report) {
    // SAFETY: the object was built from this surface's declaration.
    let summary = unsafe { workspace.analyze(StableStr::from_str(bench::TEXT)) }.expect("analyze");
    report.is("analyze renders the kernel's summary", summary.as_str(), bench::SUMMARY);

    // SAFETY: the same guarantee covers the second call.
    let unbalanced = unsafe { workspace.analyze(StableStr::from_str(bench::UNBALANCED_TEXT)) }
        .expect("analyze");
    report.that(
        "an unbalanced text is reported as such",
        unbalanced.as_str().ends_with("balanced=false"),
    );

    // SAFETY: the same guarantee covers the token list.
    let tokens = unsafe { workspace.tokens(StableStr::from_str(bench::TOKENS_TEXT)) }.expect("tokens");
    let tokens: Vec<&str> = tokens
        .as_slice()
        .iter()
        .map(|token| token.as_str())
        .collect();
    report.is("tokens keep punctuation and numbers", tokens, bench::TOKENS.to_vec());

    // SAFETY: the same guarantee covers the ranking.
    let ranking =
        unsafe { workspace.top_terms(StableStr::from_str(bench::RANKING_TEXT), bench::RANKING_LIMIT) }
            .expect("top_terms");
    let ranking: Vec<(&str, u64)> = ranking
        .as_slice()
        .iter()
        .map(|pair| (pair.left.as_str(), pair.right))
        .collect();
    report.is("the ranking is bounded and ordered", ranking, bench::RANKING.to_vec());

    // SAFETY: the same guarantee covers the empty ranking.
    let empty = unsafe { workspace.top_terms(StableStr::from_str(bench::RANKING_TEXT), 0) }
        .expect("top_terms");
    report.that("a limit of zero ranks nothing", empty.is_empty());

    // SAFETY: the same guarantee covers normalisation.
    let normalized =
        unsafe { workspace.normalize(StableStr::from_str(bench::NORMALIZE_INPUT)) }.expect("normalize");
    report.is(
        "normalisation folds case and space",
        normalized.as_str(),
        bench::NORMALIZE_OUTPUT,
    );

    // SAFETY: the same guarantee covers the analyzer scores.
    let scores = unsafe { workspace.scores(StableStr::from_str(bench::SCORES_TEXT)) }.expect("scores");
    report.is("both analyzers answer", scores.as_str(), bench::SCORES);

    // SAFETY: the same guarantee covers the byte walker.
    let scan = unsafe { workspace.scan(StableStr::from_str(bench::SCAN_TEXT)) }.expect("scan");
    report.is("the walker reports depth and punctuation", scan.as_str(), bench::SCAN);

    // SAFETY: the same guarantee covers the second walk.
    let scan = unsafe { workspace.scan(StableStr::from_str(bench::SCAN_OPEN_TEXT)) }.expect("scan");
    report.is("an open quote is reported", scan.as_str(), bench::SCAN_OPEN);
}

fn bytes(workspace: &WorkspaceObject, report: &mut Report) {
    // SAFETY: the object was built from this surface's declaration.
    let digest = unsafe { workspace.digest(StableSlice::from_slice(bench::DIGEST_INPUT)) }
        .expect("digest");
    report.is(
        "both checksums are computed over the caller's bytes",
        digest.as_str(),
        bench::DIGEST,
    );

    // SAFETY: the same guarantee covers the empty input.
    let digest =
        unsafe { workspace.digest(StableSlice::from_slice(&[])) }.expect("digest of no bytes");
    report.is(
        "an empty input still has a digest",
        digest.as_str(),
        bench::DIGEST_EMPTY,
    );
}

fn graphs(workspace: &WorkspaceObject, report: &mut Report) {
    // SAFETY: the object was built from this surface's declaration.
    let walk = unsafe { workspace.reachable(edges(bench::EDGES), bench::START) }.expect("reachable");
    report.is(
        "a walk visits every node once",
        walk.as_slice().to_vec(),
        bench::REACHABLE.to_vec(),
    );

    // SAFETY: the same guarantee covers the order.
    let order = unsafe { workspace.topological(edges(bench::EDGES)) }.expect("topological answers");
    match order.into_result() {
        Ok(nodes) => report.is(
            "an acyclic graph is ordered",
            nodes.as_slice().to_vec(),
            bench::ORDER.to_vec(),
        ),
        Err(error) => {
            report.that("an acyclic graph is ordered", false);
            report.note("the library refused a valid graph", error);
        }
    }

    // SAFETY: the same guarantee covers the refusal.
    let refused = unsafe { workspace.topological(edges(bench::CYCLE)) }.expect("topological answers");
    match refused.into_result() {
        Err(error) => {
            report.is(
                "a cycle is refused, not ordered",
                error.status(),
                AbiStatus::InvalidArgument,
            );
            report.that(
                "the kernel's reason names the cycle",
                error.message().contains("cycle"),
            );
        }
        Ok(_) => report.that("a cycle is refused, not ordered", false),
    }
}

fn numbers(workspace: &WorkspaceObject, report: &mut Report) {
    for (value, expected) in bench::BANDS {
        // SAFETY: the object was built from this surface's declaration.
        let band = unsafe { workspace.classify(*value) }.expect("classify");
        report.is("a number is classified", band.as_str(), *expected);
    }

    // SAFETY: the same guarantee covers the split.
    let split = unsafe { workspace.split_fixed(bench::FIXED) }.expect("split_fixed");
    report.is(
        "a fixed-point value splits",
        (split.left, split.right),
        bench::FIXED_PARTS,
    );

    // SAFETY: the same guarantee covers the negative split.
    let split = unsafe { workspace.split_fixed(-bench::FIXED) }.expect("split_fixed");
    let (whole, fraction) = bench::FIXED_PARTS;
    report.is(
        "the sign is carried by both parts",
        (split.left, split.right),
        (-whole, -fraction),
    );
}

fn build(workspace: &WorkspaceObject, report: &mut Report) {
    // SAFETY: the object was built from this surface's declaration.
    let capabilities = unsafe { workspace.capabilities() }.expect("capabilities");
    let capabilities: Vec<&str> = capabilities
        .as_slice()
        .iter()
        .map(|item| item.as_str())
        .collect();
    report.is(
        "the build reports every fact it publishes",
        capabilities.len(),
        bench::CAPABILITIES,
    );
    report.that(
        "the build reports its own version",
        capabilities.contains(&"version=0.1.0"),
    );
    report.that(
        "the build reports its pointer width",
        capabilities.iter().any(|item| item.starts_with("pointer_width=")),
    );

    // SAFETY: the same guarantee covers the counters.
    let stats = unsafe { workspace.stats() }.expect("stats");
    report.that(
        "the counters report every figure",
        ["analyses=", "cache_hits=", "sessions=", "tokens=", "checksums="]
            .iter()
            .all(|field| stats.as_str().contains(field)),
    );

    // SAFETY: the same guarantee covers the clock.
    let now = unsafe { workspace.now() }.expect("now");
    report.that(
        "the clock answers with the current time",
        now.seconds > 1_600_000_000 && now.nanos < 1_000_000_000,
    );
}

fn sessions(workspace: &WorkspaceObject, report: &mut Report) {
    // SAFETY: the object was built from this surface's declaration.
    let mut first = unsafe { workspace.open_session(StableStr::from_str("first")) }
        .expect("open_session");
    // SAFETY: the session was built from this surface's declaration.
    report.is(
        "a session reports the name it was opened with",
        unsafe { first.name() }.expect("name").as_str(),
        "first",
    );
    for text in bench::SESSION_TEXTS {
        // SAFETY: the same guarantee covers every add.
        unsafe { first.add(StableStr::from_str(text)) }.expect("add");
    }
    // SAFETY: the same guarantee covers the total.
    report.is(
        "a session counts what it holds",
        unsafe { first.total() }.expect("total"),
        bench::SESSION_TOTAL,
    );

    // SAFETY: the same guarantee covers the summary.
    let summary = unsafe { first.summary() }.expect("summary");
    report.is(
        "the summary describes the session",
        summary.as_str(),
        bench::SESSION_SUMMARY,
    );

    // SAFETY: the same guarantee covers the child.
    let child = unsafe { first.child() }.expect("child");
    // SAFETY: the child was built from this surface's declaration.
    report.is(
        "a session hands out a child that carries its total",
        unsafe { child.value() },
        bench::SESSION_TOTAL,
    );

    // SAFETY: the same guarantee covers the second session.
    let mut second = unsafe { workspace.open_session(StableStr::from_str("second")) }
        .expect("open_session");
    // SAFETY: the same guarantee covers the add.
    unsafe { second.add(StableStr::from_str(bench::MERGE_TEXT)) }.expect("add");
    // SAFETY: the same guarantee covers the merge; the peer is consumed.
    report.is(
        "merging adds the peer's total",
        unsafe { first.merge(second) }.expect("merge"),
        bench::MERGED_TOTAL,
    );

    // SAFETY: the same guarantee covers the panic probe.
    match unsafe { first.explode() } {
        Err(error) => report.is(
            "a panic stays on the library's side",
            error.status(),
            AbiStatus::Panic,
        ),
        Ok(_) => report.that("a panic stays on the library's side", false),
    }

    // SAFETY: the session is consumed by the call.
    let closed = unsafe { first.finish() }.expect("finish");
    report.is(
        "a finished session describes itself",
        closed.as_str(),
        bench::SESSION_CLOSED_ABSORBED,
    );
}

fn timings(workspace: &WorkspaceObject, report: &mut Report) {
    let bulk = bench::bulk_bytes();

    report.perf(
        "analyze (cached)",
        bench::measure(bench::rounds::ANALYZE, || {
            // SAFETY: the object was built from this surface's declaration.
            let summary = unsafe { workspace.analyze(StableStr::from_str(bench::TEXT)) }
                .expect("analyze");
            black_box(summary.len());
        }),
    );

    report.perf(
        "tokens",
        bench::measure(bench::rounds::TOKENS, || {
            // SAFETY: the same guarantee covers every call.
            let tokens = unsafe { workspace.tokens(StableStr::from_str(bench::TOKENS_TEXT)) }
                .expect("tokens");
            black_box(tokens.len());
        }),
    );

    report.perf(
        "top_terms",
        bench::measure(bench::rounds::TOKENS, || {
            // SAFETY: the same guarantee covers every call.
            let ranking = unsafe {
                workspace.top_terms(StableStr::from_str(bench::RANKING_TEXT), bench::RANKING_LIMIT)
            }
            .expect("top_terms");
            black_box(ranking.len());
        }),
    );

    report.perf(
        "digest of 4 KiB",
        bench::measure(bench::rounds::DIGEST, || {
            // SAFETY: the same guarantee covers every call.
            let digest = unsafe { workspace.digest(StableSlice::from_slice(&bulk)) }.expect("digest");
            black_box(digest.len());
        }),
    );

    report.perf(
        "reachable",
        bench::measure(bench::rounds::GRAPH, || {
            // SAFETY: the same guarantee covers every call.
            let walk = unsafe { workspace.reachable(edges(bench::EDGES), bench::START) }
                .expect("reachable");
            black_box(walk.len());
        }),
    );

    report.perf(
        "classify",
        bench::measure(bench::rounds::SCALARS, || {
            // SAFETY: the same guarantee covers every call.
            let band = unsafe { workspace.classify(150) }.expect("classify");
            black_box(band.len());
        }),
    );

    // Returns two numbers: no allocation, so the difference against `classify`
    // is the cost the boundary pays for building a returned owned value. The
    // argument changes every round, like the reference program's.
    let mut scalar = 0i64;
    report.perf(
        "split_fixed (no allocation)",
        bench::measure(bench::rounds::SCALARS, || {
            scalar = scalar.wrapping_add(1);
            // SAFETY: the same guarantee covers every call.
            let split = unsafe { workspace.split_fixed(black_box(scalar)) }
                .expect("split_fixed");
            black_box((split.left, split.right));
        }),
    );

    report.perf(
        "session round trip",
        bench::measure(bench::rounds::SESSION, || {
            // SAFETY: the same guarantee covers every call.
            let mut session = unsafe { workspace.open_session(StableStr::from_str("timed")) }
                .expect("open_session");
            for text in bench::SESSION_TEXTS {
                // SAFETY: the same guarantee covers every add.
                unsafe { session.add(StableStr::from_str(text)) }.expect("add");
            }
            // SAFETY: the session is consumed by the call.
            let closed = unsafe { session.finish() }.expect("finish");
            black_box(closed.len());
        }),
    );
}

/// Builds the edge sequence a graph scenario passes in.
fn edges(pairs: &[(u64, u64)]) -> StableVec<StablePair<u64, u64>> {
    let mut out = StableVec::new();
    for (from, to) in pairs {
        out.try_push(StablePair::new(*from, *to)).expect("edges");
    }
    out
}
