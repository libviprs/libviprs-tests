#![cfg(feature = "ported_tests")]

//! Ported drawing operation tests from libvips `test_draw.py`.
//!
//! These tests exercise in-place mutation drawing primitives: circles, lines,
//! rectangles, flood fill, image compositing, mask drawing, and smudge.
//! All tests use synthetically generated black images (as libvips does).

use libviprs::{PixelFormat, Raster};

/// Create a black (all-zero) single-band u8 image.
fn black_image(w: u32, h: u32) -> Raster {
    Raster::zeroed(w, h, PixelFormat::Gray8).unwrap()
}

/// Read pixel value at (x, y) from a single-band image.
fn pixel_at(im: &Raster, x: u32, y: u32) -> u8 {
    let view = im.region(x, y, 1, 1).unwrap();
    view.pixel(0, 0).unwrap()[0]
}

/// Compute the absolute max difference between two same-sized single-band images.
fn abs_max_diff(a: &Raster, b: &Raster) -> u8 {
    assert_eq!(a.width(), b.width());
    assert_eq!(a.height(), b.height());
    a.data()
        .iter()
        .zip(b.data().iter())
        .map(|(&x, &y)| (x as i16 - y as i16).unsigned_abs() as u8)
        .max()
        .unwrap_or(0)
}

#[test]
#[ignore]
/// Draw an outline and filled circle.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Draw a circle outline on the image in-place.
/// /// `ink`: pixel value(s) to draw.
/// /// `cx`, `cy`: center coordinates.
/// /// `radius`: circle radius in pixels.
/// fn Raster::draw_circle(&mut self, ink: &[u8], cx: i32, cy: i32, radius: i32);
///
/// /// Draw a filled circle on the image in-place.
/// fn Raster::draw_circle_filled(&mut self, ink: &[u8], cx: i32, cy: i32, radius: i32);
/// ```
///
/// ## Test logic (from libvips test_draw.py::test_draw_circle)
///
/// Outline circle:
/// 1. Create 100×100 black image.
/// 2. Draw circle outline: ink=100, center=(50,50), radius=25.
/// 3. Pixel at (25, 50) should be 100 (on the circle).
/// 4. Pixel at (26, 50) should be 0 (just inside, not filled).
///
/// Filled circle:
/// 1. Create 100×100 black image.
/// 2. Draw filled circle: ink=100, center=(50,50), radius=25.
/// 3. Pixel at (25, 50) should be 100 (on the circle boundary).
/// 4. Pixel at (26, 50) should be 100 (inside, filled).
/// 5. Pixel at (24, 50) should be 0 (outside the circle).
///
/// Reference: test_draw.py::test_draw_circle
fn test_draw_circle() {
    // Outline circle
    let mut im = black_image(100, 100);
    im.draw_circle(&[100], 50, 50, 25);
    assert_eq!(pixel_at(&im, 25, 50), 100, "Pixel on circle should be ink value");
    assert_eq!(pixel_at(&im, 26, 50), 0, "Pixel inside outline-only circle should be 0");

    // Filled circle
    let mut im = black_image(100, 100);
    im.draw_circle_filled(&[100], 50, 50, 25);
    assert_eq!(pixel_at(&im, 25, 50), 100, "Pixel on filled circle boundary");
    assert_eq!(pixel_at(&im, 26, 50), 100, "Pixel inside filled circle");
    assert_eq!(pixel_at(&im, 24, 50), 0, "Pixel outside filled circle");
}

#[test]
#[ignore]
/// Flood-fill an outlined circle to match a filled circle.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Flood-fill starting at (x, y) with the given ink value.
/// /// Fills all connected pixels that match the pixel value at (x, y).
/// fn Raster::draw_flood(&mut self, ink: &[u8], x: i32, y: i32);
/// ```
///
/// ## Test logic (from libvips test_draw.py::test_draw_flood)
///
/// 1. Create a 100×100 black image, draw circle outline (ink=100, center=50,50, r=25).
/// 2. Flood-fill the interior at (50, 50) with ink=100.
/// 3. Create another black image and draw a filled circle (same params).
/// 4. The two images should be identical (max abs diff = 0).
///
/// Reference: test_draw.py::test_draw_flood
fn test_draw_flood() {
    let mut im = black_image(100, 100);
    im.draw_circle(&[100], 50, 50, 25);
    im.draw_flood(&[100], 50, 50);

    let mut im2 = black_image(100, 100);
    im2.draw_circle_filled(&[100], 50, 50, 25);

    assert_eq!(abs_max_diff(&im, &im2), 0, "Flood-filled outline should match filled circle");
}

#[test]
#[ignore]
/// Flood-fill with out-of-bounds coordinates should return an error.
///
/// ## Required API
///
/// `Raster::draw_flood` should return `Result<(), DrawError>` and fail for
/// coordinates outside the image bounds.
///
/// ## Test logic (from libvips test_draw.py::test_draw_flood_out_of_bounds)
///
/// 1. Create a 100×100 black image.
/// 2. Attempt flood fill at (200, 50) — should error.
/// 3. Attempt flood fill at (50, 200) — should error.
/// 4. Attempt flood fill at (-1, 50) — should error.
/// 5. Attempt flood fill at (50, -1) — should error.
///
/// Reference: test_draw.py::test_draw_flood_out_of_bounds
fn test_draw_flood_oob() {
    let mut im = black_image(100, 100);

    assert!(im.draw_flood(&[100], 200, 50).is_err(), "x=200 should be out of bounds");
    assert!(im.draw_flood(&[100], 50, 200).is_err(), "y=200 should be out of bounds");
    assert!(im.draw_flood(&[100], -1, 50).is_err(), "x=-1 should be out of bounds");
    assert!(im.draw_flood(&[100], 50, -1).is_err(), "y=-1 should be out of bounds");
}

#[test]
#[ignore]
/// Draw (composite) one image onto another.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Draw `overlay` onto `self` at position (x, y), replacing pixels.
/// fn Raster::draw_image(&mut self, overlay: &Raster, x: i32, y: i32);
/// ```
///
/// ## Test logic (from libvips test_draw.py::test_draw_image)
///
/// 1. Create a 51×51 black image, draw filled circle (ink=100, center=25,25, r=25).
/// 2. Create a 100×100 black image, draw that circle image at (25, 25).
/// 3. Create another 100×100 black image, draw filled circle (ink=100, center=50,50, r=25).
/// 4. The two 100×100 images should be identical.
///
/// Reference: test_draw.py::test_draw_image
fn test_draw_image() {
    let mut small = black_image(51, 51);
    small.draw_circle_filled(&[100], 25, 25, 25);

    let mut im2 = black_image(100, 100);
    im2.draw_image(&small, 25, 25);

    let mut im3 = black_image(100, 100);
    im3.draw_circle_filled(&[100], 50, 50, 25);

    assert_eq!(abs_max_diff(&im2, &im3), 0, "draw_image should match direct filled circle");
}

#[test]
#[ignore]
/// Draw a line.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Draw a 1-pixel-wide line from (x1,y1) to (x2,y2) with the given ink.
/// fn Raster::draw_line(&mut self, ink: &[u8], x1: i32, y1: i32, x2: i32, y2: i32);
/// ```
///
/// ## Test logic (from libvips test_draw.py::test_draw_line)
///
/// 1. Create a 100×100 black image.
/// 2. Draw a horizontal line: ink=100, from (0,0) to (100,0).
/// 3. Pixel at (0, 0) should be 100.
/// 4. Pixel at (0, 1) should be 0 (line is only on row 0).
///
/// Reference: test_draw.py::test_draw_line
fn test_draw_line() {
    let mut im = black_image(100, 100);
    im.draw_line(&[100], 0, 0, 100, 0);

    assert_eq!(pixel_at(&im, 0, 0), 100, "Line pixel should be ink value");
    assert_eq!(pixel_at(&im, 0, 1), 0, "Pixel below line should be 0");
}

#[test]
#[ignore]
/// Draw using a mask (alpha-weighted compositing).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Draw onto `self` using a mask image for opacity weighting.
/// /// `ink`: the base colour to draw.
/// /// `mask`: single-band image where each pixel scales the ink value (0=transparent, 255=opaque).
/// /// `x`, `y`: position to place the mask on self.
/// fn Raster::draw_mask(&mut self, ink: &[u8], mask: &Raster, x: i32, y: i32);
/// ```
///
/// ## Test logic (from libvips test_draw.py::test_draw_mask)
///
/// 1. Create a 51×51 black mask, draw filled circle (ink=128, center=25,25, r=25).
/// 2. Create a 100×100 black image, draw_mask with ink=200, mask, at (25,25).
/// 3. Create another 100×100 black image, draw filled circle (ink=100, center=50,50, r=25).
///    (ink=200 * mask_value(128)/255 ≈ 100).
/// 4. The two images should be identical.
///
/// Reference: test_draw.py::test_draw_mask
fn test_draw_mask() {
    let mut mask = black_image(51, 51);
    mask.draw_circle_filled(&[128], 25, 25, 25);

    let mut im = black_image(100, 100);
    im.draw_mask(&[200], &mask, 25, 25);

    let mut im2 = black_image(100, 100);
    im2.draw_circle_filled(&[100], 50, 50, 25);

    assert_eq!(abs_max_diff(&im, &im2), 0, "Mask-drawn image should match");
}

#[test]
#[ignore]
/// Draw a filled rectangle.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Draw a filled rectangle from (left, top) with the given width and height.
/// fn Raster::draw_rect_filled(&mut self, ink: &[u8], left: i32, top: i32, width: i32, height: i32);
/// ```
///
/// ## Test logic (from libvips test_draw.py::test_draw_rect)
///
/// 1. Create a 100×100 black image, draw filled rect: ink=100, at (25,25), 50×50.
/// 2. Create another 100×100 black image, fill the same area by drawing
///    horizontal lines: for y in 25..75, draw line ink=100 from (25,y) to (74,y).
/// 3. The two images should be identical.
///
/// Reference: test_draw.py::test_draw_rect
fn test_draw_rect() {
    let mut im = black_image(100, 100);
    im.draw_rect_filled(&[100], 25, 25, 50, 50);

    let mut im2 = black_image(100, 100);
    for y in 25..75 {
        im2.draw_line(&[100], 25, y, 74, y);
    }

    assert_eq!(abs_max_diff(&im, &im2), 0, "Filled rect should match line-drawn region");
}

#[test]
#[ignore]
/// Smudge (blur/average) a rectangular region in-place.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Average (smudge) pixels in a rectangular region.
/// /// The region at (left, top, width, height) is replaced by its local average.
/// fn Raster::draw_smudge(&mut self, left: i32, top: i32, width: i32, height: i32);
/// ```
///
/// ## Test logic (from libvips test_draw.py::test_draw_smudge)
///
/// 1. Create a 100×100 black image with a filled white circle (ink=100, center=50,50, r=25).
/// 2. Smudge the region (10, 10, 50, 50).
/// 3. Extract the original region (10, 10, 50, 50) from the un-smudged image.
/// 4. Draw that extracted region back onto the smudged image at (10, 10).
/// 5. The result should equal the original (smudge + restore = identity).
///
/// Reference: test_draw.py::test_draw_smudge
fn test_draw_smudge() {
    let mut im = black_image(100, 100);
    im.draw_circle_filled(&[100], 50, 50, 25);

    let mut im2 = im.clone();
    im2.draw_smudge(10, 10, 50, 50);

    // Extract the original un-smudged region
    let patch = im.extract(10, 10, 50, 50).unwrap();

    // Draw it back to undo the smudge
    let mut im4 = im2.clone();
    im4.draw_image(&patch, 10, 10);

    assert_eq!(
        abs_max_diff(&im4, &im),
        0,
        "Restoring the original region after smudge should recover the original image"
    );
}
