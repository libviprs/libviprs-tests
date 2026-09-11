//! Cross-backend equivalence: a pyramid written to a directory and the same
//! pyramid written to a PMTiles archive have to be the same pyramid
//! (libviprs-tests#202, EPIC F libviprs/libviprs#986).
//!
//! This is the headline claim of the whole epic. PMTiles is being made the
//! default storage, and the promise attached to that is that nothing about the
//! output changes except where the bytes live. So the test generates one plan
//! into [`FsSink`] and into `PmTilesSink`, reads both back through the
//! storage-agnostic `PyramidReader` trait, and compares **decoded pixels** at
//! tolerance 0.
//!
//! # The way this test goes wrong
//!
//! Written naively it passes while proving nothing. If the archive reader
//! answers `None` for every coordinate, and the directory walk finds no files,
//! then "the two agree" is true of two empty sets and the loop body never runs.
//! That is the empty-result trap, and it is the single most likely way for this
//! file to be green and worthless.
//!
//! So the count is asserted before anything is compared, `pmtiles_holds_every_
//! tile_the_plan_declares` states the same control as a test of its own so it
//! cannot be deleted along with a refactor of the comparison, and
//! [`materialise`](pmtiles_support::materialise) leaves holes out of its result
//! rather than filling them with empty byte vectors, so a backend that holds
//! nothing produces a short list rather than a list of nothings.
//!
//! # Pixels, never bytes
//!
//! `CLI_CONTRACT.md` and `tests/common/dzsave_expected.rs` say it and this file
//! follows it: compare decoded pixels, never compressed bytes. Here the two
//! sides happen to share an encoder, so a byte comparison would pass too, and
//! that is exactly why it is the wrong test to write. It would start lying the
//! first time either side gained an encoder setting, and it would lie by
//! passing.

use std::path::Path;

mod common;
use common::dzsave_expected::{assert_tiles_pixel_equal_tol, collect_files};
use common::fixtures::canonical_raster_scaled;

#[path = "common/pmtiles.rs"]
mod pmtiles_support;
use pmtiles_support::{assert_decoded_tiles_equal, materialise};

use libviprs::planner::TileCoord;
use libviprs::pmtiles::Reader;
use libviprs::sink_pmtiles::PmTilesSink;
use libviprs::{
    DirectoryPyramidReader, EngineBuilder, EngineConfig, EngineKind, FsSink, Layout,
    PmTilesPyramidReader, PyramidPlan, PyramidPlanner, PyramidReader, Raster, TileFormat,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A ZXY plan, which is the only convention PMTiles addresses.
fn xyz_plan(w: u32, h: u32, tile: u32) -> PyramidPlan {
    PyramidPlanner::new(w, h, tile, 0, Layout::Xyz)
        .expect("a valid ZXY plan")
        .plan()
}

/// Write `src` into a loose-file tree.
///
/// `with_format` is not optional here even though `TileFormat::Png` is the
/// default: the whole point of this file is comparing two sinks over one
/// encoding, and a default that silently disagrees with the archive's encoding
/// makes the comparison a comparison of two different pyramids.
fn generate_into_fs(src: &Raster, plan: &PyramidPlan, base: &Path, format: TileFormat) {
    let sink = FsSink::new(base, plan.clone()).with_format(format);
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .expect("generate into FsSink");
}

/// Write `src` into one PMTiles archive at `path`.
fn generate_into_pmtiles(src: &Raster, plan: &PyramidPlan, path: &Path, format: TileFormat) {
    let sink = PmTilesSink::builder(path)
        .plan(plan.clone())
        .tile_format(format)
        .build()
        .expect("build a PmTilesSink");
    EngineBuilder::new(src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .expect("generate into PmTilesSink");
    assert!(
        sink.published_header().is_some(),
        "the run finished but the sink never published a header, so nothing was \
         finalised and {} is not an archive",
        path.display()
    );
}

/// Both backends for one plan, as `(relative path, encoded tile)` lists sorted
/// the same way.
fn both_backends(
    src: &Raster,
    plan: &PyramidPlan,
    format: TileFormat,
    dir: &Path,
) -> (Vec<(String, Vec<u8>)>, Vec<(String, Vec<u8>)>) {
    let base = dir.join("tiles");
    let archive = dir.join("out.pmtiles");
    generate_into_fs(src, plan, &base, format);
    generate_into_pmtiles(src, plan, &archive, format);

    let ext = format.extension();
    let fs_tiles = collect_files(&base, ext);
    let reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive we just wrote");
    let pm_tiles = materialise(&reader, plan, ext);
    (fs_tiles, pm_tiles)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The claim the epic rests on: the same plan into either backend gives the
/// same pyramid, tile for tile, pixel for pixel.
#[test]
fn fs_and_pmtiles_agree_tile_for_tile() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(512, 384);
    let plan = xyz_plan(512, 384, 128);

    let (fs_tiles, pm_tiles) = both_backends(&src, &plan, TileFormat::Png, dir.path());

    // The positive control, before anything is compared. Two empty lists are
    // equal, and a comparison of them asserts nothing at all.
    let want = plan.total_tile_count();
    assert!(
        want > 0,
        "the plan declares no tiles, so there is nothing here"
    );
    assert_eq!(
        fs_tiles.len() as u64,
        want,
        "the directory backend wrote {} of the {want} tiles the plan declares, \
         so the comparison below is not about the whole pyramid",
        fs_tiles.len()
    );
    assert_eq!(
        pm_tiles.len() as u64,
        want,
        "the archive answered for {} of the {want} tiles the plan declares. A \
         reader that answers None everywhere makes this test an empty loop that \
         passes.",
        pm_tiles.len()
    );

    // Tolerance 0: both sides ran the same encoder over the same pixels, so
    // anything at all here is a real difference, not a rounding one.
    assert_tiles_pixel_equal_tol(&fs_tiles, &pm_tiles, "fs:// vs pmtiles://", 0);
}

/// The control from the test above, stated on its own so a refactor of the
/// comparison cannot take it with it.
///
/// It also asserts against the archive's own header rather than only against
/// our walk: `addressed_tiles_count` is what the writer recorded, and a walk
/// that agrees with a count the same code produced is one source of truth, not
/// two.
#[test]
fn pmtiles_holds_every_tile_the_plan_declares() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(384, 256);
    let plan = xyz_plan(384, 256, 128);
    let archive = dir.path().join("out.pmtiles");
    generate_into_pmtiles(&src, &plan, &archive, TileFormat::Png);

    let reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive");
    let want = plan.total_tile_count();
    assert!(want > 0, "the plan declares no tiles");

    let mut present = 0u64;
    let mut empty = Vec::new();
    for coord in plan.tile_coords() {
        match reader.tile(coord).expect("read a tile back") {
            Some(bytes) => {
                if bytes.is_empty() {
                    empty.push(coord);
                }
                present += 1;
            }
            None => panic!(
                "the archive has no tile at {coord:?}, which the plan declares. \
                 A hole here is what makes a cross-backend comparison vacuous."
            ),
        }
    }
    assert_eq!(present, want);
    assert!(
        empty.is_empty(),
        "the archive answered Some with zero bytes for {} coordinate(s), \
         starting {:?}. An empty payload is a hole wearing a hit's clothes.",
        empty.len(),
        empty.first()
    );

    assert_eq!(
        reader.reader().header().addressed_tiles_count,
        want,
        "the archive's own header disagrees with the plan about how many tiles \
         it addresses"
    );
}

/// The negative control for the two above.
///
/// Every assertion so far is that a tile is there. That shape passes for a
/// reader that says yes to everything, and a reader that says yes to everything
/// is worse than one that says no: it hands back a real, decodable tile from
/// the wrong coordinate. So ask both backends for somewhere the plan does not
/// go and require both to say no, without either of them calling it an error.
#[test]
fn a_coordinate_the_plan_does_not_cover_is_absent_from_both() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(384, 256);
    let plan = xyz_plan(384, 256, 128);
    let base = dir.path().join("tiles");
    let archive = dir.path().join("out.pmtiles");
    generate_into_fs(&src, &plan, &base, TileFormat::Png);
    generate_into_pmtiles(&src, &plan, &archive, TileFormat::Png);

    let fs_reader = DirectoryPyramidReader::try_open(&base, plan.clone(), TileFormat::Png)
        .expect("open the tree");
    let pm_reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive");

    let top = plan.levels.last().expect("a plan has levels");
    // One column past the right edge of the deepest level, and one level past
    // the deepest level. Both are inside what PMTiles can address and outside
    // what this pyramid holds, which is the distinction that matters.
    let beyond = [
        TileCoord {
            level: top.level,
            col: top.cols,
            row: 0,
        },
        TileCoord {
            level: top.level + 1,
            col: 0,
            row: 0,
        },
    ];
    for coord in beyond {
        assert_eq!(
            fs_reader
                .tile(coord)
                .expect("the tree answers rather than erroring"),
            None,
            "the directory backend produced a tile at {coord:?}, which the plan \
             does not cover"
        );
        assert_eq!(
            pm_reader
                .tile(coord)
                .expect("the archive answers rather than erroring"),
            None,
            "the archive produced a tile at {coord:?}, which the plan does not \
             cover. Returning the preceding entry's bytes for a hole is the \
             failure a round-trip test cannot see, because it only ever asks \
             for tiles it wrote."
        );
    }
}

/// The JPEG path, because the two backends agreeing about PNG says nothing
/// about a format whose tile type has to be mapped into the archive's header.
#[test]
fn fs_and_pmtiles_agree_on_jpeg_tiles_too() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(256, 256);
    let plan = xyz_plan(256, 256, 128);
    let format = TileFormat::Jpeg { quality: 85 };

    let (fs_tiles, pm_tiles) = both_backends(&src, &plan, format, dir.path());

    let want = plan.total_tile_count();
    assert_eq!(fs_tiles.len() as u64, want, "the tree is short of tiles");
    assert_eq!(pm_tiles.len() as u64, want, "the archive is short of tiles");
    // Not `assert_tiles_pixel_equal_tol`: that helper calls `decode_png`
    // unconditionally, so a JPEG tile reaches it as an InvalidSignature panic
    // rather than as a comparison. Same rule, decoded pixels and never bytes,
    // through a decoder that reads the encoding off the data.
    assert_decoded_tiles_equal(&fs_tiles, &pm_tiles, "fs:// vs pmtiles:// (jpeg)");

    let archive = dir.path().join("out.pmtiles");
    let reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive");
    assert_eq!(
        reader.tile_format(),
        Some(format),
        "the archive does not report back the encoding it was written with, so \
         a consumer cannot tell a JPEG archive from a PNG one"
    );
}

/// PMTiles addresses `(z, x, y)` and nothing else, so a plan laid out any other
/// way has to be refused at construction rather than written under a layout it
/// is not in.
///
/// This is the one place the default-storage flip can quietly corrupt a
/// pyramid: `--layout` defaults to deep-zoom in the CLI, PMTiles is ZXY only,
/// and a sink that accepted a deep-zoom plan would produce an archive whose
/// coordinates mean something different from what wrote them.
#[test]
fn a_non_zxy_plan_is_refused_rather_than_relabelled() {
    let dir = tempfile::tempdir().unwrap();
    let deep_zoom = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom)
        .expect("a valid deep-zoom plan")
        .plan();

    let built = PmTilesSink::builder(dir.path().join("out.pmtiles"))
        .plan(deep_zoom)
        .tile_format(TileFormat::Png)
        .build();
    assert!(
        built.is_err(),
        "PmTilesSink accepted a DeepZoom plan. PMTiles has one coordinate \
         convention and it is ZXY, so this archive would carry tile ids that \
         mean something other than what the planner meant."
    );

    // The positive control: the same builder with a ZXY plan has to succeed, or
    // the assertion above is satisfied by a builder that refuses everything.
    let zxy = xyz_plan(256, 256, 128);
    PmTilesSink::builder(dir.path().join("ok.pmtiles"))
        .plan(zxy)
        .tile_format(TileFormat::Png)
        .build()
        .expect("a ZXY plan is the one PMTiles addresses and must be accepted");
}

/// A pyramid that came back out of an archive has to describe itself the same
/// way the tree does.
///
/// Levels, tile size and layout are what a consumer switches on, and a storage
/// change that silently altered any of them would be a breaking change wearing
/// a storage change's clothes.
#[test]
fn both_backends_describe_the_same_pyramid() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(512, 384);
    let plan = xyz_plan(512, 384, 128);
    let base = dir.path().join("tiles");
    let archive = dir.path().join("out.pmtiles");
    generate_into_fs(&src, &plan, &base, TileFormat::Png);
    generate_into_pmtiles(&src, &plan, &archive, TileFormat::Png);

    let fs_reader = DirectoryPyramidReader::try_open(&base, plan.clone(), TileFormat::Png)
        .expect("open the tree");
    let pm_reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive");

    let from_tree = fs_reader.describe().expect("describe the tree");
    let from_archive = pm_reader.describe().expect("describe the archive");

    assert_eq!(
        (from_tree.min_level, from_tree.max_level),
        (from_archive.min_level, from_archive.max_level),
        "the two backends disagree about which levels the pyramid has"
    );
    assert_eq!(
        from_tree.tile_size, from_archive.tile_size,
        "the two backends disagree about the tile size"
    );
    assert_eq!(
        from_tree.layout, from_archive.layout,
        "the two backends disagree about the layout"
    );
    assert_eq!(
        from_tree.format, from_archive.format,
        "the two backends disagree about the tile encoding"
    );

    // And the description has to be about this plan rather than about defaults
    // that happen to match on both sides.
    assert_eq!(from_archive.tile_size, Some(plan.tile_size));
    assert_eq!(from_archive.layout, Some(Layout::Xyz));
}

/// The archive stores a tile at the `(z, x, y)` the planner's coordinate names,
/// and not merely somewhere our own reader agrees with.
///
/// # Why every other cell in this file misses this
///
/// `PmTilesSink` and `PmTilesPyramidReader` both map `TileCoord` through
/// `sink_pmtiles::tile_coord_to_zxy`. That is a good thing, it is what stops the
/// two drifting apart, and it is also exactly why a mistake inside that function
/// is invisible to a comparison that goes through it on both sides: the error
/// cancels.
///
/// I measured that rather than reasoning about it. Swapping `col` and `row` in
/// `tile_coord_to_zxy` reddens **nothing** in this suite: not the cross-backend
/// comparison, not the count control, not the interop leg (go-pmtiles and our
/// raw reader agree with each other about the swapped archive, because neither
/// of them has an opinion about which one the planner meant). Every level of a
/// `2**z` grid has room for the swapped coordinate, so nothing goes out of
/// range and nothing errors. The archive is simply transposed, quietly, and a
/// consumer fetching `z/x/y` over HTTP gets the wrong tile.
///
/// So this cell asks `pmtiles::Reader::get_tile` for the raw `(z, x, y)`
/// directly, which is the one path that does not go through the shared
/// function, and compares against the file the directory backend put at that
/// same `z/x/y`.
#[test]
fn the_archive_addresses_tiles_at_the_coordinates_the_planner_named() {
    let dir = tempfile::tempdir().unwrap();
    // Deliberately not square: on a square level a transposition of the whole
    // grid is still a bijection onto itself, so half the coordinates land on a
    // tile that happens to be there and the comparison gets weaker.
    let src = canonical_raster_scaled(512, 384);
    let plan = xyz_plan(512, 384, 128);
    let base = dir.path().join("tiles");
    let archive = dir.path().join("out.pmtiles");
    generate_into_fs(&src, &plan, &base, TileFormat::Png);
    generate_into_pmtiles(&src, &plan, &archive, TileFormat::Png);

    // The control for the control: if every coordinate had col == row, swapping
    // them would be the identity and this cell could not fail.
    let off_diagonal = plan.tile_coords().filter(|c| c.col != c.row).count();
    assert!(
        off_diagonal > 0,
        "every coordinate in this plan is on the diagonal, so a transposed \
         archive would be indistinguishable from a correct one"
    );

    let reader = Reader::try_open(&archive).expect("open the archive as a raw PMTiles reader");
    let mut compared = 0usize;
    for coord in plan.tile_coords() {
        let rel = plan
            .tile_path(coord, "png")
            .expect("the plan has a path for its own coordinate");
        let on_disk = std::fs::read(base.join(&rel))
            .unwrap_or_else(|e| panic!("the directory backend has no {rel}: {e}"));
        let z = u8::try_from(coord.level).expect("a test plan stays inside u8 zooms");
        let in_archive = reader
            .get_tile(z, coord.col, coord.row)
            .unwrap_or_else(|e| panic!("({z}, {}, {}): {e}", coord.col, coord.row))
            .unwrap_or_else(|| {
                panic!(
                    "the archive holds nothing at ({z}, {}, {}), which is where \
                     the planner put {rel}",
                    coord.col, coord.row
                )
            });
        assert_decoded_tiles_equal(
            std::slice::from_ref(&(rel.clone(), on_disk)),
            std::slice::from_ref(&(rel.clone(), in_archive)),
            "planner z/x/y vs archive z/x/y",
        );
        compared += 1;
    }

    assert_eq!(
        compared as u64,
        plan.total_tile_count(),
        "only {compared} coordinates were checked against the raw archive"
    );
}
