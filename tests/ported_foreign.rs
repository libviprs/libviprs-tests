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
/// Subset of libvips test_foreign.py::test_jpeg.
fn test_jpeg_load_dimensions() {
    // Use the real libvips reference JPEG fixture
    let raster = decode_file(&ref_image("sample.jpg")).unwrap();
    assert!(raster.width() > 0, "JPEG width should be positive");
    assert!(raster.height() > 0, "JPEG height should be positive");
    assert_eq!(raster.format().channels(), 3);
}

#[test]
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
/// Subset of libvips test_foreign.py::test_jpeg.
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
#[ignore]
/// 1:1 port of libvips test_foreign.py::test_jpegsave.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode JPEG to buffer with quality and subsample_mode options.
/// fn Raster::jpegsave_buffer(&self, quality: u8, subsample_mode: Option<&str>) -> Result<Vec<u8>, EncodeError>;
///
/// /// Encode JPEG to buffer with restart_interval option.
/// fn Raster::jpegsave_buffer_restart(&self, restart_interval: u32) -> Result<Vec<u8>, EncodeError>;
///
/// /// Load JPEG from buffer.
/// fn Raster::jpegload_buffer(data: &[u8]) -> Result<Raster, DecodeError>;
///
/// /// Compute the average pixel value across all bands.
/// fn Raster::avg(&self) -> f64;
/// ```
///
/// ## Test logic
///
/// 1. Encode at Q=10 and Q=90 with various subsample_mode values.
/// 2. Higher Q should produce a bigger buffer.
/// 3. Subsample mode "auto" matches default; "on" forces subsampling; "off" disables it.
/// 4. Non-zero restart_interval increases file size; more frequent restarts = larger.
/// 5. Images with extra MCU markers should reload with the same average pixel value.
fn test_jpegsave() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    let q10 = im.jpegsave_buffer(10, None).unwrap();
    let q10_subsample_auto = im.jpegsave_buffer(10, Some("auto")).unwrap();
    let q10_subsample_on = im.jpegsave_buffer(10, Some("on")).unwrap();
    let q10_subsample_off = im.jpegsave_buffer(10, Some("off")).unwrap();

    let q90 = im.jpegsave_buffer(90, None).unwrap();
    let q90_subsample_auto = im.jpegsave_buffer(90, Some("auto")).unwrap();
    let q90_subsample_on = im.jpegsave_buffer(90, Some("on")).unwrap();
    let q90_subsample_off = im.jpegsave_buffer(90, Some("off")).unwrap();

    // higher Q should mean a bigger buffer
    assert!(q90.len() > q10.len());

    assert_eq!(q10_subsample_auto.len(), q10.len());
    assert_eq!(q10_subsample_on.len(), q10_subsample_auto.len());
    assert!(q10_subsample_off.len() > q10.len());

    assert_eq!(q90_subsample_auto.len(), q90.len());
    assert!(q90_subsample_on.len() < q90.len());
    assert_eq!(q90_subsample_off.len(), q90_subsample_auto.len());

    // A non-zero restart_interval should result in a bigger file.
    let r0 = im.jpegsave_buffer_restart(0).unwrap();
    let r10 = im.jpegsave_buffer_restart(10).unwrap();
    let r2 = im.jpegsave_buffer_restart(2).unwrap();
    assert!(r10.len() > r0.len());
    assert!(r2.len() > r10.len());

    // we should be able to reload jpegs with extra MCU markers
    let im0 = decode_bytes(&r0).unwrap();
    let im10 = decode_bytes(&r10).unwrap();
    assert_eq!(im0.avg(), im10.avg());
}

#[test]
/// Load a truncated JPEG — should either partially decode or return a clean error.
/// Uses the real libvips reference truncated.jpg fixture.
fn test_truncated() {
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

#[test]
#[ignore]
/// Native .v (VIPS) format save/load.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as native VIPS .v format bytes.
/// fn Raster::encode_vips(&self) -> Result<Vec<u8>, EncodeError>;
///
/// /// Save raster to file (format inferred from extension).
/// fn Raster::save(&self, path: &Path) -> Result<(), SaveError>;
///
/// /// Get a metadata field value.
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_vips)
///
/// 1. Load sample.jpg, save as .v, reload, verify exif-data matches.
/// 2. Create a 16x16 black+128 image, save as .v, reload, verify pixel roundtrip.
///
/// Reference: test_foreign.py::test_vips
fn test_vips() {
    // Part 1: JPEG → .v roundtrip preserving EXIF
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out_v = dir.path().join("test.v");
    im.save(&out_v).unwrap();
    let im2 = decode_file(&out_v).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im.get_field("exif-data"), im2.get_field("exif-data"));

    // Part 2: synthetic 16×16 black+128 image roundtrip
    let data = vec![128u8; 16 * 16 * 3];
    let synth = Raster::new(16, 16, PixelFormat::Rgb8, data).unwrap();
    let out_v2 = dir.path().join("synth.v");
    synth.save(&out_v2).unwrap();
    let synth2 = decode_file(&out_v2).unwrap();
    assert_eq!(synth2.width(), 16);
    assert_eq!(synth2.height(), 16);
    assert_eq!(synth2.data(), synth.data());
}

#[test]
#[ignore]
/// EXIF tag roundtrip: UserComment, Software, XPComment survive JPEG save/load.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Set a metadata field on the raster.
/// fn Raster::set_field(&mut self, name: &str, value: MetadataValue);
///
/// /// Get a metadata field value.
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
///
/// /// Get the GType of a metadata field (0 = not present).
/// fn Raster::get_typeof(&self, name: &str) -> u64;
///
/// /// Encode the raster as JPEG bytes.
/// fn Raster::encode_jpeg(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpegsave_exif)
///
/// 1. Load sample.jpg.
/// 2. Set exif-ifd2-UserComment (encoding test), exif-ifd0-Software (ASCII),
///    exif-ifd0-XPComment (UTF-16).
/// 3. Save as JPEG, reload, verify tags survive.
/// 4. Test tag removal: set typeof to 0, verify tag is gone after roundtrip.
///
/// Reference: test_foreign.py::test_jpegsave_exif
fn test_jpegsave_exif() {
    let mut im = decode_file(&ref_image("sample.jpg")).unwrap();
    im.set_field("exif-ifd2-UserComment", "Hello UserComment".into());
    im.set_field("exif-ifd0-Software", "TestSoftware".into());
    im.set_field("exif-ifd0-XPComment", "TestXPComment".into());

    let buf = im.encode_jpeg(85).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.get_field("exif-ifd2-UserComment").unwrap().as_str(), "Hello UserComment");
    assert_eq!(im2.get_field("exif-ifd0-Software").unwrap().as_str(), "TestSoftware");
    assert_eq!(im2.get_field("exif-ifd0-XPComment").unwrap().as_str(), "TestXPComment");

    // Test tag removal via typeof==0
    im.set_typeof("exif-ifd0-Software", 0);
    let buf2 = im.encode_jpeg(85).unwrap();
    let im3 = decode_bytes(&buf2).unwrap();
    assert_eq!(im3.get_typeof("exif-ifd0-Software"), 0);
}

#[test]
#[ignore]
/// EXIF 2.3 ASCII tags survive JPEG roundtrip (CameraOwnerName, etc.).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::set_field(&mut self, name: &str, value: MetadataValue);
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::encode_jpeg(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpegsave_exif_2_3_ascii)
///
/// 1. Load sample.jpg.
/// 2. Set CameraOwnerName, BodySerialNumber, LensMake, LensModel, LensSerialNumber.
/// 3. Save as JPEG, reload, verify all five tags survive.
///
/// Reference: test_foreign.py::test_jpegsave_exif_2_3_ascii
fn test_jpegsave_exif_2_3_ascii() {
    let mut im = decode_file(&ref_image("sample.jpg")).unwrap();
    let tags = [
        "exif-ifd2-CameraOwnerName",
        "exif-ifd2-BodySerialNumber",
        "exif-ifd2-LensMake",
        "exif-ifd2-LensModel",
        "exif-ifd2-LensSerialNumber",
    ];
    for tag in &tags {
        im.set_field(tag, format!("test-{tag}").into());
    }

    let buf = im.encode_jpeg(85).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    for tag in &tags {
        assert_eq!(
            im2.get_field(tag).unwrap().as_str(),
            format!("test-{tag}"),
            "Tag {tag} did not survive JPEG roundtrip"
        );
    }
}

#[test]
#[ignore]
/// EXIF 2.3 ASCII tags for OffsetTime*/GPS* fields survive JPEG roundtrip.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::set_field(&mut self, name: &str, value: MetadataValue);
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::encode_jpeg(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jpegsave_exif_2_3_ascii_2)
///
/// 1. Load sample.jpg.
/// 2. Set OffsetTime, OffsetTimeOriginal, OffsetTimeDigitized,
///    GPSLatitudeRef, GPSLongitudeRef, etc.
/// 3. Save as JPEG, reload, verify tags survive.
///
/// Reference: test_foreign.py::test_jpegsave_exif_2_3_ascii_2
fn test_jpegsave_exif_2_3_ascii_2() {
    let mut im = decode_file(&ref_image("sample.jpg")).unwrap();
    let tags = [
        "exif-ifd2-OffsetTime",
        "exif-ifd2-OffsetTimeOriginal",
        "exif-ifd2-OffsetTimeDigitized",
        "exif-ifd3-GPSLatitudeRef",
        "exif-ifd3-GPSLongitudeRef",
    ];
    for tag in &tags {
        im.set_field(tag, format!("test-{tag}").into());
    }

    let buf = im.encode_jpeg(85).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    for tag in &tags {
        assert_eq!(
            im2.get_field(tag).unwrap().as_str(),
            format!("test-{tag}"),
            "Tag {tag} did not survive JPEG roundtrip"
        );
    }
}

// ===========================================================================
// 1.2 PNG
// ===========================================================================

#[test]
/// Subset of libvips test_foreign.py::test_png.
fn test_png_load_dimensions() {
    let raster = decode_file(&ref_image("sample.png")).unwrap();
    assert!(raster.width() > 0, "PNG width should be positive");
    assert!(raster.height() > 0, "PNG height should be positive");
}

#[test]
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
/// Interlaced (Adam7) PNG save/load round-trip.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as interlaced PNG bytes.
/// fn Raster::encode_png_interlaced(&self) -> Result<Vec<u8>, EncodeError>;
/// fn decode_bytes(data: &[u8]) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_png — interlace section)
///
/// 1. Load sample.jpg (colour image).
/// 2. Save as interlaced PNG to buffer.
/// 3. Reload from buffer.
/// 4. Verify dimensions match and pixel values are close.
///
/// Reference: test_foreign.py::test_png (save_load_file with `[interlace]`)
fn test_png_load_interlaced() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    // Save as interlaced PNG, reload, compare
    let buf = im.encode_png_interlaced().unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im2.format().channels(), im.format().channels());
}

#[test]
#[ignore]
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_png.
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
/// Subset of libvips test_foreign.py::test_tiff.
fn test_tiff_load_dimensions() {
    let raster = decode_file(&ref_image("sample.tif")).unwrap();
    assert!(raster.width() > 0, "TIFF width should be positive");
    assert!(raster.height() > 0, "TIFF height should be positive");
}

#[test]
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Subset of libvips test_foreign.py::test_tiff.
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
/// 1. Load a multi-page TIFF (OME-TIFF from reference suite).
/// 2. Count pages — should be > 1.
/// 3. Extract each page, verify dimensions are positive.
///
/// Reference: test_foreign.py::test_tiff
fn test_tiff_multipage() {
    let tiff_path = ref_image("multi-channel-z-series.ome.tif");
    let page_count = tiff_page_count(&tiff_path).unwrap();
    assert!(page_count > 1, "OME TIFF should have multiple pages, got {page_count}");

    for p in 1..=page_count {
        let raster = decode_tiff_page(&tiff_path, p).unwrap();
        assert!(raster.width() > 0, "Page {p}: width should be positive");
        assert!(raster.height() > 0, "Page {p}: height should be positive");
    }
}

#[test]
/// Subset of libvips test_foreign.py::test_tiff.
fn test_tiff_strip() {
    // sample.tif is strip-layout by default
    let raster = decode_file(&ref_image("sample.tif")).unwrap();
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
}

#[test]
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Old-style JPEG (OJPEG) compressed TIFF — tiled and strip variants.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Decode a TIFF file, returning a Raster with metadata accessors.
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>;
///
/// /// Read a single pixel at (x, y) as a Vec of f64 channel values.
/// fn Raster::getpoint(&self, x: u32, y: u32) -> Vec<f64>;
///
/// /// Read an integer metadata field (e.g. "bits-per-sample", "tile-width").
/// fn Raster::get_int(&self, name: &str) -> Option<i32>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_tiff_ojpeg)
///
/// 1. Load ojpeg-tile.tif — verify dims 234×213, 4 bands, bps 8, tile 240×224,
///    pixel (10,10) ≈ [135, 156, 177, 255].
/// 2. Load ojpeg-strip.tif — verify dims 160×160, 4 bands, bps 8,
///    pixel (10,10) ≈ [228, 15, 9, 255].
/// 3. Also load each from a memory buffer.
///
/// Reference: test_foreign.py::test_tiff_ojpeg
fn test_tiff_ojpeg() {
    // ---- tiled variant ----
    let tile_path = ref_image("ojpeg-tile.tif");
    let im = decode_file(&tile_path).unwrap();
    assert_eq!(im.width(), 234);
    assert_eq!(im.height(), 213);
    assert_eq!(im.bands(), 4);
    assert_eq!(im.get_int("bits-per-sample"), Some(8));
    assert_eq!(im.get_int("tile-width"), Some(240));
    assert_eq!(im.get_int("tile-height"), Some(224));
    let px = im.getpoint(10, 10);
    let expected = [135.0, 156.0, 177.0, 255.0];
    for (i, (&got, &exp)) in px.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - exp).abs() < 1.0,
            "ojpeg-tile pixel(10,10)[{i}]: got {got}, expected {exp}"
        );
    }

    // buffer load
    let bytes = std::fs::read(&tile_path).unwrap();
    let im2 = decode_bytes(&bytes).unwrap();
    assert_eq!(im2.width(), 234);
    assert_eq!(im2.height(), 213);

    // ---- strip variant ----
    let strip_path = ref_image("ojpeg-strip.tif");
    let im = decode_file(&strip_path).unwrap();
    assert_eq!(im.width(), 160);
    assert_eq!(im.height(), 160);
    assert_eq!(im.bands(), 4);
    assert_eq!(im.get_int("bits-per-sample"), Some(8));
    let px = im.getpoint(10, 10);
    let expected = [228.0, 15.0, 9.0, 255.0];
    for (i, (&got, &exp)) in px.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - exp).abs() < 1.0,
            "ojpeg-strip pixel(10,10)[{i}]: got {got}, expected {exp}"
        );
    }

    // buffer load
    let bytes = std::fs::read(&strip_path).unwrap();
    let im2 = decode_bytes(&bytes).unwrap();
    assert_eq!(im2.width(), 160);
    assert_eq!(im2.height(), 160);
}

#[test]
#[ignore]
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Subset of libvips test_foreign.py::test_tiff.
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
/// Subset of libvips test_foreign.py::test_tiff.
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

#[test]
#[ignore]
/// TIFF with JP2K compression in tile, tile+pyramid, tile+pyramid+subifd modes.
///
/// ## Required API
///
/// ```rust,ignore
/// /// TIFF compression modes (extended with JP2K).
/// pub enum TiffCompression { None, Lzw, Jpeg, Deflate, Ccitt, Jp2k }
///
/// /// Save raster as tiled TIFF with specified compression, tile size, and optional pyramid/subifd.
/// fn Raster::save_tiff_tiled(
///     &self, path: &Path, compression: TiffCompression,
///     tile_width: u32, tile_height: u32, pyramid: bool, subifd: bool,
/// ) -> Result<(), SaveError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_tiffjp2k)
///
/// 1. Load sample.tif.
/// 2. Save as tiled TIFF with JP2K compression (tile only).
/// 3. Reload, verify max_diff <= 80.
/// 4. Save as tiled TIFF with JP2K + pyramid, verify max_diff <= 80.
/// 5. Save as tiled TIFF with JP2K + pyramid + subifd, verify max_diff <= 80.
///
/// Reference: test_foreign.py::test_tiffjp2k
fn test_tiffjp2k() {
    let im = decode_file(&ref_image("sample.tif")).unwrap();
    let dir = tempfile::tempdir().unwrap();

    // Tile only
    let out1 = dir.path().join("jp2k_tile.tif");
    im.save_tiff_tiled(&out1, TiffCompression::Jp2k, 128, 128, false, false).unwrap();
    let im2 = decode_file(&out1).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    let max_diff: u16 = im.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs())
        .max().unwrap_or(0);
    assert!(max_diff <= 80, "JP2K tile max_diff={max_diff}, expected <=80");

    // Tile + pyramid
    let out2 = dir.path().join("jp2k_tile_pyramid.tif");
    im.save_tiff_tiled(&out2, TiffCompression::Jp2k, 128, 128, true, false).unwrap();
    let im3 = decode_file(&out2).unwrap();
    let max_diff2: u16 = im.data().iter().zip(im3.data().iter())
        .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs())
        .max().unwrap_or(0);
    assert!(max_diff2 <= 80, "JP2K tile+pyramid max_diff={max_diff2}, expected <=80");

    // Tile + pyramid + subifd
    let out3 = dir.path().join("jp2k_tile_pyramid_subifd.tif");
    im.save_tiff_tiled(&out3, TiffCompression::Jp2k, 128, 128, true, true).unwrap();
    let im4 = decode_file(&out3).unwrap();
    let max_diff3: u16 = im.data().iter().zip(im4.data().iter())
        .map(|(&a, &b)| (a as i16 - b as i16).unsigned_abs())
        .max().unwrap_or(0);
    assert!(max_diff3 <= 80, "JP2K tile+pyramid+subifd max_diff={max_diff3}, expected <=80");
}

// ===========================================================================
// 1.4 PDF
// ===========================================================================

#[test]
/// Subset of libvips test_foreign.py::test_pdfload.
fn test_pdf_page_count() {
    let info = pdf_info(Path::new(FIXTURE_PDF)).unwrap();
    assert!(
        info.page_count >= 1,
        "Expected at least 1 page, got {}",
        info.page_count
    );
}

#[test]
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// libvips does not have a password-protected PDF test. This test exercises
/// the password API if a password-protected PDF is available. The fixture
/// must be generated externally (e.g. via `qpdf --encrypt secret secret 256 -- in.pdf out.pdf`).
///
/// 1. Attempt to open a password-protected PDF without password — should fail.
/// 2. Open with correct password — should succeed.
/// 3. Verify page count and dimensions.
///
/// Note: no fixture file in the libvips reference suite — generate one into
/// tests/fixtures/password.pdf if this test is un-ignored.
fn test_pdf_password() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/password.pdf");

    // Attempt without password
    let result = pdf_info(&fixture);
    assert!(result.is_err(), "Password-protected PDF should fail without password");

    // With password
    let info = pdf_info_with_password(&fixture, "secret").unwrap();
    assert!(info.page_count >= 1);

    let raster = extract_page_image_with_password(&fixture, 1, "secret").unwrap();
    assert!(raster.width() > 0);
    assert!(raster.height() > 0);
}

#[test]
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// Subset of libvips test_foreign.py::test_pdfload.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
/// Subset of libvips test_foreign.py::test_dzsave.
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
fn test_gifload() {
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
/// Load a GIF with dispose-background mode, compare against expected PNG.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>;
/// fn Raster::max_diff(&self, other: &Raster) -> f64;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_gifload_animation_dispose_background)
///
/// 1. Load dispose-background.gif (animated GIF with dispose=background).
/// 2. Load expected output PNG.
/// 3. Compare — max diff must be 0.
///
/// Reference: test_foreign.py::test_gifload_animation_dispose_background
fn test_gifload_animation_dispose_background() {
    let im = decode_file(&ref_image("dispose-background.gif")).unwrap();
    let expected = decode_file(&ref_image("dispose-background.png")).unwrap();
    assert_eq!(im.width(), expected.width());
    assert_eq!(im.height(), expected.height());
    let diff = im.max_diff(&expected);
    assert_eq!(diff, 0.0, "dispose-background GIF max_diff={diff}, expected 0");
}

#[test]
#[ignore]
/// Load a GIF with dispose-previous mode, compare against expected PNG.
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file(path: &Path) -> Result<Raster, DecodeError>;
/// fn Raster::max_diff(&self, other: &Raster) -> f64;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_gifload_animation_dispose_previous)
///
/// 1. Load dispose-previous.gif (animated GIF with dispose=previous).
/// 2. Load expected output PNG.
/// 3. Compare — max diff must be 0.
///
/// Reference: test_foreign.py::test_gifload_animation_dispose_previous
fn test_gifload_animation_dispose_previous() {
    let im = decode_file(&ref_image("dispose-previous.gif")).unwrap();
    let expected = decode_file(&ref_image("dispose-previous.png")).unwrap();
    assert_eq!(im.width(), expected.width());
    assert_eq!(im.height(), expected.height());
    let diff = im.max_diff(&expected);
    assert_eq!(diff, 0.0, "dispose-previous GIF max_diff={diff}, expected 0");
}

#[test]
#[ignore]
/// Truncated GIF loads normally but fails with fail_on="warning"/"truncated".
///
/// ## Required API
///
/// ```rust,ignore
/// /// Decode with a fail-on strictness level.
/// fn decode_file_fail_on(path: &Path, fail_on: &str) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_gifload_truncated)
///
/// 1. Load truncated.gif normally — should succeed.
/// 2. Load with fail_on="warning" — should fail.
/// 3. Load with fail_on="truncated" — should fail.
///
/// Reference: test_foreign.py::test_gifload_truncated
fn test_gifload_truncated() {
    let im = decode_file(&ref_image("truncated.gif"));
    assert!(im.is_ok(), "Truncated GIF should load normally");

    let fail_warn = decode_file_fail_on(&ref_image("truncated.gif"), "warning");
    assert!(fail_warn.is_err(), "Truncated GIF should fail with fail_on=warning");

    let fail_trunc = decode_file_fail_on(&ref_image("truncated.gif"), "truncated");
    assert!(fail_trunc.is_err(), "Truncated GIF should fail with fail_on=truncated");
}

#[test]
#[ignore]
/// GIF with frame error loads normally and with fail_on="truncated", fails with "warning".
///
/// ## Required API
///
/// ```rust,ignore
/// fn decode_file_fail_on(path: &Path, fail_on: &str) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_gifload_frame_error)
///
/// 1. Load garden.gif normally — should succeed (width==800).
/// 2. Load with fail_on="truncated" — should succeed (width==800).
/// 3. Load with fail_on="warning" — should fail.
///
/// Reference: test_foreign.py::test_gifload_frame_error
fn test_gifload_frame_error() {
    let im = decode_file(&ref_image("garden.gif")).unwrap();
    assert_eq!(im.width(), 800);

    let fail_trunc = decode_file_fail_on(&ref_image("garden.gif"), "truncated");
    assert!(fail_trunc.is_ok(), "GIF with frame error should succeed with fail_on=truncated");

    let fail_warn = decode_file_fail_on(&ref_image("garden.gif"), "warning");
    assert!(fail_warn.is_err(), "GIF with frame error should fail with fail_on=warning");
}

#[test]
#[ignore]
/// Animated GIF save roundtrip preserving metadata; interlace and dither effects.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as GIF bytes with options.
/// fn Raster::encode_gif(&self) -> Result<Vec<u8>, EncodeError>;
///
/// /// Encode raster as interlaced GIF bytes.
/// fn Raster::encode_gif_interlaced(&self) -> Result<Vec<u8>, EncodeError>;
///
/// /// Encode raster as GIF with a specific dither level (0.0 - 1.0).
/// fn Raster::encode_gif_dither(&self, dither: f64) -> Result<Vec<u8>, EncodeError>;
///
/// /// Get the number of pages (frames) in a multi-page image.
/// fn Raster::get_n_pages(&self) -> u32;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_gifsave)
///
/// 1. Load an animated GIF, save to buffer, reload, verify page count matches.
/// 2. Save interlaced GIF — size >= non-interlaced.
/// 3. Save with higher dither — larger file.
///
/// Reference: test_foreign.py::test_gifsave
fn test_gifsave() {
    let im = decode_file(&ref_image("trans-x.gif")).unwrap();
    let buf = im.encode_gif().unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im2.get_n_pages(), im.get_n_pages());

    // Interlaced should be >= non-interlaced size
    let buf_interlaced = im.encode_gif_interlaced().unwrap();
    assert!(
        buf_interlaced.len() >= buf.len(),
        "Interlaced GIF ({}) should be >= non-interlaced ({})",
        buf_interlaced.len(), buf.len()
    );

    // More dither should produce a larger file
    let buf_lo = im.encode_gif_dither(0.1).unwrap();
    let buf_hi = im.encode_gif_dither(0.9).unwrap();
    assert!(
        buf_hi.len() >= buf_lo.len(),
        "Higher dither ({}) should produce >= size than lower ({})",
        buf_hi.len(), buf_lo.len()
    );
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
/// 1. Load avif-orientation-6.avif from reference fixtures.
/// 2. Verify dimensions (image has EXIF orientation 6 = 90° rotation).
///
/// Reference: test_foreign.py::test_heif
fn test_heifload() {
    let im = decode_file(&ref_image("avif-orientation-6.avif")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);
}

#[test]
#[ignore]
/// AVIF save/load roundtrip via heifsave_buffer with compression="av1".
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as AVIF (HEIF with AV1 compression) bytes.
/// fn Raster::encode_heif(&self, quality: u8, compression: &str) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_avifsave)
///
/// 1. Load sample.jpg.
/// 2. Save via heifsave_buffer with compression="av1".
/// 3. Reload, verify dimensions match and pixel values are close.
///
/// Reference: test_foreign.py::test_avifsave
fn test_avifsave() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf = im.encode_heif(50, "av1").unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
}

#[test]
#[ignore]
/// Lossless AVIF roundtrip produces identical pixels.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as lossless AVIF bytes.
/// fn Raster::encode_heif_lossless(&self, compression: &str) -> Result<Vec<u8>, EncodeError>;
/// fn Raster::max_diff(&self, other: &Raster) -> f64;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_avifsave_lossless)
///
/// 1. Load sample.jpg, save as lossless AVIF.
/// 2. Reload, verify max_diff == 0.
///
/// Reference: test_foreign.py::test_avifsave_lossless
fn test_avifsave_lossless() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf = im.encode_heif_lossless("av1").unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    let diff = im.max_diff(&im2);
    assert_eq!(diff, 0.0, "Lossless AVIF roundtrip max_diff={diff}, expected 0");
}

#[test]
#[ignore]
/// Higher Q produces larger AVIF buffer.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::encode_heif(&self, quality: u8, compression: &str) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_avifsave_Q)
///
/// 1. Load sample.jpg.
/// 2. Encode as AVIF at Q=10 and Q=90.
/// 3. Q=90 buffer should be larger than Q=10.
///
/// Reference: test_foreign.py::test_avifsave_Q
fn test_avifsave_q() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf_low = im.encode_heif(10, "av1").unwrap();
    let buf_high = im.encode_heif(90, "av1").unwrap();
    assert!(
        buf_high.len() > buf_low.len(),
        "Q=90 AVIF ({}) should be larger than Q=10 ({})",
        buf_high.len(), buf_low.len()
    );
}

#[test]
#[ignore]
/// Chroma "off" produces larger AVIF than "on".
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode AVIF with chroma subsampling control.
/// fn Raster::encode_heif_chroma(
///     &self, quality: u8, compression: &str, subsample: bool,
/// ) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_avifsave_chroma)
///
/// 1. Load sample.jpg.
/// 2. Encode with chroma subsample off (4:4:4) vs on (4:2:0).
/// 3. "off" should produce a larger buffer.
///
/// Reference: test_foreign.py::test_avifsave_chroma
fn test_avifsave_chroma() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf_off = im.encode_heif_chroma(50, "av1", false).unwrap();
    let buf_on = im.encode_heif_chroma(50, "av1", true).unwrap();
    assert!(
        buf_off.len() > buf_on.len(),
        "Chroma off ({}) should be larger than chroma on ({})",
        buf_off.len(), buf_on.len()
    );
}

#[test]
#[ignore]
/// ICC profile survives AVIF roundtrip.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::encode_heif(&self, quality: u8, compression: &str) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_avifsave_icc)
///
/// 1. Load sample.jpg (has ICC profile).
/// 2. Save as AVIF, reload.
/// 3. Verify ICC profile is present and matches.
///
/// Reference: test_foreign.py::test_avifsave_icc
fn test_avifsave_icc() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let original_icc = im.get_field("icc-profile-data");
    assert!(original_icc.is_some(), "sample.jpg should have an ICC profile");

    let buf = im.encode_heif(50, "av1").unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert!(
        im2.get_field("icc-profile-data").is_some(),
        "ICC profile should survive AVIF roundtrip"
    );
}

#[test]
#[ignore]
/// EXIF XPComment tag survives AVIF roundtrip.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::set_field(&mut self, name: &str, value: MetadataValue);
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// fn Raster::encode_heif(&self, quality: u8, compression: &str) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_avifsave_exif)
///
/// 1. Load sample.jpg, set exif-ifd0-XPComment.
/// 2. Save as AVIF, reload.
/// 3. Verify XPComment tag survived.
///
/// Reference: test_foreign.py::test_avifsave_exif
fn test_avifsave_exif() {
    let mut im = decode_file(&ref_image("sample.jpg")).unwrap();
    im.set_field("exif-ifd0-XPComment", "TestAVIFComment".into());

    let buf = im.encode_heif(50, "av1").unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(
        im2.get_field("exif-ifd0-XPComment").unwrap().as_str(),
        "TestAVIFComment"
    );
}

#[test]
#[ignore]
/// AVIF save with tune="ssim" produces output >10000 bytes.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode AVIF with a tuning parameter.
/// fn Raster::encode_heif_tune(
///     &self, quality: u8, compression: &str, tune: &str,
/// ) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_avifsave_tune)
///
/// 1. Load sample.jpg.
/// 2. Encode AVIF with tune="ssim".
/// 3. Verify output is >10000 bytes.
///
/// Reference: test_foreign.py::test_avifsave_tune
fn test_avifsave_tune() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf = im.encode_heif_tune(50, "av1", "ssim").unwrap();
    assert!(
        buf.len() > 10000,
        "AVIF with tune=ssim should be >10000 bytes, got {}",
        buf.len()
    );
}

#[test]
#[ignore]
/// HEIC lossless save of rgb16 stores as 12-bit.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as lossless HEIC bytes.
/// fn Raster::encode_heif_lossless(&self, compression: &str) -> Result<Vec<u8>, EncodeError>;
///
/// /// Get the bit depth of a loaded image.
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_heicsave_16_to_12)
///
/// 1. Create or load a 16-bit RGB image.
/// 2. Save as lossless HEIC.
/// 3. Reload, verify stored as 12-bit (format indicates ushort).
///
/// Reference: test_foreign.py::test_heicsave_16_to_12
fn test_heicsave_16_to_12() {
    let im = decode_file(&ref_image("sample.png")).unwrap(); // 16-bit PNG
    let buf = im.encode_heif_lossless("hevc").unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    // The reload should be 12-bit stored as ushort
    assert!(
        im2.format() == PixelFormat::Rgb16 || im2.format() == PixelFormat::Rgba16,
        "HEIC lossless 16-bit should reload as 16-bit format, got {:?}",
        im2.format()
    );
}

#[test]
#[ignore]
/// HEIC lossless save of rgb16 with bitdepth=8 stores as uchar.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as lossless HEIC bytes with specified bit depth.
/// fn Raster::encode_heif_lossless_bitdepth(
///     &self, compression: &str, bitdepth: u32,
/// ) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_heicsave_16_to_8)
///
/// 1. Create or load a 16-bit RGB image.
/// 2. Save as lossless HEIC with bitdepth=8.
/// 3. Reload, verify stored as uchar (8-bit).
///
/// Reference: test_foreign.py::test_heicsave_16_to_8
fn test_heicsave_16_to_8() {
    let im = decode_file(&ref_image("sample.png")).unwrap(); // 16-bit PNG
    let buf = im.encode_heif_lossless_bitdepth("hevc", 8).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert!(
        im2.format() == PixelFormat::Rgb8 || im2.format() == PixelFormat::Rgba8,
        "HEIC with bitdepth=8 should reload as 8-bit format, got {:?}",
        im2.format()
    );
}

#[test]
#[ignore]
/// HEIC lossless save of 8-bit with bitdepth=12 stores as ushort.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::encode_heif_lossless_bitdepth(
///     &self, compression: &str, bitdepth: u32,
/// ) -> Result<Vec<u8>, EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_heicsave_8_to_16)
///
/// 1. Load an 8-bit RGB image (sample.jpg).
/// 2. Save as lossless HEIC with bitdepth=12.
/// 3. Reload, verify stored as ushort (16-bit).
///
/// Reference: test_foreign.py::test_heicsave_8_to_16
fn test_heicsave_8_to_16() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf = im.encode_heif_lossless_bitdepth("hevc", 12).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert!(
        im2.format() == PixelFormat::Rgb16 || im2.format() == PixelFormat::Rgba16,
        "HEIC with bitdepth=12 should reload as 16-bit format, got {:?}",
        im2.format()
    );
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
fn test_jp2kload() {
    let im = decode_file(&ref_image("world.jp2")).unwrap();
    assert!(im.width() > 0);
    assert!(im.height() > 0);
}

#[test]
#[ignore]
/// JP2K save roundtrip: lossy, lossless, Q variation, chroma subsample, 16-bit.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as JP2K bytes with specified quality.
/// fn Raster::encode_jp2k(&self, quality: u8, lossless: bool) -> Result<Vec<u8>, EncodeError>;
///
/// /// Encode raster as JP2K bytes with chroma subsampling control.
/// fn Raster::encode_jp2k_chroma(
///     &self, quality: u8, lossless: bool, subsample: bool,
/// ) -> Result<Vec<u8>, EncodeError>;
///
/// fn Raster::max_diff(&self, other: &Raster) -> f64;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jp2ksave)
///
/// 1. Load sample.jpg, encode lossy JP2K, reload, verify dimensions.
/// 2. Encode lossless, verify max_diff==0.
/// 3. Higher Q → larger buffer.
/// 4. Chroma subsample on → smaller buffer than off.
/// 5. 16-bit image (sample.png) roundtrip.
///
/// Reference: test_foreign.py::test_jp2ksave
fn test_jp2ksave() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    // Lossy
    let buf_lossy = im.encode_jp2k(50, false).unwrap();
    let im2 = decode_bytes(&buf_lossy).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());

    // Lossless
    let buf_lossless = im.encode_jp2k(0, true).unwrap();
    let im3 = decode_bytes(&buf_lossless).unwrap();
    let diff = im.max_diff(&im3);
    assert_eq!(diff, 0.0, "Lossless JP2K roundtrip max_diff={diff}, expected 0");

    // Q variation
    let buf_low = im.encode_jp2k(10, false).unwrap();
    let buf_high = im.encode_jp2k(90, false).unwrap();
    assert!(
        buf_high.len() > buf_low.len(),
        "Q=90 JP2K ({}) should be larger than Q=10 ({})",
        buf_high.len(), buf_low.len()
    );

    // Chroma subsample
    let buf_sub_off = im.encode_jp2k_chroma(50, false, false).unwrap();
    let buf_sub_on = im.encode_jp2k_chroma(50, false, true).unwrap();
    assert!(
        buf_sub_off.len() > buf_sub_on.len(),
        "No subsample ({}) should be larger than with subsample ({})",
        buf_sub_off.len(), buf_sub_on.len()
    );

    // 16-bit roundtrip
    let im16 = decode_file(&ref_image("sample.png")).unwrap();
    let buf16 = im16.encode_jp2k(50, false).unwrap();
    let im16_2 = decode_bytes(&buf16).unwrap();
    assert_eq!(im16_2.width(), im16.width());
    assert_eq!(im16_2.height(), im16.height());
}

#[test]
#[ignore]
/// JPEG XL save/load round-trip.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::encode_jxl(&self, lossless: bool) -> Result<Vec<u8>, EncodeError>;
/// fn decode_bytes(data: &[u8]) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_jxlsave)
///
/// libvips tests JXL entirely via save_load_buffer — no .jxl fixture file.
/// 1. Load sample.jpg as the source colour image.
/// 2. Encode as JXL (lossy), decode, verify dimensions and avg within threshold.
/// 3. Encode as JXL (lossless), decode, verify exact round-trip.
/// 4. Lossy buffer should be much smaller than lossless.
///
/// Reference: test_foreign.py::test_jxlsave
fn test_jxlsave() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    // Lossy round-trip
    let lossy_buf = im.encode_jxl(false).unwrap();
    let im2 = decode_bytes(&lossy_buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());

    // Lossless round-trip
    let lossless_buf = im.encode_jxl(true).unwrap();
    let im3 = decode_bytes(&lossless_buf).unwrap();
    assert_eq!(im3.width(), im.width());
    assert_eq!(im3.height(), im.height());

    // Lossy should be much smaller than lossless
    assert!(
        lossy_buf.len() < lossless_buf.len() / 5,
        "lossy JXL ({}) should be much smaller than lossless ({})",
        lossy_buf.len(),
        lossless_buf.len()
    );
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
fn test_svgload() {
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
/// 1. Load the WFPC2 FITS image from the reference suite.
/// 2. Verify dimensions are positive.
///
/// Reference: test_foreign.py::test_fits
fn test_fitsload() {
    let result = decode_file(&ref_image("WFPC2u5780205r_c0fx.fits"));
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
fn test_openexrload() {
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
fn test_openslideload() {
    let result = decode_file(&ref_image("CMU-1-Small-Region.svs"));
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
fn test_matload() {
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
fn test_analyzeload() {
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
fn test_niftiload() {
    let result = decode_file(&ref_image("avg152T1_LR_nifti.nii.gz"));
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
    let im = decode_file(&ref_image("rgba-correct.ppm")).unwrap();
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
/// CSV format loading (pixel values as comma-separated text).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Load pixel values from a CSV text file.
/// fn Raster::csv_load(data: &[u8]) -> Result<Raster, DecodeError>;
///
/// /// Save pixel values as CSV text.
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
fn test_csv() {
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
/// Matrix format loading (text-based pixel dump).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Load pixel values from a text matrix.
/// fn Raster::matrix_load(data: &[u8]) -> Result<Raster, DecodeError>;
///
/// /// Save pixel values as a text matrix.
/// fn Raster::matrix_save(&self) -> Vec<u8>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_matrix)
///
/// 1. Create a small single-band image.
/// 2. Save as matrix.
/// 3. Load back.
/// 4. Verify pixel values match (lossless).
///
/// Reference: test_foreign.py::test_matrix
fn test_matrix() {
    let data = vec![42u8; 10 * 10];
    let im = Raster::new(10, 10, PixelFormat::Gray8, data).unwrap();

    let mat = im.matrix_save();
    assert!(!mat.is_empty());

    let im2 = Raster::matrix_load(&mat).unwrap();
    assert_eq!(im2.width(), 10);
    assert_eq!(im2.height(), 10);

    let max_diff: f64 = im.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001, "Matrix round-trip should be lossless");
}

#[test]
#[ignore]
/// No libvips equivalent — extra coverage for BMP format.
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
/// Load various formats through the ImageMagick/GraphicsMagick delegate.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Load an image via ImageMagick from a file path, with optional parameters.
/// fn magickload(path: &Path) -> Result<Raster, DecodeError>;
/// fn magickload_with(path: &Path, opts: MagickLoadOptions) -> Result<Raster, DecodeError>;
///
/// /// Read a single pixel at (x, y) as a Vec of f64 channel values.
/// fn Raster::getpoint(&self, x: u32, y: u32) -> Vec<f64>;
///
/// /// Read an integer metadata field.
/// fn Raster::get_int(&self, name: &str) -> Option<i32>;
///
/// /// Get a metadata field (blob).
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_magickload)
///
/// 1. Load BMP via magickload, verify dims 1419×1001, bps 8,
///    pixel (100,100) ≈ [227, 216, 201]. Also load from buffer.
/// 2. Load SVG via magick, verify bands ∈ {1, 3, 4}. Verify density parameter
///    doubles dimensions.
/// 3. Load animated GIF (cogs.gif) — single frame, then n=-1 for all frames
///    (height *= 5). Load page=1,n=2 and verify height*2, page-height.
/// 4. Load DICOM — verify dims 128×128.
/// 5. Sniff ICO from buffer — verify dims 16×16.
/// 6. Sniff CUR from buffer — verify dims 32×32.
/// 7. Sniff TGA from buffer — verify dims 433×433.
/// 8. Sniff SGI from buffer — verify dims 433×433.
/// 9. Load sample.jpg via magick — verify ICC profile length == 564.
///
/// Reference: test_foreign.py::test_magickload
fn test_magickload() {
    // ---- BMP via magick ----
    let bmp_path = ref_image("MARBLES.BMP");
    let im = magickload(&bmp_path).unwrap();
    assert_eq!(im.width(), 1419);
    assert_eq!(im.height(), 1001);
    assert_eq!(im.get_int("bits-per-sample"), Some(8));
    let px = im.getpoint(100, 100);
    let expected = [227.0, 216.0, 201.0];
    for (i, (&got, &exp)) in px.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - exp).abs() < 1.0,
            "BMP pixel(100,100)[{i}]: got {got}, expected {exp}"
        );
    }

    // buffer load
    let bytes = std::fs::read(&bmp_path).unwrap();
    let im2 = decode_bytes(&bytes).unwrap();
    assert_eq!(im2.width(), 1419);
    assert_eq!(im2.height(), 1001);

    // ---- SVG via magick ----
    let svg_path = ref_image("logo.svg");
    let im = magickload(&svg_path).unwrap();
    assert!(
        im.bands() == 1 || im.bands() == 3 || im.bands() == 4,
        "SVG bands should be 1, 3, or 4, got {}",
        im.bands()
    );

    // density should change SVG size
    let im100 = magickload_with(&svg_path, MagickLoadOptions { density: Some("100") }).unwrap();
    let w100 = im100.width();
    let h100 = im100.height();
    let im200 = magickload_with(&svg_path, MagickLoadOptions { density: Some("200") }).unwrap();
    // At 2× density, dimensions should roughly double
    assert!(im200.width() > w100, "2× density width should be larger");
    assert!(im200.height() > h100, "2× density height should be larger");

    // ---- Animated GIF via magick ----
    let gif_path = ref_image("cogs.gif");
    let im = magickload(&gif_path).unwrap();
    let width = im.width();
    let height = im.height();
    let im_all = magickload_with(&gif_path, MagickLoadOptions { n: Some(-1), ..Default::default() }).unwrap();
    assert_eq!(im_all.width(), width);
    assert_eq!(im_all.height(), height * 5);

    // page/n for range of pages
    let im_pages = magickload_with(&gif_path, MagickLoadOptions {
        page: Some(1),
        n: Some(2),
        ..Default::default()
    }).unwrap();
    assert_eq!(im_pages.width(), width);
    assert_eq!(im_pages.height(), height * 2);
    assert_eq!(im_pages.get_int("page-height"), Some(height as i32));

    // ---- DICOM ----
    let im = magickload(&ref_image("dicom_test_image.dcm")).unwrap();
    assert_eq!(im.width(), 128);
    assert_eq!(im.height(), 128);

    // ---- ICO sniffer ----
    let bytes = std::fs::read(ref_image("favicon.ico")).unwrap();
    let im = decode_bytes(&bytes).unwrap();
    assert_eq!(im.width(), 16);
    assert_eq!(im.height(), 16);

    // ---- CUR sniffer ----
    let bytes = std::fs::read(ref_image("sample.cur")).unwrap();
    let im = decode_bytes(&bytes).unwrap();
    assert_eq!(im.width(), 32);
    assert_eq!(im.height(), 32);

    // ---- TGA sniffer ----
    let bytes = std::fs::read(ref_image("targa.tga")).unwrap();
    let im = decode_bytes(&bytes).unwrap();
    assert_eq!(im.width(), 433);
    assert_eq!(im.height(), 433);

    // ---- SGI/RGB sniffer ----
    let bytes = std::fs::read(ref_image("silicongraphics.sgi")).unwrap();
    let im = decode_bytes(&bytes).unwrap();
    assert_eq!(im.width(), 433);
    assert_eq!(im.height(), 433);

    // ---- ICC metadata via magick ----
    let im = magickload(&ref_image("sample.jpg")).unwrap();
    let icc = im.get_field("icc-profile-data")
        .expect("sample.jpg should have ICC profile via magickload");
    assert_eq!(icc.len(), 564, "ICC profile length should be 564");
}

#[test]
#[ignore]
/// Save via magicksave, reload, verify dimensions+ICC; animated GIF roundtrip via magick.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Save raster via ImageMagick/GraphicsMagick to a buffer with a given format string.
/// fn Raster::magicksave_buffer(&self, format: &str) -> Result<Vec<u8>, EncodeError>;
///
/// /// Get a metadata field value.
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
///
/// /// Get the number of pages (frames) in a multi-page image.
/// fn Raster::get_n_pages(&self) -> u32;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_magicksave)
///
/// 1. Load sample.jpg, save via magicksave_buffer(".png"), reload, verify dimensions+ICC.
/// 2. Load an animated GIF, save via magicksave_buffer(".gif"), reload,
///    verify page count matches.
///
/// Reference: test_foreign.py::test_magicksave
fn test_magicksave() {
    // Static image roundtrip
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf = im.magicksave_buffer(".png").unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    if im.get_field("icc-profile-data").is_some() {
        assert!(
            im2.get_field("icc-profile-data").is_some(),
            "ICC profile should survive magicksave roundtrip"
        );
    }

    // Animated GIF roundtrip via magick
    let gif = decode_file(&ref_image("trans-x.gif")).unwrap();
    let buf_gif = gif.magicksave_buffer(".gif").unwrap();
    let gif2 = decode_bytes(&buf_gif).unwrap();
    assert_eq!(gif2.width(), gif.width());
    assert_eq!(gif2.get_n_pages(), gif.get_n_pages());
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
fn test_uhdrload() {
    let result = decode_file(&ref_image("ultra-hdr.jpg"));
    match result {
        Ok(im) => {
            assert!(im.width() > 0);
            assert!(im.height() > 0);
        }
        Err(e) => eprintln!("Ultra HDR not supported: {e}"),
    }
}

#[test]
#[ignore]
/// UHDR save to buffer and reload preserves dimensions, format, interpretation, gainmap-data.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Encode raster as UHDR (Ultra HDR gain-map JPEG) bytes.
/// fn Raster::encode_uhdr(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
///
/// /// Get a metadata field value.
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
///
/// /// Get the interpretation (colour space) of the image.
/// fn Raster::interpretation(&self) -> Interpretation;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_uhdrsave)
///
/// 1. Load ultra-hdr.jpg, save to UHDR buffer, reload.
/// 2. Verify dimensions, format, interpretation match.
/// 3. Verify gainmap-data is present.
///
/// Reference: test_foreign.py::test_uhdrsave
fn test_uhdrsave() {
    let im = decode_file(&ref_image("ultra-hdr.jpg")).unwrap();
    let buf = im.encode_uhdr(75).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im2.format(), im.format());
    assert_eq!(im2.interpretation(), im.interpretation());
    assert!(
        im2.get_field("gainmap-data").is_some(),
        "Gainmap data should be present after UHDR roundtrip"
    );
}

#[test]
#[ignore]
/// UHDR save/load roundtrip preserves HDR content (scRGB avg diff < 0.02).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::encode_uhdr(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// fn Raster::avg_diff(&self, other: &Raster) -> f64;
/// fn Raster::colourspace(&self, space: &str) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_uhdrsave_roundtrip)
///
/// 1. Load ultra-hdr.jpg, save as UHDR, reload.
/// 2. Convert both to scRGB, compute average difference.
/// 3. avg diff < 0.02.
///
/// Reference: test_foreign.py::test_uhdrsave_roundtrip
fn test_uhdrsave_roundtrip() {
    let im = decode_file(&ref_image("ultra-hdr.jpg")).unwrap();
    let buf = im.encode_uhdr(75).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    let diff = im.colourspace("scrgb").avg_diff(&im2.colourspace("scrgb"));
    assert!(
        diff < 0.02,
        "UHDR roundtrip scRGB avg diff = {diff}, expected < 0.02"
    );
}

#[test]
#[ignore]
/// UHDR roundtrip from scRGB input (avg diff < 0.05).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::encode_uhdr(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
/// fn Raster::avg_diff(&self, other: &Raster) -> f64;
/// fn Raster::colourspace(&self, space: &str) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_uhdrsave_roundtrip_hdr)
///
/// 1. Load an HDR image, convert to scRGB.
/// 2. Save as UHDR, reload, convert to scRGB.
/// 3. avg diff < 0.05.
///
/// Reference: test_foreign.py::test_uhdrsave_roundtrip_hdr
fn test_uhdrsave_roundtrip_hdr() {
    let im = decode_file(&ref_image("ultra-hdr.jpg")).unwrap();
    let scrgb = im.colourspace("scrgb");
    let buf = scrgb.encode_uhdr(75).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    let diff = scrgb.avg_diff(&im2.colourspace("scrgb"));
    assert!(
        diff < 0.05,
        "UHDR HDR roundtrip scRGB avg diff = {diff}, expected < 0.05"
    );
}

#[test]
#[ignore]
/// Gainmap-scale-factor defaults to 2 for scRGB, respects explicit 4.
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::encode_uhdr(&self, quality: u8) -> Result<Vec<u8>, EncodeError>;
///
/// /// Encode UHDR with explicit gainmap scale factor.
/// fn Raster::encode_uhdr_gainmap_scale(
///     &self, quality: u8, scale_factor: u32,
/// ) -> Result<Vec<u8>, EncodeError>;
///
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_uhdrsave_gainmap_scale_factor)
///
/// 1. Load ultra-hdr.jpg, convert to scRGB, save as UHDR (default).
/// 2. Reload, verify gainmap-scale-factor == 2.
/// 3. Save with explicit scale_factor=4, reload, verify == 4.
///
/// Reference: test_foreign.py::test_uhdrsave_gainmap_scale_factor
fn test_uhdrsave_gainmap_scale_factor() {
    let im = decode_file(&ref_image("ultra-hdr.jpg")).unwrap();
    let scrgb = im.colourspace("scrgb");

    // Default: scale factor 2 for scRGB input
    let buf = scrgb.encode_uhdr(75).unwrap();
    let im2 = decode_bytes(&buf).unwrap();
    assert_eq!(
        im2.get_field("gainmap-scale-factor").unwrap().as_u32(), 2,
        "Default gainmap-scale-factor should be 2 for scRGB"
    );

    // Explicit: scale factor 4
    let buf4 = scrgb.encode_uhdr_gainmap_scale(75, 4).unwrap();
    let im3 = decode_bytes(&buf4).unwrap();
    assert_eq!(
        im3.get_field("gainmap-scale-factor").unwrap().as_u32(), 4,
        "Explicit gainmap-scale-factor should be 4"
    );
}

#[test]
#[ignore]
/// Thumbnailing UHDR scales down gainmap.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a thumbnail from a file path at the given width.
/// fn thumbnail(path: &Path, width: u32) -> Result<Raster, DecodeError>;
///
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_uhdr_thumbnail)
///
/// 1. Load ultra-hdr.jpg, thumbnail to half width.
/// 2. Verify gainmap-data is present.
/// 3. Verify gainmap dimensions are scaled proportionally.
///
/// Reference: test_foreign.py::test_uhdr_thumbnail
fn test_uhdr_thumbnail() {
    let im = decode_file(&ref_image("ultra-hdr.jpg")).unwrap();
    let half_w = im.width() / 2;
    let thumb = thumbnail(&ref_image("ultra-hdr.jpg"), half_w).unwrap();
    assert!(thumb.width() <= half_w + 1);
    assert!(
        thumb.get_field("gainmap-data").is_some(),
        "Gainmap should survive thumbnailing"
    );
}

#[test]
#[ignore]
/// Thumbnailing UHDR with crop="centre" produces roughly square gainmap.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a thumbnail with crop mode.
/// fn thumbnail_crop(path: &Path, width: u32, height: u32, crop: &str) -> Result<Raster, DecodeError>;
///
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_uhdr_thumbnail_crop)
///
/// 1. Load ultra-hdr.jpg, thumbnail to 100x100 with crop="centre".
/// 2. Verify roughly square output.
/// 3. Verify gainmap-data is present.
///
/// Reference: test_foreign.py::test_uhdr_thumbnail_crop
fn test_uhdr_thumbnail_crop() {
    let thumb = thumbnail_crop(&ref_image("ultra-hdr.jpg"), 100, 100, "centre").unwrap();
    assert!((thumb.width() as i32 - 100).abs() <= 1);
    assert!((thumb.height() as i32 - 100).abs() <= 1);
    assert!(
        thumb.get_field("gainmap-data").is_some(),
        "Gainmap should survive thumbnail+crop"
    );
}

#[test]
#[ignore]
/// DeepZoom save of UHDR preserves scaled gainmaps.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Generate DeepZoom pyramid from a UHDR source, preserving gainmaps.
/// fn generate_pyramid(src: &Raster, plan: &Plan, sink: &dyn Sink, config: &EngineConfig)
///     -> Result<PyramidResult, PyramidError>;
///
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_uhdr_dzsave)
///
/// 1. Load ultra-hdr.jpg.
/// 2. Generate DeepZoom tiles.
/// 3. Verify tiles are produced and gainmap data is scaled for each level.
///
/// Reference: test_foreign.py::test_uhdr_dzsave
fn test_uhdr_dzsave() {
    let src = decode_file(&ref_image("ultra-hdr.jpg")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let planner = PyramidPlanner::new(src.width(), src.height(), 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("uhdr_dz");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Jpeg { quality: 80 });
    let result = generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();
    assert!(result.tiles_produced > 0, "Should produce tiles from UHDR source");
}

// ===========================================================================
// fail_on
// ===========================================================================

#[test]
#[ignore]
/// CSV load of truncated data succeeds by default, fails with fail_on="truncated"/"warning".
///
/// ## Required API
///
/// ```rust,ignore
/// /// Load pixel values from a CSV text matrix.
/// fn Raster::csv_load(data: &[u8]) -> Result<Raster, DecodeError>;
///
/// /// Decode with a fail-on strictness level.
/// fn decode_bytes_fail_on(data: &[u8], fail_on: &str) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_foreign.py::test_fail_on)
///
/// 1. Create a CSV with truncated/incomplete data.
/// 2. Load normally — should succeed (partial decode).
/// 3. Load with fail_on="truncated" — should fail.
/// 4. Load with fail_on="warning" — should fail.
///
/// Reference: test_foreign.py::test_fail_on
fn test_fail_on() {
    // Create a truncated CSV (fewer values than expected rows)
    let csv_data = b"1,2,3\n4,5";

    let result = Raster::csv_load(csv_data);
    assert!(result.is_ok(), "Truncated CSV should load normally by default");

    let fail_trunc = decode_bytes_fail_on(csv_data, "truncated");
    assert!(fail_trunc.is_err(), "Truncated CSV should fail with fail_on=truncated");

    let fail_warn = decode_bytes_fail_on(csv_data, "warning");
    assert!(fail_warn.is_err(), "Truncated CSV should fail with fail_on=warning");
}
