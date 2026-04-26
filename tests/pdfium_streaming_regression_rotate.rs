//! Regression barrier for the `/Rotate 90/270` 180°-off bug class.
//!
//! # What we're guarding against
//!
//! Before commit 1e64250 on branch `feature/streaming-pdf-strip`, the
//! matrix-render path for `PdfiumStripSource` produced output that was
//! 180° rotated relative to the form-data path on `/Rotate 90/270`
//! pages — a hand-composed rotation matrix issue documented at
//! `streaming.rs:331-340` (since removed). The fix lives in the per-
//! rotation matrix table at `pdf::render_page_strip_pdfium`.
//!
//! The new test suite (`pdfium_streaming_render.rs`,
//! `pdfium_streaming_rotation_matrix.rs`,
//! `pdfium_streaming_synthetic_rotations.rs`) pins streaming-mode
//! self-consistency — stitched strips equal a single full streaming
//! render. **That is not enough.** A matrix that's *consistently* wrong
//! (e.g. always shifted by the page height, always rotated 180°) would
//! pass every self-consistency check; both stitched and full would be
//! wrong in the same way and agree.
//!
//! This file fixes that gap by comparing streaming-mode output to a
//! ground truth that does **not** go through the matrix path: the
//! cached-mode source, which uses pdfium's form-data render path
//! (auto-rotated, well-trodden, trusted). A wide-but-finite mean
//! tolerance (`CROSS_PATH_MEAN_TOLERANCE`, calibrated empirically in
//! `tests/common/mod.rs`) absorbs the legitimate AA divergence between
//! the two pdfium render paths while still catching gross matrix
//! errors — which produce drift in the tens of mean units, not single
//! digits.
//!
//! # Why this is the right shape
//!
//! Rotation bugs on the matrix path manifest as **wrong-region** output.
//! `streaming.strip(0, h)` should show the top `h` rows of the displayed
//! page; the prior bug made it show the bottom `h` rows. Comparing
//! against the cached-mode strip at the same `(y, h)` is a direct test
//! of "did the matrix put the right page region into the bitmap".
//!
//! The `CROSS_PATH_MEAN_TOLERANCE = 10.0` ceiling is wide enough to
//! pass under pdfium's path-divergence (~5 units empirically, see the
//! first failed iteration of the synthetic rotations test) but tight
//! enough that a 180°-rotated strip — which lands on entirely
//! different page content — exceeds it by an order of magnitude.

#![cfg(feature = "pdfium")]

mod common;

use common::{FIXTURE_BLUEPRINT, FIXTURE_PORTRAIT, assert_same_region_cross_path, crop_top_left};
use libviprs::PdfiumStripSource;
use libviprs::streaming::StripSource;

const DPI: u32 = 72;

/// Compares `(y, h)` strips between the cached form-data render path
/// and the streaming matrix render path. Crops both to common
/// dimensions to absorb the few-pixel drift between the two paths'
/// reported sizes.
fn assert_streaming_strip_matches_cached(label: &str, fixture: &str, y: u32, h: u32) {
    let cached = PdfiumStripSource::new(fixture, 1, DPI).expect("cached source");
    let streaming = PdfiumStripSource::new_streaming(fixture, 1, DPI).expect("streaming source");

    let common_w = cached.width().min(streaming.width());

    let cached_strip = cached.render_strip(y, h).expect("cached strip");
    let streaming_strip = streaming.render_strip(y, h).expect("streaming strip");

    let common_h = cached_strip.height().min(streaming_strip.height());

    let cached_crop = crop_top_left(&cached_strip, common_w, common_h);
    let streaming_crop = crop_top_left(&streaming_strip, common_w, common_h);

    assert_same_region_cross_path(
        &format!("{label}: streaming(y={y}, h={h}) vs cached(y={y}, h={h})"),
        &streaming_crop,
        &cached_crop,
    );
}

// ---------------------------------------------------------------------------
// /Rotate 270 — the rotation that hosted the prior bug. blueprint.pdf
// is the canonical fixture here (display orientation: landscape). If the
// matrix has any sign error in its rotation, the streaming strip lands
// on the wrong page region and the cross-path mean drift exceeds the
// tolerance ceiling.
// ---------------------------------------------------------------------------

#[test]
fn rotate_270_top_strip_is_top_of_displayed_page() {
    assert_streaming_strip_matches_cached(
        "blueprint /Rotate 270 top strip",
        FIXTURE_BLUEPRINT,
        0,
        64,
    );
}

#[test]
fn rotate_270_quarter_strip_is_quarter_position() {
    let cached = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let y = cached.height() / 4;
    assert_streaming_strip_matches_cached(
        "blueprint /Rotate 270 quarter-down strip",
        FIXTURE_BLUEPRINT,
        y,
        64,
    );
}

#[test]
fn rotate_270_mid_strip_is_middle_of_displayed_page() {
    let cached = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let y = cached.height() / 2;
    assert_streaming_strip_matches_cached(
        "blueprint /Rotate 270 mid strip",
        FIXTURE_BLUEPRINT,
        y,
        64,
    );
}

#[test]
fn rotate_270_three_quarter_strip_is_lower_half() {
    let cached = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let y = (cached.height() / 4) * 3;
    assert_streaming_strip_matches_cached(
        "blueprint /Rotate 270 three-quarters-down strip",
        FIXTURE_BLUEPRINT,
        y,
        64,
    );
}

#[test]
fn rotate_270_bottom_aligned_strip_is_bottom_of_displayed_page() {
    let cached = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let y = cached.height().saturating_sub(64);
    assert_streaming_strip_matches_cached(
        "blueprint /Rotate 270 bottom strip",
        FIXTURE_BLUEPRINT,
        y,
        64,
    );
}

// ---------------------------------------------------------------------------
// /Rotate 0 — control. Sanity check that the cross-path comparison
// itself isn't fundamentally broken on a non-rotated fixture; if these
// fail, the issue is in the test setup, not the matrix.
// ---------------------------------------------------------------------------

#[test]
fn rotate_0_top_strip_matches() {
    assert_streaming_strip_matches_cached(
        "blueprint-portrait /Rotate 0 top strip",
        FIXTURE_PORTRAIT,
        0,
        64,
    );
}

#[test]
fn rotate_0_mid_strip_matches() {
    let cached = PdfiumStripSource::new(FIXTURE_PORTRAIT, 1, DPI).unwrap();
    let y = cached.height() / 2;
    assert_streaming_strip_matches_cached(
        "blueprint-portrait /Rotate 0 mid strip",
        FIXTURE_PORTRAIT,
        y,
        64,
    );
}

// ---------------------------------------------------------------------------
// Negative test (defensive): if the streaming source rendered the
// **wrong** strip — i.e., asked for strip(0, h) but got strip(H-h, h) —
// the means would diverge wildly. This test asserts that distinct
// strip positions on a non-uniform document have distinct means, so
// the cross-path comparison is genuinely sensitive to position.
// Without this calibration, a streaming source that always returned
// (e.g.) blank white could pass the above tests on a mostly-white
// fixture.
// ---------------------------------------------------------------------------

#[test]
fn distinct_strip_positions_have_distinct_means() {
    use common::channel_means;

    let source = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let h_total = source.height();
    let h = 64u32;

    let top = source.render_strip(0, h).unwrap();
    let middle = source.render_strip(h_total / 2, h).unwrap();
    let bottom = source.render_strip(h_total.saturating_sub(h), h).unwrap();

    let top_mean = channel_means(top.data());
    let middle_mean = channel_means(middle.data());
    let bottom_mean = channel_means(bottom.data());

    // Sanity: at least one channel pair must differ by more than the
    // CROSS_PATH_MEAN_TOLERANCE between any two of these positions on
    // the blueprint fixture. If this assertion fails, blueprint.pdf has
    // become more uniform (e.g. fixture replaced) and the regression
    // tests above lose their teeth — re-evaluate fixture choice.
    let max_top_bottom = (0..4)
        .map(|i| (top_mean[i] - bottom_mean[i]).abs())
        .fold(0.0_f64, f64::max);
    let max_top_middle = (0..4)
        .map(|i| (top_mean[i] - middle_mean[i]).abs())
        .fold(0.0_f64, f64::max);

    assert!(
        max_top_bottom > common::CROSS_PATH_MEAN_TOLERANCE
            || max_top_middle > common::CROSS_PATH_MEAN_TOLERANCE,
        "blueprint.pdf strip positions are too uniform for cross-path \
         comparisons to discriminate (top={top_mean:?}, middle={middle_mean:?}, \
         bottom={bottom_mean:?}). Replace fixture or revisit test design."
    );
}
