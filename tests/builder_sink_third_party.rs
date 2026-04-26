//! Phase 0 TDD — third-party `TileSink` impl goes through the builder.
//!
//! Defines a toy sink inline so the test crate *is* the third-party consumer.
//! If this file stops compiling when libviprs bumps its `TileSink` surface,
//! we know the change is a breaking one for external implementers and needs
//! a migration story.
//!
//! Pins:
//!
//! - The minimum impl surface is `fn write_tile(&self, tile: &Tile)`; every
//!   other method must have a default so external sinks aren't forced to
//!   understand resume / checkpoints / retry counters.
//! - `EngineBuilder::new(source, plan, my_sink)` accepts any `impl TileSink`
//!   — no special trait or feature flag.
//! - `Box<dyn TileSink>` works for the match-arms-return-different-sinks
//!   case.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use libviprs::planner::TileCoord;
use libviprs::sink::{SinkError, Tile, TileSink};
use libviprs::{EngineBuilder, Layout, PyramidPlanner};

mod common;
use common::fixtures::canonical_raster_scaled;

/// Absolute-minimum third-party sink — the goal is to prove the trait
/// surface an external implementer must supply is just `write_tile`.
struct ToySink {
    seen: Mutex<Vec<TileCoord>>,
    byte_total: AtomicU64,
}

impl ToySink {
    fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            byte_total: AtomicU64::new(0),
        }
    }
    fn tile_count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    fn bytes(&self) -> u64 {
        self.byte_total.load(Ordering::Relaxed)
    }
}

impl TileSink for ToySink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.seen.lock().unwrap().push(tile.coord);
        self.byte_total
            .fetch_add(tile.raster.data().len() as u64, Ordering::Relaxed);
        Ok(())
    }
    // Every other method takes the trait default — no checkpoint, no
    // retry counters, no engine-config capture, no level init. Keeping
    // these as defaults is the contract for external implementers.
}

#[test]
fn toy_sink_accepted_by_builder() {
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = ToySink::new();
    // The builder takes `sink` by value. Accessing the sink after `.run()`
    // happens through `run_collect()` which returns `(EngineResult, S)`.
    let (result, sink) = EngineBuilder::new(&src, plan.clone(), sink)
        .run_collect()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());
    assert!(sink.bytes() > 0);
}

#[test]
fn boxed_third_party_sink_is_usable() {
    // A Box<dyn TileSink> holding any external sink must round-trip through
    // the builder the same way MemorySink does.
    let src = canonical_raster_scaled(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let boxed: Box<dyn TileSink> = Box::new(ToySink::new());
    let result = EngineBuilder::new(&src, plan.clone(), boxed).run().unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn match_returning_different_sinks_works_through_box() {
    // Demonstrates the intended pattern for call sites that pick a sink
    // based on runtime config.
    let mode = "toy"; // pretend this came from a CLI flag
    let sink: Box<dyn TileSink> = match mode {
        "toy" => Box::new(ToySink::new()),
        "memory" => Box::new(libviprs::sink::MemorySink::new()),
        _ => unreachable!(),
    };

    let src = canonical_raster_scaled(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    EngineBuilder::new(&src, plan.clone(), sink).run().unwrap();
}

#[test]
fn trait_object_for_tilesink_implements_tilesink() {
    // `impl TileSink for Box<dyn TileSink>` is load-bearing for the match
    // pattern above. This test simply exercises the trait-through-Box path
    // one more time and will fail to compile if the blanket impl regresses.
    let mut coord = TileCoord::new(0, 0, 0);
    coord.level = 1;
    let _ = coord;

    let boxed: Box<dyn TileSink> = Box::new(ToySink::new());
    // Method dispatch through the trait-object — compile-only check.
    let _count = boxed.sink_retry_count();
}
