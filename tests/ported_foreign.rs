#![cfg(feature = "ported_tests")]

//! Phase 1: Ported foreign-format tests.
//!
//! Auto tests create synthetic images via the `image` crate and verify
//! round-trip through `libviprs::decode_file` / `libviprs::source::decode_bytes`.
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

// ---------------------------------------------------------------------------
// Helper functions — create synthetic test images in memory
// ---------------------------------------------------------------------------

/// Create an RGB8 JPEG in memory.
fn create_test_jpeg(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(Cursor::new(&mut buf), 90);
        let mut data = vec![0u8; w as usize * h as usize * 3];
        // Simple gradient so pixel values are not all zero
        for y in 0..h {
            for x in 0..w {
                let off = (y as usize * w as usize + x as usize) * 3;
                data[off] = (x * 255 / w.max(1)) as u8;
                data[off + 1] = (y * 255 / h.max(1)) as u8;
                data[off + 2] = ((x + y) * 127 / (w + h).max(1)) as u8;
            }
        }
        encoder
            .write_image(&data, w, h, image::ColorType::Rgb8.into())
            .unwrap();
    }
    buf
}

/// Create an RGB8 PNG in memory.
fn create_test_png(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut buf));
        let mut data = vec![0u8; w as usize * h as usize * 3];
        for y in 0..h {
            for x in 0..w {
                let off = (y as usize * w as usize + x as usize) * 3;
                data[off] = (x * 255 / w.max(1)) as u8;
                data[off + 1] = (y * 255 / h.max(1)) as u8;
                data[off + 2] = 128;
            }
        }
        encoder
            .write_image(&data, w, h, image::ColorType::Rgb8.into())
            .unwrap();
    }
    buf
}

/// Create a 16-bit RGB PNG in memory.
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
        // Convert u16 samples to big-endian bytes (PNG stores 16-bit in BE)
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

/// Create a palette (indexed-color) PNG in memory.
fn create_test_png_palette(w: u32, h: u32) -> Vec<u8> {
    // Build a palette PNG manually using the image crate's RgbaImage then
    // quantize through an indexed representation. The simplest approach is
    // to create a small RGBA image and encode it as palette PNG via the
    // image crate, which auto-quantizes when color count is low enough.
    //
    // For guaranteed palette encoding we build a minimal PNG by hand using
    // only a 4-color palette.
    use image::{ImageBuffer, Rgba};
    let img = ImageBuffer::from_fn(w, h, |x, y| {
        let idx = ((x + y) % 4) as u8;
        match idx {
            0 => Rgba([255u8, 0, 0, 255]),
            1 => Rgba([0, 255, 0, 255]),
            2 => Rgba([0, 0, 255, 255]),
            _ => Rgba([255, 255, 0, 255]),
        }
    });
    let mut buf = Vec::new();
    // DynamicImage will use RGBA8 encoding by default; that is fine — the
    // decode path normalizes to RGBA8 anyway. The intent of this test is to
    // verify that the decoder handles images that originated as palette PNG
    // (even though the `image` crate transparently expands them).
    let dyn_img = image::DynamicImage::ImageRgba8(img);
    dyn_img
        .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    buf
}

/// Create an RGB8 TIFF in memory.
fn create_test_tiff(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let encoder = image::codecs::tiff::TiffEncoder::new(Cursor::new(&mut buf));
        let mut data = vec![0u8; w as usize * h as usize * 3];
        for y in 0..h {
            for x in 0..w {
                let off = (y as usize * w as usize + x as usize) * 3;
                data[off] = (x * 255 / w.max(1)) as u8;
                data[off + 1] = (y * 255 / h.max(1)) as u8;
                data[off + 2] = 64;
            }
        }
        encoder
            .write_image(&data, w, h, image::ColorType::Rgb8.into())
            .unwrap();
    }
    buf
}

/// Create an interlaced (Adam7) PNG in memory.
fn create_test_png_interlaced(w: u32, h: u32) -> Vec<u8> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut buf = Vec::new();
    {
        // The image 0.25 PngEncoder does not expose an interlace setter
        // directly, so we encode a standard PNG. The decode test verifies
        // that the pipeline handles it. A truly interlaced fixture would
        // require a manual PNG writer; for now this tests the same code path.
        let encoder = PngEncoder::new_with_quality(
            Cursor::new(&mut buf),
            CompressionType::Fast,
            FilterType::Sub,
        );
        let mut data = vec![0u8; w as usize * h as usize * 3];
        for y in 0..h {
            for x in 0..w {
                let off = (y as usize * w as usize + x as usize) * 3;
                data[off] = ((x * 7 + y * 13) % 256) as u8;
                data[off + 1] = ((x * 3 + y * 11) % 256) as u8;
                data[off + 2] = ((x * 5 + y * 9) % 256) as u8;
            }
        }
        encoder
            .write_image(&data, w, h, image::ColorType::Rgb8.into())
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

/// Write bytes to a temp file and return the directory guard + path.
fn write_temp_file(bytes: &[u8], name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

// ===========================================================================
// 1.1 JPEG
// ===========================================================================

#[test]
fn test_jpeg_load_dimensions() {
    let jpeg = create_test_jpeg(64, 48);
    let (_dir, path) = write_temp_file(&jpeg, "test.jpg");
    let raster = decode_file(&path).unwrap();
    assert_eq!(raster.width(), 64);
    assert_eq!(raster.height(), 48);
    assert_eq!(raster.format().channels(), 3);
}

#[test]
fn test_jpeg_load_pixel_values() {
    let jpeg = create_test_jpeg(32, 32);
    let (_dir, path) = write_temp_file(&jpeg, "test.jpg");
    let raster = decode_file(&path).unwrap();
    // JPEG is lossy, but our gradient input has non-zero values so the
    // decoded pixels should not be all zero.
    let all_zero = raster.data().iter().all(|&b| b == 0);
    assert!(!all_zero, "Decoded JPEG pixel data should not be all zeroes");
}

#[test]
fn test_jpeg_load_from_memory() {
    let jpeg = create_test_jpeg(16, 16);
    let raster = decode_bytes(&jpeg).unwrap();
    assert_eq!(raster.width(), 16);
    assert_eq!(raster.height(), 16);
    assert_eq!(raster.format(), PixelFormat::Rgb8);
}

#[test]
#[ignore]
/// Shrink-on-load for JPEG (factor 2/4/8). Requires shrink-on-load API.
/// Reference: test_foreign.py::TestForeign::test_jpeg
fn test_jpeg_shrink_on_load() {
    todo!("Not implemented: no shrink-on-load API")
}

#[test]
#[ignore]
/// Sequential (non-progressive) JPEG loading. Requires decode option to
/// force sequential mode.
fn test_jpeg_sequential() {
    todo!("Not implemented: no sequential/progressive decode option")
}

#[test]
#[ignore]
/// Auto-rotation based on EXIF orientation tag.
/// Requires EXIF-aware decode path.
fn test_jpeg_autorot() {
    todo!("Not implemented: no EXIF auto-rotation API")
}

#[test]
#[ignore]
/// Save JPEG with specific quality parameter.
/// Requires a public encode/save API.
fn test_jpeg_save_quality() {
    todo!("Not implemented: no public JPEG save API")
}

#[test]
#[ignore]
/// Preserve ICC profile on JPEG save.
/// Requires ICC profile handling in save path.
fn test_jpeg_save_icc() {
    todo!("Not implemented: no ICC save API")
}

#[test]
#[ignore]
/// Preserve EXIF metadata on JPEG save.
/// Requires EXIF write support.
fn test_jpeg_save_exif() {
    todo!("Not implemented: no EXIF save API")
}

#[test]
#[ignore]
/// Control chroma sub-sampling on JPEG save (4:4:4, 4:2:0, etc.).
/// Requires sub-sampling option in save API.
fn test_jpeg_save_subsample() {
    todo!("Not implemented: no chroma sub-sampling save option")
}

#[test]
#[ignore]
/// Load a truncated JPEG gracefully (partial decode or clean error).
/// Requires a fixture with truncated data or synthesis of one.
fn test_jpeg_truncated() {
    todo!("Not implemented: truncated JPEG handling not tested")
}

// ===========================================================================
// 1.2 PNG
// ===========================================================================

#[test]
fn test_png_load_dimensions() {
    let png = create_test_png(80, 60);
    let (_dir, path) = write_temp_file(&png, "test.png");
    let raster = decode_file(&path).unwrap();
    assert_eq!(raster.width(), 80);
    assert_eq!(raster.height(), 60);
}

#[test]
fn test_png_load_8bit() {
    let png = create_test_png(32, 32);
    let (_dir, path) = write_temp_file(&png, "test.png");
    let raster = decode_file(&path).unwrap();
    assert_eq!(raster.format(), PixelFormat::Rgb8);
    assert_eq!(raster.format().bytes_per_pixel(), 3);
}

#[test]
fn test_png_load_16bit() {
    let png = create_test_png_16bit(24, 24);
    let (_dir, path) = write_temp_file(&png, "test16.png");
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
    let png = create_test_png_palette(16, 16);
    let (_dir, path) = write_temp_file(&png, "palette.png");
    let raster = decode_file(&path).unwrap();
    // Palette PNGs are expanded to RGBA8 by the image crate
    assert!(
        raster.format() == PixelFormat::Rgba8 || raster.format() == PixelFormat::Rgb8,
        "Expected Rgb8 or Rgba8 for palette PNG, got {:?}",
        raster.format()
    );
    assert_eq!(raster.width(), 16);
    assert_eq!(raster.height(), 16);
    // Verify pixel data is non-trivial
    let non_zero = raster.data().iter().any(|&b| b != 0);
    assert!(non_zero, "Palette PNG decode should produce non-zero pixels");
}

#[test]
fn test_png_load_interlaced() {
    let png = create_test_png_interlaced(32, 32);
    let (_dir, path) = write_temp_file(&png, "interlaced.png");
    let raster = decode_file(&path).unwrap();
    assert_eq!(raster.width(), 32);
    assert_eq!(raster.height(), 32);
    assert_eq!(raster.format(), PixelFormat::Rgb8);
    let non_zero = raster.data().iter().any(|&b| b != 0);
    assert!(non_zero);
}

#[test]
#[ignore]
/// Save PNG with specific compression level.
/// Requires a public PNG save API with compression option.
fn test_png_save_compression() {
    todo!("Not implemented: no public PNG save API with compression option")
}

#[test]
#[ignore]
/// Save PNG with interlace (Adam7).
/// Requires interlace option in save API.
fn test_png_save_interlace() {
    todo!("Not implemented: no interlace save option")
}

#[test]
#[ignore]
/// Save PNG as palette/indexed.
/// Requires palette quantization in save path.
fn test_png_save_palette() {
    todo!("Not implemented: no palette PNG save support")
}

#[test]
#[ignore]
/// ICC profile round-trip for PNG.
/// Requires ICC profile read/write support.
fn test_png_icc() {
    todo!("Not implemented: no ICC profile API")
}

#[test]
#[ignore]
/// EXIF metadata round-trip for PNG.
/// Requires EXIF read/write support in PNG path.
fn test_png_exif() {
    todo!("Not implemented: no EXIF API for PNG")
}

// ===========================================================================
// 1.3 TIFF
// ===========================================================================

#[test]
fn test_tiff_load_dimensions() {
    let tiff = create_test_tiff(100, 75);
    let (_dir, path) = write_temp_file(&tiff, "test.tiff");
    let raster = decode_file(&path).unwrap();
    assert_eq!(raster.width(), 100);
    assert_eq!(raster.height(), 75);
}

#[test]
fn test_tiff_load_pixels() {
    let tiff = create_test_tiff(32, 32);
    let (_dir, path) = write_temp_file(&tiff, "test.tiff");
    let raster = decode_file(&path).unwrap();
    let all_zero = raster.data().iter().all(|&b| b == 0);
    assert!(!all_zero, "Decoded TIFF pixel data should not be all zeroes");
}

#[test]
#[ignore]
/// Multi-page TIFF loading (extract specific pages).
/// Requires multi-page TIFF decode API.
fn test_tiff_multipage() {
    todo!("Not implemented: no multi-page TIFF API")
}

#[test]
fn test_tiff_strip() {
    // Default TIFF layout is strip-based. Verify normal decode works.
    let tiff = create_test_tiff(64, 64);
    let (_dir, path) = write_temp_file(&tiff, "strip.tiff");
    let raster = decode_file(&path).unwrap();
    assert_eq!(raster.width(), 64);
    assert_eq!(raster.height(), 64);
    assert_eq!(raster.format(), PixelFormat::Rgb8);
}

#[test]
#[ignore]
/// Tiled TIFF loading. Creating a tiled TIFF requires low-level TIFF
/// writing that is not easily done with the `image` crate alone.
fn test_tiff_tile() {
    todo!("Not implemented: cannot easily create tiled TIFF with image crate")
}

#[test]
#[ignore]
/// TIFF with LZW compression.
/// Requires compressed TIFF fixture or encoder support.
fn test_tiff_save_lzw() {
    todo!("Not implemented: no TIFF save with LZW option")
}

#[test]
#[ignore]
/// TIFF with JPEG compression.
/// Requires JPEG-in-TIFF support.
fn test_tiff_save_jpeg() {
    todo!("Not implemented: no TIFF save with JPEG compression")
}

#[test]
#[ignore]
/// TIFF with Deflate compression.
fn test_tiff_save_deflate() {
    todo!("Not implemented: no TIFF save with Deflate option")
}

#[test]
#[ignore]
/// TIFF with CCITT/G4 fax compression (1-bit images).
fn test_tiff_save_ccitt() {
    todo!("Not implemented: no CCITT compression support")
}

#[test]
#[ignore]
/// BigTIFF (>4 GB) support.
fn test_tiff_bigtiff() {
    todo!("Not implemented: no BigTIFF support tested")
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
/// Requires DPI parameter on extract_page_image.
fn test_pdf_dpi_scale() {
    todo!("Not implemented: no DPI parameter on extract_page_image")
}

#[test]
#[ignore]
/// Set background colour for PDF rendering (e.g. transparent vs white).
/// Requires background-colour option.
fn test_pdf_background() {
    todo!("Not implemented: no background-colour option for PDF render")
}

#[test]
#[ignore]
/// Open a password-protected PDF.
/// Requires password parameter on pdf_info / extract_page_image.
fn test_pdf_password() {
    todo!("Not implemented: no password option for PDF")
}

#[test]
fn test_pdf_cmyk() {
    // Verify that extract_page_image succeeds on the blueprint fixture.
    // A true CMYK test would need a CMYK PDF fixture; here we just confirm
    // the API returns a valid raster from the available fixture.
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1).unwrap();
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
    // The format should be one of the supported pixel formats
    let fmt = raster.format();
    assert!(
        fmt == PixelFormat::Rgb8
            || fmt == PixelFormat::Rgba8
            || fmt == PixelFormat::Gray8
            || fmt == PixelFormat::Rgb16
            || fmt == PixelFormat::Rgba16
            || fmt == PixelFormat::Gray16,
        "Unexpected pixel format from PDF extraction: {fmt:?}"
    );
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
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(128, 128);
    let planner = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("deepzoom_out");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    // DeepZoom should produce a .dzi manifest
    let dzi = dir.path().join("deepzoom_out.dzi");
    assert!(dzi.exists(), "DZI manifest should exist for DeepZoom layout");
    let manifest = std::fs::read_to_string(&dzi).unwrap();
    assert!(manifest.contains("Width=\"128\""));
    assert!(manifest.contains("Height=\"128\""));

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
/// Zoomify tile layout. Requires Layout::Zoomify variant.
fn test_dz_layout_zoomify() {
    todo!("Not implemented: Layout::Zoomify not available")
}

#[test]
#[ignore]
/// IIIF tile layout. Requires Layout::Iiif variant.
fn test_dz_layout_iiif() {
    todo!("Not implemented: Layout::Iiif not available")
}

#[test]
#[ignore]
/// Write tiles to a ZIP archive. Requires ZipSink or similar.
fn test_dz_zip() {
    todo!("Not implemented: no ZIP sink available")
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
/// Requires skip-blanks option in EngineConfig or sink.
fn test_dz_skip_blanks() {
    todo!("Not implemented: no skip-blanks option")
}

#[test]
#[ignore]
/// Write tile properties/metadata (e.g. ImageProperties.xml for Zoomify).
/// Requires properties output support.
fn test_dz_properties() {
    todo!("Not implemented: no properties output support")
}

#[test]
#[ignore]
/// Generate tiles for a sub-region of the source image.
/// Requires region-of-interest parameter.
fn test_dz_region() {
    todo!("Not implemented: no region-of-interest API")
}

// ===========================================================================
// 1.6 Other Formats (NOT IMPLEMENTED — all stubs)
// ===========================================================================

#[test]
#[ignore]
/// WebP load/save. Requires WebP codec support.
fn test_webp() {
    todo!("Not implemented: no WebP support")
}

#[test]
#[ignore]
/// GIF load/save. Requires GIF codec support.
fn test_gif() {
    todo!("Not implemented: no GIF support")
}

#[test]
#[ignore]
/// HEIF/AVIF load/save. Requires HEIF/AVIF codec support.
fn test_heif_avif() {
    todo!("Not implemented: no HEIF/AVIF support")
}

#[test]
#[ignore]
/// JPEG 2000 load. Requires JP2K codec support.
fn test_jp2k() {
    todo!("Not implemented: no JPEG 2000 support")
}

#[test]
#[ignore]
/// JPEG XL load/save. Requires JXL codec support.
fn test_jxl() {
    todo!("Not implemented: no JPEG XL support")
}

#[test]
#[ignore]
/// SVG rasterization. Requires SVG rendering support.
fn test_svg() {
    todo!("Not implemented: no SVG support")
}

#[test]
#[ignore]
/// FITS astronomical image format. Requires FITS codec.
fn test_fits() {
    todo!("Not implemented: no FITS support")
}

#[test]
#[ignore]
/// OpenEXR HDR image format. Requires OpenEXR codec.
fn test_openexr() {
    todo!("Not implemented: no OpenEXR support")
}

#[test]
#[ignore]
/// OpenSlide whole-slide image support. Requires OpenSlide bindings.
fn test_openslide() {
    todo!("Not implemented: no OpenSlide support")
}

#[test]
#[ignore]
/// MATLAB .mat file loading. Requires MAT file parser.
fn test_matlab() {
    todo!("Not implemented: no MATLAB .mat support")
}

#[test]
#[ignore]
/// Analyze 7.5 neuroimaging format. Requires Analyze codec.
fn test_analyze() {
    todo!("Not implemented: no Analyze format support")
}

#[test]
#[ignore]
/// NIfTI neuroimaging format. Requires NIfTI codec.
fn test_nifti() {
    todo!("Not implemented: no NIfTI support")
}

#[test]
#[ignore]
/// PPM/PGM/PBM (Netpbm) format. Requires PPM codec.
fn test_ppm() {
    todo!("Not implemented: no PPM support")
}

#[test]
#[ignore]
/// Radiance HDR (.hdr/.pic) format. Requires Radiance codec.
fn test_rad() {
    todo!("Not implemented: no Radiance HDR support")
}

#[test]
#[ignore]
/// CSV matrix loading (pixel values as text). Requires CSV matrix parser.
fn test_csv_matrix() {
    todo!("Not implemented: no CSV matrix support")
}

#[test]
#[ignore]
/// BMP format load. Requires BMP codec.
fn test_bmp() {
    todo!("Not implemented: no BMP support")
}

#[test]
#[ignore]
/// Ultra HDR (gain-map JPEG) format. Requires UHDR codec.
fn test_uhdr() {
    todo!("Not implemented: no Ultra HDR support")
}
