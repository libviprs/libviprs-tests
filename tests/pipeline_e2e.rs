//! Pipeline features, end to end, on committed fixtures (libviprs-tests#231).
//!
//! EPIC M (libviprs#1135) closed with its e2e deliverable undone: nothing in
//! `tests/` named `Layout::Arrival`, `dedupe_memory_bytes` or ordered emission,
//! and several other pipeline features had no cell either. This binary is that
//! deliverable. Every cell drives the public surface the way a caller would,
//! on `canonical_input.png` (through `canonical_raster_scaled`), and checks the
//! output rather than a counter where the output is what the feature promises.
//!
//! The sections, in order:
//!
//! * **PMTiles arrival layout, the dedupe budget and ordered emission.** The
//!   ordered/unordered comparison runs at a concurrency of one on both sides.
//!   Under several workers an unordered run is a race, and a race can land in
//!   tile id order, which would hide exactly the regression the comparison
//!   exists for. go-pmtiles reads the archives as an independent reader when
//!   the pinned binary is reachable (`$GO_PMTILES_BIN` or `pmtiles` on `PATH`),
//!   and `VIPRS_REQUIRE_GO_PMTILES=1` turns its absence into a failure, the
//!   same contract `pmtiles_interop.rs` has.
//! * **Cancel then resume.** A run is cancelled through `CancelToken` part way
//!   through, resumed, and the finished tree is compared byte for byte with an
//!   uninterrupted run. Not tile counts: a resume that rewrote a tile wrongly
//!   has the right count.
//! * **Retry**, with fail-fast on and off, against a sink that fails one chosen
//!   tile a chosen number of times.
//! * **Verify.** `stream_verify::verify_from_strip_source` and
//!   `verify::pyramid_verify`, each passing a clean output and refusing a
//!   corrupted one, and each negative control asserts the refusal names the
//!   corruption it was given rather than accepting any error at all.
//! * **Observer events**: the counts agree with the plan, and the peak memory
//!   figure is reported and inside its bound.
//! * **`streaming_mapreduce`**, **`storage::output_path`** and **`geo`**, which
//!   had no e2e cell before this file.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use libviprs::pmtiles::{Header, Layout as ArchiveLayout, Reader, WriterOptions, zxy_to_tileid};
use libviprs::sink::SinkError;
use libviprs::sink_pmtiles::PmTilesSink;
use libviprs::streaming::RasterStripSource;
use libviprs::{
    CancelToken, CollectingObserver, DirectoryPyramidReader, EngineBuilder, EngineConfig,
    EngineError, EngineEvent, EngineKind, EngineObserver, FailurePolicy, FsSink, GeoCoord,
    GeoTransform, Layout, LocalWorkExecutor, MemorySink, PixelCoord, PmTilesPyramidReader,
    PyramidPlan, PyramidPlanner, PyramidStorage, Raster, ResumePolicy, RetryPolicy, StripWorkUnit,
    Tile, TileCoord, TileFormat, TileSink, WorkContext, WorkExecutor,
};

mod common;
use common::fixtures::canonical_raster_scaled;

#[path = "common/pmtiles.rs"]
mod pmtiles_support;
use pmtiles_support::ORACLE_RELEASE_TAG;

// ===========================================================================
// Shared helpers
// ===========================================================================

/// The PMTiles header is the first 127 bytes of the archive.
const HEADER_BYTES: usize = 127;

/// Byte offset of the `clustered` flag inside the header.
const CLUSTERED_BYTE: usize = 96;

/// Byte offset of `addressed_tiles_count` inside the header, a little-endian
/// `u64`.
const ADDRESSED_TILES_OFFSET: usize = 72;

/// A ZXY plan over a square source, so the archive cells have tile ids.
fn xyz_plan(side: u32, tile: u32) -> PyramidPlan {
    PyramidPlanner::new(side, side, tile, 0, Layout::Xyz)
        .expect("a valid ZXY plan")
        .plan()
}

/// A DeepZoom plan for the directory-tree cells. Not square on purpose, so a
/// transposed walk would address coordinates that do not exist.
fn deepzoom_plan(w: u32, h: u32, tile: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, tile, 0, Layout::DeepZoom)
        .expect("a valid DeepZoom plan")
        .plan()
}

/// The header of an archive on disk.
fn header_of(path: &Path) -> Header {
    let bytes = std::fs::read(path).expect("the archive is readable");
    Header::try_decode(&bytes[..HEADER_BYTES]).expect("a v3 header")
}

/// The raw tile data region of an archive.
fn tile_data(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).expect("the archive is readable");
    let header = Header::try_decode(&bytes[..HEADER_BYTES]).expect("a v3 header");
    let start = usize::try_from(header.tile_data_offset).expect("fits");
    let len = usize::try_from(header.tile_data_length).expect("fits");
    bytes[start..start + len].to_vec()
}

/// `(z, x, y)` of a ZXY plan coordinate.
fn zxy(coord: TileCoord) -> (u8, u32, u32) {
    (
        u8::try_from(coord.level).expect("a zoom fits a u8"),
        coord.col,
        coord.row,
    )
}

/// Every planned coordinate, in ascending tile id order, with its id.
fn coords_by_tile_id(plan: &PyramidPlan) -> Vec<(u64, TileCoord)> {
    let mut out: Vec<(u64, TileCoord)> = plan
        .tile_coords()
        .map(|c| {
            let (z, x, y) = zxy(c);
            (zxy_to_tileid(z, x, y).expect("a valid tile id"), c)
        })
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// Where each tile's payload sits in the data region, in tile id order.
///
/// A run of duplicates shares one offset, so the offsets are taken over the
/// distinct payloads in first-seen order, which is what "the data region is in
/// tile id order" means once dedupe is in play.
fn first_seen_offsets_in_tile_id_order(archive: &Path, plan: &PyramidPlan) -> Vec<u64> {
    let reader = Reader::try_open(archive).expect("open the archive");
    let mut seen = BTreeSet::new();
    let mut offsets = Vec::new();
    for (_, coord) in coords_by_tile_id(plan) {
        let (z, x, y) = zxy(coord);
        let (offset, _len) = reader
            .tile_span(z, x, y)
            .expect("walk the directories")
            .unwrap_or_else(|| panic!("the archive has no tile at {coord:?}"));
        if seen.insert(offset) {
            offsets.push(offset);
        }
    }
    offsets
}

/// Generate `plan` from `src` into an archive at `path`.
fn run_archive(
    src: &Raster,
    plan: &PyramidPlan,
    path: &Path,
    options: WriterOptions,
    ordered: bool,
    concurrency: usize,
) {
    let sink = PmTilesSink::builder(path)
        .plan(plan.clone())
        .tile_format(TileFormat::Png)
        .writer_options(options)
        .ordered_emission(ordered)
        .build()
        .expect("build a PmTilesSink");
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_concurrency(concurrency)
        .run()
        .expect("generate into the archive");
    assert!(
        sink.published_header().is_some(),
        "the sink never published a header, so there is no archive to check"
    );
}

/// Arrival-layout writer options, with the dedupe budget overridden when given.
fn arrival(dedupe_budget: Option<usize>) -> WriterOptions {
    let options = WriterOptions::default().with_layout(ArchiveLayout::Arrival);
    match dedupe_budget {
        Some(bytes) => options.with_dedupe_memory_bytes(bytes),
        None => options,
    }
}

/// Generate `plan` from `src` into a PNG tree, the literal-path reference the
/// archive cells compare against.
fn run_tree(src: &Raster, plan: &PyramidPlan, base: &Path, format: TileFormat) {
    let sink = FsSink::new(base, plan.clone()).with_format(format);
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .run()
        .expect("generate into FsSink");
}

/// Every regular file under `dir`, relative path to bytes.
fn tree_files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, cur: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(cur).expect("read_dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let bytes = std::fs::read(&path).expect("read a file");
                out.insert(
                    path.strip_prefix(root).expect("under root").to_path_buf(),
                    bytes,
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// The path of `coord`'s tile under a tree rooted at `base`.
fn tile_file(base: &Path, plan: &PyramidPlan, coord: TileCoord, ext: &str) -> PathBuf {
    base.join(
        plan.tile_path(coord, ext)
            .unwrap_or_else(|| panic!("the plan has no path for {coord:?}")),
    )
}

/// The tile a ZXY tree holds at `coord`, read by literal path.
fn tree_tile(base: &Path, plan: &PyramidPlan, coord: TileCoord) -> Vec<u8> {
    std::fs::read(tile_file(base, plan, coord, "png"))
        .unwrap_or_else(|e| panic!("the tree has no tile at {coord:?}: {e}"))
}

/// Encoded tiles keyed by `(level, col, row)`.
type Tiles = BTreeMap<(u32, u32, u32), Vec<u8>>;

/// Tiles of a `MemorySink`, keyed by coordinate.
fn memory_tiles(sink: &MemorySink) -> Tiles {
    sink.tiles()
        .into_iter()
        .map(|t| ((t.coord.level, t.coord.col, t.coord.row), t.data))
        .collect()
}

/// The reference output: the plan through the monolithic engine into memory.
fn reference_tiles(src: &Raster, plan: &PyramidPlan) -> Tiles {
    let sink = MemorySink::new();
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .run()
        .expect("reference run");
    let tiles = memory_tiles(&sink);
    assert_eq!(
        tiles.len() as u64,
        plan.total_tile_count(),
        "the reference run has to hold every planned tile or nothing compares against it"
    );
    tiles
}

// ===========================================================================
// go-pmtiles, the independent reader
// ===========================================================================

fn require_go_pmtiles() -> bool {
    std::env::var("VIPRS_REQUIRE_GO_PMTILES").is_ok_and(|v| v == "1")
}

/// The pinned `pmtiles` binary, or `None` when it is not reachable.
///
/// Same contract as `pmtiles_interop.rs`: `$GO_PMTILES_BIN` wins, otherwise
/// `pmtiles` on `PATH`, the version has to be the pinned release whichever
/// route found it, and `VIPRS_REQUIRE_GO_PMTILES=1` turns a skip into a panic
/// so the interop job cannot go green without running it.
fn go_pmtiles_bin() -> Option<PathBuf> {
    let candidate = match std::env::var_os("GO_PMTILES_BIN") {
        Some(explicit) => {
            let path = PathBuf::from(explicit);
            assert!(
                path.is_file(),
                "$GO_PMTILES_BIN points at {}, which is not a file",
                path.display()
            );
            path
        }
        None => PathBuf::from("pmtiles"),
    };
    let ran = match Command::new(&candidate).arg("version").output() {
        Ok(out) if out.status.success() => out,
        _ => {
            assert!(
                !require_go_pmtiles(),
                "VIPRS_REQUIRE_GO_PMTILES=1 but `{} version` did not run, so every \
                 go-pmtiles comparison in this file would skip and report green",
                candidate.display()
            );
            eprintln!("go-pmtiles not reachable; skipping the independent-reader half");
            return None;
        }
    };
    let version = format!(
        "{}{}",
        String::from_utf8_lossy(&ran.stdout),
        String::from_utf8_lossy(&ran.stderr)
    );
    assert!(
        version.contains(ORACLE_RELEASE_TAG.trim_start_matches('v')),
        "`{} version` reports {version:?}, not the pinned {ORACLE_RELEASE_TAG}",
        candidate.display()
    );
    Some(candidate)
}

fn go_pmtiles(bin: &Path, args: &[&str], archive: &Path) -> std::process::Output {
    Command::new(bin)
        .args(args)
        .arg(archive)
        .output()
        .unwrap_or_else(|e| panic!("run pmtiles {args:?}: {e}"))
}

fn go_pmtiles_header(bin: &Path, archive: &Path) -> serde_json::Value {
    let out = go_pmtiles(bin, &["show", "--header-json"], archive);
    assert!(
        out.status.success(),
        "`pmtiles show --header-json` exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "`pmtiles show --header-json` printed no JSON: {e}\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn go_pmtiles_verifies(bin: &Path, archive: &Path) {
    let out = go_pmtiles(bin, &["verify"], archive);
    assert!(
        out.status.success(),
        "`pmtiles verify` exited {} on {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status,
        archive.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

fn go_pmtiles_tile(bin: &Path, archive: &Path, coord: TileCoord) -> Vec<u8> {
    let (z, x, y) = zxy(coord);
    let out = Command::new(bin)
        .args(["tile", "--quiet"])
        .arg(archive)
        .arg(z.to_string())
        .arg(x.to_string())
        .arg(y.to_string())
        .output()
        .expect("run pmtiles tile");
    assert!(
        out.status.success(),
        "`pmtiles tile {z} {x} {y}` exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// One `label: value` line of plain `pmtiles show`.
///
/// `--header-json` prints only part of the header, and the part it leaves out
/// is the part these cells are about (the three counts and `clustered`), so
/// this reads the human listing instead and refuses a label it cannot find.
fn go_pmtiles_shown(bin: &Path, archive: &Path, label: &str) -> String {
    let out = go_pmtiles(bin, &["show"], archive);
    assert!(
        out.status.success(),
        "`pmtiles show` exited {}: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{label}: ")))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("`pmtiles show` printed no {label:?} line:\n{text}"))
}

fn go_pmtiles_count(bin: &Path, archive: &Path, label: &str) -> u64 {
    let raw = go_pmtiles_shown(bin, archive, label);
    raw.trim()
        .parse()
        .unwrap_or_else(|e| panic!("`pmtiles show` {label} is {raw:?}, not a number: {e}"))
}

// ===========================================================================
// PMTiles: arrival layout, ordered emission, the dedupe budget
// ===========================================================================

/// The ordered/unordered pair, both at a concurrency of one.
///
/// One worker on the default cascade emits the full-resolution level first and
/// the overview last, so the lowest tile id sits at the far end of an
/// unordered arrival archive every time. That is what makes the unordered half
/// a control: it is `false` deterministically, and an ordered run that stopped
/// ordering would look exactly like it. The ordered half is at one worker too,
/// so the only thing the pair varies is the emission order.
fn arrival_pair(dir: &Path) -> (PyramidPlan, PathBuf, PathBuf) {
    let plan = xyz_plan(1024, 256);
    let src = canonical_raster_scaled(1024, 1024);
    let ordered = dir.join("ordered.pmtiles");
    let unordered = dir.join("unordered.pmtiles");
    run_archive(&src, &plan, &ordered, arrival(None), true, 1);
    run_archive(&src, &plan, &unordered, arrival(None), false, 1);
    (plan, ordered, unordered)
}

#[test]
fn arrival_ordered_run_is_in_tile_id_order_and_the_unordered_control_is_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, ordered, unordered) = arrival_pair(dir.path());

    let oh = header_of(&ordered);
    let uh = header_of(&unordered);
    assert_eq!(
        oh.addressed_tiles_count,
        plan.total_tile_count(),
        "the ordered archive must address every planned tile"
    );
    assert_eq!(
        uh.addressed_tiles_count, oh.addressed_tiles_count,
        "the pair has to cover the same tiles for the comparison to mean anything"
    );
    assert_eq!(
        oh.tile_data_offset, 16384,
        "an arrival archive appends tile data after the reserved 16384-byte prefix, \
         so a different offset means the run did not take the arrival layout at all"
    );

    let ordered_offsets = first_seen_offsets_in_tile_id_order(&ordered, &plan);
    assert!(
        ordered_offsets.len() > 1,
        "the positive control: one payload is in every order at once"
    );
    assert!(
        ordered_offsets.windows(2).all(|w| w[0] < w[1]),
        "an ordered arrival run must write its payloads in tile id order, got {ordered_offsets:?}"
    );
    assert!(oh.clustered, "an ordered arrival archive earns `clustered`");
    let ordered_bytes = std::fs::read(&ordered).expect("read");
    assert_eq!(
        ordered_bytes[CLUSTERED_BYTE], 1,
        "byte 96 follows the decoded flag"
    );

    let unordered_offsets = first_seen_offsets_in_tile_id_order(&unordered, &plan);
    assert!(
        unordered_offsets.windows(2).any(|w| w[0] > w[1]),
        "the control: an unordered single-worker arrival run is in cascade order, so its \
         payloads cannot be in tile id order; {unordered_offsets:?} says the emission order \
         is not being honoured either way"
    );
    assert!(
        !uh.clustered,
        "an unordered arrival archive is not in tile id order and must say so"
    );
    let unordered_bytes = std::fs::read(&unordered).expect("read");
    assert_eq!(
        unordered_bytes[CLUSTERED_BYTE], 0,
        "byte 96 follows the decoded flag"
    );
}

#[test]
fn arrival_ordered_runs_under_four_workers_are_byte_identical_and_match_the_tile_id_layout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = xyz_plan(1024, 256);
    let src = canonical_raster_scaled(1024, 1024);
    // One file name in three directories. The archive's metadata `name`
    // defaults to the file stem, so two names would differ in that field and
    // say nothing about the thread schedule.
    let mut paths = Vec::new();
    for sub in ["first", "second", "sorted"] {
        std::fs::create_dir(dir.path().join(sub)).expect("mkdir");
        paths.push(dir.path().join(sub).join("run.pmtiles"));
    }
    let (first, second, sorted) = (&paths[0], &paths[1], &paths[2]);
    run_archive(&src, &plan, first, arrival(None), true, 4);
    run_archive(&src, &plan, second, arrival(None), true, 4);
    run_archive(&src, &plan, sorted, WriterOptions::default(), false, 4);

    let a = std::fs::read(first).expect("read");
    let b = std::fs::read(second).expect("read");
    assert!(
        a.len() > 16384,
        "the positive control: an empty archive matches itself"
    );
    assert!(
        a == b,
        "two ordered arrival runs over one source must be byte-identical whatever the \
         thread schedule did ({} vs {} bytes)",
        a.len(),
        b.len()
    );

    assert_eq!(
        header_of(first).tile_contents_count,
        header_of(sorted).tile_contents_count,
        "the dedupe window held the whole job in both runs, or this comparison is about \
         the window rather than the layout"
    );
    assert!(
        tile_data(first) == tile_data(sorted),
        "an ordered arrival run must store the data region the tile id layout sorts into"
    );
}

#[test]
fn arrival_archives_read_back_tile_for_tile_against_the_directory_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, ordered, unordered) = arrival_pair(dir.path());
    let src = canonical_raster_scaled(1024, 1024);
    let tree = dir.path().join("tree");
    run_tree(&src, &plan, &tree, TileFormat::Png);

    for archive in [&ordered, &unordered] {
        let reader = Reader::try_open(archive).expect("open the archive");
        let mut compared = 0;
        for coord in plan.tile_coords() {
            let (z, x, y) = zxy(coord);
            let ours = reader
                .get_tile(z, x, y)
                .expect("read a tile")
                .unwrap_or_else(|| panic!("{} has no tile at {coord:?}", archive.display()));
            assert!(
                ours == tree_tile(&tree, &plan, coord),
                "{} holds different bytes at {coord:?} than the tree's literal path",
                archive.display()
            );
            compared += 1;
        }
        assert_eq!(
            compared,
            plan.total_tile_count(),
            "every planned tile compared"
        );
    }
}

/// A source whose tiles repeat at long range: a 1024x1024 block of the fixture
/// tiled twice in each direction, so every top-level payload recurs three more
/// times, at least sixteen distinct payloads apart in any walk of a quadrant.
fn long_range_duplicates() -> Raster {
    let block = canonical_raster_scaled(1024, 1024);
    let bpp = 3usize;
    let (bw, side) = (1024usize, 2048usize);
    let mut data = vec![0u8; side * side * bpp];
    for y in 0..side {
        let src_row = &block.data()[(y % bw) * bw * bpp..(y % bw + 1) * bw * bpp];
        for half in 0..2 {
            let dst = (y * side + half * bw) * bpp;
            data[dst..dst + bw * bpp].copy_from_slice(src_row);
        }
    }
    Raster::new(2048, 2048, block.format(), data).expect("the tiled raster")
}

/// The dedupe budget is honoured, in both directions.
///
/// A budget of zero still buys one set of eight payloads, and every repeat in
/// this source is at least sixteen distinct payloads away from its first
/// occurrence, so at least eight of each quadrant's repeats miss the window and
/// get stored again. The default budget holds the whole job, so it stores each
/// distinct payload once. Both archives have to read back identically: missing
/// a duplicate costs bytes, never content.
#[test]
fn dedupe_budget_zero_stores_long_range_duplicates_again_and_the_default_does_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = xyz_plan(2048, 256);
    let src = long_range_duplicates();
    let tight = dir.path().join("tight.pmtiles");
    let roomy = dir.path().join("roomy.pmtiles");
    run_archive(&src, &plan, &tight, arrival(Some(0)), true, 1);
    run_archive(&src, &plan, &roomy, arrival(None), true, 1);

    let tight_reader = Reader::try_open(&tight).expect("open");
    let roomy_reader = Reader::try_open(&roomy).expect("open");
    let mut distinct = BTreeSet::new();
    for coord in plan.tile_coords() {
        let (z, x, y) = zxy(coord);
        let a = tight_reader
            .get_tile(z, x, y)
            .expect("read")
            .expect("present");
        let b = roomy_reader
            .get_tile(z, x, y)
            .expect("read")
            .expect("present");
        assert!(
            a == b,
            "the two budgets must read back the same tile at {coord:?}"
        );
        distinct.insert(a);
    }

    let roomy_contents = header_of(&roomy).tile_contents_count;
    let tight_contents = header_of(&tight).tile_contents_count;
    assert!(
        (distinct.len() as u64) < plan.total_tile_count(),
        "the positive control: this source has to carry duplicates for the budget to matter"
    );
    assert_eq!(
        roomy_contents,
        distinct.len() as u64,
        "the default budget holds every payload of this job, so it stores each distinct one once"
    );
    assert!(
        tight_contents >= roomy_contents + 8,
        "a zero budget is an eight-payload window, so long-range repeats must be stored again \
         ({tight_contents} contents against {roomy_contents} distinct); an equal count means \
         the budget never reached the writer"
    );
}

/// go-pmtiles reads every archive this section writes, and agrees with it.
#[test]
fn go_pmtiles_reads_the_arrival_archives_and_agrees_about_order_and_contents() {
    let Some(bin) = go_pmtiles_bin() else { return };
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, ordered, unordered) = arrival_pair(dir.path());
    let src = canonical_raster_scaled(1024, 1024);
    let tree = dir.path().join("tree");
    run_tree(&src, &plan, &tree, TileFormat::Png);

    for (archive, want_clustered) in [(&ordered, true), (&unordered, false)] {
        go_pmtiles_verifies(&bin, archive);
        assert_eq!(
            go_pmtiles_shown(&bin, archive, "clustered"),
            want_clustered.to_string(),
            "go-pmtiles reads the clustered flag of {} as the other value",
            archive.display()
        );
        assert_eq!(
            go_pmtiles_count(&bin, archive, "addressed tiles count"),
            plan.total_tile_count(),
            "go-pmtiles counts a different number of tiles in {}",
            archive.display()
        );
        // `verify` never checks the clustered claim, and `show` only repeats
        // the byte. `makesync` is the command that depends on it: it walks the
        // entries assuming they are in order, exits 0 on an honest clustered
        // archive and refuses one that says it is not.
        let sync = go_pmtiles(&bin, &["makesync", "--quiet"], archive);
        assert_eq!(
            sync.status.success(),
            want_clustered,
            "`pmtiles makesync` on {} exited {} (clustered should be {want_clustered})\n{}",
            archive.display(),
            sync.status,
            String::from_utf8_lossy(&sync.stderr)
        );
        for coord in plan.tile_coords() {
            assert!(
                go_pmtiles_tile(&bin, archive, coord) == tree_tile(&tree, &plan, coord),
                "go-pmtiles finds different bytes at {coord:?} in {} than the tree holds",
                archive.display()
            );
        }
    }

    let plan2 = xyz_plan(2048, 256);
    let src2 = long_range_duplicates();
    let tight = dir.path().join("tight.pmtiles");
    let roomy = dir.path().join("roomy.pmtiles");
    run_archive(&src2, &plan2, &tight, arrival(Some(0)), true, 1);
    run_archive(&src2, &plan2, &roomy, arrival(None), true, 1);
    for archive in [&tight, &roomy] {
        go_pmtiles_verifies(&bin, archive);
        assert_eq!(
            go_pmtiles_count(&bin, archive, "tile contents count"),
            header_of(archive).tile_contents_count,
            "go-pmtiles and libviprs disagree about how many payloads {} stores",
            archive.display()
        );
    }
}

// ===========================================================================
// Cancel, then resume
// ===========================================================================

/// Cancels the run once it has seen `after` completed tiles.
struct CancelAfter {
    token: CancelToken,
    seen: AtomicU64,
    after: u64,
}

impl EngineObserver for CancelAfter {
    fn on_event(&self, event: EngineEvent) {
        if let EngineEvent::TileCompleted { .. } = event
            && self.seen.fetch_add(1, Ordering::SeqCst) + 1 >= self.after
        {
            self.token.cancel();
        }
    }
}

/// Files a run leaves beside the tiles that describe the run rather than the
/// pyramid. The resume cell compares everything else byte for byte.
fn is_job_state(rel: &Path) -> bool {
    rel.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(".libviprs-job"))
}

/// How many tiles the checkpoint under `dir` records as done.
fn checkpointed_tiles(dir: &Path) -> u64 {
    libviprs::JobCheckpoint::load(dir)
        .expect("the checkpoint loads")
        .unwrap_or_else(|| panic!("no checkpoint under {}", dir.display()))
        .completed_tiles
        .len() as u64
}

/// Cancel a resumable run part way, as `cancel_then_resume_*` do, and return
/// the plan, the source, the directory and how many tiles the checkpoint holds.
fn cancelled_run(dir: &Path) -> (PyramidPlan, Raster, PathBuf, u64) {
    let plan = deepzoom_plan(640, 448, 64);
    let src = canonical_raster_scaled(640, 448);
    let base = dir.join("resumed");
    let token = CancelToken::new();
    let sink = FsSink::new(&base, plan.clone()).with_format(TileFormat::Png);
    let first = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_concurrency(1)
        .with_resume(ResumePolicy::resume().with_checkpoint_every(1))
        .with_cancel(token.clone())
        .with_observer(CancelAfter {
            token: token.clone(),
            seen: AtomicU64::new(0),
            after: plan.total_tile_count() / 3,
        })
        .run();
    assert!(
        matches!(first, Err(EngineError::Cancelled)),
        "got {first:?}"
    );
    let checkpointed = checkpointed_tiles(&base);
    (plan, src, base, checkpointed)
}

fn cancel_then_resume_matches_an_uninterrupted_run(kind: EngineKind) {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = deepzoom_plan(640, 448, 64);
    let src = canonical_raster_scaled(640, 448);
    let total = plan.total_tile_count();
    assert!(total > 40, "the plan needs room to stop part way through");

    let reference = dir.path().join("reference");
    let sink = FsSink::new(&reference, plan.clone()).with_format(TileFormat::Png);
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(kind)
        .with_concurrency(4)
        .run()
        .expect("the uninterrupted run");

    let resumed = dir.path().join("resumed");
    let token = CancelToken::new();
    let first = {
        let sink = FsSink::new(&resumed, plan.clone()).with_format(TileFormat::Png);
        EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(kind)
            .with_concurrency(4)
            .with_resume(ResumePolicy::resume().with_checkpoint_every(1))
            .with_cancel(token.clone())
            .with_observer(CancelAfter {
                token: token.clone(),
                seen: AtomicU64::new(0),
                after: total / 3,
            })
            .run()
    };
    assert!(
        matches!(first, Err(EngineError::Cancelled)),
        "a cancel mid-flight must end the run as Cancelled, got {first:?}"
    );
    let partial = tree_files(&resumed)
        .keys()
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .count() as u64;
    assert!(
        partial > 0 && partial < total,
        "the cancel has to land mid-flight for a resume to prove anything: {partial} of \
         {total} tiles on disk"
    );

    let checkpointed = checkpointed_tiles(&resumed);
    assert!(
        checkpointed > 0 && checkpointed < total,
        "the cancelled run has to leave a checkpoint naming some tiles and not all of \
         them: {checkpointed} of {total}"
    );

    let sink = FsSink::new(&resumed, plan.clone()).with_format(TileFormat::Png);
    let second = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(kind)
        .with_concurrency(4)
        .with_resume(ResumePolicy::resume().with_checkpoint_every(1))
        .run()
        .expect("the resumed run");
    assert_eq!(
        second.tiles_produced,
        total - checkpointed,
        "the resumed run has to write exactly the tiles the checkpoint does not name; \
         writing all {total} would mean it regenerated the pyramid rather than resuming it, \
         and the byte comparison below would prove nothing about resume"
    );

    let want: BTreeMap<_, _> = tree_files(&reference)
        .into_iter()
        .filter(|(p, _)| !is_job_state(p))
        .collect();
    let got: BTreeMap<_, _> = tree_files(&resumed)
        .into_iter()
        .filter(|(p, _)| !is_job_state(p))
        .collect();
    assert_eq!(
        got.keys().collect::<Vec<_>>(),
        want.keys().collect::<Vec<_>>(),
        "the resumed tree holds a different set of files from the uninterrupted one"
    );
    for (path, bytes) in &want {
        assert!(
            &got[path] == bytes,
            "{} differs between the resumed and the uninterrupted run",
            path.display()
        );
    }
}

#[test]
fn cancel_then_resume_is_byte_identical_monolithic() {
    cancel_then_resume_matches_an_uninterrupted_run(EngineKind::Monolithic);
}

#[test]
fn cancel_then_resume_is_byte_identical_streaming() {
    cancel_then_resume_matches_an_uninterrupted_run(EngineKind::Streaming);
}

/// A resumed run reports the tiles it skipped as `TileSkippedOnResume`, and
/// only the tiles it wrote as `TileCompleted`.
///
/// That is what `EngineEvent::TileSkippedOnResume` documents: "emitted for
/// tiles listed in an inbound resume checkpoint", which "must not surface as
/// TileCompleted", because an observer would otherwise double-count them
/// across the original and the resumed run.
#[test]
fn resume_reports_skipped_tiles_as_skipped_and_not_as_completed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, src, base, checkpointed) = cancelled_run(dir.path());
    let total = plan.total_tile_count();

    let observer = Arc::new(CollectingObserver::new());
    let sink = FsSink::new(&base, plan.clone()).with_format(TileFormat::Png);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_concurrency(1)
        .with_resume(ResumePolicy::resume().with_checkpoint_every(1))
        .with_observer_arc(observer.clone())
        .run()
        .expect("the resumed run");
    assert_eq!(result.tiles_produced, total - checkpointed);

    let events = observer.events();
    let completed = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::TileCompleted { .. }))
        .count() as u64;
    let skipped = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::TileSkippedOnResume { .. }))
        .count() as u64;
    assert_eq!(
        (completed, skipped),
        (total - checkpointed, checkpointed),
        "(TileCompleted, TileSkippedOnResume) for a resume over a checkpoint naming \
         {checkpointed} of {total} tiles"
    );
}

// ===========================================================================
// Retry, fail-fast on and off
// ===========================================================================

/// A sink that fails one chosen tile a chosen number of times, then lets it
/// through, and counts every attempt per coordinate.
struct FlakySink {
    inner: MemorySink,
    target: TileCoord,
    failures_left: AtomicU32,
    attempts: Mutex<BTreeMap<(u32, u32, u32), u32>>,
}

impl FlakySink {
    fn new(target: TileCoord, failures: u32) -> Self {
        Self {
            inner: MemorySink::new(),
            target,
            failures_left: AtomicU32::new(failures),
            attempts: Mutex::new(BTreeMap::new()),
        }
    }

    fn attempts_at(&self, coord: TileCoord) -> u32 {
        self.attempts
            .lock()
            .expect("attempts")
            .get(&(coord.level, coord.col, coord.row))
            .copied()
            .unwrap_or(0)
    }

    fn attempts(&self) -> BTreeMap<(u32, u32, u32), u32> {
        self.attempts.lock().expect("attempts").clone()
    }
}

impl TileSink for FlakySink {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        let c = tile.coord;
        *self
            .attempts
            .lock()
            .expect("attempts")
            .entry((c.level, c.col, c.row))
            .or_insert(0) += 1;
        if c == self.target
            && self
                .failures_left
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
        {
            return Err(SinkError::Other(format!("injected failure at {c:?}")));
        }
        self.inner.write_tile(tile)
    }

    fn finish(&self) -> Result<(), SinkError> {
        self.inner.finish()
    }
}

fn retry_fixture() -> (Raster, PyramidPlan, TileCoord) {
    let plan = deepzoom_plan(256, 192, 64);
    let src = canonical_raster_scaled(256, 192);
    let top = plan.levels.last().expect("a level");
    // An interior tile of the top level, so neither the first nor the last
    // write of the run is the one that fails.
    let target = TileCoord::new(top.level, 1, 1);
    (src, plan, target)
}

fn no_backoff(max_retries: u32) -> RetryPolicy {
    RetryPolicy::new(max_retries, Duration::ZERO).with_jitter(false)
}

fn run_flaky(
    src: &Raster,
    plan: &PyramidPlan,
    sink: &FlakySink,
    policy: FailurePolicy,
    observer: Arc<CollectingObserver>,
) -> Result<libviprs::EngineResult, EngineError> {
    EngineBuilder::new(src, plan.clone(), sink)
        .with_engine(EngineKind::Monolithic)
        .with_concurrency(1)
        .with_failure_policy(policy)
        .with_observer_arc(observer)
        .run()
}

fn assert_failed_at(result: &Result<libviprs::EngineResult, EngineError>, target: TileCoord) {
    match result {
        Err(e) => assert!(
            e.to_string().contains("injected failure"),
            "the run failed, but not with the injected failure at {target:?}: {e}"
        ),
        Ok(r) => panic!("the run succeeded through a failure at {target:?}: {r:?}"),
    }
}

#[test]
fn fail_fast_stops_at_the_first_failure_of_the_chosen_tile() {
    let (src, plan, target) = retry_fixture();
    for (label, policy) in [
        ("FailFast", FailurePolicy::FailFast),
        (
            "RetryThenFail(fail_fast)",
            FailurePolicy::RetryThenFail(RetryPolicy::fail_fast()),
        ),
    ] {
        let sink = FlakySink::new(target, 1);
        let result = run_flaky(
            &src,
            &plan,
            &sink,
            policy,
            Arc::new(CollectingObserver::new()),
        );
        assert_failed_at(&result, target);
        assert_eq!(
            sink.attempts_at(target),
            1,
            "{label}: fail-fast must not try the failing tile a second time"
        );
        assert!(
            (sink.inner.tile_count() as u64) < plan.total_tile_count(),
            "{label}: fail-fast must stop the run, not carry on past the failure"
        );
    }
}

#[test]
fn retry_recovers_when_the_failures_fit_inside_the_budget() {
    let (src, plan, target) = retry_fixture();
    let sink = FlakySink::new(target, 2);
    let observer = Arc::new(CollectingObserver::new());
    let result = run_flaky(
        &src,
        &plan,
        &sink,
        FailurePolicy::RetryThenFail(no_backoff(3)),
        observer.clone(),
    )
    .expect("two failures fit inside three retries");

    assert_eq!(
        sink.attempts_at(target),
        3,
        "two failures and then the write that lands"
    );
    for (coord, n) in sink.attempts() {
        if coord != (target.level, target.col, target.row) {
            assert_eq!(
                n, 1,
                "{coord:?} never failed and must have been written once"
            );
        }
    }
    assert_eq!(result.retry_count, 2, "the result reports the two retries");
    assert_eq!(result.skipped_due_to_failure, 0);
    assert!(
        memory_tiles(&sink.inner) == reference_tiles(&src, &plan),
        "a run that recovered through retries must produce the uninterrupted output"
    );
}

#[test]
fn retry_gives_up_when_the_failures_exceed_the_budget() {
    let (src, plan, target) = retry_fixture();
    let sink = FlakySink::new(target, 4);
    let result = run_flaky(
        &src,
        &plan,
        &sink,
        FailurePolicy::RetryThenFail(no_backoff(3)),
        Arc::new(CollectingObserver::new()),
    );
    assert_failed_at(&result, target);
    assert_eq!(
        sink.attempts_at(target),
        4,
        "three retries means four attempts in all, and then the run gives up"
    );
}

#[test]
fn retry_then_skip_drops_only_the_chosen_tile() {
    let (src, plan, target) = retry_fixture();
    let sink = FlakySink::new(target, u32::MAX);
    let observer = Arc::new(CollectingObserver::new());
    let result = run_flaky(
        &src,
        &plan,
        &sink,
        FailurePolicy::RetryThenSkip(no_backoff(2)),
        observer.clone(),
    )
    .expect("RetryThenSkip carries on past a tile that never lands");

    assert_eq!(sink.attempts_at(target), 3, "one attempt and two retries");
    assert_eq!(result.skipped_due_to_failure, 1);
    let mut want = reference_tiles(&src, &plan);
    want.remove(&(target.level, target.col, target.row));
    assert!(
        memory_tiles(&sink.inner) == want,
        "everything but the chosen tile must be the uninterrupted output"
    );
    let failed: Vec<TileCoord> = observer
        .events()
        .into_iter()
        .filter_map(|e| match e {
            EngineEvent::TileFailed { coord, .. } => Some(coord),
            _ => None,
        })
        .collect();
    assert_eq!(
        failed,
        vec![target],
        "exactly the chosen tile is reported as failed"
    );
}

/// `EngineEvent::RetryAttempted` says it fires when "a tile write failed and a
/// retry is about to be attempted", once per retry with a 1-based attempt.
#[test]
fn retry_attempts_are_reported_to_the_observer() {
    let (src, plan, target) = retry_fixture();
    let sink = FlakySink::new(target, 2);
    let observer = Arc::new(CollectingObserver::new());
    run_flaky(
        &src,
        &plan,
        &sink,
        FailurePolicy::RetryThenFail(no_backoff(3)),
        observer.clone(),
    )
    .expect("two failures fit inside three retries");
    let attempts: Vec<(TileCoord, u32)> = observer
        .events()
        .into_iter()
        .filter_map(|e| match e {
            EngineEvent::RetryAttempted { coord, attempt, .. } => Some((coord, attempt)),
            _ => None,
        })
        .collect();
    assert_eq!(
        attempts,
        vec![(target, 1), (target, 2)],
        "two retries of the chosen tile must reach the observer as RetryAttempted 1 and 2"
    );
}

// ===========================================================================
// Verify: verify_from_strip_source and pyramid_verify
// ===========================================================================

/// A clean raw tree, which is the one format `verify_from_strip_source`
/// compares byte for byte against a re-render.
fn raw_tree(dir: &Path) -> (Raster, PyramidPlan, PathBuf) {
    let plan = deepzoom_plan(320, 224, 64);
    let src = canonical_raster_scaled(320, 224);
    let base = dir.join("raw");
    run_tree(&src, &plan, &base, TileFormat::Raw);
    (src, plan, base)
}

fn strip_verify(
    src: &Raster,
    plan: &PyramidPlan,
    base: &Path,
) -> Result<libviprs::EngineResult, EngineError> {
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Raw);
    libviprs::stream_verify::verify_from_strip_source(
        &RasterStripSource::new(src),
        plan,
        &sink,
        &EngineConfig::default(),
        &libviprs::observe::NoopObserver,
    )
}

/// A tile in the middle of the top level, which is neither the first coordinate
/// a walk visits nor the last.
fn interior_top_tile(plan: &PyramidPlan) -> TileCoord {
    let top = plan.levels.last().expect("a level");
    TileCoord::new(top.level, top.cols / 2, top.rows / 2)
}

#[test]
fn stream_verify_passes_a_clean_raw_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (src, plan, base) = raw_tree(dir.path());
    let result = strip_verify(&src, &plan, &base).expect("a clean tree verifies");
    assert_eq!(result.tiles_produced, 0, "verify writes nothing");
    assert_eq!(result.levels_processed as usize, plan.levels.len());
}

#[test]
fn stream_verify_refuses_a_tile_with_one_flipped_byte() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (src, plan, base) = raw_tree(dir.path());
    let victim = interior_top_tile(&plan);
    let path = tile_file(&base, &plan, victim, "raw");
    let mut bytes = std::fs::read(&path).expect("read the tile");
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0x01;
    std::fs::write(&path, bytes).expect("write the corrupted tile");

    match strip_verify(&src, &plan, &base) {
        Err(EngineError::ChecksumMismatch { tile, .. }) => assert_eq!(
            tile, victim,
            "the mismatch must name the tile that was corrupted"
        ),
        other => panic!("a one-bit flip in {victim:?} must be a ChecksumMismatch, got {other:?}"),
    }
}

#[test]
fn stream_verify_refuses_a_missing_tile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (src, plan, base) = raw_tree(dir.path());
    let victim = interior_top_tile(&plan);
    std::fs::remove_file(tile_file(&base, &plan, victim, "raw")).expect("remove the tile");

    let err = strip_verify(&src, &plan, &base).expect_err("a tree missing a tile must not verify");
    assert!(
        err.to_string().contains(&format!("{victim:?}")),
        "the refusal must name the missing tile {victim:?}: {err}"
    );
}

fn pyramid_verify_archive(
    archive: &Path,
    plan: &PyramidPlan,
) -> Result<libviprs::EngineResult, EngineError> {
    let reader = PmTilesPyramidReader::try_open(archive).expect("open the archive");
    libviprs::verify::pyramid_verify(
        &reader,
        plan,
        Some(TileFormat::Png),
        &libviprs::observe::NoopObserver,
    )
}

/// A loose-file tree that can count what it holds.
///
/// `DirectoryPyramidReader` answers `structural_summary` with "cannot count",
/// and `pyramid_verify` refuses any pyramid that cannot count rather than drop
/// its only check for extra tiles (pinned by
/// `pyramid_verify_refuses_a_tree_that_cannot_count_itself` below). So a tree
/// reaches `pyramid_verify` through a reader that walks the directory and
/// counts the tiles in it, which is what a third-party backend over a tree
/// would have to do too. Everything else goes to `DirectoryPyramidReader`.
struct CountingTree {
    inner: DirectoryPyramidReader,
    base: PathBuf,
}

impl libviprs::PyramidReader for CountingTree {
    fn describe(&self) -> Result<libviprs::PyramidDescription, libviprs::PyramidReadError> {
        self.inner.describe()
    }

    fn structural_summary(
        &self,
    ) -> Result<libviprs::pyramid_reader::StructuralSummary, libviprs::PyramidReadError> {
        let tiles = tree_files(&self.base)
            .keys()
            .filter(|p| p.extension().is_some_and(|e| e == "png"))
            .count() as u64;
        Ok(libviprs::pyramid_reader::StructuralSummary::new().with_addressed_tiles(tiles))
    }

    fn tile(&self, coord: TileCoord) -> Result<Option<Vec<u8>>, libviprs::PyramidReadError> {
        self.inner.tile(coord)
    }

    fn tile_format(&self) -> Option<TileFormat> {
        self.inner.tile_format()
    }
}

fn pyramid_verify_tree(
    base: &Path,
    plan: &PyramidPlan,
) -> Result<libviprs::EngineResult, EngineError> {
    let reader = CountingTree {
        inner: DirectoryPyramidReader::try_open(base, plan.clone(), TileFormat::Png)
            .expect("open the tree"),
        base: base.to_path_buf(),
    };
    libviprs::verify::pyramid_verify(
        &reader,
        plan,
        Some(TileFormat::Png),
        &libviprs::observe::NoopObserver,
    )
}

fn clean_archive(dir: &Path) -> (PyramidPlan, PathBuf) {
    let plan = xyz_plan(1024, 256);
    let src = canonical_raster_scaled(1024, 1024);
    let archive = dir.join("clean.pmtiles");
    run_archive(&src, &plan, &archive, WriterOptions::default(), false, 2);
    (plan, archive)
}

fn clean_png_tree(dir: &Path) -> (PyramidPlan, PathBuf) {
    let plan = deepzoom_plan(320, 224, 64);
    let src = canonical_raster_scaled(320, 224);
    let base = dir.join("png");
    run_tree(&src, &plan, &base, TileFormat::Png);
    (plan, base)
}

#[test]
fn pyramid_verify_passes_a_clean_archive_and_a_clean_tree() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, archive) = clean_archive(dir.path());
    let result = pyramid_verify_archive(&archive, &plan).expect("a clean archive verifies");
    assert_eq!(result.tiles_produced, 0, "verify writes nothing");
    assert!(
        result.tile_evidence.is_some(),
        "the run says what it established"
    );

    let (plan, base) = clean_png_tree(dir.path());
    pyramid_verify_tree(&base, &plan).expect("a clean tree verifies");
}

#[test]
fn pyramid_verify_refuses_a_tree_missing_a_tile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, base) = clean_png_tree(dir.path());
    let victim = interior_top_tile(&plan);
    std::fs::remove_file(tile_file(&base, &plan, victim, "png")).expect("remove the tile");
    let err = pyramid_verify_tree(&base, &plan).expect_err("a missing tile must not verify");
    let text = err.to_string();
    assert!(
        text.contains("missing tile") && text.contains(&format!("{victim:?}")),
        "the refusal must name the missing tile {victim:?}: {text}"
    );
}

#[test]
fn pyramid_verify_refuses_a_tree_holding_a_tile_the_plan_does_not_name() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, base) = clean_png_tree(dir.path());
    let top = plan.levels.last().expect("a level");
    // One column past the edge of the top level: every planned coordinate
    // still answers, so only the count can see it.
    let extra = TileCoord::new(top.level, top.cols, 0);
    // The plan has no path for it, so it is spelled the way DeepZoom spells
    // every other tile, `{level}/{col}_{row}.png`.
    let near = tile_file(
        &base,
        &plan,
        TileCoord::new(top.level, top.cols - 1, 0),
        "png",
    );
    let planted = base.join(format!("{}/{}_{}.png", extra.level, extra.col, extra.row));
    assert!(
        plan.tile_path(extra, "png").is_none(),
        "the planted tile is outside the plan"
    );
    std::fs::copy(&near, &planted).expect("plant the extra tile");
    let err = pyramid_verify_tree(&base, &plan).expect_err("an extra tile must not verify");
    let text = err.to_string();
    let planned = plan.total_tile_count();
    assert!(
        text.contains(&format!("addresses {} tiles", planned + 1))
            && text.contains(&planned.to_string()),
        "the refusal must be the count, {} against {planned}: {text}",
        planned + 1
    );
}

#[test]
fn pyramid_verify_refuses_a_tree_that_cannot_count_itself() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, base) = clean_png_tree(dir.path());
    let reader =
        DirectoryPyramidReader::try_open(&base, plan.clone(), TileFormat::Png).expect("open");
    let err = libviprs::verify::pyramid_verify(
        &reader,
        &plan,
        Some(TileFormat::Png),
        &libviprs::observe::NoopObserver,
    )
    .expect_err("a reader that cannot count must be refused, not waved through");
    assert!(
        matches!(
            err,
            EngineError::Sink(SinkError::PyramidRead(
                libviprs::PyramidReadError::NotCountable(_)
            ))
        ),
        "the refusal must be the reader's inability to count, got {err:?}"
    );
}

#[test]
fn pyramid_verify_refuses_a_tree_with_an_empty_tile() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, base) = clean_png_tree(dir.path());
    let victim = interior_top_tile(&plan);
    std::fs::write(tile_file(&base, &plan, victim, "png"), b"").expect("truncate the tile");
    let err = pyramid_verify_tree(&base, &plan).expect_err("an empty tile must not verify");
    let text = err.to_string();
    assert!(
        text.contains("no bytes") && text.contains(&format!("{victim:?}")),
        "the refusal must name the empty tile {victim:?}: {text}"
    );
}

#[test]
fn pyramid_verify_refuses_an_archive_from_a_different_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (_, archive) = clean_archive(dir.path());
    // Same levels, same grid, same coordinates: only the source size differs,
    // which no sweep and no count can see.
    let foreign = xyz_plan(1000, 256);
    let err = pyramid_verify_archive(&archive, &foreign)
        .expect_err("an archive generated from another source must not verify");
    let text = err.to_string();
    assert!(
        text.contains("1024x1024") && text.contains("1000x1000"),
        "the refusal must name both source sizes: {text}"
    );
}

#[test]
fn pyramid_verify_refuses_an_archive_whose_header_miscounts_its_tiles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (plan, archive) = clean_archive(dir.path());
    let mut bytes = std::fs::read(&archive).expect("read");
    let field = &mut bytes[ADDRESSED_TILES_OFFSET..ADDRESSED_TILES_OFFSET + 8];
    let claimed = u64::from_le_bytes(field.try_into().expect("8 bytes")) + 1;
    field.copy_from_slice(&claimed.to_le_bytes());
    std::fs::write(&archive, bytes).expect("write the damaged header");

    let err = pyramid_verify_archive(&archive, &plan)
        .expect_err("a header that miscounts its tiles must not verify");
    assert!(
        err.to_string().contains(&claimed.to_string()),
        "the refusal must be about the count the header claims ({claimed}): {err}"
    );
}

// ===========================================================================
// Observer events
// ===========================================================================

fn observed_run(
    kind: EngineKind,
    budget: Option<u64>,
) -> (
    PyramidPlan,
    Raster,
    Vec<EngineEvent>,
    libviprs::EngineResult,
) {
    let plan = deepzoom_plan(512, 384, 64);
    let src = canonical_raster_scaled(512, 384);
    let observer = Arc::new(CollectingObserver::new());
    let mut builder = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_engine(kind)
        .with_concurrency(2)
        .with_observer_arc(observer.clone());
    if let Some(bytes) = budget {
        builder = builder.with_memory_budget(bytes);
    }
    let result = builder.run().expect("an observed run");
    (plan, src.clone(), observer.events(), result)
}

fn assert_events_agree_with_the_plan(label: &str, plan: &PyramidPlan, events: &[EngineEvent]) {
    let mut started = BTreeMap::new();
    let mut completed_per_level: BTreeMap<u32, u64> = BTreeMap::new();
    let mut level_completed = BTreeMap::new();
    let mut coords = Vec::new();
    let mut finished = Vec::new();
    for event in events {
        match event {
            EngineEvent::LevelStarted {
                level, tile_count, ..
            } => {
                assert!(
                    started.insert(*level, *tile_count).is_none(),
                    "{label}: level {level} started twice"
                );
            }
            EngineEvent::TileCompleted { coord, .. } => {
                *completed_per_level.entry(coord.level).or_default() += 1;
                coords.push(*coord);
            }
            EngineEvent::LevelCompleted {
                level,
                tiles_produced,
            } => {
                level_completed.insert(*level, *tiles_produced);
            }
            EngineEvent::Finished {
                total_tiles,
                levels,
            } => finished.push((*total_tiles, *levels)),
            _ => {}
        }
    }
    for level in &plan.levels {
        assert_eq!(
            started.get(&level.level),
            Some(&level.tile_count()),
            "{label}: LevelStarted for level {} carries the plan's tile count",
            level.level
        );
        assert_eq!(
            completed_per_level.get(&level.level).copied().unwrap_or(0),
            level.tile_count(),
            "{label}: TileCompleted events for level {}",
            level.level
        );
        assert_eq!(
            level_completed.get(&level.level),
            Some(&level.tile_count()),
            "{label}: LevelCompleted for level {}",
            level.level
        );
    }
    let want: BTreeSet<_> = plan
        .tile_coords()
        .map(|c| (c.level, c.col, c.row))
        .collect();
    let got: BTreeSet<_> = coords.iter().map(|c| (c.level, c.col, c.row)).collect();
    assert_eq!(
        coords.len(),
        got.len(),
        "{label}: a tile was reported completed twice"
    );
    assert_eq!(
        got, want,
        "{label}: the completed coordinates are not the plan's"
    );
    assert_eq!(
        finished,
        vec![(plan.total_tile_count(), plan.levels.len() as u32)],
        "{label}: exactly one Finished carrying the plan's totals"
    );
    assert!(
        matches!(events.last(), Some(EngineEvent::Finished { .. })),
        "{label}: Finished is the last event"
    );
}

#[test]
fn observer_event_counts_agree_with_the_plan_on_every_engine() {
    for (label, kind, budget) in [
        ("monolithic", EngineKind::Monolithic, None),
        ("streaming", EngineKind::Streaming, Some(2_000_000)),
        ("mapreduce", EngineKind::MapReduce, Some(2_000_000)),
    ] {
        let (plan, _, events, result) = observed_run(kind, budget);
        assert_events_agree_with_the_plan(label, &plan, &events);
        assert_eq!(
            result.tiles_produced,
            plan.total_tile_count(),
            "{label}: tiles_produced"
        );
    }
}

#[test]
fn peak_memory_is_reported_and_inside_its_bound() {
    let (_, src, _, mono) = observed_run(EngineKind::Monolithic, None);
    let source_bytes = src.data().len() as u64;
    assert!(
        mono.peak_memory_bytes >= source_bytes,
        "the monolithic engine holds the whole source, so its peak ({}) cannot be below \
         the source's {source_bytes} bytes",
        mono.peak_memory_bytes
    );
    assert!(
        mono.peak_memory_bytes < source_bytes * 2,
        "the levels below the top sum to a third of it, so a peak of {} against a \
         {source_bytes}-byte source is counting something twice",
        mono.peak_memory_bytes
    );

    for (label, kind) in [
        ("streaming", EngineKind::Streaming),
        ("mapreduce", EngineKind::MapReduce),
    ] {
        let budget = 2_000_000;
        let (_, _, _, result) = observed_run(kind, Some(budget));
        assert!(
            result.peak_memory_bytes > 0,
            "{label}: a peak of zero is not a report"
        );
        assert!(
            result.peak_memory_bytes <= budget,
            "{label}: peak {} is over the {budget}-byte budget",
            result.peak_memory_bytes
        );
    }
}

// ===========================================================================
// streaming_mapreduce: the WorkExecutor seam
// ===========================================================================

/// Delegates to `LocalWorkExecutor`, records every unit it was handed, and can
/// corrupt the strip at one chosen canvas row.
struct RecordingExecutor {
    units: Mutex<Vec<StripWorkUnit>>,
    corrupt_row: Option<u32>,
}

impl WorkExecutor for RecordingExecutor {
    fn execute(&self, spec: StripWorkUnit, ctx: WorkContext<'_>) -> Result<Raster, EngineError> {
        self.units.lock().expect("units").push(spec);
        let strip = LocalWorkExecutor.execute(spec, ctx)?;
        match self.corrupt_row {
            Some(row) if (spec.canvas_y..spec.canvas_y + spec.height).contains(&row) => {
                let mut data = strip.data().to_vec();
                let stride = strip.width() as usize * strip.format().bytes_per_pixel();
                let start = (row - spec.canvas_y) as usize * stride;
                for b in &mut data[start..start + stride] {
                    *b = !*b;
                }
                Ok(
                    Raster::new(strip.width(), strip.height(), strip.format(), data)
                        .expect("strip"),
                )
            }
            _ => Ok(strip),
        }
    }

    fn worker_id(&self) -> Option<libviprs::WorkerId> {
        Some(libviprs::WorkerId::new("pipeline-e2e"))
    }
}

fn mapreduce_through(
    src: &Raster,
    plan: &PyramidPlan,
    executor: Arc<RecordingExecutor>,
    observer: Arc<CollectingObserver>,
) -> (Tiles, libviprs::EngineResult) {
    let sink = MemorySink::new();
    let result = EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::MapReduce)
        .with_memory_budget(1_500_000)
        .with_executor(executor)
        .with_observer_arc(observer)
        .run()
        .expect("a MapReduce run");
    (memory_tiles(&sink), result)
}

#[test]
fn mapreduce_executor_seam_is_byte_identical_to_monolithic() {
    let plan = deepzoom_plan(512, 384, 64);
    let src = canonical_raster_scaled(512, 384);
    let executor = Arc::new(RecordingExecutor {
        units: Mutex::new(Vec::new()),
        corrupt_row: None,
    });
    let observer = Arc::new(CollectingObserver::new());
    let (tiles, result) = mapreduce_through(&src, &plan, executor.clone(), observer.clone());

    assert!(
        tiles == reference_tiles(&src, &plan),
        "a MapReduce run through the executor seam must produce the monolithic output"
    );
    assert!(
        result.peak_memory_bytes <= 1_500_000,
        "peak {} over budget",
        result.peak_memory_bytes
    );

    let mut units = executor.units.lock().expect("units").clone();
    assert!(
        units.len() > 1,
        "the budget must split the canvas into several strips"
    );
    units.sort_by_key(|u| u.canvas_y);
    let top = plan.levels.last().expect("a level").level;
    let mut next = 0;
    for unit in &units {
        assert_eq!(
            unit.canvas_y, next,
            "strips must tile the canvas with no gap or overlap"
        );
        assert_eq!(unit.level, top, "every strip is rendered at the top level");
        next += unit.height;
    }
    assert_eq!(
        next, plan.canvas_height,
        "the strips must cover the whole canvas"
    );

    let events = observer.events();
    let dispatched = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::StripDispatched { worker_id: Some(w), .. } if w.as_str() == "pipeline-e2e"))
        .count();
    let done = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::StripExecutorDone { worker_id: Some(w), .. } if w.as_str() == "pipeline-e2e"))
        .count();
    assert_eq!(
        dispatched,
        units.len(),
        "one attributed StripDispatched per executed unit"
    );
    assert_eq!(
        done,
        units.len(),
        "one attributed StripExecutorDone per executed unit"
    );
}

/// The negative control for the cell above: what the executor returns is what
/// lands in the sink. Without it the parity above also passes for an engine
/// that called the executor and then rendered the strip itself.
#[test]
fn mapreduce_executor_output_is_what_lands_in_the_sink() {
    let plan = deepzoom_plan(512, 384, 64);
    let src = canonical_raster_scaled(512, 384);
    let row = 200;
    let executor = Arc::new(RecordingExecutor {
        units: Mutex::new(Vec::new()),
        corrupt_row: Some(row),
    });
    let (tiles, _) = mapreduce_through(&src, &plan, executor, Arc::new(CollectingObserver::new()));
    let reference = reference_tiles(&src, &plan);
    let top = plan.levels.last().expect("a level");
    let changed: BTreeSet<(u32, u32, u32)> = reference
        .iter()
        .filter(|(k, v)| tiles.get(*k) != Some(*v))
        .map(|(k, _)| *k)
        .collect();
    let top_row = row / plan.tile_size;
    for col in 0..top.cols {
        assert!(
            changed.contains(&(top.level, col, top_row)),
            "inverting canvas row {row} in the executor must change top-level tile ({col}, {top_row})"
        );
    }
    assert!(
        changed
            .iter()
            .all(|(level, _, r)| *level != top.level || *r == top_row),
        "only the top-level tiles containing row {row} may change, got {changed:?}"
    );
}

// ===========================================================================
// storage::output_path
// ===========================================================================

#[test]
fn storage_output_path_is_where_each_sink_writes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = xyz_plan(512, 256);
    let src = canonical_raster_scaled(512, 512);

    for (base, want) in [
        ("city", "city.pmtiles"),
        ("tiles.v2", "tiles.v2.pmtiles"),
        ("upper.PMTILES", "upper.PMTILES"),
    ] {
        let base = dir.path().join(base);
        let out = PyramidStorage::default().output_path(&base);
        assert_eq!(
            out,
            dir.path().join(want),
            "where the archive for {} lands",
            base.display()
        );
        run_archive(&src, &plan, &out, WriterOptions::default(), false, 1);
        let reader =
            PmTilesPyramidReader::try_open(&out).expect("the archive is where output_path said");
        libviprs::verify::pyramid_verify(
            &reader,
            &plan,
            Some(TileFormat::Png),
            &libviprs::observe::NoopObserver,
        )
        .expect("and it is the whole pyramid");
        if want
            != base
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
        {
            assert!(
                !base.exists(),
                "nothing may be written at the bare base {}",
                base.display()
            );
        }
    }

    let base = dir.path().join("tree");
    let out = PyramidStorage::Directory.output_path(&base);
    assert_eq!(out, base, "a directory is the base unchanged");
    run_tree(&src, &plan, &out, TileFormat::Png);
    assert!(out.is_dir(), "the tree is at the base");
    assert!(
        !dir.path().join("tree.pmtiles").exists(),
        "and no archive appeared beside it"
    );
    pyramid_verify_tree(&out, &plan).expect("the tree is the whole pyramid");
}

#[test]
fn storage_accepts_layout_agrees_with_what_the_pmtiles_sink_builds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let layouts = [
        Layout::Xyz,
        Layout::Google,
        Layout::DeepZoom,
        Layout::Zoomify,
        Layout::Iiif,
    ];
    let mut accepted = 0;
    for (i, layout) in layouts.into_iter().enumerate() {
        let plan = PyramidPlanner::new(512, 512, 256, 0, layout)
            .expect("plan")
            .plan();
        let path = PyramidStorage::PmTiles.output_path(dir.path().join(format!("l{i}")));
        let built = PmTilesSink::builder(&path).plan(plan).build();
        let says = PyramidStorage::PmTiles.accepts_layout(layout);
        assert_eq!(
            built.is_ok(),
            says,
            "PyramidStorage says {layout:?} is {} and the sink {} it",
            if says { "accepted" } else { "refused" },
            if built.is_ok() { "built" } else { "refused" }
        );
        accepted += usize::from(says);
        assert!(
            PyramidStorage::Directory.accepts_layout(layout),
            "a directory takes {layout:?}"
        );
    }
    assert_eq!(accepted, 2, "the two ZXY layouts and only those");
}

// ===========================================================================
// geo
// ===========================================================================

/// A rotated, sheared geo-reference solved from three ground control points,
/// in degrees, over a 512x512 source. Nothing about it is axis aligned, so a
/// transposed coefficient or a swapped axis moves every answer below.
fn georeference() -> GeoTransform {
    GeoTransform::from_gcps_exact(&[
        (PixelCoord::new(0.0, 0.0), GeoCoord::new(13.3777, 52.5163)),
        (PixelCoord::new(512.0, 0.0), GeoCoord::new(13.3912, 52.5181)),
        (PixelCoord::new(0.0, 512.0), GeoCoord::new(13.3801, 52.5089)),
    ])
    .expect("three non-collinear control points")
}

#[test]
fn geo_bounds_written_from_a_georeference_read_back_from_the_archive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = xyz_plan(512, 256);
    let src = canonical_raster_scaled(512, 512);
    let geo = georeference();
    let bounds = geo.image_bounds(512, 512);
    let centre = bounds.center();
    let archive = dir.path().join("geo.pmtiles");
    run_archive(
        &src,
        &plan,
        &archive,
        WriterOptions::default()
            .with_bounds_degrees([bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y])
            .with_center_degrees(centre.x, centre.y),
        false,
        1,
    );

    let reader = Reader::try_open(&archive).expect("open");
    let (w, s, e, n) = reader.bounds();
    for (label, got, want) in [
        ("west", w, bounds.min.x),
        ("south", s, bounds.min.y),
        ("east", e, bounds.max.x),
        ("north", n, bounds.max.y),
    ] {
        assert!(
            (got - want).abs() < 1e-6,
            "the archive's {label} bound is {got} and the georeference put it at {want}"
        );
    }
    for corner in [(0.0, 0.0), (512.0, 0.0), (512.0, 512.0), (0.0, 512.0)] {
        let g = geo.pixel_to_geo(PixelCoord::new(corner.0, corner.1));
        assert!(
            g.x >= w - 1e-6 && g.x <= e + 1e-6 && g.y >= s - 1e-6 && g.y <= n + 1e-6,
            "corner {corner:?} maps to {g:?}, outside the archive's bounds"
        );
    }

    if let Some(bin) = go_pmtiles_bin() {
        let shown = go_pmtiles_header(&bin, &archive);
        let theirs: Vec<f64> = shown
            .get("bounds")
            .and_then(serde_json::Value::as_array)
            .expect("bounds")
            .iter()
            .map(|v| v.as_f64().expect("a number"))
            .collect();
        let ours = [bounds.min.x, bounds.min.y, bounds.max.x, bounds.max.y];
        for (t, o) in theirs.iter().zip(ours) {
            assert!(
                (t - o).abs() < 1e-6,
                "go-pmtiles reads bounds {theirs:?}, written {ours:?}"
            );
        }
    }
}

#[test]
fn geo_lookup_finds_the_source_pixel_through_the_pyramid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let plan = xyz_plan(512, 256);
    let src = canonical_raster_scaled(512, 512);
    let archive = dir.path().join("geo.pmtiles");
    run_archive(&src, &plan, &archive, WriterOptions::default(), false, 1);
    let reader = PmTilesPyramidReader::try_open(&archive).expect("open");
    let geo = georeference();
    let top = plan.levels.last().expect("a level");

    // Points either side of every tile seam, plus the interior.
    let probes = [
        (3.5, 7.5),
        (255.5, 100.5),
        (256.5, 100.5),
        (100.5, 255.5),
        (300.5, 256.5),
        (508.5, 509.5),
    ];
    for (px, py) in probes {
        let g = geo.pixel_to_geo(PixelCoord::new(px, py));
        let back = geo.geo_to_pixel(g).expect("invertible");
        assert!(
            (back.x - px).abs() < 1e-6 && (back.y - py).abs() < 1e-6,
            "({px}, {py}) went to {g:?} and came back as {back:?}"
        );
        let (x, y) = (back.x.floor() as u32, back.y.floor() as u32);
        let coord = TileCoord::new(top.level, x / plan.tile_size, y / plan.tile_size);
        let rect = plan.tile_rect(coord).expect("a tile");
        let bytes = libviprs::PyramidReader::tile(&reader, coord)
            .expect("read")
            .unwrap_or_else(|| panic!("no tile at {coord:?}"));
        let tile = image::load_from_memory(&bytes).expect("a PNG").to_rgb8();
        let got = tile.get_pixel(x - rect.x, y - rect.y).0;
        let off = (y as usize * 512 + x as usize) * 3;
        let want = &src.data()[off..off + 3];
        assert_eq!(
            &got[..],
            want,
            "the geo point {g:?} is source pixel ({x}, {y}); tile {coord:?} holds {got:?} there"
        );

        let c = geo.tile_center(coord.col, coord.row, plan.tile_size);
        let p = geo.geo_to_pixel(c).expect("invertible");
        assert!(
            (p.x - (f64::from(rect.x) + 128.0)).abs() < 1e-6
                && (p.y - (f64::from(rect.y) + 128.0)).abs() < 1e-6,
            "the geo centre of {coord:?} maps back to {p:?}, not the tile's centre pixel"
        );
        assert!(
            geo.image_bounds(512, 512).contains(c),
            "{coord:?}'s centre lies inside the image bounds"
        );
    }
}
