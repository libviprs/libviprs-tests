//! Closes the rotation coverage gap left by
//! `pdfium_streaming_rotation_matrix.rs` — the scanned-blueprint
//! fixtures don't carry `/Rotate 90`, `/Rotate 180` or `/Rotate 270`,
//! so those matrix branches are otherwise unexercised by integration
//! tests. We drive each branch from a checked-in canonical fixture:
//! `canonical.pdf` at `/Rotate 0` plus `canonical_rotated_{90,180,270}.pdf`,
//! all produced offline by `tests/gen_canonicals.rs`. The fixtures are
//! byte-stable inputs, so this test no longer depends on `lopdf`'s save
//! format staying constant across versions, and it never mutates a PDF
//! at test time.
//!
//! # What this test asserts
//!
//! For each `/Rotate R ∈ {0, 90, 180, 270}`:
//!
//! 1. **Construction succeeds.** No panic, no error. Catches catastrophic
//!    matrix or dimension errors at the boundary.
//! 2. **Orientation matches the rotation.** `canonical.pdf` is A4 portrait
//!    (595 × 842 pt) at `/Rotate 0`, so 90 and 270 must report landscape
//!    display dimensions; 0 and 180 stay portrait. A wrong matrix that,
//!    e.g., swaps W and H on the wrong rotations would fail here.
//! 3. **Strip self-consistency.** Stitched strips produced via the
//!    matrix render path equal a single-shot streaming render of the
//!    same page within the existing `REGION_MEAN_TOLERANCE`. This is
//!    the same property the non-synthetic fixtures pin in
//!    `pdfium_streaming_rotation_matrix.rs`; the cross-path divergence
//!    between cached (form-data) and streaming (matrix) is well
//!    understood and not what's under test here.
//! 4. **Top strip aligns with top of full render.** A wrong y-translation
//!    in the matrix `f` term would put `render_strip(0, h)` somewhere
//!    other than rows `[0, h)` of the full render — caught by the
//!    region-mean comparison.
//!
//! # Why the cross-path mean is *not* asserted here
//!
//! pdfium's form-data and matrix render paths use independent
//! rasterisers; their pixel output legitimately differs (see the
//! comment block in `streaming_pdfium_matrix.rs:13-22` and
//! `pdfium_pyramid_engine_parity.rs:33-48`). A cached-vs-streaming mean
//! comparison would conflate that AA divergence with rotation bugs and
//! is the wrong shape for this test.

#![cfg(feature = "pdfium")]

mod common;

use std::path::{Path, PathBuf};

use common::assert_same_region;
use libviprs::streaming::StripSource;
use libviprs::{PdfiumStripSource, Raster};

const DPI: u32 = 72;

/// Path to a checked-in fixture under `tests/fixtures/`.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The checked-in canonical PDF whose page carries `/Rotate rotation`.
/// `/Rotate 0` is the base `canonical.pdf`; the other three are its
/// pre-rotated variants generated offline by `gen_canonicals.rs`.
fn canonical_rotated(rotation: i64) -> PathBuf {
    match rotation {
        0 => fixture("canonical.pdf"),
        90 => fixture("canonical_rotated_90.pdf"),
        180 => fixture("canonical_rotated_180.pdf"),
        270 => fixture("canonical_rotated_270.pdf"),
        other => panic!("no checked-in canonical fixture for /Rotate {other}"),
    }
}

fn streaming_full(path: &Path) -> Raster {
    let s = PdfiumStripSource::new_streaming(path, 1, DPI)
        .unwrap_or_else(|e| panic!("streaming source on {}: {e:?}", path.display()));
    let h = s.height();
    s.render_strip(0, h).expect("streaming full render")
}

fn stitch_streaming(source: &PdfiumStripSource, strip_h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((source.width() * source.height() * 4) as usize);
    let mut y = 0u32;
    while y < source.height() {
        let h = strip_h.min(source.height() - y);
        let strip = source
            .render_strip(y, h)
            .unwrap_or_else(|e| panic!("render_strip({y}, {h}): {e:?}"));
        out.extend_from_slice(strip.data());
        y += h;
    }
    out
}

/// Per-rotation matrix correctness — the four invariants documented at
/// the top of this file.
fn run_for_rotation(rotation: i64) {
    let fixture_path = canonical_rotated(rotation);
    let path: &Path = &fixture_path;

    // (1) Construction succeeds.
    let source = PdfiumStripSource::new_streaming(path, 1, DPI)
        .unwrap_or_else(|e| panic!("/Rotate {rotation}: streaming construction failed: {e:?}"));

    // (2) Orientation matches the rotation. canonical.pdf is portrait at
    // /Rotate 0 (taller than wide). 90 and 270 must rotate it to
    // landscape; 0 and 180 keep portrait.
    let observed_landscape = source.width() > source.height();
    let expected_landscape = matches!(rotation, 90 | 270);
    assert_eq!(
        observed_landscape,
        expected_landscape,
        "/Rotate {rotation}: orientation expected {} got {} ({}x{})",
        if expected_landscape {
            "landscape"
        } else {
            "portrait"
        },
        if observed_landscape {
            "landscape"
        } else {
            "portrait"
        },
        source.width(),
        source.height(),
    );

    // (3) Strip self-consistency — the matrix-path streaming source must
    // produce the same pixels via stitched strips as via a single full
    // render of the same page. A wrong matrix would fail here because
    // the per-strip y-translation `f` must compose consistently.
    let reference = streaming_full(path);
    for &strip_h in &[64u32, 128, 256] {
        let stitched = stitch_streaming(&source, strip_h);
        assert_same_region(
            &format!("/Rotate {rotation} stitched (strip_h={strip_h}) vs streaming-full"),
            &stitched,
            reference.data(),
        );
    }

    // (4) Top strip aligns with top of full render. Pinpoints the matrix
    // y-offset for the y_offset == 0 case in particular, where any sign
    // confusion would otherwise hide behind the stitched-mean check.
    let h = 64u32.min(source.height());
    let strip = source.render_strip(0, h).unwrap();
    let row_bytes = (reference.width() * 4) as usize;
    let top = &reference.data()[..row_bytes * h as usize];
    assert_same_region(
        &format!("/Rotate {rotation} top strip vs full-top"),
        strip.data(),
        top,
    );
}

#[test]
fn rotate_0_synthetic_self_consistent() {
    run_for_rotation(0);
}

#[test]
fn rotate_90_synthetic_self_consistent() {
    run_for_rotation(90);
}

#[test]
fn rotate_180_synthetic_self_consistent() {
    run_for_rotation(180);
}

#[test]
fn rotate_270_synthetic_self_consistent() {
    run_for_rotation(270);
}
