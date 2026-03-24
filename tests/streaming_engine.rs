//! Integration tests for the strip-based streaming pyramid engine.
//!
//! These tests verify that the streaming engine (`generate_pyramid_streaming`,
//! `generate_pyramid_auto`) produces **pixel-exact** output matching the
//! monolithic engine (`generate_pyramid`) across all supported layouts,
//! tile sizes, and image dimensions.
//!
//! The test strategy mirrors the existing monolithic integration tests:
//!
//! - **Parity tests** — generate pyramids with both engines from the same
//!   source image, sort tiles by coordinate, and assert byte-exact equality.
//!   Covers DeepZoom, Google, Google+centre layouts, odd dimensions, and
//!   real PDF fixture images.
//!
//! - **Auto-selection** — verifies that `generate_pyramid_auto` picks the
//!   monolithic path when the budget is large and the streaming path when
//!   the budget is constrained.
//!
//! - **Determinism** — confirms streaming parity across different tile sizes,
//!   ruling out edge-tile padding or strip-alignment regressions.
//!
//! - **Observability** — asserts that the streaming engine emits the same
//!   observer events (LevelStarted, TileCompleted, Finished) as monolithic.
//!
//! - **Blank tile strategy** — exercises `BlankTileStrategy::Placeholder`
//!   through the streaming path, verifying identical skip counts.
//!
//! - **Memory estimation** — sanity-checks that the streaming engine's
//!   reported peak memory is lower than monolithic for large images, and
//!   that `compute_strip_height` respects budget constraints.

use std::path::Path;

use libviprs::{
    BlankTileStrategy, CollectingObserver, EngineConfig, EngineEvent, Layout, MemorySink,
    PixelFormat, PyramidPlanner, Raster, RasterStripSource, StreamingConfig, generate_pyramid,
    generate_pyramid_auto, generate_pyramid_streaming,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a synthetic gradient raster with unique pixel values that exercise
/// the downscale averaging. The R channel varies with x, G with y, and B
/// with a prime-weighted combination that avoids repetitive patterns.
fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8; // R: horizontal gradient
            data[off + 1] = (y % 256) as u8; // G: vertical gradient
            data[off + 2] = ((x * 7 + y * 13) % 256) as u8; // B: mixed
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Create a uniform solid-colour raster for blank-tile testing.
fn solid_raster(w: u32, h: u32, val: u8) -> Raster {
    let data = vec![val; w as usize * h as usize * 3];
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Run monolithic engine and return sorted tiles.
fn monolithic_tiles(
    src: &Raster,
    plan: &libviprs::PyramidPlan,
    config: &EngineConfig,
) -> Vec<libviprs::sink::CollectedTile> {
    let sink = MemorySink::new();
    generate_pyramid(src, plan, &sink, config).unwrap();
    let mut tiles = sink.tiles();
    tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    tiles
}

/// Run streaming engine and return sorted tiles.
fn streaming_tiles(
    src: &Raster,
    plan: &libviprs::PyramidPlan,
    streaming_config: &StreamingConfig,
) -> Vec<libviprs::sink::CollectedTile> {
    let sink = MemorySink::new();
    let strip_src = RasterStripSource::new(src);
    generate_pyramid_streaming(
        &strip_src,
        plan,
        &sink,
        streaming_config,
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    let mut tiles = sink.tiles();
    tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    tiles
}

/// Run auto-selecting engine and return sorted tiles.
fn auto_tiles(
    src: &Raster,
    plan: &libviprs::PyramidPlan,
    streaming_config: &StreamingConfig,
) -> Vec<libviprs::sink::CollectedTile> {
    let sink = MemorySink::new();
    generate_pyramid_auto(
        src,
        plan,
        &sink,
        streaming_config,
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    let mut tiles = sink.tiles();
    tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    tiles
}

/// Compare two tile sets for byte-exact parity, asserting identical count,
/// coordinates, and pixel data. The `context` string is included in failure
/// messages to identify which test scenario diverged.
fn assert_tiles_match(
    ref_tiles: &[libviprs::sink::CollectedTile],
    test_tiles: &[libviprs::sink::CollectedTile],
    context: &str,
) {
    assert_eq!(
        ref_tiles.len(),
        test_tiles.len(),
        "{context}: tile count mismatch ({} vs {})",
        ref_tiles.len(),
        test_tiles.len(),
    );
    for (r, t) in ref_tiles.iter().zip(test_tiles.iter()) {
        assert_eq!(r.coord, t.coord, "{context}: coord mismatch");
        assert_eq!(
            r.data,
            t.data,
            "{context}: tile data diverged at {:?} (expected {} bytes, got {} bytes)",
            t.coord,
            r.data.len(),
            t.data.len(),
        );
    }
}

// ---------------------------------------------------------------------------
// 1. Parity: streaming produces identical output to monolithic
// ---------------------------------------------------------------------------

#[test]
fn streaming_parity_deepzoom_512x384() {
    let src = gradient_raster(512, 384);
    let planner = PyramidPlanner::new(512, 384, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
    let streaming_config = StreamingConfig {
        memory_budget_bytes: 500_000, // Force streaming
        engine: EngineConfig::default(),
    };
    let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
    assert_tiles_match(&ref_tiles, &test_tiles, "DeepZoom 512x384 streaming");
}

#[test]
fn streaming_parity_deepzoom_300x200() {
    let src = gradient_raster(300, 200);
    let planner = PyramidPlanner::new(300, 200, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
    let streaming_config = StreamingConfig {
        memory_budget_bytes: 200_000,
        engine: EngineConfig::default(),
    };
    let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
    assert_tiles_match(&ref_tiles, &test_tiles, "DeepZoom 300x200 streaming");
}

#[test]
fn streaming_parity_deepzoom_odd_dimensions() {
    // Non-power-of-2 sizes that stress edge handling
    for (w, h) in [(500, 300), (1023, 769), (257, 129)] {
        let src = gradient_raster(w, h);
        let planner = PyramidPlanner::new(w, h, 256, 0, Layout::DeepZoom).unwrap();
        let plan = planner.plan();

        let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
        let streaming_config = StreamingConfig {
            memory_budget_bytes: 500_000,
            engine: EngineConfig::default(),
        };
        let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
        assert_tiles_match(
            &ref_tiles,
            &test_tiles,
            &format!("DeepZoom {w}x{h} streaming"),
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Auto-selection: large budget → monolithic (bit-exact), small → streaming
// ---------------------------------------------------------------------------

#[test]
fn auto_selects_monolithic_for_large_budget() {
    let src = gradient_raster(256, 256);
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
    let config = StreamingConfig {
        memory_budget_bytes: u64::MAX, // Huge budget → monolithic
        engine: EngineConfig::default(),
    };
    let test_tiles = auto_tiles(&src, &plan, &config);
    assert_tiles_match(&ref_tiles, &test_tiles, "auto large budget");
}

#[test]
fn auto_selects_streaming_for_tiny_budget() {
    let src = gradient_raster(512, 512);
    let planner = PyramidPlanner::new(512, 512, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let sink = MemorySink::new();
    let config = StreamingConfig {
        memory_budget_bytes: 1_000, // Tiny → streaming
        engine: EngineConfig::default(),
    };
    let result = generate_pyramid_auto(
        &src,
        &plan,
        &sink,
        &config,
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// 3. Determinism: streaming across tile sizes
// ---------------------------------------------------------------------------

#[test]
fn streaming_deterministic_across_tile_sizes() {
    let src = gradient_raster(300, 200);

    for tile_size in [64, 128, 256] {
        let planner = PyramidPlanner::new(300, 200, tile_size, 0, Layout::DeepZoom).unwrap();
        let plan = planner.plan();

        let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
        let streaming_config = StreamingConfig {
            memory_budget_bytes: 200_000,
            engine: EngineConfig::default(),
        };
        let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
        assert_tiles_match(&ref_tiles, &test_tiles, &format!("tile_size={tile_size}"));
    }
}

// ---------------------------------------------------------------------------
// 4. Google layout + centre parity
// ---------------------------------------------------------------------------

#[test]
fn streaming_parity_google_centre_small() {
    let src = gradient_raster(400, 300);
    let planner = PyramidPlanner::new(400, 300, 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();

    let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
    let streaming_config = StreamingConfig {
        memory_budget_bytes: 500_000,
        engine: EngineConfig::default(),
    };
    let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
    assert_tiles_match(&ref_tiles, &test_tiles, "Google centre 400x300");
}

#[test]
fn streaming_parity_google_no_centre() {
    let src = gradient_raster(500, 300);
    let planner = PyramidPlanner::new(500, 300, 256, 0, Layout::Google).unwrap();
    let plan = planner.plan();

    let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
    let streaming_config = StreamingConfig {
        memory_budget_bytes: 500_000,
        engine: EngineConfig::default(),
    };
    let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
    assert_tiles_match(&ref_tiles, &test_tiles, "Google no-centre 500x300");
}

// ---------------------------------------------------------------------------
// 5. Observability: streaming observer events
// ---------------------------------------------------------------------------

#[test]
fn streaming_observer_events() {
    let src = gradient_raster(512, 384);
    let planner = PyramidPlanner::new(512, 384, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();
    let obs = CollectingObserver::new();

    let config = StreamingConfig {
        memory_budget_bytes: 500_000,
        engine: EngineConfig::default(),
    };
    let strip_src = RasterStripSource::new(&src);
    let result = generate_pyramid_streaming(&strip_src, &plan, &sink, &config, &obs).unwrap();

    let events = obs.events();

    let level_starts = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::LevelStarted { .. }))
        .count();
    let tile_completes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::TileCompleted { .. }))
        .count();
    let finishes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::Finished { .. }))
        .count();

    assert_eq!(level_starts, plan.level_count());
    assert_eq!(tile_completes as u64, plan.total_tile_count());
    assert_eq!(finishes, 1);
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// 6. Blank tile strategy with streaming
// ---------------------------------------------------------------------------

#[test]
fn streaming_blank_tile_placeholder_solid_white() {
    let src = solid_raster(128, 128, 255);
    let planner = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    // Monolithic reference
    let ref_config =
        EngineConfig::default().with_blank_tile_strategy(BlankTileStrategy::Placeholder);
    let ref_sink = MemorySink::new();
    let ref_result = generate_pyramid(&src, &plan, &ref_sink, &ref_config).unwrap();

    // Streaming
    let sink = MemorySink::new();
    let config = StreamingConfig {
        memory_budget_bytes: 50_000,
        engine: EngineConfig::default().with_blank_tile_strategy(BlankTileStrategy::Placeholder),
    };
    let strip_src = RasterStripSource::new(&src);
    let result = generate_pyramid_streaming(
        &strip_src,
        &plan,
        &sink,
        &config,
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(result.tiles_skipped, ref_result.tiles_skipped);
}

#[test]
fn streaming_blank_tile_placeholder_gradient() {
    let src = gradient_raster(128, 128);
    let planner = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let config = StreamingConfig {
        memory_budget_bytes: 50_000,
        engine: EngineConfig::default().with_blank_tile_strategy(BlankTileStrategy::Placeholder),
    };
    let strip_src = RasterStripSource::new(&src);
    let sink = MemorySink::new();
    let result = generate_pyramid_streaming(
        &strip_src,
        &plan,
        &sink,
        &config,
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    // Gradient has no blank tiles
    assert_eq!(result.tiles_skipped, 0);
}

// ---------------------------------------------------------------------------
// 7. Blueprint fixture parity (PDF extract → streaming pyramid)
// ---------------------------------------------------------------------------

const FIXTURE_PDF_PORTRAIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint-portrait.pdf"
);

#[test]
fn streaming_parity_blueprint_portrait() {
    let src = libviprs::extract_page_image(Path::new(FIXTURE_PDF_PORTRAIT), 1)
        .expect("failed to extract from blueprint-portrait.pdf");

    let planner = PyramidPlanner::new(src.width(), src.height(), 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
    let streaming_config = StreamingConfig {
        memory_budget_bytes: 2_000_000, // 2 MB — forces streaming for 3300x5024
        engine: EngineConfig::default(),
    };
    let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
    assert_tiles_match(&ref_tiles, &test_tiles, "blueprint portrait DeepZoom");
}

#[test]
fn streaming_parity_blueprint_portrait_google_centre() {
    let src = libviprs::extract_page_image(Path::new(FIXTURE_PDF_PORTRAIT), 1)
        .expect("failed to extract from blueprint-portrait.pdf");

    let planner = PyramidPlanner::new(src.width(), src.height(), 256, 0, Layout::Google)
        .unwrap()
        .with_centre(true);
    let plan = planner.plan();

    let ref_tiles = monolithic_tiles(&src, &plan, &EngineConfig::default());
    let streaming_config = StreamingConfig {
        memory_budget_bytes: 5_000_000, // 5 MB
        engine: EngineConfig::default(),
    };
    let test_tiles = streaming_tiles(&src, &plan, &streaming_config);
    assert_tiles_match(&ref_tiles, &test_tiles, "blueprint portrait Google+centre");
}

// ---------------------------------------------------------------------------
// 8. Memory estimation sanity
// ---------------------------------------------------------------------------

#[test]
fn streaming_peak_memory_lower_than_monolithic() {
    let src = gradient_raster(2048, 2048);
    let planner = PyramidPlanner::new(2048, 2048, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    // Monolithic peak
    let mono_sink = MemorySink::new();
    let mono_result = generate_pyramid(&src, &plan, &mono_sink, &EngineConfig::default()).unwrap();

    // Streaming with constrained budget
    let streaming_sink = MemorySink::new();
    let config = StreamingConfig {
        memory_budget_bytes: 1_000_000, // 1 MB
        engine: EngineConfig::default(),
    };
    let strip_src = RasterStripSource::new(&src);
    let stream_result = generate_pyramid_streaming(
        &strip_src,
        &plan,
        &streaming_sink,
        &config,
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    // Streaming peak should be less than monolithic peak
    assert!(
        stream_result.peak_memory_bytes < mono_result.peak_memory_bytes,
        "Streaming peak {} should be less than monolithic peak {}",
        stream_result.peak_memory_bytes,
        mono_result.peak_memory_bytes,
    );

    // Both should produce the same tile count
    assert_eq!(stream_result.tiles_produced, mono_result.tiles_produced);
}

#[test]
fn estimate_streaming_memory_reasonable() {
    let planner = PyramidPlanner::new(4096, 4096, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let mono_est = plan.estimate_peak_memory_for_format(PixelFormat::Rgb8);
    let stream_est = plan.estimate_streaming_peak_memory(PixelFormat::Rgb8, 512);

    // Streaming estimate should be much less than monolithic for large images
    assert!(
        stream_est < mono_est,
        "Streaming estimate {} should be < monolithic estimate {}",
        stream_est,
        mono_est,
    );

    // Streaming estimate should be positive
    assert!(stream_est > 0);
}

// ---------------------------------------------------------------------------
// 8b. Memory comparison: streaming vs monolithic across image sizes
// ---------------------------------------------------------------------------

/// Demonstrates that the streaming engine uses substantially less memory than
/// the monolithic engine, and that the gap widens with image size. This test
/// generates pyramids at several resolutions with both engines and asserts that
/// streaming peak memory is always a fraction of monolithic peak memory.
#[test]
fn streaming_memory_savings_scale_with_image_size() {
    let cases: &[(u32, u32)] = &[(1024, 1024), (2048, 2048), (4096, 4096)];

    let mut results: Vec<(u32, u32, u64, u64)> = Vec::new();

    for &(w, h) in cases {
        let src = gradient_raster(w, h);
        let planner = PyramidPlanner::new(w, h, 256, 0, Layout::DeepZoom).unwrap();
        let plan = planner.plan();

        // Monolithic
        let mono_sink = MemorySink::new();
        let mono_result =
            generate_pyramid(&src, &plan, &mono_sink, &EngineConfig::default()).unwrap();

        // Streaming with 1 MB budget
        let stream_sink = MemorySink::new();
        let config = StreamingConfig {
            memory_budget_bytes: 1_000_000,
            engine: EngineConfig::default(),
        };
        let strip_src = RasterStripSource::new(&src);
        let stream_result = generate_pyramid_streaming(
            &strip_src,
            &plan,
            &stream_sink,
            &config,
            &libviprs::observe::NoopObserver,
        )
        .unwrap();

        // Same tile count
        assert_eq!(
            mono_result.tiles_produced, stream_result.tiles_produced,
            "{w}x{h}: tile count mismatch"
        );

        // Streaming must use less memory
        assert!(
            stream_result.peak_memory_bytes < mono_result.peak_memory_bytes,
            "{w}x{h}: streaming peak ({}) should be < monolithic peak ({})",
            stream_result.peak_memory_bytes,
            mono_result.peak_memory_bytes,
        );

        results.push((
            w,
            h,
            mono_result.peak_memory_bytes,
            stream_result.peak_memory_bytes,
        ));
    }

    // The memory savings ratio should improve with image size.
    // For the largest image, streaming should use less than half the monolithic memory.
    let (w, h, mono_peak, stream_peak) = results.last().unwrap();
    let ratio = *stream_peak as f64 / *mono_peak as f64;
    assert!(
        ratio < 0.50,
        "{w}x{h}: streaming/monolithic ratio {ratio:.2} should be < 0.50 \
         (streaming={stream_peak}, monolithic={mono_peak})"
    );

    // Verify that savings grow with image size: the ratio for the largest
    // image should be less than the ratio for the smallest.
    let (_, _, first_mono, first_stream) = results.first().unwrap();
    let first_ratio = *first_stream as f64 / *first_mono as f64;
    assert!(
        ratio < first_ratio,
        "Streaming savings should grow with image size: \
         first ratio {first_ratio:.2}, last ratio {ratio:.2}"
    );
}

// ---------------------------------------------------------------------------
// 9. strip_height computation
// ---------------------------------------------------------------------------

#[test]
fn compute_strip_height_respects_budget() {
    let planner = PyramidPlanner::new(4096, 4096, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    // Budget must be large enough for at least one strip unit
    // (2×tile_size rows across all pyramid levels).
    let budget: u64 = 50_000_000;
    let sh = libviprs::compute_strip_height(&plan, PixelFormat::Rgb8, budget);
    assert!(
        sh.is_some(),
        "budget {budget} should allow at least one strip unit"
    );
    let sh = sh.unwrap();
    assert!(
        sh >= 512,
        "strip_height {sh} should be at least 2*tile_size"
    );
    assert_eq!(
        sh % 512,
        0,
        "strip_height {sh} should be multiple of 2*tile_size"
    );

    // Actual memory at this strip height should be within budget
    let est = libviprs::estimate_streaming_memory(&plan, PixelFormat::Rgb8, sh);
    assert!(
        est <= budget,
        "Estimated memory {est} exceeds budget {budget} for strip_height={sh}",
    );
}

#[test]
fn compute_strip_height_returns_none_for_impossible_budget() {
    let planner = PyramidPlanner::new(4096, 4096, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let sh = libviprs::compute_strip_height(&plan, PixelFormat::Rgb8, 1);
    assert!(sh.is_none());
}
