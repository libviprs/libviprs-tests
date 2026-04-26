//! Observer-event parity across every (EngineKind × ResumeMode) combination.
//!
//! The builder's resume path is driven by an `Arc<dyn EngineObserver>`
//! attached via [`EngineBuilder::with_observer_arc`]. Every engine /
//! resume-mode combination must emit a consistent baseline event stream:
//!
//! * `LevelStarted`  — exactly once per level.
//! * `LevelCompleted` — exactly once per level.
//! * `Finished`      — exactly once per run.
//! * `TileCompleted` — once per non-skipped tile.
//!
//! These tests pin that contract so regressions in any engine's observer
//! wiring are caught by a uniform set of counters rather than
//! engine-specific assertions scattered across other files.
//!
//! The Verify-mode coverage for Streaming and MapReduce is gated on the
//! stream-verify implementation that the builder currently rejects with
//! `EngineError::IncompatibleSource`. Those two tests (`streaming_verify_*`
//! and `mapreduce_verify_*`) are marked `#[ignore]` until the feature lands;
//! their bodies express the final contract so the implementer has a
//! concrete assertion target.

#![cfg(feature = "builder_v1")]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use libviprs::resume::JobMetadata;
use libviprs::sink::{FsSink, SinkError, Tile, TileFormat, TileSink};
use libviprs::{
    CollectingObserver, EngineBuilder, EngineConfig, EngineEvent, EngineKind, Layout, PyramidPlan,
    PyramidPlanner, Raster, ResumePolicy, TileCoord,
};

mod common;
use common::fixtures::canonical_raster_scaled;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// Build a modest plan large enough that tile counts are meaningful but
/// small enough that every engine finishes promptly in CI.
fn small_plan(w: u32, h: u32, tile_size: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, tile_size, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Convenience counter bundle used by most tests. Extracting this keeps the
/// individual test bodies focused on the (EngineKind, ResumeMode) axes
/// rather than on event-filtering boilerplate.
struct EventCounts {
    level_started: usize,
    level_completed: usize,
    tile_completed: usize,
    finished: usize,
}

impl EventCounts {
    fn from_events(events: &[EngineEvent]) -> Self {
        Self {
            level_started: events
                .iter()
                .filter(|e| matches!(e, EngineEvent::LevelStarted { .. }))
                .count(),
            level_completed: events
                .iter()
                .filter(|e| matches!(e, EngineEvent::LevelCompleted { .. }))
                .count(),
            tile_completed: events
                .iter()
                .filter(|e| matches!(e, EngineEvent::TileCompleted { .. }))
                .count(),
            finished: events
                .iter()
                .filter(|e| matches!(e, EngineEvent::Finished { .. }))
                .count(),
        }
    }
}

/// Full-pyramid baseline: one LevelStarted + one LevelCompleted per level,
/// one Finished, one TileCompleted per planned tile. Shared across every
/// "full pyramid" variant so a single helper pins the invariant.
fn assert_full_stream(counts: &EventCounts, plan: &PyramidPlan) {
    assert_eq!(
        counts.level_started,
        plan.level_count(),
        "LevelStarted count ({}) does not match level count ({})",
        counts.level_started,
        plan.level_count(),
    );
    assert_eq!(
        counts.level_completed,
        plan.level_count(),
        "LevelCompleted count ({}) does not match level count ({})",
        counts.level_completed,
        plan.level_count(),
    );
    assert_eq!(
        counts.tile_completed as u64,
        plan.total_tile_count(),
        "TileCompleted count ({}) does not match total tile count ({})",
        counts.tile_completed,
        plan.total_tile_count(),
    );
    assert_eq!(
        counts.finished, 1,
        "Finished must fire exactly once, got {}",
        counts.finished
    );
}

// ---------------------------------------------------------------------------
// Test sinks
// ---------------------------------------------------------------------------

/// Panics on the Nth `write_tile` call. Mirrors the helper in
/// `tests/phase3_resume.rs`; copied verbatim so this file stays
/// self-contained (cross-file test-helper sharing is not set up in this
/// crate).
struct PanickingSink {
    inner: FsSink,
    writes: AtomicUsize,
    panic_on: usize,
}

impl PanickingSink {
    fn new(inner: FsSink, panic_on: usize) -> Self {
        Self {
            inner,
            writes: AtomicUsize::new(0),
            panic_on,
        }
    }
}

impl TileSink for PanickingSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        let n = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.panic_on {
            panic!(
                "PanickingSink: simulated crash on tile #{n} ({:?})",
                tile.coord
            );
        }
        self.inner.write_tile(tile)
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }

    fn checkpoint_root(&self) -> Option<&Path> {
        self.inner.checkpoint_root()
    }

    fn record_engine_config(&self, config: &EngineConfig) {
        self.inner.record_engine_config(config)
    }

    fn init_level_count(&self, levels: usize) {
        self.inner.init_level_count(levels)
    }
}

/// Read the persisted checkpoint as raw `JobMetadata`. Panics loudly if
/// the file is missing or unparsable — the test that uses this needs a
/// partial checkpoint to exist, so absence is a real bug.
fn read_job_metadata(dir: &Path) -> JobMetadata {
    let path = dir.join(".libviprs-job.json");
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    serde_json::from_slice::<JobMetadata>(&bytes)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// 1. Monolithic + Overwrite — full event stream.
// ---------------------------------------------------------------------------

#[test]
fn monolithic_overwrite_emits_full_event_stream() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = small_plan(256, 192, 64);

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .with_observer_arc(obs.clone())
        .run()
        .expect("monolithic overwrite should succeed");

    let counts = EventCounts::from_events(&obs.events());
    assert_full_stream(&counts, &plan);
}

// ---------------------------------------------------------------------------
// 2. Monolithic + Resume — only new writes emit events.
// ---------------------------------------------------------------------------

// Ignored: captures the pre-unification semantic where Monolithic resume
// suppressed TileCompleted events for skipped tiles. The unified resume
// path (EngineBuilder + ResumeAwareSink) now fires progress events for
// every tile the engine visits regardless of whether the wrapper
// short-circuits the actual write — sink writes and observer events are
// separate concerns. Kept as a ratchet: if we decide later to re-introduce
// skip-aware event suppression, un-ignore this test and flip the wiring.
#[test]
#[ignore = "resolved semantic: observer fires for every engine-visited tile, not per sink write"]
fn monolithic_resume_emits_events_for_new_writes_only() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    std::fs::create_dir_all(&base).unwrap();
    let src = canonical_raster_scaled(256, 256);
    let plan = small_plan(256, 256, 64);
    let total = plan.total_tile_count();
    assert!(
        total > 20,
        "plan should be large enough for a meaningful mid-run panic"
    );

    // First run: crash mid-way so the on-disk checkpoint records a strict
    // subset of completed tiles. Concurrency=1 keeps the panic offset
    // deterministic with respect to the Nth write.
    let panic_on = (total / 2) as usize;
    let panicking = PanickingSink::new(
        FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw),
        panic_on,
    );
    let config = EngineConfig::default().with_checkpoint_every(5);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = EngineBuilder::new(&src, plan.clone(), &panicking)
            .with_engine(EngineKind::Monolithic)
            .with_concurrency(1)
            .with_resume(ResumePolicy::overwrite())
            .with_config(config)
            .run();
    }));

    let partial = read_job_metadata(&base);
    let already_done = partial.completed_tiles.len();
    assert!(
        already_done > 0 && (already_done as u64) < total,
        "expected partial checkpoint, got {already_done}/{total}"
    );
    let remaining = total - already_done as u64;

    // Second run: fresh observer, Resume mode. Only the uncompleted tiles
    // should produce TileCompleted events; level and finish events still
    // fire for every level / the single Finished.
    let inner = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());
    EngineBuilder::new(&src, plan.clone(), &inner)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume())
        .with_observer_arc(obs.clone())
        .run()
        .expect("monolithic resume should succeed");

    let counts = EventCounts::from_events(&obs.events());
    assert_eq!(
        counts.tile_completed as u64, remaining,
        "Resume should emit TileCompleted only for the {remaining} unfinished tiles \
         (already_done={already_done}, total={total}), got {}",
        counts.tile_completed,
    );
    assert_eq!(counts.finished, 1, "Finished must fire exactly once");
    assert_eq!(
        counts.level_started,
        plan.level_count(),
        "LevelStarted should fire once per level"
    );
    assert_eq!(
        counts.level_completed,
        plan.level_count(),
        "LevelCompleted should fire once per level"
    );
}

// ---------------------------------------------------------------------------
// 3. Monolithic + Verify — one event per tile (Verify walks every tile).
// ---------------------------------------------------------------------------

#[test]
fn monolithic_verify_emits_events_for_every_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = small_plan(256, 192, 64);

    // Seed: produce a complete, intact pyramid so Verify has something to
    // walk.
    let seed_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &seed_sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("seed run should succeed");

    // Verify run with a fresh observer.
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());
    EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::verify())
        .with_observer_arc(obs.clone())
        .run()
        .expect("verify should succeed against intact output");

    let counts = EventCounts::from_events(&obs.events());
    assert_eq!(
        counts.tile_completed as u64,
        plan.total_tile_count(),
        "Verify walks every tile — TileCompleted should fire {} times, got {}",
        plan.total_tile_count(),
        counts.tile_completed,
    );
    assert_eq!(counts.finished, 1);
    assert_eq!(counts.level_started, plan.level_count());
    assert_eq!(counts.level_completed, plan.level_count());
}

// ---------------------------------------------------------------------------
// 4. Streaming + Overwrite — full event stream.
// ---------------------------------------------------------------------------

#[test]
fn streaming_overwrite_emits_full_event_stream() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = small_plan(256, 192, 64);

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .with_observer_arc(obs.clone())
        .run()
        .expect("streaming overwrite should succeed");

    let counts = EventCounts::from_events(&obs.events());
    assert_full_stream(&counts, &plan);
}

// ---------------------------------------------------------------------------
// 5. Streaming + Resume after a full run — no new tile events.
// ---------------------------------------------------------------------------

// Ignored: same reason as monolithic_resume_emits_events_for_new_writes_only.
// The streaming engine emits TileCompleted for every tile it renders;
// ResumeAwareSink only filters write_tile, not observer events. The
// regression guards live in builder_resume_streaming.rs
// (streaming_resume_short_circuits_already_complete_tiles) which counts
// sink writes rather than observer events.
#[test]
#[ignore = "resolved semantic: observer fires for every engine-visited tile, not per sink write"]
fn streaming_resume_emits_no_tile_events_when_complete() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = small_plan(256, 192, 64);

    // First run: produce the full pyramid + full checkpoint.
    {
        let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::Streaming)
            .with_resume(ResumePolicy::overwrite())
            .run()
            .expect("seed streaming run should succeed");
    }

    // Second run: Resume against a complete checkpoint. Every tile is
    // short-circuited by ResumeAwareSink, so no TileCompleted events
    // should fire — but Finished and the per-level bookends still do.
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::resume())
        .with_observer_arc(obs.clone())
        .run()
        .expect("streaming resume on complete checkpoint should succeed");

    let counts = EventCounts::from_events(&obs.events());
    assert_eq!(
        counts.tile_completed, 0,
        "Fully short-circuited Resume must not emit TileCompleted, got {}",
        counts.tile_completed
    );
    assert_eq!(
        counts.finished, 1,
        "Finished must still fire once on a no-op Resume"
    );
    assert_eq!(
        counts.level_started,
        plan.level_count(),
        "LevelStarted should still fire once per level on Resume"
    );
    assert_eq!(
        counts.level_completed,
        plan.level_count(),
        "LevelCompleted should still fire once per level on Resume"
    );
}

// ---------------------------------------------------------------------------
// 6. Streaming + Verify — one event per tile.
//
// Pending stream-verify implementation. Currently the builder rejects this
// combination with `EngineError::IncompatibleSource` (see
// `streaming_verify_is_rejected` in `builder_resume_streaming.rs`). This
// test expresses the target contract for when stream-verify lands.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending stream-verify implementation; currently IncompatibleSource"]
fn streaming_verify_emits_events_for_every_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = small_plan(256, 192, 64);

    // Seed a complete pyramid using the Streaming engine so Verify has
    // something to walk.
    {
        let seed = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        EngineBuilder::new(&src, plan.clone(), &seed)
            .with_engine(EngineKind::Streaming)
            .with_resume(ResumePolicy::overwrite())
            .run()
            .expect("seed streaming run should succeed");
    }

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .with_observer_arc(obs.clone())
        .run()
        .expect("streaming verify should succeed once implemented");

    let counts = EventCounts::from_events(&obs.events());
    assert_eq!(
        counts.tile_completed as u64,
        plan.total_tile_count(),
        "Streaming Verify must walk every tile"
    );
    assert_eq!(counts.finished, 1);
    assert_eq!(counts.level_started, plan.level_count());
    assert_eq!(counts.level_completed, plan.level_count());
}

// ---------------------------------------------------------------------------
// 7. MapReduce + Overwrite — full event stream.
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_overwrite_emits_full_event_stream() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = small_plan(256, 192, 64);

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .with_observer_arc(obs.clone())
        .run()
        .expect("mapreduce overwrite should succeed");

    let counts = EventCounts::from_events(&obs.events());
    assert_full_stream(&counts, &plan);
}

// ---------------------------------------------------------------------------
// 8. MapReduce + Verify — one event per tile.
//
// Pending stream-verify implementation for MapReduce. Currently rejected
// with `EngineError::IncompatibleSource`; this test expresses the target
// contract.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "pending stream-verify implementation; currently IncompatibleSource"]
fn mapreduce_verify_emits_events_for_every_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = small_plan(256, 192, 64);

    // Seed a complete pyramid using the MapReduce engine.
    {
        let seed = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        EngineBuilder::new(&src, plan.clone(), &seed)
            .with_engine(EngineKind::MapReduce)
            .with_resume(ResumePolicy::overwrite())
            .run()
            .expect("seed mapreduce run should succeed");
    }

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let obs = Arc::new(CollectingObserver::new());
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .with_observer_arc(obs.clone())
        .run()
        .expect("mapreduce verify should succeed once implemented");

    let counts = EventCounts::from_events(&obs.events());
    assert_eq!(
        counts.tile_completed as u64,
        plan.total_tile_count(),
        "MapReduce Verify must walk every tile"
    );
    assert_eq!(counts.finished, 1);
    assert_eq!(counts.level_started, plan.level_count());
    assert_eq!(counts.level_completed, plan.level_count());
}

// Suppress dead-code warnings if a future refactor stops using one of the
// helpers from an earlier test.
#[allow(dead_code)]
fn _unused_helpers_touch() {
    let _ = read_job_metadata;
    let _: fn(&[EngineEvent]) -> EventCounts = EventCounts::from_events;
    let _: fn(&EventCounts, &PyramidPlan) = assert_full_stream;
    let _: fn(u32, u32) -> Raster = canonical_raster_scaled;
    let _: fn(u32, u32, u32) -> PyramidPlan = small_plan;
    // Silence unused-import-via-dead-code for TileCoord when tests evolve.
    let _: fn() -> Option<TileCoord> = || None;
}
