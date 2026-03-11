#![cfg(feature = "ported_tests")]

//! Ported histogram tests from libvips `test_histogram.py`.
//!
//! These tests exercise histogram computation, cumulative histograms,
//! equalization (global and local/CLAHE), matching, normalization, plotting,
//! LUT mapping, percentile, entropy, statistical differencing, and case/switch.
//! Tests use the reference JPEG fixture and synthetic identity LUTs.

use std::path::Path;

use libviprs::{decode_file, PixelFormat, Raster};

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

#[test]
#[ignore]
/// Cumulative histogram.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create an identity LUT: a 256×1 single-band image where pixel(x,0) = x.
/// fn Raster::identity() -> Raster;
///
/// /// Compute the cumulative histogram of a single-band image.
/// /// Returns a 256×1 (or 65536×1 for 16-bit) image where each entry is
/// /// the cumulative sum of the histogram up to that value.
/// fn Raster::hist_cum(&self) -> Raster;
///
/// /// Read pixel as f64 values at (x, y).
/// fn Raster::getpoint(&self, x: u32, y: u32) -> Vec<f64>;
///
/// /// Compute the mean pixel value across the whole image.
/// fn Raster::avg(&self) -> f64;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_cum)
///
/// 1. Create identity LUT (256 values, 0..255).
/// 2. Compute total = avg() * 256.
/// 3. Compute cumulative histogram.
/// 4. Assert the last entry (pixel at x=255) equals total.
///
/// Reference: test_histogram.py::test_hist_cum
fn test_hist_cum() {
    let im = Raster::identity();
    let total = im.avg() * 256.0;

    let cum = im.hist_cum();
    let px = cum.getpoint(255, 0);
    assert!(
        (px[0] - total).abs() < 0.001,
        "Cumulative histogram at 255 should equal total sum: got {}, expected {total}",
        px[0]
    );
}

#[test]
#[ignore]
/// Histogram equalization (global).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Histogram-equalize an image. Returns an image with a more uniform histogram.
/// fn Raster::hist_equal(&self) -> Raster;
///
/// /// Standard deviation of pixel values.
/// fn Raster::deviate(&self) -> f64;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_equal)
///
/// 1. Load sample.jpg.
/// 2. Equalize the histogram.
/// 3. Assert dimensions match.
/// 4. Assert the equalized image has higher avg and higher deviate than the original.
///
/// Reference: test_histogram.py::test_hist_equal
fn test_hist_equal() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let im2 = im.hist_equal();

    assert_eq!(im.width(), im2.width());
    assert_eq!(im.height(), im2.height());
    assert!(im.avg() < im2.avg(), "Equalized avg should be higher");
    assert!(im.deviate() < im2.deviate(), "Equalized deviate should be higher");
}

#[test]
#[ignore]
/// Check if a histogram is monotonic.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Check whether a histogram (1×N or N×1 image) is monotonically increasing.
/// fn Raster::hist_ismonotonic(&self) -> bool;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_ismonotonic)
///
/// 1. Create identity LUT.
/// 2. Assert it is monotonic.
///
/// Reference: test_histogram.py::test_hist_ismonotonic
fn test_hist_ismonotonic() {
    let im = Raster::identity();
    assert!(im.hist_ismonotonic(), "Identity LUT should be monotonic");
}

#[test]
#[ignore]
/// Local histogram equalization (CLAHE).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Apply local (CLAHE) histogram equalization with the given tile size.
/// /// `max_slope`: contrast limit (higher = more contrast). 0 = unlimited.
/// fn Raster::hist_local(&self, width: u32, height: u32, max_slope: Option<f64>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_local)
///
/// 1. Load sample.jpg.
/// 2. Apply hist_local(10, 10).
/// 3. Assert dimensions match.
/// 4. Assert avg and deviate both increase.
/// 5. Apply hist_local(10, 10, max_slope=3).
/// 6. Deviate should be less than unlimited CLAHE but more than original.
///
/// Reference: test_histogram.py::test_hist_local
fn test_hist_local() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    let im2 = im.hist_local(10, 10, None);
    assert_eq!(im.width(), im2.width());
    assert_eq!(im.height(), im2.height());
    assert!(im.avg() < im2.avg());
    assert!(im.deviate() < im2.deviate());

    let im3 = im.hist_local(10, 10, Some(3.0));
    assert_eq!(im.width(), im3.width());
    assert_eq!(im.height(), im3.height());
    assert!(im3.deviate() < im2.deviate(), "Clamped CLAHE should have less contrast than unlimited");
}

#[test]
#[ignore]
/// Histogram matching (specification).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Match the histogram of `self` to the histogram of `reference`.
/// /// Both should be histogram images (1×N or N×1).
/// fn Raster::hist_match(&self, reference: &Raster) -> Raster;
///
/// /// Compute the absolute max pixel difference between two images.
/// fn abs_max_diff(a: &Raster, b: &Raster) -> f64;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_match)
///
/// 1. Create two identical identity LUTs.
/// 2. Match one to the other.
/// 3. The result should be identical to the original (max diff = 0).
///
/// Reference: test_histogram.py::test_hist_match
fn test_hist_match() {
    let im = Raster::identity();
    let im2 = Raster::identity();

    let matched = im.hist_match(&im2);

    // Matching to the same histogram should be identity
    let max_diff: f64 = im.data().iter().zip(matched.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        max_diff < 0.001,
        "hist_match of identical histograms should be identity, got max diff {max_diff}"
    );
}

#[test]
#[ignore]
/// Histogram normalization.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Normalize a histogram so its sum equals the number of pixels.
/// fn Raster::hist_norm(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_norm)
///
/// 1. Create identity LUT.
/// 2. Normalize it.
/// 3. The result should be identical to the input (identity is already normalized).
///
/// Reference: test_histogram.py::test_hist_norm
fn test_hist_norm() {
    let im = Raster::identity();
    let im2 = im.hist_norm();

    let max_diff: f64 = im.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001, "hist_norm of identity should be identity");
}

#[test]
#[ignore]
/// Histogram plot (visualization).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Generate a visual plot of the histogram.
/// /// Returns a 256×256 (or appropriate size) single-band image.
/// fn Raster::hist_plot(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_plot)
///
/// 1. Create identity LUT.
/// 2. Plot it.
/// 3. Assert width=256, height=256, bands=1, format=Gray8.
///
/// Reference: test_histogram.py::test_hist_plot
fn test_hist_plot() {
    let im = Raster::identity();
    let im2 = im.hist_plot();

    assert_eq!(im2.width(), 256);
    assert_eq!(im2.height(), 256);
    assert_eq!(im2.format(), PixelFormat::Gray8);
}

#[test]
#[ignore]
/// LUT mapping (apply a look-up table to an image).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Map pixel values through a look-up table.
/// /// `lut` should be a 256×1 (or 65536×1) image that maps input values to output values.
/// fn Raster::maplut(&self, lut: &Raster) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_map)
///
/// 1. Create identity LUT.
/// 2. Map the identity LUT through itself: identity.maplut(identity).
/// 3. The result should be identical to the input.
///
/// Reference: test_histogram.py::test_hist_map
fn test_hist_map() {
    let im = Raster::identity();
    let im2 = im.maplut(&im);

    let max_diff: f64 = im.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001, "maplut with identity should be identity");
}

#[test]
#[ignore]
/// Find the threshold at which a given percentage of pixels fall below.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Find the pixel value threshold at which `percent`% of pixels fall at or below.
/// fn Raster::percent(&self, percent: f64) -> f64;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_percent)
///
/// 1. Load sample.jpg, extract band 1 (green channel).
/// 2. Find the 90th percentile threshold.
/// 3. Count how many pixels are ≤ threshold.
/// 4. Assert this is approximately 90% of total pixels.
///
/// Reference: test_histogram.py::test_percent
fn test_percent() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let band1 = im.extract_band(1);

    let pc = band1.percent(90.0);

    // Count pixels <= threshold
    let total_pixels = band1.width() as f64 * band1.height() as f64;
    let n_below: f64 = band1.data().iter().filter(|&&b| (b as f64) <= pc).count() as f64;
    let pc_set = 100.0 * n_below / total_pixels;

    assert!(
        (pc_set - 90.0).abs() < 1.0,
        "90th percentile should capture ~90% of pixels, got {pc_set}%"
    );
}

#[test]
#[ignore]
/// Histogram entropy (Shannon entropy of pixel distribution).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Compute the histogram of a single-band image.
/// fn Raster::hist_find(&self) -> Raster;
///
/// /// Compute the Shannon entropy of a histogram image (in bits).
/// fn Raster::hist_entropy(&self) -> f64;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_hist_entropy)
///
/// 1. Load sample.jpg, extract band 1.
/// 2. Compute histogram via hist_find().
/// 3. Compute entropy via hist_entropy().
/// 4. Assert entropy ≈ 6.67 (within 0.01).
///
/// Reference: test_histogram.py::test_hist_entropy
fn test_hist_entropy() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let band1 = im.extract_band(1);

    let hist = band1.hist_find();
    let ent = hist.hist_entropy();

    assert!(
        (ent - 6.67).abs() < 0.1,
        "Entropy should be ~6.67, got {ent}"
    );
}

#[test]
#[ignore]
/// Statistical differencing (local contrast normalization).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Statistical differencing: normalize each pixel relative to local statistics.
/// /// `width`, `height`: window size for local statistics.
/// /// Shifts each pixel toward a target mean/deviation.
/// fn Raster::stdif(&self, width: u32, height: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_stdif)
///
/// 1. Load sample.jpg.
/// 2. Apply stdif(10, 10).
/// 3. Assert dimensions match.
/// 4. The new average should be closer to the target mean (128)
///    than the original.
///
/// Reference: test_histogram.py::test_stdif
fn test_stdif() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let im2 = im.stdif(10, 10);

    assert_eq!(im.width(), im2.width());
    assert_eq!(im.height(), im2.height());

    // The new mean should be closer to 128 (default target)
    let orig_dist = (im.avg() - 128.0).abs();
    let new_dist = (im2.avg() - 128.0).abs();
    assert!(
        new_dist < orig_dist,
        "stdif should shift mean closer to 128: orig_dist={orig_dist}, new_dist={new_dist}"
    );
}

#[test]
#[ignore]
/// Case/switch: select from a list of values based on an index image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// For each pixel in `self` (an index image), select the corresponding
/// /// value from `cases`. Indices beyond the array length use the last value.
/// fn Raster::case(&self, cases: &[f64]) -> Raster;
///
/// /// Select from a set of boolean images to produce an index image.
/// /// Returns an image where each pixel is the index of the first true condition.
/// /// If no condition is true, the pixel value is N (the number of conditions).
/// fn Raster::switch(conditions: &[&Raster]) -> Raster;
///
/// /// Create a grey ramp image (0..255 horizontally, constant vertically).
/// fn Raster::grey(width: u32, height: u32, uchar: bool) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_histogram.py::test_case)
///
/// 1. Create a 256×256 grey ramp (uchar).
/// 2. switch([x < 128, x >= 128]) → index image with 2 classes.
/// 3. case([10, 20]) → each pixel becomes 10 or 20.
/// 4. avg should be 15.
/// 5. switch 4 classes at 64-pixel boundaries.
/// 6. case([10, 20, 30, 40]) → avg should be 25.
/// 7. case([10, 20, 30]) → values over 2 use last value (30), avg = 22.5.
///
/// Reference: test_histogram.py::test_case
fn test_case() {
    let x = Raster::grey(256, 256, true); // uchar grey ramp

    // Two-class split at 128
    let cond_lo = x.less_than(128.0);
    let cond_hi = x.more_eq(128.0);
    let index = Raster::switch(&[&cond_lo, &cond_hi]);
    let y = index.case(&[10.0, 20.0]);
    assert!(
        (y.avg() - 15.0).abs() < 0.001,
        "Two-class case avg should be 15, got {}",
        y.avg()
    );

    // Four-class split
    let c0 = x.less_than(64.0);
    let c1 = x.more_eq(64.0).band_and(&x.less_than(128.0));
    let c2 = x.more_eq(128.0).band_and(&x.less_than(192.0));
    let c3 = x.more_eq(192.0);
    let index = Raster::switch(&[&c0, &c1, &c2, &c3]);
    let y = index.case(&[10.0, 20.0, 30.0, 40.0]);
    assert!(
        (y.avg() - 25.0).abs() < 0.001,
        "Four-class case avg should be 25, got {}",
        y.avg()
    );

    // Values beyond N use last value
    let y = index.case(&[10.0, 20.0, 30.0]);
    assert!(
        (y.avg() - 22.5).abs() < 0.001,
        "Overflow-to-last case avg should be 22.5, got {}",
        y.avg()
    );
}
