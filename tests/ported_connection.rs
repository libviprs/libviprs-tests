#![cfg(feature = "ported_tests")]

//! Phase 12: Ported I/O & Connection tests from libvips `test_connection.py`.
//!
//! Auto tests verify in-memory Raster creation and pixel-data round-trips.
//! Ignored stubs document Source/Target abstraction and format-specific
//! connection features that are not yet exposed in libviprs.

use std::path::Path;

use libviprs::{decode_file, PixelFormat, Raster};
use libviprs::source::decode_bytes;

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

// ===========================================================================
// 12.1 In-memory construction
// ===========================================================================

#[test]
/// Create a Raster from raw bytes and verify dimensions.
/// Reference: test_connection.py — new_from_memory
fn test_new_from_memory() {
    let w: u32 = 64;
    let h: u32 = 48;
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let data = vec![0u8; w as usize * h as usize * bpp];

    let raster = Raster::new(w, h, PixelFormat::Rgb8, data).unwrap();
    assert_eq!(raster.width(), w);
    assert_eq!(raster.height(), h);
    assert_eq!(raster.format(), PixelFormat::Rgb8);
    assert_eq!(
        raster.data().len(),
        w as usize * h as usize * bpp,
        "Pixel buffer length should equal w * h * bytes_per_pixel"
    );
}

// ===========================================================================
// 12.2 Write / read-back
// ===========================================================================

#[test]
/// Create a Raster with known pixel values, read them back via `.data()`,
/// and verify the contents match.
/// Reference: test_connection.py — write_to_memory
fn test_write_to_memory() {
    let w: u32 = 32;
    let h: u32 = 32;
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];

    // Fill with a recognisable gradient pattern.
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x + y) % 256) as u8;
        }
    }

    let raster = Raster::new(w, h, PixelFormat::Rgb8, data.clone()).unwrap();

    // Read back and compare byte-for-byte.
    assert_eq!(raster.data(), data.as_slice(), "Round-tripped pixel data must match");
}

// ===========================================================================
// 12.3 Custom source
// ===========================================================================

#[test]
#[ignore]
/// Custom Source abstraction (user-supplied read callback).
///
/// ## Required API
///
/// ```rust,ignore
/// /// A user-supplied read source for streaming image decoding.
/// /// Wraps a `Read` implementation and allows the decoder to pull bytes on demand.
/// pub struct Source<R: std::io::Read> { inner: R }
///
/// impl<R: std::io::Read> Source<R> {
///     /// Create a new Source from any `Read` implementor.
///     pub fn new(reader: R) -> Self;
///
///     /// The filename associated with this source (None for memory sources).
///     pub fn filename(&self) -> Option<&str>;
/// }
///
/// /// Decode an image from a Source.
/// fn decode_source<R: std::io::Read>(source: &mut Source<R>) -> Result<Raster, SourceError>;
/// ```
///
/// ## Test logic (from libvips test_connection.py — custom_source)
///
/// 1. Open sample.jpg as a file.
/// 2. Wrap it in a Source.
/// 3. Decode via decode_source.
/// 4. Assert width and height match decode_file result (290×442).
///
/// Reference: test_connection.py::test_image_new_from_source_file
fn test_custom_source() {
    let file = std::fs::File::open(ref_image("sample.jpg")).unwrap();
    let mut source = Source::new(file);
    let raster = decode_source(&mut source).unwrap();

    assert_eq!(raster.width(), 290);
    assert_eq!(raster.height(), 442);
}

// ===========================================================================
// 12.4 Custom target
// ===========================================================================

#[test]
#[ignore]
/// Custom Target abstraction (user-supplied write callback).
///
/// ## Required API
///
/// ```rust,ignore
/// /// A user-supplied write target for streaming image encoding.
/// pub struct Target<W: std::io::Write> { inner: W }
///
/// impl<W: std::io::Write> Target<W> {
///     pub fn new(writer: W) -> Self;
///     pub fn filename(&self) -> Option<&str>;
/// }
///
/// /// Encode a Raster to a Target in the specified format.
/// fn encode_to_target<W: std::io::Write>(
///     raster: &Raster, target: &mut Target<W>, format: &str,
/// ) -> Result<(), EncodeError>;
/// ```
///
/// ## Test logic (from libvips test_connection.py — custom_target)
///
/// 1. Load sample.jpg.
/// 2. Create a Target wrapping a Vec<u8>.
/// 3. Encode to target as JPEG.
/// 4. Encode to buffer as JPEG.
/// 5. Assert the two byte sequences are identical.
///
/// Reference: test_connection.py::test_image_write_to_target_file
fn test_custom_target() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    let mut buf = Vec::new();
    let mut target = Target::new(&mut buf);
    encode_to_target(&im, &mut target, "jpeg").unwrap();

    let buf2 = im.encode_to_buffer("jpeg").unwrap();
    assert_eq!(buf, buf2, "Target and buffer encoding should produce identical bytes");
}

// ===========================================================================
// 12.5 Source from file
// ===========================================================================

#[test]
#[ignore]
/// Open a file via a Source object for streamed reading.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Create a Source from a file path.
/// fn Source::from_file(path: &Path) -> Result<Source<File>, io::Error>;
/// ```
///
/// ## Test logic (from libvips test_connection.py — source_from_file)
///
/// 1. Create Source from sample.jpg.
/// 2. Assert filename matches.
/// 3. Decode image, assert dimensions = 290×442.
///
/// Reference: test_connection.py::test_source_new_from_file
fn test_source_from_file() {
    let path = ref_image("sample.jpg");
    let mut source = Source::from_file(&path).unwrap();
    assert_eq!(source.filename(), Some(path.to_str().unwrap()));

    let raster = decode_source(&mut source).unwrap();
    assert_eq!(raster.width(), 290);
    assert_eq!(raster.height(), 442);
}

// ===========================================================================
// 12.6 Memory round-trip via Source/Target
// ===========================================================================

#[test]
#[ignore]
/// Full encode-then-decode round-trip through in-memory Source/Target.
///
/// ## Required API
///
/// Combines `encode_to_buffer` and `decode_bytes`.
///
/// ## Test logic (from libvips test_connection.py — memory_roundtrip)
///
/// 1. Load sample.jpg.
/// 2. Encode to JPEG buffer.
/// 3. Decode back from that buffer.
/// 4. Assert dimensions match (lossy compression means pixels won't be identical).
///
/// Reference: test_connection.py::test_image_new_from_source_memory
fn test_memory_roundtrip() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let buf = im.encode_to_buffer("jpeg").unwrap();
    let im2 = decode_bytes(&buf).unwrap();

    assert_eq!(im.width(), im2.width());
    assert_eq!(im.height(), im2.height());
    assert_eq!(im.format(), im2.format());
}

// ===========================================================================
// 12.7 Metadata / fields
// ===========================================================================

#[test]
#[ignore]
/// Read image metadata fields (e.g. EXIF, ICC, XMP).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Get a metadata field by name. Returns None if the field doesn't exist.
/// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
///
/// /// List all available metadata field names.
/// fn Raster::get_fields(&self) -> Vec<String>;
///
/// pub enum MetadataValue {
///     Int(i64),
///     Double(f64),
///     String(String),
///     Blob(Vec<u8>),
/// }
/// ```
///
/// ## Test logic (from libvips test_connection.py — get_fields)
///
/// 1. Load sample.jpg.
/// 2. Get field names — should contain standard fields like "width", "height", "bands".
/// 3. Get "icc-profile-data" — should be Some(Blob) for a JPEG with an embedded profile.
///
/// Reference: test_connection.py (metadata access patterns)
fn test_get_fields() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();

    let fields = im.get_fields();
    assert!(fields.contains(&"width".to_string()));
    assert!(fields.contains(&"height".to_string()));

    // JPEG may have an embedded ICC profile
    if let Some(MetadataValue::Blob(profile)) = im.get_field("icc-profile-data") {
        assert!(!profile.is_empty(), "ICC profile should not be empty");
    }
}

// ===========================================================================
// 12.8 Revalidate / caching
// ===========================================================================

#[test]
#[ignore]
/// Invalidate and revalidate a cached image region.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Invalidate any cached data, forcing re-read on next access.
/// fn Raster::invalidate(&mut self);
/// ```
///
/// ## Test logic
///
/// This tests an internal caching mechanism. Since libviprs doesn't currently
/// cache decoded data, this is a placeholder for when caching is added.
///
/// Reference: test_connection.py — revalidate
fn test_revalidate() {
    let im = decode_file(&ref_image("sample.jpg")).unwrap();
    let avg_before = im.avg();

    // After invalidation and re-read, avg should be identical
    let mut im2 = im.clone();
    im2.invalidate();
    let avg_after = im2.avg();
    assert!((avg_before - avg_after).abs() < 0.001);
}

// ===========================================================================
// 12.9 Format-specific connections
// ===========================================================================

#[test]
#[ignore]
/// Matrix format connection (text-based pixel dump).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Save a single-band image as a text matrix.
/// fn Raster::matrix_save(&self) -> Vec<u8>;
///
/// /// Load a text matrix as a single-band image.
/// fn Raster::matrix_load(data: &[u8]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_connection.py::test_connection_matrix)
///
/// 1. Extract mono band from sample.jpg.
/// 2. Save as matrix bytes.
/// 3. Load back.
/// 4. Assert max absolute difference = 0.
///
/// Reference: test_connection.py::test_connection_matrix
fn test_connection_matrix() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    let matrix_data = mono.matrix_save();
    let im2 = Raster::matrix_load(&matrix_data);

    let max_diff: f64 = mono.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001, "Matrix round-trip should be lossless");
}

#[test]
#[ignore]
/// SVG connection (vector rasterisation via Source/Target).
///
/// ## Required API
///
/// ```rust,ignore
/// /// Decode an SVG from bytes into a raster image.
/// fn decode_svg(data: &[u8]) -> Result<Raster, SourceError>;
/// ```
///
/// ## Test logic (from libvips test_connection.py::test_connection_svg)
///
/// 1. Create minimal SVG: `<svg xmlns="..." width="1" height="1" />`.
/// 2. Decode from bytes.
/// 3. Assert width=1, height=1.
///
/// Reference: test_connection.py::test_connection_svg
fn test_connection_svg() {
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\" />";
    let im = decode_svg(svg).unwrap();
    assert_eq!(im.width(), 1);
    assert_eq!(im.height(), 1);
}

#[test]
#[ignore]
/// CSV connection (pixel data as comma-separated values).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::csv_save(&self) -> Vec<u8>;
/// fn Raster::csv_load(data: &[u8]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_connection.py::test_connection_csv)
///
/// 1. Extract mono band, save as CSV, load back.
/// 2. Assert max diff = 0.
///
/// Reference: test_connection.py::test_connection_csv
fn test_connection_csv() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    let csv_data = mono.csv_save();
    let im2 = Raster::csv_load(&csv_data);

    let max_diff: f64 = mono.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001);
}

#[test]
#[ignore]
/// PPM connection (Netpbm portable pixmap).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::ppm_save(&self) -> Vec<u8>;
/// fn Raster::ppm_load(data: &[u8]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_connection.py::test_connection_ppm)
///
/// 1. Extract mono band, save as PPM, load back.
/// 2. Assert max diff = 0.
///
/// Reference: test_connection.py::test_connection_ppm
fn test_connection_ppm() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    let ppm_data = mono.ppm_save();
    let im2 = Raster::ppm_load(&ppm_data);

    let max_diff: f64 = mono.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001);
}

#[test]
#[ignore]
/// TIFF connection via Source/Target (streamed, not file-based).
///
/// ## Required API
///
/// ```rust,ignore
/// fn Raster::tiff_save(&self) -> Vec<u8>;
/// fn Raster::tiff_load(data: &[u8]) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_connection.py::test_connection_tiff)
///
/// 1. Extract mono band, save as TIFF bytes, load back.
/// 2. Assert max diff = 0.
///
/// Reference: test_connection.py::test_connection_tiff
fn test_connection_tiff() {
    let colour = decode_file(&ref_image("sample.jpg")).unwrap();
    let mono = colour.extract_band(1);

    let tiff_data = mono.tiff_save();
    let im2 = Raster::tiff_load(&tiff_data);

    let max_diff: f64 = mono.data().iter().zip(im2.data().iter())
        .map(|(&a, &b)| (a as f64 - b as f64).abs())
        .fold(0.0_f64, f64::max);
    assert!(max_diff < 0.001);
}
