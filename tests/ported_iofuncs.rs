#![cfg(feature = "ported_tests")]

//! Ported I/O function tests from libvips `test_iofuncs.py`.
//!
//! Tests image construction from memory, metadata field access,
//! memory round-trips, and cache revalidation.

use std::path::Path;

use libviprs::source::decode_bytes;
use libviprs::{PixelFormat, Raster, decode_file};

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

// ===========================================================================
// test_new_from_image
// ===========================================================================

#[test]
#[ignore]
/// Create a constant image from an existing image, preserving geometry and metadata.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a new image with the same width, height, interpretation,
/// /// format, xres, yres, and offsets as `self`, filled with the given
/// /// constant value(s).
/// ///
/// /// A scalar produces a 1-band image; a vector of length N produces
/// /// an N-band image where every pixel equals that vector.
/// fn Raster::new_from_image(&self, value: &[f64]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_iofuncs.py::test_new_from_image)
///
/// 1. Load sample.jpg.
/// 2. `new_from_image(&[12.0])`:
///    - width and height match the source.
///    - interpretation, format, xres, yres, xoffset, yoffset match.
///    - bands == 1, avg == 12.
/// 3. `new_from_image(&[1.0, 2.0, 3.0])`:
///    - bands == 3, avg == 2 (mean of 1+2+3).
/// 4. `new_from_image(&[0.0, 0.0, 0.0, 0.0])`:
///    - bands == 4.
///
/// Reference: test_iofuncs.py::test_new_from_image
fn test_new_from_image() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    // Scalar value → 1 band, constant 12
    let im2 = im.new_from_image(&[12.0]);
    assert_eq!(im2.width(), im.width());
    assert_eq!(im2.height(), im.height());
    assert_eq!(im2.interpretation(), im.interpretation());
    assert_eq!(im2.format(), im.format());
    assert!((im2.xres() - im.xres()).abs() < 0.001);
    assert!((im2.yres() - im.yres()).abs() < 0.001);
    assert_eq!(im2.xoffset(), im.xoffset());
    assert_eq!(im2.yoffset(), im.yoffset());
    assert_eq!(im2.bands(), 1);
    assert!((im2.avg() - 12.0).abs() < 0.001);

    // 3-element vector → 3 bands, avg = (1+2+3)/3 = 2
    let im3 = im.new_from_image(&[1.0, 2.0, 3.0]);
    assert_eq!(im3.bands(), 3);
    assert!((im3.avg() - 2.0).abs() < 0.001);

    // 4-element vector → 4 bands
    let im4 = im.new_from_image(&[0.0, 0.0, 0.0, 0.0]);
    assert_eq!(im4.bands(), 4);
}

// ===========================================================================
// test_new_from_memory
// ===========================================================================

#[test]
#[ignore]
/// Construct an image from a raw memory buffer.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a new image from raw pixel data in memory.
/// fn Raster::new_from_memory(data: &[u8], width: u32, height: u32,
///                            bands: u32, format: &str) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_iofuncs.py::test_new_from_memory)
///
/// 1. Create a zeroed byte buffer of length 200.
/// 2. Construct a 20×10, 1-band uchar image from it.
/// 3. Assert width=20, height=10, format=uchar, bands=1, avg=0.
/// 4. Add 10 to every pixel, assert avg=10.
///
/// Reference: test_iofuncs.py::test_new_from_memory
fn test_new_from_memory() {
    let data = vec![0u8; 200];
    let im = Raster::new_from_memory(&data, 20, 10, 1, "uchar");

    assert_eq!(im.width(), 20);
    assert_eq!(im.height(), 10);
    assert_eq!(im.bands(), 1);
    assert!((im.avg() - 0.0).abs() < 0.001);

    let im2 = im.linear(1.0, 10.0);
    assert!((im2.avg() - 10.0).abs() < 0.001);
}

// ===========================================================================
// test_get_fields
// ===========================================================================

#[test]
#[ignore]
/// Read image metadata fields (e.g. EXIF, ICC, XMP).
///
/// ## Required API
///
/// ```rust,ignore
/// /// List all available metadata field names.
/// fn Raster::get_fields(&self) -> Vec<String>;
/// ```
///
/// ## Test logic (from libvips test_iofuncs.py::test_get_fields)
///
/// 1. Create a 10×10 black image.
/// 2. Get field names — should contain more than 10 fields.
/// 3. First field should be "width".
///
/// Reference: test_iofuncs.py::test_get_fields
fn test_get_fields() {
    let im = Raster::black(10, 10);
    let fields = im.get_fields();
    assert!(
        fields.len() > 10,
        "Should have more than 10 fields, got {}",
        fields.len()
    );
    assert_eq!(fields[0], "width");
}

// ===========================================================================
// test_write_to_memory
// ===========================================================================

#[test]
#[ignore]
/// Write image pixel data back to a memory buffer.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Write the image pixel data to a new byte buffer.
/// fn Raster::write_to_memory(&self) -> Vec<u8>;
/// ```
///
/// ## Test logic (from libvips test_iofuncs.py::test_write_to_memory)
///
/// 1. Create a zeroed byte buffer of length 200.
/// 2. Construct a 20×10, 1-band uchar image from it.
/// 3. Write back to memory via write_to_memory().
/// 4. Assert the output buffer equals the input buffer.
///
/// Reference: test_iofuncs.py::test_write_to_memory
fn test_write_to_memory() {
    let data = vec![0u8; 200];
    let im = Raster::new_from_memory(&data, 20, 10, 1, "uchar");
    let out = im.write_to_memory();

    assert_eq!(data, out, "write_to_memory should return the same bytes");
}

// ===========================================================================
// test_revalidate
// ===========================================================================

#[test]
#[ignore]
/// Invalidate and revalidate a cached image after the file changes on disk.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Invalidate any cached data, forcing re-read on next access.
/// fn Raster::invalidate(&mut self);
///
/// /// Load with revalidate flag to bypass the cache.
/// fn decode_file_with_options(path: &Path, revalidate: bool) -> Result<Raster, DecodeError>;
/// ```
///
/// ## Test logic (from libvips test_iofuncs.py::test_revalidate)
///
/// 1. Create a 10×10 black image, write to a temp .v file.
/// 2. Load it back, verify width=10.
/// 3. Create a 20×20 black image, overwrite the same file.
/// 4. Load again without revalidate — should get cached width=10.
/// 5. Load again with revalidate=true — should get new width=20.
/// 6. Load once more without revalidate — should see cached new width=20.
///
/// Reference: test_iofuncs.py::test_revalidate
fn test_revalidate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.v");

    let im1 = Raster::black(10, 10);
    im1.save(&path).unwrap();

    let load1 = decode_file(&path).unwrap();
    assert_eq!(load1.width(), 10);

    let im2 = Raster::black(20, 20);
    im2.save(&path).unwrap();

    // Cached — should still see old width
    let load2 = decode_file(&path).unwrap();
    assert_eq!(load2.width(), 10);

    // Revalidate — should see new width
    let load3 = decode_file_with_options(&path, true).unwrap();
    assert_eq!(load3.width(), 20);

    // Cache updated — should now see new width without revalidate
    let load4 = decode_file(&path).unwrap();
    assert_eq!(load4.width(), 20);
}
