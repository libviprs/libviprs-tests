//! Kill-and-resume determinism harness — the in-repo exit gate for the resume
//! cluster (issue #278, proving #272 / #273 / #275 / #277).
//!
//! The crate's marquee claim (`resume.rs`: "a clean run and a crash+resume run
//! produce byte-identical trees") had no in-repo acceptance test. The 27 ported
//! integration files pin API shape; loom modelled only the MPSC queue. Every
//! shipped concurrency bug in the resume cluster lived on the checkpoint / dedupe
//! seam, where CI stayed green while resume could corrupt (#272) or diverge
//! (#275). This harness closes that gap from the **public builder API**.
//!
//! For each of three sink configurations — plain, content-addressed dedupe, and
//! per-tile checksums — it:
//!
//!   1. renders a full pyramid uninterrupted (the reference), and
//!   2. renders the same pyramid, `panic!`s a wrapping sink mid-run to simulate
//!      an abrupt process death at several distinct crash points, resumes, and
//!   3. asserts the resumed output tree — every tile's bytes, the `_shared/`
//!      dedupe blobs, the `blank_references` map, and the per-tile checksum map —
//!      is BYTE-IDENTICAL to the reference.
//!
//! Crash points (checkpoint cadence = [`CHECKPOINT_EVERY`]):
//!   * **post-checkpoint** — just after the first periodic flush certifies a
//!     delta (`CHECKPOINT_EVERY + 1`): resume must seed the already-certified
//!     tiles and reproduce identical manifest/dedupe state (#272).
//!   * **mid-level, uncertified tail** — a count that is not a flush boundary, so
//!     the last written-but-uncertified tiles are re-rendered on resume.
//!   * **late** — deep into the run, exercising resume of most of the pyramid.
//!
//! What proves what:
//!   * **#272** — a resumed dedupe/checksum run reconstructs the sink-side
//!     manifest (`blank_references`, `checksums.per_tile`) that `finish()` emits,
//!     rather than overwriting it with a view that omits every pre-crash tile.
//!   * **#275** — the dedupe on-disk placement (which tile holds the full payload
//!     vs a 1-byte placeholder) is identical across the clean and resumed runs.
//!   * **#277** — a single checkpoint authority drives resume; the run completes
//!     and its output is consistent.
//!
//! The pure fsync-before-certify ordering of **#273** (a certified tile's bytes
//! are always durable) is invisible in on-disk state and needs an injected
//! `Durability` backend, which the crate exposes only to its own in-module tests;
//! it is model-checked instead by the in-core loom model
//! (`src/loom_checkpoint_dedupe.rs::loom_checkpoint_never_certifies_unsynced_tile`).
//!
//! Determinism is kept observable by running single-threaded
//! (`with_concurrency(0)`): the panic lands on a fixed tile, and the reference is
//! reproducible. Cross-schedule dedupe determinism under `tile_concurrency > 0`
//! is covered by `dedupe_deterministic_layout.rs` and the loom model.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use libviprs::checksum::{ChecksumAlgo, ChecksumMode};
use libviprs::manifest::ManifestBuilder;
use libviprs::sink::{DedupeStrategy, FsSink, SinkError, Tile, TileFormat, TileSink};
use libviprs::{
    BlankTileStrategy, EngineBuilder, EngineConfig, EngineKind, Layout, PixelFormat, PyramidPlan,
    PyramidPlanner, Raster, ResumePolicy,
};

/// Checkpoint cadence used across the harness. Small so a mid-run crash always
/// leaves a partial-but-usable checkpoint to resume from.
const CHECKPOINT_EVERY: u64 = 3;

const IMG: u32 = 512;
const TILE: u32 = 128;

// ---------------------------------------------------------------------------
// Fixture + sink configuration
// ---------------------------------------------------------------------------

/// A mostly-white raster with a small coloured feature in the centre, so most
/// tiles of the pyramid are identical blank whites the dedupe path collapses
/// onto a shared blob (mirrors `dedupe_deterministic_layout.rs`). Content is a
/// pure function of coordinate, so every render is byte-reproducible.
fn whitespace_heavy_raster() -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0xFFu8; IMG as usize * IMG as usize * bpp];
    let (fx0, fy0) = (IMG / 2 - 24, IMG / 2 - 24);
    for y in fy0..fy0 + 48 {
        for x in fx0..fx0 + 48 {
            let off = (y as usize * IMG as usize + x as usize) * bpp;
            data[off] = 0x10;
            data[off + 1] = 0x20;
            data[off + 2] = 0xF0;
        }
    }
    Raster::new(IMG, IMG, PixelFormat::Rgb8, data).unwrap()
}

fn make_plan() -> PyramidPlan {
    PyramidPlanner::new(IMG, IMG, TILE, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

#[derive(Clone, Copy, Debug)]
enum Config {
    /// Plain filesystem tiles, no dedupe, no checksums, no manifest.
    Plain,
    /// Content-addressed dedupe (`DedupeStrategy::Blanks`) → `_shared/` blobs +
    /// `blank_references`.
    Dedupe,
    /// Per-tile checksums emitted + verified, carried in `manifest.json`.
    Checksum,
}

impl Config {
    /// Build a fresh terminal [`FsSink`] for this configuration.
    fn build_sink(self, base: &Path, plan: &PyramidPlan) -> FsSink {
        let sink = FsSink::new(base.to_path_buf(), plan.clone()).with_format(TileFormat::Png);
        match self {
            Config::Plain => sink,
            Config::Dedupe => sink.with_dedupe(DedupeStrategy::Blanks),
            Config::Checksum => sink
                .with_manifest(ManifestBuilder::new())
                .with_checksums(ChecksumMode::Verify, ChecksumAlgo::Blake3),
        }
    }
}

/// Engine config shared by every run: single-threaded (deterministic crash
/// point + reproducible reference), Emit blank strategy (uniform tiles are real
/// PNGs the dedupe path collapses via promote-on-second-hit, exercising the
/// #275 canonicalisation), and the small checkpoint cadence.
fn engine_config() -> EngineConfig {
    EngineConfig::default()
        .with_concurrency(0)
        .with_blank_tile_strategy(BlankTileStrategy::Emit)
        .with_checkpoint_every(CHECKPOINT_EVERY)
}

// ---------------------------------------------------------------------------
// Crashing sink
// ---------------------------------------------------------------------------

/// Wraps a real [`FsSink`] and `panic!`s on the Nth `write_tile` to simulate an
/// abrupt process death mid-run. Every engine bookkeeping hook reaches the inner
/// sink through [`TileSink::inner_sink`]; only `write_tile` (to crash) and
/// `finish` (never reached before the crash, forwarded for completeness) are
/// overridden.
struct CrashingSink {
    inner: FsSink,
    writes: AtomicUsize,
    crash_on: usize,
}

impl CrashingSink {
    fn new(inner: FsSink, crash_on: usize) -> Self {
        Self {
            inner,
            writes: AtomicUsize::new(0),
            crash_on,
        }
    }
}

impl TileSink for CrashingSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        let n = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        if n == self.crash_on {
            panic!(
                "CrashingSink: simulated abrupt process death on tile #{n} ({:?})",
                tile.coord
            );
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

// ---------------------------------------------------------------------------
// Output-tree snapshot + comparison
// ---------------------------------------------------------------------------

/// A canonical, comparable view of a finished output directory:
/// * `files` — `rel_path -> bytes` for every regular file EXCEPT the volatile
///   `manifest.json` (its `created_at` timestamp is legitimately
///   non-deterministic) and the internal checkpoint files
///   (`.libviprs-job.*`, whose segment order/timestamps are run-specific and
///   are engine state, not output).
/// * `blank_references` — the dedupe placeholder→shared map from the manifest.
/// * `checksums` — the per-tile digest map from the manifest.
#[derive(Debug, PartialEq, Eq)]
struct TreeSnapshot {
    files: BTreeMap<String, Vec<u8>>,
    blank_references: BTreeMap<String, String>,
    checksums: BTreeMap<String, String>,
}

fn is_checkpoint_file(rel: &str) -> bool {
    Path::new(rel)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(".libviprs-job"))
}

fn snapshot_tree(dir: &Path) -> TreeSnapshot {
    let mut files = BTreeMap::new();
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel == "manifest.json" || is_checkpoint_file(&rel) {
                    continue;
                }
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    walk(dir, dir, &mut files);

    let (blank_references, checksums) = read_manifest_maps(dir);
    TreeSnapshot {
        files,
        blank_references,
        checksums,
    }
}

/// Parse `manifest.json` (if present) and extract the two deterministic maps we
/// compare: `blank_references` and `checksums.per_tile`.
fn read_manifest_maps(dir: &Path) -> (BTreeMap<String, String>, BTreeMap<String, String>) {
    let manifest = dir.join("manifest.json");
    let Ok(raw) = std::fs::read(&manifest) else {
        return (BTreeMap::new(), BTreeMap::new());
    };
    let v: serde_json::Value = serde_json::from_slice(&raw).expect("manifest.json parses");

    let as_string_map = |val: Option<&serde_json::Value>| -> BTreeMap<String, String> {
        val.and_then(|o| o.as_object())
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };

    let blank_references = as_string_map(v.get("blank_references"));
    let checksums = as_string_map(v.pointer("/checksums/per_tile"));
    (blank_references, checksums)
}

// ---------------------------------------------------------------------------
// Run drivers
// ---------------------------------------------------------------------------

/// Render the full pyramid uninterrupted into `base` — the reference tree.
fn run_clean(base: &Path, plan: &PyramidPlan, cfg: Config) {
    let src = whitespace_heavy_raster();
    let sink = cfg.build_sink(base, plan);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::overwrite().with_checkpoint_every(CHECKPOINT_EVERY))
        .with_config(engine_config())
        .run()
        .expect("clean reference run must succeed");
}

/// Crash the run at `crash_on`, then resume it to completion in `base`.
fn run_crash_then_resume(base: &Path, plan: &PyramidPlan, cfg: Config, crash_on: usize) {
    let src = whitespace_heavy_raster();

    // Phase 1: Overwrite run that dies on the `crash_on`-th tile write.
    let crashing = CrashingSink::new(cfg.build_sink(base, plan), crash_on);
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        EngineBuilder::new(&src, plan.clone(), &crashing)
            .with_engine(EngineKind::Monolithic)
            .with_resume(ResumePolicy::overwrite().with_checkpoint_every(CHECKPOINT_EVERY))
            .with_config(engine_config())
            .run()
    }));
    assert!(
        crashed.is_err() || matches!(crashed, Ok(Err(_))),
        "the crashing run must abort before finishing (crash_on={crash_on})"
    );
    drop(crashing);

    // Phase 2: fresh sink + Resume off the on-disk checkpoint, to completion.
    let resume_sink = cfg.build_sink(base, plan);
    EngineBuilder::new(&src, plan.clone(), &resume_sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume().with_checkpoint_every(CHECKPOINT_EVERY))
        .with_config(engine_config())
        .run()
        .unwrap_or_else(|e| panic!("resume after crash_on={crash_on} must succeed: {e:?}"));
}

/// Crash points spanning the distinct categories the harness targets, given the
/// total tile count `total` and the checkpoint cadence.
fn crash_points(total: usize) -> Vec<(usize, &'static str)> {
    let ce = CHECKPOINT_EVERY as usize;
    let mut pts = Vec::new();
    // Just after the first periodic flush certifies a delta.
    pts.push((ce + 1, "post-checkpoint"));
    // A non-boundary count so the last written tiles are uncertified.
    let mid = (total / 2) | 1; // force odd -> not a multiple of 3 for total/2 near a boundary
    if mid > ce + 1 && mid < total {
        pts.push((mid, "mid-level-uncertified"));
    }
    // Deep into the run.
    let late = total - 2;
    if late > mid && late < total {
        pts.push((late, "late"));
    }
    pts
}

/// The core assertion: a crash-and-resume at every crash point yields a
/// byte-identical output tree to the uninterrupted reference.
fn assert_resume_is_byte_identical(cfg: Config) {
    let plan = make_plan();
    let total = plan.total_tile_count() as usize;
    assert!(
        total > (CHECKPOINT_EVERY as usize) * 3,
        "plan must have enough tiles ({total}) for meaningful crash points"
    );

    // Reference: one uninterrupted run.
    let ref_dir = tempfile::tempdir().unwrap();
    let ref_base = ref_dir.path().join("out");
    run_clean(&ref_base, &plan, cfg);
    let reference = snapshot_tree(&ref_base);

    // Sanity: dedupe/checksum configs actually populated their maps, so the
    // comparison is not vacuously satisfied by two empty maps.
    match cfg {
        Config::Dedupe => {
            assert!(
                ref_base.join("_shared").is_dir() && !reference.blank_references.is_empty(),
                "dedupe reference must have produced _shared/ blobs and blank_references"
            );
        }
        Config::Checksum => assert_eq!(
            reference.checksums.len(),
            total,
            "checksum reference must record one digest per tile"
        ),
        Config::Plain => {}
    }

    for (crash_on, label) in crash_points(total) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("out");
        run_crash_then_resume(&base, &plan, cfg, crash_on);
        let resumed = snapshot_tree(&base);

        assert_eq!(
            resumed.files.keys().collect::<Vec<_>>(),
            reference.files.keys().collect::<Vec<_>>(),
            "{cfg:?}/{label} (crash_on={crash_on}): resumed output has a different \
             SET of files than the uninterrupted run",
        );
        for (rel, ref_bytes) in &reference.files {
            assert!(
                resumed.files.get(rel) == Some(ref_bytes),
                "{cfg:?}/{label} (crash_on={crash_on}): file {rel} differs between \
                 the resumed run and the uninterrupted run (resume is not \
                 byte-deterministic)",
            );
        }
        assert_eq!(
            resumed.blank_references, reference.blank_references,
            "{cfg:?}/{label} (crash_on={crash_on}): manifest blank_references \
             diverged on resume (issue #272 / #275)",
        );
        assert_eq!(
            resumed.checksums, reference.checksums,
            "{cfg:?}/{label} (crash_on={crash_on}): manifest checksum map \
             diverged on resume (issue #272)",
        );
    }
}

// ---------------------------------------------------------------------------
// Tests — one per configuration for attributable failures
// ---------------------------------------------------------------------------

/// Plain tiles: a crash-and-resume reproduces every tile byte-identically.
#[test]
fn plain_resume_is_byte_identical_to_clean_run() {
    assert_resume_is_byte_identical(Config::Plain);
}

/// Content-addressed dedupe: the resumed `_shared/` layout, full-payload vs
/// placeholder placement, and `blank_references` map are byte-identical to the
/// uninterrupted run (issues #272 + #275).
#[test]
fn dedupe_resume_is_byte_identical_to_clean_run() {
    assert_resume_is_byte_identical(Config::Dedupe);
}

/// Per-tile checksums: the resumed manifest's checksum map covers every tile and
/// matches the uninterrupted run — the sink-side manifest state is reconstructed
/// on resume rather than truncated to post-crash tiles (issue #272).
#[test]
fn checksum_resume_is_byte_identical_to_clean_run() {
    assert_resume_is_byte_identical(Config::Checksum);
}

// Silence unused warnings if a helper is trimmed during review.
#[allow(dead_code)]
fn _unused(_: PathBuf) {}
