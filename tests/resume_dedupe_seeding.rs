//! Issue #272 (P0, data corruption) — the REAL crash-resume seeding fix.
//!
//! The sibling `resume_dedupe_unsupported.rs` pins the #450 *stopgap*: today a
//! resumed run combined with content deduplication or per-tile checksums is
//! HARD-ERRORED, because the engine reconstructs the checkpoint but NOT the
//! sink-side manifest/dedupe state (`manifest_refs` / `tile_digests` / the
//! `DedupeIndex`). `ResumeAwareSink` short-circuits already-completed
//! coordinates before they reach `FsSink::write_tile`, so for every pre-crash
//! tile those maps start empty and `finish()` overwrites `manifest.json` from
//! the incomplete view — orphaning every pre-crash 1-byte placeholder and
//! truncating the checksum table.
//!
//! This file is the acceptance test for the real fix: on resume the sink SEEDS
//! that state from the (re-rendered) pre-crash tiles, so a resumed
//! dedupe+checksum run reproduces the SAME complete manifest + dedupe layout as
//! an uninterrupted run. It runs a dedupe+checksum pyramid to completion
//! (reference), then runs the SAME job interrupted partway and resumed, and
//! asserts the resumed output's `blank_references`, per-tile checksum table and
//! on-disk dedupe layout are byte-identical to the reference.
//!
//! RED / GREEN:
//! * RED against current core (`../libviprs`, which carries the #450 stopgap):
//!   the resume run returns `Err(ResumeUnsupportedWith)`, so the
//!   `.expect("... must succeed")` on the resumed run fails.
//! * GREEN against a core that has the seeding fix (verify with a
//!   `--config 'paths=[...]'` override pointing at the fix worktree): the
//!   resume run completes and the two trees are identical.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use libviprs::checksum::{ChecksumAlgo, ChecksumMode};
use libviprs::manifest::ManifestBuilder;
use libviprs::sink::{DedupeStrategy, FsSink};
use libviprs::{
    EngineBuilder, EngineConfig, EngineError, EngineKind, EngineResult, Layout, PixelFormat,
    PyramidPlan, PyramidPlanner, Raster, ResumePolicy, SinkError, Tile, TileSink,
};

const IMG: u32 = 512;
const TILE: u32 = 128;

fn plan() -> PyramidPlan {
    PyramidPlanner::new(IMG, IMG, TILE, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Mostly-white raster with a small central feature, so the vast majority of
/// tiles are uniform white and collapse under `DedupeStrategy::Blanks` into a
/// single `_shared/` blob + many 1-byte placeholders recorded in
/// `blank_references` — exactly the state a resumed run must reconstruct.
fn blank_heavy_raster() -> Raster {
    let (w, h) = (IMG, IMG);
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

// ---------------------------------------------------------------------------
// PanickingSink: crash the run at the Nth write to model a kill / OOM.
// ---------------------------------------------------------------------------

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
    fn checkpoint_root(&self) -> Option<&Path> {
        self.inner.checkpoint_root()
    }
}

// ---------------------------------------------------------------------------
// Tree / manifest comparison helpers.
// ---------------------------------------------------------------------------

fn list_files_relative(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(cur: &Path, root: &Path, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<_> = std::fs::read_dir(cur)
            .map(|it| it.filter_map(|e| e.ok()).collect())
            .unwrap_or_default();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else {
                out.push(p.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    walk(root, root, &mut out);
    out.sort();
    out
}

/// A file that legitimately differs (or appears in only one tree) between a
/// clean run and a crash+resume, with no bearing on tile-content correctness:
/// the resume bookkeeping and `manifest.json` (whose `created_at` timestamp
/// always differs — its semantic fields are compared separately).
fn is_excluded(rel: &Path) -> bool {
    matches!(
        rel.file_name().and_then(|n| n.to_str()),
        Some(".libviprs-job.json")
            | Some(".libviprs-job.segments")
            | Some(".libviprs-job.lock")
            | Some("manifest.json")
    ) || rel
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".manifest.json"))
}

/// Byte-compare two output trees excluding bookkeeping + the manifest. Returns
/// the first offending relative path (missing on one side, or differing bytes).
fn tile_tree_diff(a: &Path, b: &Path) -> Option<PathBuf> {
    let la: Vec<PathBuf> = list_files_relative(a)
        .into_iter()
        .filter(|p| !is_excluded(p))
        .collect();
    let lb: Vec<PathBuf> = list_files_relative(b)
        .into_iter()
        .filter(|p| !is_excluded(p))
        .collect();
    if la != lb {
        for p in la.iter() {
            if !lb.contains(p) {
                return Some(p.clone());
            }
        }
        for p in lb.iter() {
            if !la.contains(p) {
                return Some(p.clone());
            }
        }
    }
    la.into_iter()
        .find(|rel| std::fs::read(a.join(rel)).ok() != std::fs::read(b.join(rel)).ok())
}

fn read_manifest_json(base: &Path) -> serde_json::Value {
    let raw = std::fs::read(base.join("manifest.json"))
        .unwrap_or_else(|e| panic!("manifest.json missing at {}: {e}", base.display()));
    serde_json::from_slice(&raw).expect("manifest.json must be valid JSON")
}

fn blank_references(m: &serde_json::Value) -> BTreeMap<String, String> {
    m.get("blank_references")
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn checksum_table(m: &serde_json::Value) -> BTreeMap<String, String> {
    m.get("checksums")
        .and_then(|v| v.get("per_tile"))
        .and_then(|v| v.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn count_one_byte_files(dir: &Path) -> usize {
    let mut n = 0;
    for rel in list_files_relative(dir) {
        if std::fs::metadata(dir.join(&rel))
            .map(|m| m.len() == 1)
            .unwrap_or(false)
        {
            n += 1;
        }
    }
    n
}

// ---------------------------------------------------------------------------
// Run drivers.
// ---------------------------------------------------------------------------

/// Clean, uninterrupted reference run (no resume) with dedupe + checksums.
fn run_reference(base: &Path, src: &Raster, plan: &PyramidPlan) {
    let sink = FsSink::new(base.to_path_buf(), plan.clone())
        .with_dedupe(DedupeStrategy::Blanks)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default().with_concurrency(1))
        .run()
        .expect("clean reference dedupe+checksum run must succeed");
}

fn resume_run<S: TileSink>(
    sink: S,
    src: &Raster,
    plan: &PyramidPlan,
) -> Result<EngineResult, EngineError> {
    EngineBuilder::new(src, plan.clone(), sink)
        .with_engine(EngineKind::Monolithic)
        .with_resume(ResumePolicy::resume().with_checkpoint_every(4))
        .with_config(EngineConfig::default().with_concurrency(1))
        .run()
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

/// A dedupe + per-tile-checksum pyramid, crashed partway and resumed, must
/// produce a `blank_references` map, a per-tile checksum table, and an on-disk
/// dedupe layout byte-identical to a clean single-run reference.
#[test]
fn resumed_dedupe_checksum_output_matches_uninterrupted() {
    let root = tempfile::tempdir().unwrap();
    let src = blank_heavy_raster();
    let plan = plan();
    let total = plan.total_tile_count() as usize;
    assert!(total >= 8, "fixture must have enough tiles to crash midway");

    // --- reference: clean run to completion ---
    let ref_dir = tempfile::tempdir_in(root.path()).unwrap();
    let ref_base = ref_dir.path().join("out");
    run_reference(&ref_base, &src, &plan);

    // The reference must actually exercise dedupe (a `_shared/` blob + at least
    // one 1-byte placeholder) or the test would be vacuous.
    assert!(
        ref_base.join("_shared").is_dir(),
        "reference run did not create a _shared/ dedupe directory"
    );
    let ref_manifest = read_manifest_json(&ref_base);
    assert!(
        !blank_references(&ref_manifest).is_empty(),
        "reference blank_references is empty — dedupe never fired"
    );
    assert!(
        !checksum_table(&ref_manifest).is_empty(),
        "reference checksum table is empty — checksums never recorded"
    );

    // --- crash run: same job, crash at the halfway write ---
    let crash_dir = tempfile::tempdir_in(root.path()).unwrap();
    let crash_base = crash_dir.path().join("out");

    let inner = FsSink::new(crash_base.clone(), plan.clone())
        .with_dedupe(DedupeStrategy::Blanks)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    let panicking = PanickingSink::new(inner, total / 2);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resume_run(&panicking, &src, &plan)
    }));

    // --- resume run into the same directory ---
    let resume_sink = FsSink::new(crash_base.clone(), plan.clone())
        .with_dedupe(DedupeStrategy::Blanks)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    // RED here under the #450 stopgap: resume + dedupe/checksum returns
    // Err(ResumeUnsupportedWith); GREEN once seeding lands.
    let result = resume_run(&resume_sink, &src, &plan)
        .expect("resumed dedupe+checksum run must succeed once seeding lands (issue #272)");

    // The resume must genuinely have seeded pre-crash tiles rather than doing a
    // fresh full run — otherwise it never exercises the seeding path.
    assert!(
        (result.tiles_produced as usize) < total,
        "resume produced {} of {total} tiles — it re-ran everything and never \
         resumed from a checkpoint, so seeding was not exercised",
        result.tiles_produced
    );

    // --- assertions: resumed output == reference output ---
    let diff = tile_tree_diff(&ref_base, &crash_base);
    assert!(
        diff.is_none(),
        "resumed dedupe layout diverges from the clean reference at {} — a \
         pre-crash placeholder / shared blob was not reconstructed",
        diff.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );

    let crash_manifest = read_manifest_json(&crash_base);
    assert_eq!(
        blank_references(&crash_manifest),
        blank_references(&ref_manifest),
        "resumed blank_references differs from the reference — pre-crash \
         placeholder references were orphaned (issue #272)"
    );
    assert_eq!(
        checksum_table(&crash_manifest),
        checksum_table(&ref_manifest),
        "resumed per-tile checksum table differs from the reference — pre-crash \
         digests were dropped (issue #272)"
    );
    assert_eq!(
        count_one_byte_files(&crash_base),
        count_one_byte_files(&ref_base),
        "resumed run has a different number of 1-byte placeholders than the \
         reference"
    );
}

/// Per-tile checksums *without* dedupe: a crashed+resumed run must reproduce
/// the full checksum table (no truncation of pre-crash digests).
#[test]
fn resumed_checksum_only_table_matches_uninterrupted() {
    let root = tempfile::tempdir().unwrap();
    let src = blank_heavy_raster();
    let plan = plan();
    let total = plan.total_tile_count() as usize;

    // Reference clean run: manifest + checksums, no dedupe.
    let ref_dir = tempfile::tempdir_in(root.path()).unwrap();
    let ref_base = ref_dir.path().join("out");
    let ref_sink = FsSink::new(ref_base.clone(), plan.clone())
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    EngineBuilder::new(&src, plan.clone(), &ref_sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default().with_concurrency(1))
        .run()
        .expect("clean reference checksum run must succeed");
    let ref_manifest = read_manifest_json(&ref_base);
    assert_eq!(
        checksum_table(&ref_manifest).len(),
        total,
        "reference checksum table should cover every tile"
    );

    // Crash + resume.
    let crash_dir = tempfile::tempdir_in(root.path()).unwrap();
    let crash_base = crash_dir.path().join("out");
    let inner = FsSink::new(crash_base.clone(), plan.clone())
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    let panicking = PanickingSink::new(inner, total / 2);
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resume_run(&panicking, &src, &plan)
    }));

    let resume_sink = FsSink::new(crash_base.clone(), plan.clone())
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
    resume_run(&resume_sink, &src, &plan)
        .expect("resumed checksum run must succeed once seeding lands (issue #272)");

    let crash_manifest = read_manifest_json(&crash_base);
    assert_eq!(
        checksum_table(&crash_manifest),
        checksum_table(&ref_manifest),
        "resumed checksum table was truncated relative to the reference (issue #272)"
    );
    let diff = tile_tree_diff(&ref_base, &crash_base);
    assert!(
        diff.is_none(),
        "resumed checksum-only tree diverges from reference at {}",
        diff.as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
}
