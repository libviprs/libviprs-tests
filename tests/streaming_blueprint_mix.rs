// Streaming-engine counterpart to blueprint_mix_pyramid.rs. Mirrors the PDF→tile fixture-compare pattern through generate_pyramid_streaming.

#![allow(unused_imports, dead_code)]

use std::path::{Path, PathBuf};

use libviprs::{
    BudgetPolicy, EngineConfig, EngineEvent, EngineObserver, FsSink, Layout, MemorySink,
    PixelFormat, PyramidPlanner, RasterStripSource, StreamingConfig, TileFormat,
    extract_page_image, generate_pyramid, generate_pyramid_streaming,
};

/// Local no-op observer; the library's `NoopObserver` is not re-exported from
/// the crate root, and `generate_pyramid_streaming` requires a concrete
/// `&dyn EngineObserver` argument.
struct NoopObserver;
impl EngineObserver for NoopObserver {
    fn on_event(&self, _event: EngineEvent) {}
}

/// Build a `StreamingConfig` from a plain `EngineConfig`. The streaming API
/// does not expose a `new` constructor, so we populate the public fields
/// directly and take the remaining defaults (unbounded budget, error on
/// budget exceeded).
fn streaming_cfg(engine: EngineConfig) -> StreamingConfig {
    StreamingConfig {
        memory_budget_bytes: u64::MAX,
        engine,
        budget_policy: BudgetPolicy::Error,
    }
}

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

fn decode_png(data: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
        .expect("failed to decode PNG");
    let w = img.width();
    let h = img.height();
    let raw = img.into_bytes();
    (w, h, raw)
}

fn assert_tiles_pixel_equal_tol(
    expected_files: &[(String, Vec<u8>)],
    actual_files: &[(String, Vec<u8>)],
    context: &str,
    tolerance: u8,
) {
    let mut global_max_diff: u8 = 0;
    let mut max_diff_path = String::new();

    assert_eq!(
        expected_files.len(),
        actual_files.len(),
        "{context}: tile count mismatch: expected {}, got {}",
        expected_files.len(),
        actual_files.len(),
    );

    for ((exp_path, exp_bytes), (act_path, act_bytes)) in
        expected_files.iter().zip(actual_files.iter())
    {
        assert_eq!(
            exp_path, act_path,
            "{context}: path mismatch: expected {exp_path}, got {act_path}"
        );
        let (ew, eh, epx) = decode_png(exp_bytes);
        let (aw, ah, apx) = decode_png(act_bytes);

        assert!(
            aw >= ew && ah >= eh,
            "{context}: libviprs tile {act_path} ({aw}x{ah}) smaller than vips ({ew}x{eh})"
        );

        let exp_channels = epx.len() / (ew as usize * eh as usize);
        let act_channels = apx.len() / (aw as usize * ah as usize);
        assert_eq!(
            exp_channels, act_channels,
            "{context}: channel count mismatch at {act_path}"
        );
        let ch = exp_channels;

        for y in 0..eh as usize {
            let exp_row_start = y * ew as usize * ch;
            let act_row_start = y * aw as usize * ch;
            for x in 0..(ew as usize * ch) {
                let ev = epx[exp_row_start + x];
                let av = apx[act_row_start + x];
                let diff = (ev as i16 - av as i16).unsigned_abs() as u8;
                if diff > global_max_diff {
                    global_max_diff = diff;
                    max_diff_path = act_path.clone();
                }
                assert!(
                    diff <= tolerance,
                    "{context}: pixel mismatch at {act_path} row={y} col={} \
                     vips={ev} libviprs={av} diff={diff} tolerance={tolerance}",
                    x / ch
                );
            }
        }

        if aw > ew {
            for y in 0..ah as usize {
                let pad_start = y * aw as usize * ch + ew as usize * ch;
                let pad_end = y * aw as usize * ch + aw as usize * ch;
                assert!(
                    apx[pad_start..pad_end].iter().all(|&b| b == 255),
                    "{context}: right padding not white at row {y} in {act_path}"
                );
            }
        }
        if ah > eh {
            for y in eh as usize..ah as usize {
                let row_start = y * aw as usize * ch;
                let row_end = row_start + aw as usize * ch;
                assert!(
                    apx[row_start..row_end].iter().all(|&b| b == 255),
                    "{context}: bottom padding not white at row {y} in {act_path}"
                );
            }
        }
    }

    eprintln!(
        "{context}: max pixel diff = {global_max_diff} (at {max_diff_path}), tolerance = {tolerance}"
    );
}

fn load_blueprint_mix() -> libviprs::Raster {
    extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint-mix.pdf")
}

// ---------------------------------------------------------------------------
// PDFium-rendered pyramid constants (gated on `pdfium` feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "pdfium")]
const RENDERED_MIX_FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/rendered_blueprint_mix.png"
);
#[cfg(feature = "pdfium")]
const RENDERED_MIX_EXPECTED: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint_mix_rendered_expected"
);

#[cfg(feature = "pdfium")]
fn load_rendered_mix() -> libviprs::Raster {
    libviprs::decode_file(Path::new(RENDERED_MIX_FIXTURE))
        .expect("failed to load rendered_blueprint_mix.png")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify the extracted raster has the expected dimensions and pixel format.
/// Identical to the monolithic variant — proves the raster inputs are the
/// same as the ones fed through the streaming engine below.
#[test]
fn streaming_blueprint_mix_extraction_metadata() {
    let raster = load_blueprint_mix();

    assert_eq!(raster.width(), 12738);
    assert_eq!(raster.height(), 220);
    assert_eq!(raster.format(), PixelFormat::Rgb8);
}

/// Generate a pyramid via the streaming engine and compare every tile's
/// decoded pixels against the vips-generated expected fixtures.
#[test]
fn streaming_blueprint_mix_pyramid_matches_expected() {
    let raster = load_blueprint_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default();

    let source = RasterStripSource::new(&raster);
    let cfg = streaming_cfg(config.clone());
    let result = generate_pyramid_streaming(&source, &plan, &sink, &cfg, &NoopObserver)
        .expect("streaming pyramid");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Compare tiles (decoded pixel comparison)
    let expected_files = collect_files(Path::new(EXPECTED_DIR), "png");
    let actual_files = collect_files(&base, "png");
    assert_tiles_pixel_equal_tol(&expected_files, &actual_files, "streaming mix DeepZoom", 0);

    // Compare DZI manifest (semantic check)
    let expected_dzi = std::fs::read_to_string(PathBuf::from(EXPECTED_DIR).with_extension("dzi"))
        .expect("expected DZI manifest not found");
    let actual_dzi = std::fs::read_to_string(dir.path().join("blueprint_mix.dzi"))
        .expect("generated DZI manifest not found");

    for attr in [
        "Width=\"12738\"",
        "Height=\"220\"",
        "TileSize=\"256\"",
        "Overlap=\"0\"",
        "Format=\"png\"",
    ] {
        assert!(actual_dzi.contains(attr), "DZI missing {attr}");
        assert!(expected_dzi.contains(attr), "Expected DZI missing {attr}");
    }
}

/// Verify determinism: two streaming runs from the same source produce
/// bytewise-identical output.
#[test]
fn streaming_blueprint_mix_pyramid_deterministic() {
    let raster = load_blueprint_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink1 = MemorySink::new();
    let sink2 = MemorySink::new();
    let config = EngineConfig::default();

    let source1 = RasterStripSource::new(&raster);
    let cfg1 = streaming_cfg(config.clone());
    generate_pyramid_streaming(&source1, &plan, &sink1, &cfg1, &NoopObserver)
        .expect("streaming pyramid");

    let source2 = RasterStripSource::new(&raster);
    let cfg2 = streaming_cfg(config.clone());
    generate_pyramid_streaming(&source2, &plan, &sink2, &cfg2, &NoopObserver)
        .expect("streaming pyramid");

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

/// Concurrent streaming generation matches the vips-generated fixtures.
#[test]
fn streaming_blueprint_mix_pyramid_concurrent_matches_expected() {
    let raster = load_blueprint_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default().with_concurrency(4);

    let source = RasterStripSource::new(&raster);
    let cfg = streaming_cfg(config.clone());
    let result = generate_pyramid_streaming(&source, &plan, &sink, &cfg, &NoopObserver)
        .expect("streaming pyramid");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    let expected_files = collect_files(Path::new(EXPECTED_DIR), "png");
    let actual_files = collect_files(&base, "png");
    assert_tiles_pixel_equal_tol(
        &expected_files,
        &actual_files,
        "streaming mix concurrent",
        0,
    );
}

// ---------------------------------------------------------------------------
// PDFium-rendered streaming pyramid vs vips fixture comparison
// ---------------------------------------------------------------------------

/// Drive the streaming engine end-to-end from the pre-rendered PNG fixture and
/// compare every tile against the vips-generated fixtures (decoded pixel
/// comparison).
#[test]
#[cfg(feature = "pdfium")]
fn streaming_blueprint_mix_rendered_pyramid_matches_vips() {
    let raster = load_rendered_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix_rendered");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default();

    let source = RasterStripSource::new(&raster);
    let cfg = streaming_cfg(config.clone());
    let result = generate_pyramid_streaming(&source, &plan, &sink, &cfg, &NoopObserver)
        .expect("streaming rendered pyramid");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    let expected_files = collect_files(Path::new(RENDERED_MIX_EXPECTED), "png");
    let actual_files = collect_files(&base, "png");
    // Tolerance of 5 for RGBA rendered tiles: vips and libviprs produce
    // minor rounding differences when averaging alpha-channel pixels during
    // downscaling.
    assert_tiles_pixel_equal_tol(
        &expected_files,
        &actual_files,
        "streaming rendered mix vs vips",
        5,
    );
}

/// Concurrent streaming generation of the pdfium-rendered pyramid also
/// matches vips fixtures.
#[test]
#[cfg(feature = "pdfium")]
fn streaming_blueprint_mix_rendered_pyramid_concurrent_matches_vips() {
    let raster = load_rendered_mix();

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_mix_rendered");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default().with_concurrency(4);

    let source = RasterStripSource::new(&raster);
    let cfg = streaming_cfg(config.clone());
    let result = generate_pyramid_streaming(&source, &plan, &sink, &cfg, &NoopObserver)
        .expect("streaming rendered pyramid");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    let expected_files = collect_files(Path::new(RENDERED_MIX_EXPECTED), "png");
    let actual_files = collect_files(&base, "png");
    assert_tiles_pixel_equal_tol(
        &expected_files,
        &actual_files,
        "streaming rendered mix concurrent vs vips",
        5,
    );
}
