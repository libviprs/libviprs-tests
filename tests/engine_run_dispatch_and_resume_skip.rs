//! Acceptance tests for the `EngineBuilder::run` dispatch refactor (issue #290)
//! and the resume re-render characterisation (issue #455).
//!
//! # Issue #290 — one shared `(EngineKind × EngineSource)` dispatch
//!
//! `EngineBuilder::run_collect` used to spell the ~10-arm engine dispatch out
//! THREE times (plain, resume, and a read-only Verify variant), with the
//! seven-argument `build_mapreduce_config` call copied verbatim eight times.
//! The refactor folds the render dispatch into one `RenderDispatch::run` shared
//! by the plain and resume paths and the Verify dispatch into one
//! `dispatch_verify`. Because it is a behaviour-preserving change, the oracle is
//! that every routed cell still produces the SAME, correct output:
//!
//! * every render arm matches a `vips dzsave` golden at the full-resolution
//!   level (pixel-equal, tolerance documented below) — a genuine
//!   vips-differential golden proving each dispatch arm reaches a working
//!   engine (`dispatch_every_engine_matches_vips_golden`);
//! * the resume Overwrite path and an idempotent Resume reproduce the plain
//!   path byte-for-byte, across every engine — proving the resume dispatch arm
//!   is the same routing as the plain one
//!   (`resume_paths_reproduce_plain_output`);
//! * Verify succeeds for every engine and writes nothing
//!   (`verify_succeeds_for_every_engine`).
//!
//! The vips reference under
//! `tests/fixtures/engine_run_dispatch_and_resume_skip_expected/` was generated
//! offline with vips 8.18.4:
//!
//! ```text
//! vips extract_band canonical_input.png engine_run_dispatch_input.png 0 --n 3
//! vips dzsave engine_run_dispatch_input.png engine_run_dispatch_golden \
//!   --layout dz --tile-size 128 --overlap 0 --suffix .png --strip
//! ```
//!
//! # Issue #455 — skipped tiles are re-rendered on a dedupe/checksum resume
//!
//! `resumed_dedupe_checksum_reseeds_skipped_tiles` pins the SAFE half of resume
//! (the #272 seeding fix: a resumed dedupe+checksum run reproduces the clean
//! reference output) AND characterises the perf gap #455 targets: on resume the
//! already-completed coordinates are still driven back through the sink's write
//! path (`seed_completed_tile`), i.e. re-rendered/re-seeded rather than reused
//! from persisted refs/digests. The counting sink observes those re-seeds so a
//! future fix that stops re-rendering them will visibly flip this count.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use libviprs::checksum::{ChecksumAlgo, ChecksumMode};
use libviprs::manifest::ManifestBuilder;
use libviprs::sink::{DedupeStrategy, FsSink};
use libviprs::streaming::RasterStripSource;
use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, EngineResult, Layout, PixelFormat, PyramidPlan,
    PyramidPlanner, Raster, ResumePolicy, SinkError, Tile, TileSink,
};

const INPUT_PNG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/engine_run_dispatch_input.png"
);

const EXPECTED_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/engine_run_dispatch_and_resume_skip_expected"
);

/// Per-channel tolerance for the vips comparison. Measured max drift across
/// every engine × source arm of this fixture is exactly 0 — libviprs reproduces
/// the `vips dzsave` golden pixel-for-pixel at every level — so the golden is
/// asserted pixel-equal. A gross dispatch error (routing a source/kind to the
/// wrong engine) changes the tile count or the pixels, and this bound catches
/// either.
const VIPS_TOL: u8 = 0;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn load_input() -> Raster {
    libviprs::decode_file(Path::new(INPUT_PNG)).expect("decode engine_run_dispatch_input.png")
}

/// 256×256 at tile 128, DeepZoom — nine levels (0..=8), twelve tiles, matching
/// the committed vips golden.
fn dispatch_plan() -> PyramidPlan {
    PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom)
        .expect("planner")
        .plan()
}

/// The render arms exercised by the dispatch tests. Monolithic accepts only an
/// in-memory raster; the streaming engines accept either source kind, so each
/// is run once per source to cover both `EngineSource` arms of the dispatch.
const RENDER_ARMS: &[(EngineKind, bool)] = &[
    (EngineKind::Monolithic, false),
    (EngineKind::Streaming, false),
    (EngineKind::Streaming, true),
    (EngineKind::MapReduce, false),
    (EngineKind::MapReduce, true),
    (EngineKind::MapReduceHotCache, false),
    (EngineKind::MapReduceHotCache, true),
];

fn arm_label(kind: EngineKind, use_strip: bool) -> String {
    format!("{kind:?}/{}", if use_strip { "strip" } else { "raster" })
}

/// Run a plain (non-resume) pyramid for one dispatch arm into `sink`.
fn run_plain(
    kind: EngineKind,
    use_strip: bool,
    raster: &Raster,
    plan: &PyramidPlan,
    sink: &FsSink,
) {
    let result = if use_strip {
        let strip = RasterStripSource::new(raster);
        EngineBuilder::new(strip, plan.clone(), sink)
            .with_engine(kind)
            .with_config(EngineConfig::default())
            .run()
    } else {
        EngineBuilder::new(raster, plan.clone(), sink)
            .with_engine(kind)
            .with_config(EngineConfig::default())
            .run()
    }
    .unwrap_or_else(|e| panic!("plain run failed for {}: {e}", arm_label(kind, use_strip)));
    assert_eq!(
        result.tiles_produced,
        plan.total_tile_count(),
        "{}: tiles_produced mismatch",
        arm_label(kind, use_strip)
    );
}

/// Run a resume pyramid for one dispatch arm into `sink` with the given policy.
fn run_resume(
    kind: EngineKind,
    use_strip: bool,
    raster: &Raster,
    plan: &PyramidPlan,
    sink: &FsSink,
    policy: ResumePolicy,
) -> EngineResult {
    let outcome = if use_strip {
        let strip = RasterStripSource::new(raster);
        EngineBuilder::new(strip, plan.clone(), sink)
            .with_engine(kind)
            .with_resume(policy)
            .run()
    } else {
        EngineBuilder::new(raster, plan.clone(), sink)
            .with_engine(kind)
            .with_resume(policy)
            .run()
    };
    outcome.unwrap_or_else(|e| panic!("resume run failed for {}: {e}", arm_label(kind, use_strip)))
}

// ---------------------------------------------------------------------------
// Pixel / tree comparison helpers (mirrors blueprint_portrait_pyramid.rs style)
// ---------------------------------------------------------------------------

fn collect_png(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        if !dir.is_dir() {
            return;
        }
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("png") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                out.push((rel, std::fs::read(&path).unwrap()));
            }
        }
    }
    walk(dir, dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn decode_png(data: &[u8]) -> (u32, u32, Vec<u8>) {
    let img =
        image::load_from_memory_with_format(data, image::ImageFormat::Png).expect("decode PNG");
    (img.width(), img.height(), img.into_bytes())
}

/// Compare `actual` tiles against `expected` (vips golden). libviprs pads edge
/// tiles to full tile size with white background; vips writes natural-size
/// tiles. Compare the overlapping region within `tol` and require any padding
/// to be solid white — identical contract to the blueprint fixture comparison.
fn assert_tiles_match(
    expected: &[(String, Vec<u8>)],
    actual: &[(String, Vec<u8>)],
    ctx: &str,
    tol: u8,
) {
    assert_eq!(
        expected.len(),
        actual.len(),
        "{ctx}: tile count mismatch (vips {}, libviprs {})",
        expected.len(),
        actual.len()
    );
    let mut max_diff = 0u8;
    for ((ep, eb), (ap, ab)) in expected.iter().zip(actual.iter()) {
        assert_eq!(ep, ap, "{ctx}: tile path mismatch ({ep} vs {ap})");
        let (ew, eh, epx) = decode_png(eb);
        let (aw, ah, apx) = decode_png(ab);
        assert!(
            aw >= ew && ah >= eh,
            "{ctx}: libviprs tile {ap} ({aw}x{ah}) smaller than vips ({ew}x{eh})"
        );
        let ch = epx.len() / (ew as usize * eh as usize);
        for y in 0..eh as usize {
            let er = y * ew as usize * ch;
            let ar = y * aw as usize * ch;
            for x in 0..ew as usize * ch {
                let d = (epx[er + x] as i16 - apx[ar + x] as i16).unsigned_abs() as u8;
                max_diff = max_diff.max(d);
                assert!(d <= tol, "{ctx}: pixel diff {d} > {tol} at {ap} row {y}");
            }
        }
        if aw > ew {
            for y in 0..ah as usize {
                let s = y * aw as usize * ch + ew as usize * ch;
                let e = y * aw as usize * ch + aw as usize * ch;
                assert!(
                    apx[s..e].iter().all(|&b| b == 255),
                    "{ctx}: right padding not white in {ap}"
                );
            }
        }
        if ah > eh {
            for y in eh as usize..ah as usize {
                let s = y * aw as usize * ch;
                let e = s + aw as usize * ch;
                assert!(
                    apx[s..e].iter().all(|&b| b == 255),
                    "{ctx}: bottom padding not white in {ap}"
                );
            }
        }
    }
    eprintln!("{ctx}: max pixel diff = {max_diff} (tol {tol})");
}

/// Every relative file path under `root` (sorted), excluding resume bookkeeping
/// and the timestamped manifest, so two trees differing only in those compare
/// equal.
fn tile_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        let mut es: Vec<_> = std::fs::read_dir(dir)
            .map(|it| it.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        es.sort_by_key(|e| e.file_name());
        for e in es {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    walk(root, root, &mut out);
    out.retain(|p| {
        !matches!(
            p.file_name().and_then(|n| n.to_str()),
            Some(".libviprs-job.json")
                | Some(".libviprs-job.segments")
                | Some(".libviprs-job.lock")
                | Some("manifest.json")
        )
    });
    out.sort();
    out
}

/// First relative path whose bytes differ between the two trees (or that is
/// present in only one), ignoring bookkeeping / manifest files.
fn tree_diff(a: &Path, b: &Path) -> Option<PathBuf> {
    let (la, lb) = (tile_files(a), tile_files(b));
    if la != lb {
        return la
            .iter()
            .find(|p| !lb.contains(p))
            .or_else(|| lb.iter().find(|p| !la.contains(p)))
            .cloned();
    }
    la.into_iter()
        .find(|rel| std::fs::read(a.join(rel)).ok() != std::fs::read(b.join(rel)).ok())
}

// ---------------------------------------------------------------------------
// Instrumented sinks
// ---------------------------------------------------------------------------

/// Transparent wrapper counting how many tiles were WRITTEN vs SEEDED (the
/// resume re-render path, `seed_completed_tile`). Provides `inner_sink` so every
/// engine-bookkeeping hook forwards to the wrapped `FsSink` automatically.
struct CountingSink {
    inner: FsSink,
    writes: AtomicUsize,
    seeds: AtomicUsize,
}

impl CountingSink {
    fn new(inner: FsSink) -> Self {
        Self {
            inner,
            writes: AtomicUsize::new(0),
            seeds: AtomicUsize::new(0),
        }
    }
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
    fn seeds(&self) -> usize {
        self.seeds.load(Ordering::SeqCst)
    }
}

impl TileSink for CountingSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.write_tile(tile)
    }
    fn seed_completed_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.seeds.fetch_add(1, Ordering::SeqCst);
        self.inner.seed_completed_tile(tile)
    }
    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
    fn inner_sink(&self) -> Option<&dyn TileSink> {
        Some(&self.inner)
    }
}

/// Panics on the Nth `write_tile` to model a kill / OOM mid-run.
struct PanickingSink<S: TileSink> {
    inner: S,
    panic_at: usize,
    counter: AtomicUsize,
    fired: AtomicBool,
}

impl<S: TileSink> PanickingSink<S> {
    fn new(inner: S, panic_at: usize) -> Self {
        Self {
            inner,
            panic_at,
            counter: AtomicUsize::new(0),
            fired: AtomicBool::new(false),
        }
    }
}

impl<S: TileSink> TileSink for PanickingSink<S> {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.panic_at && !self.fired.swap(true, Ordering::SeqCst) {
            panic!("PanickingSink: deliberate crash at write #{n}");
        }
        self.inner.write_tile(tile)
    }
    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
    fn inner_sink(&self) -> Option<&dyn TileSink> {
        Some(&self.inner)
    }
}

// ===========================================================================
// Issue #290 — dispatch correctness
// ===========================================================================

/// Every `(EngineKind × EngineSource)` render arm routes to a working engine
/// whose output matches the vips dzsave golden. A gross routing bug in the
/// refactored dispatch would change the tile count or the pixels.
#[test]
fn dispatch_every_engine_matches_vips_golden() {
    let raster = load_input();
    let plan = dispatch_plan();
    let expected = collect_png(Path::new(EXPECTED_DIR));
    assert_eq!(expected.len(), 12, "vips golden should have 12 tiles");

    for &(kind, use_strip) in RENDER_ARMS {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("out");
        let sink = FsSink::new(base.clone(), plan.clone());
        run_plain(kind, use_strip, &raster, &plan, &sink);
        let actual = collect_png(&base);
        assert_tiles_match(&expected, &actual, &arm_label(kind, use_strip), VIPS_TOL);
    }
}

/// The resume Overwrite path and an idempotent Resume reproduce the plain
/// path's output byte-for-byte, for every engine. This is the crux of #290:
/// the resume dispatch and the plain dispatch are now the SAME routing, so they
/// cannot diverge per engine kind or source.
#[test]
fn resume_paths_reproduce_plain_output() {
    let raster = load_input();
    let plan = dispatch_plan();

    for &(kind, use_strip) in RENDER_ARMS {
        let label = arm_label(kind, use_strip);

        // Plain reference.
        let plain_dir = tempfile::tempdir().unwrap();
        let plain_base = plain_dir.path().join("out");
        let plain_sink = FsSink::new(plain_base.clone(), plan.clone());
        run_plain(kind, use_strip, &raster, &plan, &plain_sink);

        // Resume Overwrite into a fresh directory.
        let ow_dir = tempfile::tempdir().unwrap();
        let ow_base = ow_dir.path().join("out");
        let ow_sink = FsSink::new(ow_base.clone(), plan.clone());
        let ow = run_resume(
            kind,
            use_strip,
            &raster,
            &plan,
            &ow_sink,
            ResumePolicy::overwrite(),
        );
        assert_eq!(
            ow.tiles_produced,
            plan.total_tile_count(),
            "{label}: overwrite tile count"
        );
        assert!(
            tree_diff(&plain_base, &ow_base).is_none(),
            "{label}: resume-overwrite tree differs from plain at {:?}",
            tree_diff(&plain_base, &ow_base)
        );

        // Idempotent Resume over the now-complete directory: nothing to write.
        let again = run_resume(
            kind,
            use_strip,
            &raster,
            &plan,
            &ow_sink,
            ResumePolicy::resume(),
        );
        assert_eq!(
            again.tiles_produced, 0,
            "{label}: resume of a complete job must write no tiles"
        );
        assert!(
            tree_diff(&plain_base, &ow_base).is_none(),
            "{label}: idempotent resume mutated the tree"
        );
    }
}

/// Verify routes through the single `dispatch_verify` site for every engine:
/// it reads back a seeded pyramid, writes nothing, and reports zero tiles.
#[test]
fn verify_succeeds_for_every_engine() {
    let raster = load_input();
    let plan = dispatch_plan();

    for &(kind, use_strip) in RENDER_ARMS {
        let label = arm_label(kind, use_strip);
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("out");
        let sink = FsSink::new(base.clone(), plan.clone());

        // Seed a full pyramid first (Overwrite), then Verify it.
        run_resume(
            kind,
            use_strip,
            &raster,
            &plan,
            &sink,
            ResumePolicy::overwrite(),
        );
        let before = tile_files(&base);
        let v = run_resume(
            kind,
            use_strip,
            &raster,
            &plan,
            &sink,
            ResumePolicy::verify(),
        );
        assert_eq!(
            v.tiles_produced, 0,
            "{label}: verify must not produce tiles"
        );
        assert_eq!(
            before,
            tile_files(&base),
            "{label}: verify mutated the tree"
        );
    }
}

// ===========================================================================
// Issue #455 — skipped tiles are re-rendered / re-seeded on a dedupe resume
// ===========================================================================

const RES_IMG: u32 = 512;
const RES_TILE: u32 = 128;

fn resume_plan() -> PyramidPlan {
    PyramidPlanner::new(RES_IMG, RES_IMG, RES_TILE, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Mostly-white raster with a small central feature so most tiles are uniform
/// and collapse under `DedupeStrategy::Blanks` — the state a resumed run must
/// reconstruct.
fn blank_heavy_raster() -> Raster {
    let (w, h) = (RES_IMG, RES_IMG);
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0xFFu8; w as usize * h as usize * bpp];
    let (fx0, fy0) = (w / 2 - 12, h / 2 - 12);
    for y in fy0..fy0 + 24 {
        for x in fx0..fx0 + 24 {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = 0x10;
            data[off + 1] = 0x20;
            data[off + 2] = 0xF0;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

fn dedupe_checksum_sink(base: &Path, plan: &PyramidPlan) -> FsSink {
    FsSink::new(base.to_path_buf(), plan.clone())
        .with_dedupe(DedupeStrategy::Blanks)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3)
}

fn read_manifest(base: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap()
}

/// A dedupe + per-tile-checksum pyramid crashed partway and resumed reproduces
/// the clean reference output (the #272 seeding fix — SAFE half), AND the
/// resume drives every already-completed coordinate back through the sink's
/// write path via `seed_completed_tile` rather than reusing persisted
/// refs/digests. The second assertion characterises the #455 perf gap: those
/// seeds ARE the re-render this issue proposes to eliminate by persisting
/// refs/digests in the segment log. A future fix that stops re-rendering
/// skipped tiles will drive `seeds` toward zero and flip this expectation.
#[test]
fn resumed_dedupe_checksum_reseeds_skipped_tiles() {
    let src = blank_heavy_raster();
    let plan = resume_plan();
    let total = plan.total_tile_count() as usize;
    assert!(total >= 8, "fixture must have enough tiles to crash midway");

    // Reference: clean uninterrupted run.
    let ref_dir = tempfile::tempdir().unwrap();
    let ref_base = ref_dir.path().join("out");
    let ref_sink = dedupe_checksum_sink(&ref_base, &plan);
    EngineBuilder::new(&src, plan.clone(), &ref_sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default().with_concurrency(1))
        .run()
        .expect("clean reference run");
    assert!(
        ref_base.join("_shared").is_dir(),
        "reference did not dedupe"
    );

    // Crash the same job at the halfway write.
    let crash_dir = tempfile::tempdir().unwrap();
    let crash_base = crash_dir.path().join("out");
    let panicking = PanickingSink::new(dedupe_checksum_sink(&crash_base, &plan), total / 2);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        EngineBuilder::new(&src, plan.clone(), &panicking)
            .with_engine(EngineKind::Monolithic)
            .with_resume(ResumePolicy::resume().with_checkpoint_every(4))
            .with_config(EngineConfig::default().with_concurrency(1))
            .run()
    }));

    // Resume into the crashed directory through the counting sink.
    let counting = CountingSink::new(dedupe_checksum_sink(&crash_base, &plan));
    let result = EngineBuilder::new(&src, plan.clone(), &counting)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume().with_checkpoint_every(4))
        .with_config(EngineConfig::default().with_concurrency(1))
        .run()
        .expect("resumed dedupe+checksum run must succeed (issue #272 seeding)");

    // SAFE half (#272): the resume genuinely skipped pre-crash writes and the
    // output is byte-identical to the clean reference.
    assert!(
        (result.tiles_produced as usize) < total,
        "resume wrote {} of {total} tiles — it re-ran everything, seeding not exercised",
        result.tiles_produced
    );
    assert!(
        tree_diff(&ref_base, &crash_base).is_none(),
        "resumed dedupe tree diverges from the clean reference at {:?}",
        tree_diff(&ref_base, &crash_base)
    );
    assert_eq!(
        read_manifest(&crash_base).get("blank_references"),
        read_manifest(&ref_base).get("blank_references"),
        "resumed blank_references differs from reference"
    );

    // #455 characterisation: the skipped (pre-crash) coordinates were re-seeded
    // through the sink's write path rather than reused from persisted state.
    // These seeds are exactly the re-render #455 proposes to eliminate.
    assert!(
        counting.seeds() > 0,
        "expected the resume to re-seed already-completed tiles (issue #455 re-render path)"
    );
    assert_eq!(
        counting.writes() + counting.seeds(),
        total,
        "every planned coordinate is either written fresh or re-seeded on resume — \
         the resume still visits (renders) all {total} tiles (issue #455)"
    );
}
