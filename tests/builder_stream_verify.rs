//! TDD-style integration tests for the (not-yet-existing) Streaming+Verify
//! surface on `EngineBuilder`.
//!
//! Today `EngineKind::Streaming` paired with `ResumePolicy::verify()` is
//! rejected outright (see
//! `builder_resume_streaming::streaming_verify_is_now_supported`, which pins
//! that it is not rejected any more).
//! That blanket rejection is about to be lifted: the builder will wrap the
//! user-provided sink in a verifying adapter that walks every tile recorded
//! by the plan and re-reads it from the target directory without producing
//! any new output. The tests below pin down the contract that implementation
//! must satisfy:
//!
//!   1. `streaming_verify_passes_on_intact_output`
//!   2. `streaming_verify_accepts_strip_source`
//!   3. `streaming_verify_detects_missing_tile`
//!   4. `streaming_verify_detects_corrupt_raw_tile`
//!   5. `streaming_verify_rejects_plan_hash_mismatch`
//!   6. `streaming_verify_passes_png_by_existence_alone`
//!   7. `streaming_verify_honours_manifest_checksums`
//!   8. `streaming_verify_emits_observer_events`
//!
//! The Streaming+Verify surface is implemented on the crate side, so these
//! run in the default harness alongside the other builder-level integration
//! tests.

use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::Arc;

use libviprs::checksum::ChecksumAlgo;
use libviprs::manifest::ManifestBuilder;
use libviprs::streaming::RasterStripSource;
use libviprs::{
    CollectingObserver, EngineBuilder, EngineError, EngineEvent, EngineKind, FsSink, Layout,
    PyramidPlanner, ResumePolicy, TileFormat,
};

mod common;
use common::fixtures::canonical_raster_scaled;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Inline copy of `phase3_checksum::corrupt_file` — flips a single byte at a
/// well-known offset inside `path`. For tiny files (<16 bytes) flips index
/// 0; otherwise picks an offset past any format magic so PNG/JPEG tests can
/// still target meaningful bytes.
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

/// Find the raw-tile path for an arbitrary "middle of the plan" tile.
/// Keeps the tests from depending on any one particular coord.
fn first_tile_relpath(plan: &libviprs::planner::PyramidPlan, ext: &str) -> std::path::PathBuf {
    let coord = plan
        .tile_coords()
        .next()
        .expect("plan must have at least one tile");
    std::path::PathBuf::from(plan.tile_path(coord, ext).unwrap())
}

// ---------------------------------------------------------------------------
// 1. Verify passes on an intact pyramid that we ourselves just wrote.
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_passes_on_intact_output() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Produce the full pyramid with Streaming + Overwrite.
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("initial Streaming+Overwrite run should succeed");

    // Re-run with Streaming + Verify — must succeed without producing tiles.
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("Streaming+Verify on intact output should return Ok");

    assert_eq!(
        result.tiles_produced, 0,
        "Streaming+Verify must not produce tiles — it only validates"
    );
}

// ---------------------------------------------------------------------------
// 2. Verify accepts a RasterStripSource as its source.
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_accepts_strip_source() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let raster = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // First run: seed the output with a full pyramid via a strip source so
    // that the verify pass exercises the same input shape.
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(RasterStripSource::new(&raster), plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("Streaming+Overwrite with RasterStripSource should succeed");

    // Verify must also work with a strip source.
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(RasterStripSource::new(&raster), plan.clone(), &verify_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("Streaming+Verify must accept a RasterStripSource");

    assert_eq!(result.tiles_produced, 0);
}

// ---------------------------------------------------------------------------
// 3. Verify detects a missing tile file and returns SinkError::Other.
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_detects_missing_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Remove one tile so Verify has something to complain about.
    let rel = first_tile_relpath(&plan, "raw");
    let victim = base.join(&rel);
    assert!(victim.exists(), "victim tile missing before deletion");
    std::fs::remove_file(&victim).unwrap();

    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let err = EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect_err("Streaming+Verify must fail when a tile file is missing");

    match err {
        EngineError::Sink(libviprs::sink::SinkError::Other(msg)) => {
            assert!(
                msg.contains("Verify: missing tile"),
                "expected missing-tile message from Verify, got: {msg}"
            );
        }
        other => panic!(
            "expected EngineError::Sink(SinkError::Other(..)) for missing tile, got {other:?}"
        ),
    }
}

// ---------------------------------------------------------------------------
// 4. Verify detects a byte-level mutation on raw tiles via
//    EngineError::ChecksumMismatch (raw tiles are compared byte-for-byte).
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_detects_corrupt_raw_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Flip a single byte inside a real raw tile — enough to break the
    // byte-exact comparison that Verify performs on raw output.
    let rel = first_tile_relpath(&plan, "raw");
    let victim = base.join(&rel);
    corrupt_file(&victim);

    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let err = EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect_err("Streaming+Verify must fail on corrupt raw tile");

    assert!(
        matches!(err, EngineError::ChecksumMismatch { .. }),
        "expected EngineError::ChecksumMismatch for corrupt raw tile, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 5. Verify rejects a plan that does not match the one used to produce the
//    output directory — surfaced as EngineError::PlanHashMismatch.
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_rejects_plan_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);

    let plan_a = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let plan_b = PyramidPlanner::new(256, 192, 128, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Seed directory with a full pyramid under plan A.
    let sink_a = FsSink::new(base.clone(), plan_a.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan_a.clone(), &sink_a)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Attempt Verify using plan B — must fail with PlanHashMismatch.
    let sink_b = FsSink::new(base.clone(), plan_b.clone()).with_format(TileFormat::Raw);
    let err = EngineBuilder::new(&src, plan_b.clone(), &sink_b)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect_err("Streaming+Verify must fail when plan hash disagrees");

    assert!(
        matches!(err, EngineError::PlanHashMismatch { .. }),
        "expected EngineError::PlanHashMismatch, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 6. Verify passes PNG tiles by existence alone — a byte flip that keeps
//    the file a valid PNG should NOT be detected (mirror of the monolithic
//    Verify behaviour for lossy / re-encodable formats).
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_passes_png_by_existence_alone() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Png);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Flip a byte deep inside the PNG payload — past the 8-byte magic and
    // header so the file remains a structurally valid (if decoded wrong)
    // PNG as far as a pure existence check is concerned. The on-disk
    // length and magic are preserved, which is all Verify is allowed to
    // inspect for lossy formats.
    let rel = first_tile_relpath(&plan, "png");
    let victim = base.join(&rel);
    let mut bytes = std::fs::read(&victim).unwrap();
    assert!(
        bytes.len() > 64,
        "PNG tile unexpectedly small ({} bytes)",
        bytes.len()
    );
    // Mutate a byte well past the signature; reserved for IDAT payload.
    bytes[48] ^= 0x01;
    std::fs::write(&victim, &bytes).unwrap();

    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Png);
    let result = EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("Streaming+Verify on PNG must pass on existence alone");

    assert_eq!(result.tiles_produced, 0);
}

// ---------------------------------------------------------------------------
// 7. When the output was produced with a manifest + per-tile checksums,
//    Verify MUST use those recorded digests — matching the monolithic
//    verify path — and surface corruption as ChecksumMismatch even on
//    formats where existence-only would otherwise pass.
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_honours_manifest_checksums() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Produce with manifest-level checksums so Verify has a recorded
    // digest to compare against, even for PNG tiles.
    let sink = FsSink::new(base.clone(), plan.clone())
        .with_format(TileFormat::Png)
        .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3));
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Corrupt a single tile — because the manifest carries a per-tile
    // digest, Verify must detect the mutation regardless of format.
    let rel = first_tile_relpath(&plan, "png");
    let victim = base.join(&rel);
    corrupt_file(&victim);

    let verify_sink = FsSink::new(base.clone(), plan.clone())
        .with_format(TileFormat::Png)
        .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3));
    let err = EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect_err("Streaming+Verify must honour manifest checksums");

    assert!(
        matches!(err, EngineError::ChecksumMismatch { .. }),
        "expected EngineError::ChecksumMismatch from manifest-backed Verify, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 8. Verify emits the same lifecycle events as a normal run — the observer
//    must see LevelStarted / TileCompleted / LevelCompleted / Finished in
//    counts that match the plan.
// ---------------------------------------------------------------------------

#[test]
fn streaming_verify_emits_observer_events() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = canonical_raster_scaled(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Seed the directory with a full pyramid.
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Verify pass with an attached observer.
    let obs = Arc::new(CollectingObserver::new());
    let verify_sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &verify_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .with_observer_arc(obs.clone())
        .run()
        .expect("Streaming+Verify should succeed with an observer attached");

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
        "Verify should emit one LevelStarted per level"
    );
    assert_eq!(
        level_completes,
        plan.level_count(),
        "Verify should emit one LevelCompleted per level"
    );
    assert_eq!(
        tile_completes as u64,
        plan.total_tile_count(),
        "Verify should emit one TileCompleted per verified tile"
    );
    assert_eq!(finishes, 1, "Verify should emit exactly one Finished event");
}
