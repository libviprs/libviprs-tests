//! Exhaustive (EngineKind × ResumeMode × SourceKind) matrix for
//! `EngineBuilder::with_resume`.
//!
//! Documents which cells of the 3 × 3 × 2 grid are supported today and which
//! return a typed `EngineError::IncompatibleSource`. Each cell is one test so
//! that a regression in any single dispatch arm is easy to locate from the
//! failing test name alone.
//!
//! Matrix — rows are (source, mode); columns are engines:
//!
//! | source                  | Monolithic             | Streaming     | MapReduce     |
//! |-------------------------|------------------------|---------------|---------------|
//! | &Raster      Overwrite  | ok                     | ok            | ok            |
//! | &Raster      Resume     | ok                     | ok            | ok            |
//! | &Raster      Verify     | ok                     | ok            | ok            |
//! | StripSource  Overwrite  | IncompatibleSource     | ok            | ok            |
//! | StripSource  Resume     | IncompatibleSource     | ok            | ok            |
//! | StripSource  Verify     | IncompatibleSource     | ok            | ok            |
//!
//! Rejection cells assert the typed error. Success cells assert `Ok(_)` with
//! the expected `tiles_produced` count. Overwrite writes the whole plan, so it
//! reports `plan.total_tile_count()`. Resume and Verify cells first seed a full
//! pyramid via an Overwrite run against the same directory, so both report `0`:
//! Resume is idempotent and skips every already-recorded tile (it accounts only
//! the tiles it actually wrote, which is none once the checkpoint is complete),
//! and Verify is read-only and never produces tiles. The idempotent-resume
//! contract is pinned independently in `phase3_resume::idempotent_resume_of_completed_job`.

use std::path::{Path, PathBuf};

use libviprs::sink::{FsSink, TileFormat};
use libviprs::streaming::RasterStripSource;
use libviprs::{
    EngineBuilder, EngineError, EngineKind, Layout, PyramidPlan, PyramidPlanner, Raster,
    ResumePolicy,
};

mod common;
use common::fixtures::canonical_raster_scaled;

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// Default plan used by every cell. 128x96 at 64-tile size under DeepZoom
/// layout yields a multi-level pyramid with a non-trivial tile count.
fn build_plan() -> PyramidPlan {
    PyramidPlanner::new(128, 96, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Per-cell isolation: each test gets its own tempdir, a sink rooted inside
/// it, and a fresh plan. Returns the `_dir` guard so the caller can keep the
/// directory alive for the lifetime of the test.
fn fresh_directory() -> (tempfile::TempDir, PathBuf, PyramidPlan) {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let plan = build_plan();
    (dir, base, plan)
}

/// Seed a fully-written pyramid at `base` so a subsequent Verify run has
/// something to audit. Uses Monolithic + Overwrite — the one engine × mode
/// pair that is supported for every Verify seed scenario.
fn seed_pyramid(base: &Path, plan: &PyramidPlan, src: &Raster) {
    let sink = FsSink::new(base.to_path_buf(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();
}

// ===========================================================================
// Monolithic × { Overwrite, Resume, Verify } × { &Raster, StripSource }
// ===========================================================================

#[test]
fn monolithic_overwrite_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert!(result.tiles_produced > 0);
}

#[test]
fn monolithic_resume_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    // Seed a checkpoint with a fresh Overwrite run so Resume has something
    // to pick up from.
    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    // `tiles_produced` counts only the tiles the resumed run actually wrote.
    // With a full checkpoint every tile is skipped, so the resumed run is a
    // no-op and produces zero tiles. This is the idempotent-resume contract
    // pinned in `phase3_resume::idempotent_resume_of_completed_job`.
    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn monolithic_verify_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::verify())
        .run()
        .unwrap();

    // Verify is read-only: it walks the tree, hashes existing tiles, and
    // produces nothing.
    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn monolithic_overwrite_with_strip_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let res = EngineBuilder::new(strip, plan, &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run();

    match res {
        Err(EngineError::IncompatibleSource { .. }) => {}
        other => {
            panic!("expected IncompatibleSource for Monolithic+Overwrite+Strip, got {other:?}")
        }
    }
}

#[test]
fn monolithic_resume_with_strip_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let res = EngineBuilder::new(strip, plan, &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume())
        .run();

    match res {
        Err(EngineError::IncompatibleSource { .. }) => {}
        other => panic!("expected IncompatibleSource for Monolithic+Resume+Strip, got {other:?}"),
    }
}

#[test]
fn monolithic_verify_with_strip_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let res = EngineBuilder::new(strip, plan, &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::verify())
        .run();

    match res {
        Err(EngineError::IncompatibleSource { .. }) => {}
        other => panic!("expected IncompatibleSource for Monolithic+Verify+Strip, got {other:?}"),
    }
}

// ===========================================================================
// Streaming × { Overwrite, Resume, Verify } × { &Raster, StripSource }
// ===========================================================================

#[test]
fn streaming_overwrite_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert!(result.tiles_produced > 0);
}

#[test]
fn streaming_resume_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    // Idempotent resume over a full checkpoint writes nothing: `tiles_produced`
    // counts only tiles actually written, which is zero once every tile is
    // already recorded (see `phase3_resume::idempotent_resume_of_completed_job`).
    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn streaming_verify_with_raster_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn streaming_overwrite_with_strip_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert!(result.tiles_produced > 0);
}

#[test]
fn streaming_resume_with_strip_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    // Idempotent resume over a full checkpoint writes nothing: `tiles_produced`
    // counts only tiles actually written, which is zero once every tile is
    // already recorded (see `phase3_resume::idempotent_resume_of_completed_job`).
    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn streaming_verify_with_strip_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, 0);
}

// ===========================================================================
// MapReduce × { Overwrite, Resume, Verify } × { &Raster, StripSource }
// ===========================================================================

#[test]
fn mapreduce_overwrite_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert!(result.tiles_produced > 0);
}

#[test]
fn mapreduce_resume_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    // Idempotent resume over a full checkpoint writes nothing: `tiles_produced`
    // counts only tiles actually written, which is zero once every tile is
    // already recorded (see `phase3_resume::idempotent_resume_of_completed_job`).
    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn mapreduce_verify_with_raster_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn mapreduce_overwrite_with_strip_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);
    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);

    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert!(result.tiles_produced > 0);
}

#[test]
fn mapreduce_resume_with_strip_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    // Idempotent resume over a full checkpoint writes nothing: `tiles_produced`
    // counts only tiles actually written, which is zero once every tile is
    // already recorded (see `phase3_resume::idempotent_resume_of_completed_job`).
    assert_eq!(result.tiles_produced, 0);
}

#[test]
fn mapreduce_verify_with_strip_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = canonical_raster_scaled(128, 96);

    seed_pyramid(&base, &plan, &src);

    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .unwrap();

    assert_eq!(result.tiles_produced, 0);
}
