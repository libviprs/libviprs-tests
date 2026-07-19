//! vips-differential golden for signed / wide intermediate arithmetic
//! (issue #282, part of the #270 expert-panel review).
//!
//! The panel confirmed that the three-sample [`PixelFormat`] model
//! (`u8` / `u16` / `f32` only) silently breaks two core op semantics that
//! libvips gets right through its 10-format promotion table:
//!
//! * `subtract(uchar, uchar)` in libvips promotes to **signed short** and
//!   preserves negative differences; the crate's image-image `sub` routed
//!   through the round-and-saturate integer path, so every negative
//!   difference collapsed to `0` (silent data loss). `vips subtract` of the
//!   committed inputs yields values as low as `-254`; the pre-fix `sub`
//!   returned `0` for all of them.
//! * `divide(uchar, uchar)` in libvips promotes to **float** and keeps the
//!   fractional quotient; the crate's `div` already floats (core PRs
//!   #303/#304), so the divide arm here is a regression guard proving the
//!   already-correct float promotion still matches vips exactly.
//!
//! The proportionate fix (issue #282, not the full format redesign of
//! #283/#285) promotes image-image `sub` to a float raster — a
//! signed-capable output matching how a caller consumes vips' signed short
//! result — so negative differences survive. It is RED against the pre-fix
//! core (negatives read back as `0`) and GREEN once `sub` floats.
//!
//! # Fixture regeneration
//!
//! The four CSVs under
//! `tests/fixtures/arithmetic_signed_intermediate_reference_expected/` were
//! produced offline with the vips CLI (libvips 8.18.4), so no libvips is
//! needed at run time:
//!
//! ```text
//! # input_a.csv / input_b.csv: 4x4 single-band, b > a in several cells
//! vips csvload input_a.csv a.v && vips cast a.v a_uc.v uchar
//! vips csvload input_b.csv b.v && vips cast b.v b_uc.v uchar
//! vips subtract a_uc.v b_uc.v sub.v && vips csvsave sub.v subtract_expected.csv
//! vips divide   a_uc.v b_uc.v div.v && vips csvsave div.v divide_expected.csv
//! ```
//!
//! `vips subtract` reports `sub.v` as `short` (signed) and `vips divide`
//! reports `div.v` as `float`, exactly the promotions this test pins.

use libviprs::{PixelFormat, Raster};
use std::path::PathBuf;

/// Per-sample tolerance. The subtract results are exact integers and the
/// divide results are exact `f32` values on both sides (vips writes the
/// `float` result; libviprs reads back its own `f32` samples), so the two
/// agree to well under this bound. `1e-3` matches the tolerance the ported
/// arithmetic tests use while still catching the pre-fix bug, whose negative
/// differences drift by up to `254` units.
const TOLERANCE: f64 = 1e-3;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arithmetic_signed_intermediate_reference_expected")
        .join(name)
}

/// Parse a vips `csvsave` grid (tab / whitespace separated, one row per
/// line) into `(width, height, row-major f64 samples)`.
fn load_csv(name: &str) -> (u32, u32, Vec<f64>) {
    let text = std::fs::read_to_string(fixture(name)).expect("read fixture CSV");
    let rows: Vec<Vec<f64>> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.split_whitespace()
                .map(|tok| tok.parse::<f64>().expect("parse CSV sample"))
                .collect()
        })
        .collect();
    let height = rows.len() as u32;
    let width = rows.first().map_or(0, Vec::len) as u32;
    let samples = rows.into_iter().flatten().collect();
    (width, height, samples)
}

/// Load a committed CSV of `0..=255` samples as a single-band
/// [`PixelFormat::Gray8`] raster (the uchar input vips consumed).
fn gray8_input(name: &str) -> Raster {
    let (w, h, samples) = load_csv(name);
    let data: Vec<u8> = samples.iter().map(|&v| v as u8).collect();
    Raster::new(w, h, PixelFormat::Gray8, data).expect("Gray8 input raster")
}

/// Read every sample of `r` in row-major order as `f64`, via the public
/// `getpoint` surface (which reads each format at its own depth).
fn samples(r: &Raster) -> Vec<f64> {
    let (w, h) = (r.width(), r.height());
    let mut out = Vec::with_capacity((w * h) as usize * r.format().channels());
    for y in 0..h {
        for x in 0..w {
            out.extend(r.getpoint(x, y));
        }
    }
    out
}

fn assert_matches_reference(label: &str, actual: &Raster, expected_csv: &str) {
    let (ew, eh, expected) = load_csv(expected_csv);
    assert_eq!(
        (actual.width(), actual.height()),
        (ew, eh),
        "{label}: dimension mismatch"
    );
    let got = samples(actual);
    assert_eq!(
        got.len(),
        expected.len(),
        "{label}: sample-count mismatch (libviprs {}, vips {})",
        got.len(),
        expected.len()
    );
    for (i, (&a, &e)) in got.iter().zip(expected.iter()).enumerate() {
        assert!(
            (a - e).abs() <= TOLERANCE,
            "{label}: sample {i} (px {}): libviprs {a} vs vips {e}, \
             diff {} > {TOLERANCE}",
            i,
            (a - e).abs()
        );
    }
}

/// `sub(uchar, uchar)` must match `vips subtract` pixel-for-pixel, negative
/// differences and all. The pre-fix integer `sub` saturated every negative
/// to `0`, so this is RED against pristine core (e.g. `10 - 200` returned `0`
/// instead of `-190`) and GREEN once image-image `sub` promotes to a float
/// (signed-capable) raster.
#[test]
fn subtract_uchar_preserves_negative_like_vips() {
    let a = gray8_input("input_a.csv");
    let b = gray8_input("input_b.csv");

    let out = a.sub(&b);
    assert!(
        out.format().is_float(),
        "subtract of two uchar images must promote to a signed-capable \
         (float) output, got {:?}",
        out.format()
    );
    assert_matches_reference("subtract", &out, "subtract_expected.csv");
}

/// `div(uchar, uchar)` must match `vips divide` pixel-for-pixel, keeping the
/// fractional quotient (e.g. `10 / 200 = 0.05`, not the pre-float-promotion
/// rounded `0`). Regression guard for the already-correct float divide.
#[test]
fn divide_uchar_keeps_fraction_like_vips() {
    let a = gray8_input("input_a.csv");
    let b = gray8_input("input_b.csv");

    let out = a.div(&b);
    assert!(
        out.format().is_float(),
        "divide of two uchar images must promote to float, got {:?}",
        out.format()
    );
    assert_matches_reference("divide", &out, "divide_expected.csv");
}
