//! Shared test helpers.
//!
//! Each `tests/*.rs` file is compiled as a standalone integration-test
//! binary, so anything they share has to live here under `tests/common/`
//! and be pulled in with `mod common;` at the top of the test file.
//! Cargo treats `tests/common/mod.rs` (directory module) specially — it
//! isn't compiled as its own integration target.
//!
//! The canonical-fixture loader sits in [`fixtures`]. Below it: PDF /
//! pdfium-specific shared helpers for the streaming-mode integration
//! tests. The pdfium helpers are gated behind the `pdfium` feature so
//! the module compiles cleanly with the default feature set.
//!
//! `#[allow(dead_code)]` on the helpers is intentional — not every
//! consumer uses every helper, and Cargo would otherwise emit a
//! dead-code warning per unused symbol per test binary.

#![allow(dead_code)]

pub mod cli;
pub mod dzsave_expected;
pub mod fixtures;

#[cfg(feature = "pdfium")]
#[allow(unused_imports)]
// Tests that don't use any pdfium helpers see this re-export as unused.
pub use pdfium_helpers::*;

#[cfg(feature = "pdfium")]
mod pdfium_helpers {
    use libviprs::Raster;

    // -----------------------------------------------------------------------
    // Fixture paths
    // -----------------------------------------------------------------------

    pub const FIXTURE_BLUEPRINT: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");
    pub const FIXTURE_PORTRAIT: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/blueprint-portrait.pdf"
    );
    pub const FIXTURE_MIX: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/blueprint-mix.pdf"
    );

    pub const FIXTURES: &[&str] = &[FIXTURE_BLUEPRINT, FIXTURE_PORTRAIT, FIXTURE_MIX];

    // -----------------------------------------------------------------------
    // Tolerances
    // -----------------------------------------------------------------------

    /// Per-channel mean tolerance for region-correctness comparisons within
    /// a single pdfium render path (matrix vs matrix, or form-data vs
    /// form-data).
    ///
    /// Origin: empirically measured in `streaming_pdfium_matrix.rs` (see its
    /// rationale comment lines 13-22). pdfium's matrix path is
    /// bitmap-size-dependent in its anti-aliasing — strip(K, h) and
    /// full[K..K+h] can have up to 15 % of pixels differing with channel
    /// deltas up to 172/255 — but the **mean** over a strip stays under
    /// 1.0 unit on our blueprint fixtures. ±2.0 leaves comfortable headroom
    /// while still catching gross matrix errors (wrong rotation,
    /// off-by-page-size shift, transposed scale produce mean drift in the
    /// tens).
    pub const REGION_MEAN_TOLERANCE: f64 = 2.0;

    /// Per-channel mean tolerance for **cross-path** comparisons (form-data
    /// vs matrix). pdfium's two render paths use independent rasterisers;
    /// the AA divergence between them is real. Empirically up to 5 mean
    /// units drift on the blueprint fixtures (see the failed first-pass
    /// synthetic rotations test, where /Rotate 270 full-page diverged by
    /// 5.59). ±10.0 absorbs that without losing the ability to catch a
    /// wrong-region bug, which produces drift in the tens.
    pub const CROSS_PATH_MEAN_TOLERANCE: f64 = 10.0;

    /// Pixel drift tolerance on dimensions reported by the source. pdfium's
    /// internal scaler rounds at high DPI; +/- 4 px is observed empirically
    /// at 150 and 300 DPI between `set_target_width` and `(pts * scale) as u32`
    /// truncation paths.
    pub const DIM_DRIFT_TOLERANCE_PX: i64 = 4;

    // -----------------------------------------------------------------------
    // Channel-mean helpers
    // -----------------------------------------------------------------------

    /// Per-channel mean over an RGBA8 byte buffer. Returns f64s to keep the
    /// running sum precise across million-pixel strips.
    #[must_use]
    pub fn channel_means(data: &[u8]) -> [f64; 4] {
        assert!(
            data.len() % 4 == 0,
            "RGBA8 byte length must be a multiple of 4 (got {})",
            data.len()
        );
        let mut sums = [0u64; 4];
        let pixels = data.len() / 4;
        for chunk in data.chunks_exact(4) {
            for i in 0..4 {
                sums[i] += u64::from(chunk[i]);
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

    /// Asserts two RGBA8 byte buffers cover the same page region within
    /// [`REGION_MEAN_TOLERANCE`]. Use for matrix-vs-matrix or form-data-vs-
    /// form-data comparisons.
    pub fn assert_same_region(label: &str, actual: &[u8], reference: &[u8]) {
        assert_same_region_with_tolerance(label, actual, reference, REGION_MEAN_TOLERANCE);
    }

    /// Asserts two RGBA8 byte buffers cover the same page region within
    /// [`CROSS_PATH_MEAN_TOLERANCE`]. Use when comparing across pdfium's
    /// two render paths (cached form-data vs streaming matrix).
    pub fn assert_same_region_cross_path(label: &str, actual: &[u8], reference: &[u8]) {
        assert_same_region_with_tolerance(label, actual, reference, CROSS_PATH_MEAN_TOLERANCE);
    }

    /// General mean-tolerance comparison. Prefer the typed wrappers above
    /// over passing custom tolerances per call site — calibrate the
    /// constant once, link the empirical justification, reuse.
    pub fn assert_same_region_with_tolerance(
        label: &str,
        actual: &[u8],
        reference: &[u8],
        tolerance: f64,
    ) {
        assert_eq!(
            actual.len(),
            reference.len(),
            "{label}: byte-length mismatch (actual={}, reference={})",
            actual.len(),
            reference.len(),
        );
        let a = channel_means(actual);
        let b = channel_means(reference);
        let max = (0..4).map(|i| (a[i] - b[i]).abs()).fold(0.0_f64, f64::max);
        assert!(
            max <= tolerance,
            "{label}: per-channel mean drift {max:.3} > {tolerance:.3} \
             (actual={a:?}, reference={b:?})"
        );
    }

    // -----------------------------------------------------------------------
    // Raster cropping
    // -----------------------------------------------------------------------

    /// Extract the top-left `(w, h)` rect of an RGBA8 raster as a flat byte
    /// slice, copying row-by-row to honour stride. Used when comparing
    /// rasters whose reported widths drift by a few pixels
    /// (DIM_DRIFT_TOLERANCE_PX territory).
    #[must_use]
    pub fn crop_top_left(r: &Raster, w: u32, h: u32) -> Vec<u8> {
        assert!(
            w <= r.width() && h <= r.height(),
            "crop dims {}x{} must fit raster {}x{}",
            w,
            h,
            r.width(),
            r.height(),
        );
        let row_bytes = (r.width() * 4) as usize;
        let crop_row_bytes = (w * 4) as usize;
        let mut out = Vec::with_capacity(crop_row_bytes * h as usize);
        for y in 0..h as usize {
            let src = &r.data()[y * row_bytes..y * row_bytes + crop_row_bytes];
            out.extend_from_slice(src);
        }
        out
    }
}
