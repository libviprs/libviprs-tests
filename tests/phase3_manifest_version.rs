//! Integration tests for the **versioned manifest** feature (Phase 3).
//!
//! These tests are written TDD-style: the `libviprs::manifest` module and the
//! `FsSink::with_manifest` wiring do not yet exist, so this file is expected
//! to FAIL TO COMPILE until the API is implemented.
//!
//! The planned feature emits a `manifest.json` alongside the DZI XML that
//! records generation settings, source metadata, per-level counts, sparse
//! policy, optional per-tile checksums, and a `schema_version` tag enabling
//! forward-compatible evolution.

use libviprs::manifest::{
    ChecksumAlgo, Checksums, GenerationSettings, LevelMetadata, ManifestBuilder, ManifestV1,
    SourceMetadata, SparsePolicy,
};
use libviprs::source::generate_test_raster;
use libviprs::{
    BlankTileStrategy, EngineConfig, FsSink, Layout, MemorySink, PyramidPlanner, TileFormat,
    generate_pyramid,
};
use std::path::Path;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const IMG_W: u32 = 256;
const IMG_H: u32 = 192;
const TILE_SIZE: u32 = 64;
const OVERLAP: u32 = 0;

/// Read the `manifest.json` that the sink is expected to emit alongside the
/// DZI XML. Panics if the file is missing — that's the point of these tests.
fn read_manifest_json(output_base: &Path) -> serde_json::Value {
    // The manifest lives next to the DZI:  `<base>.dzi` and `<base>.manifest.json`
    // sit in the same parent directory. We check both a sibling file and a file
    // inside the tiles dir for flexibility; at least one must exist.
    let parent = output_base.parent().unwrap();
    let stem = output_base
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();

    // Preferred location: next to DZI as `<stem>.manifest.json`
    let sibling = parent.join(format!("{stem}.manifest.json"));
    if sibling.exists() {
        let bytes = std::fs::read(&sibling).unwrap();
        return serde_json::from_slice(&bytes).unwrap();
    }

    // Alternate location: inside the tiles dir
    let inside = output_base.join("manifest.json");
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

/// Locate the manifest.json path on disk (same rules as `read_manifest_json`).
fn manifest_path(output_base: &Path) -> std::path::PathBuf {
    let parent = output_base.parent().unwrap();
    let stem = output_base
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let sibling = parent.join(format!("{stem}.manifest.json"));
    if sibling.exists() {
        return sibling;
    }
    let inside = output_base.join("manifest.json");
    if inside.exists() {
        return inside;
    }
    panic!("manifest.json not found for base {}", output_base.display());
}

// ---------------------------------------------------------------------------
// 1. emits manifest.json
// ---------------------------------------------------------------------------

/// Tests that `FsSink` emits a `manifest.json` file after pyramid generation
/// completes. Works by generating a small pyramid into a tempdir with a
/// default `ManifestBuilder` and asserting the manifest file exists on disk.
#[test]
fn emits_manifest_json_next_to_dzi() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    // Just asserting it parses proves it both exists and is valid JSON.
    let _value = read_manifest_json(&base);
}

// ---------------------------------------------------------------------------
// 2. schema_version
// ---------------------------------------------------------------------------

/// Tests that the emitted manifest carries a `schema_version` tag of `"1"`,
/// which is the discriminator used by `#[serde(tag = "schema_version")]` to
/// support forward-compatible manifest evolution.
#[test]
fn manifest_has_schema_version_field() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let value = read_manifest_json(&base);
    assert_eq!(
        value.get("schema_version").and_then(|v| v.as_str()),
        Some("1"),
        "expected schema_version == \"1\", got {value:?}"
    );

    // Round-trip through the typed struct.
    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(parsed.schema_version, "1");
}

// ---------------------------------------------------------------------------
// 3. generation settings
// ---------------------------------------------------------------------------

/// Tests that the manifest records the generation settings supplied to the
/// engine (tile_size, overlap, layout, format, concurrency, background_rgb,
/// blank_strategy). Works by configuring a distinctive set of settings and
/// asserting they round-trip through serde into `GenerationSettings`.
#[test]
fn manifest_records_generation_settings() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Jpeg { quality: 80 })
        .with_manifest(ManifestBuilder::new());

    let config = EngineConfig::default().with_concurrency(3);
    generate_pyramid(&src, &plan, &sink, &config).unwrap();

    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();

    let g: &GenerationSettings = &parsed.generation;
    assert_eq!(g.tile_size, TILE_SIZE);
    assert_eq!(g.overlap, OVERLAP);
    // Layout and format should round-trip as their enum variants; we only
    // check equality against the expected inputs.
    assert_eq!(g.layout, Layout::DeepZoom);
    assert_eq!(g.format, TileFormat::Jpeg { quality: 80 });
    assert_eq!(g.concurrency, 3);
    // Default background is white.
    assert_eq!(g.background_rgb, [255, 255, 255]);
    // Default strategy is Emit.
    assert_eq!(g.blank_strategy, BlankTileStrategy::Emit);
}

// ---------------------------------------------------------------------------
// 4. source metadata
// ---------------------------------------------------------------------------

/// Tests that the manifest records source metadata matching the input raster:
/// width, height, and pixel format. Works by generating a test raster of
/// known dimensions and asserting the SourceMetadata mirrors those fields.
#[test]
fn manifest_records_source_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();

    let s: &SourceMetadata = &parsed.source;
    assert_eq!(s.width, IMG_W);
    assert_eq!(s.height, IMG_H);
    assert_eq!(s.pixel_format, src.format());
    // Default builder does NOT include a source hash.
    assert!(
        s.bytes_hash.is_none(),
        "expected bytes_hash=None when include_source_hash wasn't set, got {:?}",
        s.bytes_hash
    );
}

// ---------------------------------------------------------------------------
// 5. per-level metadata
// ---------------------------------------------------------------------------

/// Tests that the manifest records one `LevelMetadata` entry per planned
/// pyramid level and that counts (tiles_produced + tiles_skipped) sum to the
/// expected number of tiles per level. Works by comparing `parsed.levels`
/// against `plan.levels` element by element.
#[test]
fn manifest_records_per_level_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(
        parsed.levels.len(),
        plan.levels.len(),
        "manifest level count must match plan level count"
    );

    let mut total_produced: u64 = 0;
    let mut total_skipped: u64 = 0;
    for (lm, lp) in parsed.levels.iter().zip(plan.levels.iter()) {
        let lm: &LevelMetadata = lm;
        assert_eq!(lm.level_index, lp.level);
        assert_eq!(lm.width, lp.width);
        assert_eq!(lm.height, lp.height);
        let level_total = (lp.cols as u64) * (lp.rows as u64);
        assert_eq!(
            lm.tiles_produced + lm.tiles_skipped,
            level_total,
            "level {} counts don't sum to cols*rows",
            lp.level
        );
        total_produced += lm.tiles_produced;
        total_skipped += lm.tiles_skipped;
    }

    // With default Emit strategy, nothing is skipped.
    assert_eq!(total_skipped, 0);
    assert_eq!(total_produced, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// 6. sparse policy
// ---------------------------------------------------------------------------

/// Tests that the manifest's `sparse_policy` reflects the active blank-tile
/// strategy. Works by configuring `BlankTileStrategy::Placeholder` and
/// asserting the serialized manifest's `sparse_policy` records non-default
/// settings (e.g. dedupe enabled or tolerance set) as expected for that mode.
#[test]
fn manifest_records_sparse_policy() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    let config = EngineConfig::default().with_blank_tile_strategy(BlankTileStrategy::Placeholder);
    generate_pyramid(&src, &plan, &sink, &config).unwrap();

    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();

    let sp: &SparsePolicy = &parsed.sparse_policy;
    // Placeholder strategy means dedupe is active.
    assert!(
        sp.dedupe,
        "expected dedupe=true under Placeholder strategy, got {sp:?}"
    );
    // Tolerance defaults to 0 when exact-match blank detection is used.
    assert_eq!(sp.tolerance, 0);
}

// ---------------------------------------------------------------------------
// 7. checksums (enabled)
// ---------------------------------------------------------------------------

/// Tests that requesting `ChecksumAlgo::Blake3` via the builder causes the
/// manifest to include a populated `checksums.per_tile` map, with one entry
/// per emitted tile. Works by generating a small pyramid and asserting the
/// map has the expected length and non-empty hex digests.
#[test]
fn manifest_with_checksums_has_per_tile_hash() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3));

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();

    let cs: &Checksums = parsed
        .checksums
        .as_ref()
        .expect("expected checksums to be populated when with_checksums(Blake3) was requested");
    assert_eq!(cs.algo, ChecksumAlgo::Blake3);
    assert_eq!(
        cs.per_tile.len() as u64,
        plan.total_tile_count(),
        "expected one checksum per emitted tile"
    );
    for (path, digest) in &cs.per_tile {
        assert!(!digest.is_empty(), "empty digest for {path}");
        // Blake3 hex digest is 64 chars.
        assert_eq!(
            digest.len(),
            64,
            "blake3 hex digest for {path} should be 64 chars, got {}",
            digest.len()
        );
    }
}

// ---------------------------------------------------------------------------
// 8. checksums (disabled by default)
// ---------------------------------------------------------------------------

/// Tests that when the `ManifestBuilder` is used without `.with_checksums()`,
/// the emitted manifest's `checksums` field is `None`. Works by generating a
/// pyramid with a default builder and asserting the Option is None.
#[test]
fn manifest_without_checksums_is_none() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let parsed: ManifestV1 = serde_json::from_slice(&bytes).unwrap();

    assert!(
        parsed.checksums.is_none(),
        "default builder should leave checksums=None, got {:?}",
        parsed.checksums
    );
}

// ---------------------------------------------------------------------------
// 9. determinism
// ---------------------------------------------------------------------------

/// Tests that two independent runs with identical inputs and settings produce
/// byte-identical `manifest.json` files, with the sole exception of the
/// `created_at` timestamp. Works by generating the same pyramid twice into
/// separate tempdirs, parsing both manifests, zeroing `created_at`, and
/// re-serializing before comparing.
#[test]
fn manifest_deterministic_across_runs() {
    fn run(out: &Path) -> ManifestV1 {
        let src = generate_test_raster(IMG_W, IMG_H).unwrap();
        let planner =
            PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
        let plan = planner.plan();
        let sink = FsSink::new(out.to_path_buf(), plan.clone(), TileFormat::Png)
            .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3));
        generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();
        let bytes = std::fs::read(manifest_path(out)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();

    let mut m1 = run(&d1.path().join("pyramid"));
    let mut m2 = run(&d2.path().join("pyramid"));

    // Zero the timestamps — determinism applies to everything else.
    m1.created_at = String::new();
    m2.created_at = String::new();

    let s1 = serde_json::to_vec(&m1).unwrap();
    let s2 = serde_json::to_vec(&m2).unwrap();
    assert_eq!(
        s1, s2,
        "manifest.json must be byte-identical across runs (ignoring created_at)"
    );
}

// ---------------------------------------------------------------------------
// 10. forward compatibility with unknown fields
// ---------------------------------------------------------------------------

/// Tests that `serde_json` can deserialize a `ManifestV1` payload that
/// contains an unexpected future field, proving the struct uses permissive
/// deserialization (no `#[serde(deny_unknown_fields)]`). Works by emitting a
/// real manifest, injecting an extra `"future_field": ...` key at the top
/// level, and re-parsing through `ManifestV1`.
#[test]
fn manifest_parses_with_unknown_future_fields() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let bytes = std::fs::read(manifest_path(&base)).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // Inject a future-only field that ManifestV1 doesn't know about.
    value
        .as_object_mut()
        .unwrap()
        .insert("future_field".to_string(), serde_json::json!("hello"));
    value
        .as_object_mut()
        .unwrap()
        .insert("another_future_field".to_string(), serde_json::json!(42));
    let bumped = serde_json::to_vec(&value).unwrap();

    // Must still deserialize without error.
    let parsed: ManifestV1 = serde_json::from_slice(&bumped)
        .expect("ManifestV1 should ignore unknown fields for forward-compat");
    assert_eq!(parsed.schema_version, "1");
}

// ---------------------------------------------------------------------------
// 11. DZI XML still emitted
// ---------------------------------------------------------------------------

/// Regression test ensuring the existing `<base>.dzi` XML file is still
/// written alongside the new manifest.json. Works by generating a pyramid
/// with the manifest feature enabled and asserting the DZI file exists and
/// still contains the expected Width/Height attributes.
#[test]
fn dzi_xml_still_emitted() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("pyramid");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png)
        .with_manifest(ManifestBuilder::new());

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let dzi = dir.path().join("pyramid.dzi");
    assert!(
        dzi.exists(),
        "DZI manifest should still be emitted alongside manifest.json"
    );
    let xml = std::fs::read_to_string(&dzi).unwrap();
    assert!(xml.contains(&format!("Width=\"{IMG_W}\"")));
    assert!(xml.contains(&format!("Height=\"{IMG_H}\"")));
}

// ---------------------------------------------------------------------------
// 12. MemorySink does not emit a manifest
// ---------------------------------------------------------------------------

/// Tests that only file-system sinks emit a `manifest.json` — a `MemorySink`
/// never writes to disk. Works by generating a pyramid into a `MemorySink`
/// from within a tempdir and asserting that no `manifest.json` appears in
/// the directory after generation.
#[test]
fn memory_sink_does_not_emit_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let src = generate_test_raster(IMG_W, IMG_H).unwrap();
    let planner = PyramidPlanner::new(IMG_W, IMG_H, TILE_SIZE, OVERLAP, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    // MemorySink has no `with_manifest` API; verify that by construction it
    // can be used with generate_pyramid without emitting any files.
    let sink = MemorySink::new();
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    // The tempdir should have no files in it at all.
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert!(
        entries.is_empty(),
        "MemorySink must not write to disk; found {} entries in {}",
        entries.len(),
        dir.path().display()
    );
}
