//! Large-image stress tests.
//!
//! Unlike the rest of the test suite — which loads inputs from
//! `tests/fixtures/canonical_input.png` via
//! `tests/common/fixtures::canonical_raster_scaled` — these tests
//! deliberately keep an inline `synthetic_raster` generator. Rationale:
//!
//! 1. The stress paths run at 10K × 10K and 4K × 4K, well above the
//!    256 × 256 canonical input. Scaling the canonical that aggressively
//!    via nearest-neighbour produces large, blocky regions that don't
//!    exercise downscale-averaging the way a unique-per-pixel pattern
//!    does.
//! 2. Checking in a 10K × 10K RGB8 PNG (~300 MB) just to feed these
//!    tests would dominate the repo's disk footprint.
//! 3. Every test in this file is `#[ignore]`d by default (run via
//!    `cargo test -- --ignored stress`), so the synthetic input never
//!    feeds an automated comparison against a checked-in fixture.
//!
//! See issue #44 for the broader fixture-discipline migration; this
//! file is the documented Bucket D exception.

use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, MemorySink, PixelFormat, PyramidPlanner,
    Raster,
};

fn synthetic_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    // Stripe pattern for visual distinctness
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = if x % 64 < 32 { 200 } else { 50 };
            data[off + 1] = if y % 64 < 32 { 180 } else { 30 };
            data[off + 2] = ((x.wrapping_mul(7).wrapping_add(y.wrapping_mul(13))) % 256) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Large image stress test. Run with: cargo test -- --ignored stress
#[test]
#[ignore]
fn large_image_stress_10k() {
    let src = synthetic_raster(10_000, 10_000);
    let planner = PyramidPlanner::new(10_000, 10_000, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();
    let config = EngineConfig::default().with_concurrency(8);

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(config)
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    // Peak memory should be bounded: less than 512 MB for a ~300 MB source
    assert!(
        result.peak_memory_bytes < 512 * 1024 * 1024,
        "Peak memory {} exceeds 512 MB",
        result.peak_memory_bytes
    );
}

/// Verify determinism at scale under high concurrency.
#[test]
#[ignore]
fn large_image_determinism_stress() {
    let src = synthetic_raster(4096, 4096);
    let planner = PyramidPlanner::new(4096, 4096, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let ref_sink = MemorySink::new();
    EngineBuilder::new(&src, plan.clone(), &ref_sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap();
    let mut ref_tiles = ref_sink.tiles();
    ref_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    for concurrency in [4, 16, 32] {
        let sink = MemorySink::new();
        let config = EngineConfig::default().with_concurrency(concurrency);
        EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::Monolithic)
            .with_config(config)
            .run()
            .unwrap();

        let mut tiles = sink.tiles();
        tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

        assert_eq!(tiles.len(), ref_tiles.len());
        for (r, t) in ref_tiles.iter().zip(tiles.iter()) {
            assert_eq!(r.coord, t.coord);
            assert_eq!(
                r.data, t.data,
                "Diverged at {:?} concurrency={concurrency}",
                t.coord
            );
        }
    }
}

/// Many small pyramids in rapid succession.
#[test]
#[ignore]
fn rapid_fire_many_small_pyramids() {
    let src = synthetic_raster(256, 256);
    let planner = PyramidPlanner::new(256, 256, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    for i in 0..100 {
        let sink = MemorySink::new();
        let config = EngineConfig::default().with_concurrency(4);
        let result = EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::Monolithic)
            .with_config(config)
            .run()
            .unwrap();
        assert_eq!(
            result.tiles_produced,
            plan.total_tile_count(),
            "Failed on iteration {i}"
        );
    }
}
