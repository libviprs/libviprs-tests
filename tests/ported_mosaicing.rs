#![cfg(feature = "ported_tests")]

//! Ported mosaicing tests from libvips `test_mosaicing.py`.
//!
//! These tests exercise image merging and mosaicing operations using the
//! libvips reference mosaic fixture images (cd1–cd4 pairs).

use std::path::Path;

use libviprs::{decode_file, Raster};

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

/// Mosaic file pairs: (left, right) for horizontal join.
const MOSAIC_FILES: [&str; 8] = [
    "cd1.1.jpg", "cd1.2.jpg",
    "cd2.1.jpg", "cd2.2.jpg",
    "cd3.1.jpg", "cd3.2.jpg",
    "cd4.1.jpg", "cd4.2.jpg",
];

/// Tie-point marks for each mosaic file pair (x, y).
const MOSAIC_MARKS: [(i32, i32); 8] = [
    (489, 140), (66, 141),
    (453, 40),  (15, 43),
    (500, 122), (65, 121),
    (495, 58),  (40, 57),
];

/// Vertical tie-point marks for joining horizontal strips.
const MOSAIC_VERTICAL_MARKS: [(i32, i32); 6] = [
    (388, 44),  (364, 346),
    (384, 17),  (385, 629),
    (527, 42),  (503, 959),
];

#[test]
#[ignore]
/// Left-right merge of two overlapping images.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Merge two images horizontally (left-right) with the given overlap offset.
/// /// `dx`: horizontal displacement of `other` relative to `self` (negative = overlap).
/// /// `dy`: vertical displacement.
/// /// Returns a merged image whose width = self.width + other.width - overlap.
/// fn Raster::merge(&self, other: &Raster, direction: MergeDirection, dx: i32, dy: i32) -> Raster;
///
/// pub enum MergeDirection { Horizontal, Vertical }
/// ```
///
/// ## Test logic (from libvips test_mosaicing.py::test_lrmerge)
///
/// 1. Load `cd1.1.jpg` (left) and `cd1.2.jpg` (right) from reference fixtures.
/// 2. Merge horizontally with dx = 10 - left.width, dy = 0 (10-pixel overlap).
/// 3. Assert joined width = left.width + right.width - 10.
/// 4. Assert joined height = max(left.height, right.height).
///
/// Reference: test_mosaicing.py::test_lrmerge
fn test_lrmerge() {
    let left = decode_file(&ref_image("cd1.1.jpg")).unwrap();
    let right = decode_file(&ref_image("cd1.2.jpg")).unwrap();

    let dx = 10 - left.width() as i32;
    let join = left.merge(&right, MergeDirection::Horizontal, dx, 0);

    assert_eq!(join.width(), left.width() + right.width() - 10);
    assert_eq!(join.height(), left.height().max(right.height()));
}

#[test]
#[ignore]
/// Top-bottom merge of two overlapping images.
///
/// ## Required API
///
/// Same `Raster::merge` as above with `MergeDirection::Vertical`.
///
/// ## Test logic (from libvips test_mosaicing.py::test_tbmerge)
///
/// 1. Load `cd1.1.jpg` (top) and `cd2.1.jpg` (bottom).
/// 2. Merge vertically with dx = 0, dy = 10 - top.height.
/// 3. Assert joined width = max(top.width, bottom.width).
/// 4. Assert joined height = top.height + bottom.height - 10.
///
/// Reference: test_mosaicing.py::test_tbmerge
fn test_tbmerge() {
    let top = decode_file(&ref_image("cd1.1.jpg")).unwrap();
    let bottom = decode_file(&ref_image("cd2.1.jpg")).unwrap();

    let dy = 10 - top.height() as i32;
    let join = top.merge(&bottom, MergeDirection::Vertical, 0, dy);

    assert_eq!(join.width(), top.width().max(bottom.width()));
    assert_eq!(join.height(), top.height() + bottom.height() - 10);
}

#[test]
#[ignore]
/// Left-right mosaic with feature-point matching.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Mosaic two images using tie-points for alignment.
/// /// `direction`: Horizontal or Vertical.
/// /// `(ref_x, ref_y)`: tie-point in self.
/// /// `(sec_x, sec_y)`: corresponding tie-point in `other`.
/// /// Returns the mosaiced image with automatic blending at the seam.
/// fn Raster::mosaic(
///     &self,
///     other: &Raster,
///     direction: MergeDirection,
///     ref_x: i32, ref_y: i32,
///     sec_x: i32, sec_y: i32,
/// ) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_mosaicing.py::test_lrmosaic)
///
/// 1. Load cd1.1.jpg and cd1.2.jpg.
/// 2. Mosaic horizontally using marks: self=(left.width-30, 0), other=(30, 0).
/// 3. Assert joined width = 1014 and height = 379.
///
/// Reference: test_mosaicing.py::test_lrmosaic
fn test_lrmosaic() {
    let left = decode_file(&ref_image("cd1.1.jpg")).unwrap();
    let right = decode_file(&ref_image("cd1.2.jpg")).unwrap();

    let ref_x = left.width() as i32 - 30;
    let join = left.mosaic(&right, MergeDirection::Horizontal, ref_x, 0, 30, 0);

    assert_eq!(join.width(), 1014);
    assert_eq!(join.height(), 379);
}

#[test]
#[ignore]
/// Top-bottom mosaic with feature-point matching.
///
/// ## Required API
///
/// Same `Raster::mosaic` with `MergeDirection::Vertical`.
///
/// ## Test logic (from libvips test_mosaicing.py::test_tbmosaic)
///
/// 1. Load cd1.1.jpg (top) and cd2.1.jpg (bottom).
/// 2. Mosaic vertically using marks: self=(0, top.height-30), other=(0, 30).
/// 3. Assert joined width = 542 and height = 688.
///
/// Reference: test_mosaicing.py::test_tbmosaic
fn test_tbmosaic() {
    let top = decode_file(&ref_image("cd1.1.jpg")).unwrap();
    let bottom = decode_file(&ref_image("cd2.1.jpg")).unwrap();

    let ref_y = top.height() as i32 - 30;
    let join = top.mosaic(&bottom, MergeDirection::Vertical, 0, ref_y, 0, 30);

    assert_eq!(join.width(), 542);
    assert_eq!(join.height(), 688);
}

#[test]
#[ignore]
/// Full multi-image mosaic construction from 8 images (4 horizontal pairs
/// joined vertically).
///
/// ## Required API
///
/// Same `Raster::mosaic` used iteratively.
///
/// ## Test logic (from libvips test_mosaicing.py::test_mosaic)
///
/// 1. For each pair of files (cd{N}.1.jpg, cd{N}.2.jpg):
///    - Load both images.
///    - Mosaic horizontally using MOSAIC_MARKS tie-points.
/// 2. Join horizontal strips vertically using MOSAIC_VERTICAL_MARKS.
/// 3. Assert final mosaiced image: width=1005, height=1295, bands=1.
///
/// Reference: test_mosaicing.py::test_mosaic
fn test_mosaic() {
    let mut mosaiced: Option<Raster> = None;

    for i in (0..MOSAIC_FILES.len()).step_by(2) {
        let im = decode_file(&ref_image(MOSAIC_FILES[i])).unwrap();
        let sec_im = decode_file(&ref_image(MOSAIC_FILES[i + 1])).unwrap();

        let (ref_x, ref_y) = MOSAIC_MARKS[i];
        let (sec_x, sec_y) = MOSAIC_MARKS[i + 1];

        let horizontal_part = im.mosaic(
            &sec_im,
            MergeDirection::Horizontal,
            ref_x, ref_y,
            sec_x, sec_y,
        );

        mosaiced = Some(match mosaiced {
            None => horizontal_part,
            Some(prev) => {
                let vi = i - 2;
                let (vref_x, vref_y) = MOSAIC_VERTICAL_MARKS[vi + 1];
                let (vsec_x, vsec_y) = MOSAIC_VERTICAL_MARKS[vi];
                prev.mosaic(
                    &horizontal_part,
                    MergeDirection::Vertical,
                    vref_x, vref_y,
                    vsec_x, vsec_y,
                )
            }
        });
    }

    let result = mosaiced.unwrap();
    assert_eq!(result.width(), 1005);
    assert_eq!(result.height(), 1295);
    // Mosaic images are grayscale
    assert_eq!(result.format().channels(), 1);
}

#[test]
#[ignore]
/// Global balance: adjust brightness across a multi-image mosaic to
/// remove seam artifacts.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Apply global brightness balancing to a mosaic image.
/// /// The input must have been constructed via `mosaic()` so that it
/// /// retains the blend metadata needed for balancing.
/// /// Returns the balanced image (float format).
/// fn Raster::global_balance(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_mosaicing.py::test_globalbalance)
///
/// 1. Build the full mosaic (same as test_mosaic above).
/// 2. Call `global_balance()` on the result.
/// 3. Assert width=1005, height=1295, bands=1.
/// 4. The output should be float format.
///
/// Reference: test_mosaicing.py::test_globalbalance
fn test_globalbalance() {
    let mut mosaiced: Option<Raster> = None;

    for i in (0..MOSAIC_FILES.len()).step_by(2) {
        let im = decode_file(&ref_image(MOSAIC_FILES[i])).unwrap();
        let sec_im = decode_file(&ref_image(MOSAIC_FILES[i + 1])).unwrap();

        let (ref_x, ref_y) = MOSAIC_MARKS[i];
        let (sec_x, sec_y) = MOSAIC_MARKS[i + 1];

        let horizontal_part = im.mosaic(
            &sec_im,
            MergeDirection::Horizontal,
            ref_x, ref_y,
            sec_x, sec_y,
        );

        mosaiced = Some(match mosaiced {
            None => horizontal_part,
            Some(prev) => {
                let vi = i - 2;
                let (vref_x, vref_y) = MOSAIC_VERTICAL_MARKS[vi + 1];
                let (vsec_x, vsec_y) = MOSAIC_VERTICAL_MARKS[vi];
                prev.mosaic(
                    &horizontal_part,
                    MergeDirection::Vertical,
                    vref_x, vref_y,
                    vsec_x, vsec_y,
                )
            }
        });
    }

    let balanced = mosaiced.unwrap().global_balance();

    assert_eq!(balanced.width(), 1005);
    assert_eq!(balanced.height(), 1295);
    assert_eq!(balanced.format().channels(), 1);
    // Global balance should produce float output
    // (represented as f32 pixels in libviprs)
}
