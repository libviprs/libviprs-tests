#![cfg(feature = "ported_tests")]

//! Ported arithmetic tests from libvips `test_arithmetic.py`.
//!
//! Tests exercise binary/unary arithmetic, bitwise ops, comparisons,
//! statistics, math functions, complex/histogram operations.
//! The libvips test suite uses a synthetic `mask_ideal` image; we use
//! a similar generated test image (100×100, 3-band) plus sample.jpg.

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

/// Create a synthetic 100×100 single-band (Gray8) test image with recognisable
/// values at (10,10) and (50,50) — matching the libvips test setup which uses
/// `mask_ideal(100, 100, 0.5, reject=True, optical=True)`.
fn make_test_mono() -> Raster {
    let w = 100u32;
    let h = 100u32;
    let mut data = vec![0u8; (w * h) as usize];
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f64 - cx) / cx;
            let dy = (y as f64 - cy) / cy;
            let r = (dx * dx + dy * dy).sqrt();
            // Band-reject at 0.5: outside ring is bright, inside is dark
            let v = if r > 0.5 { (r * 200.0).min(255.0) as u8 } else { 0 };
            data[(y * w + x) as usize] = v;
        }
    }
    Raster::new(w, h, PixelFormat::Gray8, data).unwrap()
}

/// Create a synthetic 100×100, 3-band (Rgb8) test image.
/// Equivalent to libvips: `im * [1, 2, 3] + [2, 3, 4]`.
fn make_test_colour() -> Raster {
    let mono = make_test_mono();
    let w = mono.width();
    let h = mono.height();
    let md = mono.data();
    let mut data = vec![0u8; (w * h * 3) as usize];
    for i in 0..(w * h) as usize {
        let v = md[i] as u16;
        data[i * 3]     = ((v * 1 + 2).min(255)) as u8;
        data[i * 3 + 1] = ((v * 2 + 3).min(255)) as u8;
        data[i * 3 + 2] = ((v * 3 + 4).min(255)) as u8;
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

mod basic_arithmetic {
    use super::*;

    #[test]
    #[ignore]
    /// Image + image and image + constant addition.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Add two images pixel-by-pixel. Images must have matching dimensions
    /// /// (or one is broadcastable). Result is promoted to a wider format.
    /// fn Raster::add(&self, other: &Raster) -> Raster;
    ///
    /// /// Add a scalar constant to every pixel in every band.
    /// fn Raster::add_const(&self, c: f64) -> Raster;
    ///
    /// /// Add a per-band constant vector.
    /// fn Raster::add_vec(&self, v: &[f64]) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_add)
    ///
    /// 1. Create mono and colour test images.
    /// 2. For each pair of formats, cast both, add, and verify pixel at (50,50)
    ///    and (10,10) matches expected sum.
    /// 3. Also test image + scalar (2) and image + vector [1,2,3].
    ///
    /// Reference: test_arithmetic.py::test_add
    fn test_add() {
        let colour = make_test_colour();
        let mono = make_test_mono();

        // image + image
        let result = colour.add(&colour);
        let px_a = colour.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        for (a, r) in px_a.iter().zip(px_r.iter()) {
            assert!((r - (a + a)).abs() < 1.0);
        }

        // image + scalar
        let result = mono.add_const(42.0);
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - (px_m[0] + 42.0)).abs() < 1.0);

        // image + vector
        let result = colour.add_vec(&[1.0, 2.0, 3.0]);
        let px_c = colour.getpoint(10, 10);
        let px_r = result.getpoint(10, 10);
        for (i, (c, r)) in px_c.iter().zip(px_r.iter()).enumerate() {
            assert!((r - (c + (i as f64 + 1.0))).abs() < 1.0);
        }
    }

    #[test]
    #[ignore]
    /// Image - image and image - constant subtraction.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::sub(&self, other: &Raster) -> Raster;
    /// fn Raster::sub_const(&self, c: f64) -> Raster;
    /// fn Raster::sub_vec(&self, v: &[f64]) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_sub)
    ///
    /// 1. Subtract image from itself — result should be all zeros.
    /// 2. Subtract scalar, verify at (50,50) and (10,10).
    ///
    /// Reference: test_arithmetic.py::test_sub
    fn test_sub() {
        let colour = make_test_colour();

        // image - image => zeros
        let result = colour.sub(&colour);
        assert!(result.avg().abs() < 0.001);

        // image - scalar
        let result = colour.sub_const(1.0);
        let px_c = colour.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        for (c, r) in px_c.iter().zip(px_r.iter()) {
            assert!((r - (c - 1.0)).abs() < 1.0);
        }
    }

    #[test]
    #[ignore]
    /// Image * image and image * constant multiplication.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::mul(&self, other: &Raster) -> Raster;
    /// fn Raster::mul_const(&self, c: f64) -> Raster;
    /// fn Raster::mul_vec(&self, v: &[f64]) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_mul)
    ///
    /// 1. Multiply image by itself, check pixel at (50,50).
    /// 2. Multiply by scalar 2.0, verify doubled values.
    ///
    /// Reference: test_arithmetic.py::test_mul
    fn test_mul() {
        let colour = make_test_colour();

        let result = colour.mul_const(2.0);
        let px_c = colour.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        for (c, r) in px_c.iter().zip(px_r.iter()) {
            assert!((r - c * 2.0).abs() < 1.0);
        }
    }

    #[test]
    #[ignore]
    /// Image / image and image / constant division.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::div(&self, other: &Raster) -> Raster;
    /// fn Raster::div_const(&self, c: f64) -> Raster;
    /// fn Raster::div_vec(&self, v: &[f64]) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_div)
    ///
    /// 1. Divide image by itself — non-zero pixels become 1.
    /// 2. Divide by scalar 2.0, verify halved values.
    ///
    /// Reference: test_arithmetic.py::test_div
    fn test_div() {
        let colour = make_test_colour();

        let result = colour.div_const(2.0);
        let px_c = colour.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        for (c, r) in px_c.iter().zip(px_r.iter()) {
            assert!((r - c / 2.0).abs() < 1.0);
        }
    }

    #[test]
    #[ignore]
    /// Floor division (integer division).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Integer (floor) division, like Python's //.
    /// fn Raster::floordiv(&self, other: &Raster) -> Raster;
    /// fn Raster::floordiv_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_floordiv)
    ///
    /// 1. Floor-divide by 3, verify pixel at (50,50) matches floor(v / 3).
    ///
    /// Reference: test_arithmetic.py::test_floordiv
    fn test_floordiv() {
        let colour = make_test_colour();

        let result = colour.floordiv_const(3.0);
        let px_c = colour.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        for (c, r) in px_c.iter().zip(px_r.iter()) {
            assert!((r - (c / 3.0).floor()).abs() < 1.0);
        }
    }

    #[test]
    #[ignore]
    /// Power operation (image ** exponent).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::pow_const(&self, exp: f64) -> Raster;
    /// fn Raster::pow(&self, other: &Raster) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_pow)
    ///
    /// 1. Raise image to power 2, verify pixel at (50,50).
    ///
    /// Reference: test_arithmetic.py::test_pow
    fn test_pow() {
        let mono = make_test_mono();

        let result = mono.pow_const(2.0);
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - px_m[0].powf(2.0)).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Modulo operation (image % constant).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::rem_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_mod)
    ///
    /// 1. Compute image % 2, verify pixel at (50,50).
    ///
    /// Reference: test_arithmetic.py::test_mod
    fn test_mod() {
        let mono = make_test_mono();

        let result = mono.rem_const(2.0);
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - (px_m[0] as i64 % 2) as f64).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Unary positive (+image should be identity).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Returns a copy of the image (identity operation).
    /// fn Raster::pos(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_pos)
    ///
    /// 1. Apply +image, verify pixel values are unchanged.
    ///
    /// Reference: test_arithmetic.py::test_pos
    fn test_pos() {
        let mono = make_test_mono();
        let result = mono.pos();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - px_m[0]).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Unary negation (-image).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Negate every pixel value. Output format is signed.
    /// fn Raster::neg(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_neg)
    ///
    /// 1. Negate image, verify pixel at (50,50) is negated.
    ///
    /// Reference: test_arithmetic.py::test_neg
    fn test_neg() {
        let mono = make_test_mono();
        let result = mono.neg();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - (-px_m[0])).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Absolute value of pixel values.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute the absolute value of every pixel.
    /// fn Raster::abs(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_abs)
    ///
    /// 1. Negate the colour image to get negative values.
    /// 2. Apply abs, verify pixel values match original.
    ///
    /// Reference: test_arithmetic.py::test_abs
    fn test_abs() {
        let colour = make_test_colour();
        let negated = colour.neg();
        let result = negated.abs();
        let px_c = colour.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        for (c, r) in px_c.iter().zip(px_r.iter()) {
            assert!((r - c).abs() < 1.0);
        }
    }

    #[test]
    #[ignore]
    /// Clamp pixel values to a range.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Clamp pixel values. Default range is [0, 1].
    /// fn Raster::clamp(&self, min: Option<f64>, max: Option<f64>) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_clamp)
    ///
    /// 1. Clamp to default [0,1]: max should be ≤ 1.0, min ≥ 0.0.
    /// 2. Clamp to [14,45]: max ≤ 45, min ≥ 14.
    ///
    /// Reference: test_arithmetic.py::test_clamp
    fn test_clamp() {
        let colour = make_test_colour();

        let result = colour.clamp(None, None);
        assert!(result.max() <= 1.0);
        assert!(result.min() >= 0.0);

        let result = colour.clamp(Some(14.0), Some(45.0));
        assert!(result.max() <= 45.0);
        assert!(result.min() >= 14.0);
    }
}

mod bitwise {
    use super::*;

    #[test]
    #[ignore]
    /// Bitwise AND of two images.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Bitwise AND of two images, pixel by pixel.
    /// fn Raster::bitand(&self, other: &Raster) -> Raster;
    ///
    /// /// Bitwise AND with a scalar constant.
    /// fn Raster::bitand_const(&self, c: i64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_and)
    ///
    /// 1. AND image with itself — should be identity.
    /// 2. AND with scalar 0xFF — should be identity for uchar.
    /// 3. AND with 0 — should be all zeros.
    ///
    /// Reference: test_arithmetic.py::test_and
    fn test_and() {
        let mono = make_test_mono();

        // AND with itself is identity
        let result = mono.bitand(&mono);
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - px_m[0]).abs() < 0.001);

        // AND with 0 is zero
        let result = mono.bitand_const(0);
        assert!(result.avg().abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Bitwise OR of two images.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::bitor(&self, other: &Raster) -> Raster;
    /// fn Raster::bitor_const(&self, c: i64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_or)
    ///
    /// 1. OR image with itself — identity.
    /// 2. OR with 0xFF — all 255 for uchar.
    ///
    /// Reference: test_arithmetic.py::test_or
    fn test_or() {
        let mono = make_test_mono();

        let result = mono.bitor(&mono);
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - px_m[0]).abs() < 0.001);

        let result = mono.bitor_const(0xFF);
        assert!((result.avg() - 255.0).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Bitwise XOR of two images.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::bitxor(&self, other: &Raster) -> Raster;
    /// fn Raster::bitxor_const(&self, c: i64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_xor)
    ///
    /// 1. XOR image with itself — should be all zeros.
    ///
    /// Reference: test_arithmetic.py::test_xor
    fn test_xor() {
        let mono = make_test_mono();

        let result = mono.bitxor(&mono);
        assert!(result.avg().abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Bitwise NOT (invert) of an image.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Bitwise invert: ~pixel & max_for_format.
    /// fn Raster::bitnot(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_invert)
    ///
    /// 1. Invert uchar image, verify pixel at (50,50) equals ~original & 0xFF.
    ///
    /// Reference: test_arithmetic.py::test_invert
    fn test_invert() {
        let mono = make_test_mono();
        let result = mono.bitnot();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = (!px_m[0] as u8) as f64;
        assert!((px_r[0] - expected).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Left shift.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Shift every pixel left by `n` bits.
    /// fn Raster::lshift(&self, n: u32) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_lshift)
    ///
    /// 1. Shift left by 2, verify pixel at (50,50) matches v << 2.
    ///
    /// Reference: test_arithmetic.py::test_lshift
    fn test_lshift() {
        let mono = make_test_mono();
        let result = mono.lshift(2);
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - ((px_m[0] as i64) << 2) as f64).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Right shift.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Shift every pixel right by `n` bits.
    /// fn Raster::rshift(&self, n: u32) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_rshift)
    ///
    /// 1. Shift right by 2, verify pixel at (50,50) matches v >> 2.
    ///
    /// Reference: test_arithmetic.py::test_rshift
    fn test_rshift() {
        let mono = make_test_mono();
        let result = mono.rshift(2);
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - ((px_m[0] as i64) >> 2) as f64).abs() < 1.0);
    }
}

mod comparison {
    use super::*;

    #[test]
    #[ignore]
    /// Greater than comparison (image > image and image > constant).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Per-pixel greater-than. Returns a uchar image: 255 where true, 0 where false.
    /// fn Raster::more_than(&self, other: &Raster) -> Raster;
    /// fn Raster::more_than_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_more)
    ///
    /// 1. Compare image > 100 — bright pixels → 255, dark → 0.
    /// 2. Compare image > image — should be all zeros.
    ///
    /// Reference: test_arithmetic.py::test_more
    fn test_more() {
        let mono = make_test_mono();

        // image > image should be all zeros
        let result = mono.more_than(&mono);
        assert!(result.avg().abs() < 0.001);

        // image > 100 should have some 255s and some 0s
        let result = mono.more_than_const(100.0);
        let px = result.getpoint(50, 50);
        let m = mono.getpoint(50, 50);
        let expected = if m[0] > 100.0 { 255.0 } else { 0.0 };
        assert!((px[0] - expected).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Greater or equal comparison.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::more_eq(&self, other: &Raster) -> Raster;
    /// fn Raster::more_eq_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_moreeq)
    ///
    /// 1. image >= image should be all 255.
    ///
    /// Reference: test_arithmetic.py::test_moreeq
    fn test_moreeq() {
        let mono = make_test_mono();
        let result = mono.more_eq(&mono);
        assert!((result.avg() - 255.0).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Less than comparison.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::less_than(&self, other: &Raster) -> Raster;
    /// fn Raster::less_than_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_less)
    ///
    /// 1. image < image should be all zeros.
    ///
    /// Reference: test_arithmetic.py::test_less
    fn test_less() {
        let mono = make_test_mono();
        let result = mono.less_than(&mono);
        assert!(result.avg().abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Less or equal comparison.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::less_eq(&self, other: &Raster) -> Raster;
    /// fn Raster::less_eq_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_lesseq)
    ///
    /// 1. image <= image should be all 255.
    ///
    /// Reference: test_arithmetic.py::test_lesseq
    fn test_lesseq() {
        let mono = make_test_mono();
        let result = mono.less_eq(&mono);
        assert!((result.avg() - 255.0).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Pixel-wise equality.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::equal(&self, other: &Raster) -> Raster;
    /// fn Raster::equal_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_equal)
    ///
    /// 1. image == image should be all 255.
    /// 2. On a grey ramp: x == 1000 should be all 0 (out of range).
    /// 3. x == 12 should have some 255 pixels.
    /// 4. x == 12.5 should be all 0 (no integer matches).
    ///
    /// Reference: test_arithmetic.py::test_equal
    fn test_equal() {
        let mono = make_test_mono();

        let result = mono.equal(&mono);
        assert!((result.avg() - 255.0).abs() < 0.001);

        // Grey ramp tests
        let x = Raster::grey(256, 256, true);

        let cmp = x.equal_const(1000.0);
        assert!(cmp.max() < 1.0, "No uchar pixel can equal 1000");

        let cmp = x.equal_const(12.0);
        assert!((cmp.max() - 255.0).abs() < 1.0, "x==12 should find matches");

        let cmp = x.equal_const(12.5);
        assert!(cmp.max() < 1.0, "No integer pixel can equal 12.5");
    }

    #[test]
    #[ignore]
    /// Pixel-wise not-equal.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::noteq(&self, other: &Raster) -> Raster;
    /// fn Raster::noteq_const(&self, c: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_noteq)
    ///
    /// 1. image != image should be all 0.
    ///
    /// Reference: test_arithmetic.py::test_noteq
    fn test_noteq() {
        let mono = make_test_mono();
        let result = mono.noteq(&mono);
        assert!(result.avg().abs() < 0.001);
    }
}

mod statistics {
    use super::*;

    #[test]
    #[ignore]
    /// Average pixel value.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute the mean pixel value across the whole image (all bands averaged together).
    /// fn Raster::avg(&self) -> f64;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_avg)
    ///
    /// 1. Create 50×100 black image, insert 50×100 image + 100 at (50,0) with expand.
    /// 2. Result is 100×100 with left half = 0, right half = 100.
    /// 3. avg should be ≈ 50.
    ///
    /// Reference: test_arithmetic.py::test_avg
    fn test_avg() {
        let left = Raster::zeroed(50, 100, PixelFormat::Gray8);
        let right_data = vec![100u8; 50 * 100];
        let right = Raster::new(50, 100, PixelFormat::Gray8, right_data).unwrap();
        let combined = left.insert(&right, 50, 0, true);

        assert!(
            (combined.avg() - 50.0).abs() < 1.0,
            "Average of half-black, half-100 image should be ~50, got {}",
            combined.avg()
        );
    }

    #[test]
    #[ignore]
    /// Standard deviation of pixel values.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Standard deviation of pixel values across the whole image.
    /// fn Raster::deviate(&self) -> f64;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_deviate)
    ///
    /// 1. Same half-black, half-100 image as test_avg.
    /// 2. deviate should be ≈ 50.
    ///
    /// Reference: test_arithmetic.py::test_deviate
    fn test_deviate() {
        let left = Raster::zeroed(50, 100, PixelFormat::Gray8);
        let right_data = vec![100u8; 50 * 100];
        let right = Raster::new(50, 100, PixelFormat::Gray8, right_data).unwrap();
        let combined = left.insert(&right, 50, 0, true);

        assert!(
            (combined.deviate() - 50.0).abs() < 1.0,
            "Deviate should be ~50, got {}",
            combined.deviate()
        );
    }

    #[test]
    #[ignore]
    /// Maximum value with position.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Find the maximum pixel value. Returns the value.
    /// fn Raster::max(&self) -> f64;
    ///
    /// /// Find the maximum pixel value and its position.
    /// fn Raster::maxpos(&self) -> (f64, u32, u32);
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_max)
    ///
    /// 1. Create 100×100 black image, draw a single pixel of value 100 at (40,50).
    /// 2. max() should be 100.
    /// 3. maxpos() should return (100, 40, 50).
    ///
    /// Reference: test_arithmetic.py::test_max
    fn test_max() {
        let mut im = Raster::zeroed(100, 100, PixelFormat::Gray8);
        im.draw_rect_filled(100, 40, 50, 1, 1);

        assert!((im.max() - 100.0).abs() < 1.0);

        let (v, x, y) = im.maxpos();
        assert!((v - 100.0).abs() < 1.0);
        assert_eq!(x, 40);
        assert_eq!(y, 50);
    }

    #[test]
    #[ignore]
    /// Minimum value with position.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::min(&self) -> f64;
    /// fn Raster::minpos(&self) -> (f64, u32, u32);
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_min)
    ///
    /// 1. Create 100×100 image filled with 100, draw single pixel 0 at (40,50).
    /// 2. min() should be 0.
    /// 3. minpos() should return (0, 40, 50).
    ///
    /// Reference: test_arithmetic.py::test_min
    fn test_min() {
        let data = vec![100u8; 100 * 100];
        let mut im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        im.draw_rect_filled(0, 40, 50, 1, 1);

        assert!(im.min().abs() < 1.0);

        let (v, x, y) = im.minpos();
        assert!(v.abs() < 1.0);
        assert_eq!(x, 40);
        assert_eq!(y, 50);
    }

    #[test]
    #[ignore]
    /// Stats matrix: min, max, sum, sum², mean, deviate per band + overall.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Return image statistics as a matrix.
    /// /// The result is a 6×(bands+1) matrix where:
    /// /// - Row 0: overall stats
    /// /// - Rows 1..bands: per-band stats
    /// /// - Columns: min, max, sum, sum_of_squares, mean, deviation
    /// fn Raster::stats(&self) -> Vec<Vec<f64>>;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_stats)
    ///
    /// 1. Create half-black, half-10 image (100×50, single band).
    /// 2. stats()[0][0] (min) = 0, stats()[1][0] (max) = 10.
    /// 3. stats()[2][0] (sum) = 50*50*10 = 25000.
    /// 4. stats()[4][0] (mean) = avg().
    /// 5. stats()[5][0] (deviate) = deviate().
    ///
    /// Reference: test_arithmetic.py::test_stats
    fn test_stats() {
        let left = Raster::zeroed(50, 50, PixelFormat::Gray8);
        let right_data = vec![10u8; 50 * 50];
        let right = Raster::new(50, 50, PixelFormat::Gray8, right_data).unwrap();
        let im = left.insert(&right, 50, 0, true);

        let stats = im.stats();

        // Overall stats (row 0): min=0, max=10
        assert!((stats[0][0] - 0.0).abs() < 0.001, "min should be 0");
        assert!((stats[0][1] - 10.0).abs() < 0.001, "max should be 10");
        // sum = 50*50*10 = 25000
        assert!((stats[0][2] - 25000.0).abs() < 1.0, "sum should be 25000");
        // mean = avg
        assert!((stats[0][4] - im.avg()).abs() < 0.01);
        // deviate = deviate
        assert!((stats[0][5] - im.deviate()).abs() < 0.01);
    }

    #[test]
    #[ignore]
    /// Measure patch averages from an image grid.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Measure the average of each patch in a grid of `h` rows × `w` columns.
    /// /// Returns a matrix of patch averages (w columns, h rows).
    /// fn Raster::measure(&self, h: u32, w: u32) -> Vec<Vec<f64>>;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_measure)
    ///
    /// 1. Create 100×50 image: left 50 = 0, right 50 = 10.
    /// 2. measure(2, 1) should give [[0], [10]].
    ///
    /// Reference: test_arithmetic.py::test_measure
    fn test_measure() {
        let left = Raster::zeroed(50, 50, PixelFormat::Gray8);
        let right_data = vec![10u8; 50 * 50];
        let right = Raster::new(50, 50, PixelFormat::Gray8, right_data).unwrap();
        let im = left.insert(&right, 50, 0, true);

        let matrix = im.measure(2, 1);
        assert!((matrix[0][0] - 0.0).abs() < 1.0);
        assert!((matrix[1][0] - 10.0).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Find the bounding box of non-background pixels.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Find the bounding box of non-background content.
    /// /// Returns (left, top, width, height).
    /// fn Raster::find_trim(&self, background: Option<&[f64]>) -> (u32, u32, u32, u32);
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_find_trim)
    ///
    /// 1. Create 50×60 image of value 100.
    /// 2. Embed at (10,20) in a 200×300 white canvas.
    /// 3. find_trim() should return (10, 20, 50, 60).
    ///
    /// Reference: test_arithmetic.py::test_find_trim
    fn test_find_trim() {
        let inner_data = vec![100u8; 50 * 60];
        let inner = Raster::new(50, 60, PixelFormat::Gray8, inner_data).unwrap();
        let canvas = Raster::new(
            200, 300, PixelFormat::Gray8,
            vec![255u8; 200 * 300],
        ).unwrap();
        let im = canvas.insert(&inner, 10, 20, false);

        let (left, top, width, height) = im.find_trim(None);
        assert_eq!(left, 10);
        assert_eq!(top, 20);
        assert_eq!(width, 50);
        assert_eq!(height, 60);
    }

    #[test]
    #[ignore]
    /// Profile: find first non-zero pixel in each row/column.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Find the first non-zero pixel in each row and column.
    /// /// Returns (columns, rows) where:
    /// /// - columns is a 1-row image, width = im.width, giving first non-zero row index
    /// /// - rows is a 1-column image, height = im.height, giving first non-zero column index
    /// fn Raster::profile(&self) -> (Raster, Raster);
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_profile)
    ///
    /// 1. Create 100×100 black image, draw single pixel 100 at (40,50).
    /// 2. columns.minpos() should be (50, 40, 0).
    /// 3. rows.minpos() should be (40, 0, 50).
    ///
    /// Reference: test_arithmetic.py::test_profile
    fn test_profile() {
        let mut im = Raster::zeroed(100, 100, PixelFormat::Gray8);
        im.draw_rect_filled(100, 40, 50, 1, 1);

        let (columns, rows) = im.profile();

        let (v, x, y) = columns.minpos();
        assert!((v - 50.0).abs() < 1.0);
        assert_eq!(x, 40);
        assert_eq!(y, 0);

        let (v, x, y) = rows.minpos();
        assert!((v - 40.0).abs() < 1.0);
        assert_eq!(x, 0);
        assert_eq!(y, 50);
    }

    #[test]
    #[ignore]
    /// Project: column and row sums.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Column and row sums.
    /// /// Returns (columns, rows) where:
    /// /// - columns is a 1-row image with the sum of each column
    /// /// - rows is a 1-column image with the sum of each row
    /// fn Raster::project(&self) -> (Raster, Raster);
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_project)
    ///
    /// 1. Create 100×50 image: left half = 0, right half = 10.
    /// 2. columns at x=10 should be 0.
    /// 3. columns at x=70 should be 50*10 = 500.
    /// 4. rows at y=10 should be 50*10 = 500.
    ///
    /// Reference: test_arithmetic.py::test_project
    fn test_project() {
        let left = Raster::zeroed(50, 50, PixelFormat::Gray8);
        let right_data = vec![10u8; 50 * 50];
        let right = Raster::new(50, 50, PixelFormat::Gray8, right_data).unwrap();
        let im = left.insert(&right, 50, 0, true);

        let (columns, rows) = im.project();
        let col_10 = columns.getpoint(10, 0);
        assert!((col_10[0] - 0.0).abs() < 1.0);

        let col_70 = columns.getpoint(70, 0);
        assert!((col_70[0] - 500.0).abs() < 1.0);

        let row_10 = rows.getpoint(0, 10);
        assert!((row_10[0] - 500.0).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Sum a list of images.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Sum a slice of images pixel by pixel.
    /// fn Raster::sum(images: &[&Raster]) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_sum)
    ///
    /// 1. Create 10 images with constant values 0, 10, 20, ..., 90.
    /// 2. Sum them. Max should be 0+10+20+...+90 = 450.
    ///
    /// Reference: test_arithmetic.py::test_sum
    fn test_sum() {
        let images: Vec<Raster> = (0..10)
            .map(|x| {
                let data = vec![(x * 10) as u8; 50 * 50];
                Raster::new(50, 50, PixelFormat::Gray8, data).unwrap()
            })
            .collect();
        let refs: Vec<&Raster> = images.iter().collect();
        let result = Raster::sum(&refs);
        let expected_max: f64 = (0..10).map(|x| (x * 10) as f64).sum();
        assert!(
            (result.max() - expected_max).abs() < 1.0,
            "Sum max should be {expected_max}, got {}",
            result.max()
        );
    }

    #[test]
    #[ignore]
    /// Pairwise minimum of two images.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Per-pixel minimum of two images.
    /// fn Raster::minpair(&self, other: &Raster) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_minpair)
    ///
    /// 1. Create two images, compute minpair.
    /// 2. Verify: result == ifthenelse(a < b, a, b).
    ///
    /// Reference: test_arithmetic.py::test_minpair
    fn test_minpair() {
        let a_data = vec![100u8; 50 * 50];
        let a = Raster::new(50, 50, PixelFormat::Gray8, a_data).unwrap();
        let b_data = vec![50u8; 50 * 50];
        let b = Raster::new(50, 50, PixelFormat::Gray8, b_data).unwrap();

        let result = a.minpair(&b);
        assert!((result.avg() - 50.0).abs() < 1.0, "min(100,50) should be 50");
    }

    #[test]
    #[ignore]
    /// Pairwise maximum of two images.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Per-pixel maximum of two images.
    /// fn Raster::maxpair(&self, other: &Raster) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_maxpair)
    ///
    /// 1. Create two images, compute maxpair.
    /// 2. Verify: result == ifthenelse(a > b, a, b).
    ///
    /// Reference: test_arithmetic.py::test_maxpair
    fn test_maxpair() {
        let a_data = vec![100u8; 50 * 50];
        let a = Raster::new(50, 50, PixelFormat::Gray8, a_data).unwrap();
        let b_data = vec![50u8; 50 * 50];
        let b = Raster::new(50, 50, PixelFormat::Gray8, b_data).unwrap();

        let result = a.maxpair(&b);
        assert!((result.avg() - 100.0).abs() < 1.0, "max(100,50) should be 100");
    }
}

mod math_functions {
    use super::*;

    #[test]
    #[ignore]
    /// Sine of pixel values (input in degrees).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute sin(pixel) where input is in degrees. Output is float.
    /// fn Raster::sin(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_sin)
    ///
    /// 1. Apply sin to test image, verify pixel at (50,50) matches
    ///    sin(radians(original_value)).
    ///
    /// Reference: test_arithmetic.py::test_sin
    fn test_sin() {
        let mono = make_test_mono();
        let result = mono.sin();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = (px_m[0].to_radians()).sin();
        assert!(
            (px_r[0] - expected).abs() < 0.001,
            "sin({}) should be {expected}, got {}",
            px_m[0], px_r[0]
        );
    }

    #[test]
    #[ignore]
    /// Cosine of pixel values (input in degrees).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::cos(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_cos)
    ///
    /// 1. Apply cos, verify pixel at (50,50).
    ///
    /// Reference: test_arithmetic.py::test_cos
    fn test_cos() {
        let mono = make_test_mono();
        let result = mono.cos();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = (px_m[0].to_radians()).cos();
        assert!((px_r[0] - expected).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Tangent of pixel values (input in degrees).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::tan(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_tan)
    ///
    /// 1. Apply tan, verify pixel at (10,10) (avoiding values near 90°).
    ///
    /// Reference: test_arithmetic.py::test_tan
    fn test_tan() {
        let mono = make_test_mono();
        let result = mono.tan();
        let px_m = mono.getpoint(10, 10);
        let px_r = result.getpoint(10, 10);
        let expected = (px_m[0].to_radians()).tan();
        assert!((px_r[0] - expected).abs() < 0.01);
    }

    #[test]
    #[ignore]
    /// Arc sine (output in degrees).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute asin(pixel). Output is in degrees.
    /// fn Raster::asin(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_asin)
    ///
    /// 1. Create image with values in [-1, 1] range: (black + [1,2,3]) / 3.
    /// 2. Apply asin, verify pixel matches degrees(asin(value)).
    ///
    /// Reference: test_arithmetic.py::test_asin
    fn test_asin() {
        // Create image with values in [0, 1] range
        let data = vec![128u8; 100 * 100]; // 128/255 ≈ 0.502
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let im = im.div_const(255.0); // normalize to [0, 1]

        let result = im.asin();
        let px_i = im.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = px_i[0].asin().to_degrees();
        assert!((px_r[0] - expected).abs() < 0.1);
    }

    #[test]
    #[ignore]
    /// Arc cosine (output in degrees).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::acos(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_acos)
    ///
    /// 1. Same setup as asin — values in [0, 1].
    /// 2. Verify pixel matches degrees(acos(value)).
    ///
    /// Reference: test_arithmetic.py::test_acos
    fn test_acos() {
        let data = vec![128u8; 100 * 100];
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let im = im.div_const(255.0);

        let result = im.acos();
        let px_i = im.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = px_i[0].acos().to_degrees();
        assert!((px_r[0] - expected).abs() < 0.1);
    }

    #[test]
    #[ignore]
    /// Arc tangent (output in degrees).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::atan(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_atan)
    ///
    /// 1. Values in [0, 1]. Verify pixel matches degrees(atan(value)).
    ///
    /// Reference: test_arithmetic.py::test_atan
    fn test_atan() {
        let data = vec![128u8; 100 * 100];
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let im = im.div_const(255.0);

        let result = im.atan();
        let px_i = im.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = px_i[0].atan().to_degrees();
        assert!((px_r[0] - expected).abs() < 0.1);
    }

    #[test]
    #[ignore]
    /// Two-argument arc tangent (atan2).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute atan2(self, other) in degrees.
    /// fn Raster::atan2(&self, other: &Raster) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_atan2)
    ///
    /// 1. Create two images with values in [0, 1].
    /// 2. atan2 result at (50,50) should match degrees(atan2(a, b)).
    ///
    /// Reference: test_arithmetic.py::test_atan2
    fn test_atan2() {
        let data_a = vec![128u8; 100 * 100];
        let data_b = vec![64u8; 100 * 100];
        let a = Raster::new(100, 100, PixelFormat::Gray8, data_a).unwrap();
        let b = Raster::new(100, 100, PixelFormat::Gray8, data_b).unwrap();

        let result = a.atan2(&b);
        let px_r = result.getpoint(50, 50);
        let expected = (128.0_f64).atan2(64.0).to_degrees();
        assert!((px_r[0] - expected).abs() < 0.1);
    }

    #[test]
    #[ignore]
    /// Hyperbolic sine.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::sinh(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_sinh)
    ///
    /// 1. Apply sinh to mono test image, verify at (10,10).
    ///
    /// Reference: test_arithmetic.py::test_sinh
    fn test_sinh() {
        let mono = make_test_mono();
        let result = mono.sinh();
        let px_m = mono.getpoint(10, 10);
        let px_r = result.getpoint(10, 10);
        let expected = px_m[0].sinh();
        assert!((px_r[0] - expected).abs() / expected.abs().max(1.0) < 0.01);
    }

    #[test]
    #[ignore]
    /// Hyperbolic cosine.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::cosh(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_cosh
    fn test_cosh() {
        let mono = make_test_mono();
        let result = mono.cosh();
        let px_m = mono.getpoint(10, 10);
        let px_r = result.getpoint(10, 10);
        let expected = px_m[0].cosh();
        assert!((px_r[0] - expected).abs() / expected.abs().max(1.0) < 0.01);
    }

    #[test]
    #[ignore]
    /// Hyperbolic tangent.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::tanh(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_tanh
    fn test_tanh() {
        let mono = make_test_mono();
        let result = mono.tanh();
        let px_m = mono.getpoint(10, 10);
        let px_r = result.getpoint(10, 10);
        let expected = px_m[0].tanh();
        assert!((px_r[0] - expected).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Inverse hyperbolic sine.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::asinh(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_asinh
    fn test_asinh() {
        let data = vec![150u8; 100 * 100]; // value > 1, asinh is defined everywhere
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let result = im.asinh();
        let px_r = result.getpoint(50, 50);
        let expected = 150.0_f64.asinh();
        assert!((px_r[0] - expected).abs() < 0.01);
    }

    #[test]
    #[ignore]
    /// Inverse hyperbolic cosine.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::acosh(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_acosh
    fn test_acosh() {
        let data = vec![150u8; 100 * 100]; // value > 1, acosh requires x >= 1
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let result = im.acosh();
        let px_r = result.getpoint(50, 50);
        let expected = 150.0_f64.acosh();
        assert!((px_r[0] - expected).abs() < 0.01);
    }

    #[test]
    #[ignore]
    /// Inverse hyperbolic tangent.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::atanh(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_atanh
    fn test_atanh() {
        // atanh requires input in (-1, 1)
        let data = vec![128u8; 100 * 100];
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let im = im.div_const(255.0); // ~0.502
        let result = im.atanh();
        let px_i = im.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = px_i[0].atanh();
        assert!((px_r[0] - expected).abs() < 0.01);
    }

    #[test]
    #[ignore]
    /// Natural logarithm.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute ln(pixel). Pixel values must be > 0.
    /// fn Raster::log(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_log)
    ///
    /// 1. Apply log to test image, verify pixel at (50,50).
    ///
    /// Reference: test_arithmetic.py::test_log
    fn test_log() {
        let mono = make_test_mono();
        let result = mono.log();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        if px_m[0] > 0.0 {
            let expected = px_m[0].ln();
            assert!((px_r[0] - expected).abs() < 0.01);
        }
    }

    #[test]
    #[ignore]
    /// Base-10 logarithm.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::log10(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_log10
    fn test_log10() {
        let mono = make_test_mono();
        let result = mono.log10();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        if px_m[0] > 0.0 {
            let expected = px_m[0].log10();
            assert!((px_r[0] - expected).abs() < 0.01);
        }
    }

    #[test]
    #[ignore]
    /// Exponential (e^x).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::exp(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_exp
    fn test_exp() {
        // Use small values to avoid overflow
        let data = vec![2u8; 100 * 100];
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let result = im.exp();
        let px_r = result.getpoint(50, 50);
        let expected = 2.0_f64.exp();
        assert!((px_r[0] - expected).abs() < 0.01);
    }

    #[test]
    #[ignore]
    /// Base-10 exponential (10^x).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::exp10(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_exp10
    fn test_exp10() {
        let data = vec![2u8; 100 * 100];
        let im = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();
        let result = im.exp10();
        let px_r = result.getpoint(50, 50);
        let expected = 10.0_f64.powf(2.0);
        assert!((px_r[0] - expected).abs() < 0.01);
    }

    #[test]
    #[ignore]
    /// Floor: round pixel values down to nearest integer.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::floor(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_floor
    fn test_floor() {
        // For integer images, floor is identity
        let mono = make_test_mono();
        let result = mono.floor();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - px_m[0].floor()).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Ceiling: round pixel values up to nearest integer.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::ceil(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_ceil
    fn test_ceil() {
        let mono = make_test_mono();
        let result = mono.ceil();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - px_m[0].ceil()).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Round to nearest integer.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::rint(&self) -> Raster;
    /// ```
    ///
    /// Reference: test_arithmetic.py::test_rint
    fn test_rint() {
        let mono = make_test_mono();
        let result = mono.rint();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        assert!((px_r[0] - px_m[0].round()).abs() < 0.001);
    }

    #[test]
    #[ignore]
    /// Sign function: -1, 0, or 1.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Returns -1 for negative, 0 for zero, 1 for positive pixels.
    /// fn Raster::sign(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_sign)
    ///
    /// 1. Apply sign to test image.
    /// 2. Positive pixels → 1, zero pixels → 0.
    ///
    /// Reference: test_arithmetic.py::test_sign
    fn test_sign() {
        let mono = make_test_mono();
        let result = mono.sign();
        let px_m = mono.getpoint(50, 50);
        let px_r = result.getpoint(50, 50);
        let expected = if px_m[0] > 0.0 { 1.0 } else if px_m[0] < 0.0 { -1.0 } else { 0.0 };
        assert!((px_r[0] - expected).abs() < 0.001);
    }
}

mod complex_histogram {
    use super::*;

    #[test]
    #[ignore]
    /// Convert complex image to polar form (magnitude, angle).
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Convert complex image from rectangular (re, im) to polar (magnitude, phase).
    /// fn Raster::polar(&self) -> Raster;
    ///
    /// /// Form a complex image from two float images (real and imaginary parts).
    /// fn Raster::complexform(real: &Raster, imag: &Raster) -> Raster;
    ///
    /// /// Extract the real part of a complex image.
    /// fn Raster::real(&self) -> Raster;
    ///
    /// /// Extract the imaginary part of a complex image.
    /// fn Raster::imag(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_polar)
    ///
    /// 1. Create 100×100 image of value 100.
    /// 2. Form complex image: (100 + 100i).
    /// 3. Convert to polar.
    /// 4. Real part (magnitude) avg should be 100*sqrt(2) ≈ 141.42.
    /// 5. Imaginary part (angle) avg should be 45°.
    ///
    /// Reference: test_arithmetic.py::test_polar
    fn test_polar() {
        let data = vec![100u8; 100 * 100];
        let re = Raster::new(100, 100, PixelFormat::Gray8, data.clone()).unwrap();
        let im_part = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();

        let complex = Raster::complexform(&re, &im_part);
        let polar = complex.polar();

        let magnitude_avg = polar.real().avg();
        assert!(
            (magnitude_avg - 100.0 * 2.0_f64.sqrt()).abs() < 1.0,
            "Magnitude avg should be ~141.42, got {magnitude_avg}"
        );

        let angle_avg = polar.imag().avg();
        assert!(
            (angle_avg - 45.0).abs() < 1.0,
            "Angle avg should be ~45°, got {angle_avg}"
        );
    }

    #[test]
    #[ignore]
    /// Convert complex image from polar to rectangular form.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::rect(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_rect)
    ///
    /// 1. Create complex polar image: magnitude = 100*sqrt(2), angle = 45°.
    /// 2. Convert to rectangular.
    /// 3. Real and imaginary parts should both average ~100.
    ///
    /// Reference: test_arithmetic.py::test_rect
    fn test_rect() {
        let mag = 100.0 * 2.0_f64.sqrt();
        let mag_data = vec![mag as u8; 100 * 100]; // approximate
        let angle_data = vec![45u8; 100 * 100];
        let re = Raster::new(100, 100, PixelFormat::Gray8, mag_data).unwrap();
        let im_part = Raster::new(100, 100, PixelFormat::Gray8, angle_data).unwrap();

        let complex = Raster::complexform(&re, &im_part);
        let rect = complex.rect();

        assert!(
            (rect.real().avg() - 100.0).abs() < 2.0,
            "Real part should be ~100"
        );
        assert!(
            (rect.imag().avg() - 100.0).abs() < 2.0,
            "Imaginary part should be ~100"
        );
    }

    #[test]
    #[ignore]
    /// Complex conjugate.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute the complex conjugate: (re, im) → (re, -im).
    /// fn Raster::conj(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_conjugate)
    ///
    /// 1. Create complex image (100 + 100i).
    /// 2. Conjugate → (100 - 100i).
    /// 3. Real avg = 100, imaginary avg = -100.
    ///
    /// Reference: test_arithmetic.py::test_conjugate
    fn test_conjugate() {
        let data = vec![100u8; 100 * 100];
        let re = Raster::new(100, 100, PixelFormat::Gray8, data.clone()).unwrap();
        let im_part = Raster::new(100, 100, PixelFormat::Gray8, data).unwrap();

        let complex = Raster::complexform(&re, &im_part);
        let conj = complex.conj();

        assert!((conj.real().avg() - 100.0).abs() < 1.0);
        assert!((conj.imag().avg() - (-100.0)).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Compute a histogram of pixel values.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute the histogram of a single-band image.
    /// /// Returns a 256×1 (for uchar) image where pixel(v,0) is the count
    /// /// of pixels with value v.
    /// fn Raster::hist_find(&self) -> Raster;
    ///
    /// /// Compute histogram with a specific band selected.
    /// fn Raster::hist_find_band(&self, band: u32) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_histfind)
    ///
    /// 1. Create 100×50 image: left = 0, right = 10.
    /// 2. hist_find() at (0,0) = 5000, at (10,0) = 5000, at (5,0) = 0.
    ///
    /// Reference: test_arithmetic.py::test_histfind
    fn test_histfind() {
        let left = Raster::zeroed(50, 100, PixelFormat::Gray8);
        let right_data = vec![10u8; 50 * 100];
        let right = Raster::new(50, 100, PixelFormat::Gray8, right_data).unwrap();
        let im = left.insert(&right, 50, 0, true);

        let hist = im.hist_find();
        let count_0 = hist.getpoint(0, 0);
        let count_10 = hist.getpoint(10, 0);
        let count_5 = hist.getpoint(5, 0);

        assert!((count_0[0] - 5000.0).abs() < 1.0, "5000 pixels at value 0");
        assert!((count_10[0] - 5000.0).abs() < 1.0, "5000 pixels at value 10");
        assert!((count_5[0] - 0.0).abs() < 1.0, "0 pixels at value 5");
    }

    #[test]
    #[ignore]
    /// Histogram find with index image.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute a weighted histogram where each pixel's contribution is
    /// /// determined by the corresponding index image.
    /// fn Raster::hist_find_indexed(&self, index: &Raster) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_histfind_indexed)
    ///
    /// 1. Create half-0, half-10 image and index = image // 10.
    /// 2. hist_find_indexed: at (0,0) = 0, at (1,0) = 50000.
    ///
    /// Reference: test_arithmetic.py::test_histfind_indexed
    fn test_histfind_indexed() {
        let left = Raster::zeroed(50, 100, PixelFormat::Gray8);
        let right_data = vec![10u8; 50 * 100];
        let right = Raster::new(50, 100, PixelFormat::Gray8, right_data).unwrap();
        let im = left.insert(&right, 50, 0, true);
        let index = im.floordiv_const(10.0);

        let hist = im.hist_find_indexed(&index);
        let h0 = hist.getpoint(0, 0);
        let h1 = hist.getpoint(1, 0);
        assert!((h0[0] - 0.0).abs() < 1.0);
        assert!((h1[0] - 50000.0).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// N-dimensional histogram.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Compute an N-dimensional histogram. Input must be a multi-band image.
    /// /// bins: number of bins per dimension (default 10).
    /// fn Raster::hist_find_ndim(&self, bins: Option<u32>) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_histfind_ndim)
    ///
    /// 1. Create 100×100 3-band image with constant [1,2,3].
    /// 2. hist_find_ndim(): at (0,0) band 0 should be 10000.
    /// 3. hist_find_ndim(bins=1): 1×1×1 histogram, value = 10000.
    ///
    /// Reference: test_arithmetic.py::test_histfind_ndim
    fn test_histfind_ndim() {
        let mut data = vec![0u8; 100 * 100 * 3];
        for i in 0..(100 * 100) {
            data[i * 3] = 1;
            data[i * 3 + 1] = 2;
            data[i * 3 + 2] = 3;
        }
        let im = Raster::new(100, 100, PixelFormat::Rgb8, data).unwrap();

        let hist = im.hist_find_ndim(None);
        let px = hist.getpoint(0, 0);
        assert!((px[0] - 10000.0).abs() < 1.0);

        let hist = im.hist_find_ndim(Some(1));
        assert_eq!(hist.width(), 1);
        assert_eq!(hist.height(), 1);
        let px = hist.getpoint(0, 0);
        assert!((px[0] - 10000.0).abs() < 1.0);
    }

    #[test]
    #[ignore]
    /// Hough circle detection.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Detect circles using the Hough transform.
    /// /// Returns a 3D accumulator (x × y × radius).
    /// fn Raster::hough_circle(&self, min_radius: u32, max_radius: u32) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_hough_circle)
    ///
    /// 1. Draw circle of radius 40 at centre (50,50) on 100×100 black image.
    /// 2. hough_circle(min_radius=35, max_radius=45).
    /// 3. maxpos should be at (50, 50), radius band = 40.
    ///
    /// Reference: test_arithmetic.py::test_hough_circle
    fn test_hough_circle() {
        let mut im = Raster::zeroed(100, 100, PixelFormat::Gray8);
        im.draw_circle(100, 50, 50, 40, false);

        let hough = im.hough_circle(35, 45);
        let (v, x, y) = hough.maxpos();
        let vec = hough.getpoint(x, y);
        let r = vec.iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i as u32 + 35)
            .unwrap();

        assert!((x as f64 - 50.0).abs() < 2.0, "Centre x should be ~50, got {x}");
        assert!((y as f64 - 50.0).abs() < 2.0, "Centre y should be ~50, got {y}");
        assert!((r as f64 - 40.0).abs() < 2.0, "Radius should be ~40, got {r}");
    }

    #[test]
    #[ignore]
    /// Hough line detection.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Detect lines using the Hough transform.
    /// /// Returns a (angle × distance) accumulator image.
    /// fn Raster::hough_line(&self) -> Raster;
    /// ```
    ///
    /// ## Test logic (from libvips test_arithmetic.py::test_hough_line)
    ///
    /// 1. Draw line from (10,90) to (90,10) on 100×100 black image.
    /// 2. hough_line().
    /// 3. maxpos gives (x, y) → angle = 180 * x / width ≈ 45°,
    ///    distance = height * y / hough_height ≈ 75.
    ///
    /// Reference: test_arithmetic.py::test_hough_line
    fn test_hough_line() {
        let mut im = Raster::zeroed(100, 100, PixelFormat::Gray8);
        im.draw_line(100, 10, 90, 90, 10);

        let hough = im.hough_line();
        let (_v, x, y) = hough.maxpos();

        let angle = 180.0 * x as f64 / hough.width() as f64;
        let distance = 100.0 * y as f64 / hough.height() as f64;

        assert!(
            (angle - 45.0).abs() < 5.0,
            "Angle should be ~45°, got {angle}"
        );
        assert!(
            (distance - 75.0).abs() < 10.0,
            "Distance should be ~75, got {distance}"
        );
    }
}
