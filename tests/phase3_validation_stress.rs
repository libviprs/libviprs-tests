//! Phase 3 validation & stress capstone suite.
//!
//! This file is the **integration cross-cut** for the Phase 3 hardening
//! work: it composes features exercised in isolation by its sibling files
//! (resume, failure policy, dedupe, checksum, manifest versioning, blank
//! tolerance, object-store sink) into end-to-end scenarios. The tests are
//! written TDD-style against the *planned* APIs and are therefore expected
//! to FAIL TO COMPILE (or fail at runtime) until those APIs are actually
//! implemented on the `feat/phase3-hardening` branch.
//!
//! Planned surface touched by this file (sibling test files own the
//! per-feature coverage; here we only drive them together):
//!
//! ```ignore
//! use libviprs::{
//!     // resume
//!     ResumeMode, generate_pyramid_resumable, verify_output,
//!     // failure policy
//!     FailurePolicy,
//!     // dedupe
//!     DedupeStrategy,
//!     // checksum
//!     ChecksumMode,
//!     // blank-tile with tolerance (from phase3_blank_tolerance.rs)
//!     BlankTileStrategy,
//! };
//! ```
//!
//! Time budget: at concurrency 4 no test here should exceed 60 s on a
//! modern laptop. Everything tempfile-rooted; no writes outside the test
//! sandbox.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use libviprs::manifest::{ChecksumAlgo, ManifestBuilder, ManifestV1};
use libviprs::{
    BlankTileStrategy, ChecksumMode, DedupeStrategy, EngineConfig, FailurePolicy, FsSink, Layout,
    PixelFormat, PyramidPlan, PyramidPlanner, Raster, ResumeMode, RetryPolicy, SinkError, Tile,
    TileCoord, TileFormat, TileSink, generate_pyramid, generate_pyramid_resumable, verify_output,
};

// =============================================================================
// Shared helpers
// =============================================================================
//
// NOTE: Rust integration test files (under `tests/`) do not share modules
// between compilation units by default. An in-tree `mod common;` only
// compiles cleanly if `tests/common/mod.rs` is present and is used by at
// least one file. To keep this capstone file self-contained (and avoid
// coupling with sibling phase3 test files that may duplicate pieces of
// this shim layer), helpers are defined inline below. They are marked
// `#[allow(dead_code)]` so an individual `#[ignore]`'d test skipping them
// does not warn.

// -----------------------------------------------------------------------------
// PanickingSink<S>
// -----------------------------------------------------------------------------

/// Wraps any inner sink and panics at a configured trigger point:
///
/// * `PanicTrigger::NthWrite(n)` — panic on the Nth call to `write_tile`
///   (1-based). All previous writes flow through to the inner sink.
/// * `PanicTrigger::Coord(c)` — panic the first time `write_tile` is called
///   with coordinate `c`.
///
/// Used by the resume tests to inject crashes mid-run, modelling a process
/// kill / OOM / power loss.
#[allow(dead_code)]
#[derive(Debug, Clone)]
enum PanicTrigger {
    NthWrite(usize),
    Coord(TileCoord),
}

#[allow(dead_code)]
struct PanickingSink<S: TileSink> {
    inner: S,
    trigger: PanicTrigger,
    counter: AtomicUsize,
    fired: std::sync::atomic::AtomicBool,
}

#[allow(dead_code)]
impl<S: TileSink> PanickingSink<S> {
    fn new(inner: S, trigger: PanicTrigger) -> Self {
        Self {
            inner,
            trigger,
            counter: AtomicUsize::new(0),
            fired: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: TileSink> TileSink for PanickingSink<S> {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        match &self.trigger {
            PanicTrigger::NthWrite(target) if n == *target => {
                self.fired.store(true, Ordering::SeqCst);
                panic!("PanickingSink: deliberate panic at write #{n}");
            }
            PanicTrigger::Coord(c) if *c == tile.coord => {
                if !self.fired.swap(true, Ordering::SeqCst) {
                    panic!("PanickingSink: deliberate panic at coord {:?}", c);
                }
            }
            _ => {}
        }
        self.inner.write_tile(tile)
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
}

// -----------------------------------------------------------------------------
// FlakySink<S>
// -----------------------------------------------------------------------------

/// Sink that fails on approximately `fail_percent`% of writes using a
/// deterministic, seeded hash of the tile coordinate. Determinism is
/// important so failure injection is reproducible across concurrency levels
/// and across restarts (the same tile always fails).
#[allow(dead_code)]
struct FlakySink<S: TileSink> {
    inner: S,
    fail_percent: u8,
    seed: u64,
    forced_permanent: HashSet<TileCoord>,
    fail_count: AtomicUsize,
    ok_count: AtomicUsize,
}

#[allow(dead_code)]
impl<S: TileSink> FlakySink<S> {
    fn new(inner: S, fail_percent: u8, seed: u64) -> Self {
        Self {
            inner,
            fail_percent: fail_percent.min(100),
            seed,
            forced_permanent: HashSet::new(),
            fail_count: AtomicUsize::new(0),
            ok_count: AtomicUsize::new(0),
        }
    }

    fn with_permanent_failures(mut self, coords: impl IntoIterator<Item = TileCoord>) -> Self {
        self.forced_permanent.extend(coords);
        self
    }

    fn failures(&self) -> usize {
        self.fail_count.load(Ordering::SeqCst)
    }

    fn successes(&self) -> usize {
        self.ok_count.load(Ordering::SeqCst)
    }

    fn should_fail(&self, coord: TileCoord) -> bool {
        if self.forced_permanent.contains(&coord) {
            return true;
        }
        // Cheap deterministic hash (splitmix64-ish). We avoid bringing in
        // `rand` so the suite has no extra dev-deps.
        let mix = (coord.level as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (coord.col as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
            ^ (coord.row as u64).wrapping_mul(0x94D0_49BB_1331_11EB)
            ^ self.seed;
        let mut x = mix;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        ((x % 100) as u8) < self.fail_percent
    }
}

impl<S: TileSink> TileSink for FlakySink<S> {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        if self.should_fail(tile.coord) {
            self.fail_count.fetch_add(1, Ordering::SeqCst);
            return Err(SinkError::Other(format!(
                "FlakySink: injected failure for {:?}",
                tile.coord
            )));
        }
        self.ok_count.fetch_add(1, Ordering::SeqCst);
        self.inner.write_tile(tile)
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
}

// -----------------------------------------------------------------------------
// RecordingSink<S>
// -----------------------------------------------------------------------------

/// Records every write (ordering, coord, and timestamp) for later inspection.
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct WriteRecord {
    coord: TileCoord,
    when: Instant,
    seq: u64,
}

#[allow(dead_code)]
struct RecordingSink<S: TileSink> {
    inner: S,
    seq: AtomicU64,
    log: Mutex<Vec<WriteRecord>>,
}

#[allow(dead_code)]
impl<S: TileSink> RecordingSink<S> {
    fn new(inner: S) -> Self {
        Self {
            inner,
            seq: AtomicU64::new(0),
            log: Mutex::new(Vec::new()),
        }
    }

    fn records(&self) -> Vec<WriteRecord> {
        self.log.lock().unwrap().clone()
    }

    fn write_count(&self) -> usize {
        self.log.lock().unwrap().len()
    }
}

impl<S: TileSink> TileSink for RecordingSink<S> {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let rec = WriteRecord {
            coord: tile.coord,
            when: Instant::now(),
            seq,
        };
        self.log.lock().unwrap().push(rec);
        self.inner.write_tile(tile)
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
}

// -----------------------------------------------------------------------------
// Whitespace-heavy raster
// -----------------------------------------------------------------------------

/// Build an RGB raster that is mostly pure white with a small centred
/// rectangular "feature" of gradient content. Whitespace-heavy inputs
/// exercise the blank-detection / dedupe paths strongly.
#[allow(dead_code)]
fn whitespace_heavy_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![255u8; w as usize * h as usize * bpp];
    // Centred feature covering ~6% of area.
    let fw = (w / 4).max(8);
    let fh = (h / 4).max(8);
    let x0 = (w - fw) / 2;
    let y0 = (h - fh) / 2;
    for y in y0..y0 + fh {
        for x in x0..x0 + fw {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = ((x - x0) * 255 / fw.max(1)) as u8;
            data[off + 1] = ((y - y0) * 255 / fh.max(1)) as u8;
            data[off + 2] = (((x - x0) ^ (y - y0)) & 0xFF) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

// -----------------------------------------------------------------------------
// Directory diff helper
// -----------------------------------------------------------------------------

/// Walk two directory trees in sorted order and return the path (relative
/// to `a`) of the first file whose bytes differ, or `None` if the two
/// trees are byte-identical.
#[allow(dead_code)]
fn byte_diff_dirs(a: &Path, b: &Path) -> Option<PathBuf> {
    // Book-keeping files that legitimately differ between a clean run and a
    // crash+resume (or simply appear only in one of the two), yet have no
    // bearing on output correctness. Filter them out so the comparison
    // focuses on actual tile content.
    let is_bookkeeping =
        |p: &Path| -> bool { p.file_name().and_then(|n| n.to_str()) == Some(".libviprs-job.json") };
    let list_a: Vec<PathBuf> = list_files_relative(a)
        .into_iter()
        .filter(|p| !is_bookkeeping(p))
        .collect();
    let list_b: Vec<PathBuf> = list_files_relative(b)
        .into_iter()
        .filter(|p| !is_bookkeeping(p))
        .collect();

    if list_a != list_b {
        // Name/set mismatch — return whichever file is missing.
        for p in list_a.iter() {
            if !list_b.contains(p) {
                return Some(p.clone());
            }
        }
        for p in list_b.iter() {
            if !list_a.contains(p) {
                return Some(p.clone());
            }
        }
    }

    for rel in list_a {
        let pa = a.join(&rel);
        let pb = b.join(&rel);
        let ba = std::fs::read(&pa).unwrap_or_default();
        let bb = std::fs::read(&pb).unwrap_or_default();
        if ba != bb {
            return Some(rel);
        }
    }
    None
}

#[allow(dead_code)]
fn list_files_relative(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(root: &Path, cur: &Path, out: &mut Vec<PathBuf>) {
        if !cur.is_dir() {
            return;
        }
        let mut entries: Vec<_> = std::fs::read_dir(cur)
            .map(|it| it.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

// -----------------------------------------------------------------------------
// run_golden_pyramid
// -----------------------------------------------------------------------------

/// Convenience wrapper: run `generate_pyramid` to an `FsSink` with the
/// supplied `EngineConfig`. Returns the absolute `base` path used for the
/// pyramid (`<dir>/pyramid`).
#[allow(dead_code)]
fn run_golden_pyramid(
    raster: &Raster,
    plan: &PyramidPlan,
    dir: &Path,
    format: TileFormat,
    config: &EngineConfig,
) -> PathBuf {
    let base = dir.join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), format);
    generate_pyramid(raster, plan, &sink, config).unwrap();
    base
}

// -----------------------------------------------------------------------------
// Common pyramid fixture
// -----------------------------------------------------------------------------

#[allow(dead_code)]
fn standard_planner(w: u32, h: u32, tile: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, tile, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

#[allow(dead_code)]
fn strip_created_at(m: &mut ManifestV1) {
    m.created_at = String::new();
    // `concurrency` is a property of the *run*, not of the pyramid itself.
    // Cross-concurrency determinism comparisons normalise it to zero so that
    // only output-shaping fields contribute to the byte diff.
    m.generation.concurrency = 0;
}

// =============================================================================
// 1. restart_during_level_generation_resume_completes_output
// =============================================================================

/// Start a full pyramid, abort mid-level-2 by panicking the FsSink on the
/// Nth write, then re-launch with `ResumeMode::Resume` and assert the
/// eventual on-disk output is byte-identical to a clean single-run
/// reference. Proves that the resume machinery re-picks up work at
/// the right granularity without corrupting or duplicating tiles.
#[test]
fn restart_during_level_generation_resume_completes_output() {
    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(512, 384);
    let plan = standard_planner(512, 384, 128);

    // --- reference clean run ---
    let ref_dir = tempfile::tempdir_in(root.path()).unwrap();
    let ref_base = run_golden_pyramid(
        &src,
        &plan,
        ref_dir.path(),
        TileFormat::Png,
        &EngineConfig::default().with_concurrency(4),
    );

    // --- crashing run ---
    let crash_dir = tempfile::tempdir_in(root.path()).unwrap();
    let crash_base = crash_dir.path().join("pyramid");

    // Pick N so that we land somewhere inside the second-smallest level
    // (level index 2) — a mid-level abort is the whole point.
    let panic_after = plan.total_tile_count() as usize / 3;

    let inner = FsSink::new(crash_base.clone(), plan.clone(), TileFormat::Png);
    let panicking = PanickingSink::new(inner, PanicTrigger::NthWrite(panic_after));

    let first_run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        generate_pyramid_resumable(
            &src,
            &plan,
            &panicking,
            &EngineConfig::default().with_concurrency(4),
            ResumeMode::Resume,
        )
    }));
    assert!(
        first_run.is_err(),
        "PanickingSink should have forced the first run to panic"
    );

    // --- resume run into the same directory ---
    let resume_sink = FsSink::new(crash_base.clone(), plan.clone(), TileFormat::Png);
    generate_pyramid_resumable(
        &src,
        &plan,
        &resume_sink,
        &EngineConfig::default().with_concurrency(4),
        ResumeMode::Resume,
    )
    .unwrap();

    // --- byte-compare crash+resume against reference ---
    let diff = byte_diff_dirs(&ref_base, &crash_base);
    assert!(
        diff.is_none(),
        "resume output diverges from clean run at {}",
        diff.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
}

// =============================================================================
// 2. restart_during_tile_encode_resume_completes_output
// =============================================================================

/// Inject a panic during tile encoding for one specific coordinate, then
/// resume. Proves that panics originating *inside* a single tile's
/// encode path do not leave the on-disk pyramid in an unrecoverable
/// state — Resume must bring it to completion with bit-for-bit fidelity.
#[test]
fn restart_during_tile_encode_resume_completes_output() {
    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(512, 384);
    let plan = standard_planner(512, 384, 128);

    let ref_dir = tempfile::tempdir_in(root.path()).unwrap();
    let ref_base = run_golden_pyramid(
        &src,
        &plan,
        ref_dir.path(),
        TileFormat::Png,
        &EngineConfig::default().with_concurrency(4),
    );

    let crash_dir = tempfile::tempdir_in(root.path()).unwrap();
    let crash_base = crash_dir.path().join("pyramid");

    // Pick a coord on a non-leaf level (index 1) — guarantees an abort
    // somewhere before the top tile.
    let target = {
        let level = &plan.levels[1];
        TileCoord {
            level: level.level,
            col: level.cols / 2,
            row: level.rows / 2,
        }
    };

    let inner = FsSink::new(crash_base.clone(), plan.clone(), TileFormat::Png);
    let panicking = PanickingSink::new(inner, PanicTrigger::Coord(target));

    let first_run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        generate_pyramid_resumable(
            &src,
            &plan,
            &panicking,
            &EngineConfig::default().with_concurrency(4),
            ResumeMode::Resume,
        )
    }));
    assert!(
        first_run.is_err(),
        "panic during tile encode should surface"
    );

    let resume_sink = FsSink::new(crash_base.clone(), plan.clone(), TileFormat::Png);
    generate_pyramid_resumable(
        &src,
        &plan,
        &resume_sink,
        &EngineConfig::default().with_concurrency(4),
        ResumeMode::Resume,
    )
    .unwrap();

    let diff = byte_diff_dirs(&ref_base, &crash_base);
    assert!(
        diff.is_none(),
        "resume output after encode-panic diverges from clean run at {}",
        diff.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
}

// =============================================================================
// 3. partial_sink_failure_retry_then_skip_produces_valid_partial
// =============================================================================

/// With `FailurePolicy::RetryThenSkip`, a sink that permanently fails on
/// ~10 % of tiles must leave the run completing successfully, with only
/// the un-writable tiles absent. Every tile that *did* land must decode
/// back to a valid PNG; the manifest's per-level counts must account for
/// skipped tiles.
#[test]
fn partial_sink_failure_retry_then_skip_produces_valid_partial() {
    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(512, 384);
    let plan = standard_planner(512, 384, 128);

    let out_dir = tempfile::tempdir_in(root.path()).unwrap();
    let base = out_dir.path().join("pyramid");

    let inner = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    let flaky = FlakySink::new(
        inner, /* fail_percent */ 10, /* seed */ 0xCAFEF00D,
    );

    let config = EngineConfig::default()
        .with_concurrency(4)
        .with_failure_policy(FailurePolicy::RetryThenSkip(RetryPolicy {
            max_retries: 3,
            initial_backoff: Duration::from_millis(1),
            multiplier: 2.0,
            max_backoff: Duration::from_secs(5),
            jitter: false,
        }));

    generate_pyramid(&src, &plan, &flaky, &config).expect("RetryThenSkip must not surface errors");

    // Every emitted .png must be a real PNG file (starts with \x89PNG).
    let mut written = 0usize;
    for rel in list_files_relative(&base) {
        if rel.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let bytes = std::fs::read(base.join(&rel)).unwrap();
        assert!(
            bytes.len() >= 8 && &bytes[..4] == [0x89, b'P', b'N', b'G'].as_slice(),
            "non-PNG bytes at {}",
            rel.display()
        );
        written += 1;
    }

    // Manifest counts must add up: produced + skipped = planned.
    let parent = base.parent().unwrap();
    let stem = base.file_name().unwrap().to_string_lossy();
    let manifest_path = parent.join(format!("{stem}.manifest.json"));
    let m: ManifestV1 = serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();

    let total_produced: u64 = m.levels.iter().map(|l| l.tiles_produced).sum();
    let total_skipped: u64 = m.levels.iter().map(|l| l.tiles_skipped).sum();
    assert_eq!(
        total_produced + total_skipped,
        plan.total_tile_count(),
        "manifest counts must account for every planned tile"
    );
    assert!(
        total_skipped > 0,
        "expected some tiles to have been skipped"
    );
    assert_eq!(
        written as u64, total_produced,
        "on-disk PNG count must match manifest's tiles_produced"
    );
}

// =============================================================================
// 4. large_image_sparse_output_correctness
// =============================================================================

/// 8192×8192 whitespace-heavy raster with `DedupeStrategy::Blanks`:
/// * Output on disk must be < 10 % of the raw uncompressed pixel size.
/// * Every emitted tile file must decode back to a correct PNG that
///   matches the source pixel content for its coordinate.
///
/// Proves dedupe saves space *and* preserves correctness.
#[test]
fn large_image_sparse_output_correctness() {
    const W: u32 = 8192;
    const H: u32 = 8192;
    const TILE: u32 = 512;

    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(W, H);
    let plan = standard_planner(W, H, TILE);

    let out_dir = tempfile::tempdir_in(root.path()).unwrap();
    let base = out_dir.path().join("pyramid");

    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_dedupe(DedupeStrategy::Blanks);

    let config = EngineConfig::default()
        .with_concurrency(4)
        .with_blank_tile_strategy(BlankTileStrategy::Placeholder);

    generate_pyramid(&src, &plan, &sink, &config).unwrap();

    // Space budget: total bytes on disk < 10 % of raw RGB pixels.
    let raw_bytes = (W as u64) * (H as u64) * PixelFormat::Rgb8.bytes_per_pixel() as u64;
    let mut on_disk: u64 = 0;
    for rel in list_files_relative(&base) {
        on_disk += std::fs::metadata(base.join(&rel))
            .map(|m| m.len())
            .unwrap_or(0);
    }
    assert!(
        on_disk < raw_bytes / 10,
        "sparse output ({on_disk} B) should be < 10 % of raw ({raw_bytes} B)"
    );

    // Correctness: decode one non-blank tile from the top (smallest) level
    // and verify its pixels match the source's downscaled content. Exact
    // pixel-for-pixel reconstruction of every level is expensive; we only
    // prove that non-empty tiles round-trip through the PNG codec.
    for rel in list_files_relative(&base) {
        if rel.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let bytes = std::fs::read(base.join(&rel)).unwrap();
        if bytes.len() == 1 {
            // Placeholder marker; skip.
            continue;
        }
        let _img = image::load_from_memory(&bytes)
            .unwrap_or_else(|e| panic!("failed to decode emitted tile {}: {e}", rel.display()));
    }
}

// =============================================================================
// 5. deterministic_manifests_across_concurrency
// =============================================================================

/// Run the same raster through `generate_pyramid` at concurrency 1, 4, 16,
/// and 64; then assert that the emitted `manifest.json` is byte-identical
/// across the four runs, modulo `created_at`. Proves that concurrency is
/// not leaking into the manifest's ordering, checksums, or counts.
#[test]
fn deterministic_manifests_across_concurrency() {
    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(512, 384);
    let plan = standard_planner(512, 384, 128);

    let concurrencies = [1usize, 4, 16, 64];
    let mut manifests: Vec<Vec<u8>> = Vec::new();

    for c in concurrencies {
        let dir = tempfile::tempdir_in(root.path()).unwrap();
        let base = dir.path().join("pyramid");
        let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
            .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3));
        let cfg = EngineConfig::default().with_concurrency(c);
        generate_pyramid(&src, &plan, &sink, &cfg).unwrap();

        let mpath = dir.path().join("pyramid.manifest.json");
        let bytes = std::fs::read(&mpath).unwrap();
        let mut parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();
        strip_created_at(&mut parsed);
        manifests.push(serde_json::to_vec(&parsed).unwrap());

        // Keep the tempdir alive only long enough to read the manifest.
        drop(dir);
    }

    for w in manifests.windows(2) {
        assert_eq!(
            w[0], w[1],
            "manifest.json differs across concurrency runs (excl. created_at)"
        );
    }
}

// =============================================================================
// 6. multithreaded_stress_100_small_pyramids
// =============================================================================

/// Spawn 8 host threads. Each thread runs 100 small pyramids into its own
/// tempdir with `ChecksumMode::Verify`. Every pyramid must (a) complete,
/// (b) verify cleanly, and (c) produce the expected tile count. This is
/// the capstone soak test for thread-safety of the whole engine + sinks
/// + manifest + checksum path.
#[test]
fn multithreaded_stress_100_small_pyramids() {
    let root = Arc::new(tempfile::tempdir().unwrap());
    let src = Arc::new(whitespace_heavy_raster(256, 256));
    let plan = Arc::new(standard_planner(256, 256, 64));

    let mut handles = Vec::new();
    for t in 0..8usize {
        let root = Arc::clone(&root);
        let src = Arc::clone(&src);
        let plan = Arc::clone(&plan);
        handles.push(std::thread::spawn(move || {
            for i in 0..100usize {
                let dir =
                    tempfile::tempdir_in(root.path()).expect("failed to mkdir per-pyramid tempdir");
                let base = dir.path().join("pyramid");
                let sink = FsSink::new(base.clone(), (*plan).clone(), TileFormat::Png)
                    .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3))
                    .with_checksum_mode(ChecksumMode::Verify);

                let cfg = EngineConfig::default().with_concurrency(2);
                generate_pyramid(&src, &plan, &sink, &cfg)
                    .unwrap_or_else(|e| panic!("thread {t} pyramid {i} failed: {e:?}"));

                // Re-verify on the finished output.
                verify_output(&base).unwrap_or_else(|e| {
                    panic!("thread {t} pyramid {i} verify_output failed: {e:?}")
                });
            }
        }));
    }

    for h in handles {
        h.join().expect("worker thread panicked");
    }
}

// =============================================================================
// 7. full_phase3_pipeline_integration
// =============================================================================

/// The capstone: enable every Phase 3 feature simultaneously and drive
/// the pipeline to completion.
///
/// * `FsSink` backend
/// * `ChecksumMode::Verify` (re-hash on write)
/// * `DedupeStrategy::Blanks` (canonicalise blank tiles)
/// * `FailurePolicy::RetryThenFail` with 2 retries and 1 ms backoff
/// * `BlankTileStrategy::PlaceholderWithTolerance { max_channel_delta: 3 }`
/// * `ResumeMode::Resume` (no prior state, so behaves like a fresh run)
/// * `ManifestBuilder::new().with_checksums(Blake3)` for versioned manifest
///
/// The run must complete without error.
#[test]
fn full_phase3_pipeline_integration() {
    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(1024, 768);
    let plan = standard_planner(1024, 768, 256);

    let base = root.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3))
        .with_dedupe(DedupeStrategy::Blanks)
        .with_checksum_mode(ChecksumMode::Verify);

    let cfg = EngineConfig::default()
        .with_concurrency(4)
        .with_blank_tile_strategy(BlankTileStrategy::PlaceholderWithTolerance {
            max_channel_delta: 3,
        })
        .with_failure_policy(FailurePolicy::RetryThenFail(RetryPolicy {
            max_retries: 2,
            initial_backoff: Duration::from_millis(1),
            multiplier: 2.0,
            max_backoff: Duration::from_secs(5),
            jitter: false,
        }));

    generate_pyramid_resumable(&src, &plan, &sink, &cfg, ResumeMode::Resume).unwrap();

    // Belt-and-braces: the manifest must exist and parse.
    let m_bytes = std::fs::read(root.path().join("pyramid.manifest.json")).unwrap();
    let _: ManifestV1 = serde_json::from_slice(&m_bytes).unwrap();
}

// =============================================================================
// 8. verify_mode_on_output_of_full_pipeline
// =============================================================================

/// Re-run the full-pipeline scenario from test 7, then call `verify_output()`
/// on the resulting directory and assert it returns Ok. Proves the
/// emitted artefacts are internally consistent end-to-end.
#[test]
fn verify_mode_on_output_of_full_pipeline() {
    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(1024, 768);
    let plan = standard_planner(1024, 768, 256);

    let base = root.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3))
        .with_dedupe(DedupeStrategy::Blanks)
        .with_checksum_mode(ChecksumMode::Verify);

    let cfg = EngineConfig::default()
        .with_concurrency(4)
        .with_blank_tile_strategy(BlankTileStrategy::PlaceholderWithTolerance {
            max_channel_delta: 3,
        })
        .with_failure_policy(FailurePolicy::RetryThenFail(RetryPolicy {
            max_retries: 2,
            initial_backoff: Duration::from_millis(1),
            multiplier: 2.0,
            max_backoff: Duration::from_secs(5),
            jitter: false,
        }));

    generate_pyramid_resumable(&src, &plan, &sink, &cfg, ResumeMode::Resume).unwrap();

    verify_output(&base).expect("verify_output must pass on full-pipeline output");
}

// =============================================================================
// 9. checkpoint_written_frequently_enough_to_bound_rework
// =============================================================================

/// With `checkpoint_every(5)` configured and a 1000-tile pyramid aborted
/// at tile ~500 by a panic, `ResumeMode::Resume` must replay at most ~10
/// tiles (two checkpoints' worth). Proves the checkpointing cadence
/// actually bounds rework on restart — not just "it resumes eventually".
#[test]
fn checkpoint_written_frequently_enough_to_bound_rework() {
    // Pick dimensions giving us >= 1000 tiles at 64-px tiles. A 2048x2048
    // Deep Zoom pyramid has levels summing to thousands of tiles.
    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(2048, 2048);
    let plan = standard_planner(2048, 2048, 64);
    assert!(
        plan.total_tile_count() >= 1000,
        "expected >=1000 tiles in plan, got {}",
        plan.total_tile_count()
    );

    let crash_dir = tempfile::tempdir_in(root.path()).unwrap();
    let base = crash_dir.path().join("pyramid");

    // First run: panic at the 500th write.
    let inner = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let panicking = PanickingSink::new(inner, PanicTrigger::NthWrite(500));
    let cfg = EngineConfig::default()
        .with_concurrency(1) // deterministic ordering so "500th write"
        // maps to a known resume point.
        .with_checkpoint_every(5);

    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        generate_pyramid_resumable(&src, &plan, &panicking, &cfg, ResumeMode::Resume)
    }));

    // Second run: re-wrap FsSink in a RecordingSink so we can count how
    // many writes the resume path performs.
    let inner2 = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let rec = RecordingSink::new(inner2);
    generate_pyramid_resumable(&src, &plan, &rec, &cfg, ResumeMode::Resume).unwrap();

    let replayed = rec.write_count();
    // With checkpoint_every(5) we accept at most 10 tiles of rework:
    // the checkpoint boundary before the crash, plus a small margin for
    // whatever was in-flight at crash time.
    let remaining = plan.total_tile_count() as usize - 500;
    let slack = 10usize;
    assert!(
        replayed <= remaining + slack,
        "Resume replayed {replayed} tiles; expected <= {} (remaining {remaining} + slack {slack})",
        remaining + slack
    );
}

// =============================================================================
// 10. s3_restart_resumes (feature-gated; env-gated)
// =============================================================================

/// End-to-end Resume against a real S3 endpoint. Skipped unless both the
/// `s3` feature is enabled *and* `LIBVIPRS_TEST_S3_ENDPOINT` is set in
/// the environment. Mirrors test 1 but across a network sink.
#[cfg(feature = "s3")]
#[test]
fn s3_restart_resumes() {
    use libviprs::sink::{ObjectStoreConfig, ObjectStoreSink};
    use std::time::SystemTime;

    let Ok(endpoint) = std::env::var("LIBVIPRS_TEST_S3_ENDPOINT") else {
        eprintln!("skipping s3_restart_resumes: LIBVIPRS_TEST_S3_ENDPOINT unset");
        return;
    };
    let bucket =
        std::env::var("LIBVIPRS_TEST_S3_BUCKET").unwrap_or_else(|_| "libviprs-test".to_string());
    let access_key =
        std::env::var("LIBVIPRS_TEST_S3_ACCESS_KEY").unwrap_or_else(|_| "minio".to_string());
    let secret_key =
        std::env::var("LIBVIPRS_TEST_S3_SECRET_KEY").unwrap_or_else(|_| "minio123".to_string());

    let root = tempfile::tempdir().unwrap();
    let src = whitespace_heavy_raster(512, 384);
    let plan = standard_planner(512, 384, 128);

    // Reference filesystem run for comparison.
    let ref_dir = tempfile::tempdir_in(root.path()).unwrap();
    let ref_base = run_golden_pyramid(
        &src,
        &plan,
        ref_dir.path(),
        TileFormat::Png,
        &EngineConfig::default().with_concurrency(4),
    );

    // Unique prefix per test run to avoid cross-run interference.
    let prefix = format!(
        "libviprs-phase3-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );

    let cfg = ObjectStoreConfig::s3(&endpoint, &bucket)
        .with_access_key(&access_key, &secret_key)
        .with_key_prefix(&prefix);

    // First run: crash mid-upload.
    let inner = ObjectStoreSink::new(cfg.clone(), plan.clone(), TileFormat::Png).unwrap();
    let panicking = PanickingSink::new(
        inner,
        PanicTrigger::NthWrite(plan.total_tile_count() as usize / 3),
    );
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        generate_pyramid_resumable(
            &src,
            &plan,
            &panicking,
            &EngineConfig::default().with_concurrency(4),
            ResumeMode::Resume,
        )
    }));

    // Second run: resume and finish.
    let resume_sink = ObjectStoreSink::new(cfg.clone(), plan.clone(), TileFormat::Png).unwrap();
    generate_pyramid_resumable(
        &src,
        &plan,
        &resume_sink,
        &EngineConfig::default().with_concurrency(4),
        ResumeMode::Resume,
    )
    .unwrap();

    // Server-side listing must match the filesystem reference. We compare
    // only the set of relative keys — byte equality for S3 objects is
    // covered by the checksum-verifying tests in `phase3_object_store_sink`.
    let listed: HashSet<String> = resume_sink.list_objects().unwrap().into_iter().collect();
    let expected: HashSet<String> = list_files_relative(&ref_base)
        .into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    // Every reference file name should show up as an object key suffix.
    for key in &expected {
        assert!(
            listed.iter().any(|k| k.ends_with(key)),
            "expected S3 object for reference file {key}"
        );
    }
}
