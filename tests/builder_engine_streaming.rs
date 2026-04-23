//! Phase 0 TDD — `EngineBuilder` over a `StripSource` parity with
//! `generate_pyramid_streaming`.
//!
//! Pins:
//!
//! - `EngineBuilder::new(strip_source, plan, sink)` accepts anything
//!   implementing `StripSource` via `From<T: StripSource>` on `EngineSource`.
//! - `.with_memory_budget(bytes)` drives strip-height selection identically
//!   to today's `StreamingConfig::memory_budget_bytes`.
//! - `.with_budget_policy(BudgetPolicy::…)` honoured when the budget is too
//!   tight (error vs. auto-adjust).
//! - Strip-source path still emits the full set of tiles byte-for-byte
//!   matching the monolithic path.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use libviprs::sink::{CollectedTile, MemorySink, TileSink};
use libviprs::streaming::{BudgetPolicy, RasterStripSource, StreamingConfig, StripSource};
use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, PixelFormat, PyramidPlanner, Raster,
    generate_pyramid_observed, generate_pyramid_streaming,
};

fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x * 3 + y * 5) % 256) as u8;
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

#[test]
fn builder_accepts_strip_source() {
    let src = gradient_raster(256, 256);
    let strip = RasterStripSource::new(&src);
    let plan = PyramidPlanner::new(256, 256, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (_, sink) = EngineBuilder::new(strip, plan.clone(), MemorySink::new())
        .run_collect()
        .unwrap();

    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());
}

#[test]
fn builder_strip_source_parity_with_streaming_free_fn() {
    let src = gradient_raster(320, 240);
    let plan = PyramidPlanner::new(320, 240, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Legacy path: explicit streaming free function.
    let legacy = MemorySink::new();
    generate_pyramid_streaming(
        &RasterStripSource::new(&src),
        &plan,
        &legacy,
        &StreamingConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    // Builder path: same source wrapped via From, routed by auto-engine.
    let (_, builder) = EngineBuilder::new(RasterStripSource::new(&src), plan, MemorySink::new())
        .run_collect()
        .unwrap();

    assert_same_tiles(&sorted(&legacy), &sorted(&builder));
}

#[test]
fn builder_strip_parity_with_monolithic() {
    // Streaming path and monolithic path must agree byte-for-byte — the
    // builder must not introduce its own rounding/ordering differences.
    let src = gradient_raster(192, 128);
    let plan = PyramidPlanner::new(192, 128, 64, 0, Layout::Google)
        .unwrap()
        .plan();

    let mono = MemorySink::new();
    generate_pyramid_observed(
        &src,
        &plan,
        &mono,
        &EngineConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    let (_, stream) = EngineBuilder::new(RasterStripSource::new(&src), plan, MemorySink::new())
        .with_engine(EngineKind::Streaming)
        .run_collect()
        .unwrap();

    assert_same_tiles(&sorted(&mono), &sorted(&stream));
}

#[test]
fn with_memory_budget_drives_strip_height() {
    let src = gradient_raster(512, 512);
    let plan = PyramidPlanner::new(512, 512, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Tight budget — builder must still succeed under auto-adjust policy.
    let (_, sink) = EngineBuilder::new(
        RasterStripSource::new(&src),
        plan.clone(),
        MemorySink::new(),
    )
    .with_engine(EngineKind::Streaming)
    .with_memory_budget(4 * 1024 * 1024) // 4 MiB
    .with_budget_policy(BudgetPolicy::AutoAdjustDpi { min_dpi: 72 })
    .run_collect()
    .unwrap();

    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());
}

#[test]
fn with_budget_policy_error_surfaces_on_tight_budget() {
    let src = gradient_raster(4096, 4096);
    let plan = PyramidPlanner::new(4096, 4096, 256, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let res = EngineBuilder::new(RasterStripSource::new(&src), plan, MemorySink::new())
        .with_engine(EngineKind::Streaming)
        .with_memory_budget(1024) // deliberately infeasible
        .with_budget_policy(BudgetPolicy::Error)
        .run();

    assert!(
        res.is_err(),
        "tight budget + Error policy must surface an error"
    );
}

#[test]
fn raster_source_auto_wrapped_for_streaming_kind() {
    // `EngineKind::Streaming` + `&Raster` — builder must auto-wrap into a
    // RasterStripSource. No strip-source plumbing at the call site.
    let src = gradient_raster(256, 128);
    let plan = PyramidPlanner::new(256, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (result, sink) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::Streaming)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());
}

#[test]
fn explicit_streaming_kind_does_not_change_output_vs_auto() {
    let src = gradient_raster(160, 160);
    let plan = PyramidPlanner::new(160, 160, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (_, auto) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .run_collect()
        .unwrap();

    let (_, streaming) = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_engine(EngineKind::Streaming)
        .run_collect()
        .unwrap();

    assert_same_tiles(&sorted(&auto), &sorted(&streaming));
}
