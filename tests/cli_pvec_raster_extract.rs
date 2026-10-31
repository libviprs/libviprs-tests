//! `viprs pmtiles extract` on the PMTiles export of a `.pvec` raster yields the producer's tile
//! bytes (libviprs-tests#305, the CLI half of the interchange claim).
//!
//! Drives the real `viprs` binary, so it runs in the `cli-differential` job with
//! `VIPRS_REQUIRE_CLI=1`, which turns the skip into a failure. The commands the CLI gained for
//! rasters (libviprs/libviprs-cli#130) have their own suite, `cli_pvec_raster.rs`. Needs
//! `--features pvec-raster`.

#![cfg(feature = "pvec-raster")]

mod common;
#[path = "common/pvec_raster.rs"]
mod fx;

use common::cli::{cli_available, run_viprs_ok};

#[test]
fn viprs_pmtiles_extract_yields_the_same_tile_bytes() {
    if !cli_available() {
        eprintln!(
            "skipping: no libviprs-cli sibling (set VIPRS_REQUIRE_CLI=1 to make this a failure)"
        );
        return;
    }
    let (pyramid, pvec) = fx::interchange_pvec();
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("export.pmtiles");
    std::fs::write(&archive, fx::export_raster_0(&pvec)).unwrap();
    let tree = dir.path().join("tiles");
    run_viprs_ok(&[
        "pmtiles",
        "extract",
        archive.to_str().unwrap(),
        tree.to_str().unwrap(),
    ]);
    for (z, x, y, planned) in pyramid.tiles() {
        let file = tree.join(format!("{z}/{x}/{y}.png"));
        let got = std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        assert_eq!(
            fx::sha256_hex(&got),
            fx::sha256_hex(planned),
            "tile {z}/{x}/{y}"
        );
    }
}
