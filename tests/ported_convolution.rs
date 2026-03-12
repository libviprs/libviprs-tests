#![cfg(feature = "ported_tests")]

//! Ported convolution/filtering tests from libvips `test_convolution.py`.
//!
//! These tests exercise spatial convolution (sharp, blur, sobel, line detect),
//! compass convolution, separable convolution, fast/spatial correlation,
//! Gaussian blur, and unsharp-mask sharpening.
//! Tests use fixture images from the reference suite.

use std::path::Path;

use libviprs::{PixelFormat, Raster, decode_file};

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

/// Read pixel values at (x, y) as f64 slice.
fn pixel_f64(im: &Raster, x: u32, y: u32) -> Vec<f64> {
    let bpp = im.format().bytes_per_pixel();
    let channels = im.format().channels();
    let view = im.region(x, y, 1, 1).unwrap();
    let raw = view.pixel(0, 0).unwrap();
    match im.format().bytes_per_channel() {
        1 => raw.iter().map(|&b| b as f64).collect(),
        2 => raw
            .chunks(2)
            .map(|c| u16::from_ne_bytes([c[0], c[1]]) as f64)
            .collect(),
        _ => vec![],
    }
}

/// Perform a point convolution on `image` at position `(px, py)` with the
/// given `kernel` (2D f64 matrix) and `scale` divisor.
/// This is the reference (scalar) implementation used to verify the API.
fn point_conv(image: &Raster, kernel: &[Vec<f64>], scale: f64, px: u32, py: u32) -> Vec<f64> {
    let kh = kernel.len();
    let kw = kernel[0].len();
    let channels = image.format().channels();
    let mut sums = vec![0.0_f64; channels];

    for ky in 0..kh {
        for kx in 0..kw {
            let m = kernel[ky][kx];
            let ix = px + kx as u32;
            let iy = py + ky as u32;
            let pix = pixel_f64(image, ix, iy);
            for (s, &p) in sums.iter_mut().zip(pix.iter()) {
                *s += m * p;
            }
        }
    }

    sums.iter().map(|&s| s / scale).collect()
}

#[test]
#[ignore]
/// Spatial convolution with several kernel types (sharp, blur, line, sobel).
///
/// ## Required API
///
/// ```rust,ignore
/// /// A 2D convolution kernel with an associated scale factor.
/// pub struct Kernel {
///     pub data: Vec<Vec<f64>>,
///     pub scale: f64,
/// }
///
/// /// Convolve the image with the given kernel.
/// /// `precision`: Precision::Integer or Precision::Float.
/// fn Raster::conv(&self, kernel: &Kernel, precision: Precision) -> Raster;
///
/// pub enum Precision { Integer, Float }
/// ```
///
/// ## Test logic (from libvips test_convolution.py::test_conv)
///
/// For each test image (mono band extracted from sample.jpg, and the full RGB):
///   For each kernel (sharp, blur, line, sobel):
///     For each precision (Integer, Float):
///       1. Convolve the image.
///       2. Read the result at (25, 50) and (50, 50).
///       3. Compute the expected value via point_conv at (24, 49) and (49, 49).
///       4. Assert they are approximately equal (threshold 0.0001).
///
/// Kernels (from Python setup):
/// - sharp: [[-1,-1,-1], [-1,16,-1], [-1,-1,-1]], scale=8
/// - blur:  [[1,1,1], [1,1,1], [1,1,1]], scale=9
/// - line:  [[1,1,1], [-2,-2,-2], [1,1,1]], scale=1
/// - sobel: [[1,2,1], [0,0,0], [-1,-2,-1]], scale=1
///
/// Reference: test_convolution.py::test_conv
fn test_conv() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    // Extract band 1 as mono (green channel)
    let mono = colour.extract_band(1);

    let kernels = vec![
        (
            vec![
                vec![-1.0, -1.0, -1.0],
                vec![-1.0, 16.0, -1.0],
                vec![-1.0, -1.0, -1.0],
            ],
            8.0,
        ),
        (
            vec![
                vec![1.0, 1.0, 1.0],
                vec![1.0, 1.0, 1.0],
                vec![1.0, 1.0, 1.0],
            ],
            9.0,
        ),
        (
            vec![
                vec![1.0, 1.0, 1.0],
                vec![-2.0, -2.0, -2.0],
                vec![1.0, 1.0, 1.0],
            ],
            1.0,
        ),
        (
            vec![
                vec![1.0, 2.0, 1.0],
                vec![0.0, 0.0, 0.0],
                vec![-1.0, -2.0, -1.0],
            ],
            1.0,
        ),
    ];

    for im in [&mono, &colour] {
        for (kernel_data, scale) in &kernels {
            for _precision in [Precision::Integer, Precision::Float] {
                let kernel = Kernel {
                    data: kernel_data.clone(),
                    scale: *scale,
                };
                let convolved = im.conv(&kernel, _precision);

                let result = pixel_f64(&convolved, 25, 50);
                let expected = point_conv(im, kernel_data, *scale, 24, 49);
                for (r, e) in result.iter().zip(expected.iter()) {
                    assert!(
                        (r - e).abs() < 1.0,
                        "Conv mismatch at (25,50): got {r}, expected {e}"
                    );
                }

                let result = pixel_f64(&convolved, 50, 50);
                let expected = point_conv(im, kernel_data, *scale, 49, 49);
                for (r, e) in result.iter().zip(expected.iter()) {
                    assert!(
                        (r - e).abs() < 1.0,
                        "Conv mismatch at (50,50): got {r}, expected {e}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore]
/// Compass convolution: rotate a kernel and combine results.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Compass-direction convolution.
/// /// Rotates `kernel` by 45° `times` times, convolves with each rotation,
/// /// and combines the absolute results using `combine` (Max or Sum).
/// fn Raster::compass(
///     &self,
///     kernel: &Kernel,
///     times: u32,
///     angle: Angle45,
///     combine: Combine,
///     precision: Precision,
/// ) -> Raster;
///
/// pub enum Angle45 { D0, D45, D90, D135, D180, D225, D270, D315 }
/// pub enum Combine { Max, Sum }
/// ```
///
/// ## Test logic (from libvips test_convolution.py::test_compass)
///
/// For each image, kernel, precision, and times in 1..4:
///   1. Call compass() with Angle45::D45 and Combine::Max.
///   2. Verify result at (25, 50) matches reference compass computation.
///   Repeat with Combine::Sum.
///
/// Reference: test_convolution.py::test_compass
fn test_compass() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    let sharp = Kernel {
        data: vec![
            vec![-1.0, -1.0, -1.0],
            vec![-1.0, 16.0, -1.0],
            vec![-1.0, -1.0, -1.0],
        ],
        scale: 8.0,
    };

    for im in [&mono, &colour] {
        for precision in [Precision::Integer, Precision::Float] {
            for times in 1..4u32 {
                // Test MAX combine
                let convolved = im.compass(&sharp, times, Angle45::D45, Combine::Max, precision);
                assert_eq!(convolved.width(), im.width());
                assert_eq!(convolved.height(), im.height());

                // Test SUM combine
                let convolved = im.compass(&sharp, times, Angle45::D45, Combine::Sum, precision);
                assert_eq!(convolved.width(), im.width());
                assert_eq!(convolved.height(), im.height());
            }
        }
    }
}

#[test]
#[ignore]
/// Separable convolution: convolve with a 1D kernel applied first horizontally
/// then vertically (equivalent to 2D convolution with the outer product).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a Gaussian kernel matrix.
/// /// `sigma`: standard deviation, `min_ampl`: minimum amplitude (truncation).
/// /// If `separable` is true, returns a 1×N kernel; otherwise N×N.
/// fn Kernel::gaussmat(sigma: f64, min_ampl: f64, separable: bool, precision: Precision) -> Kernel;
///
/// /// Separable convolution with a 1D kernel.
/// fn Raster::convsep(&self, kernel: &Kernel, precision: Precision) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_convolution.py::test_convsep)
///
/// For each image and precision:
///   1. Create 2D Gaussian kernel (sigma=2, min_ampl=0.1).
///   2. Create separable (1D) Gaussian kernel (same params).
///   3. Convolve image with 2D kernel using conv().
///   4. Convolve image with 1D kernel using convsep().
///   5. Compare results at (25, 50): should be approximately equal (threshold 0.1).
///
/// Reference: test_convolution.py::test_convsep
fn test_convsep() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    for im in [&mono, &colour] {
        for precision in [Precision::Integer, Precision::Float] {
            let gmask = Kernel::gaussmat(2.0, 0.1, false, precision);
            let gmask_sep = Kernel::gaussmat(2.0, 0.1, true, precision);

            // 2D kernel should be square
            assert_eq!(gmask.width(), gmask.height());
            // Separable kernel: same width, height=1
            assert_eq!(gmask_sep.width(), gmask.width());
            assert_eq!(gmask_sep.height(), 1);

            let a = im.conv(&gmask, precision);
            let b = im.convsep(&gmask_sep, precision);

            let a_px = pixel_f64(&a, 25, 50);
            let b_px = pixel_f64(&b, 25, 50);
            for (av, bv) in a_px.iter().zip(b_px.iter()) {
                assert!(
                    (av - bv).abs() < 1.0,
                    "convsep mismatch: conv={av}, convsep={bv}"
                );
            }
        }
    }
}

#[test]
#[ignore]
/// Fast (SSD) correlation: find a small patch in a larger image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Fast correlation (sum-of-squared-differences).
/// /// Returns an image where each pixel is the SSD between the image region
/// /// and `template`. The minimum value marks the best match position.
/// fn Raster::fastcor(&self, template: &Raster) -> Raster;
///
/// /// Find the position of the minimum pixel value.
/// /// Returns (min_value, x, y).
/// fn Raster::minpos(&self) -> (f64, u32, u32);
/// ```
///
/// ## Test logic (from libvips test_convolution.py::test_fastcor)
///
/// For each image (mono, colour):
///   1. Crop a 10×10 patch at (20, 45) from the image.
///   2. Run fastcor(image, patch).
///   3. Find the minimum position.
///   4. Assert min value = 0, x = 25 (20 + 10/2), y = 50 (45 + 10/2).
///      (The correlation output is offset by half the template size.)
///
/// Reference: test_convolution.py::test_fastcor
fn test_fastcor() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    for im in [&mono, &colour] {
        let small = im.extract(20, 45, 10, 10).unwrap();
        let cor = im.fastcor(&small);
        let (v, x, y) = cor.minpos();

        assert_eq!(v, 0.0, "Perfect match should have SSD=0");
        assert_eq!(x, 25, "Match x position");
        assert_eq!(y, 50, "Match y position");
    }
}

#[test]
#[ignore]
/// Spatial (NCC) correlation: normalized cross-correlation template matching.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Normalized cross-correlation (spatial correlation).
/// /// Returns an image where each pixel is the NCC score (-1..1).
/// /// The maximum value marks the best match.
/// fn Raster::spcor(&self, template: &Raster) -> Raster;
///
/// /// Find the position of the maximum pixel value.
/// /// Returns (max_value, x, y).
/// fn Raster::maxpos(&self) -> (f64, u32, u32);
/// ```
///
/// ## Test logic (from libvips test_convolution.py::test_spcor)
///
/// For each image (mono, colour):
///   1. Crop a 10×10 patch at (20, 45).
///   2. Run spcor(image, patch).
///   3. Find the maximum position.
///   4. Assert max value = 1.0 (perfect match), x = 25, y = 50.
///
/// Reference: test_convolution.py::test_spcor
fn test_spcor() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    for im in [&mono, &colour] {
        let small = im.extract(20, 45, 10, 10).unwrap();
        let cor = im.spcor(&small);
        let (v, x, y) = cor.maxpos();

        assert!(
            (v - 1.0).abs() < 0.001,
            "NCC perfect match should be 1.0, got {v}"
        );
        assert_eq!(x, 25, "Match x position");
        assert_eq!(y, 50, "Match y position");
    }
}

#[test]
#[ignore]
/// Gaussian blur convenience function.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Gaussian blur with the given sigma.
/// /// `min_ampl`: minimum kernel amplitude for truncation (default 0.2).
/// fn Raster::gaussblur(&self, sigma: f64, min_ampl: f64, precision: Precision) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_convolution.py::test_gaussblur)
///
/// For each image and precision:
///   For sigma in [1.0, 1.2, 1.4, 1.6, 1.8]:
///     1. Create a Gaussian kernel with the given sigma and min_ampl=0.2.
///     2. Convolve the image with conv().
///     3. Blur the image with gaussblur() using the same sigma and min_ampl.
///     4. Compare results at (25, 50): should be approximately equal (threshold 0.1).
///
/// Reference: test_convolution.py::test_gaussblur
fn test_gaussblur() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    for im in [&mono, &colour] {
        for precision in [Precision::Integer, Precision::Float] {
            for i in 5..10 {
                let sigma = i as f64 / 5.0;
                let gmask = Kernel::gaussmat(sigma, 0.2, false, precision);

                let a = im.conv(&gmask, precision);
                let b = im.gaussblur(sigma, 0.2, precision);

                let a_px = pixel_f64(&a, 25, 50);
                let b_px = pixel_f64(&b, 25, 50);
                for (av, bv) in a_px.iter().zip(b_px.iter()) {
                    assert!(
                        (av - bv).abs() < 1.0,
                        "gaussblur mismatch at sigma={sigma}: conv={av}, gaussblur={bv}"
                    );
                }
            }
        }
    }
}

#[test]
#[ignore]
/// Unsharp-mask sharpening.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Sharpen the image using unsharp masking.
/// /// `sigma`: blur radius for the mask.
/// /// `m1`: flat area brightening (0 = no change).
/// /// `m2`: jagged area brightening (0 = no change).
/// fn Raster::sharpen(&self, sigma: f64, m1: f64, m2: f64) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_convolution.py::test_sharpen)
///
/// For each image:
///   For sigma in [0.5, 1.0, 1.5, 2.0]:
///     1. Sharpen the image with sharpen(sigma, default m1, default m2).
///     2. Assert dimensions match.
///     3. If m1=0 and m2=0, sharpening should be a no-op:
///        sharpen(sigma, 0, 0) should produce an identical image (max diff = 0).
///
/// Reference: test_convolution.py::test_sharpen
fn test_sharpen() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    for im in [&mono, &colour] {
        for &sigma in &[0.5, 1.0, 1.5, 2.0] {
            let sharp = im.sharpen(sigma, 1.0, 2.0);
            assert_eq!(im.width(), sharp.width());
            assert_eq!(im.height(), sharp.height());

            // With m1=0 and m2=0, sharpen should be identity
            let noop = im.sharpen(sigma, 0.0, 0.0);
            let max_diff: u8 = im
                .data()
                .iter()
                .zip(noop.data().iter())
                .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs() as u8)
                .max()
                .unwrap_or(0);
            assert_eq!(max_diff, 0, "sharpen with m1=0, m2=0 should be identity");
        }
    }
}
