//! Integration tests for `BlankTileStrategy` API.
//!
//! All tests load input images from `tests/fixtures/blank_tile_strategy/inputs/`
//! and compare generated output byte-for-byte against pre-existing expected
//! output in `tests/fixtures/blank_tile_strategy/expected/`.
//!
//! To regenerate fixtures after intentional output changes, run:
//!   cargo test --test gen_blank_tile_fixtures -- --ignored generate_fixtures

use libviprs::{
    BlankTileStrategy, EngineConfig, FsSink, Layout, PixelFormat, PyramidPlanner, Raster,
    TileFormat, BLANK_TILE_MARKER, generate_pyramid, is_blank_tile,
};
use std::path::{Path, PathBuf};

const FIXTURE_BASE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blank_tile_strategy");
const IMG_SIZE: u32 = 128;
const TILE_SIZE: u32 = 64;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn input_path(name: &str) -> PathBuf {
    Path::new(FIXTURE_BASE).join("inputs").join(name)
}

fn expected_dir(name: &str) -> PathBuf {
    Path::new(FIXTURE_BASE).join("expected").join(name)
}

fn load_input_png(name: &str) -> Raster {
    let path = input_path(name);
    let img = image::open(&path)
        .unwrap_or_else(|e| panic!("failed to load input fixture {}: {e}", path.display()));
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    Raster::new(w, h, PixelFormat::Rgb8, rgb.into_raw()).unwrap()
}

/// Collect all files under `dir` with the given extension, returning
/// (relative path, file bytes) pairs sorted by path.
fn collect_files(dir: &Path, ext: &str) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    collect_files_recursive(dir, dir, ext, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn collect_files_recursive(
    root: &Path,
    dir: &Path,
    ext: &str,
    out: &mut Vec<(String, Vec<u8>)>,
) {
    if !dir.is_dir() {
        return;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, ext, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().to_string();
            let bytes = std::fs::read(&path).unwrap();
            out.push((rel, bytes));
        }
    }
}

/// Run pyramid generation and compare every output tile byte-for-byte against
/// the expected fixture directory.
fn run_and_compare(input_fixture: &str, strategy: BlankTileStrategy, expected_name: &str) {
    let src = load_input_png(input_fixture);
    let planner = PyramidPlanner::new(IMG_SIZE, IMG_SIZE, TILE_SIZE, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("output");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);
    let config = EngineConfig::default().with_blank_tile_strategy(strategy);

    generate_pyramid(&src, &plan, &sink, &config).unwrap();

    // Compare output against expected fixtures
    let expected_base = expected_dir(expected_name);
    let expected_files = collect_files(&expected_base, "raw");
    let actual_files = collect_files(&base, "raw");

    assert_eq!(
        expected_files.len(),
        actual_files.len(),
        "File count mismatch for {expected_name}: expected {}, got {}",
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
            "Content mismatch at {act_path} in {expected_name}: \
             expected {} bytes, got {} bytes",
            exp_bytes.len(),
            act_bytes.len(),
        );
    }
}

// ---------------------------------------------------------------------------
// Emit strategy tests
// ---------------------------------------------------------------------------

/**
 * Tests that Emit strategy on a solid white image produces output identical
 * to the pre-generated expected fixtures.
 * Works by loading solid_white_128x128.png, running generate_pyramid with
 * BlankTileStrategy::Emit, and byte-comparing every output tile against the
 * corresponding file in expected/emit_solid_white/.
 * Input: solid_white_128x128.png fixture, tile_size=64 -> Output: matches emit_solid_white fixtures.
 */
#[test]
fn emit_solid_white_matches_expected() {
    run_and_compare(
        "solid_white_128x128.png",
        BlankTileStrategy::Emit,
        "emit_solid_white",
    );
}

/**
 * Tests that Emit strategy on a gradient image produces output identical
 * to the pre-generated expected fixtures.
 * Works by loading gradient_128x128.png, running generate_pyramid with
 * BlankTileStrategy::Emit, and byte-comparing every output tile against the
 * corresponding file in expected/emit_gradient/.
 * Input: gradient_128x128.png fixture, tile_size=64 -> Output: matches emit_gradient fixtures.
 */
#[test]
fn emit_gradient_matches_expected() {
    run_and_compare(
        "gradient_128x128.png",
        BlankTileStrategy::Emit,
        "emit_gradient",
    );
}

/**
 * Tests that tiles_skipped is zero when using the default Emit strategy,
 * even when the input image is entirely white (all tiles are uniform).
 * Works by loading the solid white fixture, generating a pyramid with Emit,
 * and asserting tiles_skipped == 0 and tiles_produced matches the plan.
 * Input: solid_white_128x128.png, Emit strategy -> Output: tiles_skipped == 0.
 */
#[test]
fn emit_solid_white_tiles_skipped_is_zero() {
    let src = load_input_png("solid_white_128x128.png");
    let planner = PyramidPlanner::new(IMG_SIZE, IMG_SIZE, TILE_SIZE, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = libviprs::MemorySink::new();
    let config = EngineConfig::default();

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();

    assert_eq!(result.tiles_skipped, 0);
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// Placeholder strategy tests
// ---------------------------------------------------------------------------

/**
 * Tests that Placeholder strategy on a solid white image produces output
 * identical to the pre-generated expected fixtures (all 1-byte markers).
 * Works by loading solid_white_128x128.png, running generate_pyramid with
 * BlankTileStrategy::Placeholder, and byte-comparing every output tile
 * against the corresponding file in expected/placeholder_solid_white/.
 * Input: solid_white_128x128.png, Placeholder -> Output: matches placeholder_solid_white fixtures.
 */
#[test]
fn placeholder_solid_white_matches_expected() {
    run_and_compare(
        "solid_white_128x128.png",
        BlankTileStrategy::Placeholder,
        "placeholder_solid_white",
    );
}

/**
 * Tests that Placeholder strategy on a gradient image produces output
 * identical to the pre-generated expected fixtures (all full-size tiles,
 * since no tiles in a gradient image are blank).
 * Works by loading gradient_128x128.png, running generate_pyramid with
 * BlankTileStrategy::Placeholder, and byte-comparing every output tile
 * against the corresponding file in expected/placeholder_gradient/.
 * Input: gradient_128x128.png, Placeholder -> Output: matches placeholder_gradient fixtures.
 */
#[test]
fn placeholder_gradient_matches_expected() {
    run_and_compare(
        "gradient_128x128.png",
        BlankTileStrategy::Placeholder,
        "placeholder_gradient",
    );
}

/**
 * Tests that Placeholder strategy on a half-white image produces output
 * identical to the pre-generated expected fixtures (mix of 1-byte markers
 * for blank tiles and full raster data for non-blank tiles).
 * Works by loading half_white_128x128.png, running generate_pyramid with
 * BlankTileStrategy::Placeholder, and byte-comparing every output tile
 * against the corresponding file in expected/placeholder_half_white/.
 * Input: half_white_128x128.png, Placeholder -> Output: matches placeholder_half_white fixtures.
 */
#[test]
fn placeholder_half_white_matches_expected() {
    run_and_compare(
        "half_white_128x128.png",
        BlankTileStrategy::Placeholder,
        "placeholder_half_white",
    );
}

// ---------------------------------------------------------------------------
// Placeholder marker file structure
// ---------------------------------------------------------------------------

/**
 * Tests that every tile in the placeholder_solid_white expected fixture
 * is exactly 1 byte containing BLANK_TILE_MARKER (0x00).
 * Works by scanning all .raw files in expected/placeholder_solid_white/
 * and asserting each has length 1 with the correct marker byte.
 * Input: pre-generated placeholder_solid_white fixtures -> Output: all files are 1-byte 0x00.
 */
#[test]
fn placeholder_solid_white_all_tiles_are_1_byte_markers() {
    let expected_files = collect_files(&expected_dir("placeholder_solid_white"), "raw");
    assert!(!expected_files.is_empty());

    for (path, bytes) in &expected_files {
        assert_eq!(
            bytes.len(),
            1,
            "Expected 1-byte placeholder at {path}, got {} bytes",
            bytes.len()
        );
        assert_eq!(
            bytes[0], BLANK_TILE_MARKER,
            "Wrong marker byte at {path}: expected 0x{BLANK_TILE_MARKER:02x}, got 0x{:02x}",
            bytes[0]
        );
    }
}

/**
 * Tests that every tile in the emit_solid_white expected fixture contains
 * full raster data (not a 1-byte placeholder), confirming that the Emit
 * strategy writes complete tile images even for uniform-color tiles.
 * Works by scanning all .raw files in expected/emit_solid_white/ and
 * asserting each has length greater than 1.
 * Input: pre-generated emit_solid_white fixtures -> Output: all files > 1 byte.
 */
#[test]
fn emit_solid_white_all_tiles_are_full_size() {
    let expected_files = collect_files(&expected_dir("emit_solid_white"), "raw");
    assert!(!expected_files.is_empty());

    for (path, bytes) in &expected_files {
        assert!(
            bytes.len() > 1,
            "Tile at {path} should be full raster data, not a 1-byte placeholder"
        );
    }
}

/**
 * Tests that no tiles in the placeholder_gradient expected fixture are
 * 1-byte markers, since a gradient image has no uniform-color tiles.
 * Works by scanning all .raw files in expected/placeholder_gradient/ and
 * asserting each has length greater than 1.
 * Input: pre-generated placeholder_gradient fixtures -> Output: all files > 1 byte.
 */
#[test]
fn placeholder_gradient_no_tiles_are_markers() {
    let expected_files = collect_files(&expected_dir("placeholder_gradient"), "raw");
    assert!(!expected_files.is_empty());

    for (path, bytes) in &expected_files {
        assert!(
            bytes.len() > 1,
            "Gradient tile at {path} should not be a placeholder"
        );
    }
}

/**
 * Tests that the placeholder_half_white expected fixture contains a mix of
 * 1-byte marker files (for the solid white right-half tiles) and full-size
 * raster files (for the gradient left-half tiles).
 * Works by counting files with length == 1 and length > 1 in
 * expected/placeholder_half_white/ and asserting both counts are non-zero.
 * Input: pre-generated placeholder_half_white fixtures -> Output: markers > 0 and full > 0.
 */
#[test]
fn placeholder_half_white_has_mix_of_markers_and_full_tiles() {
    let expected_files = collect_files(&expected_dir("placeholder_half_white"), "raw");
    assert!(!expected_files.is_empty());

    let markers = expected_files.iter().filter(|(_, b)| b.len() == 1).count();
    let full = expected_files.iter().filter(|(_, b)| b.len() > 1).count();

    assert!(markers > 0, "Expected some placeholder markers in half-white output");
    assert!(full > 0, "Expected some full tiles in half-white output");
}

// ---------------------------------------------------------------------------
// Placeholder with concurrency matches single-threaded expected output
// ---------------------------------------------------------------------------

/**
 * Tests that multi-threaded Placeholder strategy produces output identical
 * to the single-threaded expected fixtures, ensuring thread-safe blank
 * tile detection and marker generation.
 * Works by loading solid_white_128x128.png, running generate_pyramid with
 * BlankTileStrategy::Placeholder and concurrency=4, then byte-comparing
 * every output tile against expected/placeholder_solid_white/ (which was
 * generated single-threaded).
 * Input: solid_white_128x128.png, Placeholder, 4 threads -> Output: matches single-threaded fixtures.
 */
#[test]
fn placeholder_concurrent_matches_expected() {
    let src = load_input_png("solid_white_128x128.png");
    let planner = PyramidPlanner::new(IMG_SIZE, IMG_SIZE, TILE_SIZE, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("output");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);
    let config = EngineConfig::default()
        .with_blank_tile_strategy(BlankTileStrategy::Placeholder)
        .with_concurrency(4);

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(result.tiles_skipped, plan.total_tile_count());

    // Compare against single-threaded expected output
    let expected_files = collect_files(&expected_dir("placeholder_solid_white"), "raw");
    let actual_files = collect_files(&base, "raw");

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
// is_blank_tile utility — tested against fixture inputs
// ---------------------------------------------------------------------------

/**
 * Tests that is_blank_tile returns true for the solid white fixture image,
 * since every pixel in the raster is identical (255, 255, 255).
 * Works by loading solid_white_128x128.png from fixtures and passing the
 * decoded raster to is_blank_tile.
 * Input: solid_white_128x128.png fixture -> Output: true.
 */
#[test]
fn is_blank_tile_detects_solid_white_fixture() {
    let src = load_input_png("solid_white_128x128.png");
    assert!(is_blank_tile(&src));
}

/**
 * Tests that is_blank_tile returns false for the gradient fixture image,
 * since pixels vary across the raster.
 * Works by loading gradient_128x128.png from fixtures and passing the
 * decoded raster to is_blank_tile.
 * Input: gradient_128x128.png fixture -> Output: false.
 */
#[test]
fn is_blank_tile_rejects_gradient_fixture() {
    let src = load_input_png("gradient_128x128.png");
    assert!(!is_blank_tile(&src));
}

/**
 * Tests that is_blank_tile returns false for the half-white fixture image,
 * since the left half contains gradient pixels that differ from the white
 * right half.
 * Works by loading half_white_128x128.png from fixtures and passing the
 * decoded raster to is_blank_tile.
 * Input: half_white_128x128.png fixture -> Output: false.
 */
#[test]
fn is_blank_tile_rejects_half_white_fixture() {
    let src = load_input_png("half_white_128x128.png");
    assert!(!is_blank_tile(&src));
}

// ---------------------------------------------------------------------------
// tiles_skipped metric
// ---------------------------------------------------------------------------

/**
 * Tests that tiles_skipped equals total_tile_count when using Placeholder
 * strategy on a solid white image, since every tile is uniform-color.
 * Works by loading solid_white_128x128.png, generating a pyramid with
 * BlankTileStrategy::Placeholder into a MemorySink, and asserting
 * tiles_skipped == tiles_produced == plan.total_tile_count().
 * Input: solid_white_128x128.png, Placeholder -> Output: tiles_skipped == total_tile_count.
 */
#[test]
fn placeholder_solid_white_tiles_skipped_equals_total() {
    let src = load_input_png("solid_white_128x128.png");
    let planner = PyramidPlanner::new(IMG_SIZE, IMG_SIZE, TILE_SIZE, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = libviprs::MemorySink::new();
    let config = EngineConfig::default()
        .with_blank_tile_strategy(BlankTileStrategy::Placeholder);

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();

    assert_eq!(result.tiles_skipped, plan.total_tile_count());
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

/**
 * Tests that tiles_skipped is zero when using Placeholder strategy on a
 * gradient image, since no tiles in a gradient are uniform-color.
 * Works by loading gradient_128x128.png, generating a pyramid with
 * BlankTileStrategy::Placeholder into a MemorySink, and asserting
 * tiles_skipped == 0.
 * Input: gradient_128x128.png, Placeholder -> Output: tiles_skipped == 0.
 */
#[test]
fn placeholder_gradient_tiles_skipped_is_zero() {
    let src = load_input_png("gradient_128x128.png");
    let planner = PyramidPlanner::new(IMG_SIZE, IMG_SIZE, TILE_SIZE, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = libviprs::MemorySink::new();
    let config = EngineConfig::default()
        .with_blank_tile_strategy(BlankTileStrategy::Placeholder);

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();

    assert_eq!(result.tiles_skipped, 0);
}

/**
 * Tests that tiles_skipped is between 0 and tiles_produced (exclusive)
 * when using Placeholder strategy on a half-white image, confirming that
 * only the uniform-color tiles are skipped while the gradient tiles are
 * written in full.
 * Works by loading half_white_128x128.png, generating a pyramid with
 * BlankTileStrategy::Placeholder into a MemorySink, and asserting
 * 0 < tiles_skipped < tiles_produced.
 * Input: half_white_128x128.png, Placeholder -> Output: 0 < tiles_skipped < tiles_produced.
 */
#[test]
fn placeholder_half_white_tiles_skipped_is_partial() {
    let src = load_input_png("half_white_128x128.png");
    let planner = PyramidPlanner::new(IMG_SIZE, IMG_SIZE, TILE_SIZE, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = libviprs::MemorySink::new();
    let config = EngineConfig::default()
        .with_blank_tile_strategy(BlankTileStrategy::Placeholder);

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();

    assert!(result.tiles_skipped > 0, "Expected some tiles skipped");
    assert!(
        result.tiles_skipped < result.tiles_produced,
        "Expected only partial skip, not all tiles"
    );
}
