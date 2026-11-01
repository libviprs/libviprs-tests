//! The CLI half of the raster suite (libviprs-tests#305; the commands are libviprs/libviprs-cli#130).
//!
//! The CLI repo tests its own commands on a generated gradient. This runs them on the committed
//! fixtures of `tests/fixtures/pvec-raster/` and against the library of this suite: the scan goes
//! from the command line into a `.pvec`, every tile comes back out of `viprs pvec rasters --tile`
//! with pixels equal to the `--storage directory` run (tolerance 0), `viprs pmtiles to-pvec` passes
//! the tiles of an archive through byte for byte, and `viprs pvec rasters --json` equals what
//! PVec's reader says. Exit codes are not evidence: every cell asserts on an artefact.
//!
//! The binary is built with the CLI's `pvec-raster` feature (it is in `full`, not in the default
//! set). Runs in the `cli-differential` job with `VIPRS_REQUIRE_CLI=1`. Needs `--features
//! pvec-raster`.

#![cfg(feature = "pvec-raster")]

mod common;
#[path = "common/pvec_raster.rs"]
mod fx;

use std::path::{Path, PathBuf};
use std::process::Output;

use common::cli::{cli_available, run_bin, viprs_bin_for};
use pvec_facade::PvecReader;
use pvec_facade::unstable::RasterId;

fn bin() -> Option<PathBuf> {
    cli_available().then(|| viprs_bin_for(false, &["pvec-raster"]))
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[track_caller]
fn ok(bin: &Path, args: &[&str]) -> Output {
    let out = run_bin(bin, args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{args:?}:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn fixture(name: &str) -> PathBuf {
    fx::dir().join(name)
}

fn open(path: &Path) -> PvecReader {
    PvecReader::open(std::fs::read(path).unwrap()).expect("PVec's reader opens the output")
}

/// `pixel -> geo` for a north-up raster at (100.25, 199.75) with half a unit a pixel.
const PIXEL_TO_GEO: [&str; 4] = ["--geo-origin", "100,200", "--geo-scale", "0.5,-0.5"];

#[test]
fn the_scan_goes_into_a_pvec_and_every_tile_comes_back_equal_to_the_directory_run() {
    let Some(bin) = bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sheet.pvec");
    let scan = fixture("scan_gray_1537x1031.png");
    ok(
        &bin,
        &[
            "pyramid",
            s(&scan),
            s(&out),
            "--storage",
            "pvec",
            "--pvec-into",
            s(&fixture("drawing.pvec")),
            "--pixel-to-source",
            "1,0,0,-1,0,1031",
            "--drop-blanks",
        ],
    );
    let reader = open(&out);
    let p = reader.raster_placements().unwrap().remove(0);
    assert_eq!((p.width, p.height, p.tile_edge), (1537, 1031, 256));
    assert_eq!(p.pixel_to_source, [1.0, 0.0, 0.0, -1.0, 0.0, 1031.0]);

    let tree = dir.path().join("tree");
    ok(
        &bin,
        &[
            "pyramid",
            s(&scan),
            s(&tree),
            "--storage",
            "directory",
            "--layout",
            "google",
        ],
    );
    let plan = fx::plan(1537, 1031, 256);
    let (mut stored, mut left_out) = (0, 0);
    for level in &plan.levels {
        for row in 0..level.height.div_ceil(256) {
            for col in 0..level.width.div_ceil(256) {
                let want = image::open(tree.join(format!("{}/{row}/{col}.png", level.level)))
                    .unwrap()
                    .to_luma8();
                let tile = dir.path().join("t.bin");
                let _ = std::fs::remove_file(&tile);
                let args = [
                    "pvec",
                    "rasters",
                    s(&out),
                    "--tile",
                    "0",
                    &level.level.to_string(),
                    &col.to_string(),
                    &row.to_string(),
                    s(&tile),
                ];
                let got = reader
                    .raster_tile(RasterId::new(0).unwrap(), level.level, col, row)
                    .unwrap();
                match got {
                    Some(lib_bytes) => {
                        ok(&bin, &args);
                        let from_cli = std::fs::read(&tile).unwrap();
                        assert_eq!(from_cli, lib_bytes, "the command and PVec's reader agree");
                        let have = image::load_from_memory(&from_cli).unwrap().to_luma8();
                        assert!(
                            have.as_raw() == want.as_raw(),
                            "tile {}/{col}/{row} differs from the directory run",
                            level.level
                        );
                        stored += 1;
                    }
                    None => {
                        // Not stored: the directory run's tile must have been blank.
                        let first = want.as_raw()[0];
                        assert!(
                            want.as_raw().iter().all(|&v| v == first),
                            "a tile with content was dropped: {}/{col}/{row}",
                            level.level
                        );
                        left_out += 1;
                    }
                }
            }
        }
    }
    assert!(
        stored > 20 && left_out >= 6,
        "{stored} stored, {left_out} left out"
    );
}

#[test]
fn to_pvec_passes_the_tiles_of_an_archive_through_and_composes_the_stored_transform() {
    let Some(bin) = bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let scan = fixture("scan_gray_1537x1031.png");
    let pm = dir.path().join("scan.pmtiles");
    let mut args = vec!["pyramid", s(&scan), s(&pm), "--layout", "google"];
    args.extend(PIXEL_TO_GEO);
    ok(&bin, &args);
    let out = dir.path().join("sheet.pvec");
    ok(
        &bin,
        &[
            "pmtiles",
            "to-pvec",
            s(&pm),
            s(&out),
            "--pvec-into",
            s(&fixture("drawing.pvec")),
            "--source-to-geo",
            "1,0,100,0,1,100",
        ],
    );
    let reader = open(&out);
    let p = reader.raster_placements().unwrap().remove(0);
    // pixel -> geo [0.5 0 100 / 0 -0.5 200] through source -> geo shifted by (100, 100).
    assert_eq!(
        p.pixel_to_source.map(f64::to_bits),
        [0.5, 0.0, 0.0, -0.5, 0.0, 100.0].map(f64::to_bits)
    );
    let mut compared = 0;
    for level in &fx::plan(1537, 1031, 256).levels {
        for row in 0..level.height.div_ceil(256) {
            for col in 0..level.width.div_ceil(256) {
                let from_archive = ok(
                    &bin,
                    &[
                        "pmtiles",
                        "tile",
                        s(&pm),
                        &level.level.to_string(),
                        &col.to_string(),
                        &row.to_string(),
                    ],
                )
                .stdout;
                let from_pvec = reader
                    .raster_tile(RasterId::new(0).unwrap(), level.level, col, row)
                    .unwrap()
                    .unwrap_or_else(|| panic!("tile {}/{col}/{row} missing", level.level));
                assert_eq!(
                    fx::sha256_hex(&from_archive),
                    fx::sha256_hex(&from_pvec),
                    "tile {}/{col}/{row}",
                    level.level
                );
                compared += 1;
            }
        }
    }
    assert!(compared > 20);
}

#[test]
fn pvec_rasters_json_equals_what_the_library_reads() {
    let Some(bin) = bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sheet.pvec");
    ok(
        &bin,
        &[
            "pyramid",
            s(&fixture("photo_rgb_700x500.png")),
            s(&out),
            "--storage",
            "pvec",
            "--pvec-into",
            s(&fixture("drawing.pvec")),
            "--pixel-to-source",
            "0.5,0.25,0.125,-0.5,100,50",
            "--attribution",
            "Survey office 2026",
        ],
    );
    let json: serde_json::Value =
        serde_json::from_slice(&ok(&bin, &["pvec", "rasters", s(&out), "--json"]).stdout).unwrap();
    let lib = open(&out).raster_placements().unwrap();
    assert_eq!(json["rasters"].as_array().unwrap().len(), lib.len());
    let (j, l) = (&json["rasters"][0], &lib[0]);
    assert_eq!(j["width"], l.width);
    assert_eq!(j["height"], l.height);
    assert_eq!(j["tile_edge"], l.tile_edge);
    assert_eq!(j["attribution"], l.attribution.as_str());
    assert_eq!(j["pixel_to_source"], serde_json::json!(l.pixel_to_source));
    assert_eq!(j["visible_box"], serde_json::json!(l.visible_box));
    assert_eq!(j["pixel_format"], "rgb");
}

#[test]
fn the_refusals_are_by_name_and_write_nothing() {
    let Some(bin) = bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("sheet.pvec");
    let scan = fixture("scan_gray_1537x1031.png");
    let drawing = fixture("drawing.pvec");
    let base = [
        "--storage",
        "pvec",
        "--pvec-into",
        s(&drawing),
        "--pixel-to-source",
        "1,0,0,-1,0,1031",
    ];
    // What the command line alone decides is a usage error (exit 2) naming the way out.
    for (extra, needle) in [
        (vec!["--tile-size", "2048"], "256, 512 or 1024"),
        (vec!["--centre"], "centre"),
        (vec!["--format", "raw"], "raw"),
    ] {
        let mut args = vec!["pyramid", s(&scan), s(&out)];
        args.extend(base);
        args.extend(extra);
        let r = run_bin(&bin, &args);
        assert_eq!(r.status.code(), Some(2), "{args:?}");
        let err = String::from_utf8_lossy(&r.stderr);
        assert!(err.contains(needle), "{args:?}: {needle:?} not in {err}");
        assert!(!out.exists(), "a refused run wrote {}", out.display());
    }
    // A centred archive needs the input to be refused, so that one is exit 1.
    let pm = dir.path().join("centred.pmtiles");
    ok(
        &bin,
        &[
            "pyramid",
            s(&scan),
            s(&pm),
            "--layout",
            "google",
            "--centre",
        ],
    );
    let r = run_bin(
        &bin,
        &[
            "pmtiles",
            "to-pvec",
            s(&pm),
            s(&out),
            "--pvec-into",
            s(&drawing),
            "--pixel-to-source",
            "1,0,0,-1,0,1031",
        ],
    );
    assert_eq!(r.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&r.stderr).contains("centred"));
    assert!(!out.exists());
}
