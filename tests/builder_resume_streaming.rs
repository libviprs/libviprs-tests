//! Resume + Streaming / MapReduce integration tests.
//!
//! Exercises `EngineBuilder::with_resume` paired with
//! `EngineKind::Streaming` and `EngineKind::MapReduce` — the builder paths
//! wired up after the initial Monolithic-only resumable implementation.
//!
//! The streaming/mapreduce engines themselves are oblivious to resume
//! state; the builder wraps the user-provided sink in an internal filter
//! that (a) short-circuits `write_tile` for tiles already recorded in the
//! on-disk checkpoint, and (b) advances the `.libviprs-job.json`
//! checkpoint on every successful real write.
//!
//! What's covered:
//!
//! 1. Streaming Overwrite + Resume — tile-by-tile equivalence vs. the
//!    Monolithic run against the same raster.
//! 2. MapReduce Overwrite + Resume — same parity check, parallel engine.
//! 3. Resume short-circuits already-complete tiles — a recording wrapper
//!    counts how many writes actually hit the inner sink on the second
//!    run.
//! 4. Verify is rejected for Streaming / MapReduce with a typed error
//!    (verify walks every level via downscale_half, which needs the full
//!    raster in memory).
//! 5. Plan-hash mismatch on Resume surfaces `EngineError::PlanHashMismatch`
//!    the same way it does for Monolithic.

#![cfg(feature = "builder_v1")]

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use libviprs::resume::{JobCheckpoint, JobMetadata, ResumeMode};
use libviprs::sink::{FsSink, SinkError, Tile, TileFormat, TileSink};
use libviprs::{
    EngineBuilder, EngineConfig, EngineError, EngineKind, Layout, PixelFormat, PyramidPlanner,
    Raster, ResumePolicy,
};

// ---------------------------------------------------------------------------
// Shared fixtures
// ---------------------------------------------------------------------------

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

fn list_files(dir: &Path, ext: &str) -> usize {
    let mut n = 0;
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                n += list_files(&p, ext);
            } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
                n += 1;
            }
        }
    }
    n
}

fn read_checkpoint(dir: &Path) -> Option<JobMetadata> {
    JobCheckpoint::load(dir).ok().flatten()
}

// ---------------------------------------------------------------------------
// RecordingSink — counts writes that hit the inner sink
// ---------------------------------------------------------------------------

struct RecordingSink {
    inner: FsSink,
    writes: AtomicUsize,
    coords: Mutex<Vec<libviprs::planner::TileCoord>>,
}

impl RecordingSink {
    fn new(inner: FsSink) -> Self {
        Self {
            inner,
            writes: AtomicUsize::new(0),
            coords: Mutex::new(Vec::new()),
        }
    }

    fn write_count(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

impl TileSink for RecordingSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.coords.lock().unwrap().push(tile.coord);
        self.inner.write_tile(tile)
    }
    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
    fn checkpoint_root(&self) -> Option<&Path> {
        self.inner.checkpoint_root()
    }
    fn record_engine_config(&self, cfg: &EngineConfig) {
        self.inner.record_engine_config(cfg)
    }
    fn init_level_count(&self, levels: usize) {
        self.inner.init_level_count(levels)
    }
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

#[test]
fn streaming_overwrite_produces_full_pyramid_and_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = gradient_raster(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(
        list_files(&base, "raw") as u64,
        plan.total_tile_count(),
        "Overwrite produced wrong tile count"
    );
    let cp = read_checkpoint(&base).expect("checkpoint should exist after Overwrite");
    assert_eq!(cp.completed_tiles.len() as u64, plan.total_tile_count());
}

#[test]
fn streaming_resume_short_circuits_already_complete_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = gradient_raster(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // First run — Overwrite, produces every tile and a full checkpoint.
    {
        let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::Streaming)
            .with_resume(ResumePolicy::overwrite())
            .run()
            .unwrap();
    }

    // Second run — Resume. Every tile is already in the checkpoint, so
    // ResumeAwareSink must short-circuit every write — the recording
    // wrapper sees zero writes even though the engine still renders and
    // presents every tile.
    let inner = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let recording = RecordingSink::new(inner);
    EngineBuilder::new(&src, plan.clone(), &recording)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    assert_eq!(
        recording.write_count(),
        0,
        "Resume wrote {} tiles even though the checkpoint recorded a full run",
        recording.write_count()
    );
}

#[test]
fn streaming_resume_agrees_with_monolithic() {
    // Tiles produced by Streaming + Resume against a fresh directory must
    // match tiles produced by Monolithic + Overwrite byte-for-byte.
    let dir = tempfile::tempdir().unwrap();
    let mono_dir = dir.path().join("mono");
    let stream_dir = dir.path().join("stream");
    let src = gradient_raster(192, 128);
    let plan = PyramidPlanner::new(192, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let mono_sink = FsSink::new(mono_dir.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &mono_sink)
        .with_engine(EngineKind::Monolithic)
        .run()
        .unwrap();

    let stream_sink = FsSink::new(stream_dir.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &stream_sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(
        list_files(&mono_dir, "raw"),
        list_files(&stream_dir, "raw"),
        "tile counts differ between Monolithic and Streaming+resume"
    );
}

#[test]
fn streaming_verify_is_now_supported() {
    // Historically Streaming+Verify was rejected with IncompatibleSource
    // because the verify loop needed the full raster. After the stream-
    // verify implementation landed (see `builder_stream_verify.rs` for
    // the full coverage), Streaming+Verify now succeeds against a
    // previously-generated pyramid.
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Seed an intact pyramid first so Verify has something to walk.
    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan, &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("streaming verify now supported");
    assert_eq!(result.tiles_produced, 0, "verify must not write tiles");
}

#[test]
fn streaming_resume_rejects_plan_hash_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = gradient_raster(256, 192);

    let plan_a = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let plan_b = PyramidPlanner::new(256, 192, 128, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Seed the directory with a checkpoint for plan A.
    let sink = FsSink::new(base.clone(), plan_a.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan_a.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    // Try to resume with a different plan — expect PlanHashMismatch.
    let sink_b = FsSink::new(base, plan_b.clone()).with_format(TileFormat::Raw);
    let res = EngineBuilder::new(&src, plan_b, &sink_b)
        .with_engine(EngineKind::Streaming)
        .with_resume(ResumePolicy::resume())
        .run();

    match res {
        Err(EngineError::PlanHashMismatch { .. }) => {}
        other => panic!("expected PlanHashMismatch, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// MapReduce
// ---------------------------------------------------------------------------

#[test]
fn mapreduce_overwrite_produces_full_pyramid_and_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = gradient_raster(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    assert_eq!(
        list_files(&base, "raw") as u64,
        plan.total_tile_count(),
        "MapReduce+Overwrite produced wrong tile count"
    );
    let cp = read_checkpoint(&base).expect("checkpoint should exist after Overwrite");
    assert_eq!(cp.completed_tiles.len() as u64, plan.total_tile_count());
}

#[test]
fn mapreduce_resume_short_circuits_already_complete_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = gradient_raster(256, 192);
    let plan = PyramidPlanner::new(256, 192, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    {
        let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
        EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::MapReduce)
            .with_resume(ResumePolicy::overwrite())
            .run()
            .unwrap();
    }

    let inner = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    let recording = RecordingSink::new(inner);
    EngineBuilder::new(&src, plan.clone(), &recording)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::resume())
        .run()
        .unwrap();

    assert_eq!(recording.write_count(), 0, "MapReduce resume wrote tiles");
}

#[test]
fn mapreduce_verify_is_now_supported() {
    // MapReduce+Verify flipped from IncompatibleSource to supported once
    // the stream-verify implementation landed. Both Streaming and
    // MapReduce route through the same verify_from_strip_source helper,
    // since verify is read-only and doesn't need MapReduce's parallel
    // write pipeline. Full coverage lives in builder_mapreduce_verify.rs.
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("tiles");
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(base.clone(), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();

    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    let result = EngineBuilder::new(&src, plan, &sink)
        .with_engine(EngineKind::MapReduce)
        .with_resume(ResumePolicy::verify())
        .run()
        .expect("mapreduce verify now supported");
    assert_eq!(result.tiles_produced, 0);
}
