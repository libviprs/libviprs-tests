// Streaming-engine counterpart to pdf_to_pyramid.rs. Mirrors the PDF→tile fixture-compare pattern through generate_pyramid_streaming.

use std::path::Path;

use libviprs::{
    BudgetPolicy, EngineConfig, EngineEvent, EngineObserver, FsSink, GeoCoord, GeoTransform,
    Layout, MemorySink, PixelFormat, PyramidPlanner, RasterStripSource, StreamingConfig,
    TileFormat, extract_page_image, generate_pyramid, generate_pyramid_auto,
    generate_pyramid_streaming,
};

const FIXTURE_PDF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");

/// Local no-op observer — `NoopObserver` from the crate is not re-exported.
struct NoopObs;
impl EngineObserver for NoopObs {
    fn on_event(&self, _event: EngineEvent) {}
}

/// Build a `StreamingConfig` from an `EngineConfig` and a budget.
fn streaming_cfg(engine: EngineConfig, budget: u64) -> StreamingConfig {
    StreamingConfig {
        memory_budget_bytes: budget,
        engine,
        budget_policy: BudgetPolicy::Error,
    }
}

// ---------------------------------------------------------------------------
// Ported reference tests (streaming branch)
// ---------------------------------------------------------------------------

/// Full pipeline: PDF → raster → geo-reference → streaming pyramid (in-memory sink).
#[test]
fn streaming_pdf_to_georeferenced_pyramid_memory() {
    // Step 1: Extract raster from PDF
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let w = raster.width();
    let h = raster.height();
    assert!(w > 0 && h > 0, "Extracted raster has zero dimensions");

    // Step 2: Attach geo-reference (simulated construction site coordinates)
    let geo =
        GeoTransform::from_origin_and_scale(GeoCoord::new(-122.4194, 37.7749), 0.0001, -0.0001);

    let bounds = geo.image_bounds(w, h);
    assert!(bounds.width() > 0.0);
    assert!(bounds.height() > 0.0);
    assert!(
        bounds
            .contains(geo.pixel_to_geo(libviprs::PixelCoord::new(w as f64 / 2.0, h as f64 / 2.0)))
    );

    // Step 3: Generate tile pyramid via the streaming engine
    let tile_size = 256;
    let planner = PyramidPlanner::new(w, h, tile_size, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink = MemorySink::new();
    let config = EngineConfig::default().with_concurrency(4);

    let source = RasterStripSource::new(&raster);
    let s_config = streaming_cfg(config.clone(), u64::MAX);
    let result = generate_pyramid_streaming(&source, &plan, &sink, &s_config, &NoopObs)
        .expect("streaming pyramid");

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());

    // Step 4: Verify geo-coordinates for interior tile centers are within bounds.
    let top_level = plan.levels.last().unwrap();
    for row in 0..top_level.rows {
        for col in 0..top_level.cols {
            let px_center_x = (col as f64 + 0.5) * tile_size as f64;
            let px_center_y = (row as f64 + 0.5) * tile_size as f64;
            if px_center_x >= w as f64 || px_center_y >= h as f64 {
                continue;
            }
            let center = geo.tile_center(col, row, tile_size);
            assert!(
                bounds.contains(center),
                "Tile ({col}, {row}) center ({}, {}) outside image bounds",
                center.x,
                center.y,
            );
        }
    }
}

/// Full streaming pipeline to filesystem with PNG encoding.
#[test]
fn streaming_pdf_to_pyramid_filesystem_png() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let w = raster.width();
    let h = raster.height();

    let tile_size = 256;
    let planner = PyramidPlanner::new(w, h, tile_size, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_tiles");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default().with_concurrency(4);

    let source = RasterStripSource::new(&raster);
    let s_config = streaming_cfg(config.clone(), u64::MAX);
    let result = generate_pyramid_streaming(&source, &plan, &sink, &s_config, &NoopObs)
        .expect("streaming pyramid");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Verify DZI manifest attributes
    let dzi_path = dir.path().join("blueprint_tiles.dzi");
    assert!(dzi_path.exists(), "DZI manifest not found");
    let manifest = std::fs::read_to_string(&dzi_path).unwrap();
    assert!(
        manifest.contains(&format!("Width=\"{w}\"")),
        "DZI missing Width attribute"
    );
    assert!(
        manifest.contains(&format!("Height=\"{h}\"")),
        "DZI missing Height attribute"
    );
    assert!(
        manifest.contains(&format!("TileSize=\"{tile_size}\"")),
        "DZI missing TileSize attribute"
    );
    assert!(
        manifest.contains("Format=\"png\""),
        "DZI missing/incorrect Format attribute: {manifest}"
    );
    assert!(
        manifest.contains("Overlap="),
        "DZI missing Overlap attribute"
    );

    // Verify representative top-level tile is valid PNG
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0_0.png", top.level));
    assert!(tile_path.exists(), "Top-level tile not found");
    let bytes = std::fs::read(&tile_path).unwrap();
    assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
}

/// Verify determinism: same PDF produces identical streamed pyramids across runs.
#[test]
fn streaming_pdf_pyramid_deterministic() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink1 = MemorySink::new();
    let sink2 = MemorySink::new();
    let config = EngineConfig::default().with_concurrency(4);

    let source = RasterStripSource::new(&raster);
    let s_config = streaming_cfg(config.clone(), u64::MAX);

    generate_pyramid_streaming(&source, &plan, &sink1, &s_config, &NoopObs)
        .expect("streaming pyramid run 1");
    generate_pyramid_streaming(&source, &plan, &sink2, &s_config, &NoopObs)
        .expect("streaming pyramid run 2");

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

/// Verify extracted raster pixel format is usable for streaming pyramiding.
#[test]
fn streaming_pdf_raster_format_compatible() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let fmt = raster.format();
    assert!(
        fmt == PixelFormat::Rgb8 || fmt == PixelFormat::Rgba8 || fmt == PixelFormat::Gray8,
        "Unexpected pixel format from PDF extraction: {:?}",
        fmt
    );
}

// ---------------------------------------------------------------------------
// Auto-selector tests
// ---------------------------------------------------------------------------

/// Under a tight memory budget, `generate_pyramid_auto` must route to the
/// streaming branch and produce bytewise-identical output to a direct
/// `generate_pyramid_streaming` run with the same config.
#[test]
fn streaming_auto_matches_streaming_branch_under_tight_budget() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let engine = EngineConfig::default().with_concurrency(4);

    // 8 MB — tight enough to force streaming for a typical blueprint page,
    // but still large enough to cover the minimum aligned strip.
    let tight_budget: u64 = 8 * 1024 * 1024;
    let s_config = streaming_cfg(engine, tight_budget);

    // Auto path (should select streaming under tight budget).
    let auto_sink = MemorySink::new();
    generate_pyramid_auto(&raster, &plan, &auto_sink, &s_config, &NoopObs)
        .expect("auto pyramid (tight)");

    // Direct streaming path.
    let stream_sink = MemorySink::new();
    let source = RasterStripSource::new(&raster);
    generate_pyramid_streaming(&source, &plan, &stream_sink, &s_config, &NoopObs)
        .expect("direct streaming pyramid");

    let mut auto_tiles = auto_sink.tiles();
    let mut stream_tiles = stream_sink.tiles();
    auto_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    stream_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    assert_eq!(auto_tiles.len(), stream_tiles.len());
    for (a, s) in auto_tiles.iter().zip(stream_tiles.iter()) {
        assert_eq!(a.coord, s.coord);
        assert_eq!(
            a.data, s.data,
            "Tile {:?} differs between auto and streaming",
            a.coord
        );
    }
}

/// Under a huge memory budget, `generate_pyramid_auto` must route to the
/// monolithic branch and produce bytewise-identical output to a direct
/// `generate_pyramid` call.
#[test]
fn streaming_auto_matches_monolithic_branch_under_large_budget() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let engine = EngineConfig::default().with_concurrency(4);
    let s_config = streaming_cfg(engine.clone(), u64::MAX);

    // Auto path (should select monolithic under unlimited budget).
    let auto_sink = MemorySink::new();
    generate_pyramid_auto(&raster, &plan, &auto_sink, &s_config, &NoopObs)
        .expect("auto pyramid (huge budget)");

    // Direct monolithic path.
    let mono_sink = MemorySink::new();
    generate_pyramid(&raster, &plan, &mono_sink, &engine).expect("monolithic pyramid");

    let mut auto_tiles = auto_sink.tiles();
    let mut mono_tiles = mono_sink.tiles();
    auto_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    mono_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    assert_eq!(auto_tiles.len(), mono_tiles.len());
    for (a, m) in auto_tiles.iter().zip(mono_tiles.iter()) {
        assert_eq!(a.coord, m.coord);
        assert_eq!(
            a.data, m.data,
            "Tile {:?} differs between auto and monolithic",
            a.coord
        );
    }
}
