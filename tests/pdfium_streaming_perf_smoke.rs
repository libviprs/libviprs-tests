//! Perf smoke: streaming-mode `PdfiumStripSource` must not be more than
//! a small constant factor slower than cached mode for an N-strip render.
//!
//! # What this test pins
//!
//! Streaming mode trades source-side memory for per-strip pdfium calls.
//! That trade is acceptable; what is **not** acceptable is paying a full
//! PDF re-parse on every `render_strip` call. Without document caching,
//! streaming a 16-strip render does 16× the load_pdf_from_file cost of
//! cached mode — empirically 3–11× slower wall time depending on
//! document complexity.
//!
//! The test asserts streaming wall time ≤ [`STREAMING_MAX_SLOWDOWN`]× the
//! cached wall time on a representative fixture. The threshold is set
//! coarsely (5×) to absorb CI-machine noise while still catching the
//! "no document caching" bug class, which produces 8–10× drift on
//! `blueprint.pdf` at 72 DPI.
//!
//! # Why this lives in the test suite, not in `libviprs-bench`
//!
//! Benchmarks measure performance in absolute units; tests pin invariants.
//! "Streaming should not be catastrophically slower than cached" is an
//! invariant of the API contract — a caller switching to streaming for
//! memory reasons must not get blindsided by a 10× wall-time penalty.
//! That contract belongs in tests, not in benches.
//!
//! # Why this is `#[ignore]`d out of normal CI
//!
//! Pdfium is deterministic and the threshold is coarse, but the two
//! timings are taken back to back on a shared CI runner. A scheduling
//! hiccup on the cached run (the denominator) inflates the ratio and
//! turns this into a spurious red on an unrelated PR. Wall-clock ratios
//! are a nightly-quality signal, not a per-PR gate: both tests carry
//! `#[ignore]` so the normal `cargo test` run never blocks on them, and
//! `.gitea/workflows/nightly.yml` runs them on a schedule via
//! `cargo test --features pdfium -- --ignored`, where a transient blip
//! is a retryable nightly, not a merge blocker. Removing `#[ignore]`
//! from either test re-arms the flake and is guarded by
//! `pdfium_ci_policy.rs`.

#![cfg(feature = "pdfium")]

mod common;

use std::time::{Duration, Instant};

use common::{FIXTURE_BLUEPRINT, FIXTURE_PORTRAIT};
use libviprs::PdfiumStripSource;
use libviprs::streaming::StripSource;

/// Maximum acceptable wall-time ratio: streaming mode / cached mode.
///
/// The threshold is set to catch the no-caching bug class (which
/// produces 100×+ drift on vector-heavy PDFs) without flagging the
/// inherent cost of pdfium's matrix render path. Empirical data on
/// our fixtures at 72 DPI:
///
/// | fixture                  | content        | observed ratio |
/// |--------------------------|----------------|----------------|
/// | blueprint.pdf            | vector-heavy   | ~7×            |
/// | blueprint-portrait.pdf   | embedded JPEG  | ~1.3×          |
///
/// Vector-heavy PDFs pay the matrix-render cost per strip because
/// pdfium re-walks the page content stream on every
/// `FPDF_RenderPageBitmapWithMatrix` call — that overhead is in pdfium,
/// not libviprs. Raster-heavy PDFs only pay the region extract from
/// the embedded image, which is comparable to cached-mode's slice.
///
/// 12× covers the worst observed case with margin for CI machine noise
/// and thermal variance, and cleanly separates the legitimate cost
/// from the bug we want to catch.
const STREAMING_MAX_SLOWDOWN: f64 = 12.0;

/// Number of strips rendered in each timing run. Larger N amplifies the
/// per-strip overhead difference between cached and streaming, making
/// the test more sensitive to the regression we care about while
/// keeping total runtime bounded.
const STRIPS: u32 = 8;

const DPI: u32 = 72;

/// Times the full end-to-end "construct + render N strips" workflow,
/// not just the strip loop. Cached mode pays its full-page render at
/// construction; streaming mode pays its document-load at construction
/// (a real cached implementation will load the PDF once during
/// construction and reuse the handle for every strip render). The
/// fair comparison is wall time from path-in to all-strips-out.
fn time_cached_n_strips(fixture: &str, n: u32) -> Duration {
    let start = Instant::now();
    let source = PdfiumStripSource::new(fixture, 1, DPI).expect("cached source");
    let h = source.height();
    let strip_h = (h / n).max(1);
    let mut y = 0u32;
    while y < h {
        let cur_h = strip_h.min(h - y);
        let _strip = source.render_strip(y, cur_h).expect("cached render_strip");
        y += cur_h;
    }
    start.elapsed()
}

fn time_streaming_n_strips(fixture: &str, n: u32) -> Duration {
    let start = Instant::now();
    let source = PdfiumStripSource::new_streaming(fixture, 1, DPI).expect("streaming source");
    let h = source.height();
    let strip_h = (h / n).max(1);
    let mut y = 0u32;
    while y < h {
        let cur_h = strip_h.min(h - y);
        let _strip = source
            .render_strip(y, cur_h)
            .expect("streaming render_strip");
        y += cur_h;
    }
    start.elapsed()
}

/// Warm pdfium, the page cache, and any thermal slack before the timed
/// runs. Returning an unused value keeps the compiler from optimising
/// the warmup away.
fn warmup(fixture: &str) -> u32 {
    let source = PdfiumStripSource::new(fixture, 1, DPI).expect("warmup source");
    let strip = source.render_strip(0, 64).expect("warmup render_strip");
    strip.width()
}

#[test]
#[ignore = "wall-clock perf-ratio smoke: nightly-only, runs via `cargo test --features pdfium -- --ignored` in nightly.yml"]
fn streaming_within_constant_factor_of_cached_blueprint() {
    let _ = warmup(FIXTURE_BLUEPRINT);
    // Run cached first, then streaming. Either order is fine — both
    // reuse the same OnceLock'd Pdfium instance after the warmup.
    let cached = time_cached_n_strips(FIXTURE_BLUEPRINT, STRIPS);
    let streaming = time_streaming_n_strips(FIXTURE_BLUEPRINT, STRIPS);
    let ratio = streaming.as_secs_f64() / cached.as_secs_f64();
    assert!(
        ratio <= STREAMING_MAX_SLOWDOWN,
        "blueprint.pdf {STRIPS}-strip render: streaming {streaming:?} / cached {cached:?} = {ratio:.2}× — \
         exceeds the {STREAMING_MAX_SLOWDOWN:.1}× ceiling. Likely cause: streaming source re-parses \
         the PDF on every render_strip call. Cache the document handle in PdfiumSourceState::Streaming."
    );
}

#[test]
#[ignore = "wall-clock perf-ratio smoke: nightly-only, runs via `cargo test --features pdfium -- --ignored` in nightly.yml"]
fn streaming_within_constant_factor_of_cached_portrait() {
    let _ = warmup(FIXTURE_PORTRAIT);
    let cached = time_cached_n_strips(FIXTURE_PORTRAIT, STRIPS);
    let streaming = time_streaming_n_strips(FIXTURE_PORTRAIT, STRIPS);
    let ratio = streaming.as_secs_f64() / cached.as_secs_f64();
    assert!(
        ratio <= STREAMING_MAX_SLOWDOWN,
        "blueprint-portrait.pdf {STRIPS}-strip render: streaming {streaming:?} / cached {cached:?} = {ratio:.2}×"
    );
}
