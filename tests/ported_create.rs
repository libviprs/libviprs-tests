#![cfg(feature = "ported_tests")]

//! Ported image-creation tests from libvips `test_create.py`.
//!
//! These tests exercise factory functions that generate synthetic images:
//! black/constant, LUT construction, eye/sines patterns, Gaussian/fractal
//! noise, grey ramps, identity LUTs, matrix operations, frequency-domain
//! masks, text rendering, and procedural noise (Worley/Perlin).
//! All tests use generated images (as the libvips originals do).

use libviprs::{PixelFormat, Raster};

#[test]
#[ignore]
/// Create a black (all-zero) image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a black image with the given dimensions and band count.
/// /// Default format is Gray8 (bands=1) or Rgb8 (bands=3).
/// fn Raster::black(width: u32, height: u32) -> Raster;
/// fn Raster::black_bands(width: u32, height: u32, bands: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_black)
///
/// 1. Create 100×100 single-band black image.
/// 2. Assert: width=100, height=100, format=Gray8, bands=1.
/// 3. Every pixel along the diagonal should be [0].
/// 4. Create 100×100 3-band black image.
/// 5. Assert: bands=3, every diagonal pixel should be [0, 0, 0].
///
/// Reference: test_create.py::test_black
fn test_black() {
    let im = Raster::black(100, 100);
    assert_eq!(im.width(), 100);
    assert_eq!(im.height(), 100);
    assert_eq!(im.format(), PixelFormat::Gray8);
    assert_eq!(im.format().channels(), 1);
    for i in 0..100u32 {
        let px = im.getpoint(i, i);
        assert_eq!(px, vec![0.0], "Pixel at ({i},{i}) should be 0");
    }

    let im = Raster::black_bands(100, 100, 3);
    assert_eq!(im.format().channels(), 3);
    for i in 0..100u32 {
        let px = im.getpoint(i, i);
        assert_eq!(px, vec![0.0, 0.0, 0.0]);
    }
}

#[test]
#[ignore]
/// Build a piece-wise linear LUT from control points.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Build a LUT from a matrix of control points.
/// /// Input: an N×M image (or 2D array) where column 0 is the input value
/// /// and columns 1.. are output band values. Interpolates linearly between points.
/// /// Returns a 256×1 image (or 65536×1 for 16-bit input range).
/// fn Raster::buildlut(control_points: &[Vec<f64>]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_buildlut)
///
/// Two-point LUT:
/// 1. Control points: [[0, 0], [255, 100]].
/// 2. Build LUT → 256×1, 1 band.
/// 3. pixel(0,0) = 0.0, pixel(255,0) = 100.0, pixel(10,0) ≈ 100*10/255.
///
/// Three-point multi-band LUT:
/// 1. Control points: [[0, 0, 100], [255, 100, 0], [128, 10, 90]].
/// 2. Build LUT → 256×1, 2 bands.
/// 3. pixel(0,0) ≈ [0.0, 100.0], pixel(64,0) ≈ [5.0, 95.0].
///
/// Reference: test_create.py::test_buildlut
fn test_buildlut() {
    // Simple two-point LUT
    let lut = Raster::buildlut(&[vec![0.0, 0.0], vec![255.0, 100.0]]);
    assert_eq!(lut.width(), 256);
    assert_eq!(lut.height(), 1);
    assert_eq!(lut.format().channels(), 1);

    let p0 = lut.getpoint(0, 0);
    assert!((p0[0] - 0.0).abs() < 0.001);
    let p255 = lut.getpoint(255, 0);
    assert!((p255[0] - 100.0).abs() < 0.001);
    let p10 = lut.getpoint(10, 0);
    assert!((p10[0] - 100.0 * 10.0 / 255.0).abs() < 0.1);

    // Multi-band LUT
    let lut = Raster::buildlut(&[
        vec![0.0, 0.0, 100.0],
        vec![255.0, 100.0, 0.0],
        vec![128.0, 10.0, 90.0],
    ]);
    assert_eq!(lut.width(), 256);
    assert_eq!(lut.format().channels(), 2);
    let p0 = lut.getpoint(0, 0);
    assert!((p0[0] - 0.0).abs() < 0.1);
    assert!((p0[1] - 100.0).abs() < 0.1);
    let p64 = lut.getpoint(64, 0);
    assert!((p64[0] - 5.0).abs() < 0.5);
    assert!((p64[1] - 95.0).abs() < 0.5);
}

#[test]
#[ignore]
/// Eye (frequency test) pattern.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create an "eye" frequency test image.
/// /// Float output ranges from -1.0 to 1.0. If `uchar` is true, maps to 0..255.
/// fn Raster::eye(width: u32, height: u32, uchar: bool) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_eye)
///
/// Float version: width=100, height=90, bands=1, float format, max=1.0, min=-1.0.
/// Uchar version: same dimensions, uchar format, max=255.0, min=0.0.
///
/// Reference: test_create.py::test_eye
fn test_eye() {
    let im = Raster::eye(100, 90, false);
    assert_eq!(im.width(), 100);
    assert_eq!(im.height(), 90);
    assert_eq!(im.format().channels(), 1);
    assert!((im.max_value() - 1.0).abs() < 0.001);
    assert!((im.min_value() - (-1.0)).abs() < 0.001);

    let im = Raster::eye(100, 90, true);
    assert_eq!(im.format(), PixelFormat::Gray8);
    assert!((im.max_value() - 255.0).abs() < 0.001);
    assert!((im.min_value() - 0.0).abs() < 0.001);
}

#[test]
#[ignore]
/// FFT of a small image (smoke test).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Forward FFT of a real-valued image. Returns a complex image.
/// fn Raster::fwfft(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_fwfft_small_image)
///
/// 1. Create a 2×1 black image.
/// 2. Call fwfft(). Should not panic or error.
///
/// Reference: test_create.py::test_fwfft_small_image
fn test_fwfft_small() {
    let im = Raster::black(2, 1);
    let _fft = im.fwfft(); // Should not panic
}

#[test]
#[ignore]
/// Fractal surface.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Generate a fractal surface with the given fractal dimension.
/// /// Returns a float image.
/// fn Raster::fractsurf(width: u32, height: u32, fractal_dimension: f64) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_fractsurf)
///
/// 1. Create fractsurf(100, 90, 2.5).
/// 2. Assert: width=100, height=90, bands=1, float format.
///
/// Reference: test_create.py::test_fractsurf
fn test_fractsurf() {
    let im = Raster::fractsurf(100, 90, 2.5);
    assert_eq!(im.width(), 100);
    assert_eq!(im.height(), 90);
    assert_eq!(im.format().channels(), 1);
}

#[test]
#[ignore]
/// Gaussian convolution matrix.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a Gaussian convolution matrix with the given sigma and min amplitude.
/// /// If `separable`, returns a 1D kernel (1×N); otherwise N×N.
/// /// The matrix carries a `scale` metadata field.
/// fn Kernel::gaussmat(sigma: f64, min_ampl: f64, separable: bool) -> Kernel;
///
/// /// A convolution kernel with 2D data and a scale factor.
/// pub struct Kernel {
///     pub data: Vec<Vec<f64>>,
///     pub scale: f64,
/// }
/// ```
///
/// ## Test logic (from libvips test_create.py::test_gaussmat)
///
/// 2D kernel (sigma=1, min_ampl=0.1):
/// - width=5, height=5, bands=1.
/// - max value = 20.
/// - sum = avg * width * height = scale.
/// - Center pixel = 20.0.
///
/// Separable kernel (sigma=1, min_ampl=0.1):
/// - width=5, height=1.
/// - max value = 1.0.
/// - Center pixel = 1.0.
///
/// Reference: test_create.py::test_gaussmat
fn test_gaussmat() {
    let k = Kernel::gaussmat(1.0, 0.1, false);
    assert_eq!(k.width(), 5);
    assert_eq!(k.height(), 5);
    assert!((k.max() - 20.0).abs() < 0.001);
    let center = k.data[2][2];
    assert!((center - 20.0).abs() < 0.001);

    let ks = Kernel::gaussmat(1.0, 0.1, true);
    assert_eq!(ks.width(), 5);
    assert_eq!(ks.height(), 1);
    assert!((ks.max() - 1.0).abs() < 0.001);
    let center = ks.data[0][2];
    assert!((center - 1.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Gaussian noise image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a Gaussian noise image with the given mean and sigma.
/// fn Raster::gaussnoise(width: u32, height: u32, mean: f64, sigma: f64) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_gaussnoise)
///
/// 1. Default: gaussnoise(100, 90) → float, bands=1.
/// 2. With sigma=10, mean=100:
///    - deviate ≈ 10 (within 0.4).
///    - avg ≈ 100 (within 0.4).
///
/// Reference: test_create.py::test_gaussnoise
fn test_gaussnoise() {
    let im = Raster::gaussnoise(100, 90, 0.0, 1.0);
    assert_eq!(im.width(), 100);
    assert_eq!(im.height(), 90);
    assert_eq!(im.format().channels(), 1);

    let im = Raster::gaussnoise(100, 90, 100.0, 10.0);
    assert!((im.deviate() - 10.0).abs() < 0.4);
    assert!((im.avg() - 100.0).abs() < 0.4);
}

#[test]
#[ignore]
/// Grey ramp (horizontal gradient).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a horizontal grey ramp from 0.0 (left) to 1.0 (right).
/// /// If `uchar`, maps to 0..255.
/// fn Raster::grey(width: u32, height: u32, uchar: bool) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_grey)
///
/// Float: 100×90, pixel(0,0)=0.0, pixel(99,0)=1.0, all rows identical.
/// Uchar: pixel(0,0)=0, pixel(99,0)=255.
///
/// Reference: test_create.py::test_grey
fn test_grey() {
    let im = Raster::grey(100, 90, false);
    assert_eq!(im.width(), 100);
    assert_eq!(im.height(), 90);
    let p = im.getpoint(0, 0);
    assert!((p[0] - 0.0).abs() < 0.001);
    let p = im.getpoint(99, 0);
    assert!((p[0] - 1.0).abs() < 0.001);

    let im = Raster::grey(100, 90, true);
    assert_eq!(im.format(), PixelFormat::Gray8);
    let p = im.getpoint(0, 0);
    assert_eq!(p[0], 0.0);
    let p = im.getpoint(99, 0);
    assert_eq!(p[0], 255.0);
}

#[test]
#[ignore]
/// Identity LUT.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create an identity look-up table: a 256×1 (or 65536×1 if ushort) image
/// /// where pixel(x,0) = x.
/// fn Raster::identity() -> Raster;
/// fn Raster::identity_ushort() -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_identity)
///
/// 8-bit: width=256, height=1, bands=1, uchar.
///   pixel(0,0)=0, pixel(255,0)=255, pixel(128,0)=128.
/// 16-bit: width=65536, height=1, ushort.
///   pixel(0,0)=0, pixel(99,0)=99, pixel(65535,0)=65535.
///
/// Reference: test_create.py::test_identity
fn test_identity() {
    let im = Raster::identity();
    assert_eq!(im.width(), 256);
    assert_eq!(im.height(), 1);
    assert_eq!(im.format(), PixelFormat::Gray8);
    assert_eq!(im.getpoint(0, 0), vec![0.0]);
    assert_eq!(im.getpoint(255, 0), vec![255.0]);
    assert_eq!(im.getpoint(128, 0), vec![128.0]);

    let im = Raster::identity_ushort();
    assert_eq!(im.width(), 65536);
    assert_eq!(im.height(), 1);
    assert_eq!(im.getpoint(0, 0), vec![0.0]);
    assert_eq!(im.getpoint(65535, 0), vec![65535.0]);
}

#[test]
#[ignore]
/// Invert a LUT (swap axes).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Invert a look-up table: swap input and output so the inverse mapping is produced.
/// fn Raster::invertlut(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_invertlut)
///
/// 1. Create a matrix: [[0.1,0.2,0.3,0.1], [0.2,0.4,0.4,0.2], [0.7,0.5,0.6,0.3]].
/// 2. Invert: width=256, height=1, bands=3, double format.
/// 3. pixel(0,0) ≈ [0,0,0], pixel(255,0) ≈ [1,1,1].
/// 4. pixel(0.2*255,0)[0] ≈ 0.1, pixel(0.3*255,0)[1] ≈ 0.1, pixel(0.1*255,0)[2] ≈ 0.1.
///
/// Reference: test_create.py::test_invertlut
fn test_invertlut() {
    let lut = Raster::from_matrix(&[
        vec![0.1, 0.2, 0.3, 0.1],
        vec![0.2, 0.4, 0.4, 0.2],
        vec![0.7, 0.5, 0.6, 0.3],
    ]);
    let im = lut.invertlut();

    assert_eq!(im.width(), 256);
    assert_eq!(im.height(), 1);
    assert_eq!(im.format().channels(), 3);

    let p = im.getpoint(0, 0);
    for &v in &p { assert!(v.abs() < 0.001); }
    let p = im.getpoint(255, 0);
    for &v in &p { assert!((v - 1.0).abs() < 0.001); }

    let p = im.getpoint((0.2 * 255.0) as u32, 0);
    assert!((p[0] - 0.1).abs() < 0.1);
}

#[test]
#[ignore]
/// Matrix inversion (4×4).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Invert a square matrix image. Returns a double-format image.
/// fn Raster::matrixinvert(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_matrixinvert)
///
/// 1. Create 4×4 matrix: [[4,0,0,0],[0,0,2,0],[0,1,2,0],[1,0,0,1]].
/// 2. Invert: width=4, height=4, bands=1, double.
/// 3. pixel(0,0) = 0.25, pixel(3,3) = 1.0.
///
/// Reference: test_create.py::test_matrixinvert
fn test_matrixinvert() {
    let mat = Raster::from_matrix(&[
        vec![4.0, 0.0, 0.0, 0.0],
        vec![0.0, 0.0, 2.0, 0.0],
        vec![0.0, 1.0, 2.0, 0.0],
        vec![1.0, 0.0, 0.0, 1.0],
    ]);
    let inv = mat.matrixinvert();

    assert_eq!(inv.width(), 4);
    assert_eq!(inv.height(), 4);

    let p = inv.getpoint(0, 0);
    assert!((p[0] - 0.25).abs() < 0.001);
    let p = inv.getpoint(3, 3);
    assert!((p[0] - 1.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Laplacian of Gaussian matrix.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a Laplacian of Gaussian convolution matrix.
/// fn Kernel::logmat(sigma: f64, min_ampl: f64, separable: bool) -> Kernel;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_logmat)
///
/// 2D: sigma=1, min_ampl=0.1 → 7×7, max=20, center=20.
/// Separable: 7×1, max=1.0, center=1.0.
///
/// Reference: test_create.py::test_logmat
fn test_logmat() {
    let k = Kernel::logmat(1.0, 0.1, false);
    assert_eq!(k.width(), 7);
    assert_eq!(k.height(), 7);
    assert!((k.max() - 20.0).abs() < 0.001);
    assert!((k.data[3][3] - 20.0).abs() < 0.001);

    let ks = Kernel::logmat(1.0, 0.1, true);
    assert_eq!(ks.width(), 7);
    assert_eq!(ks.height(), 1);
    assert!((ks.max() - 1.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Butterworth frequency-domain masks (band, ring, and basic).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a Butterworth bandpass mask.
/// fn Raster::mask_butterworth(w: u32, h: u32, order: f64, frequency_cutoff: f64,
///     amplitude_cutoff: f64, nodc: bool, optical: bool, uchar: bool) -> Raster;
///
/// /// Butterworth band-reject/band-pass mask.
/// fn Raster::mask_butterworth_band(w: u32, h: u32, order: f64, freq_cutoff_x: f64,
///     freq_cutoff_y: f64, radius: f64, ampl_cutoff: f64, uchar: bool, optical: bool, nodc: bool) -> Raster;
///
/// /// Butterworth ring mask.
/// fn Raster::mask_butterworth_ring(w: u32, h: u32, order: f64, freq_cutoff: f64,
///     ampl_cutoff: f64, ringwidth: f64, nodc: bool) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_mask_butterworth*)
///
/// For each mask variant:
/// - Assert dimensions, bands=1, float format.
/// - Verify specific pixel values match expected.
///
/// Reference: test_create.py::test_mask_butterworth, test_mask_butterworth_band, test_mask_butterworth_ring
fn test_butterworth() {
    // Basic Butterworth
    let im = Raster::mask_butterworth(128, 128, 2.0, 0.7, 0.1, true, false, false);
    assert_eq!(im.width(), 128);
    assert_eq!(im.height(), 128);
    let p = im.getpoint(0, 0);
    assert!((p[0] - 0.0).abs() < 0.001, "DC should be 0 with nodc");
    let (_, mx, my) = im.maxpos();
    assert_eq!(mx, 64);
    assert_eq!(my, 64);

    // Butterworth band
    let im = Raster::mask_butterworth_band(128, 128, 2.0, 0.5, 0.5, 0.7, 0.1, false, false, false);
    assert_eq!(im.width(), 128);
    assert!((im.max_value() - 1.0).abs() < 0.01);

    // Butterworth ring
    let im = Raster::mask_butterworth_ring(128, 128, 2.0, 0.7, 0.1, 0.5, true);
    assert_eq!(im.width(), 128);
    let p = im.getpoint(45, 0);
    assert!((p[0] - 1.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Fractal frequency-domain mask.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::mask_fractal(w: u32, h: u32, fractal_dimension: f64) -> Raster;
/// ```
///
/// Reference: test_create.py::test_mask_fractal
fn test_mask_fractal() {
    let im = Raster::mask_fractal(128, 128, 2.3);
    assert_eq!(im.width(), 128);
    assert_eq!(im.height(), 128);
    assert_eq!(im.format().channels(), 1);
}

#[test]
#[ignore]
/// Gaussian frequency-domain masks (band, ring, and basic).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::mask_gaussian(w: u32, h: u32, freq_cutoff: f64, ampl_cutoff: f64, nodc: bool) -> Raster;
/// fn Raster::mask_gaussian_band(w: u32, h: u32, freq_x: f64, freq_y: f64, radius: f64, ampl: f64) -> Raster;
/// fn Raster::mask_gaussian_ring(w: u32, h: u32, freq: f64, ampl: f64, ringwidth: f64, nodc: bool) -> Raster;
/// ```
///
/// Reference: test_create.py::test_mask_gaussian*
fn test_gaussian_masks() {
    let im = Raster::mask_gaussian(128, 128, 0.7, 0.1, true);
    assert_eq!(im.width(), 128);
    let p = im.getpoint(0, 0);
    assert!((p[0] - 0.0).abs() < 0.01);

    let im = Raster::mask_gaussian_band(128, 128, 0.5, 0.5, 0.7, 0.1);
    assert!((im.max_value() - 1.0).abs() < 0.01);

    let im = Raster::mask_gaussian_ring(128, 128, 0.7, 0.1, 0.5, true);
    let p = im.getpoint(45, 0);
    assert!((p[0] - 1.0).abs() < 0.01);
}

#[test]
#[ignore]
/// Ideal frequency-domain masks (band, ring, and basic).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::mask_ideal(w: u32, h: u32, freq_cutoff: f64, nodc: bool) -> Raster;
/// fn Raster::mask_ideal_band(w: u32, h: u32, freq_x: f64, freq_y: f64, radius: f64) -> Raster;
/// fn Raster::mask_ideal_ring(w: u32, h: u32, freq: f64, ringwidth: f64, nodc: bool) -> Raster;
/// ```
///
/// Reference: test_create.py::test_mask_ideal*
fn test_ideal_masks() {
    let im = Raster::mask_ideal(128, 128, 0.7, true);
    assert_eq!(im.width(), 128);
    let p = im.getpoint(0, 0);
    assert!((p[0] - 0.0).abs() < 0.01);

    let im = Raster::mask_ideal_band(128, 128, 0.5, 0.5, 0.7);
    assert!((im.max_value() - 1.0).abs() < 0.01);

    let im = Raster::mask_ideal_ring(128, 128, 0.7, 0.5, true);
    let p = im.getpoint(45, 0);
    assert!((p[0] - 1.0).abs() < 0.01);
}

#[test]
#[ignore]
/// Sines pattern image.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::sines(width: u32, height: u32) -> Raster;
/// ```
///
/// Reference: test_create.py::test_sines
fn test_sines() {
    let im = Raster::sines(128, 128);
    assert_eq!(im.width(), 128);
    assert_eq!(im.height(), 128);
    assert_eq!(im.format().channels(), 1);
}

#[test]
#[ignore]
/// Text rendering.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Render text to a single-band image.
/// /// `dpi`: rendering DPI.
/// /// `width`/`height`: optional max dimensions (auto-sizes font).
/// /// `wrap`: optional wrapping mode ("char", "word", etc.).
/// fn Raster::text(text: &str, dpi: Option<u32>, width: Option<u32>, height: Option<u32>, wrap: Option<&str>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_create.py::test_text)
///
/// 1. Render "Hello, world!" at 300 DPI.
/// 2. Assert width > 10, height > 10, bands=1, uchar.
/// 3. Max > 240, min = 0.
/// 4. Auto-fit: render at width=500, height=500 → actual width ≈ 500 (±50).
///
/// Reference: test_create.py::test_text
fn test_text() {
    let im = Raster::text("Hello, world!", Some(300), None, None, None);
    assert!(im.width() > 10);
    assert!(im.height() > 10);
    assert_eq!(im.format().channels(), 1);
    assert_eq!(im.format(), PixelFormat::Gray8);
    assert!(im.max_value() > 240.0);
    assert!((im.min_value() - 0.0).abs() < 0.001);

    // Auto-fit
    let im = Raster::text("Hello, world!", None, Some(500), Some(500), None);
    assert!((im.width() as i32 - 500).abs() < 50);
}

#[test]
#[ignore]
/// Tone curve LUT.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a tone-mapping LUT for printing.
/// /// Returns a 32768×1 ushort image that is monotonic.
/// fn Raster::tonelut() -> Raster;
/// ```
///
/// Reference: test_create.py::test_tonelut
fn test_tonelut() {
    let im = Raster::tonelut();
    assert_eq!(im.format().channels(), 1);
    assert_eq!(im.width(), 32768);
    assert_eq!(im.height(), 1);
    assert!(im.hist_ismonotonic());
}

#[test]
#[ignore]
/// Coordinate (XYZ) image: a 2-band image where pixel(x,y) = [x, y].
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a coordinate image where band 0 = column index, band 1 = row index.
/// fn Raster::xyz(width: u32, height: u32) -> Raster;
/// ```
///
/// Reference: test_create.py::test_xyz
fn test_xyz() {
    let im = Raster::xyz(128, 128);
    assert_eq!(im.format().channels(), 2);
    assert_eq!(im.width(), 128);
    assert_eq!(im.height(), 128);
    let p = im.getpoint(45, 35);
    assert!((p[0] - 45.0).abs() < 0.001);
    assert!((p[1] - 35.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Signed distance field (SDF) images.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create an SDF image for the given shape.
/// /// `shape`: "circle", "box", "rounded-box", "line".
/// /// Shape parameters vary by type.
/// fn Raster::sdf(width: u32, height: u32, shape: &str, params: SdfParams) -> Raster;
/// ```
///
/// Reference: test_create.py::test_sdf
fn test_sdf() {
    // Circle SDF
    let im = Raster::sdf(128, 128, "circle", &SdfParams { a: [64, 64], r: Some(32), ..Default::default() });
    assert_eq!(im.width(), 128);
    let p = im.getpoint(45, 35);
    assert!((p[0] - 2.670).abs() < 0.1);

    // Box SDF
    let im = Raster::sdf(128, 128, "box", &SdfParams { a: [10, 10], b: Some([50, 40]), ..Default::default() });
    let p = im.getpoint(45, 35);
    assert!((p[0] - (-5.0)).abs() < 0.1);

    // Line SDF
    let im = Raster::sdf(128, 128, "line", &SdfParams { a: [10, 10], b: Some([50, 40]), ..Default::default() });
    let p = im.getpoint(45, 35);
    assert!((p[0] - 1.0).abs() < 0.1);
}

#[test]
#[ignore]
/// Zone plate pattern.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::zone(width: u32, height: u32) -> Raster;
/// ```
///
/// Reference: test_create.py::test_zone
fn test_zone() {
    let im = Raster::zone(128, 128);
    assert_eq!(im.width(), 128);
    assert_eq!(im.height(), 128);
    assert_eq!(im.format().channels(), 1);
}

#[test]
#[ignore]
/// Worley and Perlin procedural noise.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Generate Worley (cellular) noise.
/// fn Raster::worley(width: u32, height: u32) -> Raster;
///
/// /// Generate Perlin (gradient) noise.
/// fn Raster::perlin(width: u32, height: u32) -> Raster;
/// ```
///
/// Reference: test_create.py::test_worley, test_perlin
fn test_worley_perlin() {
    let im = Raster::worley(512, 512);
    assert_eq!(im.width(), 512);
    assert_eq!(im.height(), 512);
    assert_eq!(im.format().channels(), 1);

    let im = Raster::perlin(512, 512);
    assert_eq!(im.width(), 512);
    assert_eq!(im.height(), 512);
    assert_eq!(im.format().channels(), 1);
}
