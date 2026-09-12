//! Interop with the real `protomaps/go-pmtiles` (libviprs-tests#202).
//!
//! Two halves, and they prove different things.
//!
//! **Offline.** Three archives under `tests/fixtures/pmtiles/`, every byte of
//! them written by go-pmtiles v1.31.2, plus the reference's own answers for a
//! handful of coordinates in each. Our reader has to agree with an
//! implementation that has never seen our code. This half needs no network, no
//! binary and no environment, so it runs everywhere the suite runs.
//!
//! **The external binary.** When a pinned `pmtiles` is reachable, generate an
//! archive with libviprs and have the reference accept it: `pmtiles verify`
//! exits 0, and `pmtiles tile` hands back the bytes our reader hands back. That
//! half skips when the binary is absent, which is right for a dev machine and a
//! false green anywhere else, so `VIPRS_REQUIRE_GO_PMTILES=1` turns the skip
//! into a panic. `tests/pmtiles_ci_wiring.rs` holds CI to setting it.
//!
//! # Why an oracle and not a round trip
//!
//! A writer and a reader that share a misreading of the spec round-trip
//! perfectly and both look correct. That is not a hypothetical on this epic:
//! the PMTiles v3 specification gives no algorithm for the tile id mapping, and
//! four different conventions reproduce every row of the spec's own worked
//! example. A property test cannot separate them, because a wrong mapping is
//! still a perfect inverse of itself. Only a pinned external value can.
//!
//! # The rows we refuse rather than match
//!
//! `tiles.json` also records what the reference does with a coordinate whose x
//! or y is past `2**z - 1`: `ZxyToID` masks it into a different, valid tile and
//! serves that tile's payload with exit 0. Those rows are evidence about the
//! reference, not a target. Reproducing them would mean reproducing a bug that
//! silently answers the wrong question, so
//! `an_out_of_range_coordinate_is_refused_rather_than_masked` requires the
//! refusal and uses the reference's own answer as the control that there really
//! was something to refuse.

use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::fixtures::canonical_raster_scaled;

#[path = "common/pmtiles.rs"]
mod pmtiles_support;
use pmtiles_support::{
    DISTINCT_GOLDEN, DISTINCT_GOLDEN_SHA256, GOLDENS, ORACLE_RELEASE_TAG, TILES_VECTOR_GOLDENS,
    assert_probes_discriminate, golden_path, header_vectors, oracle_header_fields,
    oracle_header_i64, oracle_header_u64, oracle_shown, oracle_shown_numbers, read_checked,
    sha256_hex, show_vectors, sweep_vectors, tile_rows, tiles_vectors, vector_archive_names,
    vector_golden_sha256,
};

use libviprs::pmtiles::directory::deserialize_entries;
use libviprs::pmtiles::header::SPEC_VERSION;
use libviprs::pmtiles::{Compression, Entry, Reader, tileid_to_zxy};
use libviprs::sink_pmtiles::PmTilesSink;
use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, FsSink, Layout, PyramidPlan, PyramidPlanner,
    TileFormat,
};

// ---------------------------------------------------------------------------
// The offline half
// ---------------------------------------------------------------------------

/// The committed archives have to be the archives the vectors describe.
///
/// Two independent digests for the same bytes: one in `tests/common/pmtiles.rs`
/// and one the oracle's dump program wrote into the vector file beside them. A
/// fixture edited to make a red test pass has to defeat both, and the second is
/// not somewhere anybody editing a test would think to look.
///
/// Two vector files, because `tiles.json` carries hand-picked rows and the two
/// goldens added later are covered by `sweep.json`, which holds every cell of
/// every zoom instead. Both lists are checked against the committed set so an
/// archive cannot land in the fixtures directory and be described by neither.
#[test]
fn the_committed_goldens_are_the_ones_the_vectors_describe() {
    let tiles = tiles_vectors();
    let names = vector_archive_names(&tiles);
    assert_eq!(
        names.len(),
        TILES_VECTOR_GOLDENS.len(),
        "tiles.json covers {:?} but {} archives expect rows in it, so one of \
         them is being tested by nothing",
        names,
        TILES_VECTOR_GOLDENS.len()
    );
    for (name, digest) in TILES_VECTOR_GOLDENS {
        assert!(
            names.iter().any(|n| n == name),
            "{name} expects rows in tiles.json and has none"
        );
        let bytes = read_checked(name, digest);
        assert_eq!(
            sha256_hex(&bytes),
            vector_golden_sha256(&tiles, name),
            "{name} on disk is not the archive tiles.json was dumped from"
        );
    }

    // Every committed golden, including the two that have no hand-picked rows,
    // has to be described somewhere.
    let swept = vector_archive_names(&sweep_vectors());
    let shown = vector_archive_names(&show_vectors());
    for (name, digest) in GOLDENS {
        assert!(
            swept.iter().any(|n| n == name),
            "{name} is committed and sweep.json has no cells for it, so nothing \
             sweeps it"
        );
        assert!(
            shown.iter().any(|n| n == name),
            "{name} is committed and show.json has no fields for it"
        );
        let bytes = read_checked(name, digest);
        assert_eq!(
            sha256_hex(&bytes),
            vector_golden_sha256(&sweep_vectors(), name),
            "{name} on disk is not the archive sweep.json was dumped from"
        );
    }
}

/// Our reader has to return, byte for byte, what `pmtiles tile` returned.
#[test]
fn libviprs_reads_every_go_pmtiles_golden() {
    let tiles = tiles_vectors();
    let mut checked = 0usize;

    for (name, digest) in TILES_VECTOR_GOLDENS {
        let path = golden_path(name, digest);
        let reader = Reader::try_open(&path).unwrap_or_else(|e| panic!("open {name}: {e}"));

        for row in tile_rows(&tiles, name, "hits") {
            assert_eq!(
                row.exit_code, 0,
                "{name} hit row ({}, {}, {}) is recorded with a non-zero exit, \
                 so it is not a hit",
                row.z, row.x, row.y
            );
            assert!(
                row.length > 0,
                "{name} hit row ({}, {}, {}) has zero length, which is how the \
                 reference reports an absent tile, so it belongs in the absent \
                 list rather than here",
                row.z,
                row.x,
                row.y
            );

            let got = reader
                .get_tile(row.z, row.x, row.y)
                .unwrap_or_else(|e| panic!("{name} ({}, {}, {}): {e}", row.z, row.x, row.y))
                .unwrap_or_else(|| {
                    panic!(
                        "{name} has no tile at ({}, {}, {}), but go-pmtiles \
                         returned {} bytes for it",
                        row.z, row.x, row.y, row.length
                    )
                });

            assert_eq!(
                got.len(),
                row.length,
                "{name} ({}, {}, {}): we read {} bytes, go-pmtiles read {}",
                row.z,
                row.x,
                row.y,
                got.len(),
                row.length
            );
            assert_eq!(
                sha256_hex(&got),
                row.sha256,
                "{name} ({}, {}, {}): same length, different bytes. Equal \
                 lengths are exactly what a wrong-but-plausible offset \
                 produces, which is why this compares the digest and not the \
                 count.",
                row.z,
                row.x,
                row.y
            );
            checked += 1;
        }
    }

    assert!(
        checked >= 20,
        "only {checked} tiles were compared against the reference across three \
         archives, which is fewer than the vector file carries, so the sweep \
         above is skipping rows"
    );
}

/// A tile the archive does not hold is absent, not an error.
///
/// The negative control for the test above. A reader that says yes to
/// everything passes every hit row and hands back a real, decodable tile from
/// the wrong place for everything else.
#[test]
fn a_tile_the_archive_does_not_hold_is_absent_rather_than_an_error() {
    let tiles = tiles_vectors();
    let mut checked = 0usize;

    for (name, digest) in TILES_VECTOR_GOLDENS {
        let reader = Reader::try_open(golden_path(name, digest))
            .unwrap_or_else(|e| panic!("open {name}: {e}"));
        for row in tile_rows(&tiles, name, "absent") {
            assert_eq!(
                row.length, 0,
                "{name} absent row ({}, {}, {}) records a payload, so it is not \
                 absent",
                row.z, row.x, row.y
            );
            let got = reader.get_tile(row.z, row.x, row.y).unwrap_or_else(|e| {
                panic!("{name} ({}, {}, {}) errored: {e}", row.z, row.x, row.y)
            });
            assert_eq!(
                got, None,
                "{name} answered with bytes at ({}, {}, {}), which go-pmtiles \
                 reports as empty. A directory lookup lands on the preceding \
                 entry for every hole, and returning that entry's bytes looks \
                 like working code because they decode.",
                row.z, row.x, row.y
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no absent rows were checked at all");
}

/// The reference masks an out-of-grid coordinate into a different tile. We
/// refuse it.
///
/// Being faithful to a reference is not the same as reproducing it. `ZxyToID`
/// takes `(z=2, x=4, y=0)`, which does not exist on a 4-wide grid, wraps it,
/// and serves a real tile with exit 0. A caller that asked the wrong question
/// gets a confident wrong answer. The control here is the reference's own row:
/// it recorded a non-empty payload, so there is genuinely something we are
/// declining to hand back.
#[test]
fn an_out_of_range_coordinate_is_refused_rather_than_masked() {
    let tiles = tiles_vectors();
    let mut checked = 0usize;

    for (name, digest) in TILES_VECTOR_GOLDENS {
        let reader = Reader::try_open(golden_path(name, digest))
            .unwrap_or_else(|e| panic!("open {name}: {e}"));
        for row in tile_rows(&tiles, name, "out_of_range") {
            assert!(
                row.length > 0 && row.exit_code == 0,
                "{name} out-of-range row ({}, {}, {}) is recorded as failing or \
                 empty, so this cell would be asserting that we refuse \
                 something the reference also refused, which proves nothing",
                row.z,
                row.x,
                row.y
            );
            let answer = reader.get_tile(row.z, row.x, row.y);
            assert!(
                answer.is_err(),
                "{name} answered {:?} for ({}, {}, {}), a coordinate outside its \
                 own grid. go-pmtiles masks that into a different valid tile and \
                 serves it; matching that would mean reproducing a bug that \
                 answers a question nobody asked.",
                answer.map(|o| o.map(|b| b.len())),
                row.z,
                row.x,
                row.y
            );
            checked += 1;
        }
    }
    assert!(checked > 0, "no out-of-range rows were checked at all");
}

/// A tile entry found inside a leaf is relative to `tile_data_offset`, not to
/// the leaf it was found in, and not to anything else that happens to work on
/// one fixture.
///
/// The spec keys an entry's base off the **entry kind**, not off the directory
/// it was read from, and none of the three fixtures in the upstream spec
/// repository has leaf directories at all, so nothing upstream exercises this.
/// A writer and a reader that make the same wrong choice agree with each other.
///
/// # The base that used to hide
///
/// This ran on `leaves-z0z7` and demonstrated one wrong base,
/// `leaf_directory_offset + entry_offset`. There are others, and the
/// interesting one is the leaf's own first entry offset, which a writer
/// rebasing each leaf onto itself produces. Every leaf in `leaves-z0z7` starts
/// at entry offset 0, so that base is the identity there and the fixture cannot
/// see it: measured, a reader rebased that way passes the entire PMTiles suite.
///
/// So this runs on `distinct-z0z7`, whose leaves start at 49164, 98324, 147497
/// and 196597, and it checks every candidate base rather than one. The leaf
/// golden stays committed and stays swept, it just stops carrying a claim it
/// cannot support.
///
/// Under a wrong base the offsets still land inside the file, on the leaf
/// directory region, so `get_tile` returns `Some` either way and a test that
/// checks for `Some` passes on a reader handing back directory bytes dressed as
/// a tile. So this compares the payload, and then shows what each wrong base
/// would have returned instead, which is the proof that the comparison can fail.
#[test]
fn leaf_entries_resolve_against_tile_data_not_against_any_other_base() {
    let headers = header_vectors();
    let leaf_offset = oracle_header_u64(&headers, DISTINCT_GOLDEN, "leaf_directory_offset");
    let leaf_length = oracle_header_u64(&headers, DISTINCT_GOLDEN, "leaf_directory_length");
    let data_offset = oracle_header_u64(&headers, DISTINCT_GOLDEN, "tile_data_offset");
    let root_offset = oracle_header_u64(&headers, DISTINCT_GOLDEN, "root_offset");

    assert!(
        leaf_length > 0,
        "{DISTINCT_GOLDEN} has no leaf directories, so this cell is about a code \
         path the fixture never reaches"
    );
    assert_ne!(
        leaf_offset, data_offset,
        "the two candidate bases are the same number in this fixture, so it \
         cannot tell them apart"
    );

    let archive = read_checked(DISTINCT_GOLDEN, DISTINCT_GOLDEN_SHA256);
    let reader = Reader::try_open(golden_path(DISTINCT_GOLDEN, DISTINCT_GOLDEN_SHA256))
        .expect("open the distinct golden");
    assert_eq!(reader.header().leaf_directories_offset, leaf_offset);
    assert_eq!(reader.header().leaf_directories_length, leaf_length);
    assert_eq!(reader.header().tile_data_offset, data_offset);

    // Walk the root for leaf pointers, which carry a run length of zero and
    // nothing else that marks them, then read the entries behind each one.
    let root = decompress_section(&archive, root_offset, reader.header().root_length);
    let root_entries = deserialize_entries(&root).expect("deserialise the root directory");
    let pointers: Vec<Entry> = root_entries
        .iter()
        .filter(|e| e.run_length == 0)
        .copied()
        .collect();
    assert!(
        pointers.len() >= 2,
        "{DISTINCT_GOLDEN} has {} leaf pointers, so there is no second leaf for \
         a per-leaf base to be wrong about",
        pointers.len()
    );
    let starts: Vec<u64> = pointers.iter().map(|p| p.offset).collect();
    assert!(
        starts.iter().any(|s| *s != 0),
        "every leaf in {DISTINCT_GOLDEN} starts at offset 0, which makes a \
         per-leaf rebase the identity and this whole cell decorative. The \
         leaves start at {starts:?}"
    );

    let mut checked = 0usize;
    let mut discriminating = 0usize;
    for pointer in &pointers {
        let leaf = decompress_section(
            &archive,
            leaf_offset + pointer.offset,
            u64::from(pointer.length),
        );
        let entries = deserialize_entries(&leaf).expect("deserialise a leaf directory");
        let first_offset = entries.first().map(|e| e.offset).unwrap_or(0);

        for entry in entries.iter().filter(|e| e.run_length > 0).take(16) {
            let (z, x, y) = tileid_to_zxy(entry.tile_id).expect("a committed id is in range");
            let got = reader
                .get_tile(z, x, y)
                .unwrap_or_else(|e| panic!("({z}, {x}, {y}): {e}"))
                .unwrap_or_else(|| panic!("({z}, {x}, {y}) is absent"));
            let right = slice_at(&archive, data_offset + entry.offset, entry.length);
            assert_eq!(
                sha256_hex(&got),
                sha256_hex(right),
                "({z}, {x}, {y}) did not come back from tile_data_offset + \
                 entry.offset"
            );
            checked += 1;

            // Every base a writer or a reader could plausibly use instead. Each
            // has to land on different bytes, or the fixture is not evidence
            // about which one we use.
            for (label, base) in [
                ("leaf_directory_offset", leaf_offset),
                ("the leaf's own start", leaf_offset + pointer.offset),
                ("the leaf's first entry offset", data_offset + first_offset),
                ("root_offset", root_offset),
            ] {
                if base == data_offset {
                    // The leaf's first entry is at offset 0 in some leaves, and
                    // then that candidate *is* the right base rather than a
                    // wrong one. Not evidence either way, so not counted.
                    continue;
                }
                let Some(wrong) = try_slice_at(&archive, base + entry.offset, entry.length) else {
                    continue;
                };
                assert_ne!(
                    sha256_hex(wrong),
                    sha256_hex(right),
                    "({z}, {x}, {y}) reads the same whether the base is \
                     tile_data_offset or {label}, so this fixture cannot tell \
                     those two apart and the assertion above is satisfied by a \
                     reader with the wrong one"
                );
                discriminating += 1;
            }
        }
    }

    assert!(
        checked >= 32,
        "only {checked} entries behind a leaf pointer were resolved"
    );
    assert!(
        discriminating >= checked * 2,
        "only {discriminating} base comparisons over {checked} entries could \
         tell two bases apart"
    );
}

/// A gzip section of the archive, decompressed. Both directories in these
/// goldens are gzip, which `header.json` records and the header test pins.
fn decompress_section(archive: &[u8], offset: u64, length: u64) -> Vec<u8> {
    let at = usize::try_from(offset).expect("a committed offset fits in a usize");
    let len = usize::try_from(length).expect("a committed length fits in a usize");
    Compression::Gzip
        .decompress(&archive[at..at + len], 1 << 26)
        .expect("a committed directory section decompresses")
}

fn slice_at(archive: &[u8], offset: u64, length: u32) -> &[u8] {
    try_slice_at(archive, offset, length).expect("a committed entry is inside the archive")
}

/// The same, returning `None` when the range falls outside the file, which is
/// what a wrong base does near the ends.
fn try_slice_at(archive: &[u8], offset: u64, length: u32) -> Option<&[u8]> {
    let at = usize::try_from(offset).ok()?;
    let end = at.checked_add(length as usize)?;
    if end > archive.len() {
        return None;
    }
    Some(&archive[at..end])
}

/// The duplicate golden has to work through both duplicate shapes.
///
/// Runs, where one entry covers several consecutive tile ids, and repeated
/// offsets that are not adjacent, where several entries point at one blob. A
/// reader that implements only the first returns the preceding tile's bytes for
/// the second and looks fine on every other fixture.
#[test]
fn the_duplicate_golden_serves_runs_as_well_as_direct_entries() {
    let tiles = tiles_vectors();
    let name = "dupes-z0z3.pmtiles";
    let digest = TILES_VECTOR_GOLDENS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, d)| *d)
        .expect("the dupes golden is in the fixture list");
    let reader = Reader::try_open(golden_path(name, digest)).expect("open the dupes golden");

    let rows = tile_rows(&tiles, name, "hits");
    let run_served = rows.iter().filter(|r| !r.directly_addressed).count();
    let directly = rows.iter().filter(|r| r.directly_addressed).count();
    assert!(
        run_served > 0,
        "no row in {name} is served by a run, so the run path is not exercised \
         here at all"
    );
    assert!(
        directly > 0,
        "every row in {name} is served by a run, so the direct-entry path is \
         not exercised here"
    );

    for row in &rows {
        let got = reader
            .get_tile(row.z, row.x, row.y)
            .unwrap_or_else(|e| panic!("({}, {}, {}): {e}", row.z, row.x, row.y))
            .unwrap_or_else(|| panic!("({}, {}, {}) is absent", row.z, row.x, row.y));
        assert_eq!(
            sha256_hex(&got),
            row.sha256,
            "({}, {}, {}) came back with the wrong bytes; this row is {}",
            row.z,
            row.x,
            row.y,
            if row.directly_addressed {
                "addressed by its own entry"
            } else {
                "served by a run"
            }
        );
    }
}

/// Our header decoding has to match the reference's, field for field.
///
/// `pmtiles.DeserializeHeader` read the same 127 bytes we do, and every number
/// it produced is recorded. This is where an endianness mistake or an offset
/// off by a field width shows up as itself rather than as a confusing failure
/// three layers up.
///
/// # Every field, because naming them is how this stopped covering twelve
///
/// It used to compare thirteen of the twenty-five fields the reference decodes,
/// all of them offsets, lengths, counts and zooms. The twelve it left out were
/// the six `i32` positions, the two compressions, the tile type, the clustered
/// flag and the spec version, and they are exactly the ones a position bug
/// lives in: switching our `i32` reads to big-endian leaves the entire PMTiles
/// suite green, because nothing here ever read a bound or a centre back. So the
/// list below is checked against the reference's own field names and the test
/// fails if it is not covering all of them, rather than growing silently
/// staler.
#[test]
fn our_header_decoding_matches_the_reference_field_for_field() {
    let headers = header_vectors();
    let mut compared = 0usize;

    for (name, digest) in GOLDENS {
        let reader = Reader::try_open(golden_path(name, digest))
            .unwrap_or_else(|e| panic!("open {name}: {e}"));
        let header = reader.header();
        let want = |field: &str| oracle_header_u64(&headers, name, field);
        let want_signed = |field: &str| oracle_header_i64(&headers, name, field);
        let (min_lon, min_lat, max_lon, max_lat) = header.bounds_degrees();
        let (center_lon, center_lat) = header.center_degrees();

        let mut covered: Vec<&str> = Vec::new();
        for (field, ours) in [
            ("root_offset", header.root_offset),
            ("root_length", header.root_length),
            ("metadata_offset", header.metadata_offset),
            ("metadata_length", header.metadata_length),
            ("leaf_directory_offset", header.leaf_directories_offset),
            ("leaf_directory_length", header.leaf_directories_length),
            ("tile_data_offset", header.tile_data_offset),
            ("tile_data_length", header.tile_data_length),
            ("addressed_tiles_count", header.addressed_tiles_count),
            ("tile_entries_count", header.tile_entries_count),
            ("tile_contents_count", header.tile_contents_count),
            ("min_zoom", u64::from(header.min_zoom)),
            ("max_zoom", u64::from(header.max_zoom)),
            ("center_zoom", u64::from(header.center_zoom)),
            ("spec_version", u64::from(SPEC_VERSION)),
            ("clustered", u64::from(header.clustered)),
            (
                "internal_compression",
                u64::from(header.internal_compression.to_byte()),
            ),
            (
                "tile_compression",
                u64::from(header.tile_compression.to_byte()),
            ),
            ("tile_type", u64::from(header.tile_type.to_byte())),
        ] {
            assert_eq!(
                ours,
                want(field),
                "{name}: we decode {field} as {ours}, go-pmtiles decoded it as {}",
                want(field)
            );
            covered.push(field);
            compared += 1;
        }

        // The six positions are stored as `i32` hundred-nanodegrees. Comparing
        // the accessors' degrees back at that scale is exact, and it is the
        // accessor rather than the raw field that a caller ever sees.
        for (field, ours) in [
            ("min_lon_e7", min_lon),
            ("min_lat_e7", min_lat),
            ("max_lon_e7", max_lon),
            ("max_lat_e7", max_lat),
            ("center_lon_e7", center_lon),
            ("center_lat_e7", center_lat),
        ] {
            let ours_e7 = (ours * 1e7).round() as i64;
            assert_eq!(
                ours_e7,
                want_signed(field),
                "{name}: our accessor reports {field} as {ours} degrees, which \
                 is {ours_e7} at e7, and go-pmtiles decoded {}",
                want_signed(field)
            );
            covered.push(field);
            compared += 1;
        }

        let reported = oracle_header_fields(&headers, name);
        let missing: Vec<&String> = reported
            .iter()
            .filter(|f| !covered.contains(&f.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "{name}: go-pmtiles decoded {} header fields and this compares {}. \
             Not compared: {missing:?}. Every one of those is a field a decoding \
             mistake can live in unobserved, which is how a big-endian i32 left \
             the whole suite green.",
            reported.len(),
            covered.len(),
        );
    }

    assert!(
        compared >= 75,
        "only {compared} header fields were compared across the goldens, so the \
         sweep is dropping rows"
    );
}

/// The accessors the format defines on top of the raw fields have to agree
/// with the reference's own rendering of them.
///
/// `header.json` carries what `DeserializeHeader` returned. This reads what
/// `pmtiles show` prints, which is the reference putting those numbers through
/// its own accessors, so the two files disagree if we match the fields and
/// still compute bounds, the centre or the leaf flag from them differently.
///
/// `header-mvt-z2z4` is the fixture that makes this able to fail. In the other
/// goldens the bounds are the symmetric whole world, the centre is `0,0`, the
/// tile type and the two compressions sit at neighbouring byte values and the
/// minimum zoom is 0, so half a dozen distinct mistakes are all the identity.
#[test]
fn our_header_accessors_match_what_the_reference_prints() {
    let shown = show_vectors();
    let mut compared = 0usize;

    for (name, digest) in GOLDENS {
        let reader = Reader::try_open(golden_path(name, digest))
            .unwrap_or_else(|e| panic!("open {name}: {e}"));
        let header = reader.header();
        let want = |field: &str| oracle_shown(&shown, name, field);

        let (min_lon, min_lat, max_lon, max_lat) = header.bounds_degrees();
        let bounds = oracle_shown_numbers(&shown, name, "bounds");
        assert_eq!(bounds.len(), 4, "{name}: the reference printed {bounds:?}");
        for (label, ours, theirs) in [
            ("west", min_lon, bounds[0]),
            ("south", min_lat, bounds[1]),
            ("east", max_lon, bounds[2]),
            ("north", max_lat, bounds[3]),
        ] {
            assert!(
                (ours - theirs).abs() < 1e-6,
                "{name}: our {label} bound is {ours}, `pmtiles show` printed \
                 {theirs}"
            );
            compared += 1;
        }

        let (center_lon, center_lat) = header.center_degrees();
        let centre = oracle_shown_numbers(&shown, name, "center");
        assert_eq!(centre.len(), 2, "{name}: the reference printed {centre:?}");
        assert!(
            (center_lon - centre[0]).abs() < 1e-6 && (center_lat - centre[1]).abs() < 1e-6,
            "{name}: our centre is ({center_lon}, {center_lat}), `pmtiles show` \
             printed ({}, {})",
            centre[0],
            centre[1]
        );
        compared += 2;

        for (label, ours, theirs) in [
            (
                "tile type",
                format!("{:?}", header.tile_type).to_lowercase(),
                want("tile type"),
            ),
            (
                "internal compression",
                format!("{:?}", header.internal_compression).to_lowercase(),
                want("internal compression"),
            ),
            (
                "tile compression",
                format!("{:?}", header.tile_compression).to_lowercase(),
                want("tile compression"),
            ),
            ("clustered", header.clustered.to_string(), want("clustered")),
            ("min zoom", header.min_zoom.to_string(), want("min zoom")),
            ("max zoom", header.max_zoom.to_string(), want("max zoom")),
            (
                "center zoom",
                header.center_zoom.to_string(),
                want("center zoom"),
            ),
        ] {
            assert_eq!(
                ours, theirs,
                "{name}: we report {label} as {ours:?}, `pmtiles show` printed \
                 {theirs:?}"
            );
            compared += 1;
        }

        // `has_leaves` is ours alone, so the reference's leaf length is the
        // control for it.
        let leaf_length = oracle_header_u64(&header_vectors(), name, "leaf_directory_length");
        assert_eq!(
            header.has_leaves(),
            leaf_length > 0,
            "{name}: has_leaves() says {} and the reference reports a \
             {leaf_length}-byte leaf section",
            header.has_leaves()
        );
        compared += 1;
    }

    assert!(
        compared >= 70,
        "only {compared} accessor readings were compared, so the sweep is \
         dropping rows"
    );
}

// ---------------------------------------------------------------------------
// The external-binary half
// ---------------------------------------------------------------------------

/// Whether the caller demands the external binary actually run.
///
/// The dedicated CI job sets this so a runner that failed to lay `pmtiles` down
/// hard-fails rather than skipping every comparison and reporting a pass. Any
/// other value, or an unset variable, means "skip cleanly if it is absent",
/// which is the dev-machine behaviour.
fn require_go_pmtiles() -> bool {
    std::env::var("VIPRS_REQUIRE_GO_PMTILES").is_ok_and(|v| v == "1")
}

/// The pinned `pmtiles` binary, or `None` when it is not reachable.
///
/// `$GO_PMTILES_BIN` wins so a container mirror can point at a mounted copy;
/// otherwise `pmtiles` on `PATH`, which is where the CI job installs it.
///
/// # The false-green guard
///
/// This is the half of the skip the issue did not specify. `cli_available()` in
/// `tests/common/cli.rs` is only safe because `VIPRS_REQUIRE_CLI=1` converts its
/// skip into a panic in CI, and the pdfium block this job's download is modelled
/// on has no such guard to copy. Without it, a job whose download silently
/// failed reports green for exactly the reason a laptop does.
/// # And the version is checked wherever the binary came from
///
/// A `pmtiles` that runs is not the same thing as the `pmtiles` the fixtures
/// came out of. Verifying our archives with a different build and reporting that
/// as interop would be the pleasant kind of wrong answer, so the release the
/// vector files name has to be the release that answers `pmtiles version`,
/// whichever route found it.
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

    let probe = Command::new(&candidate).arg("version").output();
    let ran = match probe {
        Ok(out) if out.status.success() => out,
        _ => {
            assert!(
                !require_go_pmtiles(),
                "VIPRS_REQUIRE_GO_PMTILES=1 but `{} version` did not run. This \
                 job would SKIP every interop comparison and report a false \
                 green. Check that the download step ran and that `sha256sum -c` \
                 accepted it.",
                candidate.display()
            );
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
        "`{} version` reports {version:?}, which is not the pinned \
         {ORACLE_RELEASE_TAG} that tests/fixtures/pmtiles/ came out of. \
         Verifying our archives with a different build of the tool is not the \
         check this job claims to be.",
        candidate.display()
    );
    Some(candidate)
}

/// Generate one pyramid twice: into a directory tree and into an archive.
///
/// Both come back, because the directory tree is what the external comparison
/// checks the archive against. The tree is addressed by literal path, so
/// nothing about it goes through the tile-id function, which is the whole
/// reason it is the right side of that comparison.
fn libviprs_pyramid(
    dir: &Path,
    width: u32,
    height: u32,
    tile: u32,
) -> (PathBuf, PathBuf, PyramidPlan) {
    let plan = PyramidPlanner::new(width, height, tile, 0, Layout::Xyz)
        .expect("a valid ZXY plan")
        .plan();
    let src = canonical_raster_scaled(width, height);

    let tree = dir.join("tiles");
    let fs_sink = FsSink::new(&tree, plan.clone()).with_format(TileFormat::Png);
    EngineBuilder::new(&src, plan.clone(), &fs_sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .expect("generate into FsSink");

    let archive = dir.join("libviprs.pmtiles");
    let sink = PmTilesSink::builder(&archive)
        .plan(plan.clone())
        .tile_format(TileFormat::Png)
        .build()
        .expect("build a PmTilesSink");
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .expect("generate into PmTilesSink");
    assert!(
        sink.published_header().is_some(),
        "the sink never published a header, so there is no archive to hand to \
         go-pmtiles"
    );

    (archive, tree, plan)
}

/// `pmtiles tile <archive> z x y`, as bytes. Empty means the reference could
/// not find it, which is how it reports absence.
fn oracle_tile(bin: &Path, archive: &Path, z: u8, x: u32, y: u32) -> Vec<u8> {
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

/// `pmtiles verify` has to accept what we write.
///
/// Necessary and nowhere near sufficient. It never compares `offset + length`
/// against the archive size, which is how it walks past a root offset of 999999
/// in an 1878-byte file, so exit 0 on its own says very little. The tile
/// comparisons are what say the bytes are in the right place.
fn assert_oracle_verifies(bin: &Path, archive: &Path) {
    let verify = Command::new(bin)
        .arg("verify")
        .arg(archive)
        .output()
        .expect("run pmtiles verify");
    assert!(
        verify.status.success(),
        "`pmtiles verify` exited {} on an archive libviprs wrote\n--- stdout \
         ---\n{}\n--- stderr ---\n{}",
        verify.status,
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr),
    );
}

/// The reference has to find every tile where the directory backend put it.
///
/// # Why this compares against the directory tree and not our own reader
///
/// It used to compare `pmtiles tile z x y` against `reader.get_tile(z, x, y)`,
/// and that comparison cannot fail for the reason it exists. Our reader and our
/// writer share one tile-id function, so a misreading of the spec moves both
/// sides by the same permutation and cancels out. The directory backend
/// addresses tiles by literal path, `z/x/y.png`, with no id arithmetic
/// anywhere, so it is the only thing in this repo that can hold the archive to
/// the coordinate the planner actually named.
///
/// # Why every cell, and not a handful
///
/// The handful it used to probe were `(0,0)`, `(1,0)`, `(0,1)` and the far
/// corner of a 4x4 level, and every one of those is a fixed point of every
/// plausible tile-id mistake. Ten of the sixteen cells on that level would have
/// caught a dropped rotation and none of the four was one of them, so three
/// separate mutations of the writer left this test green with `pmtiles verify`
/// reporting exit 0 over a transposed archive. The plan is small enough to
/// sweep whole, so it gets swept whole, and
/// [`assert_probes_discriminate`](pmtiles_support::assert_probes_discriminate)
/// refuses the sweep if the cells in it could not tell the difference.
///
/// The source is deliberately not square. On a square level a transposition is
/// a bijection onto itself and half the coordinates land on a tile that happens
/// to be there.
#[test]
fn go_pmtiles_finds_every_tile_where_the_directory_backend_put_it() {
    let Some(bin) = go_pmtiles_bin() else {
        eprintln!(
            "skipping: no pinned go-pmtiles reachable. Set GO_PMTILES_BIN, or \
             VIPRS_REQUIRE_GO_PMTILES=1 to make this a failure."
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let (archive, tree, plan) = libviprs_pyramid(dir.path(), 512, 384, 128);
    assert_oracle_verifies(&bin, &archive);

    let top = plan.levels.last().expect("a plan has levels");
    let z = u8::try_from(top.level).expect("a test plan stays inside u8 zooms");
    let probes: Vec<(u32, u32)> = plan
        .tile_coords()
        .filter(|c| c.level == top.level)
        .map(|c| (c.col, c.row))
        .collect();
    // The grid the permutation is computed over is the whole zoom level, which
    // is wider than the plan where the source is not a full world.
    assert_probes_discriminate(z, 1u32 << z, &probes, "the external go-pmtiles comparison");

    let mut compared = 0usize;
    for coord in plan.tile_coords() {
        let rel = plan
            .tile_path(coord, "png")
            .expect("the plan has a path for its own coordinate");
        let on_disk = std::fs::read(tree.join(&rel))
            .unwrap_or_else(|e| panic!("the directory backend has no {rel}: {e}"));
        let z = u8::try_from(coord.level).expect("a test plan stays inside u8 zooms");
        let from_oracle = oracle_tile(&bin, &archive, z, coord.col, coord.row);
        assert!(
            !from_oracle.is_empty(),
            "`pmtiles tile {z} {} {}` wrote nothing, which is how go-pmtiles \
             reports a tile it cannot find. The directory backend put {rel} \
             there.",
            coord.col,
            coord.row
        );
        assert_eq!(
            sha256_hex(&from_oracle),
            sha256_hex(&on_disk),
            "go-pmtiles read different bytes out of the archive at ({z}, {}, \
             {}) than the directory backend wrote to {rel}. The two backends \
             ran off one plan, so this is the archive addressing that tile \
             somewhere other than where the planner named it.",
            coord.col,
            coord.row
        );
        compared += 1;
    }

    assert_eq!(
        compared as u64,
        plan.total_tile_count(),
        "only {compared} of {} coordinates reached the reference",
        plan.total_tile_count()
    );
}

/// The reference has to read a libviprs archive that spilled into leaves.
///
/// Nothing anywhere had ever asked it to. Every interop archive in this repo
/// and in the core crate is root-only, and the leaf-entry base is the one
/// mistake the epic itself flagged as invisible to a shared misreading: our
/// reader and our writer would agree on a wrong base and the archive would look
/// perfect from inside. An outside implementation walking our leaves is the
/// only check that closes it.
///
/// The writer spills when the directory will not fit 16384 entries or 16257
/// bytes, so the plan is sized past the entry ceiling with small tiles rather
/// than a large raster. Every cell of one middle zoom is swept, plus the first
/// and last coordinate of every level, which is where an off-by-one in the leaf
/// boundary lands.
#[test]
fn go_pmtiles_reads_a_libviprs_archive_that_spilled_into_leaves() {
    let Some(bin) = go_pmtiles_bin() else {
        eprintln!(
            "skipping: no pinned go-pmtiles reachable. Set GO_PMTILES_BIN, or \
             VIPRS_REQUIRE_GO_PMTILES=1 to make this a failure."
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let (archive, tree, plan) = libviprs_pyramid(dir.path(), 4096, 3072, 16);

    let reader = Reader::try_open(&archive).expect("open the archive we just wrote");
    assert!(
        reader.header().has_leaves(),
        "this plan produced {} entries in a root-only archive, so the leaf path \
         it exists to exercise was never taken. The writer spills above 16384 \
         entries; raise the plan until it does.",
        reader.header().tile_entries_count
    );
    assert_oracle_verifies(&bin, &archive);

    // One whole zoom, which is what makes this able to fail: a wrong leaf base
    // moves every entry behind a leaf pointer at once, and a wrong convention
    // moves most cells of any level wide enough to have leaves behind it.
    let sweep_level = plan
        .levels
        .iter()
        .find(|l| l.cols >= 16 && l.rows >= 16)
        .expect("a plan this size has a level at least 16 wide");
    let z = u8::try_from(sweep_level.level).expect("a test plan stays inside u8 zooms");
    let probes: Vec<(u32, u32)> = plan
        .tile_coords()
        .filter(|c| c.level == sweep_level.level)
        .map(|c| (c.col, c.row))
        .collect();
    assert_probes_discriminate(z, 1u32 << z, &probes, "the leaf-directory sweep");

    let mut edges: Vec<libviprs::planner::TileCoord> = Vec::new();
    for level in &plan.levels {
        let mut on_level: Vec<_> = plan
            .tile_coords()
            .filter(|c| c.level == level.level)
            .collect();
        on_level.sort_by_key(|c| (c.row, c.col));
        if let Some(first) = on_level.first() {
            edges.push(*first);
        }
        if let Some(last) = on_level.last() {
            edges.push(*last);
        }
    }

    let mut compared = 0usize;
    let mut leaf_served = 0usize;
    for coord in plan
        .tile_coords()
        .filter(|c| c.level == sweep_level.level)
        .chain(edges)
    {
        let rel = plan
            .tile_path(coord, "png")
            .expect("the plan has a path for its own coordinate");
        let on_disk = std::fs::read(tree.join(&rel))
            .unwrap_or_else(|e| panic!("the directory backend has no {rel}: {e}"));
        let z = u8::try_from(coord.level).expect("a test plan stays inside u8 zooms");
        let from_oracle = oracle_tile(&bin, &archive, z, coord.col, coord.row);
        assert_eq!(
            sha256_hex(&from_oracle),
            sha256_hex(&on_disk),
            "go-pmtiles walked our leaf directories to ({z}, {}, {}) and came \
             back with bytes the directory backend did not write to {rel}",
            coord.col,
            coord.row
        );
        compared += 1;
        if coord.level == sweep_level.level {
            leaf_served += 1;
        }
    }

    assert!(
        compared > 256,
        "only {compared} coordinates reached the reference, which is not a \
         sweep of anything"
    );
    assert!(
        leaf_served >= 256,
        "only {leaf_served} cells of zoom {z} were swept"
    );
}
