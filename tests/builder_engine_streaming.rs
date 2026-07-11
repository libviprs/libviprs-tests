//! Phase 0 → Phase 2 — `EngineBuilder` over a `StripSource`.
//!
//! Pins:
//!
//! - `EngineBuilder::new(strip_source, plan, sink)` accepts anything
//!   implementing `StripSource` via the `IntoEngineSource` impl.
//! - `.with_memory_budget(bytes)` drives strip-height selection.
//! - `.with_budget_policy(BudgetPolicy::…)` honoured when the budget is
//!   too tight (error vs. auto-adjust).
//! - Strip-source path produces the same tile set as the default
//!   (Monolithic) path — they are two lenses on the same pyramid.
//!
//! The previous revision compared against the legacy
//! `generate_pyramid_streaming` free function. That function has been
//! deleted; these tests now assert on builder-only observables.

#![allow(unused_imports)]

use libviprs::sink::{CollectedTile, MemorySink, TileSink};
use libviprs::streaming::{BudgetPolicy, RasterStripSource, StreamingConfig, StripSource};
use libviprs::{EngineBuilder, EngineConfig, EngineKind, Layout, PyramidPlanner, Raster};

mod common;
use common::fixtures::canonical_raster_scaled;

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
    let src = canonical_raster_scaled(256, 256);
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
fn strip_source_and_monolithic_agree() {
    // Running the same raster through Monolithic vs Streaming must produce
    // byte-identical output — engines are lenses on the same pyramid.
    let src = canonical_raster_scaled(192, 128);
    let plan = PyramidPlanner::new(192, 128, 64, 0, Layout::Google)
        .unwrap()
        .plan();

    let (_, mono) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_engine(EngineKind::Monolithic)
        .run_collect()
        .unwrap();

    let (_, stream) = EngineBuilder::new(RasterStripSource::new(&src), plan, MemorySink::new())
        .with_engine(EngineKind::Streaming)
        .run_collect()
        .unwrap();

    assert_same_tiles(&sorted(&mono), &sorted(&stream));
}

#[test]
fn with_memory_budget_drives_strip_height() {
    let src = canonical_raster_scaled(512, 512);
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
    let src = canonical_raster_scaled(4096, 4096);
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
    let src = canonical_raster_scaled(256, 128);
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
    let src = canonical_raster_scaled(160, 160);
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
