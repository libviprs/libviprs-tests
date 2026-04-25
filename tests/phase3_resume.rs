//! Phase 3 — Resumable pyramid generation (TDD).
//!
//! These are **failing** integration tests that pin down the on-disk /
//! in-memory contract for `libviprs::resume::*`. The module referenced here
//! does not yet exist in the core crate; the tests are authored first so
//! the implementation has a concrete target.
//!
//! Planned API under test:
//!
//! ```ignore
//! use libviprs::resume::{ResumeMode, JobMetadata, generate_pyramid_resumable};
//!
//! pub enum ResumeMode { Overwrite, Resume, Verify }
//!
//! pub struct JobMetadata {
//!     pub schema_version: &'static str,   // "1"
//!     pub plan_hash: String,              // blake3 of plan params
//!     pub completed_tiles: Vec<TileCoord>,
//!     pub levels_completed: Vec<u32>,
//!     pub started_at: String,
//!     pub last_checkpoint_at: String,
//! }
//!
//! pub fn run_resumable(
//!     source: &Raster,
//!     plan: &PyramidPlan,
//!     sink: &dyn TileSink,
//!     config: &EngineConfig,
//!     mode: ResumeMode,
//! ) -> Result<EngineResult, EngineError>;
//! ```
//!
//! The checkpoint file lives at `<output_dir>/.libviprs-job.json`.
//!
//! Test scenarios (10 total):
//!   1. `overwrite_mode_starts_fresh`
//!   2. `resume_mode_continues_after_partial_run`
//!   3. `resume_mode_refuses_if_plan_hash_differs`
//!   4. `verify_mode_passes_on_intact_output`
//!   5. `verify_mode_fails_on_missing_tile`
//!   6. `verify_mode_fails_on_corrupted_tile`
//!   7. `checkpoint_written_periodically`
//!   8. `checkpoint_file_survives_kill_and_resume`
//!   9. `resume_survives_process_boundary`
//!   10. `idempotent_resume_of_completed_job`

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use libviprs::resume::{JobMetadata, ResumeMode};
use libviprs::{
    EngineBuilder, EngineConfig, EngineError, EngineResult, FsSink, Layout, PixelFormat,
    PyramidPlan, PyramidPlanner, Raster, ResumePolicy, SinkError, Tile, TileCoord, TileFormat,
    TileSink,
};

/// Test shim: route the old `generate_pyramid_resumable(raster, plan, sink,
/// &cfg, mode)` call shape through the new `EngineBuilder::with_resume`
/// pathway. Keeps the bodies of existing tests mechanically small.
fn run_resumable<S: TileSink>(
    raster: &Raster,
    plan: &PyramidPlan,
    sink: S,
    cfg: &EngineConfig,
    mode: ResumeMode,
) -> Result<EngineResult, EngineError> {
    let policy = match mode {
        ResumeMode::Overwrite => ResumePolicy::overwrite(),
        ResumeMode::Resume => ResumePolicy::resume(),
        ResumeMode::Verify => ResumePolicy::verify(),
    };
    // Set the resume policy *before* with_config so the latter can thread
    // `checkpoint_every` / `checkpoint_root` from the incoming EngineConfig
    // into the policy (matching how these lived directly on the config in
    // the old `generate_pyramid_resumable` signature).
    EngineBuilder::new(raster, plan.clone(), sink)
        .with_resume(policy)
        .with_config(cfg.clone())
        .run()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The well-known filename for the on-disk checkpoint.
const JOB_FILE: &str = ".libviprs-job.json";

/// Synthetic gradient raster. Deterministic so two runs produce identical
/// tiles and Verify-mode comparisons are meaningful.
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

/// Build a small plan sized so that tile counts are comfortably above the
/// checkpoint cadence we test with (enough tiles for "every 10" to fire
/// multiple times).
///
/// For the larger (1024² / 512²) callers we deliberately ignore the
/// `tile_size` argument and dial it down to 64 px so the resulting pyramid
/// has the ~hundred-tile scale those tests' setup assertions require. The
/// 128² caller retains its native tile_size to keep the tight
/// `Resume`/`Verify` splits from that call deterministic.
fn small_plan(w: u32, h: u32, tile_size: u32) -> PyramidPlan {
    let ts = if w >= 512 || h >= 512 { 64 } else { tile_size };
    PyramidPlanner::new(w, h, ts, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Deserialise the job checkpoint from disk. Panics with a descriptive
/// message if missing or malformed — tests want loud failures here.
fn read_job_metadata(dir: &Path) -> JobMetadata {
    let path = dir.join(JOB_FILE);
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
    // The production type is expected to `impl Serialize + Deserialize`.
    // If it does not, this test file fails to compile — which is correct
    // TDD behaviour.
    serde_json::from_slice::<JobMetadata>(&bytes)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", path.display()))
}

/// Recursively list every regular file under `dir`, returning paths relative
/// to `dir`. Useful for asserting "no extra tiles" and for snapshotting
/// Verify-mode expected output.
fn list_tile_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(root: &Path, cur: &Path, out: &mut Vec<PathBuf>) {
        if !cur.is_dir() {
            return;
        }
        for entry in std::fs::read_dir(cur).unwrap() {
            let entry = entry.unwrap();
            let p = entry.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Run a pristine pyramid to a freshly created directory and return the
/// sink base path. Used to set up "already complete" state for Verify and
/// idempotent-resume tests.
fn run_fresh_pyramid(dir: &Path, src: &Raster, plan: &PyramidPlan) -> PathBuf {
    let base = dir.join("tiles");
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let config = EngineConfig::default();
    let _ = run_resumable(src, plan, &sink, &config, ResumeMode::Overwrite)
        .expect("fresh overwrite run should succeed");
    base
}

// ---------------------------------------------------------------------------
// Test sinks
// ---------------------------------------------------------------------------

/// Wraps an `FsSink` and counts every `write_tile` call so tests can assert
/// how many tiles a resumed engine actually wrote (as opposed to skipped).
struct RecordingSink {
    inner: FsSink,
    writes: AtomicUsize,
    written_coords: Mutex<Vec<TileCoord>>,
}

impl RecordingSink {
    fn new(inner: FsSink) -> Self {
        Self {
            inner,
            writes: AtomicUsize::new(0),
            written_coords: Mutex::new(Vec::new()),
        }
    }

    fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }

    fn written_coords(&self) -> Vec<TileCoord> {
        self.written_coords.lock().unwrap().clone()
    }
}

impl TileSink for RecordingSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.written_coords.lock().unwrap().push(tile.coord);
        self.inner.write_tile(tile)
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }

    fn checkpoint_root(&self) -> Option<&Path> {
        self.inner.checkpoint_root()
    }
}

/// Panics on the Nth `write_tile` call. Used to simulate an abrupt process
/// death mid-run so we can verify that the on-disk checkpoint is sufficient
/// to resume.
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
}

// ---------------------------------------------------------------------------
// 1. Overwrite mode removes any pre-existing output and regenerates.
// ---------------------------------------------------------------------------

#[test]
fn overwrite_mode_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    std::fs::create_dir_all(&base).unwrap();

    // Pre-populate with garbage that should not survive an Overwrite run.
    let garbage = base.join("garbage.raw");
    std::fs::write(&garbage, b"this should be removed").unwrap();
    let stale_subdir = base.join("stale");
    std::fs::create_dir_all(&stale_subdir).unwrap();
    std::fs::write(stale_subdir.join("stale.raw"), b"stale").unwrap();

    // Stale job metadata that should also be discarded.
    std::fs::write(
        base.join(JOB_FILE),
        br#"{"schema_version":"1","plan_hash":"deadbeef","completed_tiles":[],"levels_completed":[],"started_at":"old","last_checkpoint_at":"old"}"#,
    )
    .unwrap();

    let src = gradient_raster(128, 128);
    let plan = small_plan(128, 128, 64);
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);

    let result = run_resumable(
        &src,
        &plan,
        &sink,
        &EngineConfig::default(),
        ResumeMode::Overwrite,
    )
    .expect("overwrite run should succeed");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Garbage must be gone.
    assert!(
        !garbage.exists(),
        "Overwrite did not remove pre-existing garbage file"
    );
    assert!(
        !stale_subdir.exists(),
        "Overwrite did not remove pre-existing stale subdir"
    );

    // Fresh job metadata reflects the NEW plan, not the stale one.
    let meta = read_job_metadata(&base);
    assert_eq!(meta.schema_version, "1");
    assert_ne!(meta.plan_hash, "deadbeef", "Overwrite kept stale plan_hash");
    assert_eq!(
        meta.completed_tiles.len() as u64,
        plan.total_tile_count(),
        "completed_tiles should cover every planned tile"
    );
}

// ---------------------------------------------------------------------------
// 2. Resume mode skips tiles already recorded in the checkpoint.
// ---------------------------------------------------------------------------

#[test]
fn resume_mode_continues_after_partial_run() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    std::fs::create_dir_all(&base).unwrap();

    let src = gradient_raster(128, 128);
    let plan = small_plan(128, 128, 64);

    // Do a first full run so we have valid tile output on disk. We'll then
    // doctor the checkpoint to claim only half of the tiles are complete and
    // assert the resumed run writes exactly the remaining half.
    let first_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    run_resumable(
        &src,
        &plan,
        &first_sink,
        &EngineConfig::default(),
        ResumeMode::Overwrite,
    )
    .unwrap();

    let meta_full = read_job_metadata(&base);
    let total = meta_full.completed_tiles.len();
    assert!(
        total >= 4,
        "plan should have enough tiles for a meaningful split"
    );
    let half = total / 2;
    #[allow(clippy::type_complexity)]
    let (done_idx, remaining_idx): (Vec<(usize, &TileCoord)>, Vec<(usize, &TileCoord)>) = meta_full
        .completed_tiles
        .iter()
        .enumerate()
        .partition(|(i, _)| *i < half);
    let done: Vec<TileCoord> = done_idx.into_iter().map(|(_, c)| *c).collect();
    let remaining: Vec<TileCoord> = remaining_idx.into_iter().map(|(_, c)| *c).collect();

    // Hand-crafted partial checkpoint. Keep the same plan_hash so the engine
    // accepts it.
    let mut partial = JobMetadata::new(meta_full.plan_hash.clone(), "1970-01-01T00:00:00Z".into());
    partial.completed_tiles = done.clone();
    partial.last_checkpoint_at = "1970-01-01T00:00:00Z".into();
    std::fs::write(base.join(JOB_FILE), serde_json::to_vec(&partial).unwrap()).unwrap();

    // Delete the tile files that our fake checkpoint claims are "missing"
    // so the resumed engine has real work to do.
    for coord in &remaining {
        let rel = plan.tile_path(*coord, "raw").unwrap();
        let p = base.join(rel);
        if p.exists() {
            std::fs::remove_file(&p).unwrap();
        }
    }

    // Resumed run — wrap FsSink in a RecordingSink to count writes.
    let recording =
        RecordingSink::new(FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw));
    let result = run_resumable(
        &src,
        &plan,
        &recording,
        &EngineConfig::default(),
        ResumeMode::Resume,
    )
    .expect("resume run should succeed");

    assert_eq!(
        recording.write_count(),
        remaining.len(),
        "Resume should only write the uncompleted tiles"
    );
    let mut actual = recording.written_coords();
    actual.sort_by_key(|c| (c.level, c.row, c.col));
    let mut expected = remaining.clone();
    expected.sort_by_key(|c| (c.level, c.row, c.col));
    assert_eq!(
        actual, expected,
        "Resumed tiles do not match the expected remainder"
    );

    // Final checkpoint should now cover the full plan.
    let meta_final = read_job_metadata(&base);
    assert_eq!(
        meta_final.completed_tiles.len() as u64,
        plan.total_tile_count()
    );
    assert_eq!(result.tiles_produced, remaining.len() as u64);
}

// ---------------------------------------------------------------------------
// 3. A checkpoint whose plan_hash disagrees with the current plan must
//    refuse to resume rather than silently produce incoherent output.
// ---------------------------------------------------------------------------

#[test]
fn resume_mode_refuses_if_plan_hash_differs() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    std::fs::create_dir_all(&base).unwrap();

    let src = gradient_raster(128, 128);
    let plan = small_plan(128, 128, 64);

    // Pre-existing checkpoint with an obviously wrong hash.
    let stale = JobMetadata::new(
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
        "1970-01-01T00:00:00Z".into(),
    );
    std::fs::write(base.join(JOB_FILE), serde_json::to_vec(&stale).unwrap()).unwrap();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let err = run_resumable(
        &src,
        &plan,
        &sink,
        &EngineConfig::default(),
        ResumeMode::Resume,
    )
    .expect_err("Resume must fail when plan_hash disagrees");

    // We don't pin the exact variant name here — the implementer may call it
    // `PlanHashMismatch`, `CheckpointMismatch`, `IncompatiblePlan`, etc.
    // Whatever it is, the Display output must make the problem obvious.
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("plan")
            || msg.contains("hash")
            || msg.contains("mismatch")
            || msg.contains("checkpoint"),
        "EngineError does not describe a plan/hash/checkpoint mismatch: {err}"
    );

    // Sanity: no tiles were produced as a side-effect of the failure.
    assert!(
        list_tile_files(&base)
            .iter()
            .all(|p| p.file_name().unwrap() == JOB_FILE),
        "Failed Resume should not write tile files"
    );

    // Leave err un-dropped so the cast below is a compile-time check:
    // EngineError must remain the declared error type.
    let _: EngineError = err;
}

// ---------------------------------------------------------------------------
// 4. Verify mode passes when the on-disk pyramid matches the plan.
// ---------------------------------------------------------------------------

#[test]
fn verify_mode_passes_on_intact_output() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(128, 128);
    let plan = small_plan(128, 128, 64);
    let base = run_fresh_pyramid(dir.path(), &src, &plan);

    // Re-run in Verify mode. MUST NOT touch any output files.
    let before = list_tile_files(&base);
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let result = run_resumable(
        &src,
        &plan,
        &sink,
        &EngineConfig::default(),
        ResumeMode::Verify,
    )
    .expect("Verify should succeed on intact output");
    let after = list_tile_files(&base);

    assert_eq!(before, after, "Verify mutated the output directory");
    assert_eq!(
        result.tiles_produced, 0,
        "Verify must not 'produce' tiles — it only checks"
    );
}

// ---------------------------------------------------------------------------
// 5. Verify fails when a tile is missing.
// ---------------------------------------------------------------------------

#[test]
fn verify_mode_fails_on_missing_tile() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(128, 128);
    let plan = small_plan(128, 128, 64);
    let base = run_fresh_pyramid(dir.path(), &src, &plan);

    // Delete a single tile.
    let victim = plan.tile_coords().next().unwrap();
    let rel = plan.tile_path(victim, "raw").unwrap();
    std::fs::remove_file(base.join(&rel)).unwrap();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let err = run_resumable(
        &src,
        &plan,
        &sink,
        &EngineConfig::default(),
        ResumeMode::Verify,
    )
    .expect_err("Verify must fail when a tile is missing");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("missing") || msg.contains("not found") || msg.contains("verify"),
        "Verify error should mention the missing tile: {err}"
    );
}

// ---------------------------------------------------------------------------
// 6. Verify fails when a tile is present but corrupted.
// ---------------------------------------------------------------------------

#[test]
fn verify_mode_fails_on_corrupted_tile() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(128, 128);
    let plan = small_plan(128, 128, 64);
    let base = run_fresh_pyramid(dir.path(), &src, &plan);

    // Flip one byte in one tile file.
    let victim = plan.tile_coords().next().unwrap();
    let rel = plan.tile_path(victim, "raw").unwrap();
    let path = base.join(&rel);
    let mut bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.is_empty(), "victim tile unexpectedly empty");
    bytes[0] ^= 0xFF;
    std::fs::write(&path, &bytes).unwrap();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let err = run_resumable(
        &src,
        &plan,
        &sink,
        &EngineConfig::default(),
        ResumeMode::Verify,
    )
    .expect_err("Verify must fail on byte-corrupted tile");
    let msg = format!("{err}").to_lowercase();
    assert!(
        msg.contains("corrupt")
            || msg.contains("checksum")
            || msg.contains("mismatch")
            || msg.contains("verify"),
        "Verify error should describe corruption/checksum mismatch: {err}"
    );
}

// ---------------------------------------------------------------------------
// 7. The checkpoint is updated roughly every `checkpoint_every` tiles.
// ---------------------------------------------------------------------------

#[test]
fn checkpoint_written_periodically() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    std::fs::create_dir_all(&base).unwrap();

    // Choose a plan that yields at least ~100 tiles so that a cadence of 10
    // fires roughly 10 times.
    let src = gradient_raster(1024, 1024);
    let plan = small_plan(1024, 1024, 128);
    assert!(plan.total_tile_count() >= 100, "test setup too small");

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    // `with_checkpoint_every` is a new builder method on EngineConfig.
    let config = EngineConfig::default().with_checkpoint_every(10);

    // Intercept checkpoint writes by polling the completed_tiles count of
    // the on-disk job file via a background thread while the engine runs.
    // Simpler and more robust: parse the final checkpoint AND ensure the
    // engine wrote an intermediate one (not just the final flush).
    //
    // We approximate "how many checkpoint updates happened" by installing a
    // file-change counter: after the run, read the file to ensure its shape
    // is valid and its completed_tiles equals the plan. The intermediate
    // count is asserted indirectly: we expect `total_tile_count / 10`
    // checkpoints to have been flushed during the run, and at minimum the
    // final checkpoint contains every tile.
    //
    // To catch the "only one final checkpoint ever got written" regression,
    // we watch the file's mtime by polling from a scratch thread.
    let observed_updates = std::sync::Arc::new(AtomicUsize::new(0));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let base_for_thread = base.clone();
    let updates_for_thread = observed_updates.clone();
    let stop_for_thread = stop.clone();
    let watcher = std::thread::spawn(move || {
        let mut last: Option<std::time::SystemTime> = None;
        while !stop_for_thread.load(Ordering::SeqCst) {
            if let Ok(md) = std::fs::metadata(base_for_thread.join(JOB_FILE)) {
                if let Ok(mt) = md.modified() {
                    if last != Some(mt) {
                        last = Some(mt);
                        updates_for_thread.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    run_resumable(&src, &plan, &sink, &config, ResumeMode::Overwrite).unwrap();
    stop.store(true, Ordering::SeqCst);
    watcher.join().unwrap();

    // Mtime-based polling undercounts when consecutive writes land within a
    // single filesystem mtime tick — severe on virtiofs/9p (Docker on macOS),
    // where 30+ flushes can collapse to ~13 observed ticks. The regression we
    // actually care about is "only the final flush happened" (count == 1), so
    // pin the floor at a small constant well above 1 but well below the ideal.
    let ideal = (plan.total_tile_count() / 10) as usize;
    let expected_min = 3;
    assert!(
        observed_updates.load(Ordering::SeqCst) >= expected_min,
        "Expected at least {expected_min} checkpoint updates (ideal {ideal}), observed {}",
        observed_updates.load(Ordering::SeqCst)
    );

    // Final checkpoint is complete and well-formed.
    let meta = read_job_metadata(&base);
    assert_eq!(meta.completed_tiles.len() as u64, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// 8. A mid-run crash leaves a usable checkpoint; Resume completes the job.
// ---------------------------------------------------------------------------

#[test]
fn checkpoint_file_survives_kill_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    std::fs::create_dir_all(&base).unwrap();

    let src = gradient_raster(512, 512);
    let plan = small_plan(512, 512, 128);
    let total = plan.total_tile_count();
    assert!(total > 50, "need a plan large enough for a mid-run panic");

    // Panic on the 50th tile write. The engine should propagate the panic
    // (wrapped as WorkerPanic or via unwind), but the on-disk checkpoint
    // must reflect the tiles that were successfully flushed before the kill.
    let panicking = PanickingSink::new(
        FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw),
        50,
    );
    // Use catch_unwind so we can observe the crash without aborting the
    // whole test binary. Single-threaded execution (concurrency=0) keeps the
    // panic deterministic with respect to the 50th write.
    let config = EngineConfig::default().with_checkpoint_every(10);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_resumable(&src, &plan, &panicking, &config, ResumeMode::Overwrite)
    }));
    assert!(
        crashed.is_err() || matches!(crashed, Ok(Err(_))),
        "PanickingSink should have aborted the run"
    );

    // Checkpoint exists and records strictly fewer than all tiles.
    let meta = read_job_metadata(&base);
    assert!(
        (meta.completed_tiles.len() as u64) < total,
        "Checkpoint should be partial after crash, got {}/{total}",
        meta.completed_tiles.len()
    );
    assert!(
        !meta.completed_tiles.is_empty(),
        "Checkpoint cadence of 10 should have flushed at least once before tile 50"
    );

    let already = meta.completed_tiles.len();

    // Resume with a recording sink to count the remaining writes.
    let recording =
        RecordingSink::new(FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw));
    run_resumable(
        &src,
        &plan,
        &recording,
        &EngineConfig::default(),
        ResumeMode::Resume,
    )
    .expect("Resume after crash should succeed");

    assert_eq!(
        recording.write_count() as u64 + already as u64,
        total,
        "Resume wrote {} tiles; already {} completed; expected total {total}",
        recording.write_count(),
        already,
    );

    // Final checkpoint covers the full plan.
    let final_meta = read_job_metadata(&base);
    assert_eq!(final_meta.completed_tiles.len() as u64, total);
}

// ---------------------------------------------------------------------------
// 9. Resume only needs the on-disk job.json — no in-memory engine state.
//    We simulate a fresh process by dropping every engine-related value and
//    constructing brand-new instances from the filesystem.
// ---------------------------------------------------------------------------

#[test]
fn resume_survives_process_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    std::fs::create_dir_all(&base).unwrap();
    let src = gradient_raster(256, 256);
    let plan = small_plan(256, 256, 64);

    // First "process": panic mid-run so we have a partial checkpoint.
    let total = plan.total_tile_count();
    assert!(total > 20);
    {
        let panicking = PanickingSink::new(
            FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw),
            (total / 2) as usize,
        );
        let config = EngineConfig::default().with_checkpoint_every(5);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Intentionally ignore the Result; the catch_unwind boundary is
            // what we actually care about here.
            let _ = run_resumable(&src, &plan, &panicking, &config, ResumeMode::Overwrite);
        }));
        // Deliberately drop every engine value here. The only surviving
        // state is the tiles already on disk plus the checkpoint JSON.
    }

    // Second "process": reconstruct plan + sink + config from scratch and
    // verify Resume completes the job using only the on-disk checkpoint.
    let meta_before = read_job_metadata(&base);
    let before_done = meta_before.completed_tiles.len();
    assert!(before_done > 0 && (before_done as u64) < total);

    let plan2 = small_plan(256, 256, 64);
    assert_eq!(plan, plan2, "plan reconstruction must be deterministic");
    let src2 = gradient_raster(256, 256);
    let sink2 = FsSink::new(base.clone(), plan2.clone()).with_format(TileFormat::Raw);
    run_resumable(
        &src2,
        &plan2,
        &sink2,
        &EngineConfig::default(),
        ResumeMode::Resume,
    )
    .expect("second-process Resume should succeed");

    let meta_after = read_job_metadata(&base);
    assert_eq!(
        meta_after.completed_tiles.len() as u64,
        total,
        "second-process Resume did not finish the pyramid"
    );
}

// ---------------------------------------------------------------------------
// 10. Resume on an already-complete job is a no-op (no tiles re-written).
// ---------------------------------------------------------------------------

#[test]
fn idempotent_resume_of_completed_job() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(128, 128);
    let plan = small_plan(128, 128, 64);
    let base = run_fresh_pyramid(dir.path(), &src, &plan);

    // Second Resume run wrapped in a recording sink: the count MUST be 0.
    let recording =
        RecordingSink::new(FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw));
    let result = run_resumable(
        &src,
        &plan,
        &recording,
        &EngineConfig::default(),
        ResumeMode::Resume,
    )
    .expect("Resume on a finished job should be Ok");

    assert_eq!(
        recording.write_count(),
        0,
        "Idempotent Resume must not rewrite any tiles"
    );
    assert_eq!(result.tiles_produced, 0);

    // Checkpoint is still fully populated and unchanged in shape.
    let meta = read_job_metadata(&base);
    assert_eq!(meta.completed_tiles.len() as u64, plan.total_tile_count());
}

// Suppress dead-code warnings for helpers that may not be reached if an
// earlier test fails to compile against the (yet-unimplemented) API.
#[allow(dead_code)]
fn _unused_helpers_touch() {
    let _ = list_tile_files;
    let _ = read_job_metadata;
}
