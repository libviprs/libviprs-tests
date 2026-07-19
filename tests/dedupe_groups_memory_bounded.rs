//! Issue #452 — `dedupe_groups` memory/scalability follow-up to the #275
//! deterministic-dedupe fix.
//!
//! The #275 fix records, per shared key, enough bookkeeping to reassign the
//! full-payload holder to the coordinate-minimal occurrence at `finish()`. The
//! original implementation retained EVERY tile occurrence (`(coord, path)`) for
//! the whole run and, at `finish()`, cloned the whole table, re-cloned each
//! group's occurrences, and cloned the entire `manifest_refs` map once per
//! group (O(groups × refs)). On the headline sparse-blueprint workload —
//! millions of identical blank tiles collapsing onto a single `_shared/` blob —
//! that single group's occurrence list grows to N entries: a large retained
//! allocation plus several full copies, a potential OOM.
//!
//! The fix retains only what canonicalization consumes — the running
//! coordinate-minimal occurrence and the single `WriteNew` holder — so
//! retention is bounded per shared key, hoists the `manifest_refs` clone out of
//! the per-group loop, and consumes the group table by value. It is a
//! behaviour-PRESERVING refactor: the on-disk dedupe layout and the
//! `manifest.json::blank_references` map must stay byte-identical to the #275
//! behaviour.
//!
//! This file is the behaviour guard for that refactor. It drives the exact
//! sparse-blueprint scenario the fix targets — a raster whose pyramid is
//! overwhelmingly identical blank tiles, so one shared key absorbs a large
//! occurrence count — and asserts that replaying those tiles in canonical order
//! and in reverse order yields a byte-identical output tree and an identical
//! `blank_references` map (the #275 order-invariance the fix must not weaken).
//!
//! The occurrence-retention allocation bound itself is white-box (the retained
//! footprint is an internal detail invisible through the public sink API), so
//! it is asserted directly against the core bookkeeping in the core crate's
//! `dedupe_groups_retention_is_occurrence_independent` unit test, which is RED
//! against the pre-fix per-occurrence retention and GREEN after. A vips
//! differential golden is not applicable: the `_shared/` dedupe layout and the
//! `blank_references` manifest map are libviprs-specific artifacts the vips CLI
//! cannot express, so the golden here is a libviprs order-invariance golden.

use libviprs::sink::{DedupeStrategy, FsSink, Tile, TileSink};
use libviprs::{
    BlankTileStrategy, EngineBuilder, EngineConfig, EngineKind, Layout, PixelFormat, PyramidPlan,
    PyramidPlanner, Raster, TileCoord,
};
use std::collections::BTreeMap;
use std::path::Path;

// A large-ish image with small tiles so the pyramid has many tiles, the vast
// majority of them identical blank white — exactly the sparse-blueprint shape
// where one shared key absorbs a large occurrence count.
const IMG_SIZE: u32 = 1024;
const TILE_SIZE: u32 = 64;

fn make_plan(w: u32, h: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, TILE_SIZE, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// A raster that is almost entirely white with one tiny coloured feature, so
/// nearly every tile of the resulting pyramid is an identical blank white the
/// dedupe path collapses onto a single shared blob.
fn sparse_blueprint_raster() -> Raster {
    let w = IMG_SIZE;
    let h = IMG_SIZE;
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0xFFu8; w as usize * h as usize * bpp];
    // A single 12x12 speck near the centre — small enough that only a couple of
    // full-resolution tiles are non-blank, so the blank tiles dominate.
    let fx0 = w / 2 - 6;
    let fy0 = h / 2 - 6;
    for y in fy0..fy0 + 12 {
        for x in fx0..fx0 + 12 {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = 0x10;
            data[off + 1] = 0x20;
            data[off + 2] = 0xF0;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

fn coord_key(c: &TileCoord) -> (u32, u32, u32) {
    (c.level, c.row, c.col)
}

/// Collect the exact tiles a real pyramid run produces (coordinate + raster) so
/// they can be replayed into an `FsSink` in a chosen order — precisely what
/// different thread schedules do to the consumer under `tile_concurrency > 0`.
fn collect_tiles(src: &Raster, plan: &PyramidPlan) -> Vec<(TileCoord, Raster)> {
    let mem = libviprs::MemorySink::new();
    let config = EngineConfig::default().with_blank_tile_strategy(BlankTileStrategy::Emit);
    EngineBuilder::new(src, plan.clone(), &mem)
        .with_engine(EngineKind::Monolithic)
        .with_config(config)
        .run()
        .unwrap();
    mem.tiles()
        .into_iter()
        .map(|t| (t.coord, t.raster))
        .collect()
}

/// Replay `tiles` (in the given slice order) into a fresh dedupe-enabled
/// `FsSink` rooted at `base`, then finish.
fn run_dedupe(base: &Path, plan: &PyramidPlan, tiles: &[(TileCoord, Raster)]) {
    let sink = FsSink::new(base.to_path_buf(), plan.clone()).with_dedupe(DedupeStrategy::Blanks);
    for (coord, raster) in tiles {
        let tile = Tile {
            coord: *coord,
            raster: raster.clone(),
            blank: false,
        };
        sink.write_tile(&tile).unwrap();
    }
    sink.finish().unwrap();
}

/// Recursively collect `relative_path -> file_bytes` for every regular file
/// under `dir`, EXCLUDING `manifest.json` (its `created_at` timestamp is
/// legitimately non-deterministic — the dedupe-relevant `blank_references`
/// field is compared separately). Paths are normalised with `/` separators.
fn tree_snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
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
                if rel == "manifest.json" {
                    continue;
                }
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    walk(dir, dir, &mut out);
    out
}

/// Parse `<dir>/manifest.json` and return its `blank_references` map.
fn blank_references(dir: &Path) -> BTreeMap<String, String> {
    let raw = std::fs::read(dir.join("manifest.json")).expect("manifest.json exists");
    let v: serde_json::Value = serde_json::from_slice(&raw).expect("manifest.json parses");
    v.get("blank_references")
        .and_then(|o| o.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, val)| (k.clone(), val.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// The behaviour guard for issue #452: on the sparse-blueprint workload (one
/// shared key absorbing a large occurrence count) the memory-bounded
/// bookkeeping must produce exactly the #275 order-invariant layout. Replaying
/// the identical set of tiles into two dedupe sinks in canonical order and in
/// reverse order must yield byte-identical output trees and an identical
/// `blank_references` map. Any regression in which occurrence is promoted to
/// the full payload — the risk when trimming the retained occurrence list —
/// would surface here as a divergent tree or manifest map.
#[test]
fn sparse_blueprint_dedupe_layout_survives_bounded_retention() {
    let plan = make_plan(IMG_SIZE, IMG_SIZE);
    let src = sparse_blueprint_raster();

    let mut ordered = collect_tiles(&src, &plan);
    ordered.sort_by_key(|(c, _)| coord_key(c));
    let reversed: Vec<(TileCoord, Raster)> = ordered.iter().rev().cloned().collect();

    // The workload must actually be sparse: far more tiles than distinct shared
    // keys, so a single key absorbs a large occurrence count (the retained
    // allocation the fix bounds). Otherwise the guard would be vacuous.
    assert!(
        ordered.len() >= 128,
        "sparse workload must produce many tiles, got {}",
        ordered.len()
    );

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let base_a = dir_a.path().join("out");
    let base_b = dir_b.path().join("out");

    run_dedupe(&base_a, &plan, &ordered);
    run_dedupe(&base_b, &plan, &reversed);

    // Sanity: dedupe actually collapsed the blank tiles, otherwise the guard
    // would pass vacuously.
    assert!(
        base_a.join("_shared").is_dir(),
        "dedupe must have produced a _shared/ directory"
    );
    let blank_refs_a = blank_references(&base_a);
    assert!(
        blank_refs_a.len() >= 32,
        "the sparse workload must collapse many blank tiles onto shared blobs; \
         only {} blank_references recorded",
        blank_refs_a.len()
    );

    let snap_a = tree_snapshot(&base_a);
    let snap_b = tree_snapshot(&base_b);

    // Same set of output files.
    let keys_a: Vec<&String> = snap_a.keys().collect();
    let keys_b: Vec<&String> = snap_b.keys().collect();
    assert_eq!(
        keys_a, keys_b,
        "write order changed the SET of output files under bounded retention"
    );

    // Byte-identical content for every file — this is where a placeholder-vs-
    // full-payload divergence (a broken holder-reassignment) would surface.
    for (rel, bytes_a) in &snap_a {
        let bytes_b = &snap_b[rel];
        assert!(
            bytes_a == bytes_b,
            "file {rel} differs between arrival orders — bounded-retention \
             canonicalization is not order-independent (issue #452 regressed \
             #275)"
        );
    }

    // The manifest's dedupe map must not depend on arrival order.
    assert_eq!(
        blank_references(&base_a),
        blank_references(&base_b),
        "manifest.json::blank_references depends on tile arrival order under \
         bounded retention"
    );
}

/// A concrete coordinate-minimal-holder check on a single large group: many
/// identical blank tiles across one row, fed in reverse arrival order. Exactly
/// one tile path holds the full payload, it is the coordinate-minimal
/// occurrence (col 0), and every other occurrence is a 1-byte placeholder
/// resolvable via `blank_references` — the #275 contract the bounded-retention
/// bookkeeping (running-min + single holder) must reproduce for a key that
/// absorbs many duplicates.
#[test]
fn large_group_promotes_coordinate_minimal_holder() {
    // 512x8 @ tile 8 => full-resolution level is 64 tiles wide, 1 tall: one
    // shared key absorbs 64 identical blank occurrences.
    let planner = PyramidPlanner::new(512, 8, 8, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let top = plan.levels.last().unwrap();
    let cols = top.cols;
    assert!(cols >= 16, "test needs a wide level, got cols={cols}");
    let level = top.level;

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("out");
    let sink = FsSink::new(base.clone(), plan.clone()).with_dedupe(DedupeStrategy::Blanks);

    // Feed the identical (zeroed => uniform => blank) tiles in REVERSE column
    // order so the first arrival is the coordinate-maximal tile.
    for col in (0..cols).rev() {
        let rect = plan.tile_rect(TileCoord::new(level, col, 0)).unwrap();
        let tile = Tile {
            coord: TileCoord::new(level, col, 0),
            raster: Raster::zeroed(rect.width, rect.height, PixelFormat::Rgb8).unwrap(),
            blank: false,
        };
        sink.write_tile(&tile).unwrap();
    }
    sink.finish().unwrap();

    let tile_len = |col: u32| -> u64 {
        let rel = plan
            .tile_path(TileCoord::new(level, col, 0), "png")
            .unwrap();
        std::fs::metadata(base.join(rel)).unwrap().len()
    };
    let full_cols: Vec<u32> = (0..cols).filter(|&c| tile_len(c) > 1).collect();

    assert_eq!(
        full_cols,
        vec![0],
        "exactly the coordinate-minimal tile (col 0) must hold the full payload \
         regardless of arrival order; full-payload cols = {full_cols:?}"
    );

    // Exactly one shared blob backs all the occurrences.
    let shared: Vec<_> = std::fs::read_dir(base.join("_shared"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(shared.len(), 1, "expected exactly one shared blob");

    // Every non-canonical occurrence is a placeholder recorded in the manifest.
    let refs = blank_references(&base);
    let placeholder_cols: Vec<u32> = (0..cols)
        .filter(|&col| {
            plan.tile_path(TileCoord::new(level, col, 0), "png")
                .map(|rel| refs.contains_key(&rel))
                .unwrap_or(false)
        })
        .collect();
    let expected: Vec<u32> = (1..cols).collect();
    assert_eq!(
        placeholder_cols, expected,
        "every occurrence except the canonical col 0 must be a blank_references \
         placeholder"
    );
}
