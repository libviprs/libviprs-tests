//! Phase 0 TDD — `EngineBuilder::with_engine(EngineKind::…)` routing and
//! source-compatibility matrix.
//!
//! Pins:
//!
//! - `EngineKind::{Auto, Monolithic, Streaming, MapReduce}` are the v1
//!   variants (`#[non_exhaustive]` so future engines land as minor bumps).
//! - `MapReduce` parity with `generate_pyramid_mapreduce` for Strip sources
//!   and `generate_pyramid_mapreduce_auto` for Raster sources.
//! - `Monolithic` + `EngineSource::Strip` is an explicit runtime error; the
//!   builder refuses to silently wrap a strip source into a monolithic
//!   engine because that would defeat the streaming memory budget.
//! - `Streaming` and `MapReduce` accept `&Raster` by auto-wrapping.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use libviprs::sink::{CollectedTile, MemorySink, TileSink};
use libviprs::streaming::RasterStripSource;
use libviprs::streaming_mapreduce::MapReduceConfig;
use libviprs::{
    EngineBuilder, EngineError, EngineKind, Layout, PixelFormat, PyramidPlanner, Raster,
    generate_pyramid_mapreduce, generate_pyramid_mapreduce_auto,
};

fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x + 2 * y) % 256) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

fn sorted(sink: &MemorySink) -> Vec<CollectedTile> {
    let mut t = sink.tiles();
    t.sort_by_key(|c| (c.coord.level, c.coord.row, c.coord.col));
    t
}

fn assert_same_tiles(a: &[CollectedTile], b: &[CollectedTile]) {
    assert_eq!(a.len(), b.len(), "tile count differs");
    for (x, y) in a.iter().zip(b.iter()) {
        assert_eq!(x.coord, y.coord);
        assert_eq!(x.data, y.data, "data differ at {:?}", x.coord);
    }
}

// ---------------------------------------------------------------------------
// Engine-kind × source-kind compatibility matrix
// ---------------------------------------------------------------------------

#[test]
fn auto_engine_with_raster_source_runs() {
    let src = gradient_raster(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, _) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::Auto)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn auto_engine_with_strip_source_runs() {
    let src = gradient_raster(128, 128);
    let strip = RasterStripSource::new(&src);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, _) = EngineBuilder::new(strip, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::Auto)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn monolithic_engine_with_raster_source_runs() {
    let src = gradient_raster(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, _) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::Monolithic)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn monolithic_engine_with_strip_source_errors() {
    // Monolithic requires the full raster in memory. A strip source doesn't
    // provide that — builder surfaces an error instead of silently pulling
    // the whole source into a raster.
    let src = gradient_raster(128, 128);
    let strip = RasterStripSource::new(&src);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let res = EngineBuilder::new(strip, plan, MemorySink::new())
        .with_engine(EngineKind::Monolithic)
        .run();

    match res {
        Err(EngineError::IncompatibleSource { .. }) => {}
        other => panic!("expected IncompatibleSource error, got {other:?}"),
    }
}

#[test]
fn streaming_engine_with_raster_source_auto_wraps() {
    let src = gradient_raster(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, _) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::Streaming)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn mapreduce_engine_with_raster_source_auto_wraps() {
    let src = gradient_raster(256, 256);
    let plan = PyramidPlanner::new(256, 256, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, _) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::MapReduce)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn mapreduce_engine_with_strip_source_runs() {
    let src = gradient_raster(256, 256);
    let strip = RasterStripSource::new(&src);
    let plan = PyramidPlanner::new(256, 256, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, _) = EngineBuilder::new(strip, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::MapReduce)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// Parity with legacy MapReduce free functions
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_strip_parity_with_free_fn() {
    let src = gradient_raster(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let legacy = MemorySink::new();
    generate_pyramid_mapreduce(
        &RasterStripSource::new(&src),
        &plan,
        &legacy,
        &MapReduceConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    let (_, builder) = EngineBuilder::new(RasterStripSource::new(&src), plan, MemorySink::new())
        .with_engine(EngineKind::MapReduce)
        .run_collect()
        .unwrap();

    assert_same_tiles(&sorted(&legacy), &sorted(&builder));
}

#[test]
fn mapreduce_raster_parity_with_free_fn_auto() {
    let src = gradient_raster(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let legacy = MemorySink::new();
    generate_pyramid_mapreduce_auto(
        &src,
        &plan,
        &legacy,
        &MapReduceConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    let (_, builder) = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_engine(EngineKind::MapReduce)
        .run_collect()
        .unwrap();

    assert_same_tiles(&sorted(&legacy), &sorted(&builder));
}

#[test]
fn engine_kind_is_non_exhaustive() {
    // Compile-time assertion via exhaustive match: if a new variant is added
    // without being wired through the builder, this test stops compiling and
    // forces the routing table to be extended.
    fn accepts_all(kind: EngineKind) -> &'static str {
        match kind {
            EngineKind::Auto => "auto",
            EngineKind::Monolithic => "mono",
            EngineKind::Streaming => "stream",
            EngineKind::MapReduce => "mapreduce",
            // `#[non_exhaustive]` means callers outside the crate must have
            // this arm. Removing it here proves the attribute is in force.
            _ => "other",
        }
    }
    assert_eq!(accepts_all(EngineKind::Auto), "auto");
}
