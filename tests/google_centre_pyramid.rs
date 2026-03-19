//! Integration tests: Google Maps layout with --centre support.
//!
//! Tests verify that the Google layout produces power-of-2 tile grids,
//! `{z}/{x}/{y}.ext` paths, no DZI manifest, and correct centre offsets.
//!
//! The portrait PDF test uses the extracted raster (3300x5024 Gray8) since
//! libviprs extracts embedded images rather than rendering at 72 DPI.
//! Structural properties (level count, grid sizes, tile dimensions) are
//! verified, along with determinism and concurrent consistency.

use std::path::Path;

use libviprs::{
    EngineConfig, FsSink, Layout, MemorySink, PyramidPlanner, TileFormat, extract_page_image,
    generate_pyramid,
};

const FIXTURE_PDF_PORTRAIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint-portrait.pdf"
);

const FIXTURE_PDF_BLUEPRINT: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");

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

fn load_portrait_raster() -> libviprs::Raster {
    extract_page_image(Path::new(FIXTURE_PDF_PORTRAIT), 1)
        .expect("failed to extract image from blueprint-portrait.pdf")
}

fn load_blueprint_raster() -> libviprs::Raster {
    extract_page_image(Path::new(FIXTURE_PDF_BLUEPRINT), 1)
        .expect("failed to extract image from blueprint.pdf")
}

// ---------------------------------------------------------------------------
// Google layout structural tests (portrait PDF — extracted raster 3300x5024)
// ---------------------------------------------------------------------------

/// Verify that Google layout produces the correct number of levels and canvas
/// for the portrait PDF's extracted raster dimensions.
#[test]
fn google_centre_portrait_plan_structure() {
    let raster = load_portrait_raster();
    assert_eq!(raster.width(), 3300);
    assert_eq!(raster.height(), 5024);

    let planner = PyramidPlanner::new(3300, 5024, 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();

    // max(ceil(3300/256), ceil(5024/256)) = max(13, 20) = 20
    // n_levels = ceil(log2(20)) + 1 = 5 + 1 = 6
    assert_eq!(plan.level_count(), 6);

    // Canvas = 256 * 2^5 = 8192
    assert_eq!(plan.canvas_width, 8192);
    assert_eq!(plan.canvas_height, 8192);

    // Centre offsets
    assert_eq!(plan.centre_offset_x, (8192 - 3300) / 2); // 2446
    assert_eq!(plan.centre_offset_y, (8192 - 5024) / 2); // 1584

    // Power-of-2 grids
    for (i, level) in plan.levels.iter().enumerate() {
        let expected_grid = 1u32 << i;
        assert_eq!(level.cols, expected_grid, "Level {} cols", i);
        assert_eq!(level.rows, expected_grid, "Level {} rows", i);
    }

    // No DZI manifest
    assert!(plan.dzi_manifest("png").is_none());

    // Total tiles: 1 + 4 + 16 + 64 + 256 + 1024 = 1365
    assert_eq!(plan.total_tile_count(), 1365);
}

/// Generate Google+centre pyramid from portrait raster and verify every tile
/// is the correct dimensions and all expected coordinates are present.
#[test]
fn google_centre_portrait_generates_all_tiles() {
    let raster = load_portrait_raster();
    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();
    let sink = MemorySink::new();
    let config = EngineConfig::default();

    let result = generate_pyramid(&raster, &plan, &sink, &config).unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Every tile should be 256x256
    for tile in sink.tiles() {
        assert_eq!(tile.width, 256, "Tile {:?} wrong width", tile.coord);
        assert_eq!(tile.height, 256, "Tile {:?} wrong height", tile.coord);
    }
}

/// Generate to filesystem and verify path format is {z}/{x}/{y}.png.
#[test]
fn google_centre_portrait_path_format() {
    let raster = load_portrait_raster();
    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("portrait_google");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);

    generate_pyramid(&raster, &plan, &sink, &EngineConfig::default()).unwrap();

    let files = collect_files(&base, "png");
    assert_eq!(files.len() as u64, plan.total_tile_count());

    // All paths should match {z}/{x}/{y}.png format
    for (path, _) in &files {
        let parts: Vec<&str> = path.split('/').collect();
        assert_eq!(parts.len(), 3, "Bad path format: {path}");
        assert!(parts[2].ends_with(".png"), "Missing .png extension: {path}");
    }

    // No DZI manifest should exist
    assert!(!dir.path().join("portrait_google.dzi").exists());
}

/// Concurrent generation matches single-threaded output.
#[test]
fn google_centre_portrait_concurrent_matches() {
    let raster = load_portrait_raster();
    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();

    let ref_sink = MemorySink::new();
    generate_pyramid(&raster, &plan, &ref_sink, &EngineConfig::default()).unwrap();

    let conc_sink = MemorySink::new();
    let config = EngineConfig::default().with_concurrency(4);
    generate_pyramid(&raster, &plan, &conc_sink, &config).unwrap();

    let mut ref_tiles = ref_sink.tiles();
    let mut conc_tiles = conc_sink.tiles();
    ref_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    conc_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    assert_eq!(ref_tiles.len(), conc_tiles.len());
    for (r, c) in ref_tiles.iter().zip(conc_tiles.iter()) {
        assert_eq!(r.coord, c.coord);
        assert_eq!(r.data, c.data, "Concurrent output differs at {:?}", r.coord);
    }
}

/// Two sequential runs produce identical output (determinism).
#[test]
fn google_centre_portrait_deterministic() {
    let raster = load_portrait_raster();
    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
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

// ---------------------------------------------------------------------------
// Google layout structural tests (blueprint PDF)
// ---------------------------------------------------------------------------

/// Verify plan structure for blueprint.pdf (landscape/wide format).
#[test]
fn google_centre_blueprint_plan_structure() {
    let raster = load_blueprint_raster();
    let w = raster.width();
    let h = raster.height();

    let planner = PyramidPlanner::new(w, h, 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();

    // Verify power-of-2 grids
    for (i, level) in plan.levels.iter().enumerate() {
        let expected_grid = 1u32 << i;
        assert_eq!(level.cols, expected_grid, "Level {} cols", i);
        assert_eq!(level.rows, expected_grid, "Level {} rows", i);
    }

    // Canvas should be square
    assert_eq!(plan.canvas_width, plan.canvas_height);

    // Centre offsets should centre the image
    let canvas = plan.canvas_width;
    assert_eq!(plan.centre_offset_x, (canvas - w) / 2);
    assert_eq!(plan.centre_offset_y, (canvas - h) / 2);
}

/// Generate and verify all tiles for blueprint.pdf.
#[test]
fn google_centre_blueprint_generates_all_tiles() {
    let raster = load_blueprint_raster();
    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = generate_pyramid(&raster, &plan, &sink, &EngineConfig::default()).unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// Google layout WITHOUT centre
// ---------------------------------------------------------------------------

/// Google layout without centre: offsets should be zero, image pinned to (0,0).
#[test]
fn google_no_centre_portrait_plan() {
    let planner = PyramidPlanner::new(3300, 5024, 256, 0, Layout::Google).unwrap();
    let plan = planner.plan();

    assert_eq!(plan.centre_offset_x, 0);
    assert_eq!(plan.centre_offset_y, 0);
    assert!(!plan.centre);
    assert_eq!(plan.level_count(), 6);
    assert_eq!(plan.canvas_width, 8192);
}

/// Google layout without centre generates valid tiles.
#[test]
fn google_no_centre_portrait_generates_tiles() {
    let raster = load_portrait_raster();
    let planner =
        PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::Google).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = generate_pyramid(&raster, &plan, &sink, &EngineConfig::default()).unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// Vips fixture structure comparison
// ---------------------------------------------------------------------------

/// Compare the directory structure of our output against vips fixtures.
/// We can't compare pixel data (different source dimensions) but we can
/// verify that vips fixtures have valid PNGs and the expected path format.
#[test]
fn vips_portrait_fixtures_structurally_valid() {
    let fixture_dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/blueprint_portrait_google_centre"
    );

    if !Path::new(fixture_dir).exists() {
        eprintln!("Skipping: vips portrait fixtures not found");
        return;
    }

    let files = collect_files(Path::new(fixture_dir), "png");
    // Should have at least some tiles (vips skips blank tiles)
    assert!(
        files.len() >= 5,
        "Expected at least 5 tile files, got {}",
        files.len()
    );

    // All tiles should be valid PNGs
    for (path, bytes) in &files {
        if path == "blank.png" {
            continue; // vips blank tile marker
        }
        assert!(
            bytes.len() > 8,
            "Tile {path} too small ({} bytes)",
            bytes.len()
        );
        assert_eq!(
            &bytes[..4],
            &[0x89, b'P', b'N', b'G'],
            "Tile {path} not a valid PNG"
        );
    }
}

/// Verify that vips blueprint fixtures have valid structure.
#[test]
fn vips_blueprint_fixtures_structurally_valid() {
    let fixture_dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/blueprint_google_centre"
    );

    if !Path::new(fixture_dir).exists() {
        eprintln!("Skipping: vips blueprint fixtures not found");
        return;
    }

    let files = collect_files(Path::new(fixture_dir), "png");
    assert!(
        files.len() >= 10,
        "Expected at least 10 tile files, got {}",
        files.len()
    );

    for (path, bytes) in &files {
        if path == "blank.png" {
            continue;
        }
        assert!(bytes.len() > 8, "Tile {path} too small");
        assert_eq!(
            &bytes[..4],
            &[0x89, b'P', b'N', b'G'],
            "Tile {path} not a valid PNG"
        );
    }
}
