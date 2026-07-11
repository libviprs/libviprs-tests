//! MapReduce + Verify integration tests (TDD).
//!
//! These tests pin down the **not-yet-existent** behaviour of pairing
//! `EngineBuilder::with_engine(EngineKind::MapReduce)` with
//! `ResumePolicy::verify()`. The sibling `builder_resume_streaming.rs`
//! file currently asserts that Verify is *rejected* for MapReduce with an
//! `IncompatibleSource` error (because verify walks every level via
//! `downscale_half`, which the MapReduce engine does not perform itself).
//!
//! The tests here encode the target surface after that restriction is
//! lifted: Verify becomes a read-only audit pass that re-derives every
//! level from the source raster (same as the Monolithic Verify
//! implementation does today) and compares against what the MapReduce
//! run wrote to disk. Until the builder actually wires this up these
//! tests are expected to fail, which is the TDD contract.
//!
//! Covered scenarios:
//!
//! 1. `mapreduce_verify_passes_on_intact_output` — Overwrite produce
//!    followed by Verify returns Ok without touching the output tree.
//! 2. `mapreduce_verify_accepts_strip_source` — same, but with
//!    `RasterStripSource` as the source (MapReduce's native input shape).
//! 3. `mapreduce_verify_detects_missing_tile` — deleting a tile surfaces
//!    `EngineError::Sink(SinkError::Other("Verify: missing tile …"))`.
//! 4. `mapreduce_verify_detects_corrupt_raw_tile` — flipping a byte in a
//!    raw tile surfaces `EngineError::ChecksumMismatch`.
//! 5. `mapreduce_verify_rejects_plan_hash_mismatch` — seeding the
//!    directory with plan A's checkpoint and re-running Verify with
//!    plan B surfaces `EngineError::PlanHashMismatch`.
//! 6. `mapreduce_verify_emits_observer_events` — observer receives the
//!    full `LevelStarted` / `TileCompleted` / `LevelCompleted` / `Finished`
//!    sequence during Verify.
//! 7. `mapreduce_verify_honours_manifest_checksums` — when the sink
//!    emitted a Blake3 manifest, corrupting a tile is caught via the
//!    checksum branch rather than the byte-exact branch.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use libviprs::checksum::{ChecksumAlgo, ChecksumMode};
use libviprs::manifest::ManifestBuilder;
use libviprs::sink::{FsSink, TileFormat};
use libviprs::streaming::RasterStripSource;
use libviprs::{
    CollectingObserver, EngineBuilder, EngineError, EngineEvent, EngineKind, Layout,
    PyramidPlanner, ResumePolicy, SinkError,
};

mod common;
use common::fixtures::canonical_raster_scaled;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// Recursively enumerate every regular file under `dir` with the given
/// extension. Used to assert that a Verify pass did not mutate the
/// output tree.
fn list_tile_paths(dir: &Path, ext: &str) -> Vec<std::path::PathBuf> {
    fn walk(cur: &Path, ext: &str, out: &mut Vec<std::path::PathBuf>) {
        if !cur.is_dir() {
            return;
        }
        for entry in std::fs::read_dir(cur).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                walk(&p, ext, out);
            } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
                out.push(p);
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, ext, &mut v);
    v.sort();
    v
}

/// Flip one byte in `path`. For raw tiles we clobber index 0; for larger
/// encoded payloads we pick offset 12 to skip past format magic. The
/// file length is preserved so the Verify path reads a same-size but
/// semantically different buffer.
fn corrupt_file(path: &Path) {
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("open {} for corruption: {e}", path.display()));
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    assert!(
        !buf.is_empty(),
        "cannot corrupt empty file {}",
        path.display()
    );
    let off = if buf.len() < 16 { 0 } else { 12 };
    buf[off] ^= 0xFF;
    f.seek(SeekFrom::Start(0)).unwrap();
    f.write_all(&buf).unwrap();
    f.set_len(buf.len() as u64).unwrap();
}

// ---------------------------------------------------------------------------
// 1. Verify passes on an intact MapReduce output tree.
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_verify_passes_on_intact_output() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Produce the pyramid under Overwrite so the directory is populated
    // and the checkpoint on-disk matches the current plan.
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("MapReduce + Overwrite should succeed");

    // Snapshot the tile tree so we can prove Verify is read-only.
    let before = list_tile_paths(&base, "raw");
    assert_eq!(
        before.len() as u64,
        plan.total_tile_count(),
        "produce run did not write the expected number of tiles"
    );

    // Re-run in Verify mode. Must succeed and must not mutate the tree.
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("Verify should succeed on intact output");

    let after = list_tile_paths(&base, "raw");
    assert_eq!(before, after, "Verify mutated the output directory");
    assert_eq!(
        result.tiles_produced, 0,
        "Verify must not 'produce' tiles — it only checks"
    );
}

// ---------------------------------------------------------------------------
// 2. Verify also accepts `RasterStripSource` (MapReduce's native shape).
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_verify_accepts_strip_source() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Produce via strip-source, which is the real-world feed for
    // MapReduce runs (no in-memory raster).
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(RasterStripSource::new(&src), plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("MapReduce + Overwrite (StripSource) should succeed");

    let before = list_tile_paths(&base, "raw");

    // Verify through a fresh strip source must likewise succeed.
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(RasterStripSource::new(&src), plan.clone(), &verify_sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("Verify (StripSource) should succeed on intact output");

    let after = list_tile_paths(&base, "raw");
    assert_eq!(before, after, "Verify mutated the output directory");
    assert_eq!(result.tiles_produced, 0);
}

// ---------------------------------------------------------------------------
// 3. Verify fails with a typed Sink(Other) error on a missing tile.
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_verify_detects_missing_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Remove a tile so Verify has something concrete to complain about.
    let victim = plan.tile_coords().next().unwrap();
    let rel = plan.tile_path(victim, "raw").unwrap();
    let path = base.join(&rel);
    std::fs::remove_file(&path).expect("remove victim tile");

    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let err = EngineBuilder::new(&src, plan, &verify_sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect_err("Verify must fail when a tile is missing");

    match err {
        EngineError::Sink(SinkError::Other(ref msg)) => {
            assert!(
                msg.contains("Verify: missing tile"),
                "expected 'Verify: missing tile' in error, got: {msg}"
            );
        }
        other => panic!("expected EngineError::Sink(SinkError::Other), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 4. Verify catches byte-level corruption in raw tiles via ChecksumMismatch.
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_verify_detects_corrupt_raw_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Flip one byte in one tile.
    let victim = plan.tile_coords().next().unwrap();
    let rel = plan.tile_path(victim, "raw").unwrap();
    let path = base.join(&rel);
    corrupt_file(&path);

    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let err = EngineBuilder::new(&src, plan, &verify_sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect_err("Verify must fail on byte-corrupted raw tile");

    match err {
        EngineError::ChecksumMismatch { .. } => {}
        other => panic!("expected EngineError::ChecksumMismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 5. Verify refuses to run against a checkpoint for a different plan.
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_verify_rejects_plan_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);

    let plan_a = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let plan_b = PyramidPlanner::new(256, 192, 128, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Seed the directory with plan A's full pyramid + checkpoint.
    let sink_a = FsSink::new(base.clone(), plan_a.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan_a.clone(), &sink_a)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Attempt a Verify run with plan B — the on-disk checkpoint's
    // plan_hash cannot match and the builder must surface
    // `PlanHashMismatch` rather than silently overwriting.
    let sink_b = FsSink::new(base.clone(), plan_b.clone()).with_format(TileFormat::Raw);
    let res = EngineBuilder::new(&src, plan_b, &sink_b)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run();

    match res {
        Err(EngineError::PlanHashMismatch { .. }) => {}
        other => panic!("expected EngineError::PlanHashMismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 6. Verify emits the full observer event stream.
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_verify_emits_observer_events() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(192, 128);
    let plan = PyramidPlanner::new(192, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Produce first so Verify has a valid tree to audit.
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Verify with an observer wired in.
    let obs = Arc::new(CollectingObserver::new());
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .with_observer_arc(obs.clone())
        .run()
        .expect("Verify should succeed");

    let events = obs.events();
    let level_starts = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::LevelStarted { .. }))
        .count();
    let level_completes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::LevelCompleted { .. }))
        .count();
    let tile_completes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::TileCompleted { .. }))
        .count();
    let finishes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::Finished { .. }))
        .count();

    assert_eq!(
        level_starts,
        plan.level_count(),
        "Verify must emit one LevelStarted per level"
    );
    assert_eq!(
        level_completes,
        plan.level_count(),
        "Verify must emit one LevelCompleted per level"
    );
    assert_eq!(
        tile_completes as u64,
        plan.total_tile_count(),
        "Verify must emit one TileCompleted per tile"
    );
    assert_eq!(finishes, 1, "Verify must emit exactly one Finished");
}

// ---------------------------------------------------------------------------
// 7. Verify honours manifest checksums (Blake3) on non-raw formats.
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_verify_honours_manifest_checksums() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(192, 128);
    let plan = PyramidPlanner::new(192, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Produce PNG tiles with a Blake3-backed manifest. Verify's
    // byte-exact branch does not apply to encoded formats, so the only
    // way corruption can be detected is through the manifest.
    let sink = FsSink::new(base.clone(), plan.clone())
        .with_format(TileFormat::Png)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Clean-room Verify first — should pass.
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Png);
    EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("Verify should pass on an intact PNG pyramid with manifest");

    // Corrupt one PNG tile and re-run Verify.
    let victim = plan.tile_coords().next().unwrap();
    let rel = plan.tile_path(victim, TileFormat::Png.extension()).unwrap();
    let path = base.join(&rel);
    corrupt_file(&path);

    let verify_sink2 = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Png);
    let err = EngineBuilder::new(&src, plan, &verify_sink2)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect_err("Verify must fail when a manifest-listed PNG tile is corrupted");

    match err {
        EngineError::ChecksumMismatch { .. } => {}
        other => panic!("expected EngineError::ChecksumMismatch, got {other:?}"),
    }
}
