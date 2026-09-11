//! The CLI half of libviprs-tests#202: `viprs pyramid` writing a PMTiles
//! archive, and the `viprs pmtiles` group reading one back.
//!
//! The rest of the PMTiles suite in this repo talks to the library. This file
//! is the only place that drives the real `viprs` binary, so it is the only
//! place that can tell you the CLI and the library agree about what a pyramid
//! is. EPIC F flips `viprs pyramid`'s default storage to PMTiles
//! (`libviprs/libviprs-cli#54`), which means every existing caller of that
//! command now gets an archive where it used to get a tree, and the promise
//! attached to the flip is that the pixels are the same ones.
//!
//! # Exit codes are not evidence
//!
//! A cell that runs `viprs pyramid` and asserts exit 0 passes when the command
//! wrote nothing at all, and a cell that runs `viprs pmtiles verify` and
//! asserts exit 0 passes when verify is a stub that returns success. So every
//! test here asserts on an artifact or on decoded output:
//!
//! * the archive exists, and the library reads tiles out of it;
//! * the tiles decode, and their pixels equal the `--storage directory` run of
//!   the same input, tile for tile, at tolerance 0;
//! * `pmtiles info`'s numbers equal the ones the library reads from the same
//!   file, and are different numbers for a different archive;
//! * `pmtiles verify` refuses a damaged archive as well as accepting a good
//!   one;
//! * `pmtiles tile`'s bytes decode to the same pixels as the tile of the same
//!   name in the directory tree.
//!
//! # The skip, and why it is safe
//!
//! [`cli_available`](common::cli::cli_available) returns false when the
//! `libviprs-cli` sibling is not laid down, and every test here returns early
//! when it does. A skip reads to `cargo test` as a pass, so on its own that is
//! a false green waiting to happen. What makes it safe is `VIPRS_REQUIRE_CLI=1`
//! on the `cli-differential` job: with it set, `cli_available` panics instead
//! of returning false. `tests/pmtiles_ci_wiring.rs` holds CI to both halves of
//! that, the env var and the `--test cli_pmtiles` line, because the skip is
//! only safe while something refuses to take it.
//!
//! # Nothing here may sit behind a cargo feature
//!
//! `tests/common/cli.rs` builds the binary with `cargo build --release
//! --no-default-features --bin viprs` (CLI_CONTRACT.md §7). `libviprs-cli`'s
//! only default feature is `pdfium`, and `viprs pmtiles` is compiled
//! unconditionally, so the whole group survives that build. A future flag that
//! hid any of it behind a feature would make these cells skip nothing and test
//! nothing, which is why they drive PNG input rather than PDF.

mod common;

use std::path::{Path, PathBuf};

use common::cli::{cli_available, run_viprs, run_viprs_ok};
use common::dzsave_expected::collect_files;
use common::fixtures::canonical_raster_scaled;

#[path = "common/pmtiles.rs"]
mod pmtiles_support;
use pmtiles_support::assert_decoded_tiles_equal;

use libviprs::planner::TileCoord;
use libviprs::pmtiles::tileid_to_zxy;
use libviprs::{PixelFormat, PmTilesPyramidReader, PyramidReader, Raster};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Skip guard
// ---------------------------------------------------------------------------

/// `true` (with a printed reason) when the CLI sibling is absent.
///
/// Under `$VIPRS_REQUIRE_CLI=1` this never returns: [`cli_available`] panics
/// first, which is what stops the `cli-differential` job reporting green for
/// the same reason a laptop does, namely that it did not run.
fn skip_if_no_cli(test: &str) -> bool {
    if cli_available() {
        return false;
    }
    eprintln!(
        "SKIP {test}: libviprs-cli sibling not checked out \
         (set $VIPRS_CLI_DIR / $VIPRS_BIN, or run in the cli-differential job)."
    );
    true
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The input every cell here starts from: a PNG on disk, because that is what
/// `viprs pyramid` takes and what its default-output rule derives a name from.
///
/// 640x480 at the default 256-pixel tile is three zoom levels with a
/// non-square top level (3 cols by 2 rows), so an archive that quietly
/// transposed x and y, or collapsed a level, would not survive the comparison.
fn write_input(dir: &Path, stem: &str, w: u32, h: u32) -> PathBuf {
    let src: Raster = canonical_raster_scaled(w, h);
    let path = dir.join(format!("{stem}.png"));
    let colour = match src.format() {
        PixelFormat::Gray8 => image::ColorType::L8,
        PixelFormat::Rgb8 => image::ColorType::Rgb8,
        PixelFormat::Rgba8 => image::ColorType::Rgba8,
        other => panic!("the canonical fixture is {other:?}; teach this helper about it"),
    };
    image::save_buffer(&path, src.data(), src.width(), src.height(), colour)
        .expect("write the PNG the CLI will read");
    assert!(
        path.is_file() && std::fs::metadata(&path).expect("stat the input").len() > 0,
        "the input PNG was not written, so every `viprs pyramid` below would be \
         reading nothing"
    );
    path
}

fn s(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// `viprs pyramid <input>` with no output and no `--storage`: the flipped
/// default. Returns the archive it must have written.
fn pyramid_default(input: &Path) -> PathBuf {
    let out = run_viprs(&["pyramid", s(input)]);
    assert!(
        out.status.success(),
        "viprs pyramid {} exited {}\n--- stderr ---\n{}",
        input.display(),
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    input.with_extension("pmtiles")
}

/// `viprs pyramid <input> <dir> --storage directory --layout xyz`: the same
/// pyramid as loose files, addressed the way PMTiles addresses tiles.
///
/// `--layout xyz` is not optional. `--storage directory` still defaults to
/// Deep Zoom, so without it the tree comes back as `{z}/{x}_{y}.png` and every
/// path comparison below would be comparing two different namings of two
/// different pyramids.
fn pyramid_tree(input: &Path, dir: &Path) {
    let out = run_viprs(&[
        "pyramid",
        s(input),
        s(dir),
        "--storage",
        "directory",
        "--layout",
        "xyz",
    ]);
    assert!(
        out.status.success(),
        "viprs pyramid --storage directory exited {}\n--- stderr ---\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
}

/// A `{z}/{x}/{y}.png` relative path back into the coordinate it names.
///
/// Deriving the coordinates from the tree the CLI wrote, rather than from a
/// `PyramidPlanner` this file builds, is deliberate: a plan constructed here
/// would be this repo's opinion about what the CLI should have produced, and
/// the two agreeing would then be one source of truth wearing two hats.
fn coord_of(rel: &str) -> TileCoord {
    let parts: Vec<&str> = rel.trim_end_matches(".png").split('/').collect();
    assert_eq!(
        parts.len(),
        3,
        "expected an xyz tile path of the form z/x/y.png, got {rel:?}"
    );
    let num = |what: &str, text: &str| {
        text.parse::<u32>()
            .unwrap_or_else(|e| panic!("{what} in {rel:?} is not a number: {e}"))
    };
    TileCoord {
        level: num("the zoom", parts[0]),
        col: num("the column", parts[1]),
        row: num("the row", parts[2]),
    }
}

/// Every tile the archive holds for the coordinates the tree names, in the
/// tree's own path order, with holes left out so a count assertion can see them.
fn archive_tiles(archive: &Path, tree: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let reader = PmTilesPyramidReader::try_open(archive)
        .unwrap_or_else(|e| panic!("open {}: {e}", archive.display()));
    let mut out = Vec::new();
    for (rel, _) in tree {
        let coord = coord_of(rel);
        match reader
            .tile(coord)
            .unwrap_or_else(|e| panic!("read {rel}: {e}"))
        {
            Some(bytes) if !bytes.is_empty() => out.push((rel.clone(), bytes)),
            Some(_) => panic!(
                "the archive answered {rel} with zero bytes. An empty payload is \
                 a hole wearing a hit's clothes, and it would compare equal to \
                 nothing at all."
            ),
            None => panic!(
                "the archive has no tile at {rel}, which `viprs pyramid \
                 --storage directory` wrote from the same input. The two \
                 backends do not hold the same pyramid."
            ),
        }
    }
    out
}

/// The value after `label` on the one line of `stdout` that starts with it.
fn field<'a>(stdout: &'a str, label: &str) -> &'a str {
    let mut hits = stdout
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(label));
    let first = hits
        .next()
        .unwrap_or_else(|| panic!("no {label:?} line in:\n{stdout}"))
        .trim();
    assert!(
        hits.next().is_none(),
        "{label:?} appears more than once, so reading the first one is a guess:\n{stdout}"
    );
    first
}

fn field_u64(stdout: &str, label: &str) -> u64 {
    let text = field(stdout, label);
    text.split_whitespace()
        .next()
        .unwrap_or(text)
        .parse::<u64>()
        .unwrap_or_else(|e| panic!("{label:?} is {text:?}, which is not a number: {e}"))
}

// ---------------------------------------------------------------------------
// viprs pyramid
// ---------------------------------------------------------------------------

/// The flipped default, end to end: no output argument, no `--storage`, and
/// what lands is an archive this repo's library reads tiles out of.
///
/// The count assertions are the point. Without them a reader that answers
/// `None` everywhere and a `pyramid` that wrote a 127-byte header would both
/// pass, because "every tile I asked for matched" is true of no tiles.
#[test]
fn cli_pyramid_writes_a_pmtiles_the_library_can_read() {
    if skip_if_no_cli("cli_pyramid_writes_a_pmtiles_the_library_can_read") {
        return;
    }
    let dir = TempDir::new().expect("temp dir");
    let input = write_input(dir.path(), "drawing", 640, 480);

    let archive = pyramid_default(&input);
    assert!(
        archive.is_file(),
        "`viprs pyramid drawing.png` must write drawing.pmtiles beside its \
         input, and {} is not there",
        archive.display()
    );
    assert!(
        !dir.path().join("drawing").exists(),
        "the default run also left a loose `drawing/` tree behind, so the flip \
         to archive storage has not actually happened"
    );

    let reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive viprs wrote");
    let header = *reader.reader().header();
    assert!(
        header.addressed_tiles_count > 0,
        "the archive addresses no tiles at all, so it is a header and nothing \
         else"
    );
    assert!(
        header.max_zoom > header.min_zoom,
        "a 640x480 input at the default 256-pixel tile is more than one zoom \
         level, and this archive reports {}-{}",
        header.min_zoom,
        header.max_zoom
    );

    // Every addressed tile has to be reachable and has to decode. The walk is
    // driven by the index rather than by a sweep of every coordinate a zoom
    // range allows: this pyramid is zoom 0 to 10, so a sweep would be 1.4
    // million lookups to find 17 tiles, and it would grow by a factor of four
    // per level the fixture gained.
    //
    // Going through `tileid_to_zxy` and back through `PyramidReader::tile` also
    // says something the count alone does not: every id the index holds maps to
    // a coordinate, and that coordinate maps back to the same entry. A reader
    // whose two directions disagree holds tiles nobody can ask for.
    assert!(
        !header.has_leaves(),
        "this fixture is a 17-tile archive and it has grown leaf directories, \
         so walking the root no longer reaches everything and this cell would \
         be checking a subset without saying so"
    );
    let mut seen = 0u64;
    for entry in reader.reader().root_entries() {
        assert!(
            entry.run_length > 0,
            "a leaf pointer in the root of an archive whose header says it has \
             no leaves"
        );
        for id in entry.tile_id..entry.tile_id + u64::from(entry.run_length) {
            let (z, x, y) = tileid_to_zxy(id).unwrap_or_else(|e| {
                panic!("the index holds tile id {id}, which is not a coordinate: {e}")
            });
            let coord = TileCoord {
                level: u32::from(z),
                col: x,
                row: y,
            };
            let bytes = reader
                .tile(coord)
                .unwrap_or_else(|e| panic!("read {z}/{x}/{y}: {e}"))
                .unwrap_or_else(|| {
                    panic!(
                        "the index holds an entry for {z}/{x}/{y} and asking for \
                         that coordinate answers None, so the two directions of \
                         the id mapping disagree"
                    )
                });
            let decoded = image::load_from_memory(&bytes)
                .unwrap_or_else(|e| panic!("tile {z}/{x}/{y} does not decode: {e}"));
            assert!(
                decoded.width() > 0 && decoded.height() > 0,
                "tile {z}/{x}/{y} decoded to nothing"
            );
            seen += 1;
        }
    }
    assert_eq!(
        seen, header.addressed_tiles_count,
        "the walk reached {seen} readable tiles and the header claims {}. Either \
         the header is lying or the index does not reach everything it counted.",
        header.addressed_tiles_count
    );
}

/// The cross-backend claim, stated where the CLI decides it.
///
/// `phase_pmtiles.rs` pins the same equality through the library API. This one
/// is about `viprs pyramid`'s own flag routing: `--storage pmtiles` and
/// `--storage directory` are two different code paths in `src/main.rs` that
/// build two different sinks, and nothing in the library can notice one of them
/// picking a different plan.
#[test]
fn cli_pyramid_gives_the_same_pyramid_to_both_storages() {
    if skip_if_no_cli("cli_pyramid_gives_the_same_pyramid_to_both_storages") {
        return;
    }
    let dir = TempDir::new().expect("temp dir");
    let input = write_input(dir.path(), "both", 640, 480);
    let tree = dir.path().join("tree");

    let archive = pyramid_default(&input);
    pyramid_tree(&input, &tree);

    let tree_tiles = collect_files(&tree, "png");
    assert!(
        tree_tiles.len() > 1,
        "`--storage directory --layout xyz` wrote {} tile(s), so the comparison \
         below would be about almost nothing",
        tree_tiles.len()
    );
    let archive_tiles = archive_tiles(&archive, &tree_tiles);

    assert_decoded_tiles_equal(
        &tree_tiles,
        &archive_tiles,
        "viprs pyramid --storage directory vs the default archive",
    );
}

// ---------------------------------------------------------------------------
// viprs pmtiles info
// ---------------------------------------------------------------------------

/// `pmtiles info` has to report this archive's numbers, not a template's.
///
/// Reading one archive proves less than it looks: every number printed could be
/// a constant and the assertions would still pass. So two archives of different
/// sizes go through, each is checked against what the library reads from that
/// same file, and the two are required to disagree with each other. That last
/// assertion is the negative control, and it is the one that fails if `info`
/// ever stops reading the file it was handed.
#[test]
fn cli_pmtiles_info_reports_the_archive_the_library_sees() {
    if skip_if_no_cli("cli_pmtiles_info_reports_the_archive_the_library_sees") {
        return;
    }
    let dir = TempDir::new().expect("temp dir");

    let mut reported = Vec::new();
    for (stem, w, h) in [("small", 300u32, 200u32), ("large", 900, 700)] {
        let input = write_input(dir.path(), stem, w, h);
        let archive = pyramid_default(&input);

        let stdout = run_viprs_ok(&["pmtiles", "info", s(&archive)]);
        let reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive");
        let header = *reader.reader().header();
        let bytes_on_disk = std::fs::metadata(&archive).expect("stat the archive").len();

        assert_eq!(field(&stdout, "Version:"), "3");
        assert_eq!(
            field(&stdout, "Tile type:"),
            "png",
            "`viprs pyramid` defaults to PNG tiles and info says otherwise:\n{stdout}"
        );
        assert_eq!(
            field(&stdout, "Zoom:"),
            format!("{}-{}", header.min_zoom, header.max_zoom),
            "info's zoom range is not the header's:\n{stdout}"
        );
        assert_eq!(
            field_u64(&stdout, "Addressed tiles:"),
            header.addressed_tiles_count,
            "info's addressed-tile count is not the header's:\n{stdout}"
        );
        assert_eq!(
            field_u64(&stdout, "Tile entries:"),
            header.tile_entries_count,
            "info's tile-entry count is not the header's:\n{stdout}"
        );
        assert_eq!(
            field_u64(&stdout, "Unique payloads:"),
            header.tile_contents_count,
            "info's unique-payload count is not the header's:\n{stdout}"
        );
        assert_eq!(
            field_u64(&stdout, "Archive size:"),
            bytes_on_disk,
            "info's archive size is not the size of the file on disk:\n{stdout}"
        );
        assert_eq!(
            field_u64(&stdout, "Root entries:"),
            reader.reader().root_entries().len() as u64,
            "info's root-entry count is not the one the library reads:\n{stdout}"
        );

        reported.push((header.addressed_tiles_count, header.max_zoom, bytes_on_disk));
    }

    let (small, large) = (reported[0], reported[1]);
    assert_ne!(
        small, large,
        "a 300x200 input and a 900x700 input produced identical \
         (addressed tiles, max zoom, size) triples, so every assertion above is \
         consistent with `info` printing constants"
    );
}

// ---------------------------------------------------------------------------
// viprs pmtiles verify
// ---------------------------------------------------------------------------

/// `verify` accepts what `pyramid` wrote, and refuses an archive that has been
/// damaged.
///
/// The acceptance half on its own is satisfied by a `verify` that returns 0
/// unconditionally, which is the failure mode worth guarding: a checker nobody
/// has ever seen say no. So the same archive is truncated by its last 64 bytes
/// and fed back in, and `verify` has to refuse it.
///
/// Truncation is the right mutation because it leaves every structure intact
/// and breaks only the arithmetic. The header still parses, the root directory
/// still decodes, and what stops being true is that the tile data the header
/// points at fits inside the file. Measured on a 217720-byte archive: the good
/// one verifies with `OK` and exit 0, and the 217656-byte one comes back exit 1
/// with "the tile data at offset 464 for 217256 bytes does not fit in a 217656
/// byte archive". go-pmtiles' own `verify` never adds a length to an offset, so
/// that bound is ours rather than the reference's.
///
/// The assertions stay on the exit code and the absence of `OK` rather than on
/// that sentence: the message belongs to the other repository and rewording it
/// is not a regression.
#[test]
fn cli_pmtiles_verify_accepts_what_pyramid_wrote_and_refuses_a_damaged_one() {
    if skip_if_no_cli("cli_pmtiles_verify_accepts_what_pyramid_wrote_and_refuses_a_damaged_one") {
        return;
    }
    let dir = TempDir::new().expect("temp dir");
    let input = write_input(dir.path(), "verified", 640, 480);
    let archive = pyramid_default(&input);

    let stdout = run_viprs_ok(&["pmtiles", "verify", s(&archive)]);
    assert!(
        stdout.contains("OK"),
        "verify exited 0 without saying so, which means the verdict line has \
         moved and this cell is reading the wrong thing:\n{stdout}"
    );

    let reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive");
    let header = *reader.reader().header();
    assert_eq!(
        field_u64(&stdout, "Addressed tiles:"),
        header.addressed_tiles_count,
        "verify walked a different number of tiles than the header counts, and \
         still called it OK:\n{stdout}"
    );
    assert!(
        header.addressed_tiles_count > 0,
        "verify said OK about an archive with no tiles in it"
    );

    // Same bytes, minus the last 64. The header still parses and the directory
    // still decodes; what stops being true is that every entry's offset plus
    // length lands inside the file.
    let good = std::fs::read(&archive).expect("read the archive back");
    assert!(
        good.len() > 64,
        "the archive is {} bytes, so lopping 64 off it is not a truncation, it \
         is a deletion",
        good.len()
    );
    let damaged = dir.path().join("damaged.pmtiles");
    std::fs::write(&damaged, &good[..good.len() - 64]).expect("write the truncated archive");

    let out = run_viprs(&["pmtiles", "verify", s(&damaged)]);
    assert!(
        !out.status.success(),
        "verify accepted an archive whose last 64 bytes are missing. A checker \
         that has never said no is not a checker.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("OK"),
        "verify refused the damaged archive and printed OK anyway, so the exit \
         code and the message disagree"
    );

    // The truncation above is caught when the archive is opened, before the
    // walk starts, so on its own it says nothing about the walk. Measured: with
    // `verify`'s problem list neutered so it can never report anything, the
    // truncated archive was still refused and this cell stayed green. That is a
    // mutation nothing here caught, which is the same as not testing the walk.
    //
    // So damage the one thing only the walk can see. `addressed_tiles_count` is
    // a header field nothing validates at open time: the archive parses, every
    // directory decodes, every tile reads, and the only thing wrong with it is
    // that the header's own count disagrees with what the runs cover. A writer
    // that miscounts produces exactly this, every reader still reads it, and
    // `verify` is the only thing that would ever notice.
    const OFF_ADDRESSED_TILES: usize = 72;
    let mut miscounted = good.clone();
    let claimed = u64::from_le_bytes(
        miscounted[OFF_ADDRESSED_TILES..OFF_ADDRESSED_TILES + 8]
            .try_into()
            .expect("eight bytes"),
    );
    assert_eq!(
        claimed, header.addressed_tiles_count,
        "byte {OFF_ADDRESSED_TILES} of the header is not the addressed-tile \
         count any more, so this damage is landing somewhere else"
    );
    miscounted[OFF_ADDRESSED_TILES..OFF_ADDRESSED_TILES + 8]
        .copy_from_slice(&(claimed + 1).to_le_bytes());
    let miscounted_path = dir.path().join("miscounted.pmtiles");
    std::fs::write(&miscounted_path, &miscounted).expect("write the miscounted archive");

    // The positive control for the damage: it still opens. Without this the
    // refusal below would be indistinguishable from the truncation's, which is
    // the thing this second case exists to be different from.
    let reopened =
        PmTilesPyramidReader::try_open(&miscounted_path).expect("the miscounted archive must open");
    assert_eq!(
        reopened.reader().header().addressed_tiles_count,
        claimed + 1,
        "the damage did not take"
    );

    let out = run_viprs(&["pmtiles", "verify", s(&miscounted_path)]);
    assert!(
        !out.status.success(),
        "verify accepted an archive whose header claims {} addressed tiles while \
         its runs cover {}. Every reader reads this file; verify is the only \
         thing that would ever notice.\n--- stdout ---\n{}\n--- stderr ---\n{}",
        claimed + 1,
        claimed,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// viprs pmtiles tile
// ---------------------------------------------------------------------------

/// `pmtiles tile` hands back the tile the loose tree holds at the same name.
///
/// Decoded pixels, never the compressed bytes: the two sides share an encoder
/// today, so a byte comparison would pass and would start lying the first time
/// either gained an encoder setting.
///
/// The last assertion is the negative control. Everything above it is "the tool
/// said yes", and a tool that says yes to everything hands back a real,
/// decodable tile from the wrong coordinate. So ask for a zoom level above the
/// archive's and require a refusal with nothing on stdout, because a caller
/// piping this into a decoder must not get a sentence where the bytes go.
#[test]
fn cli_pmtiles_tile_hands_back_the_bytes_the_tree_holds() {
    if skip_if_no_cli("cli_pmtiles_tile_hands_back_the_bytes_the_tree_holds") {
        return;
    }
    let dir = TempDir::new().expect("temp dir");
    let input = write_input(dir.path(), "tiles", 640, 480);
    let tree = dir.path().join("tree");
    let archive = pyramid_default(&input);
    pyramid_tree(&input, &tree);

    let tree_tiles = collect_files(&tree, "png");
    assert!(
        tree_tiles.len() >= 3,
        "the tree holds {} tile(s); this cell samples three and needs them to \
         be three different ones",
        tree_tiles.len()
    );

    // First, middle and last of the sorted tree: one from the top of the
    // pyramid, one from the middle, one from the deepest level.
    let picks = [0, tree_tiles.len() / 2, tree_tiles.len() - 1];
    let mut want: Vec<(String, Vec<u8>)> = Vec::new();
    let mut got: Vec<(String, Vec<u8>)> = Vec::new();
    for (n, i) in picks.iter().enumerate() {
        let (rel, bytes) = &tree_tiles[*i];
        let coord = coord_of(rel);
        let out_file = dir.path().join(format!("tile-{n}.png"));
        let out = run_viprs(&[
            "pmtiles",
            "tile",
            s(&archive),
            &coord.level.to_string(),
            &coord.col.to_string(),
            &coord.row.to_string(),
            "-o",
            s(&out_file),
        ]);
        assert!(
            out.status.success(),
            "viprs pmtiles tile {rel} exited {}\n--- stderr ---\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
        );
        let written = std::fs::read(&out_file)
            .unwrap_or_else(|e| panic!("pmtiles tile exited 0 and wrote no {rel}: {e}"));
        assert!(
            !written.is_empty(),
            "pmtiles tile exited 0 and wrote an empty file for {rel}"
        );
        want.push((rel.clone(), bytes.clone()));
        got.push((rel.clone(), written));
    }
    assert_eq!(
        got.len(),
        3,
        "this cell claims to compare three tiles and compared {}",
        got.len()
    );
    assert_decoded_tiles_equal(&want, &got, "the tree vs viprs pmtiles tile");

    let reader = PmTilesPyramidReader::try_open(&archive).expect("open the archive");
    let beyond = u32::from(reader.reader().header().max_zoom) + 1;
    let out = run_viprs(&[
        "pmtiles",
        "tile",
        s(&archive),
        &beyond.to_string(),
        "0",
        "0",
    ]);
    assert!(
        !out.status.success(),
        "pmtiles tile answered for zoom {beyond}, which is past the archive's \
         own max zoom. A reader that says yes everywhere returns a real tile \
         from the wrong place."
    );
    assert!(
        out.stdout.is_empty(),
        "pmtiles tile failed and still put {} bytes on stdout; a caller piping \
         this into a decoder gets a sentence where the tile goes",
        out.stdout.len()
    );
}
