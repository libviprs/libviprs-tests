use std::path::Path;

use libviprs::{extract_page_image, pdf_info};

const FIXTURE_PDF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint.pdf"
);

#[test]
fn pdf_info_reads_page_count() {
    let info = pdf_info(Path::new(FIXTURE_PDF)).unwrap();
    assert!(info.page_count >= 1, "Expected at least 1 page");
}

#[test]
fn pdf_info_reads_page_dimensions() {
    let info = pdf_info(Path::new(FIXTURE_PDF)).unwrap();
    let page = &info.pages[0];
    assert!(page.width_pts > 0.0, "Page width should be > 0");
    assert!(page.height_pts > 0.0, "Page height should be > 0");
}

#[test]
fn pdf_info_detects_images() {
    let info = pdf_info(Path::new(FIXTURE_PDF)).unwrap();
    assert!(
        info.pages[0].has_images,
        "Scanned blueprint page should contain images"
    );
}

#[test]
fn extract_page_image_from_blueprint() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1).unwrap();
    assert!(raster.width() > 100, "Extracted image too small: {}x{}", raster.width(), raster.height());
    assert!(raster.height() > 100);
}

#[test]
fn extract_page_image_wrong_page() {
    let result = extract_page_image(Path::new(FIXTURE_PDF), 999);
    assert!(result.is_err());
}

#[test]
fn pdf_info_nonexistent_file() {
    let result = pdf_info(Path::new("/nonexistent.pdf"));
    assert!(result.is_err());
}
