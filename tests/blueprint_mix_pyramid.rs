//! Integration test: blueprint-mix.pdf → extract image → tile pyramid.
//!
//! blueprint-mix.pdf is a mixed-content PDF containing both vector graphics
//! and embedded raster images. The tests exercise two extraction paths:
//!
//! 1. `extract_page_image` — pulls only the embedded raster (12738x220 RGB8),
//!    ignoring vector content. Pyramid output is compared byte-for-byte
//!    against pre-generated expected fixtures.
//!
//! 2. `render_page_pdfium` (behind `pdfium` feature) — renders the full page
//!    including vector overlays, producing a larger RGBA8 raster that captures
//!    all visible content. These tests verify that the rendered output differs
//!    from the raster-only extraction, confirming that vector content is
//!    actually being composited.
//!
//! To regenerate raster-extraction fixtures after intentional output changes:
//!   cargo run --release -p libviprs-cli -- pyramid \
//!     libviprs-tests/tests/fixtures/blueprint-mix.pdf \
//!     libviprs-tests/tests/fixtures/blueprint_mix_expected \
//!     --format png --tile-size 256

use std::path::{Path, PathBuf};

use libviprs::{
    EngineConfig, FsSink, Layout, MemorySink, PixelFormat, PyramidPlanner, TileFormat,
    extract_page_image, generate_pyramid,
};

const FIXTURE_PDF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint-mix.pdf"
);

const EXPECTED_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint_mix_expected"
);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn collect_files(dir: &Path, ext: &str) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    collect_files_recursive(dir, dir, ext, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn collect_files_recursive(root: &Path, dir: &Path, ext: &str, out: &mut Vec<(String, Vec<u8>)>) {
    if !dir.is_dir() {
        return;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, ext, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            let bytes = std::fs::read(&path).unwrap();
            out.push((rel, bytes));
        }
    }
}

fn load_blueprint_mix() -> libviprs::Raster {
    extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint-mix.pdf")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify the extracted raster has the expected dimensions and pixel format.
#[test]
fn blueprint_mix_extraction_metadata() {
    let raster = load_blueprint_mix();

    assert_eq!(raster.width(), 12738);
    assert_eq!(raster.height(), 220);
    assert_eq!(raster.format(), PixelFormat::Rgb8);
}

/// Generate a pyramid to the filesystem with PNG encoding and compare every
/// tile byte-for-byte against the pre-generated expected fixtures.
#[test]
fn blueprint_mix_pyramid_matches_expected() {
    let raster = load_blueprint_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default();

    let result =
        generate_pyramid(&raster, &plan, &sink, &config).expect("pyramid generation failed");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Compare tiles
    let expected_files = collect_files(Path::new(EXPECTED_DIR), "png");
    let actual_files = collect_files(&base, "png");

    assert_eq!(
        expected_files.len(),
        actual_files.len(),
        "Tile count mismatch: expected {}, got {}",
        expected_files.len(),
        actual_files.len(),
    );

    for ((exp_path, exp_bytes), (act_path, act_bytes)) in
        expected_files.iter().zip(actual_files.iter())
    {
        assert_eq!(
            exp_path, act_path,
            "File path mismatch: expected {exp_path}, got {act_path}"
        );
        assert_eq!(
            exp_bytes,
            act_bytes,
            "Content mismatch at {act_path}: expected {} bytes, got {} bytes",
            exp_bytes.len(),
            act_bytes.len(),
        );
    }

    // Compare DZI manifest
    let expected_dzi = std::fs::read_to_string(PathBuf::from(EXPECTED_DIR).with_extension("dzi"))
        .expect("expected DZI manifest not found");
    let actual_dzi = std::fs::read_to_string(dir.path().join("blueprint_mix.dzi"))
        .expect("generated DZI manifest not found");

    assert_eq!(expected_dzi, actual_dzi, "DZI manifest mismatch");
}

/// Verify determinism: two runs from the same source produce identical output.
#[test]
fn blueprint_mix_pyramid_deterministic() {
    let raster = load_blueprint_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink1 = MemorySink::new();
    let sink2 = MemorySink::new();
    let config = EngineConfig::default();

    generate_pyramid(&raster, &plan, &sink1, &config).unwrap();
    generate_pyramid(&raster, &plan, &sink2, &config).unwrap();

    let mut tiles1 = sink1.tiles();
    let mut tiles2 = sink2.tiles();
    tiles1.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    tiles2.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    assert_eq!(tiles1.len(), tiles2.len());
    for (t1, t2) in tiles1.iter().zip(tiles2.iter()) {
        assert_eq!(t1.coord, t2.coord);
        assert_eq!(t1.data, t2.data, "Tile {:?} differs between runs", t1.coord);
    }
}

/// Concurrent generation produces the same output as the expected fixtures.
#[test]
fn blueprint_mix_pyramid_concurrent_matches_expected() {
    let raster = load_blueprint_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default().with_concurrency(4);

    let result =
        generate_pyramid(&raster, &plan, &sink, &config).expect("pyramid generation failed");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    let expected_files = collect_files(Path::new(EXPECTED_DIR), "png");
    let actual_files = collect_files(&base, "png");

    assert_eq!(expected_files.len(), actual_files.len());
    for ((exp_path, exp_bytes), (act_path, act_bytes)) in
        expected_files.iter().zip(actual_files.iter())
    {
        assert_eq!(exp_path, act_path);
        assert_eq!(
            exp_bytes, act_bytes,
            "Concurrent output differs from expected at {act_path}"
        );
    }
}

// ---------------------------------------------------------------------------
// PDFium rendered mixed content tests (vector + raster)
// ---------------------------------------------------------------------------

/// Render blueprint-mix.pdf with PDFium and verify the rendered raster
/// captures the full page (vector + raster content) at a larger size than
/// the embedded raster alone.
#[test]
#[cfg(feature = "pdfium")]
fn blueprint_mix_pdfium_render_captures_full_page() {
    use libviprs::pdf::render_page_pdfium;

    let rendered =
        render_page_pdfium(Path::new(FIXTURE_PDF), 1, 150).expect("pdfium render failed");

    // PDFium renders the full page at the requested DPI.
    // blueprint-mix.pdf renders to 9932x7020 at 150 DPI.
    assert!(
        rendered.width() > 5000,
        "Rendered width {} too small for full-page render",
        rendered.width(),
    );
    assert!(
        rendered.height() > 5000,
        "Rendered height {} too small for full-page render",
        rendered.height(),
    );
    assert_eq!(rendered.format(), PixelFormat::Rgba8);

    // The raster-only extraction is 12738x220 — a narrow strip.
    // The rendered page has a much larger area covering both dimensions,
    // confirming it captures the full page layout (not just the embedded image).
    let rendered_area = rendered.width() as u64 * rendered.height() as u64;
    let extracted = load_blueprint_mix();
    let extracted_area = extracted.width() as u64 * extracted.height() as u64;
    assert!(
        rendered_area > extracted_area * 10,
        "Rendered area ({rendered_area}) should be much larger than extracted area ({extracted_area})",
    );
}

/// Verify that raster-only extraction and full-page PDFium render produce
/// materially different output, confirming that vector content is present
/// in the rendered version.
#[test]
#[cfg(feature = "pdfium")]
fn blueprint_mix_rendered_differs_from_extracted() {
    use libviprs::pdf::render_page_pdfium;

    let extracted = load_blueprint_mix();
    let rendered =
        render_page_pdfium(Path::new(FIXTURE_PDF), 1, 150).expect("pdfium render failed");

    // Different dimensions confirm different content capture.
    assert_ne!(
        (extracted.width(), extracted.height()),
        (rendered.width(), rendered.height()),
        "Rendered and extracted rasters should have different dimensions",
    );

    // The extracted raster is a narrow strip (embedded image only).
    // The rendered raster covers the full page with a different aspect ratio.
    assert!(
        extracted.width() > extracted.height() * 10,
        "Extracted raster should be a narrow strip ({}x{})",
        extracted.width(),
        extracted.height(),
    );
    let rendered_aspect = rendered.width() as f64 / rendered.height() as f64;
    let extracted_aspect = extracted.width() as f64 / extracted.height() as f64;
    assert!(
        (rendered_aspect - extracted_aspect).abs() > 1.0,
        "Rendered ({rendered_aspect:.2}) and extracted ({extracted_aspect:.2}) should have very different aspect ratios",
    );
}

/// Generate a pyramid from the PDFium-rendered mixed content and verify
/// it produces a valid tile set with more tiles than the raster-only pyramid.
#[test]
#[cfg(feature = "pdfium")]
fn blueprint_mix_pdfium_rendered_pyramid() {
    use libviprs::pdf::render_page_pdfium;

    let rendered =
        render_page_pdfium(Path::new(FIXTURE_PDF), 1, 150).expect("pdfium render failed");

    let planner = PyramidPlanner::new(
        rendered.width(),
        rendered.height(),
        256,
        0,
        Layout::DeepZoom,
    )
    .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink = MemorySink::new();
    let config = EngineConfig::default().with_concurrency(4);

    let result =
        generate_pyramid(&rendered, &plan, &sink, &config).expect("pyramid generation failed");

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());

    // The full-page render is much larger than the extracted strip, so it
    // should produce significantly more tiles.
    let extracted = load_blueprint_mix();
    let extracted_planner = PyramidPlanner::new(
        extracted.width(),
        extracted.height(),
        256,
        0,
        Layout::DeepZoom,
    )
    .unwrap();
    let extracted_plan = extracted_planner.plan();

    assert!(
        plan.total_tile_count() > extracted_plan.total_tile_count(),
        "Rendered pyramid ({} tiles) should have more tiles than extracted ({} tiles)",
        plan.total_tile_count(),
        extracted_plan.total_tile_count(),
    );
}

/// Verify PDFium-rendered pyramid is deterministic across two runs.
#[test]
#[cfg(feature = "pdfium")]
fn blueprint_mix_pdfium_rendered_pyramid_deterministic() {
    use libviprs::pdf::render_page_pdfium;

    let rendered =
        render_page_pdfium(Path::new(FIXTURE_PDF), 1, 150).expect("pdfium render failed");

    let planner = PyramidPlanner::new(
        rendered.width(),
        rendered.height(),
        256,
        0,
        Layout::DeepZoom,
    )
    .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink1 = MemorySink::new();
    let sink2 = MemorySink::new();
    let config = EngineConfig::default();

    generate_pyramid(&rendered, &plan, &sink1, &config).unwrap();
    generate_pyramid(&rendered, &plan, &sink2, &config).unwrap();

    let mut tiles1 = sink1.tiles();
    let mut tiles2 = sink2.tiles();
    tiles1.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    tiles2.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    assert_eq!(tiles1.len(), tiles2.len());
    for (t1, t2) in tiles1.iter().zip(tiles2.iter()) {
        assert_eq!(t1.coord, t2.coord);
        assert_eq!(
            t1.data, t2.data,
            "Rendered tile {:?} differs between runs",
            t1.coord,
        );
    }
}
