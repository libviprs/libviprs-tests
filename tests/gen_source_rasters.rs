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
//! A third kind, for the raster-into-PVec suites (libviprs-tests#305), lives under
//! `tests/fixtures/pvec-raster/` and is made by `tests/common/pvec_raster.rs`:
//! `write_pvec_raster_fixtures` (ignored) writes the files and `SHA256SUMS`, and
//! `pvec_raster_fixtures_regenerate_to_their_digests` (always on) regenerates every one in memory
//! and compares it with the committed bytes and with `SHA256SUMS`, which is what makes "they can be
//! regenerated" a checked claim.
//!
//! Run with:
//!   cargo test --test gen_source_rasters -- --ignored                    # extraction only
//!   cargo test --test gen_source_rasters --features pdfium -- --ignored  # extraction + rendering

use libviprs::{PixelFormat, extract_page_image};
use std::path::Path;

#[path = "common/pvec_raster.rs"]
mod pvec_raster;

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

/// Write the PVec raster fixtures and their `SHA256SUMS`.
///
///   cargo test --test gen_source_rasters -- --ignored write_pvec_raster_fixtures
#[test]
#[ignore]
fn write_pvec_raster_fixtures() {
    let dir = pvec_raster::dir();
    std::fs::create_dir_all(&dir).unwrap();
    let files = pvec_raster::all();
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
        eprintln!("  wrote {} ({} bytes)", name, bytes.len());
    }
    std::fs::write(dir.join("SHA256SUMS"), pvec_raster::sums(&files)).unwrap();
}

#[test]
fn pvec_raster_fixtures_regenerate_to_their_digests() {
    let dir = pvec_raster::dir();
    let files = pvec_raster::all();
    let committed = std::fs::read_to_string(dir.join("SHA256SUMS")).expect("SHA256SUMS");
    assert_eq!(
        committed,
        pvec_raster::sums(&files),
        "SHA256SUMS differs from what gen_source_rasters regenerates; run \
         `cargo test --test gen_source_rasters -- --ignored write_pvec_raster_fixtures`"
    );
    for (name, bytes) in &files {
        let on_disk = std::fs::read(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            on_disk == *bytes,
            "{name}: the committed file is not what the generator makes"
        );
    }
}

/// The control the issue asks for: one flipped byte in a fixture fails its digest check.
#[test]
fn a_fixture_with_one_flipped_byte_fails_its_sha256sums_check() {
    let dir = pvec_raster::dir();
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).expect("SHA256SUMS");
    let mut checked = 0;
    for line in sums.lines() {
        let (want, name) = line.split_once("  ").expect("sha256sum format");
        let mut bytes = std::fs::read(dir.join(name)).unwrap();
        assert_eq!(
            pvec_raster::sha256_hex(&bytes),
            want,
            "{name} is as committed"
        );
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0x01;
        assert_ne!(
            pvec_raster::sha256_hex(&bytes),
            want,
            "{name}: a flipped byte passes"
        );
        checked += 1;
    }
    assert_eq!(checked, pvec_raster::all().len());
}
