//! Test the CMYK FlateDecode PDF extraction path end-to-end.
//!
//! Each test loads a checked-in CMYK fixture under `tests/fixtures/`
//! (produced by `tests/gen_canonicals.rs`) and asserts that
//! `extract_page_image` returns RGB8 pixels matching the expected
//! CMYK → RGB conversion. The fixtures embed a single
//! `/DeviceCMYK` `/FlateDecode` image XObject per page; no
//! `/Contents` stream is needed because `extract_page_image` pulls
//! the embedded image directly rather than rendering the page.

use libviprs::{PixelFormat, extract_page_image, pdf_info};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn extract_cmyk_image_from_canonical_pdf() {
    let path = fixture("canonical_cmyk.pdf");

    let info = pdf_info(&path).expect("failed to parse canonical_cmyk.pdf");
    assert_eq!(info.page_count, 1);
    assert!(
        info.pages[0].has_images,
        "Page should have an image XObject"
    );

    let raster = extract_page_image(&path, 1).expect("failed to extract CMYK image");
    assert_eq!(raster.format(), PixelFormat::Rgb8);
    assert_eq!(raster.width(), 16);
    assert_eq!(raster.height(), 16);

    // First pixel of the gradient is C=0, M=0, Y=0, K=0 → RGB white.
    let data = raster.data();
    assert_eq!(data[0], 255, "R channel for zero CMYK should be 255");
    assert_eq!(data[1], 255, "G channel for zero CMYK should be 255");
    assert_eq!(data[2], 255, "B channel for zero CMYK should be 255");
}

#[test]
fn cmyk_full_black_converts_correctly() {
    // 1×1 fixture with K=255 → should produce black (0,0,0).
    let raster = extract_page_image(&fixture("canonical_cmyk_black_1x1.pdf"), 1)
        .expect("failed to extract CMYK image");
    let data = raster.data();
    assert_eq!(data[0], 0, "R should be 0 for full black");
    assert_eq!(data[1], 0, "G should be 0 for full black");
    assert_eq!(data[2], 0, "B should be 0 for full black");
}

#[test]
fn cmyk_pure_cyan_converts_correctly() {
    // 1×1 fixture with C=255, M=0, Y=0, K=0 → R=0, G=255, B=255.
    let raster = extract_page_image(&fixture("canonical_cmyk_cyan_1x1.pdf"), 1)
        .expect("failed to extract CMYK image");
    let data = raster.data();
    assert_eq!(data[0], 0, "R should be 0 for pure cyan");
    assert_eq!(data[1], 255, "G should be 255 for pure cyan");
    assert_eq!(data[2], 255, "B should be 255 for pure cyan");
}
