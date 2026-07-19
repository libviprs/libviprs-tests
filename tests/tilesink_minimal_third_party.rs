//! Issue #292 — the public `TileSink` contract must stay a *one-obligation*
//! trait for external implementers, and the wrapper forwarding impls
//! (`Box`/`&`/`Arc`) must be generated from a single macro so they cannot drift
//! apart or silently drop a newly added engine hook.
//!
//! What this pins that `builder_sink_third_party.rs` does not:
//!
//! - A third-party sink shared through `Arc<T>` is accepted by
//!   `EngineBuilder::new` *by value*, so an external author can keep a handle
//!   to a custom collecting sink and read it back after `run()` without
//!   juggling borrows across the `run()` call. Before #292 only `Box<T>` and
//!   `&T` forwarded, so `Arc<T>` did not implement `TileSink` and this file did
//!   not compile.
//! - Every engine-bookkeeping hook forwards through the `Arc<T>` wrapper the
//!   same way it must through `Box`/`&` — guarding the drift hazard #292 cites
//!   (the class of per-wrapper omission that caused the silent-data-loss trap
//!   of #137).
//!
//! Non-behavioural finding (API surface / architecture): the assertions are
//! deterministic trait-contract checks, not pixel comparisons, so there is no
//! vips-differential golden fixture here.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use libviprs::planner::TileCoord;
use libviprs::sink::{SinkError, Tile, TileFormat, TileSink};
use libviprs::{EngineBuilder, EngineConfig, Layout, PyramidPlanner};

mod common;
use common::fixtures::canonical_raster_scaled;

/// Absolute-minimum third-party sink: overrides only `write_tile` and
/// `finish`. Records the coords it saw so the test can prove the engine
/// actually drove it, and flags that `finish` ran.
struct CountingSink {
    seen: Mutex<Vec<TileCoord>>,
    finished: AtomicBool,
}

impl CountingSink {
    fn new() -> Self {
        Self {
            seen: Mutex::new(Vec::new()),
            finished: AtomicBool::new(false),
        }
    }
    fn tile_count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    fn finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
}

impl TileSink for CountingSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.seen.lock().unwrap().push(tile.coord);
        Ok(())
    }
    fn finish(&self) -> Result<(), SinkError> {
        self.finished.store(true, Ordering::Relaxed);
        Ok(())
    }
    // Every engine hook takes the trait default — no checkpoint, no retry
    // counters, no engine-config capture, no level init. That an external
    // author needs none of them is the contract this file pins.
}

/// A third-party sink shared through `Arc` is accepted by the builder by value;
/// the caller keeps a clone and reads it back after `run()`.
#[test]
fn arc_shared_minimal_sink_drives_engine() {
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = Arc::new(CountingSink::new());
    let handle = Arc::clone(&sink);

    // `Arc<CountingSink>` must satisfy `TileSink` (issue #292): the builder
    // takes the sink by value, so sharing a handle needs `Arc<T>: TileSink`.
    // Before the macro forwarding covered `Arc<T>` this call did not compile.
    let result = EngineBuilder::new(&src, plan.clone(), sink).run().unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(handle.tile_count() as u64, plan.total_tile_count());
    assert!(
        handle.finished(),
        "engine must call finish() on the Arc-shared minimal sink"
    );
}

/// A leaf sink that returns a distinct, non-default value from every engine
/// bookkeeping hook so a wrapper's forwarding can be observed end to end.
struct HookLeaf {
    root: PathBuf,
    skips: AtomicU64,
    levels_seen: AtomicU64,
    config_seen: AtomicBool,
}

impl HookLeaf {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            skips: AtomicU64::new(0),
            levels_seen: AtomicU64::new(0),
            config_seen: AtomicBool::new(false),
        }
    }
}

impl TileSink for HookLeaf {
    fn write_tile(&self, _tile: &Tile) -> Result<(), SinkError> {
        Ok(())
    }
    fn record_engine_config(&self, _config: &EngineConfig) {
        self.config_seen.store(true, Ordering::Relaxed);
    }
    fn sink_retry_count(&self) -> u64 {
        7
    }
    fn sink_skipped_due_to_failure(&self) -> u64 {
        3
    }
    fn note_sink_skipped(&self) {
        self.skips.fetch_add(1, Ordering::Relaxed);
    }
    fn checkpoint_root(&self) -> Option<&Path> {
        Some(&self.root)
    }
    fn init_level_count(&self, levels: usize) {
        self.levels_seen.store(levels as u64, Ordering::Relaxed);
    }
    fn content_format(&self) -> Option<TileFormat> {
        Some(TileFormat::Jpeg { quality: 80 })
    }
    fn applies_retry_policy(&self) -> bool {
        true
    }
}

/// Exercises the side-effecting hooks through a generic `S: TileSink` bound so
/// the wrapper's `TileSink` impl — not the leaf's via deref — is what forwards.
fn drive_side_effecting_hooks<S: TileSink>(s: &S) {
    s.note_sink_skipped();
    s.note_sink_skipped();
    s.init_level_count(5);
    s.record_engine_config(&EngineConfig::default());
}

/// Every engine-bookkeeping hook forwards through the `Arc<T>` wrapper — the
/// same guarantee the `Box`/`&` wrappers give. If the single forwarding macro
/// (#292) ever dropped a method, `Arc<HookLeaf>` would stop implementing
/// `TileSink` and this test would fail to compile.
#[test]
fn arc_wrapper_forwards_every_engine_hook() {
    let root = PathBuf::from("/tmp/libviprs-292-checkpoint");
    let leaf = Arc::new(HookLeaf::new(root.clone()));

    // `Arc<HookLeaf>: TileSink` is required by the generic bound here; before
    // #292 only `Box`/`&` forwarded, so this did not compile.
    drive_side_effecting_hooks(&leaf);

    // Value-returning hooks forward through the Arc wrapper without any
    // hand-written per-method forwarding on the caller side.
    assert_eq!(
        leaf.sink_retry_count(),
        7,
        "retry count must forward through Arc"
    );
    assert_eq!(
        leaf.sink_skipped_due_to_failure(),
        3,
        "skip count must forward through Arc"
    );
    assert_eq!(
        leaf.checkpoint_root(),
        Some(root.as_path()),
        "checkpoint root must forward through Arc — else resume breaks"
    );
    assert_eq!(
        leaf.content_format(),
        Some(TileFormat::Jpeg { quality: 80 }),
        "content format must forward through Arc — else resume plan-hash mixes formats"
    );
    assert!(
        leaf.applies_retry_policy(),
        "applies_retry_policy must forward through Arc — else the builder double-wraps"
    );

    // Side-effecting hooks reached the inner leaf through the Arc.
    assert_eq!(
        leaf.skips.load(Ordering::Relaxed),
        2,
        "note_sink_skipped must forward through Arc — else skip totals under-count"
    );
    assert_eq!(
        leaf.levels_seen.load(Ordering::Relaxed),
        5,
        "init_level_count must forward through Arc — else per-level counters mis-size"
    );
    assert!(
        leaf.config_seen.load(Ordering::Relaxed),
        "record_engine_config must forward through Arc — else the manifest loses settings"
    );
}
