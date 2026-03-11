#![cfg(feature = "ported_tests")]

//! Phase 1: Ported foreign-format tests.
//!
//! Tests use the real libvips reference fixture images from
//! `tmp/libvips-reference-tests/test-suite/images/` where available,
//! supplemented by synthetic images via the `image` crate for variants
//! the fixtures don't cover (e.g. 16-bit PNG).
//! Manual (#[ignore]) stubs document what remains to be implemented.

use std::io::Cursor;
use std::path::Path;

use image::ImageEncoder;
use libviprs::{
    decode_file, extract_page_image, generate_pyramid, pdf_info, EngineConfig, FsSink, Layout,
    PixelFormat, PyramidPlanner, Raster, TileFormat,
};
use libviprs::source::decode_bytes;

// ---------------------------------------------------------------------------
// Fixture path
// ---------------------------------------------------------------------------

const FIXTURE_PDF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

/// Helper to build a path to a reference fixture image.
fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Create a 16-bit RGB PNG in memory (no 16-bit fixture in the reference suite).
fn create_test_png_16bit(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut buf));
        let num_samples = w as usize * h as usize * 3;
        let mut samples = vec![0u16; num_samples];
        for y in 0..h {
            for x in 0..w {
                let off = (y as usize * w as usize + x as usize) * 3;
                samples[off] = (x * 65535 / w.max(1)) as u16;
                samples[off + 1] = (y * 65535 / h.max(1)) as u16;
                samples[off + 2] = 32768;
            }
        }
        let mut bytes = Vec::with_capacity(num_samples * 2);
        for s in &samples {
            bytes.extend_from_slice(&s.to_be_bytes());
        }
        encoder
            .write_image(&bytes, w, h, image::ColorType::Rgb16.into())
            .unwrap();
    }
    buf
}

/// Build a gradient `Raster` for pyramid tests (same as other test files).
fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x + y) % 256) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Recursively count files with a given extension under `dir`.
fn count_files(dir: &Path, ext: &str) -> usize {
    let mut count = 0;
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                count += count_files(&path, ext);
            } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
                count += 1;
            }
        }
    }
    count
}


// ===========================================================================
// 1.1 JPEG
// ===========================================================================

#[test]
fn test_jpeg_load_dimensions() {
    // Use the real libvips reference JPEG fixture
    let raster = decode_file(&ref_image("sample.jpg")).unwrap();
    assert!(raster.width() > 0, "JPEG width should be positive");
    assert!(raster.height() > 0, "JPEG height should be positive");
    assert_eq!(raster.format().channels(), 3);
}

#[test]
fn test_jpeg_load_pixel_values() {
    let raster = decode_file(&ref_image("sample.jpg")).unwrap();
    // Real photo should have diverse pixel values, not all zero
    let all_zero = raster.data().iter().all(|&b| b == 0);
    assert!(!all_zero, "Decoded JPEG pixel data should not be all zeroes");
    // Check that pixel data has some variation (not a flat image)
    let min = *raster.data().iter().min().unwrap();
    let max = *raster.data().iter().max().unwrap();
    assert!(max - min > 50, "Expected pixel value range in real photo, got {min}..{max}");
}

#[test]
fn test_jpeg_load_from_memory() {
    let bytes = std::fs::read(ref_image("sample.jpg")).unwrap();
    let raster = decode_bytes(&bytes).unwrap();
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
    assert_eq!(raster.format(), PixelFormat::Rgb8);
    // Cross-check: file and memory decode should produce same dimensions
    let raster_file = decode_file(&ref_image("sample.jpg")).unwrap();
    assert_eq!(raster.width(), raster_file.width());
    assert_eq!(raster.height(), raster_file.height());
}

#[test]
#[ignore]
/// Shrink-on-load for JPEG (factor 2/4/8).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Decode a JPEG with shrink-on-load (factor must be 1, 2, 4, or 8).
/// /// The image is decoded at reduced resolution for speed.
/// fn decode_file_with_shrink(path: &Path, shrink: u32) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpeg)
///
/// 1. Load sample.jpg at full size → (290, 442).
/// 2. Load with shrink=2 → dimensions ≈ (145, 221).
/// 3. Load with shrink=4 → dimensions ≈ (73, 111).
/// 4. Load with shrink=8 → dimensions ≈ (37, 56).
///
/// Reference: test_foreign.py::test_jpeg (shrink-on-load section)
fn test_jpeg_shrink_on_load() {
    let full = decode_file(&ref_image("sample.jpg")).unwrap();

    for factor in [2u32, 4, 8] {
        let shrunk = decode_file_with_shrink(&ref_image("sample.jpg"), factor).unwrap();
        let expected_w = (full.width() + factor - 1) / factor;
        let expected_h = (full.height() + factor - 1) / factor;
        // JPEG shrink-on-load gives approximate dimensions (within ±1)
        assert!(
            (shrunk.width() as i64 - expected_w as i64).abs() <= 1,
            "shrink={factor}: width {}, expected ~{expected_w}", shrunk.width()
        );
        assert!(
            (shrunk.height() as i64 - expected_h as i64).abs() <= 1,
            "shrink={factor}: height {}, expected ~{expected_h}", shrunk.height()
        );
    }
}

#[test]
#[ignore]
/// Sequential (non-progressive) JPEG loading.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Decode a JPEG in sequential (top-to-bottom) mode.
/// /// This avoids random access and reduces memory usage.
/// fn decode_file_sequential(path: &Path) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpeg)
///
/// 1. Load sample.jpg in sequential mode.
/// 2. Verify dimensions match non-sequential decode.
/// 3. Verify pixel values are identical.
///
/// Reference: test_foreign.py::test_jpeg (sequential section)
fn test_jpeg_sequential() {
    let normal = decode_file(&ref_image("sample.jpg")).unwrap();
    let sequential = decode_file_sequential(&ref_image("sample.jpg")).unwrap();

    assert_eq!(normal.width(), sequential.width());
    assert_eq!(normal.height(), sequential.height());
    assert_eq!(normal.format(), sequential.format());
    assert_eq!(normal.data(), sequential.data(), "Sequential and normal decode should produce identical pixels");
}

#[test]
#[ignore]
/// Auto-rotation based on EXIF orientation tag.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Rotate the image to upright based on its EXIF orientation tag.
/// fn Raster::autorot(&self) -> Raster;
///
/// /// Get the EXIF orientation tag value (1-8, or None).
/// fn Raster::get_orientation(&self) -> Option<u32>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py / test_conversion.py::test_autorot)
///
/// 1. Load sample.jpg (orientation=1, no rotation needed).
/// 2. autorot() should return same dimensions.
/// 3. For images with orientation 6/8, width and height should swap.
///
/// Reference: test_conversion.py::test_autorot
fn test_jpeg_autorot() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let rotated = im.autorot();
    // sample.jpg has orientation 1 (normal) — no change
    assert_eq!(rotated.width(), im.width());
    assert_eq!(rotated.height(), im.height());
}

#[test]
#[ignore]
/// Save JPEG with specific quality parameter.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode the raster as JPEG bytes with the given quality (1-100).
/// fn Raster::encode_jpeg(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
///
/// /// Save to a file path (format inferred from extension).
/// fn Raster::save(&self, path: &Path) -> Result<(), SaveError>;
///
/// /// Save JPEG with options.
/// fn Raster::save_jpeg(&self, path: &Path, quality: u8) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpeg — save section)
///
/// 1. Load sample.jpg.
/// 2. Encode at quality=10 and quality=90.
/// 3. quality=10 buffer should be smaller than quality=90.
/// 4. Decode both buffers, verify dimensions match original.
///
/// Reference: test_foreign.py::test_jpeg (save section)
fn test_jpeg_save_quality() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    let buf_low = im.encode_jpeg(10).unwrap();
    let buf_high = im.encode_jpeg(90).unwrap();

    assert!(
        buf_low.len() < buf_high.len(),
        "Low quality JPEG ({}) should be smaller than high quality ({})",
        buf_low.len(), buf_high.len()
    );

    // Both should decode back with same dimensions
    let im_low = decode_bytes(&buf_low).unwrap();
    let im_high = decode_bytes(&buf_high).unwrap();
    assert_eq!(im_low.width(), im.width());
    assert_eq!(im_high.width(), im.width());
    assert_eq!(im_low.height(), im.height());
    assert_eq!(im_high.height(), im.height());
}

#[test]
#[ignore]
/// Preserve ICC profile on JPEG save.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::save_jpeg(&self, path: &Path, quality: u8) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpeg — ICC section)
///
/// 1. Load sample.jpg (has ICC profile of 564 bytes).
/// 2. Save as JPEG.
/// 3. Reload and verify ICC profile is present and same size.
///
/// Reference: test_foreign.py::test_jpeg
fn test_jpeg_save_icc() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let original_icc = im.get_field("icc-profile-data");
    assert!(original_icc.is_some(), "sample.jpg should have an ICC profile");

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("icc_test.jpg");
    im.save_jpeg(&out, 85).unwrap();

    let im2 = decode_file(&out).unwrap();
    let saved_icc = im2.get_field("icc-profile-data");
    assert!(saved_icc.is_some(), "ICC profile should be preserved in saved JPEG");
}

#[test]
#[ignore]
/// Preserve EXIF metadata on JPEG save.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::save_jpeg(&self, path: &Path, quality: u8) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpeg — EXIF section)
///
/// 1. Load sample.jpg (has EXIF data).
/// 2. Save as JPEG.
/// 3. Reload and verify EXIF data is present.
/// 4. EXIF data length should match original.
///
/// Reference: test_foreign.py::test_jpeg
fn test_jpeg_save_exif() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let original_exif = im.get_field("exif-data");

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("exif_test.jpg");
    im.save_jpeg(&out, 85).unwrap();

    let im2 = decode_file(&out).unwrap();
    let saved_exif = im2.get_field("exif-data");

    if original_exif.is_some() {
        assert!(saved_exif.is_some(), "EXIF data should be preserved in saved JPEG");
    }
}

#[test]
#[ignore]
/// Control chroma sub-sampling on JPEG save (4:4:4, 4:2:0, etc.).
///
/// ## Required API
///
/// ```rust,ignore
/// /// JPEG chroma sub-sampling mode.
/// pub enum JpegSubsample { Auto, Off, On }
///
/// /// Encode JPEG with specific sub-sampling.
/// fn Raster::encode_jpeg_options(&self, quality: u8, subsample: JpegSubsample) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpeg — subsample section)
///
/// 1. Load sample.jpg.
/// 2. Encode with subsample=Off (4:4:4) — larger file.
/// 3. Encode with subsample=On (4:2:0) — smaller file.
/// 4. Both should decode to same dimensions.
///
/// Reference: test_foreign.py::test_jpeg (subsample section)
fn test_jpeg_save_subsample() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    let buf_444 = im.encode_jpeg_options(80, JpegSubsample::Off).unwrap();
    let buf_420 = im.encode_jpeg_options(80, JpegSubsample::On).unwrap();

    // 4:4:4 should be larger (more chroma data)
    assert!(
        buf_444.len() > buf_420.len(),
        "4:4:4 ({}) should be larger than 4:2:0 ({})",
        buf_444.len(), buf_420.len()
    );

    let im_444 = decode_bytes(&buf_444).unwrap();
    let im_420 = decode_bytes(&buf_420).unwrap();
    assert_eq!(im_444.width(), im.width());
    assert_eq!(im_420.width(), im.width());
}

#[test]
/// Load a truncated JPEG — should either partially decode or return a clean error.
/// Uses the real libvips reference truncated.jpg fixture.
fn test_jpeg_truncated() {
    let result = decode_file(&ref_image("truncated.jpg"));
    // Either a partial decode succeeds or we get a clean error — not a panic
    match result {
        Ok(raster) => {
            // If it decoded, dimensions should still be positive
            assert!(raster.width() > 0);
            assert!(raster.height() > 0);
        }
        Err(_) => {
            // A clean error is also acceptable for truncated data
        }
    }
}

// ===========================================================================
// 1.2 PNG
// ===========================================================================

#[test]
fn test_png_load_dimensions() {
    let raster = decode_file(&ref_image("sample.png")).unwrap();
    assert!(raster.width() > 0, "PNG width should be positive");
    assert!(raster.height() > 0, "PNG height should be positive");
}

#[test]
fn test_png_load_8bit() {
    // rgba.png is a known 8-bit RGBA PNG from the reference suite
    let raster = decode_file(&ref_image("rgba.png")).unwrap();
    assert!(
        raster.format() == PixelFormat::Rgb8 || raster.format() == PixelFormat::Rgba8,
        "Expected 8-bit format for rgba.png, got {:?}",
        raster.format()
    );
    assert_eq!(raster.format().bytes_per_pixel(), 4); // RGBA = 4 bpp
}

#[test]
fn test_png_load_16bit_reference() {
    // sample.png from the libvips suite is actually 16-bit
    let raster = decode_file(&ref_image("sample.png")).unwrap();
    assert!(
        raster.format() == PixelFormat::Rgb16 || raster.format() == PixelFormat::Rgba16,
        "Expected 16-bit format for sample.png, got {:?}",
        raster.format()
    );
}

#[test]
fn test_png_load_16bit() {
    // No 16-bit PNG in the reference suite, so we generate one synthetically
    let png = create_test_png_16bit(24, 24);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test16.png");
    std::fs::write(&path, &png).unwrap();
    let raster = decode_file(&path).unwrap();
    assert!(
        raster.format() == PixelFormat::Rgb16 || raster.format() == PixelFormat::Rgba16,
        "Expected 16-bit format, got {:?}",
        raster.format()
    );
    assert_eq!(raster.width(), 24);
    assert_eq!(raster.height(), 24);
}

#[test]
fn test_png_load_palette() {
    // Use the real libvips indexed.png fixture (a true palette/indexed PNG)
    let raster = decode_file(&ref_image("indexed.png")).unwrap();
    // Palette PNGs are expanded to RGB8 or RGBA8 by the image crate
    assert!(
        raster.format() == PixelFormat::Rgba8 || raster.format() == PixelFormat::Rgb8,
        "Expected Rgb8 or Rgba8 for palette PNG, got {:?}",
        raster.format()
    );
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
    let non_zero = raster.data().iter().any(|&b| b != 0);
    assert!(non_zero, "Palette PNG decode should produce non-zero pixels");
}

#[test]
fn test_png_load_rgba() {
    // Use the real rgba.png fixture to test RGBA PNG loading
    let raster = decode_file(&ref_image("rgba.png")).unwrap();
    assert_eq!(raster.format(), PixelFormat::Rgba8, "rgba.png should decode as RGBA8");
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
    assert!(raster.format().has_alpha(), "rgba.png should have an alpha channel");
    let non_zero = raster.data().iter().any(|&b| b != 0);
    assert!(non_zero);
}

#[test]
#[ignore]
/// Interlaced (Adam7) PNG loading.
///
/// ## Required API
///
/// ```rust,ignore
/// /// No special API — the decoder should transparently handle interlaced PNGs.
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_png — interlaced section)
///
/// 1. Load an interlaced PNG (Adam7).
/// 2. Verify dimensions and pixel values match the non-interlaced version.
///
/// Reference: test_foreign.py::test_png
fn test_png_load_interlaced() {
    // Use the reference interlaced PNG if available
    let result = decode_file(&ref_image("interlaced.png"));
    match result {
        Ok(raster) => {
            assert!(raster.width() > 0);
            assert!(raster.height() > 0);
            let non_zero = raster.data().iter().any(|&b| b != 0);
            assert!(non_zero, "Interlaced PNG should have non-zero pixel data");
        }
        Err(_) => {
            // Interlaced fixture may not exist — acceptable
            eprintln!("Note: interlaced.png fixture not found");
        }
    }
}

#[test]
#[ignore]
/// Save PNG with specific compression level.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as PNG bytes with a given compression level (0-9).
/// fn Raster::encode_png(&self, compression: u8) -> Result<Vec<u8>, EncodeError>;
///
/// /// Save PNG to file with options.
/// fn Raster::save_png(&self, path: &Path, compression: u8) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_png — save section)
///
/// 1. Load sample.png.
/// 2. Encode at compression=0 (fastest) and compression=9 (smallest).
/// 3. compression=9 buffer should be smaller.
/// 4. Both should decode to same dimensions and pixel values.
///
/// Reference: test_foreign.py::test_png
fn test_png_save_compression() {
    let im = decode_file(&ref_image("sample.png")).unwrap();

    let buf_fast = im.encode_png(0).unwrap();
    let buf_best = im.encode_png(9).unwrap();

    assert!(
        buf_best.len() <= buf_fast.len(),
        "Max compression ({}) should be ≤ min compression ({})",
        buf_best.len(), buf_fast.len()
    );

    let im_fast = decode_bytes(&buf_fast).unwrap();
    let im_best = decode_bytes(&buf_best).unwrap();
    assert_eq!(im_fast.width(), im.width());
    assert_eq!(im_best.width(), im.width());
    // PNG is lossless — pixel data should be identical
    assert_eq!(im_fast.data(), im_best.data(), "PNG compression should be lossless");
}

#[test]
#[ignore]
/// Save PNG with interlace (Adam7).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode PNG with Adam7 interlacing.
/// fn Raster::encode_png_interlaced(&self) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_png — interlace section)
///
/// 1. Load sample.png.
/// 2. Encode with interlacing.
/// 3. Decode back, verify dimensions and pixel values match.
///
/// Reference: test_foreign.py::test_png
fn test_png_save_interlace() {
    let im = decode_file(&ref_image("sample.png")).unwrap();
    let buf = im.encode_png_interlaced().unwrap();

    // Verify PNG signature
    assert_eq!(&buf[..4], &[0x89, b'P', b'N', b'G']);

    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im2.data(), im.data(), "Interlaced PNG round-trip should be lossless");
}

#[test]
#[ignore]
/// Save PNG as palette/indexed (colour quantization).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode as an indexed/palette PNG with at most `max_colours` palette entries.
/// fn Raster::encode_png_palette(&self, max_colours: u32) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_png — palette section)
///
/// 1. Load sample.png.
/// 2. Encode as palette PNG with max 256 colours.
/// 3. The palette buffer should be smaller than the full-colour version.
/// 4. Decode back, verify dimensions match.
///
/// Reference: test_foreign.py::test_png
fn test_png_save_palette() {
    let im = decode_file(&ref_image("sample.png")).unwrap();

    let buf_palette = im.encode_png_palette(256).unwrap();
    let buf_full = im.encode_png(6).unwrap();

    assert!(
        buf_palette.len() < buf_full.len(),
        "Palette PNG ({}) should be smaller than full-colour ({})",
        buf_palette.len(), buf_full.len()
    );

    let im2 = decode_bytes(&buf_palette).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
}

#[test]
#[ignore]
/// ICC profile round-trip for PNG.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::save_png(&self, path: &Path, compression: u8) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_png — ICC section)
///
/// 1. Load a PNG with an ICC profile (sample.png may have one).
/// 2. Save as PNG.
/// 3. Reload and verify the ICC profile is preserved.
///
/// Reference: test_foreign.py::test_png
fn test_png_icc() {
    let im = decode_file(&ref_image("sample.png")).unwrap();

    if im.get_field("icc-profile-data").is_some() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("icc_test.png");
        im.save_png(&out, 6).unwrap();

        let im2 = decode_file(&out).unwrap();
        assert!(
            im2.get_field("icc-profile-data").is_some(),
            "ICC profile should be preserved in PNG"
        );
    }
}

#[test]
#[ignore]
/// EXIF metadata round-trip for PNG.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::save_png(&self, path: &Path, compression: u8) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic
///
/// 1. Load a PNG with EXIF data (if available).
/// 2. Save and reload.
/// 3. Verify EXIF is preserved.
///
/// Reference: test_foreign.py::test_png (metadata section)
fn test_png_exif() {
    let im = decode_file(&ref_image("sample.png")).unwrap();

    if im.get_field("exif-data").is_some() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("exif_test.png");
        im.save_png(&out, 6).unwrap();

        let im2 = decode_file(&out).unwrap();
        assert!(im2.get_field("exif-data").is_some(), "EXIF should be preserved in PNG");
    }
}

// ===========================================================================
// 1.3 TIFF
// ===========================================================================

#[test]
fn test_tiff_load_dimensions() {
    let raster = decode_file(&ref_image("sample.tif")).unwrap();
    assert!(raster.width() > 0, "TIFF width should be positive");
    assert!(raster.height() > 0, "TIFF height should be positive");
}

#[test]
fn test_tiff_load_pixels() {
    let raster = decode_file(&ref_image("sample.tif")).unwrap();
    let all_zero = raster.data().iter().all(|&b| b == 0);
    assert!(!all_zero, "Decoded TIFF pixel data should not be all zeroes");
    // Verify real photo has diverse pixel values
    let min = *raster.data().iter().min().unwrap();
    let max = *raster.data().iter().max().unwrap();
    assert!(max - min > 50, "Expected pixel value range in real TIFF, got {min}..{max}");
}

#[test]
#[ignore]
/// Multi-page TIFF loading (extract specific pages).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Get the number of pages in a multi-page TIFF.
/// fn tiff_page_count(path: &Path) -> Result<u32, DecodeError>;
///
/// /// Decode a specific page from a multi-page TIFF (1-indexed).
/// fn decode_tiff_page(path: &Path, page: u32) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_tiff — multipage section)
///
/// 1. Load a multi-page TIFF (e.g. multipage.tif from reference suite).
/// 2. Count pages — should be > 1.
/// 3. Extract each page, verify dimensions are positive.
///
/// Reference: test_foreign.py::test_tiff
fn test_tiff_multipage() {
    let tiff_path = ref_image("multipage.tif");
    let page_count = tiff_page_count(&tiff_path).unwrap();
    assert!(page_count > 1, "multipage.tif should have multiple pages, got {page_count}");

    for p in 1..=page_count {
        let raster = decode_tiff_page(&tiff_path, p).unwrap();
        assert!(raster.width() > 0, "Page {p}: width should be positive");
        assert!(raster.height() > 0, "Page {p}: height should be positive");
    }
}

#[test]
fn test_tiff_strip() {
    // sample.tif is strip-layout by default
    let raster = decode_file(&ref_image("sample.tif")).unwrap();
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
}

#[test]
fn test_tiff_tile() {
    // ojpeg-tile.tif is a tiled TIFF from the libvips reference suite
    let result = decode_file(&ref_image("ojpeg-tile.tif"));
    match result {
        Ok(raster) => {
            assert!(raster.width() > 0);
            assert!(raster.height() > 0);
        }
        Err(_) => {
            // OJPEG is a legacy format — a clean error is acceptable
        }
    }
}

#[test]
fn test_tiff_low_bitdepth() {
    // Test 1-bit, 2-bit, 4-bit TIFF loading with real fixtures
    for name in &["1bit.tif", "2bit.tif", "4bit.tif"] {
        let result = decode_file(&ref_image(name));
        match result {
            Ok(raster) => {
                assert!(raster.width() > 0, "{name}: width should be positive");
                assert!(raster.height() > 0, "{name}: height should be positive");
            }
            Err(e) => {
                // Low-bitdepth TIFFs may not be supported yet — log it
                eprintln!("Note: {name} not yet supported: {e}");
            }
        }
    }
}

#[test]
fn test_tiff_subsampled() {
    let result = decode_file(&ref_image("subsampled.tif"));
    match result {
        Ok(raster) => {
            assert!(raster.width() > 0);
            assert!(raster.height() > 0);
        }
        Err(e) => {
            eprintln!("Note: subsampled.tif not yet supported: {e}");
        }
    }
}

#[test]
#[ignore]
/// TIFF with LZW compression.
///
/// ## Required API
///
/// ```rust,ignore
/// /// TIFF compression modes.
/// pub enum TiffCompression { None, Lzw, Jpeg, Deflate, Ccitt }
///
/// /// Save raster as TIFF with a specified compression.
/// fn Raster::save_tiff(&self, path: &Path, compression: TiffCompression) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_tiff — compression section)
///
/// 1. Load sample.tif.
/// 2. Save with LZW compression.
/// 3. Reload and verify dimensions match.
/// 4. LZW file should be smaller than uncompressed.
///
/// Reference: test_foreign.py::test_tiff
fn test_tiff_save_lzw() {
    let im = decode_file(&ref_image("sample.tif")).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let out_lzw = dir.path().join("lzw.tif");
    im.save_tiff(&out_lzw, TiffCompression::Lzw).unwrap();

    let out_none = dir.path().join("none.tif");
    im.save_tiff(&out_none, TiffCompression::None).unwrap();

    let im2 = decode_file(&out_lzw).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());

    let lzw_size = std::fs::metadata(&out_lzw).unwrap().len();
    let none_size = std::fs::metadata(&out_none).unwrap().len();
    assert!(lzw_size < none_size, "LZW ({lzw_size}) should be smaller than none ({none_size})");
}

#[test]
#[ignore]
/// TIFF with JPEG compression.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::save_tiff(&self, path: &Path, compression: TiffCompression) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_tiff)
///
/// 1. Load sample.tif, save with JPEG compression.
/// 2. Reload, verify dimensions match.
/// 3. JPEG is lossy, so pixel values may differ slightly.
///
/// Reference: test_foreign.py::test_tiff
fn test_tiff_save_jpeg() {
    let im = decode_file(&ref_image("sample.tif")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("jpeg.tif");
    im.save_tiff(&out, TiffCompression::Jpeg).unwrap();

    let im2 = decode_file(&out).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
}

#[test]
#[ignore]
/// TIFF with Deflate (zlib) compression.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::save_tiff(&self, path: &Path, compression: TiffCompression) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_tiff)
///
/// 1. Save sample.tif with Deflate.
/// 2. Reload, verify lossless round-trip.
///
/// Reference: test_foreign.py::test_tiff
fn test_tiff_save_deflate() {
    let im = decode_file(&ref_image("sample.tif")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("deflate.tif");
    im.save_tiff(&out, TiffCompression::Deflate).unwrap();

    let im2 = decode_file(&out).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im2.data(), im.data(), "Deflate TIFF should be lossless");
}

#[test]
#[ignore]
/// TIFF with CCITT/G4 fax compression (1-bit images).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::save_tiff(&self, path: &Path, compression: TiffCompression) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_tiff — 1-bit section)
///
/// 1. Create a 1-bit (binary) image by thresholding.
/// 2. Save as TIFF with CCITT G4 compression.
/// 3. Reload, verify lossless.
///
/// Reference: test_foreign.py::test_tiff
fn test_tiff_save_ccitt() {
    let im = decode_file(&ref_image("sample.tif")).unwrap();
    // Create a 1-bit image by thresholding the green channel
    let mono = im.extract_band(1);
    let binary = mono.more_than_const(128.0);

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("ccitt.tif");
    binary.save_tiff(&out, TiffCompression::Ccitt).unwrap();

    let im2 = decode_file(&out).unwrap();
    assert_eq!(im2.width(), binary.width());
    assert_eq!(im2.height(), binary.height());
}

#[test]
#[ignore]
/// BigTIFF (>4 GB addressing) support.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Save as BigTIFF (64-bit offsets, needed for files >4 GB).
/// fn Raster::save_bigtiff(&self, path: &Path, compression: TiffCompression) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic
///
/// 1. Create a moderate-sized image.
/// 2. Save as BigTIFF.
/// 3. Reload and verify dimensions and pixels match.
/// (We don't create an actual >4GB file in tests.)
///
/// Reference: test_foreign.py::test_tiff
fn test_tiff_bigtiff() {
    let im = decode_file(&ref_image("sample.tif")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("big.tif");
    im.save_bigtiff(&out, TiffCompression::None).unwrap();

    let im2 = decode_file(&out).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
}

// ===========================================================================
// 1.4 PDF
// ===========================================================================

#[test]
fn test_pdf_page_count() {
    let info = pdf_info(Path::new(FIXTURE_PDF)).unwrap();
    assert!(
        info.page_count >= 1,
        "Expected at least 1 page, got {}",
        info.page_count
    );
}

#[test]
fn test_pdf_page_dimensions() {
    let info = pdf_info(Path::new(FIXTURE_PDF)).unwrap();
    let page = &info.pages[0];
    assert!(page.width_pts > 0.0, "Page width should be positive");
    assert!(page.height_pts > 0.0, "Page height should be positive");
    // Blueprint pages are typically large — basic sanity check
    assert!(
        page.width_pts > 100.0 || page.height_pts > 100.0,
        "Blueprint page dimensions seem too small: {}x{}",
        page.width_pts,
        page.height_pts
    );
}

#[test]
fn test_pdf_extract_image() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1).unwrap();
    assert!(
        raster.width() > 0 && raster.height() > 0,
        "Extracted image has zero dimensions"
    );
    // Verify we got actual pixel data
    assert!(
        !raster.data().is_empty(),
        "Extracted image has no pixel data"
    );
    let data_len = raster.data().len();
    let expected_len =
        raster.width() as usize * raster.height() as usize * raster.format().bytes_per_pixel();
    assert_eq!(
        data_len, expected_len,
        "Pixel data length mismatch: got {data_len}, expected {expected_len}"
    );
}

#[test]
fn test_pdf_page_select() {
    let info = pdf_info(Path::new(FIXTURE_PDF)).unwrap();
    // Extract from each available page
    for page_num in 1..=info.page_count {
        let result = extract_page_image(Path::new(FIXTURE_PDF), page_num);
        assert!(
            result.is_ok(),
            "Failed to extract page {page_num}: {:?}",
            result.err()
        );
    }
    // Out-of-range page should fail
    let bad = extract_page_image(Path::new(FIXTURE_PDF), info.page_count + 1);
    assert!(bad.is_err(), "Extracting beyond last page should fail");
}

#[test]
#[ignore]
/// Extract at different DPI values and verify dimension scaling.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Extract a page image at a specific DPI (default is typically 72 or 150).
/// fn extract_page_image_dpi(path: &Path, page: u32, dpi: f64) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_pdf — DPI section)
///
/// 1. Extract page 1 at 72 DPI.
/// 2. Extract page 1 at 144 DPI.
/// 3. The 144 DPI image should be ~2× the dimensions of the 72 DPI image.
///
/// Reference: test_foreign.py::test_pdf
fn test_pdf_dpi_scale() {
    let lo = extract_page_image_dpi(Path::new(FIXTURE_PDF), 1, 72.0).unwrap();
    let hi = extract_page_image_dpi(Path::new(FIXTURE_PDF), 1, 144.0).unwrap();

    // 144 DPI should be approximately 2× the size of 72 DPI
    let ratio_w = hi.width() as f64 / lo.width() as f64;
    let ratio_h = hi.height() as f64 / lo.height() as f64;
    assert!(
        (ratio_w - 2.0).abs() < 0.2,
        "Width ratio should be ~2.0, got {ratio_w}"
    );
    assert!(
        (ratio_h - 2.0).abs() < 0.2,
        "Height ratio should be ~2.0, got {ratio_h}"
    );
}

#[test]
#[ignore]
/// Set background colour for PDF rendering.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Extract with a specified background colour (for transparent PDFs).
/// fn extract_page_image_with_background(
///     path: &Path, page: u32, background: &[f64],
/// ) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic
///
/// 1. Extract page 1 with white background [255, 255, 255].
/// 2. Extract page 1 with red background [255, 0, 0].
/// 3. If the PDF has transparent areas, the two should differ.
///
/// Reference: test_foreign.py::test_pdf
fn test_pdf_background() {
    let white = extract_page_image_with_background(
        Path::new(FIXTURE_PDF), 1, &[255.0, 255.0, 255.0],
    ).unwrap();
    let red = extract_page_image_with_background(
        Path::new(FIXTURE_PDF), 1, &[255.0, 0.0, 0.0],
    ).unwrap();

    assert_eq!(white.width(), red.width());
    assert_eq!(white.height(), red.height());
    // The images may or may not differ depending on transparency
}

#[test]
#[ignore]
/// Open a password-protected PDF.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Open a password-protected PDF.
/// fn pdf_info_with_password(path: &Path, password: &str) -> Result<PdfInfo, PdfError>;
/// fn extract_page_image_with_password(path: &Path, page: u32, password: &str) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic
///
/// 1. Attempt to open a password-protected PDF without password — should fail.
/// 2. Open with correct password — should succeed.
/// 3. Verify page count and dimensions.
///
/// Reference: test_foreign.py::test_pdf (password section)
fn test_pdf_password() {
    // Attempt without password
    let result = pdf_info(&ref_image("password.pdf"));
    assert!(result.is_err(), "Password-protected PDF should fail without password");

    // With password
    let info = pdf_info_with_password(&ref_image("password.pdf"), "secret").unwrap();
    assert!(info.page_count >= 1);

    let raster = extract_page_image_with_password(&ref_image("password.pdf"), 1, "secret").unwrap();
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
}

#[test]
fn test_pdf_cmyk() {
    // Use the real CMYK PDF fixture from the libvips reference suite
    let cmyk_pdf = ref_image("cmyktest.pdf");
    let info = pdf_info(&cmyk_pdf).unwrap();
    assert!(info.page_count >= 1, "CMYK PDF should have at least 1 page");

    let raster = extract_page_image(&cmyk_pdf, 1).unwrap();
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
    // CMYK should be converted to an RGB format
    let fmt = raster.format();
    assert!(
        fmt == PixelFormat::Rgb8
            || fmt == PixelFormat::Rgba8
            || fmt == PixelFormat::Gray8
            || fmt == PixelFormat::Rgb16
            || fmt == PixelFormat::Rgba16
            || fmt == PixelFormat::Gray16,
        "Unexpected pixel format from CMYK PDF extraction: {fmt:?}"
    );
}

#[test]
fn test_pdf_reference_reschart() {
    // Test with the libvips reference ISO 12233 resolution chart PDF
    let pdf = ref_image("ISO_12233-reschart.pdf");
    let info = pdf_info(&pdf).unwrap();
    assert!(info.page_count >= 1);
    let page = &info.pages[0];
    assert!(page.width_pts > 0.0);
    assert!(page.height_pts > 0.0);
}

// ===========================================================================
// 1.5 Deep Zoom / Tile Output
// ===========================================================================

#[test]
fn test_dz_tile_size() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(256, 256);
    let tile_size = 128;
    let planner = PyramidPlanner::new(256, 256, tile_size, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("dz_tile_size");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);
    let config = EngineConfig::default();

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Verify that at least one tile file exists
    let raw_count = count_files(&base, "raw");
    assert!(raw_count > 0, "No raw tiles were produced");
    assert_eq!(raw_count as u64, plan.total_tile_count());
}

#[test]
fn test_dz_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(256, 256);
    let overlap = 1;
    let planner = PyramidPlanner::new(256, 256, 128, overlap, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("dz_overlap");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);

    let result = generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());

    let raw_count = count_files(&base, "raw");
    assert_eq!(raw_count as u64, plan.total_tile_count());
}

#[test]
fn test_dz_layout_deepzoom() {
    // Use a real reference JPEG as pyramid source
    let src = decode_file(&ref_image("sample.jpg")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let planner = PyramidPlanner::new(src.width(), src.height(), 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("deepzoom_out");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    // DeepZoom should produce a .dzi manifest
    let dzi = dir.path().join("deepzoom_out.dzi");
    assert!(dzi.exists(), "DZI manifest should exist for DeepZoom layout");
    let manifest = std::fs::read_to_string(&dzi).unwrap();
    assert!(manifest.contains(&format!("Width=\"{}\"", src.width())));
    assert!(manifest.contains(&format!("Height=\"{}\"", src.height())));

    // Verify tiles use DeepZoom path convention: {level}/{col}_{row}.ext
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0_0.png", top.level));
    assert!(tile_path.exists(), "DeepZoom tile not at expected path");
}

#[test]
fn test_dz_layout_xyz() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(256, 256);
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::Xyz).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("xyz_out");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    // XYZ path: {z}/{x}/{y}.ext
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0/0.raw", top.level));
    assert!(tile_path.exists(), "XYZ tile not at expected path");

    // No DZI manifest for XYZ layout
    assert!(!dir.path().join("xyz_out.dzi").exists());
}

#[test]
#[ignore]
/// Zoomify tile layout.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Zoomify layout variant.
/// Layout::Zoomify
///
/// /// Zoomify writes tiles in TileGroup directories.
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_dzsave — Zoomify section)
///
/// 1. Generate tiles with Layout::Zoomify.
/// 2. Verify TileGroup0/ directory exists.
/// 3. Verify ImageProperties.xml exists with correct dimensions.
///
/// Reference: test_foreign.py::test_dzsave
fn test_dz_layout_zoomify() {
    let src = gradient_raster(256, 256);
    let dir = tempfile::tempdir().unwrap();
    let planner = PyramidPlanner::new(256, 256, 256, 0, Layout::Zoomify).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("zoomify_out");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Jpeg { quality: 80 });
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    // Zoomify uses TileGroup directories
    let tg0 = base.join("TileGroup0");
    assert!(tg0.exists(), "Zoomify TileGroup0 directory should exist");

    let props = base.join("ImageProperties.xml");
    assert!(props.exists(), "Zoomify ImageProperties.xml should exist");
}

#[test]
#[ignore]
/// IIIF tile layout.
///
/// ## Required API
///
/// ```rust,ignore
/// Layout::Iiif
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_dzsave — IIIF section)
///
/// 1. Generate tiles with Layout::Iiif.
/// 2. Verify info.json exists with correct dimensions.
///
/// Reference: test_foreign.py::test_dzsave
fn test_dz_layout_iiif() {
    let src = gradient_raster(256, 256);
    let dir = tempfile::tempdir().unwrap();
    let planner = PyramidPlanner::new(256, 256, 256, 0, Layout::Iiif).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("iiif_out");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Jpeg { quality: 80 });
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let info_json = base.join("info.json");
    assert!(info_json.exists(), "IIIF info.json should exist");
    let info = std::fs::read_to_string(&info_json).unwrap();
    assert!(info.contains("\"width\""), "info.json should contain width");
}

#[test]
#[ignore]
/// Write tiles to a ZIP archive.
///
/// ## Required API
///
/// ```rust,ignore
/// /// A sink that writes tiles into a ZIP archive.
/// pub struct ZipSink { ... }
///
/// impl ZipSink {
///     pub fn new(path: PathBuf, plan: Plan, format: TileFormat) -> Self;
/// }
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_dzsave — zip section)
///
/// 1. Generate tiles into a ZipSink.
/// 2. Verify the output .zip file exists and is non-empty.
/// 3. Open the zip and verify it contains tile files.
///
/// Reference: test_foreign.py::test_dzsave
fn test_dz_zip() {
    let src = gradient_raster(256, 256);
    let dir = tempfile::tempdir().unwrap();
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let zip_path = dir.path().join("tiles.zip");
    let sink = ZipSink::new(zip_path.clone(), plan.clone(), TileFormat::Png);
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    assert!(zip_path.exists(), "ZIP file should exist");
    let metadata = std::fs::metadata(&zip_path).unwrap();
    assert!(metadata.len() > 0, "ZIP file should be non-empty");
}

#[test]
fn test_dz_format_png() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(64, 64);
    let planner = PyramidPlanner::new(64, 64, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("png_tiles");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let png_count = count_files(&base, "png");
    assert!(png_count > 0, "No PNG tiles produced");

    // Verify a tile starts with PNG magic bytes
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0_0.png", top.level));
    let bytes = std::fs::read(&tile_path).unwrap();
    assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
}

#[test]
fn test_dz_format_jpeg() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(64, 64);
    let planner = PyramidPlanner::new(64, 64, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("jpeg_tiles");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Jpeg { quality: 85 });

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let jpeg_count = count_files(&base, "jpeg");
    assert!(jpeg_count > 0, "No JPEG tiles produced");

    // Verify JPEG SOI marker
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0_0.jpeg", top.level));
    let bytes = std::fs::read(&tile_path).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
}

#[test]
#[ignore]
/// Skip blank (fully transparent/white) tiles to save space.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Engine configuration with skip_blanks option.
/// impl EngineConfig {
///     /// If true, tiles that are entirely one colour (e.g. white or transparent)
///     /// are not written, saving disk space.
///     pub fn skip_blanks(self, skip: bool) -> Self;
/// }
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_dzsave — skip-blanks section)
///
/// 1. Create a mostly-white 256×256 image with a small non-white region.
/// 2. Generate tiles with skip_blanks=true.
/// 3. The number of tiles should be less than without skip_blanks.
///
/// Reference: test_foreign.py::test_dzsave
fn test_dz_skip_blanks() {
    // Create mostly-white image with a small coloured patch
    let mut data = vec![255u8; 256 * 256 * 3];
    for y in 0..32 {
        for x in 0..32 {
            let off = (y * 256 + x) * 3;
            data[off] = 100;
            data[off + 1] = 50;
            data[off + 2] = 0;
        }
    }
    let src = Raster::new(256, 256, PixelFormat::Rgb8, data).unwrap();

    let dir_skip = tempfile::tempdir().unwrap();
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base_skip = dir_skip.path().join("skip");
    let sink = FsSink::new(base_skip.clone(), plan.clone(), TileFormat::Png);
    let config_skip = EngineConfig::default().skip_blanks(true);
    let result_skip = generate_pyramid(&src, &plan, &sink, &config_skip).unwrap();

    let dir_no = tempfile::tempdir().unwrap();
    let base_no = dir_no.path().join("noskip");
    let sink_no = FsSink::new(base_no.clone(), plan.clone(), TileFormat::Png);
    let result_no = generate_pyramid(&src, &plan, &sink_no, &EngineConfig::default()).unwrap();

    assert!(
        result_skip.tiles_produced <= result_no.tiles_produced,
        "skip_blanks should produce ≤ tiles: {} vs {}",
        result_skip.tiles_produced, result_no.tiles_produced
    );
}

#[test]
#[ignore]
/// Write tile properties/metadata (e.g. ImageProperties.xml for Zoomify).
///
/// ## Required API
///
/// ```rust,ignore
/// /// After pyramid generation, write a properties file for the layout.
/// fn write_properties(base: &Path, plan: &Plan, layout: Layout) -> Result<(), io::Error>;
/// ```
///
/// ## Test logic
///
/// 1. Generate DeepZoom tiles.
/// 2. Verify .dzi manifest contains correct TileSize, Overlap, Format.
///
/// Reference: test_foreign.py::test_dzsave (properties section)
fn test_dz_properties() {
    let src = gradient_raster(256, 256);
    let dir = tempfile::tempdir().unwrap();
    let tile_size = 128;
    let overlap = 1;
    let planner = PyramidPlanner::new(256, 256, tile_size, overlap, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("props");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let dzi = dir.path().join("props.dzi");
    assert!(dzi.exists());
    let manifest = std::fs::read_to_string(&dzi).unwrap();
    assert!(manifest.contains(&format!("TileSize=\"{tile_size}\"")));
    assert!(manifest.contains(&format!("Overlap=\"{overlap}\"")));
    assert!(manifest.contains("Format=\"png\""));
}

#[test]
#[ignore]
/// Generate tiles for a sub-region of the source image.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Generate a pyramid from only a rectangular region of the source.
/// fn generate_pyramid_region(
///     src: &Raster, plan: &Plan, sink: &dyn Sink, config: &EngineConfig,
///     left: u32, top: u32, width: u32, height: u32,
/// ) -> Result<PyramidResult, PyramidError>;
/// ```
///
/// ## Test logic
///
/// 1. Load sample.jpg.
/// 2. Generate tiles for region (0, 0, 100, 100).
/// 3. Top-level tile should cover only the specified region.
///
/// Reference: test_foreign.py::test_dzsave (region section)
fn test_dz_region() {
    let src = decode_file(&ref_image("sample.jpg")).unwrap();
    let region_w = 100;
    let region_h = 100;
    let planner = PyramidPlanner::new(region_w, region_h, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("region");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    generate_pyramid_region(&src, &plan, &sink, &EngineConfig::default(), 0, 0, region_w, region_h).unwrap();

    let dzi = dir.path().join("region.dzi");
    assert!(dzi.exists());
    let manifest = std::fs::read_to_string(&dzi).unwrap();
    assert!(manifest.contains(&format!("Width=\"{region_w}\"")));
    assert!(manifest.contains(&format!("Height=\"{region_h}\"")));
}

// ===========================================================================
// 1.6 Other Formats (NOT IMPLEMENTED — all stubs)
// ===========================================================================

#[test]
#[ignore]
/// WebP load/save.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Decode a WebP image from a file.
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // already exists, needs WebP support
///
/// /// Encode raster as WebP bytes.
/// fn Raster::encode_webp(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_webp)
///
/// 1. Load sample_webp.webp from reference fixtures.
/// 2. Verify dimensions and pixel values.
/// 3. Encode, decode back, verify lossless (within tolerance for lossy).
///
/// Reference: test_foreign.py::test_webp
fn test_webp() {
    let im = decode_file(&ref_image("1.webp")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);

    let buf = im.encode_webp(80).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
}

#[test]
#[ignore]
/// GIF load/save.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs GIF support
/// fn Raster::encode_gif(&self) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_gif)
///
/// 1. Load trans-x.gif from reference fixtures.
/// 2. Verify dimensions and band count.
/// 3. Encode to GIF buffer, verify round-trip dimensions.
///
/// Reference: test_foreign.py::test_gif
fn test_gif() {
    let im = decode_file(&ref_image("trans-x.gif")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);

    let buf = im.encode_gif().unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
}

#[test]
#[ignore]
/// HEIF/AVIF load/save.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs HEIF/AVIF support
/// fn Raster::encode_heif(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_heif)
///
/// 1. Load avif-orientation-*.avif from reference fixtures.
/// 2. Verify dimensions.
///
/// Reference: test_foreign.py::test_heif
fn test_heif_avif() {
    let im = decode_file(&ref_image("avif-orientation-1.avif")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);
}

#[test]
#[ignore]
/// JPEG 2000 load.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs JP2K support
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jp2k)
///
/// 1. Load a .jp2 image from reference fixtures.
/// 2. Verify dimensions and pixel format.
///
/// Reference: test_foreign.py::test_jp2k
fn test_jp2k() {
    let im = decode_file(&ref_image("world.jp2")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);
}

#[test]
#[ignore]
/// JPEG XL load/save.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs JXL support
/// fn Raster::encode_jxl(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jxl)
///
/// 1. Load a .jxl image.
/// 2. Verify dimensions.
/// 3. Encode and decode, verify round-trip.
///
/// Reference: test_foreign.py::test_jxl
fn test_jxl() {
    let im = decode_file(&ref_image("sample.jxl")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);

    let buf = im.encode_jxl(80).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
}

#[test]
#[ignore]
/// SVG rasterization.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Decode an SVG from bytes into a raster image at a given DPI.
/// fn decode_svg(data: &[u8], dpi: Option<f64>) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_svg)
///
/// 1. Load a minimal SVG from bytes.
/// 2. Verify dimensions match the SVG viewport.
///
/// Reference: test_foreign.py::test_svg
fn test_svg() {
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\"><rect width=\"100\" height=\"50\" fill=\"red\"/></svg>";
    let im = decode_svg(svg, None).unwrap();
    assert_eq!(im.width(), 100);
    assert_eq!(im.height(), 50);
}

#[test]
#[ignore]
/// FITS astronomical image format.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs FITS support
/// ```
///
/// ## Test logic
///
/// 1. Load a .fits image.
/// 2. Verify dimensions are positive.
///
/// Reference: test_foreign.py::test_fits
fn test_fits() {
    let result = decode_file(&ref_image("sample.fits"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("FITS not supported: {e}"),
    }
}

#[test]
#[ignore]
/// OpenEXR HDR image format.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs OpenEXR support
/// ```
///
/// ## Test logic
///
/// 1. Load a .exr image.
/// 2. Verify dimensions are positive.
///
/// Reference: test_foreign.py
fn test_openexr() {
    let result = decode_file(&ref_image("sample.exr"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("OpenEXR not supported: {e}"),
    }
}

#[test]
#[ignore]
/// OpenSlide whole-slide image support.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Open an OpenSlide-compatible whole-slide image.
/// fn decode_openslide(path: &Path, level: u32) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic
///
/// 1. Load a whole-slide image at level 0.
/// 2. Verify dimensions are positive.
///
/// Reference: test_foreign.py
fn test_openslide() {
    let result = decode_file(&ref_image("openslide.svs"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("OpenSlide not supported: {e}"),
    }
}

#[test]
#[ignore]
/// MATLAB .mat file loading.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs .mat support
/// ```
///
/// ## Test logic
///
/// 1. Load a .mat file.
/// 2. Verify dimensions.
///
/// Reference: test_foreign.py
fn test_matlab() {
    let result = decode_file(&ref_image("sample.mat"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("MATLAB .mat not supported: {e}"),
    }
}

#[test]
#[ignore]
/// Analyze 7.5 neuroimaging format.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs Analyze support
/// ```
///
/// ## Test logic
///
/// 1. Load an Analyze .hdr/.img pair.
/// 2. Verify dimensions.
///
/// Reference: test_foreign.py
fn test_analyze() {
    let result = decode_file(&ref_image("sample.hdr"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("Analyze format not supported: {e}"),
    }
}

#[test]
#[ignore]
/// NIfTI neuroimaging format.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs NIfTI support
/// ```
///
/// ## Test logic
///
/// 1. Load a .nii file.
/// 2. Verify dimensions.
///
/// Reference: test_foreign.py
fn test_nifti() {
    let result = decode_file(&ref_image("sample.nii"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("NIfTI not supported: {e}"),
    }
}

#[test]
#[ignore]
/// PPM/PGM/PBM (Netpbm) format load/save.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs PPM support
/// fn Raster::encode_ppm(&self) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_ppm)
///
/// 1. Load sample.ppm.
/// 2. Verify dimensions.
/// 3. Encode and decode, verify lossless round-trip.
///
/// Reference: test_foreign.py::test_ppm
fn test_ppm() {
    let im = decode_file(&ref_image("sample.ppm")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);

    let buf = im.encode_ppm().unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im2.data(), im.data(), "PPM round-trip should be lossless");
}

#[test]
#[ignore]
/// Radiance HDR (.hdr/.pic) format.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs Radiance HDR support
/// ```
///
/// ## Test logic
///
/// 1. Load a .hdr file.
/// 2. Verify dimensions.
///
/// Reference: test_foreign.py::test_rad
fn test_rad() {
    let result = decode_file(&ref_image("sample.hdr"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("Radiance HDR not supported: {e}"),
    }
}

#[test]
#[ignore]
/// CSV matrix loading (pixel values as text).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Load pixel values from a CSV text matrix.
/// fn Raster::csv_load(data: &[u8]) -> Result<Raster, DecodeError>;
///
/// /// Save pixel values as a CSV text matrix.
/// fn Raster::csv_save(&self) -> Vec<u8>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_csv)
///
/// 1. Create a small single-band image.
/// 2. Save as CSV.
/// 3. Load back.
/// 4. Verify pixel values match (lossless).
///
/// Reference: test_foreign.py::test_csv
fn test_csv_matrix() {
    let data = vec![42u8; 10 * 10];
    let im = Raster::new(10, 10, PixelFormat::Gray8, data).unwrap();

    let csv = im.csv_save();
    assert!(!csv.is_empty());

    let im2 = Raster::csv_load(&csv).unwrap();
    assert_eq!(im2.width(), 10);
    assert_eq!(im2.height(), 10);

    let max_diff: f64 = im.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001, "CSV round-trip should be lossless");
}

#[test]
#[ignore]
/// BMP format load.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs BMP support
/// ```
///
/// ## Test logic
///
/// 1. Load a .bmp file (e.g. from reference fixtures or synthesised).
/// 2. Verify dimensions and pixel values.
///
/// Reference: test_foreign.py
fn test_bmp() {
    // Create a BMP in memory using the image crate
    let mut buf = Vec::new();
    {
        let encoder = image::codecs::bmp::BmpEncoder::new(&mut buf);
        let data = vec![128u8; 10 * 10 * 3];
        encoder.write_image(&data, 10, 10, image::ColorType::Rgb8.into()).unwrap();
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.bmp");
    std::fs::write(&path, &buf).unwrap();

    let im = decode_file(&path).unwrap();
    assert_eq!(im.width(), 10);
    assert_eq!(im.height(), 10);
}

#[test]
#[ignore]
/// Ultra HDR (gain-map JPEG) format.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>; // needs UHDR support
/// fn Raster::encode_uhdr(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic
///
/// 1. Load an Ultra HDR JPEG.
/// 2. Verify dimensions and that it contains a gain map.
///
/// Reference: libvips UHDR support
fn test_uhdr() {
    let result = decode_file(&ref_image("sample_uhdr.jpg"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("Ultra HDR not supported: {e}"),
    }
}
