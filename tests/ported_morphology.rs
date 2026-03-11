#![cfg(feature = "ported_tests")]

//! Ported morphology tests from libvips `test_morphology.py`.
//!
//! These tests exercise binary morphological operations (erosion, dilation),
//! connected-component labeling, line counting, and rank filtering.
//! All tests use synthetically created images (as libvips does) —
//! no external fixture files needed.

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

/// Create a black (all-zero) single-band u8 image.
fn black_image(w: u32, h: u32) -> Raster {
    Raster::zeroed(w, h, PixelFormat::Gray8).unwrap()
}

#[test]
#[ignore]
/// Count horizontal lines in a binary image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Draw a 1-pixel-wide horizontal line with the given intensity.
/// /// ink: pixel value to draw (0–255 for Gray8).
/// /// (x1, y) to (x2, y).
/// fn Raster::draw_line(&mut self, ink: &[u8], x1: i32, y1: i32, x2: i32, y2: i32);
///
/// /// Count the number of distinct horizontal or vertical line segments
/// /// in a single-band image whose pixel values exceed zero.
/// /// `direction`: Direction::Horizontal | Direction::Vertical.
/// /// Returns the count as f64.
/// fn Raster::countlines(&self, direction: Direction) -> f64;
///
/// pub enum Direction { Horizontal, Vertical }
/// ```
///
/// ## Test logic (from libvips test_morphology.py::test_countlines)
///
/// 1. Create a 100×100 black image.
/// 2. Draw a horizontal white line across row 50 (ink=255, from (0,50) to (100,50)).
/// 3. Call `countlines(Direction::Horizontal)`.
/// 4. Assert the result is 1.
///
/// Reference: test_morphology.py::test_countlines
fn test_countlines() {
    let mut im = black_image(100, 100);

    // Draw a horizontal line: ink=255 from (0,50) to (100,50)
    im.draw_line(&[255], 0, 50, 100, 50);

    let n_lines = im.countlines(Direction::Horizontal);
    assert_eq!(n_lines, 1.0);
}

#[test]
#[ignore]
/// Label connected regions in a binary image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Draw a filled circle on the image in-place.
/// /// ink: pixel intensity, cx/cy: center, radius: circle radius.
/// fn Raster::draw_circle_filled(&mut self, ink: &[u8], cx: i32, cy: i32, radius: i32);
///
/// /// Label connected regions in a single-band image.
/// /// Returns (labelled_image, segment_count) where each connected region
/// /// gets a unique integer label starting from 0 (background).
/// fn Raster::label_regions(&self) -> (Raster, u32);
/// ```
///
/// ## Test logic (from libvips test_morphology.py::test_labelregions)
///
/// 1. Create a 100×100 black image.
/// 2. Draw a filled white circle at (50,50) with radius 25.
/// 3. Call `label_regions()`.
/// 4. Assert segment count is 3 (background, circle interior, circle ring — or
///    equivalently: background=0 outside, ring pixels=1, interior=2 depending on
///    the labeling convention).
/// 5. Assert the max label value in the output is 2.
///
/// Reference: test_morphology.py::test_labelregions
fn test_labelregions() {
    let mut im = black_image(100, 100);
    im.draw_circle_filled(&[255], 50, 50, 25);

    let (mask, segments) = im.label_regions();
    assert_eq!(segments, 3);

    let max_label = mask.data().iter().copied().max().unwrap();
    assert_eq!(max_label, 2);
}

#[test]
#[ignore]
/// Binary erosion with a structuring element.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Erode a single-band image with the given structuring element.
/// /// `kernel` is a 2D matrix where 255 = "must match foreground",
/// /// 128 = "don't care", 0 = "must match background".
/// /// Returns a new eroded image of the same dimensions.
/// fn Raster::erode(&self, kernel: &[&[u8]]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_morphology.py::test_erode)
///
/// 1. Create a 100×100 black image.
/// 2. Draw a filled white circle at (50,50) r=25.
/// 3. Erode with a cross-shaped 3×3 kernel:
///    ```text
///    [128, 255, 128]
///    [255, 255, 255]
///    [128, 255, 128]
///    ```
/// 4. Assert dimensions match the original.
/// 5. Assert the eroded image has a lower average (fewer white pixels).
///
/// Reference: test_morphology.py::test_erode
fn test_erode() {
    let mut im = black_image(100, 100);
    im.draw_circle_filled(&[255], 50, 50, 25);

    let kernel: &[&[u8]] = &[
        &[128, 255, 128],
        &[255, 255, 255],
        &[128, 255, 128],
    ];
    let im2 = im.erode(kernel);

    assert_eq!(im.width(), im2.width());
    assert_eq!(im.height(), im2.height());
    assert_eq!(im.format(), im2.format());

    let avg_before: f64 = im.data().iter().map(|&b| b as f64).sum::<f64>()
        / im.data().len() as f64;
    let avg_after: f64 = im2.data().iter().map(|&b| b as f64).sum::<f64>()
        / im2.data().len() as f64;
    assert!(
        avg_before > avg_after,
        "Erosion should reduce the average pixel value: before={avg_before}, after={avg_after}"
    );
}

#[test]
#[ignore]
/// Binary dilation with a structuring element.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Dilate a single-band image with the given structuring element.
/// /// Same kernel encoding as `erode`.
/// fn Raster::dilate(&self, kernel: &[&[u8]]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_morphology.py::test_dilate)
///
/// 1. Create a 100×100 black image with a filled white circle at (50,50) r=25.
/// 2. Dilate with the same cross-shaped 3×3 kernel.
/// 3. Assert dimensions match.
/// 4. Assert the dilated image has a higher average (more white pixels).
///
/// Reference: test_morphology.py::test_dilate
fn test_dilate() {
    let mut im = black_image(100, 100);
    im.draw_circle_filled(&[255], 50, 50, 25);

    let kernel: &[&[u8]] = &[
        &[128, 255, 128],
        &[255, 255, 255],
        &[128, 255, 128],
    ];
    let im2 = im.dilate(kernel);

    assert_eq!(im.width(), im2.width());
    assert_eq!(im.height(), im2.height());
    assert_eq!(im.format(), im2.format());

    let avg_before: f64 = im.data().iter().map(|&b| b as f64).sum::<f64>()
        / im.data().len() as f64;
    let avg_after: f64 = im2.data().iter().map(|&b| b as f64).sum::<f64>()
        / im2.data().len() as f64;
    assert!(
        avg_after > avg_before,
        "Dilation should increase the average pixel value: before={avg_before}, after={avg_after}"
    );
}

#[test]
#[ignore]
/// Rank filter (median / percentile filter in a window).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Apply a rank filter over a `width`×`height` window, selecting the
/// /// pixel at the given `index` in sorted order (0 = min, width*height-1 = max).
/// /// Index 4 in a 3×3 window = median.
/// fn Raster::rank(&self, width: u32, height: u32, index: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_morphology.py::test_rank)
///
/// 1. Create a 100×100 black image with a filled white circle at (50,50) r=25.
/// 2. Apply rank filter: window 3×3, index 8 (= max of 9 elements).
/// 3. Assert dimensions match.
/// 4. Assert the filtered image has higher average (max filter dilates).
///
/// Reference: test_morphology.py::test_rank
fn test_rank() {
    let mut im = black_image(100, 100);
    im.draw_circle_filled(&[255], 50, 50, 25);

    let im2 = im.rank(3, 3, 8);

    assert_eq!(im.width(), im2.width());
    assert_eq!(im.height(), im2.height());
    assert_eq!(im.format(), im2.format());

    let avg_before: f64 = im.data().iter().map(|&b| b as f64).sum::<f64>()
        / im.data().len() as f64;
    let avg_after: f64 = im2.data().iter().map(|&b| b as f64).sum::<f64>()
        / im2.data().len() as f64;
    assert!(
        avg_after > avg_before,
        "Max rank filter should increase average: before={avg_before}, after={avg_after}"
    );
}
