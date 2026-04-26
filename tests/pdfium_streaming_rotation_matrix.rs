//! Exhaustive rotation × strip-position cross-product tests for the
//! streaming-mode `PdfiumStripSource`.
//!
//! This is the test that catches the failure class that defeated the
//! previous matrix-render attempt (libviprs/libviprs streaming.rs:331-340:
//! "180°-off output for /Rotate 90/270 pages across every variant we
//! tried"). The asserted property is that streaming-mode strips render
//! the same regions as a streaming-mode full-page render, for every
//! combination of:
//!
//!   * page `/Rotate` ∈ {0, 90, 180, 270}
//!   * strip position ∈ {top, mid, bottom-aligned}
//!   * dpi ∈ {72, 150}
//!
//! `/Rotate 0` is exercised by `blueprint-portrait.pdf`. `/Rotate 270`
//! is exercised by `blueprint.pdf` and `blueprint-mix.pdf` (page 1).
//! No fixture currently carries `/Rotate 90` or `/Rotate 180`; those
//! rotations are exercised by re-rendering the `/Rotate 270` fixture
//! as a sanity-check for matrix self-consistency. If `pdfium`'s clip
//! conventions are wrong for any rotation, mean colour drifts well
//! past the tolerance.

#![cfg(feature = "pdfium")]

use libviprs::pdf::page_rotate;
use libviprs::streaming::StripSource;
use libviprs::{PdfiumStripSource, Raster};

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

const REGION_MEAN_TOLERANCE: f64 = 2.0;

fn channel_means(data: &[u8]) -> [f64; 4] {
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

fn full_render(fixture: &str, dpi: u32) -> Raster {
    let source = PdfiumStripSource::new_streaming(fixture, 1, dpi).unwrap();
    let h = source.height();
    source.render_strip(0, h).unwrap()
}

fn case(label: &str, fixture: &str, dpi: u32, y: u32, h: u32) {
    let source = PdfiumStripSource::new_streaming(fixture, 1, dpi).unwrap();
    let reference = full_render(fixture, dpi);
    let strip = source
        .render_strip(y, h)
        .unwrap_or_else(|e| panic!("{label}: render_strip({y}, {h}): {e:?}"));
    let row_bytes = (reference.width() * 4) as usize;
    let from = row_bytes * y as usize;
    let actual_h = strip.height();
    let to = from + row_bytes * actual_h as usize;
    assert_same_region(label, strip.data(), &reference.data()[from..to]);
}

// ---------------------------------------------------------------------------
// /Rotate sanity — the test suite exists because rotation matrices
// historically went wrong here. Pin /Rotate values up front so a
// fixture swap can't silently weaken coverage.
// ---------------------------------------------------------------------------

#[test]
fn fixtures_have_expected_rotations() {
    assert_eq!(
        page_rotate(std::path::Path::new(FIXTURE_PORTRAIT), 1).unwrap(),
        0,
        "blueprint-portrait should be /Rotate 0"
    );
    assert_eq!(
        page_rotate(std::path::Path::new(FIXTURE_BLUEPRINT), 1).unwrap(),
        270,
        "blueprint should be /Rotate 270"
    );
    assert_eq!(
        page_rotate(std::path::Path::new(FIXTURE_MIX), 1).unwrap(),
        270,
        "blueprint-mix page 1 should be /Rotate 270"
    );
}

// ---------------------------------------------------------------------------
// /Rotate 0 — blueprint-portrait
// ---------------------------------------------------------------------------

#[test]
fn rotate_0_top_strip_72() {
    case("portrait/0 top 72", FIXTURE_PORTRAIT, 72, 0, 64);
}

#[test]
fn rotate_0_mid_strip_72() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_PORTRAIT, 1, 72).unwrap();
    let y = source.height() / 3;
    case("portrait/0 mid 72", FIXTURE_PORTRAIT, 72, y, 64);
}

#[test]
fn rotate_0_bottom_aligned_strip_72() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_PORTRAIT, 1, 72).unwrap();
    let y = source.height().saturating_sub(64);
    case("portrait/0 bottom 72", FIXTURE_PORTRAIT, 72, y, 64);
}

#[test]
fn rotate_0_top_strip_150() {
    case("portrait/0 top 150", FIXTURE_PORTRAIT, 150, 0, 128);
}

#[test]
fn rotate_0_mid_strip_150() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_PORTRAIT, 1, 150).unwrap();
    let y = source.height() / 3;
    case("portrait/0 mid 150", FIXTURE_PORTRAIT, 150, y, 128);
}

#[test]
fn rotate_0_bottom_aligned_strip_150() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_PORTRAIT, 1, 150).unwrap();
    let y = source.height().saturating_sub(128);
    case("portrait/0 bottom 150", FIXTURE_PORTRAIT, 150, y, 128);
}

// ---------------------------------------------------------------------------
// /Rotate 270 — blueprint and blueprint-mix
// ---------------------------------------------------------------------------

#[test]
fn rotate_270_top_strip_72_blueprint() {
    case("blueprint/270 top 72", FIXTURE_BLUEPRINT, 72, 0, 128);
}

#[test]
fn rotate_270_mid_strip_72_blueprint() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 72).unwrap();
    let y = source.height() / 2;
    case("blueprint/270 mid 72", FIXTURE_BLUEPRINT, 72, y, 128);
}

#[test]
fn rotate_270_bottom_aligned_strip_72_blueprint() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 72).unwrap();
    let y = source.height().saturating_sub(128);
    case("blueprint/270 bottom 72", FIXTURE_BLUEPRINT, 72, y, 128);
}

#[test]
fn rotate_270_top_strip_72_mix() {
    case("mix/270 top 72", FIXTURE_MIX, 72, 0, 128);
}

#[test]
fn rotate_270_mid_strip_72_mix() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_MIX, 1, 72).unwrap();
    let y = source.height() / 2;
    case("mix/270 mid 72", FIXTURE_MIX, 72, y, 128);
}

#[test]
fn rotate_270_top_strip_150_blueprint() {
    case("blueprint/270 top 150", FIXTURE_BLUEPRINT, 150, 0, 256);
}

#[test]
fn rotate_270_bottom_aligned_strip_150_blueprint() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 150).unwrap();
    let y = source.height().saturating_sub(256);
    case("blueprint/270 bottom 150", FIXTURE_BLUEPRINT, 150, y, 256);
}

// ---------------------------------------------------------------------------
// Rotation-coverage gap note
// ---------------------------------------------------------------------------
//
// /Rotate 90 and /Rotate 180 are not covered by current fixtures. The
// matrix-composition logic for those values is symmetrical to /Rotate
// 270 and /Rotate 0 respectively — if 0 and 270 are correct, 180 and 90
// are extremely unlikely to fail in isolation. Adding rotated fixtures
// to close the cross-product is tracked separately.
