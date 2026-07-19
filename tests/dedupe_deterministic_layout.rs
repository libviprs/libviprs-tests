//! Issue #275 — dedupe on-disk placement must be a pure function of tile
//! CONTENT + COORDINATE, independent of the order in which tiles arrive at the
//! sink.
//!
//! The [`TileSink`] contract (see the trait docs in core `sink.rs`) is
//! explicit:
//!
//! > Implementations must treat each `write_tile` as an independent,
//! > commutative placement keyed by `Tile::coord`; the produced pyramid must be
//! > identical regardless of the order in which the calls arrive. All in-tree
//! > sinks (`FsSink`, ...) satisfy this because they place each tile by its
//! > coordinate rather than by arrival position.
//!
//! Under `DedupeStrategy::Blanks` / `DedupeStrategy::All` the current `FsSink`
//! VIOLATES this contract. Its "promote on 2nd hit" scheme makes the tile that
//! receives the full payload (a hardlink to `_shared/blank_<hash>.<ext>`)
//! versus a 1-byte placeholder (recorded in `manifest.json::blank_references`)
//! a function of ARRIVAL ORDER: whichever occurrence of a given content hash
//! reaches `DedupeIndex::record` first keeps the full bytes, the rest become
//! placeholders. Under `tile_concurrency > 0` arrival order is producer
//! completion order over an MPSC channel, so two identical runs — or a
//! clean run vs a crash+resume run — can produce byte-divergent trees and
//! divergent `blank_references` maps.
//!
//! These tests exercise the defect through the public sink API by *replaying*
//! the exact same set of tiles into separate `FsSink`s in different orders
//! (which is precisely what different thread schedules do to the consumer).
//!
//! * [`dedupe_layout_is_independent_of_write_order`] — replay a realistic
//!   pyramid's tiles in canonical order and in reverse; the two output trees
//!   (every `_shared/` blob, every full tile, every placeholder) and the
//!   `blank_references` map must be byte-identical.
//! * [`dedupe_full_payload_tile_is_coordinate_minimal`] — a self-contained set
//!   of three identical tiles fed in reverse-coordinate order; the full-payload
//!   tile must be the coordinate-minimal occurrence, not the first to arrive.
//!
//! Both are RED against order-dependent (current) core and GREEN once
//! placement is content-addressed / coordinate-canonical (issue #275 fix).

use libviprs::sink::{DedupeStrategy, FsSink, Tile, TileSink};
use libviprs::{
    BlankTileStrategy, EngineBuilder, EngineConfig, EngineKind, Layout, PixelFormat, PyramidPlan,
    PyramidPlanner, Raster, TileCoord,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const IMG_SIZE: u32 = 512;
const TILE_SIZE: u32 = 128;

fn make_plan(w: u32, h: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, TILE_SIZE, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// A raster that is mostly white with a small coloured feature in the centre,
/// so most tiles of the resulting pyramid are identical blank whites that the
/// dedupe path collapses onto a shared blob.
fn whitespace_heavy_raster() -> Raster {
    let w = IMG_SIZE;
    let h = IMG_SIZE;
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0xFFu8; w as usize * h as usize * bpp];
    let fx0 = w / 2 - 8;
    let fy0 = h / 2 - 8;
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

/// Collect the exact tiles a real pyramid run produces (coordinate + raster)
/// so they can be replayed into an `FsSink` in a chosen order. Uses the
/// in-memory sink and the monolithic engine so the collected content matches
/// what a filesystem run would encode.
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

fn coord_key(c: &TileCoord) -> (u32, u32, u32) {
    (c.level, c.row, c.col)
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
/// under `dir`, EXCLUDING `manifest.json` (whose `created_at` timestamp is
/// legitimately non-deterministic — its dedupe-relevant `blank_references`
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

// ---------------------------------------------------------------------------
// 1. Whole-tree order independence
// ---------------------------------------------------------------------------

/// Replaying the identical set of tiles into two dedupe sinks in canonical
/// order and in reverse order must yield byte-identical output trees and an
/// identical `blank_references` map. RED on current (arrival-order-dependent)
/// core: the reverse run promotes a different occurrence to the full payload,
/// so the two trees diverge on which tile holds the full PNG vs the 1-byte
/// placeholder, and `blank_references` differs accordingly.
#[test]
fn dedupe_layout_is_independent_of_write_order() {
    let plan = make_plan(IMG_SIZE, IMG_SIZE);
    let src = whitespace_heavy_raster();

    let mut ordered = collect_tiles(&src, &plan);
    ordered.sort_by_key(|(c, _)| coord_key(c));
    let reversed: Vec<(TileCoord, Raster)> = ordered.iter().rev().cloned().collect();

    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let base_a = dir_a.path().join("out");
    let base_b = dir_b.path().join("out");

    run_dedupe(&base_a, &plan, &ordered);
    run_dedupe(&base_b, &plan, &reversed);

    // Sanity: dedupe actually kicked in (a shared blob exists), otherwise the
    // test would pass vacuously.
    assert!(
        base_a.join("_shared").is_dir(),
        "dedupe must have produced a _shared/ directory"
    );

    let snap_a = tree_snapshot(&base_a);
    let snap_b = tree_snapshot(&base_b);

    // Same set of files.
    let keys_a: Vec<&String> = snap_a.keys().collect();
    let keys_b: Vec<&String> = snap_b.keys().collect();
    assert_eq!(
        keys_a, keys_b,
        "write order changed the SET of output files (dedupe placement is \
         scheduling-dependent)"
    );

    // Same bytes for every file (this is where the placeholder-vs-full-payload
    // divergence surfaces).
    for (rel, bytes_a) in &snap_a {
        let bytes_b = &snap_b[rel];
        assert_eq!(
            bytes_a.len(),
            bytes_b.len(),
            "file {rel} differs in size between write orders \
             ({} vs {} bytes) — dedupe placement is order-dependent",
            bytes_a.len(),
            bytes_b.len()
        );
        assert!(
            bytes_a == bytes_b,
            "file {rel} differs in content between write orders — dedupe \
             placement is order-dependent"
        );
    }

    // The manifest's dedupe map must not depend on arrival order.
    assert_eq!(
        blank_references(&base_a),
        blank_references(&base_b),
        "manifest.json::blank_references depends on tile arrival order"
    );
}

// ---------------------------------------------------------------------------
// 2. Canonical full-payload holder is coordinate-minimal
// ---------------------------------------------------------------------------

/// Three byte-identical tiles at coords (col 0,1,2 / row 0) fed to the sink in
/// REVERSE coordinate order (col 2, then 1, then 0). Exactly one tile path
/// must hold the full payload and the other two must be 1-byte placeholders;
/// the full-payload holder must be the coordinate-minimal occurrence (col 0),
/// which is NOT the first to arrive. RED on current core (the first arrival —
/// col 2 — keeps the full bytes); GREEN once placement is coordinate-canonical.
#[test]
fn dedupe_full_payload_tile_is_coordinate_minimal() {
    // 24x8 @ tile 8 => full-resolution level is 3 tiles wide, 1 tall.
    let planner = PyramidPlanner::new(24, 8, 8, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let top = plan.levels.last().unwrap();
    assert!(
        top.cols >= 3,
        "test needs a level with >=3 tiles, got cols={}",
        top.cols
    );
    let level = top.level;

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("out");
    let sink = FsSink::new(base.clone(), plan.clone()).with_dedupe(DedupeStrategy::Blanks);

    // Feed the three identical (zeroed => uniform => blank) tiles in reverse
    // coordinate order so the FIRST arrival is the coordinate-MAXIMAL tile.
    for col in [2u32, 1, 0] {
        let rect = plan.tile_rect(TileCoord::new(level, col, 0)).unwrap();
        let tile = Tile {
            coord: TileCoord::new(level, col, 0),
            raster: Raster::zeroed(rect.width, rect.height, PixelFormat::Rgb8).unwrap(),
            blank: false,
        };
        sink.write_tile(&tile).unwrap();
    }
    sink.finish().unwrap();

    // Classify the three tile paths: full payload (len > 1) vs placeholder.
    let tile_len = |col: u32| -> u64 {
        let rel = plan
            .tile_path(TileCoord::new(level, col, 0), "png")
            .unwrap();
        std::fs::metadata(base.join(rel)).unwrap().len()
    };
    let full_cols: Vec<u32> = (0..3).filter(|&c| tile_len(c) > 1).collect();
    let placeholder_cols: Vec<u32> = (0..3).filter(|&c| tile_len(c) == 1).collect();

    assert_eq!(
        full_cols.len(),
        1,
        "exactly one tile must hold the full payload; full={full_cols:?}, \
         placeholders={placeholder_cols:?}"
    );
    assert_eq!(
        full_cols[0], 0,
        "the full-payload tile must be the coordinate-minimal occurrence \
         (col 0), independent of arrival order; got col {} (the first tile to \
         arrive under reverse feed)",
        full_cols[0]
    );

    // And the placeholders — everything except the canonical col 0 — must be
    // resolvable via blank_references.
    let refs = read_blank_reference_cols(&base, &plan, level);
    assert_eq!(
        refs,
        vec![1, 2],
        "blank_references must list exactly the non-canonical occurrences"
    );
}

/// Return the set of columns (row 0) at `level` that appear as keys in
/// `blank_references`, sorted ascending.
fn read_blank_reference_cols(base: &Path, plan: &PyramidPlan, level: u32) -> Vec<u32> {
    let refs = blank_references(base);
    let mut cols: Vec<u32> = (0..64)
        .filter(|&col| {
            plan.tile_path(TileCoord::new(level, col, 0), "png")
                .map(|rel| refs.contains_key(&rel))
                .unwrap_or(false)
        })
        .collect();
    cols.sort_unstable();
    cols
}

// Silence unused-import warnings if a helper is trimmed during review.
#[allow(dead_code)]
fn _unused(_: PathBuf) {}
