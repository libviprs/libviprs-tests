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

mod common;

use std::path::Path;

use common::{
    DIM_DRIFT_TOLERANCE_PX, FIXTURE_BLUEPRINT, FIXTURE_MIX, FIXTURE_PORTRAIT, FIXTURES,
    assert_same_region,
};
use libviprs::pdf::{PdfError, render_page_pdfium};
use libviprs::streaming::{BudgetPolicy, StripSource};
use libviprs::{PdfiumStripSource, PixelFormat};

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
    match result {
        Err(PdfError::PageOutOfRange { page, .. }) => assert_eq!(page, 999),
        other => panic!("expected PageOutOfRange for page 999, got {other:?}"),
    }
}

#[test]
fn new_streaming_page_zero_errors_typed() {
    let result = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 0, 72);
    match result {
        Err(PdfError::PageOutOfRange { page, .. }) => assert_eq!(page, 0),
        other => panic!("expected PageOutOfRange for page 0, got {other:?}"),
    }
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
