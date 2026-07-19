//! Issue #296 — the dedupe promote critical section is sharded by content
//! digest so tiles of DISTINCT content no longer serialise on one
//! process-wide lock, while every correctness invariant the coarse lock
//! guaranteed is preserved.
//!
//! This finding is a concurrency/perf improvement: its *observable* output
//! (the on-disk tree, the `manifest.json::blank_references` map, the
//! at-least-one-hardlink guarantee) is identical before and after the fix —
//! the coarse single-mutex path was already correct (issue #111), just
//! needlessly serial. A `vips`-differential golden therefore does not apply
//! (there is no pixel/op to reproduce with the CLI); the strongest black-box
//! guard is a determinism-preservation test.
//!
//! [`concurrent_distinct_content_matches_serial_replay`] drives one
//! dedupe-enabled `FsSink` from MANY host threads, each writing tiles of
//! DISTINCT content (so distinct-content promotes happen concurrently — the
//! exact path the sharded lock unblocks), then asserts the resulting tree,
//! the `blank_references` map, and the shared-blob inode invariant are
//! byte-for-byte identical to a serial, single-threaded replay of the same
//! tiles. The internal RED→GREEN proof that distinct content no longer
//! serialises lives in the core unit tests
//! (`src/sink.rs::dedupe_distinct_content_promotes_without_serialising` /
//! `dedupe_same_content_serialises_on_promote_shard`), which can reach the
//! private shard lock; this integration test guards that the fix keeps every
//! externally-visible invariant intact under real concurrency.

use libviprs::checksum::ChecksumAlgo;
use libviprs::sink::{DedupeStrategy, FsSink, Tile, TileSink};
use libviprs::{Layout, PixelFormat, PyramidPlan, PyramidPlanner, Raster, TileCoord};

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

const TILE_SIZE: u32 = 8;

/// A plan whose full-resolution level is wide enough to host every tile we
/// synthesise. `w = TILE_SIZE * cols` gives exactly `cols` tiles across at the
/// deepest level.
fn wide_plan(cols: u32) -> PyramidPlan {
    PyramidPlanner::new(TILE_SIZE * cols, TILE_SIZE, TILE_SIZE, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// A uniform-colour tile filled with `val` on every channel. Uniform so the
/// sink routes it through the dedupe path; the value makes its content — and
/// therefore its shard and shared key — distinct per `val`.
fn solid_tile(coord: TileCoord, rect_w: u32, rect_h: u32, val: u8) -> Tile {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let data = vec![val; rect_w as usize * rect_h as usize * bpp];
    Tile {
        coord,
        raster: Raster::new(rect_w, rect_h, PixelFormat::Rgb8, data).unwrap(),
        blank: false,
    }
}

/// Build the tile set: `distinct` different solid colours, each appearing
/// EXACTLY TWICE (adjacent columns), so every colour is promoted to a shared
/// blob with one hardlink holder + one placeholder. Returns
/// `(coord, val)` pairs in canonical column order.
fn build_tiles(distinct: u32) -> Vec<(TileCoord, u8)> {
    let level = wide_plan(distinct * 2).levels.last().unwrap().level;
    let mut tiles = Vec::new();
    for i in 0..distinct {
        // Colours 1..=distinct (avoid 0 so the raster is never mistaken for a
        // trivial all-zero blank; every colour is still distinct).
        let val = (1 + i) as u8;
        tiles.push((TileCoord::new(level, i * 2, 0), val));
        tiles.push((TileCoord::new(level, i * 2 + 1, 0), val));
    }
    tiles
}

/// Replay `order` (a permutation of tile indices) into a fresh dedupe `FsSink`
/// rooted at `base`. When `threads > 1`, the tiles are partitioned across host
/// threads and written concurrently through the single shared sink; when
/// `threads == 1` they are written serially on the calling thread.
fn run(
    base: &Path,
    plan: &PyramidPlan,
    tiles: &[(TileCoord, u8)],
    order: &[usize],
    threads: usize,
) {
    let sink = Arc::new(FsSink::new(base.to_path_buf(), plan.clone()).with_dedupe(
        DedupeStrategy::All {
            algo: ChecksumAlgo::Blake3,
        },
    ));

    let write_one = |sink: &FsSink, idx: usize| {
        let (coord, val) = tiles[idx];
        let rect = plan.tile_rect(coord).unwrap();
        sink.write_tile(&solid_tile(coord, rect.width, rect.height, val))
            .unwrap();
    };

    if threads <= 1 {
        for &idx in order {
            write_one(&sink, idx);
        }
    } else {
        let order: Vec<usize> = order.to_vec();
        let chunk = order.len().div_ceil(threads);
        std::thread::scope(|s| {
            for part in order.chunks(chunk) {
                let sink = Arc::clone(&sink);
                let part = part.to_vec();
                s.spawn(move || {
                    for idx in part {
                        write_one(&sink, idx);
                    }
                });
            }
        });
    }

    sink.finish().unwrap();
}

/// `relative_path -> bytes` for every regular file under `dir`, EXCLUDING the
/// timestamped `manifest.json` (its `created_at` is legitimately
/// non-deterministic; the dedupe-relevant `blank_references` field is compared
/// separately). Paths are normalised with `/`.
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

/// Every shared blob under `_shared/` must be referenced by at least one
/// hardlinked tile path (nlink >= 2): the at-least-one-hardlink invariant that
/// the promote critical section guarantees, which sharding must not weaken.
#[cfg(unix)]
fn assert_every_shared_blob_is_hardlinked(base: &Path) {
    use std::os::unix::fs::MetadataExt;
    let shared_dir = base.join("_shared");
    let entries: Vec<_> = std::fs::read_dir(&shared_dir)
        .expect("_shared/ must exist")
        .map(|e| e.unwrap().path())
        .collect();
    assert!(!entries.is_empty(), "expected at least one shared blob");
    for blob in entries {
        let md = std::fs::metadata(&blob).unwrap();
        assert!(
            md.nlink() >= 2,
            "shared blob {blob:?} has nlink={}, so no tile is hardlinked to it \
             (the at-least-one-hardlink invariant was broken under concurrency)",
            md.nlink()
        );
    }
}

/// Concurrent, distinct-content dedupe writes must produce a tree and a
/// `blank_references` map byte-identical to a serial replay of the same tiles,
/// and must uphold the at-least-one-hardlink invariant. Repeated so a scheduling
/// window that only occasionally interleaves the promotes is still caught.
#[test]
fn concurrent_distinct_content_matches_serial_replay() {
    const DISTINCT: u32 = 24;
    let plan = wide_plan(DISTINCT * 2);
    let tiles = build_tiles(DISTINCT);
    let canonical: Vec<usize> = (0..tiles.len()).collect();

    // Serial reference run (single thread, canonical order).
    let ref_dir = tempfile::tempdir().unwrap();
    let ref_base = ref_dir.path().join("out");
    run(&ref_base, &plan, &tiles, &canonical, 1);

    // Sanity: dedupe actually engaged.
    assert!(
        ref_base.join("_shared").is_dir(),
        "reference run must have produced a _shared/ directory"
    );
    let ref_tree = tree_snapshot(&ref_base);
    let ref_refs = blank_references(&ref_base);
    #[cfg(unix)]
    assert_every_shared_blob_is_hardlinked(&ref_base);

    for iter in 0..24 {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("out");
        // 8 host threads racing distinct-content promotes through one sink.
        run(&base, &plan, &tiles, &canonical, 8);

        let tree = tree_snapshot(&base);
        assert_eq!(
            tree.keys().collect::<Vec<_>>(),
            ref_tree.keys().collect::<Vec<_>>(),
            "iter {iter}: concurrent run produced a different SET of files than the serial replay"
        );
        for (rel, bytes) in &tree {
            assert_eq!(
                bytes, &ref_tree[rel],
                "iter {iter}: file {rel} differs between the concurrent run and the serial replay"
            );
        }
        assert_eq!(
            blank_references(&base),
            ref_refs,
            "iter {iter}: blank_references depends on concurrent scheduling"
        );
        #[cfg(unix)]
        assert_every_shared_blob_is_hardlinked(&base);
    }
}

/// All tiles of DISTINCT content written concurrently must simply succeed —
/// every shared blob materialised, every duplicate resolvable — regardless of
/// how many host threads drive the sink.
#[test]
fn concurrent_distinct_content_all_succeed() {
    const DISTINCT: u32 = 32;
    let plan = wide_plan(DISTINCT * 2);
    let tiles = build_tiles(DISTINCT);
    let order: Vec<usize> = (0..tiles.len()).collect();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("out");
    run(&base, &plan, &tiles, &order, 8);

    // One shared blob per distinct colour.
    let shared_dir = base.join("_shared");
    let count = std::fs::read_dir(&shared_dir).unwrap().count();
    assert_eq!(
        count as u32, DISTINCT,
        "expected one shared blob per distinct content ({DISTINCT}), got {count}"
    );

    // Every planned tile path resolves to a readable file (full payload or
    // placeholder) — nothing was dropped by the concurrent promotes.
    for (coord, _) in &tiles {
        let rel = plan.tile_path(*coord, "png").unwrap();
        assert!(
            base.join(&rel).exists(),
            "tile {rel} missing after concurrent dedupe writes"
        );
    }

    #[cfg(unix)]
    assert_every_shared_blob_is_hardlinked(&base);
}
