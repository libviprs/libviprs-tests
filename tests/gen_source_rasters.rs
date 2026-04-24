//! Extracts/renders rasters from fixture PDFs and saves them as lossless PNGs.
//!
//! These PNG files serve as the common input for both libviprs tests and
//! `vips dzsave` fixture generation, ensuring both tools start from
//! identical pixel data.
//!
//! Two kinds of source rasters are generated:
//! - `extracted_*.png` — embedded raster images pulled from scanned PDFs
//! - `rendered_*.png` — full-page PDFium renders (vector + raster content)
//!
//! Run with:
//!   cargo test --test gen_source_rasters -- --ignored                    # extraction only
//!   cargo test --test gen_source_rasters --features pdfium -- --ignored  # extraction + rendering

use libviprs::{PixelFormat, extract_page_image};
use std::path::Path;

const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn save_raster_as_png(raster: &libviprs::Raster, path: &Path) {
    let color_type = match raster.format() {
        PixelFormat::Gray8 => image::ColorType::L8,
        PixelFormat::Rgb8 => image::ColorType::Rgb8,
        PixelFormat::Rgba8 => image::ColorType::Rgba8,
        other => panic!("unsupported pixel format: {:?}", other),
    };
    image::save_buffer(
        path,
        raster.data(),
        raster.width(),
        raster.height(),
        color_type,
    )
    .unwrap();
    eprintln!(
        "  wrote {}x{} {:?} → {}",
        raster.width(),
        raster.height(),
        raster.format(),
        path.display()
    );
}

#[test]
#[ignore]
fn extract_source_rasters() {
    eprintln!("\n=== Extracting source rasters from fixture PDFs ===\n");

    let portrait = extract_page_image(
        Path::new(FIXTURE_DIR)
            .join("blueprint-portrait.pdf")
            .as_path(),
        1,
    )
    .expect("failed to extract from blueprint-portrait.pdf");
    save_raster_as_png(
        &portrait,
        &Path::new(FIXTURE_DIR).join("extracted_blueprint_portrait.png"),
    );

    let mix = extract_page_image(
        Path::new(FIXTURE_DIR).join("blueprint-mix.pdf").as_path(),
        1,
    )
    .expect("failed to extract from blueprint-mix.pdf");
    save_raster_as_png(
        &mix,
        &Path::new(FIXTURE_DIR).join("extracted_blueprint_mix.png"),
    );

    eprintln!("\n=== Done. Now run: bash tools/gen_fixtures.sh ===\n");
}

/// Render fixture PDFs at 72 DPI via PDFium and save as lossless PNGs.
/// These rendered rasters are used as source input for vips dzsave to
/// generate fixtures for the pdfium pyramid comparison tests.
#[test]
#[ignore]
#[cfg(feature = "pdfium")]
fn render_source_rasters() {
    use libviprs::pdf::render_page_pdfium;

    eprintln!("\n=== Rendering source rasters from fixture PDFs via PDFium at 72 DPI ===\n");

    let mix = render_page_pdfium(
        Path::new(FIXTURE_DIR).join("blueprint-mix.pdf").as_path(),
        1,
        72,
    )
    .expect("failed to render blueprint-mix.pdf");
    save_raster_as_png(
        &mix,
        &Path::new(FIXTURE_DIR).join("rendered_blueprint_mix.png"),
    );

    let portrait = render_page_pdfium(
        Path::new(FIXTURE_DIR)
            .join("blueprint-portrait.pdf")
            .as_path(),
        1,
        72,
    )
    .expect("failed to render blueprint-portrait.pdf");
    save_raster_as_png(
        &portrait,
        &Path::new(FIXTURE_DIR).join("rendered_blueprint_portrait.png"),
    );

    eprintln!("\n=== Done. Now run: bash tools/gen_fixtures.sh ===\n");
}
