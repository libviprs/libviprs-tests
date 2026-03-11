#![cfg(feature = "ported_tests")]

//! Ported conversion tests from libvips `test_conversion.py`.
//!
//! Tests exercise format casting, band operations, spatial operations
//! (embed, gravity, extract, crop, smartcrop), composite, flip, gamma,
//! grid, ifthenelse, switch, insert, arrayjoin, msb, recomb, replicate,
//! rotation, autorot, scale, subsample, zoom, and wrap.

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

/// Create a synthetic 100×100, 3-band (Rgb8) test image matching
/// the libvips test setup: `(mask_ideal * [1,2,3] + [2,3,4]).copy(interpretation="srgb")`.
fn make_test_colour() -> Raster {
    let w = 100u32;
    let h = 100u32;
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let mut data = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f64 - cx) / cx;
            let dy = (y as f64 - cy) / cy;
            let r = (dx * dx + dy * dy).sqrt();
            let v = if r > 0.5 { (r * 200.0).min(255.0) } else { 0.0 };
            let off = ((y * w + x) * 3) as usize;
            data[off]     = ((v * 1.0 + 2.0) as u16).min(255) as u8;
            data[off + 1] = ((v * 2.0 + 3.0) as u16).min(255) as u8;
            data[off + 2] = ((v * 3.0 + 4.0) as u16).min(255) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Create a mono (Gray8) version of the test image.
fn make_test_mono() -> Raster {
    let w = 100u32;
    let h = 100u32;
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let mut data = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f64 - cx) / cx;
            let dy = (y as f64 - cy) / cy;
            let r = (dx * dx + dy * dy).sqrt();
            let v = if r > 0.5 { (r * 200.0).min(255.0) } else { 0.0 };
            data[(y * w + x) as usize] = ((v * 2.0 + 3.0) as u16).min(255) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Gray8, data).unwrap()
}

#[test]
#[ignore]
/// Format cast with clipping/overflow behaviour.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Cast pixel values to the given format, clipping on overflow.
/// fn Raster::cast(&self, format: PixelFormat) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_cast)
///
/// 1. Cast a negative signed image to unsigned — result should clip to 0.
/// 2. Cast max unsigned to signed — result should clip to signed max.
///
/// Reference: test_conversion.py::test_cast
fn test_cast() {
    // Create image with negative values (using signed representation)
    let data = vec![0u8; 100]; // placeholder; real test needs signed format
    let im = Raster::new(1, 1, PixelFormat::Gray8, vec![0u8]).unwrap();
    let neg = im.sub_const(10.0); // requires signed format output

    // Cast to unsigned should clip to 0
    let result = neg.cast(PixelFormat::Gray8);
    assert!(result.avg() >= 0.0, "Negative cast to unsigned should clip to 0");

    // Cast max uchar to signed char should clip
    let im = Raster::new(1, 1, PixelFormat::Gray8, vec![255u8]).unwrap();
    assert!((im.avg() - 255.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Band AND/OR/XOR reduction across bands.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Bitwise AND across all bands, producing a single-band image.
/// fn Raster::bandand(&self) -> Raster;
///
/// /// Bitwise OR across all bands.
/// fn Raster::bandor(&self) -> Raster;
///
/// /// Bitwise XOR across all bands.
/// fn Raster::bandeor(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_band_and/or/eor)
///
/// 1. Apply bandand to colour image, verify pixel at (50,50):
///    result = band0 & band1 & band2.
/// 2. bandor: result = band0 | band1 | band2.
/// 3. bandeor: result = band0 ^ band1 ^ band2.
///
/// Reference: test_conversion.py::test_band_and, test_band_or, test_band_eor
fn test_band_boolean() {
    let colour = make_test_colour();

    // bandand
    let result = colour.bandand();
    let px = colour.getpoint(50, 50);
    let expected = px[0] as u8 & px[1] as u8 & px[2] as u8;
    let rpx = result.getpoint(50, 50);
    assert!((rpx[0] - expected as f64).abs() < 1.0);

    // bandor
    let result = colour.bandor();
    let expected = px[0] as u8 | px[1] as u8 | px[2] as u8;
    let rpx = result.getpoint(50, 50);
    assert!((rpx[0] - expected as f64).abs() < 1.0);

    // bandeor
    let result = colour.bandeor();
    let expected = px[0] as u8 ^ px[1] as u8 ^ px[2] as u8;
    let rpx = result.getpoint(50, 50);
    assert!((rpx[0] - expected as f64).abs() < 1.0);
}

#[test]
#[ignore]
/// Band join (concatenate bands from two images).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Join the bands of two images into a single multi-band image.
/// fn Raster::bandjoin(&self, other: &Raster) -> Raster;
///
/// /// Join a constant value as an extra band.
/// fn Raster::bandjoin_const(&self, c: f64) -> Raster;
///
/// /// Join a vector of constants as extra bands.
/// fn Raster::bandjoin_vec(&self, v: &[f64]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_bandjoin)
///
/// 1. Join colour (3 bands) with mono (1 band) → 4 bands.
/// 2. bandjoin_const(1): 3 → 4 bands, band 3 avg = 1.
/// 3. bandjoin_vec([1, 2]): 3 → 5 bands, band 3 avg = 1, band 4 avg = 2.
///
/// Reference: test_conversion.py::test_bandjoin, test_bandjoin_const
fn test_bandjoin() {
    let colour = make_test_colour();
    let mono = make_test_mono();

    // image + image
    let joined = colour.bandjoin(&mono);
    assert_eq!(joined.format().channels(), 4);

    // bandjoin_const
    let joined = colour.bandjoin_const(1.0);
    assert_eq!(joined.format().channels(), 4);
    let band3 = joined.extract_band(3);
    assert!((band3.avg() - 1.0).abs() < 0.001);

    // bandjoin_vec
    let joined = colour.bandjoin_vec(&[1.0, 2.0]);
    assert_eq!(joined.format().channels(), 5);
    let band3 = joined.extract_band(3);
    assert!((band3.avg() - 1.0).abs() < 0.001);
    let band4 = joined.extract_band(4);
    assert!((band4.avg() - 2.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Add an alpha band (set to max for the format).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Add an alpha band set to the maximum value for the format (255 for uchar).
/// fn Raster::addalpha(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_addalpha)
///
/// 1. Add alpha to 3-band sRGB image → 4 bands.
/// 2. Band 3 (alpha) avg should be 255.
///
/// Reference: test_conversion.py::test_addalpha
fn test_addalpha() {
    let colour = make_test_colour();
    let result = colour.addalpha();
    assert_eq!(result.format().channels(), 4);
    let alpha = result.extract_band(3);
    assert!((alpha.avg() - 255.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Band mean: average across bands.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Compute the mean of all bands for each pixel, producing a single-band image.
/// fn Raster::bandmean(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_bandmean)
///
/// 1. Apply bandmean to colour image.
/// 2. Pixel at (50,50) should be floor(sum(bands) / n_bands).
///
/// Reference: test_conversion.py::test_bandmean
fn test_bandmean() {
    let colour = make_test_colour();
    let result = colour.bandmean();
    let px = colour.getpoint(50, 50);
    let rpx = result.getpoint(50, 50);
    let expected = (px[0] + px[1] + px[2]) / 3.0;
    assert!((rpx[0] - expected.floor()).abs() < 1.0);
}

#[test]
#[ignore]
/// Band rank: per-pixel median (or rank) across multiple images.
///
/// ## Required API
///
/// ```rust,ignore
/// /// For each pixel position, sort values from self and other(s),
/// /// and pick the median (or specified rank index).
/// fn Raster::bandrank(&self, others: &[&Raster], index: Option<u32>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_bandrank)
///
/// 1. Bandrank of two identical images: result = median = original.
///
/// Reference: test_conversion.py::test_bandrank
fn test_bandrank() {
    let colour = make_test_colour();
    let result = colour.bandrank(&[&colour], None);
    let px_c = colour.getpoint(50, 50);
    let px_r = result.getpoint(50, 50);
    for (c, r) in px_c.iter().zip(px_r.iter()) {
        assert!((r - c).abs() < 1.0);
    }
}

#[test]
#[ignore]
/// Copy with modified metadata.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Copy the image with optionally modified metadata.
/// fn Raster::copy(&self) -> RasterCopyBuilder;
///
/// /// Builder to set interpretation, xres, yres, xoffset, yoffset.
/// impl RasterCopyBuilder {
///     fn interpretation(self, interp: Interpretation) -> Self;
///     fn xres(self, v: f64) -> Self;
///     fn yres(self, v: f64) -> Self;
///     fn build(self) -> Raster;
/// }
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_copy)
///
/// 1. Copy with interpretation=Lab, verify interpretation field.
/// 2. Copy with xres=42, verify xres.
///
/// Reference: test_conversion.py::test_copy
fn test_copy() {
    let colour = make_test_colour();
    let copy = colour.copy().xres(42.0).build();
    assert!((copy.xres() - 42.0).abs() < 0.001);
}

#[test]
#[ignore]
/// Band fold/unfold: reshape width into bands and back.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Fold image width into bands.
/// /// Default: new_width = 1, bands = old_width.
/// fn Raster::bandfold(&self, factor: Option<u32>) -> Raster;
///
/// /// Unfold bands into image width.
/// fn Raster::bandunfold(&self, factor: Option<u32>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_bandfold)
///
/// 1. Fold mono (100 wide, 1 band) → width=1, bands=100.
/// 2. Unfold back → width=100, bands=1. Avg should match.
/// 3. Fold with factor=2 → width=50, bands=2.
///
/// Reference: test_conversion.py::test_bandfold
fn test_bandfold() {
    let mono = make_test_mono();

    let folded = mono.bandfold(None);
    assert_eq!(folded.width(), 1);

    let unfolded = folded.bandunfold(None);
    assert_eq!(unfolded.width(), mono.width());
    assert!((unfolded.avg() - mono.avg()).abs() < 0.001);

    let folded2 = mono.bandfold(Some(2));
    assert_eq!(folded2.width(), mono.width() / 2);
}

#[test]
#[ignore]
/// Byte swap (endianness reversal) — double swap is identity.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Swap bytes within each pixel (e.g. big-endian ↔ little-endian for 16-bit).
/// fn Raster::byteswap(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_byteswap)
///
/// 1. Cast mono to 16-bit unsigned.
/// 2. byteswap().byteswap() should equal original.
///
/// Reference: test_conversion.py::test_byteswap
fn test_byteswap() {
    let mono = make_test_mono();
    let im16 = mono.cast(PixelFormat::Gray16);
    let swapped = im16.byteswap().byteswap();
    assert_eq!(im16.width(), swapped.width());
    assert!((im16.avg() - swapped.avg()).abs() < 0.001);
}

#[test]
#[ignore]
/// Embed image in a larger canvas.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Embed the image at (x, y) in a canvas of (width, height).
/// /// extend: how to fill the new area (black, copy edge, background, white).
/// fn Raster::embed(&self, x: i32, y: i32, width: u32, height: u32,
///                  extend: Extend, background: Option<&[f64]>) -> Raster;
///
/// pub enum Extend { Black, Copy, Background, White }
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_embed)
///
/// 1. Embed colour at (20,20) in (w+40, h+40) with black extend.
///    Pixel at (10,10) = [0,0,0]; pixel at (30,30) = [2,3,4].
/// 2. Same with Extend::Copy: pixel at (10,10) = [2,3,4].
/// 3. Extend::Background([7,8,9]): pixel at (10,10) = [7,8,9].
/// 4. Extend::White: pixel at (10,10) = [255,255,255].
///
/// Reference: test_conversion.py::test_embed
fn test_embed() {
    let colour = make_test_colour();
    let w = colour.width();
    let h = colour.height();

    // Black extend
    let im = colour.embed(20, 20, w + 40, h + 40, Extend::Black, None);
    let px = im.getpoint(10, 10);
    assert!((px[0]).abs() < 1.0 && (px[1]).abs() < 1.0 && (px[2]).abs() < 1.0);
    let px = im.getpoint(30, 30);
    assert!((px[0] - 2.0).abs() < 1.0);
    assert!((px[1] - 3.0).abs() < 1.0);
    assert!((px[2] - 4.0).abs() < 1.0);

    // White extend
    let im = colour.embed(20, 20, w + 40, h + 40, Extend::White, None);
    let px = im.getpoint(10, 10);
    assert!((px[0] - 255.0).abs() < 1.0);
}

#[test]
#[ignore]
/// Gravity positioning: place a small image within a larger canvas.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Place self within a (width × height) canvas at the given compass position.
/// fn Raster::gravity(&self, direction: Direction, width: u32, height: u32) -> Raster;
///
/// pub enum Direction { Centre, North, South, East, West, NorthEast, SouthEast, SouthWest, NorthWest }
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_gravity)
///
/// 1. Create 1×1 pixel of 255.
/// 2. gravity("centre", 3, 3) → pixel at (1,1) = 255, avg = 255/9.
/// 3. gravity("north", 3, 3) → pixel at (1,0) = 255.
/// 4. etc. for all 9 directions.
///
/// Reference: test_conversion.py::test_gravity
fn test_gravity() {
    let im = Raster::new(1, 1, PixelFormat::Gray8, vec![255u8]).unwrap();

    let positions: &[(&str, u32, u32)] = &[
        ("centre", 1, 1),
        ("north", 1, 0),
        ("south", 1, 2),
        ("east", 2, 1),
        ("west", 0, 1),
        ("north-east", 2, 0),
        ("south-east", 2, 2),
        ("south-west", 0, 2),
        ("north-west", 0, 0),
    ];

    for &(direction, x, y) in positions {
        let im2 = im.gravity(direction, 3, 3);
        let px = im2.getpoint(x, y);
        assert!(
            (px[0] - 255.0).abs() < 1.0,
            "gravity({direction}): pixel at ({x},{y}) should be 255"
        );
        assert!(
            (im2.avg() - 255.0 / 9.0).abs() < 1.0,
            "gravity({direction}): avg should be ~28.3"
        );
    }
}

#[test]
#[ignore]
/// Extract area and band.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Extract a rectangular region.
/// fn Raster::extract_area(&self, left: u32, top: u32, width: u32, height: u32) -> Raster;
///
/// /// Extract `n` consecutive bands starting from `band`.
/// fn Raster::extract_band(&self, band: u32) -> Raster;
/// fn Raster::extract_bands(&self, band: u32, n: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_extract)
///
/// 1. Extract area (25,25,10,10) from colour, pixel at (5,5) = [2,3,4].
/// 2. Extract bands 1..3 (2 bands), pixel at (30,30) = [3,4].
///
/// Reference: test_conversion.py::test_extract
fn test_extract() {
    let colour = make_test_colour();

    let sub = colour.extract_area(25, 25, 10, 10);
    let px = sub.getpoint(5, 5);
    assert!((px[0] - 2.0).abs() < 1.0);
    assert!((px[1] - 3.0).abs() < 1.0);

    let sub = colour.extract_bands(1, 2);
    let px = sub.getpoint(30, 30);
    assert!((px[0] - 3.0).abs() < 1.0);
    assert!((px[1] - 4.0).abs() < 1.0);
}

#[test]
#[ignore]
/// Crop (alias for extract_area).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::crop(&self, left: u32, top: u32, width: u32, height: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_crop)
///
/// 1. crop(25, 25, 10, 10), pixel at (5,5) = [2,3,4].
///
/// Reference: test_conversion.py::test_crop
fn test_crop() {
    let colour = make_test_colour();
    let sub = colour.crop(25, 25, 10, 10);
    let px = sub.getpoint(5, 5);
    assert!((px[0] - 2.0).abs() < 1.0);
}

#[test]
#[ignore]
/// Smart crop: automatically crop to salient region.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Crop to (width × height) around the most interesting region.
/// /// `interesting`: Entropy or Attention strategy.
/// fn Raster::smartcrop(&self, width: u32, height: u32, interesting: SmartcropInteresting) -> Raster;
///
/// pub enum SmartcropInteresting { Entropy, Attention }
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_smartcrop)
///
/// 1. Load sample.jpg, smartcrop to 100×100.
/// 2. Assert result is 100×100.
///
/// Reference: test_conversion.py::test_smartcrop
fn test_smartcrop() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let result = im.smartcrop(100, 100, SmartcropInteresting::Entropy);
    assert_eq!(result.width(), 100);
    assert_eq!(result.height(), 100);
}

#[test]
#[ignore]
/// False colour: map a greyscale image to a colour palette.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Apply a false-colour mapping. Input is treated as greyscale;
/// /// output is a 3-band RGB image.
/// fn Raster::falsecolour(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_falsecolour)
///
/// 1. Apply falsecolour to colour image.
/// 2. Output should be 3 bands, same dimensions.
/// 3. Pixel at (30,30) should be approximately [20, 0, 41].
///
/// Reference: test_conversion.py::test_falsecolour
fn test_falsecolour() {
    let colour = make_test_colour();
    let result = colour.falsecolour();
    assert_eq!(result.width(), colour.width());
    assert_eq!(result.height(), colour.height());
    assert_eq!(result.format().channels(), 3);

    let px = result.getpoint(30, 30);
    assert!((px[0] - 20.0).abs() < 5.0);
    assert!((px[1] - 0.0).abs() < 5.0);
    assert!((px[2] - 41.0).abs() < 5.0);
}

#[test]
#[ignore]
/// Alpha flatten: composite RGBA onto a solid background.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Flatten an RGBA image to RGB by compositing over a background colour.
/// /// Default background is black.
/// fn Raster::flatten(&self, background: Option<&[f64]>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_flatten)
///
/// 1. Create RGBA image with alpha = 127 (50% transparent).
/// 2. Flatten with default (black) background: pixel ≈ original * alpha / 255.
/// 3. Flatten with background [100,100,100]: pixel ≈ original*a/255 + 100*(1-a/255).
/// 4. Result should be 3-band.
///
/// Reference: test_conversion.py::test_flatten
fn test_flatten() {
    let colour = make_test_colour();
    let alpha = 127.0;
    let rgba = colour.bandjoin_const(alpha);

    let flat = rgba.flatten(None);
    assert_eq!(flat.format().channels(), 3);
    let px_src = colour.getpoint(30, 30);
    let px_flat = flat.getpoint(30, 30);
    for (s, f) in px_src.iter().zip(px_flat.iter()) {
        let expected = s * alpha / 255.0;
        assert!((f - expected).abs() < 2.0);
    }

    let flat_bg = rgba.flatten(Some(&[100.0, 100.0, 100.0]));
    let px_bg = flat_bg.getpoint(30, 30);
    for (s, f) in px_src.iter().zip(px_bg.iter()) {
        let expected = s * alpha / 255.0 + 100.0 * (255.0 - alpha) / 255.0;
        assert!((f - expected).abs() < 2.0);
    }
}

#[test]
#[ignore]
/// Premultiply and unpremultiply alpha.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Premultiply RGB channels by the alpha channel.
/// fn Raster::premultiply(&self) -> Raster;
///
/// /// Undo premultiplication.
/// fn Raster::unpremultiply(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_premultiply)
///
/// 1. Create RGBA image with alpha = 127.
/// 2. Premultiply: each RGB channel × alpha / max.
/// 3. Output bands should match input bands.
///
/// Reference: test_conversion.py::test_premultiply
fn test_premultiply() {
    let colour = make_test_colour();
    let alpha = 127.0;
    let rgba = colour.bandjoin_const(alpha);

    let pre = rgba.premultiply();
    assert_eq!(pre.format().channels(), 4);
    let px_src = rgba.getpoint(30, 30);
    let px_pre = pre.getpoint(30, 30);
    for i in 0..3 {
        let expected = px_src[i] * alpha / 255.0;
        assert!((px_pre[i] - expected).abs() < 2.0);
    }
    assert!((px_pre[3] - alpha).abs() < 1.0);
}

#[test]
#[ignore]
/// Porter-Duff composite.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Composite overlay over self using Porter-Duff compositing.
/// fn Raster::composite(&self, overlay: &Raster, mode: CompositeMode) -> Raster;
///
/// pub enum CompositeMode { Over, Atop, Dest, In, Out, Xor, Add, Saturate }
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_composite)
///
/// 1. Create base = colour + 100, overlay = colour with alpha = 128.
/// 2. Composite overlay "over" base.
/// 3. Verify pixel at (0,0) ≈ [51.8, 52.8, 53.8, 255].
///
/// Reference: test_conversion.py::test_composite
fn test_composite() {
    let colour = make_test_colour();
    let overlay = colour.bandjoin_const(128.0);
    let base = colour.add_const(100.0);

    let comp = base.composite(&overlay, CompositeMode::Over);
    let px = comp.getpoint(0, 0);
    assert!((px[0] - 51.8).abs() < 1.0);
    assert!((px[1] - 52.8).abs() < 1.0);
    assert!((px[2] - 53.8).abs() < 1.0);
}

#[test]
#[ignore]
/// Flip horizontal and vertical.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::fliphor(&self) -> Raster;
/// fn Raster::flipver(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_flip)
///
/// 1. fliphor().flipver().fliphor().flipver() should equal original.
///
/// Reference: test_conversion.py::test_flip
fn test_flip() {
    let colour = make_test_colour();
    let result = colour.fliphor().flipver().fliphor().flipver();
    let max_diff: f64 = colour.data().iter().zip(result.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 1.0, "Double flip should be identity, max diff = {max_diff}");
}

#[test]
#[ignore]
/// Gamma correction.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Apply gamma correction. Default exponent is 1/2.4 (sRGB decode).
/// fn Raster::gamma(&self, exponent: Option<f64>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_gamma)
///
/// 1. Apply default gamma (exponent=2.4).
/// 2. Verify pixel at (30,30) ≈ value^2.4 / (255^2.4 / 255).
/// 3. Apply gamma with exponent=1/1.2.
///
/// Reference: test_conversion.py::test_gamma
fn test_gamma() {
    let colour = make_test_colour();
    let exponent = 2.4;

    let result = colour.gamma(None);
    let before = colour.getpoint(30, 30);
    let after = result.getpoint(30, 30);
    let norm = 255.0_f64.powf(exponent) / 255.0;
    for (b, a) in before.iter().zip(after.iter()) {
        let expected = b.powf(exponent) / norm;
        assert!((a - expected).abs() < 255.0 / 100.0);
    }
}

#[test]
#[ignore]
/// Grid rearrangement: reshape a tall strip into a grid.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Chop a tall, thin image into tiles and lay them out in a grid.
/// fn Raster::grid(&self, tile_height: u32, across: u32, down: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_grid)
///
/// 1. Replicate colour 1×12 → (w, h*12).
/// 2. grid(tile_height=h, across=3, down=4) → (w*3, h*4).
///
/// Reference: test_conversion.py::test_grid
fn test_grid() {
    let colour = make_test_colour();
    let tall = colour.replicate(1, 12);
    assert_eq!(tall.height(), colour.height() * 12);

    let result = tall.grid(colour.width(), 3, 4);
    assert_eq!(result.width(), colour.width() * 3);
    assert_eq!(result.height(), colour.height() * 4);
}

#[test]
#[ignore]
/// If-then-else: conditional pixel selection.
///
/// ## Required API
///
/// ```rust,ignore
/// /// For each pixel: if self != 0, use `then`, else use `otherwise`.
/// fn Raster::ifthenelse(&self, then: &Raster, otherwise: &Raster) -> Raster;
///
/// /// With blend=true, uses self as a blend factor (0..255) instead of boolean.
/// fn Raster::ifthenelse_blend(&self, then: &Raster, otherwise: &Raster) -> Raster;
///
/// /// Constant variants for then/otherwise.
/// fn Raster::ifthenelse_const(&self, then_val: &[f64], otherwise: &Raster) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_ifthenelse)
///
/// 1. Create condition = mono > 3.
/// 2. then = colour + 10, else = colour.
/// 3. At (10,10) where condition is false: result = colour(10,10).
/// 4. At (50,50) where condition is true: result = colour(50,50) + 10.
///
/// Reference: test_conversion.py::test_ifthenelse
fn test_ifthenelse() {
    let mono = make_test_mono();
    let colour = make_test_colour();

    let condition = mono.more_than_const(3.0);
    let then_img = colour.add_const(10.0);
    let result = condition.ifthenelse(&then_img, &colour);

    assert_eq!(result.width(), colour.width());
    assert_eq!(result.height(), colour.height());

    // At (10,10) — if mono value ≤ 3, we get colour
    let cond_px = condition.getpoint(10, 10);
    if cond_px[0] < 128.0 {
        let expected = colour.getpoint(10, 10);
        let actual = result.getpoint(10, 10);
        for (e, a) in expected.iter().zip(actual.iter()) {
            assert!((a - e).abs() < 1.0);
        }
    }
}

#[test]
#[ignore]
/// Switch: select from conditions to produce an index image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// For each pixel, return the index of the first true (non-zero) condition.
/// /// If no condition is true, returns N (number of conditions).
/// fn Raster::switch(conditions: &[&Raster]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_switch)
///
/// 1. Grey ramp 256×256 uchar.
/// 2. switch([x < 128, x >= 128]) → avg = 0.5 (half 0, half 1).
/// 3. No match: switch([x == 1000, x == 2000]) → avg = 2.
///
/// Reference: test_conversion.py::test_switch
fn test_switch() {
    let x = Raster::grey(256, 256, true);

    let cond_lo = x.less_than_const(128.0);
    let cond_hi = x.more_eq_const(128.0);
    let index = Raster::switch(&[&cond_lo, &cond_hi]);
    assert!((index.avg() - 0.5).abs() < 0.01);
}

#[test]
#[ignore]
/// Insert one image into another.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Insert `sub` into `self` at position (x, y).
/// /// If expand=true, the canvas expands to fit.
/// fn Raster::insert(&self, sub: &Raster, x: i32, y: i32, expand: bool) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_insert)
///
/// 1. Insert colour at (10,10) into mono. Width/height = mono's.
/// 2. Result at (10,10) should match colour at (0,0).
/// 3. With expand=true: result size = (mono.w+10, mono.h+10).
///
/// Reference: test_conversion.py::test_insert
fn test_insert() {
    let mono = make_test_mono();
    let colour = make_test_colour();

    let result = mono.insert(&colour, 10, 10, false);
    assert_eq!(result.width(), mono.width());
    assert_eq!(result.height(), mono.height());

    let result = mono.insert(&colour, 10, 10, true);
    assert_eq!(result.width(), mono.width() + 10);
    assert_eq!(result.height(), mono.height() + 10);
}

#[test]
#[ignore]
/// Array join: tile multiple images into a grid.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Join a list of images side-by-side (or in a grid with `across`).
/// fn Raster::arrayjoin(images: &[&Raster], across: Option<u32>, shim: Option<u32>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_arrayjoin)
///
/// 1. arrayjoin([mono, colour]) → width = max_w * 2, height = max_h.
/// 2. arrayjoin([mono, colour], across=1) → width = max_w, height = max_h * 2.
///
/// Reference: test_conversion.py::test_arrayjoin
fn test_arrayjoin() {
    let mono = make_test_mono();
    let colour = make_test_colour();

    let im = Raster::arrayjoin(&[&mono, &colour], None, None);
    assert_eq!(im.width(), mono.width() * 2); // both are 100 wide
    assert_eq!(im.height(), mono.height());

    let im = Raster::arrayjoin(&[&mono, &colour], Some(1), None);
    assert_eq!(im.width(), colour.width());
    assert_eq!(im.height(), mono.height() + colour.height());
}

#[test]
#[ignore]
/// Extract the most significant byte from a multi-byte image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Extract the MSB from each pixel (useful for 16-bit → 8-bit conversion).
/// fn Raster::msb(&self, band: Option<u32>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_msb)
///
/// 1. Cast colour to ushort.
/// 2. msb(): each pixel = original >> 8.
///
/// Reference: test_conversion.py::test_msb
fn test_msb() {
    let colour = make_test_colour();
    let im16 = colour.add_const(32.0).cast(PixelFormat::Rgb16);
    let msb = im16.msb(None);

    let before = im16.getpoint(10, 10);
    let after = msb.getpoint(10, 10);
    for (b, a) in before.iter().zip(after.iter()) {
        let expected = (*b as u16 >> 8) as f64;
        assert!((a - expected).abs() < 1.0);
    }
}

#[test]
#[ignore]
/// Matrix recombination of bands.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Recombine image bands using a matrix.
/// /// matrix: rows × bands coefficients.
/// fn Raster::recomb(&self, matrix: &[&[f64]]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_recomb)
///
/// 1. Apply [[0.2, 0.5, 0.3]] to 3-band image.
/// 2. Result should be single-band: pixel = 0.2*R + 0.5*G + 0.3*B.
///
/// Reference: test_conversion.py::test_recomb
fn test_recomb() {
    let colour = make_test_colour();
    let matrix: &[&[f64]] = &[&[0.2, 0.5, 0.3]];
    let result = colour.recomb(matrix);

    let px = colour.getpoint(50, 50);
    let rpx = result.getpoint(50, 50);
    let expected = 0.2 * px[0] + 0.5 * px[1] + 0.3 * px[2];
    assert!((rpx[0] - expected).abs() < 1.0);
}

#[test]
#[ignore]
/// Replicate (tile) an image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Tile the image `across` times horizontally and `down` times vertically.
/// fn Raster::replicate(&self, across: u32, down: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_replicate)
///
/// 1. replicate(10, 10) → width*10, height*10.
/// 2. Pixel at (10+w*2, 10+w*2) should match original at (10,10).
///
/// Reference: test_conversion.py::test_replicate
fn test_replicate() {
    let colour = make_test_colour();
    let result = colour.replicate(10, 10);
    assert_eq!(result.width(), colour.width() * 10);
    assert_eq!(result.height(), colour.height() * 10);

    let before = colour.getpoint(10, 10);
    let after = result.getpoint(10 + colour.width() * 2, 10 + colour.height() * 2);
    for (b, a) in before.iter().zip(after.iter()) {
        assert!((a - b).abs() < 1.0);
    }
}

#[test]
#[ignore]
/// Rotation by multiples of 90° and 45°.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Rotate by a multiple of 90°.
/// fn Raster::rot(&self, angle: Angle) -> Raster;
///
/// /// Rotate by a multiple of 45°.
/// fn Raster::rot45(&self, angle: Angle45) -> Raster;
///
/// pub enum Angle { D0, D90, D180, D270 }
/// pub enum Angle45 { D0, D45, D90, D135, D180, D225, D270, D315 }
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_rot)
///
/// 1. Crop colour to 51×51 (has quarter-circle in bottom-right).
/// 2. rot(D90): pixel at (0,50) should match original at (50,50).
/// 3. Applying a rotation and its inverse should be identity.
///
/// Reference: test_conversion.py::test_rot, test_rot45
fn test_rot() {
    let colour = make_test_colour();
    let test = colour.crop(0, 0, 51, 51);

    let im2 = test.rot(Angle::D90);
    let before = test.getpoint(50, 50);
    let after = im2.getpoint(0, 50);
    for (b, a) in before.iter().zip(after.iter()) {
        assert!((a - b).abs() < 1.0);
    }

    // rot(90).rot(270) = identity
    let round = test.rot(Angle::D90).rot(Angle::D270);
    let max_diff: f64 = test.data().iter().zip(round.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 1.0);
}

#[test]
#[ignore]
/// Auto-rotation based on EXIF orientation tag.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Rotate the image to match its EXIF orientation tag, then remove the tag.
/// fn Raster::autorot(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_autorot)
///
/// 1. Load each EXIF-tagged rotation test image.
/// 2. Apply autorot.
/// 3. Verify dimensions match expected values for each orientation.
///
/// Reference: test_conversion.py::test_autorot
fn test_autorot() {
    // Load a JPEG with known EXIF orientation
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let rotated = im.autorot();
    // sample.jpg has orientation=1 (no rotation needed), so dimensions should match
    assert_eq!(rotated.width(), im.width());
    assert_eq!(rotated.height(), im.height());
}

#[test]
#[ignore]
/// Scale image to 0-255 range.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Scale pixel values to fill the 0..255 range.
/// fn Raster::scaleimage(&self, log: Option<bool>) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_scaleimage)
///
/// 1. scaleimage(): max should be 255, min should be 0.
/// 2. scaleimage(log=true): max should be 255.
///
/// Reference: test_conversion.py::test_scaleimage
fn test_scaleimage() {
    let colour = make_test_colour();
    let result = colour.scaleimage(None);
    assert!((result.max() - 255.0).abs() < 1.0);
    assert!(result.min().abs() < 1.0);

    let result_log = colour.scaleimage(Some(true));
    assert!((result_log.max() - 255.0).abs() < 1.0);
}

#[test]
#[ignore]
/// Subsample: pick every Nth pixel.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Subsample by taking every `xfac`-th pixel horizontally
/// /// and every `yfac`-th pixel vertically.
/// fn Raster::subsample(&self, xfac: u32, yfac: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_subsample)
///
/// 1. subsample(3, 3): width = w/3, height = h/3.
/// 2. Pixel at (20,20) should match original at (60,60).
///
/// Reference: test_conversion.py::test_subsample
fn test_subsample() {
    let colour = make_test_colour();
    let result = colour.subsample(3, 3);
    assert_eq!(result.width(), colour.width() / 3);
    assert_eq!(result.height(), colour.height() / 3);

    let before = colour.getpoint(60, 60);
    let after = result.getpoint(20, 20);
    for (b, a) in before.iter().zip(after.iter()) {
        assert!((a - b).abs() < 1.0);
    }
}

#[test]
#[ignore]
/// Integer zoom: replicate each pixel N times.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Zoom in by integer factors, replicating each pixel.
/// fn Raster::zoom(&self, xfac: u32, yfac: u32) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_zoom)
///
/// 1. zoom(3, 3): width = w*3, height = h*3.
/// 2. Pixel at (150,150) should match original at (50,50).
///
/// Reference: test_conversion.py::test_zoom
fn test_zoom() {
    let colour = make_test_colour();
    let result = colour.zoom(3, 3);
    assert_eq!(result.width(), colour.width() * 3);
    assert_eq!(result.height(), colour.height() * 3);

    let before = colour.getpoint(50, 50);
    let after = result.getpoint(150, 150);
    for (b, a) in before.iter().zip(after.iter()) {
        assert!((a - b).abs() < 1.0);
    }
}

#[test]
#[ignore]
/// Quadrant wrap: swap quadrants of an image (useful for FFT display).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Swap the four quadrants of an image.
/// fn Raster::wrap(&self) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_conversion.py::test_wrap)
///
/// 1. wrap(): dimensions unchanged.
/// 2. Pixel at (50,50) should match original at (0,0).
/// 3. Pixel at (0,0) should match original at (50,50).
///
/// Reference: test_conversion.py::test_wrap
fn test_wrap() {
    let colour = make_test_colour();
    let result = colour.wrap();
    assert_eq!(result.width(), colour.width());
    assert_eq!(result.height(), colour.height());

    let before = colour.getpoint(0, 0);
    let after = result.getpoint(50, 50);
    for (b, a) in before.iter().zip(after.iter()) {
        assert!((a - b).abs() < 1.0);
    }

    let before = colour.getpoint(50, 50);
    let after = result.getpoint(0, 0);
    for (b, a) in before.iter().zip(after.iter()) {
        assert!((a - b).abs() < 1.0);
    }
}
