//! Migrating a Google-layout tile tree into a PMTiles archive (libviprs-tests#262).
//!
//! iasbuilt/server keeps its sheet tiles as `{z}/{row}/{col}.png` trees, which
//! is what `Layout::Google` writes, and moves them into one archive with
//! `migrate_directory_to_pmtiles`. The archive is then read back by
//! `(z, x, y)` over HTTP. The step that went wrong there was the coordinate
//! mapping on that migration: a tile that sat at `{z}/{row}/{col}` on disk has
//! to come out at `(z, x = col, y = row)`, and a transposition is invisible to
//! any check that goes through the library's own mapping on both sides.
//!
//! `phase_pmtiles.rs` pins the same thing for an Xyz plan generated straight
//! into a `PmTilesSink`. Nothing there, or anywhere else in this repo, runs
//! the migration, and nothing combines Google layout with an archive. These
//! cells do.
//!
//! # How the expectations stay independent
//!
//! The on-disk path is spelled out here as `{z}/{row}/{col}` rather than asked
//! of `PyramidPlan::tile_path`, and the archive is read through
//! `pmtiles::Reader::get_tile` with a raw `(z, x, y)` rather than through
//! `PmTilesPyramidReader`. Both of those would otherwise go through the same
//! `TileCoord` mapping the migration wrote with, and an error in it would
//! cancel.
//!
//! # Red proof
//!
//! These cells are green on a correct library, because the library is
//! correct. The red proof is a mutation: swapping `col` and `row` in
//! `sink_pmtiles::tile_coord_to_zxy` has to turn the migrate cells red. The PR
//! records that run.

use std::path::{Path, PathBuf};

#[path = "common/pmtiles.rs"]
mod pmtiles_support;
use pmtiles_support::read_checked;

use libviprs::planner::TileCoord;
use libviprs::pmtiles::Reader;
use libviprs::{
    DirectoryPyramidReader, EngineBuilder, EngineConfig, EngineKind, FsSink, Layout,
    MigrateOptions, PmTilesPyramidReader, PyramidPlan, PyramidPlanner, PyramidReader, Raster,
    TileFormat, extract_page_image, migrate_directory_to_pmtiles,
};

/// The input: page 1 of the portrait blueprint, 3300x5024, which is the sheet
/// shape the server renders and is not square.
const PORTRAIT_PDF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint-portrait.pdf"
);

/// A second, different sheet, for the regenerate cell.
const BLUEPRINT_PDF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");

/// The stored output the migration is compared against, under
/// `tests/fixtures/pmtiles/`, with its provenance in `PROVENANCE.md` beside it.
const GOLDEN: &str = "migrate-google-blueprint-portrait.pmtiles";
const GOLDEN_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn portrait_raster() -> Raster {
    extract_page_image(Path::new(PORTRAIT_PDF), 1)
        .expect("extract page 1 of blueprint-portrait.pdf")
}

fn blueprint_raster() -> Raster {
    extract_page_image(Path::new(BLUEPRINT_PDF), 1).expect("extract page 1 of blueprint.pdf")
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A Google plan, optionally centred, which is how the server plans a sheet.
fn google_plan(w: u32, h: u32, tile: u32, centre: bool) -> PyramidPlan {
    PyramidPlanner::new(w, h, tile, 0, Layout::Google)
        .expect("a valid Google plan")
        .with_centre(centre)
        .plan()
}

/// Write `src` into a loose-file Google tree under `base`.
fn generate_tree(src: &Raster, plan: &PyramidPlan, base: &Path) {
    let sink = FsSink::new(base, plan.clone()).with_format(TileFormat::Png);
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .expect("generate the Google tree");
}

/// Where `Layout::Google` puts a tile: row before column.
///
/// Spelled out on purpose, see the module comment.
fn google_file(base: &Path, z: u32, row: u32, col: u32) -> PathBuf {
    base.join(format!("{z}/{row}/{col}.png"))
}

/// Migrate `base` into `archive` the way the server does.
fn migrate(base: &Path, plan: &PyramidPlan, archive: &Path) -> libviprs::MigrateReport {
    let reader = DirectoryPyramidReader::try_open(base, plan.clone(), TileFormat::Png)
        .expect("open the tree we just wrote");
    migrate_directory_to_pmtiles(&reader, archive, MigrateOptions::default())
        .expect("migrate the tree into an archive")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Every tile in the tree is in the archive at `(z, x = col, y = row)`, with
/// the bytes the file had, and nothing is in the archive that the tree did not
/// have.
///
/// Not square, and centred and not: on a square level a transposed grid is
/// still a bijection onto itself, so half the coordinates land on a tile that
/// happens to exist and the comparison weakens. Centring moves which cells are
/// blank, which is what the server's real sheets look like.
#[test]
fn a_migrated_google_tree_puts_every_tile_at_zxy_with_its_own_bytes() {
    for centre in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let src = portrait_raster();
        let plan = google_plan(src.width(), src.height(), 256, centre);
        let base = dir.path().join("tiles");
        let archive = dir.path().join("out.pmtiles");
        generate_tree(&src, &plan, &base);
        migrate(&base, &plan, &archive);

        // The control for the control: if every coordinate had col == row,
        // swapping them would be the identity and this cell could not fail.
        let off_diagonal = plan.tile_coords().filter(|c| c.col != c.row).count();
        assert!(
            off_diagonal > 0,
            "centre={centre}: every tile sits on the diagonal, so a transposed migration \
             would pass this cell"
        );

        let reader = Reader::try_open(&archive).expect("open the archive");
        let mut compared = 0u64;
        let mut present = 0u64;
        for TileCoord { level, col, row } in plan.tile_coords() {
            let on_disk = std::fs::read(google_file(&base, level, row, col)).ok();
            let in_archive = reader
                .get_tile(level as u8, col, row)
                .expect("read the archive at (z, x, y)");
            assert_eq!(
                in_archive,
                on_disk,
                "centre={centre}: z={level} x(col)={col} y(row)={row}: the archive answers with \
                 {} bytes and the tree's {{z}}/{{row}}/{{col}} file has {}",
                in_archive.as_ref().map_or(0, Vec::len),
                on_disk.as_ref().map_or(0, Vec::len),
            );
            compared += 1;
            present += u64::from(on_disk.is_some());
        }
        assert_eq!(compared, plan.total_tile_count());
        assert!(
            present > 0,
            "centre={centre}: the tree has no tiles, so nothing was compared"
        );
    }
}

/// The report's counts say what happened: every coordinate visited, and every
/// one of them either written or absent, matching the files on disk.
#[test]
fn the_migrate_report_counts_agree_with_the_plan_and_the_tree() {
    for centre in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let src = portrait_raster();
        let plan = google_plan(src.width(), src.height(), 256, centre);
        let base = dir.path().join("tiles");
        let archive = dir.path().join("out.pmtiles");
        generate_tree(&src, &plan, &base);
        let report = migrate(&base, &plan, &archive);

        let files_on_disk = plan
            .tile_coords()
            .filter(|c| google_file(&base, c.level, c.row, c.col).is_file())
            .count() as u64;

        assert_eq!(
            report.coords_visited,
            plan.total_tile_count(),
            "centre={centre}: the migration did not visit every coordinate in the plan"
        );
        assert_eq!(
            report.tiles_written, files_on_disk,
            "centre={centre}: tiles written differs from the files in the tree"
        );
        assert_eq!(
            report.tiles_written + report.tiles_absent,
            report.coords_visited,
            "centre={centre}: a coordinate was neither written nor reported absent"
        );
        assert_eq!(report.tile_format, TileFormat::Png);
        assert_eq!(report.out_path, archive);
    }
}

/// `PmTilesPyramidReader`, which the server serves through, answers the same
/// bytes as the directory reader it was migrated from, coordinate by
/// coordinate, so the two ways of asking agree as well as the raw one.
#[test]
fn the_server_reader_answers_what_the_tree_reader_answered() {
    let dir = tempfile::tempdir().unwrap();
    let src = portrait_raster();
    let plan = google_plan(src.width(), src.height(), 256, true);
    let base = dir.path().join("tiles");
    let archive = dir.path().join("out.pmtiles");
    generate_tree(&src, &plan, &base);
    migrate(&base, &plan, &archive);

    let tree = DirectoryPyramidReader::try_open(&base, plan.clone(), TileFormat::Png)
        .expect("open the tree");
    let served = PmTilesPyramidReader::try_open(&archive).expect("open the archive");
    for coord in plan.tile_coords() {
        assert_eq!(
            served.tile(coord).expect("read through the archive reader"),
            tree.tile(coord).expect("read through the tree reader"),
            "{coord:?}: the archive reader and the tree reader disagree"
        );
    }
}

/// Deleting an archive and migrating again replaces it, and a reader opened
/// afterwards sees the new bytes rather than the old ones.
///
/// This is the update path the server takes when a sheet is re-rendered: there
/// is no in-place rewrite (libviprs#1129 is open), so the archive goes away and
/// a new one is written beside the same tree. A reader that was already open
/// holds the old file, which is the server's cached-reader problem and the
/// reason it invalidates; a reader opened after the swap must not.
#[test]
fn a_deleted_archive_is_replaced_and_a_fresh_reader_sees_the_new_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("out.pmtiles");

    // Both renders first, so the probe can be a tile that really differs
    // between them rather than a blank one both sheets share.
    let src1 = portrait_raster();
    let plan1 = google_plan(src1.width(), src1.height(), 256, true);
    let base1 = dir.path().join("tiles-1");
    generate_tree(&src1, &plan1, &base1);

    let src2 = blueprint_raster();
    let plan2 = google_plan(src2.width(), src2.height(), 256, true);
    let base2 = dir.path().join("tiles-2");
    generate_tree(&src2, &plan2, &base2);

    let probe = plan1
        .tile_coords()
        .find(|c| {
            let a = std::fs::read(google_file(&base1, c.level, c.row, c.col)).ok();
            let b = std::fs::read(google_file(&base2, c.level, c.row, c.col)).ok();
            a.is_some() && b.is_some() && a != b
        })
        .expect("a coordinate both renders have and that differs between them");

    // First archive.
    migrate(&base1, &plan1, &archive);
    let before = Reader::try_open(&archive).expect("open the first archive");
    let first_bytes = before
        .get_tile(probe.level as u8, probe.col, probe.row)
        .unwrap()
        .expect("the probe tile is in the first archive");
    drop(before);

    // Delete, as the server does, and the archive is gone rather than empty.
    std::fs::remove_file(&archive).expect("delete the archive");
    assert!(
        Reader::try_open(&archive).is_err(),
        "a deleted archive still opens"
    );
    assert!(!archive.exists());

    // Second archive, from the other sheet, beside the same path.
    migrate(&base2, &plan2, &archive);

    let after = Reader::try_open(&archive).expect("open the regenerated archive");
    for TileCoord { level, col, row } in plan2.tile_coords() {
        let on_disk = std::fs::read(google_file(&base2, level, row, col)).ok();
        let in_archive = after.get_tile(level as u8, col, row).unwrap();
        assert_eq!(
            in_archive, on_disk,
            "z={level} x(col)={col} y(row)={row}: the regenerated archive does not match the \
             second tree"
        );
    }
    let probed = after
        .get_tile(probe.level as u8, probe.col, probe.row)
        .unwrap()
        .expect("the probe tile is in the regenerated archive");
    assert_ne!(
        probed, first_bytes,
        "the regenerated archive answers the probe tile with the first render's bytes, which \
         is a stale archive"
    );
}

/// The migrated archive agrees, tile for tile, with the one stored beside the
/// other PMTiles fixtures, so a change in what the migration produces for the
/// server's sheet shape shows up as a diff against a file rather than only
/// against a second run of the same code.
///
/// The stored archive is digest-checked before it is read, for the reason
/// `common/pmtiles.rs` gives: "fixing" a red cell by regenerating the golden
/// has to be visible. `PROVENANCE.md` says how it was made. The tree-to-archive
/// cell above is what keeps the golden itself honest, since it compares the
/// archive with the `{z}/{row}/{col}` files directly and does not go through
/// the golden.
#[test]
fn the_migrated_archive_matches_the_stored_golden_tile_for_tile() {
    let golden_bytes = read_checked(GOLDEN, GOLDEN_SHA256);
    let dir = tempfile::tempdir().unwrap();
    let golden_path = dir.path().join("golden.pmtiles");
    std::fs::write(&golden_path, golden_bytes).unwrap();

    let src = portrait_raster();
    let plan = google_plan(src.width(), src.height(), 256, true);
    let base = dir.path().join("tiles");
    let archive = dir.path().join("out.pmtiles");
    generate_tree(&src, &plan, &base);
    migrate(&base, &plan, &archive);

    let want = Reader::try_open(&golden_path).expect("open the golden");
    let got = Reader::try_open(&archive).expect("open the migrated archive");
    for TileCoord { level, col, row } in plan.tile_coords() {
        assert_eq!(
            got.get_tile(level as u8, col, row).unwrap(),
            want.get_tile(level as u8, col, row).unwrap(),
            "z={level} x(col)={col} y(row)={row}: the migrated archive differs from the golden"
        );
    }
}

/// Writes the golden. Not part of a normal run: it is how the file the cell
/// above compares against gets made, and the digest it prints is the one to
/// pin in `GOLDEN_SHA256` and `PROVENANCE.md`.
///
/// ```text
/// cargo test --test pmtiles_migrate_google -- --ignored write_the_migrate_golden --nocapture
/// ```
#[test]
#[ignore = "writes tests/fixtures/pmtiles/migrate-google-blueprint-portrait.pmtiles"]
fn write_the_migrate_golden() {
    let dir = tempfile::tempdir().unwrap();
    let src = portrait_raster();
    let plan = google_plan(src.width(), src.height(), 256, true);
    let base = dir.path().join("tiles");
    let archive = dir.path().join("out.pmtiles");
    generate_tree(&src, &plan, &base);
    migrate(&base, &plan, &archive);
    let out = pmtiles_support::fixtures_dir().join(GOLDEN);
    std::fs::copy(&archive, &out).expect("write the golden");
    let bytes = std::fs::read(&out).unwrap();
    println!(
        "wrote {} ({} bytes), sha256 {}",
        out.display(),
        bytes.len(),
        pmtiles_support::sha256_hex(&bytes)
    );
}
