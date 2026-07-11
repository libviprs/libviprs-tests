//! Exhaustive coverage of the error surface of `ResumePolicy::verify()` when
//! run through `EngineBuilder`.
//!
//! Each test exercises one specific failure path of Verify mode and asserts
//! on the exact typed `EngineError` variant (or the precise `SinkError::Other`
//! message) the engine is contracted to return.
//!
//! All scenarios are looped over every engine kind that supports Verify. Today
//! that is only [`EngineKind::Monolithic`] — the Streaming and MapReduce
//! engines currently reject Verify with
//! [`EngineError::IncompatibleSource`] (see `builder_resume_streaming.rs`).
//! The loop below is kept with all three kinds so that the day the
//! stream-verify implementation lands, the entire error surface is already
//! pinned down. Until then, the Streaming / MapReduce arms carry a
//! short-circuit that skips the scenario if the engine returns
//! `IncompatibleSource` — the test still runs on Monolithic, and flips to
//! full coverage automatically.

use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use libviprs::checksum::{ChecksumAlgo, ChecksumMode};
use libviprs::manifest::ManifestBuilder;
use libviprs::resume::JobMetadata;
use libviprs::sink::{FsSink, MemorySink, TileFormat};
use libviprs::{
    EngineBuilder, EngineError, EngineKind, Layout, PyramidPlan, PyramidPlanner, Raster,
    ResumePolicy, SinkError, TileCoord,
};

mod common;
use common::fixtures::canonical_raster_scaled;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The well-known filename for the on-disk checkpoint.
const JOB_FILE: &str = ".libviprs-job.json";

/// Engine kinds that are expected to grow a Verify implementation. Today only
/// [`EngineKind::Monolithic`] actually runs the Verify path end-to-end; the
/// others return [`EngineError::IncompatibleSource`]. Once stream-verify
/// lands, the per-kind arms below start asserting real verify behaviour
/// without any changes to this file.
const VERIFY_KINDS: &[EngineKind] = &[
    EngineKind::Monolithic,
    EngineKind::Streaming,
    EngineKind::MapReduce,
];

/// Small plan: enough tiles to matter, but the tests are cheap to run.
fn small_plan(w: u32, h: u32, tile_size: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, tile_size, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Flip a byte in `path`. Copied from `phase3_resume.rs` to keep this test
/// file self-contained. Targets offset 0 for tiny files and offset 12 for
/// larger ones so we don't clobber format magic bytes (irrelevant here — raw
/// tiles have no magic — but keeps the helper reusable for PNG/JPEG).
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
    f.seek(std::io::SeekFrom::Start(0)).unwrap();
    f.write_all(&buf).unwrap();
    f.set_len(buf.len() as u64).unwrap();
}

/// Seed `base` with a fresh raw pyramid for the given plan, writing a
/// checkpoint alongside the tiles. Uses the Monolithic engine (the only one
/// that guarantees a complete run today) regardless of the kind under test —
/// Verify scenarios only care that the on-disk state is valid before the
/// re-run.
fn seed_raw_pyramid(base: &Path, src: &Raster, plan: &PyramidPlan) {
    let sink = FsSink::new(base.to_path_buf(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("seed Overwrite run should succeed");
}

/// Seed `base` with a fresh raw pyramid + manifest carrying per-tile
/// checksums. Variant of [`seed_raw_pyramid`] for tests that want to
/// exercise the manifest-checksum branch of Verify.
fn seed_raw_pyramid_with_checksums(base: &Path, src: &Raster, plan: &PyramidPlan) {
    let sink = FsSink::new(base.to_path_buf(), plan.clone())
        .with_format(TileFormat::Raw)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("seed Overwrite run (with checksums) should succeed");
}

/// Resolve the on-disk absolute path of a single tile file.
fn tile_abs(base: &Path, plan: &PyramidPlan, coord: TileCoord) -> PathBuf {
    let rel = plan
        .tile_path(coord, "raw")
        .expect("plan should produce a path for every tile coord");
    base.join(rel)
}

/// Skip the current engine-kind arm when the engine reports that it does not
/// (yet) know how to run Verify. Keeps the tests green on today's codebase
/// while still covering Monolithic exhaustively; once stream-verify lands,
/// this branch stops firing and the assertions below become live for all
/// three engines.
fn is_incompatible(err: &EngineError) -> bool {
    matches!(err, EngineError::IncompatibleSource { .. })
}

// ---------------------------------------------------------------------------
// 1. missing tile file -> EngineError::Sink(SinkError::Other("Verify: missing tile ..."))
// ---------------------------------------------------------------------------

#[test]
fn verify_missing_tile_returns_sink_other_error() {
    for kind in VERIFY_KINDS.iter().copied() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("tiles");
        let src = canonical_raster_scaled(128, 128);
        let plan = small_plan(128, 128, 64);

        seed_raw_pyramid(&base, &src, &plan);

        // Remove exactly one tile file — Verify must notice it's gone.
        let victim = plan
            .tile_coords()
            .next()
            .expect("plan has at least one tile");
        let victim_path = tile_abs(&base, &plan, victim);
        std::fs::remove_file(&victim_path)
            .unwrap_or_else(|e| panic!("remove {}: {e}", victim_path.display()));

        let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        let res = EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(kind)
            .with_resume(ResumePolicy::verify())
            .run();

        match res {
            Err(EngineError::Sink(SinkError::Other(msg))) => {
                assert!(
                    msg.contains("Verify: missing tile for coord"),
                    "[{kind:?}] unexpected SinkError::Other message: {msg}"
                );
            }
            Err(ref e) if is_incompatible(e) => { /* engine kind not yet verify-capable */ }
            other => panic!(
                "[{kind:?}] expected Sink(Other(\"Verify: missing tile ...\")), got {other:?}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// 2. corrupt raw tile -> EngineError::ChecksumMismatch (byte-exact raw path)
// ---------------------------------------------------------------------------

#[test]
fn verify_corrupt_raw_tile_returns_checksum_mismatch() {
    for kind in VERIFY_KINDS.iter().copied() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("tiles");
        let src = canonical_raster_scaled(128, 128);
        let plan = small_plan(128, 128, 64);

        seed_raw_pyramid(&base, &src, &plan);

        // Flip a byte in a known raw tile. For raw tiles Verify does a
        // byte-for-byte comparison against the regenerated tile and must
        // surface ChecksumMismatch without ever consulting a manifest.
        let victim = plan
            .tile_coords()
            .next()
            .expect("plan has at least one tile");
        let victim_path = tile_abs(&base, &plan, victim);
        corrupt_file(&victim_path);

        let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        let res = EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(kind)
            .with_resume(ResumePolicy::verify())
            .run();

        match res {
            Err(EngineError::ChecksumMismatch {
                tile,
                expected,
                got,
            }) => {
                // The raw branch annotates expected/got with byte-count
                // descriptors rather than hex digests. We don't pin the
                // exact strings — the variant alone is the contract.
                let _ = (tile, expected, got);
            }
            Err(ref e) if is_incompatible(e) => { /* engine kind not yet verify-capable */ }
            other => panic!("[{kind:?}] expected ChecksumMismatch, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 3. plan-hash mismatch -> EngineError::PlanHashMismatch
// ---------------------------------------------------------------------------

#[test]
fn verify_plan_hash_mismatch_returns_typed_error() {
    for kind in VERIFY_KINDS.iter().copied() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("tiles");
        let src = canonical_raster_scaled(128, 128);
        let plan = small_plan(128, 128, 64);

        seed_raw_pyramid(&base, &src, &plan);

        // Overwrite the on-disk checkpoint with a hash that cannot match
        // the current plan.
        let stale = JobMetadata::new(
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
            "1970-01-01T00:00:00Z".into(),
        );
        std::fs::write(base.join(JOB_FILE), serde_json::to_vec(&stale).unwrap()).unwrap();

        let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        let res = EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(kind)
            .with_resume(ResumePolicy::verify())
            .run();

        match res {
            Err(EngineError::PlanHashMismatch { .. }) => {}
            Err(ref e) if is_incompatible(e) => { /* engine kind not yet verify-capable */ }
            other => panic!("[{kind:?}] expected PlanHashMismatch, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 4. manifest-checksum mismatch -> EngineError::ChecksumMismatch
// ---------------------------------------------------------------------------

#[test]
fn verify_manifest_checksum_mismatch_returns_typed_error() {
    for kind in VERIFY_KINDS.iter().copied() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("pyramid");
        let src = canonical_raster_scaled(128, 128);
        let plan = small_plan(128, 128, 64);

        seed_raw_pyramid_with_checksums(&base, &src, &plan);

        // Corrupt a raw tile. Because a manifest with per-tile checksums is
        // present, Verify is contracted to surface the mismatch via the
        // manifest-checksum branch *before* the byte-exact raw branch runs.
        // Either path returns ChecksumMismatch, so the match below is
        // robust to implementation ordering.
        let victim = plan
            .tile_coords()
            .next()
            .expect("plan has at least one tile");
        let victim_path = tile_abs(&base, &plan, victim);
        corrupt_file(&victim_path);

        let sink = FsSink::new(base.clone(), plan.clone())
            .with_format(TileFormat::Raw)
            .with_manifest(ManifestBuilder::new())
            .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
        let res = EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(kind)
            .with_resume(ResumePolicy::verify())
            .run();

        match res {
            Err(EngineError::ChecksumMismatch { .. }) => {}
            Err(ref e) if is_incompatible(e) => { /* engine kind not yet verify-capable */ }
            other => {
                panic!("[{kind:?}] expected ChecksumMismatch from manifest branch, got {other:?}")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 5. MemorySink (no on-disk root) -> EngineError::VerifyRequiresOnDiskSink
// ---------------------------------------------------------------------------

#[test]
fn verify_no_checkpoint_root_returns_typed_error() {
    for kind in VERIFY_KINDS.iter().copied() {
        let src = canonical_raster_scaled(64, 64);
        let plan = small_plan(64, 64, 32);

        // MemorySink returns `None` from `checkpoint_root`, and we pass no
        // explicit `EngineConfig::checkpoint_root`, so Verify has nowhere
        // to read tiles from.
        let sink = MemorySink::new();
        let res = EngineBuilder::new(&src, plan.clone(), sink)
            .with_engine(kind)
            .with_resume(ResumePolicy::verify())
            .run();

        match res {
            Err(EngineError::VerifyRequiresOnDiskSink) => {}
            Err(ref e) if is_incompatible(e) => { /* engine kind not yet verify-capable */ }
            other => panic!("[{kind:?}] expected VerifyRequiresOnDiskSink, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 6. fresh empty directory -> EngineError::Sink(Other("missing tile"))
// ---------------------------------------------------------------------------

#[test]
fn verify_empty_directory_returns_missing_tile() {
    for kind in VERIFY_KINDS.iter().copied() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("tiles");
        std::fs::create_dir_all(&base).unwrap();

        let src = canonical_raster_scaled(128, 128);
        let plan = small_plan(128, 128, 64);

        // No seeding — the directory exists but is empty.
        let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        let res = EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(kind)
            .with_resume(ResumePolicy::verify())
            .run();

        match res {
            Err(EngineError::Sink(SinkError::Other(msg))) => {
                assert!(
                    msg.to_lowercase().contains("missing tile"),
                    "[{kind:?}] expected 'missing tile' in message, got: {msg}"
                );
            }
            Err(ref e) if is_incompatible(e) => { /* engine kind not yet verify-capable */ }
            other => panic!(
                "[{kind:?}] expected Sink(Other(\"...missing tile...\")) on empty dir, got {other:?}"
            ),
        }
    }
}
