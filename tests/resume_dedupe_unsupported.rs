//! Issue #272 (P0, data corruption) — stopgap guard repro.
//!
//! On **resume**, the engine reconstructs the checkpoint / `CheckpointState`
//! but DISCARDS the sink-side manifest/dedupe state (`manifest_refs`,
//! `tile_digests`, the `DedupeIndex`). `ResumeAwareSink` short-circuits
//! already-completed coordinates *before* they reach `FsSink::write_tile`, so
//! for a resumed run those maps start empty for every pre-crash tile. Then
//! `FsSink::finish()` unconditionally rebuilds and atomically overwrites
//! `manifest.json` from those empty-for-pre-crash in-memory maps whenever a
//! manifest is emitted (dedupe active, or a checksum/manifest builder
//! attached). The result: every pre-crash 1-byte placeholder — resolvable
//! only through `blank_references` — becomes permanently unresolvable, and the
//! per-tile checksum table is silently truncated. Silent, reader-visible data
//! corruption on the documented resume + dedupe / resume + checksum combos.
//!
//! Until resume learns to seed that state (the real fix, tracked separately on
//! #272 to land after #275/#277), the stopgap is to HARD-ERROR up front when a
//! resuming `ResumePolicy` is combined with dedupe or checksums, rather than
//! proceed and corrupt.
//!
//! These tests assert the combination now returns `Err`. They are RED against
//! a core WITHOUT the guard (today `run()` returns `Ok(..)` — it silently
//! proceeds) and GREEN once the stopgap lands. The third test pins the
//! narrowness of the guard: resume with neither dedupe nor checksum stays a
//! supported, `Ok` run.

use libviprs::checksum::{ChecksumAlgo, ChecksumMode};
use libviprs::manifest::ManifestBuilder;
use libviprs::sink::{DedupeStrategy, FsSink};
use libviprs::{
    EngineBuilder, EngineKind, Layout, PixelFormat, PyramidPlan, PyramidPlanner, Raster,
    ResumePolicy,
};

const IMG: u32 = 512;
const TILE: u32 = 128;

fn plan() -> PyramidPlan {
    PyramidPlanner::new(IMG, IMG, TILE, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// A mostly-white raster with a tiny central feature so most tiles are blank.
/// Under `DedupeStrategy::Blanks` those tiles collapse into `_shared/`
/// placeholders + a `blank_references` map — exactly the state a resumed run
/// fails to reconstruct (issue #272).
fn blank_heavy_raster() -> Raster {
    let (w, h) = (IMG, IMG);
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0xFFu8; w as usize * h as usize * bpp];
    let (fx0, fy0) = (w / 2 - 8, h / 2 - 8);
    for y in fy0..fy0 + 16 {
        for x in fx0..fx0 + 16 {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = 0x10;
            data[off + 1] = 0x20;
            data[off + 2] = 0xF0;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Case 1: `ResumePolicy::resume()` + `FsSink::with_dedupe(Blanks)`.
///
/// The seed Overwrite run materialises `_shared/` placeholders + a
/// `blank_references` manifest. A subsequent Resume run with dedupe still
/// enabled would (today) skip every seeded tile, leave `manifest_refs` empty,
/// and have `finish()` overwrite `manifest.json` with a view that omits every
/// pre-crash placeholder mapping — orphaning them. The stopgap must refuse it.
#[test]
fn resume_plus_dedupe_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("out");
    let src = blank_heavy_raster();
    let plan = plan();

    // Seed: full run under Overwrite with dedupe -> tiles, _shared/, manifest,
    // and a checkpoint the Resume run below can pick up from.
    let seed = FsSink::new(base.clone(), plan.clone()).with_dedupe(DedupeStrategy::Blanks);
    EngineBuilder::new(&src, plan.clone(), &seed)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("seed overwrite+dedupe run should succeed");

    // Resume the same directory with dedupe still enabled.
    let sink = FsSink::new(base.clone(), plan.clone()).with_dedupe(DedupeStrategy::Blanks);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume())
        .run();

    assert!(
        result.is_err(),
        "resume + dedupe must be refused (issue #272 stopgap): got Ok — the run \
         silently proceeded and finish() would overwrite manifest.json from empty \
         per-run maps, orphaning every pre-crash placeholder tile"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("resume") && (msg.to_lowercase().contains("dedup") || msg.contains("272")),
        "error should explain the resume/dedupe incompatibility, got: {msg}"
    );
}

/// Case 2: `ResumePolicy::resume()` + per-tile checksums / manifest.
///
/// With `ChecksumMode::EmitOnly` the sink accumulates `tile_digests` in memory
/// and `finish()` writes them into `manifest.json::checksums`. A resumed run
/// skips pre-crash tiles, so their digests are never recomputed and the
/// overwritten manifest carries a truncated checksum table. The stopgap must
/// refuse it.
#[test]
fn resume_plus_checksum_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("out");
    let src = blank_heavy_raster();
    let plan = plan();

    let seed = FsSink::new(base.clone(), plan.clone())
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    EngineBuilder::new(&src, plan.clone(), &seed)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("seed overwrite+checksum run should succeed");

    let sink = FsSink::new(base.clone(), plan.clone())
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume())
        .run();

    assert!(
        result.is_err(),
        "resume + checksum must be refused (issue #272 stopgap): got Ok — the run \
         silently proceeded and finish() would overwrite manifest.json's checksum \
         table with a view missing every pre-crash tile"
    );
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("resume") && (msg.to_lowercase().contains("checksum") || msg.contains("272")),
        "error should explain the resume/checksum incompatibility, got: {msg}"
    );
}

/// Narrowness control: resume with NEITHER dedupe NOR checksum is the safe
/// path (no manifest is (re)written from per-run state), and must stay a
/// supported, `Ok` run both before and after the stopgap. This is green
/// against current core and must remain green after the guard lands.
#[test]
fn resume_without_dedupe_or_checksum_is_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("out");
    let src = blank_heavy_raster();
    let plan = plan();

    let seed = FsSink::new(base.clone(), plan.clone());
    EngineBuilder::new(&src, plan.clone(), &seed)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .expect("seed overwrite run should succeed");

    let sink = FsSink::new(base.clone(), plan.clone());
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume())
        .run();

    assert!(
        result.is_ok(),
        "resume without dedupe or checksum must remain supported (the stopgap must \
         not over-reach): {:?}",
        result.err()
    );
}
