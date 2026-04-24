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
//! the expected `tiles_produced` count: non-zero for Overwrite and Resume
//! (resumed runs are idempotent — `tiles_produced` still counts every tile
//! the engine visited), and zero for Verify (Verify is read-only and does
//! not produce tiles). For Verify cells the pyramid is first seeded via an
//! Overwrite run against the same directory.

#![cfg(feature = "builder_v1")]

use std::path::PathBuf;

use libviprs::sink::{FsSink, TileFormat};
use libviprs::streaming::RasterStripSource;
use libviprs::{
    EngineBuilder, EngineError, EngineKind, Layout, PixelFormat, PyramidPlan, PyramidPlanner,
    Raster, ResumePolicy,
};

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

/// A small 128x96 RGB8 gradient. Kept deliberately small so the 18-cell
/// matrix stays fast; every Verify cell has to seed a pyramid first, so the
/// test binary effectively runs each Overwrite case twice.
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
fn seed_pyramid(base: &PathBuf, plan: &PyramidPlan, src: &Raster) {
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);

    // Seed a checkpoint with a fresh Overwrite run so Resume has something
    // to pick up from.
    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    // `tiles_produced` counts every tile the engine visits, including ones
    // short-circuited by the skip set. With a full checkpoint the count
    // still equals the plan's total — the short-circuit happens at the sink
    // layer, not in the engine's accounting.
    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert!(result.tiles_produced > 0);
}

#[test]
fn monolithic_verify_with_raster_source() {
    let (_dir, base, plan) = fresh_directory();
    let src = gradient_raster(128, 96);

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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);

    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    assert!(result.tiles_produced > 0);
}

#[test]
fn streaming_verify_with_raster_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = gradient_raster(128, 96);

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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);

    seed_pyramid(&base, &plan, &src);

    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    assert!(result.tiles_produced > 0);
}

#[test]
fn streaming_verify_with_strip_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = gradient_raster(128, 96);

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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);

    seed_pyramid(&base, &plan, &src);

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    assert!(result.tiles_produced > 0);
}

#[test]
fn mapreduce_verify_with_raster_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = gradient_raster(128, 96);

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
    let src = gradient_raster(128, 96);
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
    let src = gradient_raster(128, 96);

    seed_pyramid(&base, &plan, &src);

    let strip = RasterStripSource::new(&src);
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(strip, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    assert!(result.tiles_produced > 0);
}

#[test]
fn mapreduce_verify_with_strip_source() {
    // New cell — unlocks once the stream-verify implementation lands.
    let (_dir, base, plan) = fresh_directory();
    let src = gradient_raster(128, 96);

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
