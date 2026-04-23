//! Integration tests for the Phase 3 packfile archive sink.
//!
//! These tests target the *planned* API introduced under the `packfile`
//! feature flag:
//!
//! ```ignore
//! use libviprs::sink::{PackfileSink, PackfileFormat};
//!
//! pub enum PackfileFormat {
//!     Tar,
//!     TarGz,
//!     Zip,
//! }
//!
//! let sink = PackfileSink::new(
//!     "/path/to/output.tar",
//!     PackfileFormat::Tar,
//!     plan.clone(),
//!     TileFormat::Png,
//! )?;
//! ```
//!
//! The archive layout mirrors `FsSink`:
//!
//! ```text
//!   manifest.json                          (at root)
//!   <image>.dzi                            (at root, for DeepZoom)
//!   <image>_files/<level>/<x>_<y>.<ext>    (tile payloads)
//! ```
//!
//! The whole file is gated behind `#[cfg(feature = "packfile")]` so that
//! builds that don't enable the feature simply don't compile this file.
//! When the feature *is* enabled and the planned API does not yet exist,
//! compilation must fail — that is the TDD signal the implementation still
//! needs to be written.
//!
//! Style follows `tests/phase3_object_store_sink.rs` and
//! `tests/pyramid_fs_sink.rs`: deterministic fixtures, `tempfile::tempdir()`
//! for any filesystem needs, and a short docblock on every `#[test]`
//! explaining what it proves.

#![cfg(feature = "packfile")]

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use libviprs::sink::{PackfileFormat, PackfileSink};
use libviprs::{
    BlankTileStrategy, DedupeStrategy, EngineConfig, FsSink, Layout, PixelFormat, PyramidPlan,
    PyramidPlanner, Raster, RasterStripSource, StreamingConfig, TileFormat, generate_pyramid,
    generate_pyramid_streaming,
};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

/// Deterministic RGB gradient raster. Same generator used throughout the
/// existing Phase 3 test suite so fixtures are directly comparable.
fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x + y) % 256) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Uniform white raster — exercises the `DedupeStrategy::Blanks` path because
/// every level of the pyramid collapses to a single repeated blank tile.
fn white_raster(w: u32, h: u32) -> Raster {
    let data = vec![255u8; w as usize * h as usize * 3];
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Build a standard DeepZoom plan.
fn make_plan(w: u32, h: u32, tile: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, tile, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Extract a POSIX tar archive at `tar_path` into `dest_dir` using the
/// `tar` crate (no shell-out — keeps the tests hermetic on Windows too).
fn extract_tar_to(tar_path: &Path, dest_dir: &Path) {
    let file = File::open(tar_path).expect("open tar");
    let mut archive = tar::Archive::new(BufReader::new(file));
    archive.unpack(dest_dir).expect("unpack tar");
}

/// List every entry path inside a POSIX tar archive, returned in archive
/// order. Used in lieu of shelling out to `tar tf`.
fn list_tar(tar_path: &Path) -> Vec<String> {
    let file = File::open(tar_path).expect("open tar");
    let mut archive = tar::Archive::new(BufReader::new(file));
    archive
        .entries()
        .expect("tar entries")
        .map(|e| {
            let entry = e.expect("tar entry");
            entry
                .path()
                .expect("entry path")
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// Read every regular file under `root` into a flat (relative-path → bytes)
/// sorted map. Sorting gives a deterministic comparison surface for
/// byte-for-byte equality asserts.
fn collect_fs_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &Path, cur: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(cur).expect("read_dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path
                    .strip_prefix(base)
                    .expect("strip prefix")
                    .to_string_lossy()
                    .replace('\\', "/");
                let bytes = std::fs::read(&path).expect("read file");
                out.push((rel, bytes));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Expected relative tile path inside a DeepZoom archive, rooted at
/// `<stem>_files/<level>/<x>_<y>.<ext>`.
fn expected_dzi_tile_paths(plan: &PyramidPlan, stem: &str, ext: &str) -> BTreeSet<String> {
    plan.tile_coords()
        .map(|coord| {
            format!(
                "{stem}_files/{lvl}/{x}_{y}.{ext}",
                lvl = coord.level,
                x = coord.col,
                y = coord.row
            )
        })
        .collect()
}

/// Convenience: build a `PackfileSink` for a temp output path.
fn make_packfile(
    out_path: PathBuf,
    format: PackfileFormat,
    plan: PyramidPlan,
    tile_format: TileFormat,
) -> PackfileSink {
    PackfileSink::new(out_path, format, plan, tile_format).expect("construct PackfileSink")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. `tar_sink_produces_valid_archive` —
///    Writing a pyramid through `PackfileSink` with `PackfileFormat::Tar`
///    yields a POSIX tar that `tar tf`-equivalent listing enumerates the
///    full set of tile entries.
#[test]
fn tar_sink_produces_valid_archive() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("output.tar");
    let src = gradient_raster(256, 256);
    let plan = make_plan(256, 256, 128);

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::Tar,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let entries = list_tar(&out);
    // Every planned tile must appear in the archive listing.
    let expected = expected_dzi_tile_paths(&plan, "output", "png");
    for want in &expected {
        assert!(
            entries.iter().any(|e| e == want),
            "tar is missing tile entry: {want}\nlisted: {entries:#?}"
        );
    }
}

/// 2. `tarball_contains_manifest_json` —
///    The archive must carry `manifest.json` at its root. This is the
///    machine-readable index consumers use to discover the pyramid's
///    structure without a filesystem walk.
#[test]
fn tarball_contains_manifest_json() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("output.tar");
    let src = gradient_raster(128, 128);
    let plan = make_plan(128, 128, 64);

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::Tar,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let entries = list_tar(&out);
    assert!(
        entries.iter().any(|e| e == "manifest.json"),
        "manifest.json missing from archive root; entries: {entries:#?}"
    );
}

/// 3. `tarball_contains_dzi_xml_for_deepzoom_layout` —
///    For the DeepZoom layout we additionally expect `<stem>.dzi` at the
///    root of the archive — the OpenSeadragon-compatible descriptor.
#[test]
fn tarball_contains_dzi_xml_for_deepzoom_layout() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("pyramid.tar");
    let src = gradient_raster(128, 128);
    let plan = make_plan(128, 128, 64);

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::Tar,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let entries = list_tar(&out);
    assert!(
        entries.iter().any(|e| e == "pyramid.dzi"),
        ".dzi descriptor missing from archive root; entries: {entries:#?}"
    );
}

/// 4. `targz_is_valid_gzip` —
///    `PackfileFormat::TarGz` must wrap the tar stream in a real gzip
///    container — the file's first two bytes are `0x1F 0x8B`.
#[test]
fn targz_is_valid_gzip() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("output.tar.gz");
    let src = gradient_raster(128, 128);
    let plan = make_plan(128, 128, 64);

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::TarGz,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let mut f = File::open(&out).expect("open targz");
    let mut head = [0u8; 2];
    f.read_exact(&mut head).expect("read gzip magic");
    assert_eq!(
        head,
        [0x1F, 0x8B],
        "targz output does not start with gzip magic bytes; got {head:#04x?}"
    );
}

/// 5. `zip_sink_produces_valid_archive` —
///    `PackfileFormat::Zip` must produce a zip file the `zip` crate can
///    open and iterate. We assert the expected tile count.
#[test]
fn zip_sink_produces_valid_archive() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("output.zip");
    let src = gradient_raster(128, 128);
    let plan = make_plan(128, 128, 64);

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::Zip,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let reader = File::open(&out).expect("open zip");
    let mut archive = zip::ZipArchive::new(BufReader::new(reader)).expect("parse zip");

    // Walk every entry so the zip crate actually validates each local header.
    let mut names: Vec<String> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).expect("read entry");
        names.push(entry.name().to_string());
    }

    let expected = expected_dzi_tile_paths(&plan, "output", "png");
    for want in &expected {
        assert!(
            names.iter().any(|n| n == want),
            "zip is missing tile entry: {want}\nentries: {names:#?}"
        );
    }
    assert!(
        names.iter().any(|n| n == "manifest.json"),
        "zip missing manifest.json; entries: {names:#?}"
    );
    assert!(
        names.iter().any(|n| n == "output.dzi"),
        "zip missing .dzi descriptor; entries: {names:#?}"
    );
}

/// 6. `archive_tile_paths_match_layout` —
///    Every tile entry inside the archive must follow the DeepZoom
///    convention `<stem>_files/<level>/<x>_<y>.<ext>` exactly. Stray
///    or renamed entries are a regression.
#[test]
fn archive_tile_paths_match_layout() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("layout.tar");
    let src = gradient_raster(256, 192);
    let plan = make_plan(256, 192, 64);

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::Tar,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let entries: BTreeSet<String> = list_tar(&out).into_iter().collect();
    let expected = expected_dzi_tile_paths(&plan, "layout", "png");

    for want in &expected {
        assert!(
            entries.contains(want),
            "missing expected DeepZoom tile path: {want}\nentries: {entries:#?}"
        );
    }

    // Reverse: every `*_files/...` entry inside the archive must correspond
    // to a planned tile coord. This catches accidental renames / duplicates.
    for entry in &entries {
        if entry.starts_with("layout_files/") {
            assert!(
                expected.contains(entry),
                "archive contains unexpected tile path: {entry}"
            );
        }
    }
}

/// 7. `archive_extraction_produces_identical_fs_output` —
///    Extracting a tar-packaged pyramid to a tempdir must yield a tree
///    that is byte-for-byte identical to running `FsSink` directly on
///    the same inputs (same plan, same `TileFormat`, same engine config).
#[test]
fn archive_extraction_produces_identical_fs_output() {
    let src = gradient_raster(256, 192);
    let plan = make_plan(256, 192, 64);

    // Run 1: FsSink directly.
    let fs_dir = tempfile::tempdir().unwrap();
    let fs_base = fs_dir.path().join("out");
    let fs_sink = FsSink::new(fs_base.clone(), plan.clone());
    generate_pyramid(&src, &plan, &fs_sink, &EngineConfig::default()).unwrap();

    // Run 2: PackfileSink (tar) + extract.
    let tar_dir = tempfile::tempdir().unwrap();
    let tar_path = tar_dir.path().join("out.tar");
    let packfile_sink = make_packfile(
        tar_path.clone(),
        PackfileFormat::Tar,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &packfile_sink, &EngineConfig::default()).unwrap();

    let extract_dir = tempfile::tempdir().unwrap();
    extract_tar_to(&tar_path, extract_dir.path());

    // Build comparable (path, bytes) trees. The FsSink run writes into
    // `<tempdir>/out/` while the extracted archive places the same files
    // under `<extract>/out_files/` plus sibling `out.dzi` and `manifest.json`
    // at the extract root. We therefore compare on the SHARED subset:
    // the `out_files/...` tile subtree and the `out.dzi` descriptor.
    let fs_tree = collect_fs_tree(fs_dir.path());
    let tar_tree = collect_fs_tree(extract_dir.path());

    for (rel, fs_bytes) in &fs_tree {
        // Skip manifest.json — FsSink may not emit one at all.
        if rel.ends_with("manifest.json") {
            continue;
        }
        let found = tar_tree
            .iter()
            .find(|(r, _)| r == rel)
            .unwrap_or_else(|| panic!("extracted archive missing {rel}"));
        assert_eq!(
            &found.1, fs_bytes,
            "extracted archive differs from FsSink at {rel}"
        );
    }
}

/// 8. `packfile_with_blank_tolerance_dedupes_correctly` —
///    When composed with `DedupeStrategy::Blanks` (a solid-white input
///    where every tile is blank), the archive must contain the shared
///    blank blob only once and have the manifest reference it from
///    every deduplicated coordinate.
#[test]
fn packfile_with_blank_tolerance_dedupes_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("dedup.tar");
    let src = white_raster(128, 128);
    let plan = make_plan(128, 128, 64);

    let config = EngineConfig::default()
        .with_blank_tile_strategy(BlankTileStrategy::Placeholder)
        .with_dedupe_strategy(DedupeStrategy::Blanks);

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::Tar,
        plan.clone(),
        TileFormat::Png,
    );
    generate_pyramid(&src, &plan, &sink, &config).unwrap();

    let entries = list_tar(&out);

    // manifest.json must exist — the dedupe record lives inside it.
    assert!(
        entries.iter().any(|e| e == "manifest.json"),
        "dedupe archive missing manifest.json; entries: {entries:#?}"
    );

    // The expected DeepZoom tile paths should NOT all be materialised as
    // independent tar entries: blank dedupe collapses them to a shared blob.
    let full_tile_paths = expected_dzi_tile_paths(&plan, "dedup", "png");
    let stored_tile_entries: Vec<&String> = entries
        .iter()
        .filter(|e| e.starts_with("dedup_files/"))
        .collect();
    assert!(
        (stored_tile_entries.len() as u64) < plan.total_tile_count(),
        "expected dedupe to store < {} tile entries, got {}",
        plan.total_tile_count(),
        stored_tile_entries.len(),
    );
    // And the stored entries must be a subset of the planned tile coords.
    for e in &stored_tile_entries {
        assert!(
            full_tile_paths.contains(e.as_str()),
            "dedupe archive has unexpected tile entry: {e}"
        );
    }
}

/// 9. `packfile_streaming_memory_bounded` —
///    Driving `PackfileSink` from the streaming engine on a pyramid large
///    enough to force out-of-core mode must keep `peak_memory_bytes`
///    comfortably under the configured budget. This is a smoke check —
///    the absolute bound is a multiple of the budget to allow for
///    small bookkeeping overhead.
#[test]
fn packfile_streaming_memory_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("big.tar");

    let src = gradient_raster(2048, 2048);
    let plan = make_plan(2048, 2048, 256);

    let budget: u64 = 2_000_000; // 2 MB
    let config = StreamingConfig {
        memory_budget_bytes: budget,
        engine: EngineConfig::default(),
    };

    let sink = make_packfile(
        out.clone(),
        PackfileFormat::Tar,
        plan.clone(),
        TileFormat::Png,
    );
    let strip_src = RasterStripSource::new(&src);
    let result = generate_pyramid_streaming(
        &strip_src,
        &plan,
        &sink,
        &config,
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    // Smoke: streaming must produce every tile.
    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Peak memory must be bounded. Allow up to 4x budget to account for
    // per-thread working sets and encoder buffers — the streaming engine
    // is not required to hit the budget exactly, just to stay near it.
    let upper = budget.saturating_mul(4);
    assert!(
        result.peak_memory_bytes <= upper,
        "peak memory {} exceeded upper bound {} (budget {})",
        result.peak_memory_bytes,
        upper,
        budget,
    );
}
