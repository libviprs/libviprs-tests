#![cfg(feature = "pdfium")]

use libviprs::pdf::render_page_pdfium;
use pdfium_render::prelude::*;
use std::path::Path;

const FIXTURE_PDF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint.pdf"
);

/// Verify that the PDFium shared library can be found on this system.
#[test]
fn pdfium_library_loads() {
    let pdfium = Pdfium::default();
    // If we get here, the library was found and loaded successfully.
    // Load a trivial document to confirm it's functional.
    let doc = pdfium.load_pdf_from_file(FIXTURE_PDF, None);
    assert!(
        doc.is_ok(),
        "PDFium loaded but failed to open fixture PDF: {:?}",
        doc.err()
    );
}

/// Verify we can read page count and dimensions via pdfium.
#[test]
fn pdfium_reads_page_info() {
    let pdfium = Pdfium::default();
    let doc = pdfium
        .load_pdf_from_file(FIXTURE_PDF, None)
        .expect("failed to load fixture PDF");
    let pages = doc.pages();

    assert!(pages.len() >= 1, "Expected at least 1 page");

    let page = pages.get(0).expect("failed to get first page");
    let width = page.width().value;
    let height = page.height().value;

    assert!(width > 0.0, "Page width should be > 0, got {width}");
    assert!(height > 0.0, "Page height should be > 0, got {height}");
}

/// Verify pdfium can render a page to a bitmap at a reasonable DPI.
#[test]
fn pdfium_renders_page_to_bitmap() {
    let pdfium = Pdfium::default();
    let doc = pdfium
        .load_pdf_from_file(FIXTURE_PDF, None)
        .expect("failed to load fixture PDF");
    let page = doc.pages().get(0).expect("failed to get first page");

    let dpi: f32 = 150.0;
    let scale = dpi / 72.0;
    let width = (page.width().value * scale) as i32;
    let height = (page.height().value * scale) as i32;

    let config = PdfRenderConfig::new()
        .set_target_width(width)
        .set_maximum_height(height);

    let bitmap = page.render_with_config(&config);
    assert!(
        bitmap.is_ok(),
        "PDFium render failed: {:?}",
        bitmap.err()
    );

    let bmp = bitmap.unwrap();
    let img = bmp.as_image();
    let rgba = img.to_rgba8();

    assert!(rgba.width() > 0, "Rendered width is 0");
    assert!(rgba.height() > 0, "Rendered height is 0");
}

/// Verify that libviprs::render_page_pdfium produces a valid Raster.
#[test]
fn libviprs_render_page_pdfium() {
    let raster = render_page_pdfium(Path::new(FIXTURE_PDF), 1, 150)
        .expect("render_page_pdfium failed");

    assert!(raster.width() > 100, "Rendered raster too narrow: {}", raster.width());
    assert!(raster.height() > 100, "Rendered raster too short: {}", raster.height());
    assert_eq!(raster.format(), libviprs::PixelFormat::Rgba8);
}

/// Verify pdfium handles an invalid page number gracefully.
#[test]
fn libviprs_render_page_pdfium_invalid_page() {
    let result = render_page_pdfium(Path::new(FIXTURE_PDF), 999, 150);
    assert!(result.is_err(), "Expected error for out-of-range page");
}

/// Verify pdfium handles a nonexistent file gracefully.
#[test]
fn libviprs_render_page_pdfium_missing_file() {
    let result = render_page_pdfium(Path::new("/nonexistent.pdf"), 1, 150);
    assert!(result.is_err(), "Expected error for missing file");
}
