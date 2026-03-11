#![cfg(feature = "ported_tests")]

//! Ported resampling tests from libvips `test_resample.py`.
//!
//! Covers resize, shrink, affine transforms, similarity transforms,
//! arbitrary rotation, reduce with kernel selection, thumbnail, and mapim.
//! Tests use the reference JPEG fixture (`sample.jpg`).

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

// ---------------------------------------------------------------------------
// 5.1 Resize
// ---------------------------------------------------------------------------
mod resize {
    use super::*;

    #[test]
    fn test_resize_quarter() {
        let src = libviprs::source::generate_test_raster(256, 256).unwrap();
        let half = libviprs::resize::downscale_half(&src).unwrap();
        assert_eq!(half.width(), 128);
        assert_eq!(half.height(), 128);
        let quarter = libviprs::resize::downscale_half(&half).unwrap();
        assert_eq!(quarter.width(), 64);
        assert_eq!(quarter.height(), 64);
    }

    #[test]
    #[ignore]
    /// Verify downscale behaviour with odd-dimension inputs.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Resize an image by the given scale factor (0.0..∞).
    /// /// Uses an appropriate combination of shrink + reduce for quality.
    /// fn Raster::resize(&self, scale: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_resize)
    ///
    /// 1. Load sample.jpg.
    /// 2. Resize by 0.25: width ≈ round(orig_width / 4), height ≈ round(orig_height / 4).
    /// 3. Resize a 100×1 black image by 0.5: should give exactly 50×1.
    /// 4. Resize a 1600×1000 black image by 10/1600: should give 10×6.
    ///
    /// Reference: test_resample.py::test_resize
    fn test_resize_rounding() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();
        let im2 = im.resize(0.25);
        assert_eq!(im2.width(), ((im.width() as f64 / 4.0) + 0.5) as u32);
        assert_eq!(im2.height(), ((im.height() as f64 / 4.0) + 0.5) as u32);

        // Edge case: thin image
        let im = Raster::black(100, 1);
        let x = im.resize(0.5);
        assert_eq!(x.width(), 50);
        assert_eq!(x.height(), 1);

        // Double-precision calculation
        let im = Raster::black(1600, 1000);
        let x = im.resize(10.0 / 1600.0);
        assert_eq!(x.width(), 10);
        assert_eq!(x.height(), 6);
    }

    #[test]
    #[ignore]
    /// Shrink an image by an integer factor.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Shrink (box-average downsample) by integer factors.
    /// fn Raster::shrink(&self, xfactor: f64, yfactor: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_shrink)
    ///
    /// 1. Load sample.jpg, shrink by (4, 4).
    /// 2. Assert dimensions match round(orig / 4).
    /// 3. Assert average doesn't change much (|avg_diff| < 1).
    /// 4. Shrink by (2.5, 2.5), same checks.
    ///
    /// Reference: test_resample.py::test_shrink
    fn test_shrink() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        let im2 = im.shrink(4.0, 4.0);
        assert_eq!(im2.width(), ((im.width() as f64 / 4.0) + 0.5) as u32);
        assert_eq!(im2.height(), ((im.height() as f64 / 4.0) + 0.5) as u32);
        assert!((im.avg() - im2.avg()).abs() < 1.0);

        let im2 = im.shrink(2.5, 2.5);
        assert_eq!(im2.width(), ((im.width() as f64 / 2.5) + 0.5) as u32);
        assert_eq!(im2.height(), ((im.height() as f64 / 2.5) + 0.5) as u32);
        assert!((im.avg() - im2.avg()).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Shrink using box-average filter and compare quality.
    ///
    /// ## Required API
    ///
    /// Same `Raster::shrink` with average quality comparison.
    ///
    /// ## Test logic
    ///
    /// Verify that box-average shrink preserves the mean pixel value
    /// across different integer and non-integer scale factors.
    ///
    /// Reference: test_resample.py::test_shrink (additional quality checks)
    fn test_shrink_average() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        for &factor in &[2.0, 3.0, 4.0, 5.0, 8.0] {
            let shrunk = im.shrink(factor, factor);
            assert!(
                (im.avg() - shrunk.avg()).abs() < 1.0,
                "Average should be preserved when shrinking by {factor}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 5.2 Affine Transform
// ---------------------------------------------------------------------------
mod affine {
    use super::*;

    #[test]
    #[ignore]
    /// Apply an affine rotation then its inverse; verify round-trip fidelity.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Apply an affine transform defined by a 2×2 matrix [a, b, c, d]
    /// /// where output(x,y) = input(a*x + b*y, c*x + d*y).
    /// /// `interpolate`: resampling kernel ("nearest", "bilinear", "bicubic", "nohalo", "lbb").
    /// fn Raster::affine(&self, matrix: [f64; 4], interpolate: &str) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_affine)
    ///
    /// For each interpolator (nearest, bicubic, bilinear, nohalo, lbb):
    ///   1. Apply 90° rotation [0,1,1,0] four times.
    ///   2. After 4 rotations, the image should match the original exactly (max diff = 0).
    ///
    /// Reference: test_resample.py::test_affine
    fn test_affine_rotation_roundtrip() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        for interp in &["nearest", "bicubic", "bilinear", "nohalo", "lbb"] {
            let mut x = im.clone();
            for _ in 0..4 {
                x = x.affine([0.0, 1.0, 1.0, 0.0], interp);
            }

            let max_diff = im.data().iter().zip(x.data().iter())
                .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs() as u8)
                .max()
                .unwrap_or(0);
            assert_eq!(max_diff, 0, "4× rotation round-trip should be identity for {interp}");
        }
    }

    #[test]
    #[ignore]
    /// Rotate an image using the similarity operator.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Apply a similarity transform (rotate + scale).
    /// /// `angle`: rotation in degrees.
    /// /// `scale`: scale factor (1.0 = no scaling).
    /// fn Raster::similarity(&self, angle: f64, scale: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_similarity)
    ///
    /// 1. Load sample.jpg.
    /// 2. similarity(angle=90) should approximately match affine([0,-1,1,0]).
    /// 3. Max difference < 50 (rounding in angle-to-matrix conversion).
    ///
    /// Reference: test_resample.py::test_similarity
    fn test_similarity_rotate() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        let im2 = im.similarity(90.0, 1.0);
        let im3 = im.affine([0.0, -1.0, 1.0, 0.0], "bilinear");

        let max_diff = im2.data().iter().zip(im3.data().iter())
            .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(max_diff < 50, "similarity(90) vs affine rotation: max_diff={max_diff}");
    }

    #[test]
    #[ignore]
    /// Scale an image using the similarity operator.
    ///
    /// ## Required API
    ///
    /// Same `Raster::similarity` with scale parameter.
    ///
    /// ## Test logic (from libvips test_resample.py::test_similarity_scale)
    ///
    /// 1. similarity(angle=0, scale=2) should match affine([2,0,0,2]) exactly.
    ///
    /// Reference: test_resample.py::test_similarity_scale
    fn test_similarity_scale() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        let im2 = im.similarity(0.0, 2.0);
        let im3 = im.affine([2.0, 0.0, 0.0, 2.0], "bilinear");

        let max_diff = im2.data().iter().zip(im3.data().iter())
            .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert_eq!(max_diff, 0, "similarity(scale=2) should match affine(2x)");
    }

    #[test]
    #[ignore]
    /// Rotate an image by an arbitrary angle and check dimensions.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Rotate an image by an arbitrary angle (in degrees).
    /// /// Automatically expands the canvas to fit the rotated image.
    /// fn Raster::rotate(&self, angle: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_rotate)
    ///
    /// 1. rotate(90) should approximately match affine([0,-1,1,0]).
    /// 2. Max difference < 50.
    ///
    /// Reference: test_resample.py::test_rotate
    fn test_rotate_arbitrary() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        let im2 = im.rotate(90.0);
        let im3 = im.affine([0.0, -1.0, 1.0, 0.0], "bilinear");

        let max_diff = im2.data().iter().zip(im3.data().iter())
            .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs())
            .max()
            .unwrap_or(0);
        assert!(max_diff < 50, "rotate(90) vs affine: max_diff={max_diff}");
    }
}

// ---------------------------------------------------------------------------
// 5.3 Advanced Resampling
// ---------------------------------------------------------------------------
mod advanced_resampling {
    use super::*;

    #[test]
    #[ignore]
    /// Compare output across different resampling kernels.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Reduce (downsample with anti-aliasing) by the given factor using the specified kernel.
    /// /// `kernel`: "nearest", "linear", "cubic", "lanczos2", "lanczos3".
    /// fn Raster::reduce(&self, xfactor: f64, yfactor: f64, kernel: &str) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_reduce)
    ///
    /// 1. Load sample.jpg, cast to signed char (0..127 range).
    /// 2. For factor in [1.0, 1.1, 1.5, 1.999]:
    ///    For each kernel: reduce and check |avg_diff| < 2.
    /// 3. For constant images (0, 1, 2, 254, 255):
    ///    reduce(2, 2) with each kernel should preserve the constant exactly.
    ///
    /// Reference: test_resample.py::test_reduce
    fn test_reduce_kernels() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        let kernels = ["nearest", "linear", "cubic", "lanczos2", "lanczos3"];

        for &fac in &[1.0, 1.1, 1.5, 1.999] {
            for kernel in &kernels {
                let r = im.reduce(fac, fac, kernel);
                let d = (r.avg() - im.avg()).abs();
                assert!(d < 2.0, "reduce(fac={fac}, kernel={kernel}) avg diff={d}");
            }
        }

        // Constant image preservation
        for &val in &[0u8, 1, 2, 254, 255] {
            let constant = Raster::constant_u8(10, 10, val);
            for kernel in &kernels {
                let r = constant.reduce(2.0, 2.0, kernel);
                assert!(
                    (r.avg() - val as f64).abs() < 0.001,
                    "Constant {val} should be preserved by reduce with {kernel}"
                );
            }
        }
    }

    #[test]
    #[ignore]
    /// Generate a thumbnail respecting aspect ratio.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Generate a thumbnail of the image at the given target height.
    /// /// Preserves aspect ratio. If `crop` is true, crops to exact dimensions.
    /// fn Raster::thumbnail(path: &Path, width: u32, height: Option<u32>, crop: bool) -> Raster;
    /// fn Raster::thumbnail_buffer(data: &[u8], width: u32) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_thumbnail)
    ///
    /// 1. Thumbnail sample.jpg to height=100: assert height=100, bands=3.
    /// 2. Verify average doesn't shift much (|diff| < 1).
    /// 3. For heights from 440 down to 2, step -13: assert exact height match.
    /// 4. With width=100 and height=300: width should be 100 (height adjusts).
    /// 5. With crop=true: both dimensions should match.
    /// 6. Buffer thumbnail should match file thumbnail.
    ///
    /// Reference: test_resample.py::test_thumbnail
    fn test_thumbnail() {
        let path = ref_image("sample.jpg");
        let im = Raster::thumbnail(&path, 100, None, false);
        assert_eq!(im.height(), 100);
        assert_eq!(im.format().channels(), 3);

        let im_orig = decode_file(&path).unwrap();
        assert!((im_orig.avg() - im.avg()).abs() < 1.0);

        // Exact height for a range of sizes
        for height in (2..=440).rev().step_by(13) {
            let im = Raster::thumbnail(&path, height, None, false);
            assert_eq!(im.height(), height, "Thumbnail height mismatch for target={height}");
        }

        // Width and height constraints
        let im = Raster::thumbnail(&path, 100, Some(300), false);
        assert_eq!(im.width(), 100);
        assert_ne!(im.height(), 300);

        let im = Raster::thumbnail(&path, 300, Some(100), false);
        assert_ne!(im.width(), 300);
        assert_eq!(im.height(), 100);

        // Crop mode
        let im = Raster::thumbnail(&path, 100, Some(300), true);
        assert_eq!(im.width(), 100);
        assert_eq!(im.height(), 300);

        // Buffer thumbnail
        let im1 = Raster::thumbnail(&path, 100, None, false);
        let buf = std::fs::read(&path).unwrap();
        let im2 = Raster::thumbnail_buffer(&buf, 100);
        assert!((im1.avg() - im2.avg()).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Thumbnail with ICC profile handling.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::thumbnail_with_profile(path: &Path, width: u32, output_profile: &str) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_thumbnail_icc)
    ///
    /// 1. Thumbnail sample-xyb.jpg at width=442 with output_profile="srgb".
    /// 2. Assert width=290, height=442, bands=3.
    /// 3. dE00 vs original sample.jpg should be < 10.
    ///
    /// Reference: test_resample.py::test_thumbnail_icc
    fn test_thumbnail_icc() {
        let im = Raster::thumbnail_with_profile(
            &ref_image("sample-xyb.jpg"),
            442,
            "srgb",
        );
        assert_eq!(im.width(), 290);
        assert_eq!(im.height(), 442);
        assert_eq!(im.format().channels(), 3);

        let im_orig = decode_file(&ref_image("sample.jpg")).unwrap();
        let de = im_orig.de00(&im);
        assert!(de.max_value() < 10.0, "ICC thumbnail dE00 should be < 10");
    }

    #[test]
    #[ignore]
    /// Remap pixels via an index image.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Remap pixels using a 2-band coordinate image.
    /// /// `index`: a 2-band image where band 0 = source x, band 1 = source y.
    /// /// `interpolate`: resampling kernel name.
    /// fn Raster::mapim(&self, index: &Raster, interpolate: &str) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_resample.py::test_mapim)
    ///
    /// 1. Load sample.jpg.
    /// 2. Create a coordinate image (xyz) of the same size.
    /// 3. mapim(xyz, "bicubic") should produce the same image (identity remap).
    /// 4. Assert avg matches exactly.
    ///
    /// Reference: test_resample.py::test_mapim
    fn test_mapim() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        // Identity remap: coordinate image
        let mp = Raster::xyz(im.width(), im.height());
        let remapped = im.mapim(&mp, "bicubic");
        assert!(
            (im.avg() - remapped.avg()).abs() < 0.001,
            "Identity mapim should preserve average exactly"
        );
    }
}
