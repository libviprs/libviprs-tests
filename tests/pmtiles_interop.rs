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
    GOLDENS, LEAVES_GOLDEN, LEAVES_GOLDEN_SHA256, ORACLE_RELEASE_TAG, golden_path, header_vectors,
    oracle_header_u64, read_checked, sha256_hex, tile_rows, tiles_vectors, vector_archive_names,
    vector_golden_sha256,
};

use libviprs::pmtiles::Reader;
use libviprs::sink_pmtiles::PmTilesSink;
use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, PyramidPlan, PyramidPlanner, TileFormat,
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
#[test]
fn the_committed_goldens_are_the_ones_the_vectors_describe() {
    let tiles = tiles_vectors();
    let names = vector_archive_names(&tiles);
    assert_eq!(
        names.len(),
        GOLDENS.len(),
        "the vector file covers {:?} but {} archives are committed, so one of \
         them is being tested by nothing",
        names,
        GOLDENS.len()
    );
    for (name, digest) in GOLDENS {
        assert!(
            names.iter().any(|n| n == name),
            "{name} is committed but the vector file has no rows for it"
        );
        let bytes = read_checked(name, digest);
        assert_eq!(
            sha256_hex(&bytes),
            vector_golden_sha256(&tiles, name),
            "{name} on disk is not the archive the vector file was dumped from"
        );
    }
}

/// Our reader has to return, byte for byte, what `pmtiles tile` returned.
#[test]
fn libviprs_reads_every_go_pmtiles_golden() {
    let tiles = tiles_vectors();
    let mut checked = 0usize;

    for (name, digest) in GOLDENS {
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

    for (name, digest) in GOLDENS {
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

    for (name, digest) in GOLDENS {
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
/// the leaf it was found in.
///
/// The spec keys an entry's base off the **entry kind**, not off the directory
/// it was read from, and none of the three fixtures in the upstream spec
/// repository has leaf directories at all, so nothing upstream exercises this.
/// A writer and a reader that make the same wrong choice agree with each other.
///
/// What makes this cell worth having is the second assertion. Under the wrong
/// base the offsets still land inside the file, on the leaf directory region,
/// so `get_tile` returns `Some` either way and a test that checks for `Some`
/// passes on a reader that is handing back directory bytes dressed as a tile.
/// So this compares the payload, and then shows what the wrong base would have
/// returned instead, which is the proof that the comparison can fail.
#[test]
fn leaf_entries_resolve_against_tile_data_not_the_leaf_start() {
    let headers = header_vectors();
    let leaf_offset = oracle_header_u64(&headers, LEAVES_GOLDEN, "leaf_directory_offset");
    let leaf_length = oracle_header_u64(&headers, LEAVES_GOLDEN, "leaf_directory_length");
    let data_offset = oracle_header_u64(&headers, LEAVES_GOLDEN, "tile_data_offset");

    assert!(
        leaf_length > 0,
        "{LEAVES_GOLDEN} has no leaf directories, so this cell is about a code \
         path the fixture never reaches"
    );
    assert_ne!(
        leaf_offset, data_offset,
        "the two candidate bases are the same number in this fixture, so it \
         cannot tell them apart"
    );

    let archive = read_checked(LEAVES_GOLDEN, LEAVES_GOLDEN_SHA256);
    let reader = Reader::try_open(golden_path(LEAVES_GOLDEN, LEAVES_GOLDEN_SHA256))
        .expect("open the leaf golden");

    // Our own reader has to agree with the reference about where the sections
    // are before anything below means anything.
    assert_eq!(reader.header().leaf_directories_offset, leaf_offset);
    assert_eq!(reader.header().leaf_directories_length, leaf_length);
    assert_eq!(reader.header().tile_data_offset, data_offset);

    let tiles = tiles_vectors();
    let mut discriminating = 0usize;

    for row in tile_rows(&tiles, LEAVES_GOLDEN, "hits") {
        let got = reader
            .get_tile(row.z, row.x, row.y)
            .unwrap_or_else(|e| panic!("({}, {}, {}): {e}", row.z, row.x, row.y))
            .unwrap_or_else(|| panic!("({}, {}, {}) is absent", row.z, row.x, row.y));
        assert_eq!(
            sha256_hex(&got),
            row.sha256,
            "({}, {}, {}) came back with the wrong bytes",
            row.z,
            row.x,
            row.y
        );

        let Some(entry_offset) = row.directory_entry_offset else {
            continue;
        };
        let wrong = (leaf_offset + entry_offset) as usize;
        let end = wrong + row.length;
        if end > archive.len() {
            continue;
        }
        // The wrong base lands inside the file, so a reader using it returns
        // bytes rather than an error. Show that they are different bytes.
        assert_ne!(
            sha256_hex(&archive[wrong..end]),
            row.sha256,
            "({}, {}, {}) reads the same under both bases, so this fixture \
             cannot discriminate between them and the assertion above is \
             satisfied by a reader with the wrong one",
            row.z,
            row.x,
            row.y
        );
        discriminating += 1;
    }

    assert!(
        discriminating > 0,
        "no row in {LEAVES_GOLDEN} could tell the two bases apart, so nothing \
         here is about the leaf offset base"
    );
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
    let digest = GOLDENS
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
#[test]
fn our_header_decoding_matches_the_reference_field_for_field() {
    let headers = header_vectors();
    let mut compared = 0usize;

    for (name, digest) in GOLDENS {
        let reader = Reader::try_open(golden_path(name, digest))
            .unwrap_or_else(|e| panic!("open {name}: {e}"));
        let header = reader.header();
        let want = |field: &str| oracle_header_u64(&headers, name, field);

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
        ] {
            assert_eq!(
                ours,
                want(field),
                "{name}: we decode {field} as {ours}, go-pmtiles decoded it as {}",
                want(field)
            );
            compared += 1;
        }
    }

    assert!(
        compared >= 39,
        "only {compared} header fields were compared across three archives, so \
         the sweep is dropping rows"
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

/// Write a small pyramid into an archive and hand back its path.
fn libviprs_archive(dir: &Path) -> (PathBuf, PyramidPlan) {
    let plan = PyramidPlanner::new(512, 512, 128, 0, Layout::Xyz)
        .expect("a valid ZXY plan")
        .plan();
    let src = canonical_raster_scaled(512, 512);
    let path = dir.join("libviprs.pmtiles");
    let sink = PmTilesSink::builder(&path)
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
    (path, plan)
}

/// The reference has to accept what we write, and read the same bytes out of it.
///
/// `pmtiles verify` walks the directories and checks the invariants it knows
/// about. It is not a complete check (it never compares `offset + length`
/// against the archive size, which is how it walks past a root offset of 999999
/// in an 1878-byte file), so exit 0 is necessary and not sufficient. The tile
/// comparison after it is the part that says the bytes are in the right place.
#[test]
fn go_pmtiles_accepts_and_reads_an_archive_libviprs_wrote() {
    let Some(bin) = go_pmtiles_bin() else {
        eprintln!(
            "skipping: no pinned go-pmtiles reachable. Set GO_PMTILES_BIN, or \
             VIPRS_REQUIRE_GO_PMTILES=1 to make this a failure."
        );
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let (archive, plan) = libviprs_archive(dir.path());

    let verify = Command::new(&bin)
        .arg("verify")
        .arg(&archive)
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

    let reader = Reader::try_open(&archive).expect("open our own archive");
    let top = plan.levels.last().expect("a plan has levels");
    let z = u8::try_from(top.level).expect("a test plan stays inside u8 zooms");

    let mut compared = 0usize;
    for (x, y) in [(0, 0), (1, 0), (0, 1), (top.cols - 1, top.rows - 1)] {
        let out = Command::new(&bin)
            .args(["tile", "--quiet"])
            .arg(&archive)
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
        assert!(
            !out.stdout.is_empty(),
            "`pmtiles tile {z} {x} {y}` wrote nothing, which is how go-pmtiles \
             reports a tile it cannot find. libviprs wrote one there."
        );
        let ours = reader
            .get_tile(z, x, y)
            .expect("read our own archive")
            .unwrap_or_else(|| panic!("we have no tile at ({z}, {x}, {y})"));
        assert_eq!(
            sha256_hex(&out.stdout),
            sha256_hex(&ours),
            "go-pmtiles and libviprs read different bytes for ({z}, {x}, {y}) \
             out of the same archive"
        );
        compared += 1;
    }
    assert_eq!(compared, 4, "not every coordinate was compared");
}
