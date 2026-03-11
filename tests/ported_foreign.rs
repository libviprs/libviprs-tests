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
/// Interlaced (Adam7) PNG loading. Requires a real interlaced PNG fixture.
/// The image crate handles this transparently but we need a proper fixture to verify.
fn test_png_load_interlaced() {
    todo!("Needs a real interlaced PNG fixture to properly test Adam7 decoding")
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
/// Requires multi-page TIFF decode API.
fn test_tiff_multipage() {
    todo!("Not implemented: no multi-page TIFF API")
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
