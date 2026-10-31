//! A file with a raster in it, opened by a 1.0.0 reader (libviprs-tests#305; the compat matrix row
//! "old-reader-raster-file-open-degraded" of libviprs/pvec#216).
//!
//! ADR 0018 makes capability 21 optional, so a reader that predates rasters must open the file
//! `OpenDegraded`: capability 21 listed as not implemented, the kind 9 and 10 blocks skipped by
//! their directory entries (their payload never fetched), and the geometry bit-identical to the
//! file the same drawing makes without the raster. The 1.0.0 reader is pvec's own container reader
//! built with the capability set and version of the matrix's `s_old` (geometry 1 to 13, no component
//! streams), so this is the reader the row names and not one imitated here. Needs
//! `--features pvec-raster`.

#![cfg(feature = "pvec-raster")]

#[path = "common/pvec_raster.rs"]
mod fx;

use libviprs::raster_pvec::container::write_rasters;
use libviprs::raster_pvec::pyramid::{
    RasterPlacement, RasterPyramidOptions, produce_raster_pyramid,
};
use pvec_capabilities::{CapabilityId, Version};
use pvec_container::directory::BlockKind;
use pvec_container::negotiate::{ReaderCapabilities, SkipReason};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;

/// The reader of the matrix's `s_old` row: capabilities 14 to 21 removed, version 1.0.0.
fn reader_1_0_0() -> ReaderCapabilities {
    let mut r = (14..=21).fold(ReaderCapabilities::v1(), |r, id| {
        r.without(CapabilityId::new(id))
    });
    r.version = Version::new(1, 0, 0);
    r
}

/// The scan (blank quarter skipped) riding the drawing, as the producer writes it.
fn raster_file() -> Vec<u8> {
    let raster = fx::load("scan_gray_1537x1031.png");
    let mut options = RasterPyramidOptions::new(RasterPlacement::Explicit([
        1.0, 0.0, 0.0, -1.0, 0.0, 1031.0,
    ]));
    options.skip_blanks = true;
    let pyramid =
        produce_raster_pyramid(&raster, &fx::plan(1537, 1031, 256), &options).expect("pyramid");
    let mut w = fx::drawing_writer();
    write_rasters(&mut w, std::slice::from_ref(&pyramid)).expect("write");
    w.finish().expect("finish")
}

fn open(bytes: Vec<u8>, caps: ReaderCapabilities) -> ContainerReader<MemorySource<Vec<u8>>> {
    let cfg = ReaderConfig {
        capabilities: caps,
        ..ReaderConfig::default()
    };
    ContainerReader::open(MemorySource::new(bytes), cfg).expect("the file opens")
}

/// The geometry blocks as the matrix compares them: (kind, subtype, cell, payload).
fn geometry(r: &ContainerReader<MemorySource<Vec<u8>>>) -> Vec<(u16, u16, u32, Vec<u8>)> {
    let mut out = Vec::new();
    for (i, e) in r.entries().iter().enumerate() {
        if matches!(
            e.block_kind(),
            Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)
        ) {
            let b = r.read_block(BlockId(i as u32)).expect("geometry reads");
            out.push((e.kind, e.subtype, e.cell, b.as_bytes().to_vec()));
        }
    }
    out
}

#[test]
fn the_reader_under_test_is_a_1_0_0_reader() {
    // The version assertion of the issue's controls: this suite is only about an old reader.
    assert_eq!(reader_1_0_0().version, Version::new(1, 0, 0));
}

#[test]
fn a_1_0_0_reader_opens_the_raster_file_degraded_and_says_what_it_skipped() {
    let r = open(raster_file(), reader_1_0_0());
    let report = r.partial_open().expect("OpenDegraded has a report");

    let caps: Vec<(u32, SkipReason)> = report
        .capabilities
        .iter()
        .map(|c| (c.id.get(), c.reason))
        .collect();
    assert!(
        caps.contains(&(21, SkipReason::NotImplemented)),
        "capability 21 is listed as not implemented: {caps:?}"
    );

    // The kind 9 (index) and kind 10 (tile) blocks are skipped by their directory entries, naming 21.
    let skipped: Vec<(u16, SkipReason, Option<u32>)> = report
        .blocks
        .iter()
        .map(|b| (b.kind, b.reason, b.capability.map(|c| c.get())))
        .collect();
    assert!(!skipped.is_empty());
    for (kind, reason, cap) in &skipped {
        assert!(
            matches!(kind, 9 | 10),
            "only raster kinds are skipped: {skipped:?}"
        );
        assert_eq!(*reason, SkipReason::UnknownBlockKind);
        assert_eq!(*cap, Some(21));
    }
    assert!(skipped.iter().any(|(k, ..)| *k == 9), "the index block");
    assert!(skipped.iter().any(|(k, ..)| *k == 10), "a tile block");
    // And the file validates around them.
    r.validate().expect("the rest of the file validates");
}

#[test]
fn the_geometry_report_equals_the_raster_free_twins_byte_for_byte() {
    let with = open(raster_file(), reader_1_0_0());
    let twin = open(fx::drawing_pvec(), reader_1_0_0());
    let (a, b) = (geometry(&with), geometry(&twin));
    assert!(!a.is_empty());
    assert!(
        a == b,
        "the geometry of the raster file differs from the twin's"
    );
    // And a current reader reads the same geometry out of the raster file.
    let new = open(raster_file(), ReaderCapabilities::v1());
    assert!(geometry(&new) == b);
}

#[test]
fn a_reader_that_is_not_1_0_0_does_not_pass_as_one() {
    // The control: the current reader implements 21, so nothing is degraded for it. If this
    // were degraded, the tests above would be passing for a reason other than the version.
    let r = open(raster_file(), ReaderCapabilities::v1());
    let skipped_21 = r
        .partial_open()
        .is_some_and(|p| p.capabilities.iter().any(|c| c.id.get() == 21));
    assert!(
        !skipped_21,
        "a reader that knows capability 21 skips nothing of it"
    );
    assert_ne!(ReaderCapabilities::v1().version, Version::new(1, 0, 0));
}
