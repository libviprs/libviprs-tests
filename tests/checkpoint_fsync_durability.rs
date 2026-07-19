//! Checkpoint durability barrier — builder path (issues #273 + #277).
//!
//! Issue #273 (P0, durability): the engine's periodic checkpoint
//! (`CheckpointState::flush`) fsyncs the segment log and header but, on the
//! **builder** resume path, never fsyncs the tile files it certifies. The only
//! tile-data fsync in the crate lived in the sink-side checkpoint writer
//! (`FsSink::write_resume_checkpoint`), reachable solely through the standalone
//! `FsSink::with_resume(true)` flag — a flag the documented
//! `EngineBuilder::with_resume` path never sets. So a builder-driven run with a
//! plain `FsSink` certifies tiles whose bytes are still only in the page cache:
//! a power loss / kernel panic after a checkpoint but before the OS flush loses
//! "checkpointed" tiles, and resume skips them forever.
//!
//! Issue #277 (architecture) is the reason #273 exists: two parallel checkpoint
//! writers (the engine's `CheckpointState` and the sink's own
//! `write_resume_checkpoint`) meant the fsync responsibility fell into the seam
//! between them. The combined fix deletes the sink-side writer and gives
//! `TileSink` a single durability barrier hook, `sync_pending`, that the
//! engine's (now sole) `CheckpointState` invokes **before** it certifies a
//! checkpoint delta.
//!
//! This test drives the BUILDER path (not a hand-constructed
//! `FsSink::with_resume(true)`) through a checkpointed run and asserts that the
//! durability barrier is invoked, and that at the moment of each barrier the
//! on-disk checkpoint has NOT yet certified the tiles that barrier is making
//! durable. An injected sink records that the durability hook fired before the
//! checkpoint advanced.
//!
//! RED against current core (`libviprs/libviprs` @ 6583ad5): there is no
//! `TileSink::sync_pending` durability barrier at all, so this file does not
//! compile — the exact shape of the defect (the builder path has no way to
//! fsync tiles before certifying them). GREEN once the combined #273 + #277 fix
//! lands the `sync_pending` hook and wires `CheckpointState::flush` to call it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use libviprs::resume::{JobCheckpoint, JobMetadata};
use libviprs::sink::{FsSink, SinkError, Tile, TileFormat, TileSink};
use libviprs::{EngineBuilder, Layout, PyramidPlanner, ResumePolicy};

mod common;
use common::fixtures::canonical_raster_scaled;

fn read_checkpoint(dir: &Path) -> Option<JobMetadata> {
    JobCheckpoint::load(dir).ok().flatten()
}

fn count_files(dir: &Path, ext: &str) -> usize {
    let mut n = 0;
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                n += count_files(&p, ext);
            } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
                n += 1;
            }
        }
    }
    n
}

/// A transparent wrapper around a real [`FsSink`] that observes the durability
/// barrier. Every engine bookkeeping hook (`checkpoint_root`,
/// `arm_durability_tracking`, `record_engine_config`, ...) reaches the inner
/// sink through [`TileSink::inner_sink`]; only [`TileSink::sync_pending`] is
/// intercepted, to record how many tiles the on-disk checkpoint has certified
/// at the instant the barrier runs, then forwarded to the inner sink so the
/// real tile-data fsync still happens.
struct BarrierProbeSink {
    inner: FsSink,
    root: PathBuf,
    /// For each `sync_pending` invocation, the number of coordinates the
    /// on-disk checkpoint had certified when the barrier ran.
    certified_at_barrier: Mutex<Vec<usize>>,
}

impl TileSink for BarrierProbeSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.inner.write_tile(tile)
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }

    fn inner_sink(&self) -> Option<&dyn TileSink> {
        Some(&self.inner)
    }

    fn sync_pending(&self) -> Result<(), SinkError> {
        let certified = read_checkpoint(&self.root)
            .map(|m| m.completed_tiles.len())
            .unwrap_or(0);
        self.certified_at_barrier.lock().unwrap().push(certified);
        // Forward so the inner FsSink actually fsyncs the tracked tile files.
        self.inner.sync_pending()
    }
}

/// The engine must invoke the `TileSink` durability barrier on the builder's
/// checkpoint path, and it must do so BEFORE it certifies the tiles that
/// barrier is making durable. Concretely: with `checkpoint_every(1)` a flush —
/// and therefore a barrier — fires for every completed tile, and the barrier
/// serialised with each flush runs before that flush appends its delta to the
/// segment log. So the first barrier must observe zero certified tiles, the
/// observed certified-counts never go backwards, and a barrier fires at least
/// once per certified tile.
///
/// Pre-fix the builder path never fsyncs tiles (no barrier hook exists at all),
/// so this cannot even be expressed — the file fails to compile against current
/// core. Post-fix the barrier fires and precedes certification.
#[test]
fn builder_checkpoint_fsyncs_tiles_before_certifying_them() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let total = plan.total_tile_count();

    let inner = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let probe = BarrierProbeSink {
        inner,
        root: base.clone(),
        certified_at_barrier: Mutex::new(Vec::new()),
    };

    // Overwrite (avoids the resume+dedupe stopgap) with the smallest possible
    // cadence so a checkpoint flush — and therefore a durability barrier —
    // fires on every tile.
    EngineBuilder::new(&src, plan.clone(), &probe)
        .with_resume(ResumePolicy::overwrite().with_checkpoint_every(1))
        .run()
        .expect("checkpointed builder run must succeed");

    let observed = probe.certified_at_barrier.lock().unwrap().clone();

    // (1) The durability barrier must have been invoked at all. This is the
    //     heart of #273: on the pre-fix builder path it never was.
    assert!(
        !observed.is_empty(),
        "the checkpoint durability barrier (TileSink::sync_pending) was never \
         invoked on the builder path — periodic checkpoints certify tiles whose \
         bytes were never fsynced (issue #273)"
    );

    // (2) The first barrier must run before the checkpoint certifies anything:
    //     tile data is made durable strictly before it is recorded complete.
    assert_eq!(
        observed[0], 0,
        "the first durability barrier must run before the checkpoint certifies \
         its first tile; observed certified-counts at each barrier: {observed:?}"
    );

    // (3) The certified-count observed at each barrier never goes backwards, and
    //     a barrier fires at least once per certified tile — the durability
    //     barrier keeps pace with (and precedes) every certification.
    assert!(
        observed.windows(2).all(|w| w[0] <= w[1]),
        "certified-count observed at successive barriers went backwards: \
         {observed:?}"
    );
    assert!(
        observed.len() as u64 >= total,
        "expected a durability barrier for every certified tile (>= {total}), \
         got {}: {observed:?}",
        observed.len()
    );

    // (4) The finished run is complete and internally consistent: every tile is
    //     on disk and the sole checkpoint certifies exactly those tiles.
    assert_eq!(
        count_files(&base, "raw") as u64,
        total,
        "every planned tile must be written to disk"
    );
    let meta = read_checkpoint(&base).expect("a checkpoint must exist after the run");
    assert_eq!(
        meta.completed_tiles.len() as u64,
        total,
        "the single checkpoint must certify every completed tile"
    );
}
