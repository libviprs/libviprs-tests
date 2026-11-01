//! A `.pvec` raster out to PMTiles through pvec-interchange (libviprs/pvec#222), and the tiles
//! read back with libviprs's own reader (libviprs-tests#305).
//!
//! The passthrough claim is that the archive holds the tile bytes the producer wrote and nothing
//! else. This file checks it with libviprs's `pmtiles::Reader`, which shares no code with the
//! writer. The two readers that need an external tool live where that tool is laid down:
//! `viprs pmtiles extract` in `cli_pvec_raster_extract.rs` (the CLI job) and go-pmtiles `show`,
//! for both the `vnd.pvec` key of the export and the `vnd.libviprs` GeoTransform key of an archive
//! libviprs wrote (option D' of ADR 0018), in `pmtiles_interop.rs`, which owns the pinned oracle.
//! Needs `--features pvec-raster`.

#![cfg(feature = "pvec-raster")]

#[path = "common/pvec_raster.rs"]
mod fx;

use libviprs::pmtiles::Reader;
use pvec_interchange::pmtiles::{RasterReadLimits, RasterTileType, read_raster_archive};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn the_export_extracts_to_the_same_tile_bytes_digest_equal_per_tile() {
    let (pyramid, pvec) = fx::interchange_pvec();
    let archive = fx::export_raster_0(&pvec);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.pmtiles");
    std::fs::write(&path, &archive).unwrap();

    let reader = Reader::try_open(&path).expect("libviprs opens the export");
    let mut n = 0;
    for (z, x, y, planned) in pyramid.tiles() {
        let got = reader
            .get_tile(u8::try_from(z).unwrap(), x, y)
            .expect("lookup")
            .unwrap_or_else(|| panic!("tile {z}/{x}/{y} is missing from the export"));
        assert_eq!(
            fx::sha256_hex(&got),
            fx::sha256_hex(planned),
            "tile {z}/{x}/{y}"
        );
        n += 1;
    }
    assert!(n > 20, "{n} tiles");

    // Nothing else is in the archive: the blank quarter's tiles were not stored, and are not there.
    let top = pyramid.levels - 1;
    assert_eq!(reader.get_tile(top, 3, 2).expect("lookup"), None);
    let parsed = read_raster_archive(&archive, &RasterReadLimits::default()).expect("reads back");
    assert_eq!(parsed.tile_type, RasterTileType::Png);
    assert_eq!(
        parsed.tiles.len(),
        n,
        "the archive holds exactly the stored tiles"
    );
    assert_eq!(
        parsed.attribution.as_deref(),
        Some("Scan of the ground floor, 1987")
    );
}

#[test]
fn a_tile_with_one_flipped_byte_is_not_the_exported_tile() {
    // The control for the digest comparison above.
    let (pyramid, pvec) = fx::interchange_pvec();
    let parsed =
        read_raster_archive(&fx::export_raster_0(&pvec), &RasterReadLimits::default()).unwrap();
    let t = &parsed.tiles[0];
    let planned = pyramid
        .tile(u32::from(t.z), t.x, t.y)
        .expect("planned tile");
    assert_eq!(fx::sha256_hex(&t.bytes), fx::sha256_hex(planned));
    let mut flipped = t.bytes.clone();
    flipped[t.bytes.len() / 2] ^= 1;
    assert_ne!(fx::sha256_hex(&flipped), fx::sha256_hex(planned));
}
