//! Phase 3 hardening: failing integration tests for per-tile checksum
//! verification.
//!
//! These tests are written TDD-style. The `libviprs::checksum` module, the
//! `FsSink::with_checksums` wiring, the `verify_output` entry point, and the
//! `EngineError::ChecksumMismatch` variant do not yet exist, so this file is
//! expected to FAIL TO COMPILE until the API is implemented on the
//! `feat/phase3-hardening` branch of `libviprs`.
//!
//! Planned surface:
//! ```ignore
//! use libviprs::checksum::{ChecksumMode, ChecksumAlgo, verify_output};
//!
//! pub enum ChecksumAlgo { Blake3, Sha256 }
//! pub enum ChecksumMode { None, EmitOnly, Verify }
//!
//! let sink = FsSink::new(dir, plan, fmt)
//!     .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);
//!
//! pub fn verify_output(dir: &Path) -> Result<VerifyReport, VerifyError>;
//!
//! pub struct VerifyReport {
//!     pub tiles_checked: u64,
//!     pub tiles_ok: u64,
//!     pub tiles_mismatched: Vec<PathBuf>,
//!     pub tiles_missing: Vec<PathBuf>,
//! }
//! ```
//!
//! Tests exercised:
//!  1. `no_checksums_by_default`
//!  2. `emit_only_populates_manifest_checksums`
//!  3. `checksum_algo_blake3_default`
//!  4. `checksum_algo_sha256_opt_in`
//!  5. `checksums_deterministic_across_runs`
//!  6. `verify_mode_fails_on_corrupted_tile`
//!  7. `verify_mode_detects_missing_tile`
//!  8. `verify_mode_passes_on_intact_output`
//!  9. `verify_happens_inline_when_checksum_mode_verify`
//! 10. `sha256_hex_length`
//! 11. `checksums_work_with_jpeg_and_raw_formats`
//! 12. `checksum_of_blank_placeholder_is_stable`

use libviprs::checksum::{ChecksumAlgo, ChecksumMode, verify_output};
use libviprs::manifest::ManifestBuilder;
use libviprs::sink::BLANK_TILE_MARKER;
use libviprs::sink::{SinkError, Tile, TileSink};
use libviprs::source::generate_test_raster;
use libviprs::{
    BlankTileStrategy, EngineConfig, EngineError, FsSink, Layout, PyramidPlanner, TileFormat,
    generate_pyramid,
};

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Shared parameters
// ---------------------------------------------------------------------------

const IMG_W: u32 = 256;
const IMG_H: u32 = 192;
const TILE_SIZE: u32 = 64;
const OVERLAP: u32 = 0;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Flips a single byte in-place at a well-known offset in `path`. For tests
/// we target byte index 0 for raw tiles and an offset deep enough inside
/// PNG/JPEG payloads to avoid clobbering magic bytes (tests assert on
/// checksum mismatch, not on decoder behaviour, so any byte inside the file
/// is fine as long as the file length is > offset).
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
    // Pick an offset past any format magic.  For tiny placeholder files
    // (1 byte) just flip index 0.
    let off = if buf.len() < 16 { 0 } else { 12 };
    buf[off] ^= 0xFF;
    use std::io::Seek;
    f.seek(std::io::SeekFrom::Start(0)).unwrap();
    f.write_all(&buf).unwrap();
    f.set_len(buf.len() as u64).unwrap();
}

/// Read the `manifest.json` that the sink is expected to emit alongside the
/// DZI XML and the per-tile checksum map. Mirrors the layout used by
/// `phase3_manifest_version.rs`.
fn read_manifest_json(dir: &Path) -> serde_json::Value {
    let parent = dir.parent().unwrap();
    let stem = dir.file_name().unwrap().to_string_lossy().to_string();

    let sibling = parent.join(format!("{stem}.manifest.json"));
    if sibling.exists() {
        let bytes = std::fs::read(&sibling).unwrap();
        return serde_json::from_slice(&bytes).unwrap();
    }

    let inside = dir.join("manifest.json");
    if inside.exists() {
        let bytes = std::fs::read(&inside).unwrap();
        return serde_json::from_slice(&bytes).unwrap();
    }

    panic!(
        "manifest.json not found. Checked {} and {}",
        sibling.display(),
        inside.display()
    );
}

/// Walk `dir` and return every regular file (relative to `dir`), skipping the
/// manifest and DZI. Sorted for determinism.
fn collect_tile_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(root: &Path, cur: &Path, out: &mut Vec<PathBuf>) {
        if !cur.is_dir() {
            return;
        }
        for entry in std::fs::read_dir(cur).unwrap() {
            let entry = entry.unwrap();
            let p = entry.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_path_buf();
                let fname = rel.file_name().unwrap().to_string_lossy().to_string();
                if fname == "manifest.json" {
                    continue;
                }
                out.push(rel);
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

/// Boilerplate: build a fresh pyramid under `base` with a given checksum
/// configuration and tile format. Returns the plan so callers can reason
/// about tile counts.
fn run_pyramid(
    base: &Path,
    fmt: TileFormat,
    mode: ChecksumMode,
    algo: ChecksumAlgo,
    config: EngineConfig,
) -> libviprs::planner::PyramidPlan {
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let sink = FsSink::new(base.to_path_buf(), plan.clone())
        .with_format(fmt)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(mode, algo);

    generate_pyramid(&src, &plan, &sink, &config).unwrap();
    plan
}

// ---------------------------------------------------------------------------
// 1. no_checksums_by_default
// ---------------------------------------------------------------------------

/// Tests that without `.with_checksums(...)` the manifest's `checksums`
/// field is `null`/absent. Works by emitting a manifest via the default
/// `ManifestBuilder` and asserting the JSON does not carry a populated
/// `checksums` object.
#[test]
fn no_checksums_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");

    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    // No `.with_checksums(...)` call.
    let sink = FsSink::new(base.clone(), plan.clone()).with_manifest(ManifestBuilder::new());
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let value = read_manifest_json(&base);
    match value.get("checksums") {
        None => {}
        Some(serde_json::Value::Null) => {}
        Some(other) => {
            panic!("expected manifest.checksums to be absent/null by default, got {other}")
        }
    }
}

// ---------------------------------------------------------------------------
// 2. emit_only_populates_manifest_checksums
// ---------------------------------------------------------------------------

/// Tests that `ChecksumMode::EmitOnly` causes the manifest to carry a
/// populated `checksums.per_tile` map with one entry per written tile.
/// Works by running a small pyramid, reading the manifest, and asserting
/// the map length equals the plan's total tile count.
#[test]
fn emit_only_populates_manifest_checksums() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");

    let plan = run_pyramid(
        &base,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );

    let value = read_manifest_json(&base);
    let per_tile = value
        .pointer("/checksums/per_tile")
        .and_then(|v| v.as_object())
        .expect("manifest.checksums.per_tile should be a JSON object");
    assert_eq!(
        per_tile.len() as u64,
        plan.total_tile_count(),
        "expected one checksum per emitted tile"
    );
    for (path, digest) in per_tile {
        let digest = digest
            .as_str()
            .unwrap_or_else(|| panic!("digest for {path} not a string"));
        assert!(!digest.is_empty(), "empty digest for {path}");
    }
}

// ---------------------------------------------------------------------------
// 3. checksum_algo_blake3_default
// ---------------------------------------------------------------------------

/// Tests that when `ChecksumAlgo::Blake3` is requested, the manifest
/// serializes `checksums.algo == "blake3"`. Works by inspecting the JSON
/// and asserting the string form.
#[test]
fn checksum_algo_blake3_default() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");

    let _ = run_pyramid(
        &base,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );

    let value = read_manifest_json(&base);
    let algo = value
        .pointer("/checksums/algo")
        .and_then(|v| v.as_str())
        .expect("manifest.checksums.algo should be a string");
    assert_eq!(algo, "blake3");
}

// ---------------------------------------------------------------------------
// 4. checksum_algo_sha256_opt_in
// ---------------------------------------------------------------------------

/// Tests that switching to `ChecksumAlgo::Sha256` makes the manifest
/// serialize `checksums.algo == "sha256"`. Works by running the same
/// pyramid twice with different algos and asserting each manifest's
/// `algo` field matches the requested variant.
#[test]
fn checksum_algo_sha256_opt_in() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");

    let _ = run_pyramid(
        &base,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Sha256,
        EngineConfig::default(),
    );

    let value = read_manifest_json(&base);
    let algo = value
        .pointer("/checksums/algo")
        .and_then(|v| v.as_str())
        .expect("manifest.checksums.algo should be a string");
    assert_eq!(algo, "sha256");
}

// ---------------------------------------------------------------------------
// 5. checksums_deterministic_across_runs
// ---------------------------------------------------------------------------

/// Tests that two independent runs with identical inputs and settings
/// produce byte-identical per-tile checksum maps. Works by running the
/// same pyramid twice into separate tempdirs and asserting the sorted
/// per_tile JSON objects are equal.
#[test]
fn checksums_deterministic_across_runs() {
    fn map_of(base: &Path) -> serde_json::Map<String, serde_json::Value> {
        let value = read_manifest_json(base);

        value
            .pointer("/checksums/per_tile")
            .and_then(|v| v.as_object())
            .expect("per_tile missing")
            .clone()
    }

    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let b1 = d1.path().join("pyramid");
    let b2 = d2.path().join("pyramid");

    let _ = run_pyramid(
        &b1,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );
    let _ = run_pyramid(
        &b2,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );

    let m1 = map_of(&b1);
    let m2 = map_of(&b2);

    assert_eq!(m1.len(), m2.len(), "per_tile map sizes differ");
    for (k, v) in &m1 {
        let other = m2
            .get(k)
            .unwrap_or_else(|| panic!("tile {k} missing in second run"));
        assert_eq!(v, other, "digest for {k} differs across runs");
    }

    // Also confirm the serialized bytes are identical (order-insensitive
    // via BTreeMap-backed serialization — implementation detail, so we
    // compare as sorted JSON objects).
    let s1 = serde_json::to_vec(&serde_json::Value::Object(m1)).unwrap();
    let s2 = serde_json::to_vec(&serde_json::Value::Object(m2)).unwrap();
    assert_eq!(s1, s2, "per_tile JSON not byte-identical across runs");
}

// ---------------------------------------------------------------------------
// 6. verify_mode_fails_on_corrupted_tile
// ---------------------------------------------------------------------------

/// Tests that `verify_output` on a previously-emitted pyramid whose tile
/// files have been mutated after the fact reports the corrupted tile in
/// `tiles_mismatched`. Works by running `EmitOnly`, flipping one byte in
/// a known tile, and inspecting the `VerifyReport`.
#[test]
fn verify_mode_fails_on_corrupted_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");
    let plan = run_pyramid(
        &base,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );

    // Pick any real tile on disk and flip a byte.
    let top = plan.levels.last().unwrap();
    let rel = plan
        .tile_path(
            libviprs::TileCoord {
                level: top.level,
                col: 0,
                row: 0,
            },
            TileFormat::Png.extension(),
        )
        .unwrap();
    let victim = base.join(&rel);
    assert!(victim.exists(), "victim tile missing: {}", victim.display());
    corrupt_file(&victim);

    let report = verify_output(&base).expect("verify_output should succeed at reading");
    assert_eq!(report.tiles_checked, plan.total_tile_count());
    assert_eq!(
        report.tiles_ok,
        plan.total_tile_count() - 1,
        "every tile except the corrupted one should pass"
    );
    assert!(
        report.tiles_mismatched.iter().any(|p| p.ends_with(&rel)),
        "expected corrupted tile {} in tiles_mismatched: {:?}",
        rel,
        report.tiles_mismatched,
    );
    assert!(
        report.tiles_missing.is_empty(),
        "no tiles should be missing, got {:?}",
        report.tiles_missing
    );
}

// ---------------------------------------------------------------------------
// 7. verify_mode_detects_missing_tile
// ---------------------------------------------------------------------------

/// Tests that `verify_output` flags deleted tile files in `tiles_missing`.
/// Works by running `EmitOnly`, deleting a known tile, and asserting the
/// report contains its relative path.
#[test]
fn verify_mode_detects_missing_tile() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");
    let plan = run_pyramid(
        &base,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );

    let top = plan.levels.last().unwrap();
    let rel = plan
        .tile_path(
            libviprs::TileCoord {
                level: top.level,
                col: 0,
                row: 0,
            },
            TileFormat::Png.extension(),
        )
        .unwrap();
    let victim = base.join(&rel);
    std::fs::remove_file(&victim).unwrap();

    let report = verify_output(&base).expect("verify_output should succeed at reading");
    assert_eq!(report.tiles_checked, plan.total_tile_count());
    assert!(
        report.tiles_missing.iter().any(|p| p.ends_with(&rel)),
        "expected deleted tile {} in tiles_missing: {:?}",
        rel,
        report.tiles_missing,
    );
    assert!(
        report.tiles_mismatched.is_empty(),
        "no tiles should be mismatched, got {:?}",
        report.tiles_mismatched
    );
}

// ---------------------------------------------------------------------------
// 8. verify_mode_passes_on_intact_output
// ---------------------------------------------------------------------------

/// Tests that `verify_output` reports zero mismatches and zero missing
/// tiles on a fresh, untouched output directory. Works by running
/// `EmitOnly` and calling `verify_output` immediately afterwards.
#[test]
fn verify_mode_passes_on_intact_output() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");
    let plan = run_pyramid(
        &base,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );

    let report = verify_output(&base).expect("verify_output should succeed on intact output");
    assert_eq!(report.tiles_checked, plan.total_tile_count());
    assert_eq!(report.tiles_ok, plan.total_tile_count());
    assert!(
        report.tiles_mismatched.is_empty(),
        "fresh output should have zero mismatches, got {:?}",
        report.tiles_mismatched
    );
    assert!(
        report.tiles_missing.is_empty(),
        "fresh output should have zero missing tiles, got {:?}",
        report.tiles_missing
    );
}

// ---------------------------------------------------------------------------
// 9. verify_happens_inline_when_ChecksumMode_Verify
// ---------------------------------------------------------------------------

/// An adversarial `TileSink` wrapper that corrupts the first emitted
/// tile's raw bytes *during* the write path. When used under
/// `ChecksumMode::Verify`, the engine must notice the mismatch between
/// the computed tile hash and the bytes that landed on disk, returning
/// `EngineError::ChecksumMismatch`.
struct CorruptingSink {
    inner: FsSink,
    // Toggle so we only corrupt the first tile written.
    corrupted: Arc<Mutex<bool>>,
}

impl CorruptingSink {
    fn new(inner: FsSink) -> Self {
        Self {
            inner,
            corrupted: Arc::new(Mutex::new(false)),
        }
    }
}

impl TileSink for CorruptingSink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        // First let the inner sink perform its normal write (including
        // checksum recording in the manifest).
        self.inner.write_tile(tile)?;

        // Then, on the very first call, rewrite the file on disk with
        // garbage so the checksum that was recorded no longer matches
        // the bytes the engine sees at verify time. We don't know the
        // exact output path here without re-implementing FsSink's
        // path logic, so we walk the base dir and clobber the most
        // recently modified regular file — which, since we just wrote
        // it synchronously on this call, is the tile we care about.
        let mut done = self.corrupted.lock().unwrap();
        if !*done {
            fn newest_file(dir: &Path) -> Option<PathBuf> {
                fn walk(dir: &Path, best: &mut Option<(std::time::SystemTime, PathBuf)>) {
                    if !dir.is_dir() {
                        return;
                    }
                    for entry in std::fs::read_dir(dir).unwrap().flatten() {
                        let p = entry.path();
                        if p.is_dir() {
                            walk(&p, best);
                        } else {
                            let md = entry.metadata().unwrap();
                            let m = md.modified().unwrap();
                            if best.as_ref().map(|(t, _)| m >= *t).unwrap_or(true) {
                                *best = Some((m, p));
                            }
                        }
                    }
                }
                let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
                walk(dir, &mut best);
                best.map(|(_, p)| p)
            }
            if let Some(victim) = newest_file(self.inner.base_dir()) {
                corrupt_file(&victim);
                *done = true;
            }
        }
        Ok(())
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
}

/// Tests that `ChecksumMode::Verify` causes the engine to surface an
/// `EngineError::ChecksumMismatch` when a tile's on-disk bytes do not
/// match the hash computed while writing. Works by wrapping a real
/// `FsSink` in `CorruptingSink` and asserting the returned error variant.
#[test]
fn verify_happens_inline_when_checksum_mode_verify() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");

    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let inner = FsSink::new(base.clone(), plan.clone())
        .with_format(TileFormat::Raw)
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::Verify, ChecksumAlgo::Blake3);
    let sink = CorruptingSink::new(inner);

    let err = generate_pyramid(
        &src,
        &plan,
        &sink,
        &EngineConfig::default().with_concurrency(1),
    )
    .expect_err("engine must fail when a tile fails its checksum round-trip");

    assert!(
        matches!(err, EngineError::ChecksumMismatch { .. }),
        "expected EngineError::ChecksumMismatch, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// 10. sha256_hex_length  /  blake3 hex length
// ---------------------------------------------------------------------------

/// Tests that hex digests recorded in the manifest have 64 hex chars for
/// both Blake3 and SHA-256 (each a 32-byte hash). Works by running
/// `EmitOnly` with each algo and asserting every digest in `per_tile`
/// has `len() == 64` and is valid hex.
#[test]
fn sha256_hex_length() {
    fn assert_len_64(base: &Path) {
        let value = read_manifest_json(base);
        let per_tile = value
            .pointer("/checksums/per_tile")
            .and_then(|v| v.as_object())
            .expect("per_tile missing");
        assert!(!per_tile.is_empty(), "per_tile should be non-empty");
        for (path, digest) in per_tile {
            let s = digest.as_str().expect("digest should be string");
            assert_eq!(
                s.len(),
                64,
                "digest for {path} must be 64 hex chars, got {} ({s})",
                s.len()
            );
            assert!(
                s.chars().all(|c| c.is_ascii_hexdigit()),
                "digest for {path} must be hex, got {s}"
            );
        }
    }

    let d_blake = tempfile::tempdir().unwrap();
    let b_blake = d_blake.path().join("pyramid");
    let _ = run_pyramid(
        &b_blake,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Blake3,
        EngineConfig::default(),
    );
    assert_len_64(&b_blake);

    let d_sha = tempfile::tempdir().unwrap();
    let b_sha = d_sha.path().join("pyramid");
    let _ = run_pyramid(
        &b_sha,
        TileFormat::Png,
        ChecksumMode::EmitOnly,
        ChecksumAlgo::Sha256,
        EngineConfig::default(),
    );
    assert_len_64(&b_sha);
}

// ---------------------------------------------------------------------------
// 11. checksums_work_with_jpeg_and_raw_formats
// ---------------------------------------------------------------------------

/// Tests that checksum emission and round-trip verification work for
/// every `TileFormat` variant. Works by parametrising across Raw, Png,
/// and Jpeg, and for each one asserting the manifest is populated and
/// `verify_output` passes on the intact output.
#[test]
fn checksums_work_with_jpeg_and_raw_formats() {
    let formats = [
        TileFormat::Raw,
        TileFormat::Png,
        TileFormat::Jpeg { quality: 80 },
    ];
    for fmt in formats {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("pyramid");
        let plan = run_pyramid(
            &base,
            fmt,
            ChecksumMode::EmitOnly,
            ChecksumAlgo::Blake3,
            EngineConfig::default(),
        );

        let value = read_manifest_json(&base);
        let per_tile = value
            .pointer("/checksums/per_tile")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("per_tile missing for {fmt:?}"));
        assert_eq!(
            per_tile.len() as u64,
            plan.total_tile_count(),
            "per_tile count mismatch for {fmt:?}"
        );

        let report = verify_output(&base)
            .unwrap_or_else(|e| panic!("verify_output failed for {fmt:?}: {e:?}"));
        assert_eq!(
            report.tiles_ok,
            plan.total_tile_count(),
            "verify failed on intact {fmt:?} output: {report:?}"
        );

        // Sanity: every tile file still exists on disk.
        let files = collect_tile_files(&base);
        assert!(!files.is_empty(), "no tile files written for {fmt:?}");
    }
}

// ---------------------------------------------------------------------------
// 12. checksum_of_blank_placeholder_is_stable
// ---------------------------------------------------------------------------

/// Tests that the per-tile hash recorded for a blank-placeholder tile
/// equals the well-known hash of the single `BLANK_TILE_MARKER` byte.
/// Works by forcing the engine to emit placeholders (a blank source),
/// then computing the expected digest via the `blake3` dev-dep and
/// comparing it against every placeholder entry in the manifest.
#[test]
fn checksum_of_blank_placeholder_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("pyramid");

    // Build a solid-white raster so that every tile is blank and the
    // Placeholder strategy writes 1-byte marker files.
    let bpp = libviprs::PixelFormat::Rgb8.bytes_per_pixel();
    let data = vec![255u8; (IMG_W as usize) * (IMG_H as usize) * bpp];
    let src = libviprs::Raster::new(IMG_W, IMG_H, libviprs::PixelFormat::Rgb8, data).unwrap();

    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let sink = FsSink::new(base.clone(), plan.clone())
        .with_manifest(ManifestBuilder::new())
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3);

    let config = EngineConfig::default().with_blank_tile_strategy(BlankTileStrategy::Placeholder);
    generate_pyramid(&src, &plan, &sink, &config).unwrap();

    // Expected hash = blake3 of a single BLANK_TILE_MARKER byte.
    let expected_hex = blake3::hash(&[BLANK_TILE_MARKER]).to_hex().to_string();

    let value = read_manifest_json(&base);
    let per_tile = value
        .pointer("/checksums/per_tile")
        .and_then(|v| v.as_object())
        .expect("per_tile missing");

    // Find at least one placeholder tile on disk (length == 1 and first
    // byte == BLANK_TILE_MARKER) and assert its recorded hash matches.
    let mut saw_placeholder = false;
    for (rel, digest) in per_tile {
        let p = base.join(rel);
        let bytes = match std::fs::read(&p) {
            Ok(b) => b,
            Err(_) => continue, // missing file — not this test's concern
        };
        if bytes.len() == 1 && bytes[0] == BLANK_TILE_MARKER {
            saw_placeholder = true;
            let got = digest.as_str().expect("digest must be string");
            assert_eq!(
                got, expected_hex,
                "placeholder hash for {rel} should be blake3(BLANK_TILE_MARKER)"
            );
        }
    }

    assert!(
        saw_placeholder,
        "expected at least one placeholder tile in fully-blank pyramid output"
    );
}
