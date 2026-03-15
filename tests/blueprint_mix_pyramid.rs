//! Integration test: blueprint-mix.pdf → extract image → tile pyramid.
//!
//! Generates a pyramid from the multi-content blueprint PDF and compares
//! every output tile byte-for-byte against pre-generated expected output
//! in `tests/fixtures/blueprint_mix_expected/`.
//!
//! To regenerate expected fixtures after intentional output changes, run:
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

    let planner =
        PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
            .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default();

    let result = generate_pyramid(&raster, &plan, &sink, &config)
        .expect("pyramid generation failed");

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
            exp_bytes, act_bytes,
            "Content mismatch at {act_path}: expected {} bytes, got {} bytes",
            exp_bytes.len(),
            act_bytes.len(),
        );
    }

    // Compare DZI manifest
    let expected_dzi = std::fs::read_to_string(
        PathBuf::from(EXPECTED_DIR).with_extension("dzi"),
    )
    .expect("expected DZI manifest not found");
    let actual_dzi = std::fs::read_to_string(
        dir.path().join("blueprint_mix.dzi"),
    )
    .expect("generated DZI manifest not found");

    assert_eq!(expected_dzi, actual_dzi, "DZI manifest mismatch");
}

/// Verify determinism: two runs from the same source produce identical output.
#[test]
fn blueprint_mix_pyramid_deterministic() {
    let raster = load_blueprint_mix();

    let planner =
        PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
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

    let planner =
        PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
            .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default().with_concurrency(4);

    let result = generate_pyramid(&raster, &plan, &sink, &config)
        .expect("pyramid generation failed");

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
