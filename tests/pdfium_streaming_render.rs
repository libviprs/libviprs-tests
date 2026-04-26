//! Tests for the true-streaming `PdfiumStripSource` constructors
//! (`new_streaming` / `new_streaming_with_budget`) introduced by
//! libviprs#70.
//!
//! Streaming mode uses pdfium's matrix render path (one strip-sized bitmap
//! per `render_strip` call) instead of caching the full display-oriented
//! page. Public contract:
//!
//! 1. `new_streaming(path, page, dpi)` reports rotation-aware dimensions
//!    matching the form-data baseline within a few pixels of drift —
//!    same tolerance as the existing cached-mode `new` constructor.
//! 2. Each `render_strip` is deterministic — repeated calls return
//!    identical bytes.
//! 3. Stitched strips render the *same regions* as a full render of
//!    the same source, by per-channel mean. Pixel-exact equality is
//!    not asserted; pdfium's matrix path is bitmap-size-dependent in
//!    its anti-aliasing (see the rationale block in
//!    `streaming_pdfium_matrix.rs`).
//! 4. Streaming-mode dims agree with cached-mode dims (within drift
//!    tolerance) — the two constructors are interchangeable from the
//!    `StripSource` interface's point of view.
//! 5. Boundary strips (top, bottom-aligned, bottom-partial) render the
//!    correct page region.
//! 6. Out-of-range pages and page=0 return typed `PdfError`.

#![cfg(feature = "pdfium")]

use std::path::Path;

use libviprs::pdf::{PdfError, render_page_pdfium};
use libviprs::streaming::{BudgetPolicy, StripSource};
use libviprs::{PdfiumStripSource, PixelFormat};

const FIXTURE_BLUEPRINT: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");
const FIXTURE_PORTRAIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint-portrait.pdf"
);
const FIXTURE_MIX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint-mix.pdf"
);
const FIXTURES: &[&str] = &[FIXTURE_BLUEPRINT, FIXTURE_PORTRAIT, FIXTURE_MIX];

const DIM_DRIFT_TOLERANCE_PX: i64 = 4;
const REGION_MEAN_TOLERANCE: f64 = 2.0;

fn channel_means(data: &[u8]) -> [f64; 4] {
    assert!(data.len() % 4 == 0);
    let mut sums = [0u64; 4];
    let pixels = data.len() / 4;
    for chunk in data.chunks_exact(4) {
        for i in 0..4 {
            sums[i] += chunk[i] as u64;
        }
    }
    let n = pixels.max(1) as f64;
    [
        sums[0] as f64 / n,
        sums[1] as f64 / n,
        sums[2] as f64 / n,
        sums[3] as f64 / n,
    ]
}

fn assert_same_region(label: &str, actual: &[u8], reference: &[u8]) {
    assert_eq!(
        actual.len(),
        reference.len(),
        "{label}: byte-length mismatch"
    );
    let a = channel_means(actual);
    let b = channel_means(reference);
    let max = (0..4).map(|i| (a[i] - b[i]).abs()).fold(0.0_f64, f64::max);
    assert!(
        max <= REGION_MEAN_TOLERANCE,
        "{label}: per-channel mean drift {max:.3} > {REGION_MEAN_TOLERANCE:.3} \
         (actual={a:?}, reference={b:?})"
    );
}

// ---------------------------------------------------------------------------
// Constructor & metadata
// ---------------------------------------------------------------------------

#[test]
fn new_streaming_reports_rotation_aware_dimensions() {
    for &fixture in FIXTURES {
        for &dpi in &[72u32, 150] {
            let source = PdfiumStripSource::new_streaming(fixture, 1, dpi)
                .unwrap_or_else(|e| panic!("new_streaming({fixture}, 1, {dpi}): {e:?}"));
            let baseline = render_page_pdfium(Path::new(fixture), 1, dpi)
                .unwrap_or_else(|e| panic!("baseline form-data render: {e:?}"));
            let dw = (source.width() as i64 - baseline.width() as i64).abs();
            let dh = (source.height() as i64 - baseline.height() as i64).abs();
            assert!(
                dw <= DIM_DRIFT_TOLERANCE_PX && dh <= DIM_DRIFT_TOLERANCE_PX,
                "{fixture} {dpi}DPI: drift exceeds tolerance \
                 (source={}x{} baseline={}x{})",
                source.width(),
                source.height(),
                baseline.width(),
                baseline.height(),
            );
            let source_landscape = source.width() > source.height();
            let baseline_landscape = baseline.width() > baseline.height();
            assert_eq!(
                source_landscape, baseline_landscape,
                "{fixture} {dpi}DPI: orientation mismatch"
            );
        }
    }
}

#[test]
fn new_streaming_format_is_rgba8() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 72).unwrap();
    assert_eq!(source.format(), PixelFormat::Rgba8);
}

#[test]
fn new_streaming_page_out_of_range_errors_typed() {
    let result = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 999, 72);
    assert!(result.is_err());
}

#[test]
fn new_streaming_page_zero_errors_typed() {
    let result = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 0, 72);
    assert!(result.is_err());
}

/// Streaming-mode and cached-mode constructors must report dims within the
/// same drift tolerance. Callers swapping between them should not see
/// dimension changes.
#[test]
fn new_streaming_dims_match_cached_mode() {
    for &fixture in FIXTURES {
        for &dpi in &[72u32, 150] {
            let cached = PdfiumStripSource::new(fixture, 1, dpi).unwrap();
            let streaming = PdfiumStripSource::new_streaming(fixture, 1, dpi).unwrap();
            let dw = (cached.width() as i64 - streaming.width() as i64).abs();
            let dh = (cached.height() as i64 - streaming.height() as i64).abs();
            assert!(
                dw <= DIM_DRIFT_TOLERANCE_PX && dh <= DIM_DRIFT_TOLERANCE_PX,
                "{fixture} {dpi}DPI: cached vs streaming dim drift > {DIM_DRIFT_TOLERANCE_PX}px \
                 (cached={}x{} streaming={}x{})",
                cached.width(),
                cached.height(),
                streaming.width(),
                streaming.height(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn streaming_render_strip_is_deterministic() {
    for &fixture in FIXTURES {
        let source = PdfiumStripSource::new_streaming(fixture, 1, 72).unwrap();
        let h = 128.min(source.height());
        let a = source.render_strip(0, h).unwrap();
        let b = source.render_strip(0, h).unwrap();
        assert_eq!(
            a.data(),
            b.data(),
            "{fixture}: streaming render_strip is non-deterministic"
        );
    }
}

// ---------------------------------------------------------------------------
// Region correctness — stitched strips cover the same regions as a full
// streaming-mode render. We compare against the streaming source's own
// full-height render (avoids tying parity to cached-mode bytes, which
// pdfium's two render paths legitimately differ on).
// ---------------------------------------------------------------------------

fn streaming_full_render(fixture: &str, dpi: u32) -> libviprs::Raster {
    let source = PdfiumStripSource::new_streaming(fixture, 1, dpi).unwrap();
    let h = source.height();
    source
        .render_strip(0, h)
        .unwrap_or_else(|e| panic!("streaming full render({fixture}, {dpi}): {e:?}"))
}

fn stitch_streaming_strips(fixture: &str, dpi: u32, strip_h: u32) -> Vec<u8> {
    let source = PdfiumStripSource::new_streaming(fixture, 1, dpi).unwrap();
    let mut out = Vec::with_capacity((source.width() * source.height() * 4) as usize);
    let mut y = 0u32;
    while y < source.height() {
        let h = strip_h.min(source.height() - y);
        let strip = source
            .render_strip(y, h)
            .unwrap_or_else(|e| panic!("render_strip({y}, {h}): {e:?}"));
        assert_eq!(strip.width(), source.width());
        assert_eq!(strip.height(), h);
        out.extend_from_slice(strip.data());
        y += h;
    }
    out
}

#[test]
fn streaming_stitched_equal_streaming_full_72_dpi() {
    for &fixture in FIXTURES {
        let reference = streaming_full_render(fixture, 72);
        for &strip_h in &[64u32, 128, 256, 512] {
            let stitched = stitch_streaming_strips(fixture, 72, strip_h);
            let label = format!("{fixture} dpi=72 strip_h={strip_h}");
            assert_same_region(&label, &stitched, reference.data());
        }
    }
}

#[test]
fn streaming_stitched_equal_streaming_full_150_dpi() {
    for &fixture in &[FIXTURE_BLUEPRINT, FIXTURE_PORTRAIT] {
        let reference = streaming_full_render(fixture, 150);
        for &strip_h in &[64u32, 128, 256, 512] {
            let stitched = stitch_streaming_strips(fixture, 150, strip_h);
            let label = format!("{fixture} dpi=150 strip_h={strip_h}");
            assert_same_region(&label, &stitched, reference.data());
        }
    }
}

// ---------------------------------------------------------------------------
// Boundary strips
// ---------------------------------------------------------------------------

#[test]
fn first_strip_matches_top_of_full_render() {
    for &fixture in FIXTURES {
        let source = PdfiumStripSource::new_streaming(fixture, 1, 72).unwrap();
        let reference = streaming_full_render(fixture, 72);
        let h = 64u32.min(source.height());
        let strip = source.render_strip(0, h).unwrap();
        let row_bytes = (reference.width() * 4) as usize;
        let top = &reference.data()[..row_bytes * h as usize];
        assert_same_region(&format!("{fixture}: top strip"), strip.data(), top);
    }
}

#[test]
fn bottom_aligned_strip_matches_bottom_of_full_render() {
    for &fixture in &[FIXTURE_BLUEPRINT, FIXTURE_PORTRAIT] {
        let source = PdfiumStripSource::new_streaming(fixture, 1, 72).unwrap();
        let reference = streaming_full_render(fixture, 72);
        let strip_h = 64u32;
        let y = source.height().saturating_sub(strip_h);
        let strip = source.render_strip(y, strip_h).unwrap();
        let row_bytes = (reference.width() * 4) as usize;
        let bottom = &reference.data()[row_bytes * y as usize..];
        assert_same_region(
            &format!("{fixture}: bottom-aligned strip"),
            strip.data(),
            bottom,
        );
    }
}

#[test]
fn partial_last_strip_returns_actual_height() {
    // Request a strip larger than what remains. The `StripSource` contract
    // (streaming.rs:127-129) allows the implementation to return a shorter
    // raster — and the cached-mode source already does (`strip_h =
    // height.min(...)` at streaming.rs:341). Streaming mode must match.
    for &fixture in FIXTURES {
        let source = PdfiumStripSource::new_streaming(fixture, 1, 72).unwrap();
        let h_total = source.height();
        let y = h_total.saturating_sub(50);
        let strip = source.render_strip(y, 200).unwrap();
        assert_eq!(
            strip.height(),
            h_total - y,
            "{fixture}: partial last strip height mismatch"
        );
    }
}

// ---------------------------------------------------------------------------
// Budget mode
// ---------------------------------------------------------------------------

const TILE_SIZE: u32 = 256;
const MIN_STRIP_HEIGHT: u32 = 2 * TILE_SIZE;

fn worst_case_strip_bytes(width: u32) -> u64 {
    width as u64 * MIN_STRIP_HEIGHT as u64 * 4
}

#[test]
fn new_streaming_with_budget_passes_when_fits() {
    let baseline = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 72).unwrap();
    let budget = worst_case_strip_bytes(baseline.width()) * 4;
    let source = PdfiumStripSource::new_streaming_with_budget(
        FIXTURE_BLUEPRINT,
        1,
        72,
        MIN_STRIP_HEIGHT,
        budget,
        BudgetPolicy::Error,
    )
    .expect("budget large enough");
    assert_eq!(source.width(), baseline.width());
    assert_eq!(source.height(), baseline.height());
}

#[test]
fn new_streaming_with_budget_returns_typed_error_when_too_small() {
    let result = PdfiumStripSource::new_streaming_with_budget(
        FIXTURE_BLUEPRINT,
        1,
        300,
        MIN_STRIP_HEIGHT,
        1,
        BudgetPolicy::Error,
    );
    match result {
        Err(PdfError::BudgetExceeded { .. }) => {}
        other => panic!("expected BudgetExceeded, got {other:?}"),
    }
}

#[test]
fn new_streaming_with_budget_auto_adjust_reduces_dpi() {
    let baseline_300 = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 300).unwrap();
    let budget = worst_case_strip_bytes(baseline_300.width()) / 16;
    let source = PdfiumStripSource::new_streaming_with_budget(
        FIXTURE_BLUEPRINT,
        1,
        300,
        MIN_STRIP_HEIGHT,
        budget,
        BudgetPolicy::AutoAdjustDpi { min_dpi: 1 },
    )
    .expect("auto-adjust must succeed");
    assert!(source.width() < baseline_300.width());
    assert!(worst_case_strip_bytes(source.width()) <= budget);
}
