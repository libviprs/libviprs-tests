//! Phase 0 → Phase 2 — `EngineBuilder::run()` baseline surface tests.
//!
//! Pins the shape of the v1 builder API:
//!
//! - `EngineBuilder::new(source, plan, sink)` — three required args.
//! - Returns `EngineBuilder<S: TileSink>` so `.run()` is monomorphic in the sink.
//! - `.run()` produces the expected tile count for an in-memory `&Raster`.
//! - `.run_collect()` recovers the sink so callers can inspect tiles
//!   post-run.
//!
//! The previous revision of this file compared the builder output against
//! the old `generate_pyramid_observed` free function for parity. Those free
//! functions have been deleted; the builder is the sole public entry point
//! so parity is trivially satisfied. The tests here now assert on
//! builder-only observables (tile counts, produced tiles, background
//! padding) instead.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use libviprs::sink::{CollectedTile, MemorySink, TileFormat, TileSink};
use libviprs::{EngineBuilder, EngineConfig, Layout, PixelFormat, PyramidPlanner, Raster};

fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x * 7 + y * 13) % 256) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

fn sorted_tiles(sink: &MemorySink) -> Vec<CollectedTile> {
    let mut t = sink.tiles();
    t.sort_by_key(|c| (c.coord.level, c.coord.row, c.coord.col));
    t
}

fn same_tiles(a: &[CollectedTile], b: &[CollectedTile]) {
    assert_eq!(
        a.len(),
        b.len(),
        "tile count differs: {} vs {}",
        a.len(),
        b.len()
    );
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.coord, y.coord, "coord mismatch");
        assert_eq!(x.data, y.data, "data differ at {:?}", x.coord);
    }
}

#[test]
fn builder_new_accepts_raster_plan_sink() {
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = MemorySink::new();

    let _ = EngineBuilder::new(&src, plan, sink);
}

#[test]
fn builder_run_produces_expected_tile_count() {
    let src = gradient_raster(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let expected = plan.total_tile_count();

    let result = EngineBuilder::new(&src, plan, MemorySink::new())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, expected);
}

#[test]
fn builder_run_returns_sink_or_result_handle() {
    let src = gradient_raster(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, sink) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());
}

#[test]
fn builder_honours_with_concurrency() {
    let src = gradient_raster(200, 200);
    let plan = PyramidPlanner::new(200, 200, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (_, serial) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_concurrency(1)
        .run_collect()
        .unwrap();
    let (_, parallel) = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_concurrency(4)
        .run_collect()
        .unwrap();

    same_tiles(&sorted_tiles(&serial), &sorted_tiles(&parallel));
}

#[test]
fn builder_honours_with_buffer_size() {
    let src = gradient_raster(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, _) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_concurrency(2)
        .with_buffer_size(4)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn builder_honours_with_background_rgb() {
    // Smaller-than-tile image forces padding; background_rgb colours the pad.
    let src = gradient_raster(50, 50);
    let plan = PyramidPlanner::new(50, 50, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (_, sink) = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_background_rgb([10, 20, 30])
        .run_collect()
        .unwrap();

    // One edge tile at the top level must include the padding colour.
    let tiles = sink.tiles();
    let top = tiles
        .iter()
        .filter(|t| t.coord.level == tiles.iter().map(|t| t.coord.level).max().unwrap())
        .next()
        .unwrap();
    assert!(
        top.data.windows(3).any(|w| w == [10, 20, 30]),
        "background rgb not present in padded tile data"
    );
}

#[test]
fn builder_produces_expected_count_across_layouts() {
    for layout in [Layout::DeepZoom, Layout::Xyz, Layout::Google] {
        let src = gradient_raster(128, 96);
        let plan = PyramidPlanner::new(128, 96, 64, 0, layout).unwrap().plan();
        let expected = plan.total_tile_count();

        let (_, sink) = EngineBuilder::new(&src, plan, MemorySink::new())
            .run_collect()
            .unwrap();
        assert_eq!(sink.tile_count() as u64, expected, "layout {layout:?}");
    }
}

#[test]
fn builder_produces_expected_count_with_centre() {
    // --centre changes canvas shape; builder must route the plan through
    // unchanged. Assert tile count matches plan, not the un-centered count.
    let src = gradient_raster(100, 70);
    let plan = PyramidPlanner::new(100, 70, 64, 0, Layout::Google)
        .unwrap()
        .with_centre(true)
        .plan();
    let expected = plan.total_tile_count();

    let (_, sink) = EngineBuilder::new(&src, plan, MemorySink::new())
        .run_collect()
        .unwrap();

    assert_eq!(sink.tile_count() as u64, expected);
}

#[test]
fn builder_boxed_sink_is_usable() {
    // Supports `EngineBuilder<Box<dyn TileSink>>` so match-arms returning
    // different concrete sinks can be unified at the call site.
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink: Box<dyn TileSink> = Box::new(MemorySink::new());
    let result = EngineBuilder::new(&src, plan.clone(), sink).run().unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn builder_implements_debug() {
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let builder = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_concurrency(2)
        .with_buffer_size(8);

    let dbg = format!("{builder:?}");
    assert!(dbg.contains("concurrency") || dbg.contains("2"));
}
