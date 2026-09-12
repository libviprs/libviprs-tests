//! Large offsets and bounded reads (libviprs-tests#202).
//!
//! Two claims that are easy to state and easy to test badly.
//!
//! **Archives over 4 GiB work.** The part that silently does not is the
//! arithmetic: a `u32` somewhere truncates an offset and the read lands
//! somewhere plausible. Materialising 4 GiB to find that out is not affordable
//! in CI, on this disk, or on a laptop, so this file builds an archive that is
//! 5 GiB *wide* and a few hundred bytes *large*: a header, a root directory and
//! two payloads at their real absolute positions, with holes in between that
//! refuse to be read. A reader that truncates an offset computes a position in
//! a hole and fails by name. Against a small archive it would have computed the
//! right answer by accident.
//!
//! Saying that plainly matters, because "bounded memory on a synthetic large
//! pyramid" is how the issue words it, and a cell that materialised 4 GiB once
//! on one machine and never again would be a worse test that sounded better.
//!
//! **A read is proportional to the index plus the payload, not to the
//! archive.** That is the whole reason a single-file archive is acceptable at
//! all, and it is a measurement rather than a claim only because
//! `pmtiles::Reader` is generic over `RangeReader`: wrap the file reader in one
//! that counts, and the number is the answer.
//!
//! # The trap in the counting test
//!
//! "Total bytes read is small" passes for a reader that read nothing and
//! returned `None`. So every count assertion here sits next to an assertion
//! that the tile came back, non-empty, and with the bytes the archive holds.

use std::path::Path;

mod common;
use common::fixtures::canonical_raster_scaled;

#[path = "common/pmtiles.rs"]
mod pmtiles_support;
use pmtiles_support::{CountingRangeReader, sha256_hex, sparse_archive};

use libviprs::pmtiles::{FileRangeReader, RangeReader, Reader, zxy_to_tileid};
use libviprs::sink_pmtiles::PmTilesSink;
use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, PixelFormat, PyramidPlan, PyramidPlanner,
    RasterStripSource, TileFormat,
};

/// Four gigabytes, the boundary a `u32` offset cannot cross.
const FOUR_GIB: u64 = 1 << 32;

// ---------------------------------------------------------------------------
// Offsets past 4 GiB
// ---------------------------------------------------------------------------

/// A tile whose payload sits past 4 GiB has to be read from past 4 GiB.
#[test]
fn tile_offsets_past_four_gigabytes_resolve() {
    let first = vec![0xA5u8; 96];
    let second = vec![0x5Au8; 128];
    let base = FOUR_GIB + 1_000_000_000; // a little under 5 GiB

    let id_a = zxy_to_tileid(3, 1, 2).expect("a valid coordinate");
    let id_b = zxy_to_tileid(3, 5, 6).expect("a valid coordinate");
    let archive = sparse_archive(base, &[(id_a, first.clone()), (id_b, second.clone())])
        .expect("assemble a sparse archive");

    // Positive control on the fixture itself. If the archive is not actually
    // wider than a u32, nothing below is about 64-bit arithmetic.
    let size = archive
        .size()
        .expect("the sparse archive knows its size")
        .expect("and reports it");
    assert!(
        size > u64::from(u32::MAX),
        "the synthetic archive is {size} bytes, which fits in a u32, so a \
         reader that truncated an offset would still land in the right place"
    );

    // And the holes really are holes: a read that is not the prefix and not a
    // payload has to fail, or "the reader found the tile" would be satisfied by
    // a fixture that answers every offset.
    assert!(
        archive.read_range(base - 4096, 16).is_err(),
        "the sparse archive served bytes from a hole, so a reader computing the \
         wrong position would be handed something rather than failing"
    );

    let reader = Reader::try_new(archive).expect("open the sparse archive");
    assert_eq!(
        reader.header().tile_data_offset,
        base,
        "the header did not survive the round trip through 64-bit offsets"
    );

    for (z, x, y, want) in [(3u8, 1u32, 2u32, &first), (3, 5, 6, &second)] {
        let got = reader
            .get_tile(z, x, y)
            .unwrap_or_else(|e| {
                panic!(
                    "({z}, {x}, {y}) at an offset past 4 GiB: {e}. A read landing \
                     in a hole is a truncated offset, not a missing tile."
                )
            })
            .unwrap_or_else(|| panic!("({z}, {x}, {y}) came back absent"));
        assert_eq!(
            sha256_hex(&got),
            sha256_hex(want),
            "({z}, {x}, {y}) came back with the wrong bytes from past 4 GiB"
        );
    }

    // The negative control: a coordinate the archive does not hold is still
    // absent rather than the nearest payload.
    assert_eq!(
        reader.get_tile(3, 7, 7).expect("a legal coordinate"),
        None,
        "the sparse archive answered for a tile it does not hold"
    );
}

/// The same arithmetic, one byte at a time, at the exact boundary.
///
/// `2**32 - 1` and `2**32` land either side of the place a 32-bit truncation
/// stops working, so a reader that wraps there reads the first payload for the
/// second.
#[test]
fn the_four_gigabyte_boundary_itself_is_not_a_wrap() {
    let low = vec![0x11u8; 64];
    let high = vec![0x22u8; 64];
    // The first payload ends one byte before 4 GiB, so the second starts on it.
    let base = FOUR_GIB - low.len() as u64;

    let id_a = zxy_to_tileid(2, 0, 0).expect("a valid coordinate");
    let id_b = zxy_to_tileid(2, 3, 3).expect("a valid coordinate");
    let archive = sparse_archive(base, &[(id_a, low.clone()), (id_b, high.clone())])
        .expect("assemble a sparse archive");
    let reader = Reader::try_new(archive).expect("open the sparse archive");

    let below = reader
        .get_tile(2, 0, 0)
        .expect("the tile below the boundary")
        .expect("is present");
    let above = reader
        .get_tile(2, 3, 3)
        .expect("the tile above the boundary")
        .expect("is present");

    assert_ne!(
        sha256_hex(&low),
        sha256_hex(&high),
        "the two payloads are identical, so a reader that returned the same one \
         twice would pass"
    );
    assert_eq!(sha256_hex(&below), sha256_hex(&low));
    assert_eq!(
        sha256_hex(&above),
        sha256_hex(&high),
        "the tile starting exactly at 4 GiB came back as the one below it, \
         which is what a 32-bit offset does at the boundary"
    );
}

// ---------------------------------------------------------------------------
// Bounded reads
// ---------------------------------------------------------------------------

/// Build a real archive with enough tiles to make "index plus payload" a
/// meaningfully smaller number than "the whole file".
fn many_tile_archive(dir: &Path) -> (std::path::PathBuf, PyramidPlan) {
    let plan = PyramidPlanner::new(2048, 2048, 128, 0, Layout::Xyz)
        .expect("a valid ZXY plan")
        .plan();
    let src = canonical_raster_scaled(2048, 2048);
    let path = dir.join("many.pmtiles");
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
    assert!(sink.published_header().is_some(), "nothing was finalised");
    (path, plan)
}

/// Opening an archive reads the header and the root directory, and nothing
/// else.
#[test]
fn opening_an_archive_reads_the_header_and_the_root_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _plan) = many_tile_archive(dir.path());
    let size = std::fs::metadata(&path).expect("stat the archive").len();

    // Positive control: on a tiny archive "read less than the whole file" is
    // satisfied by reading the whole file, so require a file worth the claim.
    assert!(
        size > 100_000,
        "the archive is only {size} bytes, which is small enough that reading \
         all of it would satisfy every bound below"
    );

    let counting = CountingRangeReader::new(
        FileRangeReader::try_open(&path).expect("open the archive for ranged reads"),
    );
    let reader = Reader::try_new(counting).expect("open the archive");
    let header = *reader.header();
    let read_at_open = reader.source().bytes();

    assert_eq!(
        read_at_open,
        127 + header.root_length,
        "opening the archive read {read_at_open} bytes; the header is 127 and \
         the root directory is {}, and a reader that pulled anything else has \
         fetched something it was not asked for",
        header.root_length
    );
    assert!(
        read_at_open < size,
        "opening the archive read all {size} bytes of it"
    );
}

/// Fetching one tile costs the payload, plus at most the directory pages on the
/// way to it.
#[test]
fn fetching_one_tile_reads_the_payload_and_not_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let (path, plan) = many_tile_archive(dir.path());
    let size = std::fs::metadata(&path).expect("stat the archive").len();
    assert!(size > 100_000, "the archive is too small for this claim");

    let counting = CountingRangeReader::new(
        FileRangeReader::try_open(&path).expect("open the archive for ranged reads"),
    );
    let reader = Reader::try_new(counting).expect("open the archive");
    let has_leaves = reader.header().has_leaves();
    reader.source().reset();

    let top = plan.levels.last().expect("a plan has levels");
    let z = u8::try_from(top.level).expect("a test plan stays inside u8 zooms");
    let tile = reader
        .get_tile(z, top.cols / 2, top.rows / 2)
        .expect("read a tile")
        .expect("the middle of the deepest level is present");

    // The control. Every bound below is satisfied by a reader that read nothing
    // and answered None, so say first that it answered with a real tile.
    assert!(!tile.is_empty(), "the tile came back empty");
    assert_eq!(
        &tile[1..4],
        b"PNG",
        "the bytes that came back are not a PNG, so whatever was read was not \
         the payload"
    );

    let read = reader.source().bytes();
    let calls = reader.source().calls();
    if has_leaves {
        assert!(
            read <= tile.len() as u64 + reader.header().leaf_directories_length,
            "one tile cost {read} bytes: more than the payload ({}) plus every \
             leaf directory in the file ({})",
            tile.len(),
            reader.header().leaf_directories_length
        );
    } else {
        assert_eq!(
            calls, 1,
            "the root addresses every tile in this archive, so one tile is one \
             read, and this took {calls}"
        );
        assert_eq!(
            read,
            tile.len() as u64,
            "one tile cost {read} bytes for a {}-byte payload",
            tile.len()
        );
    }
    assert!(
        read < size / 4,
        "one tile cost {read} bytes out of a {size}-byte archive, which is not \
         a ranged read of an index, it is most of the file"
    );
}

/// Walking every tile fetches the payloads and nothing else.
///
/// The single-tile bound above is satisfied by a reader that slurps the whole
/// file on the first call and answers the rest from memory, which is exactly
/// what a single-file format cannot afford at scale. This one would catch that,
/// because the total has to come out equal to the bytes actually handed back.
///
/// Note what it is NOT: "less than the archive". Dedupe means several
/// coordinates share one payload, so a full walk legitimately reads more bytes
/// than the tile data section holds, and on this fixture it reads more than the
/// whole archive. Measured here, 341 tiles over a 102622-byte tile data
/// section came to 252605 bytes. An archive-size bound would have looked
/// tighter and been wrong.
#[test]
fn walking_every_tile_reads_the_payloads_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let (path, plan) = many_tile_archive(dir.path());

    let counting = CountingRangeReader::new(
        FileRangeReader::try_open(&path).expect("open the archive for ranged reads"),
    );
    let reader = Reader::try_new(counting).expect("open the archive");
    let has_leaves = reader.header().has_leaves();
    reader.source().reset();

    let mut found = 0u64;
    let mut handed_back = 0u64;
    for coord in plan.tile_coords() {
        let z = u8::try_from(coord.level).expect("a test plan stays inside u8 zooms");
        if let Some(bytes) = reader
            .get_tile(z, coord.col, coord.row)
            .expect("read a tile")
        {
            handed_back += bytes.len() as u64;
            found += 1;
        }
    }

    assert_eq!(
        found,
        plan.total_tile_count(),
        "only {found} of {} tiles came back, so the byte count below is about a \
         partial walk",
        plan.total_tile_count()
    );
    assert!(
        handed_back > 0,
        "the walk handed back no bytes at all, so every bound below is vacuous"
    );

    let read = reader.source().bytes();
    let calls = reader.source().calls();
    if has_leaves {
        assert!(
            read <= handed_back + reader.header().leaf_directories_length,
            "the walk read {read} bytes to hand back {handed_back}, more than \
             every leaf directory in the file ({}) could account for",
            reader.header().leaf_directories_length
        );
    } else {
        assert_eq!(
            calls as u64, found,
            "the root addresses every tile here, so a walk of {found} tiles is \
             {found} reads, and this took {calls}"
        );
        assert_eq!(
            read, handed_back,
            "the walk read {read} bytes and handed back {handed_back}. They have \
             to be the same number: anything more is the reader fetching \
             something nobody asked for."
        );
    }
}

// ---------------------------------------------------------------------------
// Bounded memory
// ---------------------------------------------------------------------------

/// Writing an archive from the streaming engine stays inside its budget.
///
/// The same shape as `packfile_streaming_memory_bounded`, and with the same
/// second bound, because the first one on its own is satisfied by an engine that
/// read the whole image at once: four times a 4 MB budget is more than a 12 MB
/// canvas. The tighter claim is that peak stays under one full canvas, and it is
/// the one that says anything was streamed.
#[test]
fn pmtiles_streaming_memory_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("streamed.pmtiles");

    let src = canonical_raster_scaled(2048, 2048);
    let plan = PyramidPlanner::new(2048, 2048, 256, 0, Layout::Xyz)
        .expect("a valid ZXY plan")
        .plan();

    // The budget has to clear the streaming engine's pre-flight floor, which is
    // one minimum aligned strip: `2 * tile_size` rows at canvas width, so
    // 2048 * 512 * 3 = 3_145_728 bytes for this Rgb8 canvas. A budget under the
    // floor makes the run answer BudgetExceeded and every assertion after it
    // unreachable, which is how the packfile twin of this cell sat unrun.
    let budget: u64 = 4_000_000;

    let sink = PmTilesSink::builder(&path)
        .plan(plan.clone())
        .tile_format(TileFormat::Png)
        .build()
        .expect("build a PmTilesSink");
    let result = EngineBuilder::new(RasterStripSource::new(&src), plan.clone(), &sink)
        .with_engine(EngineKind::Streaming)
        .with_config(EngineConfig::default())
        .with_memory_budget(budget)
        .run()
        .expect("stream into PmTilesSink");

    assert_eq!(
        result.tiles_produced,
        plan.total_tile_count(),
        "the streaming run produced {} of {} tiles, so the memory numbers below \
         are about a run that did not finish the job",
        result.tiles_produced,
        plan.total_tile_count()
    );

    let canvas = 2048u64 * 2048 * PixelFormat::Rgb8.bytes_per_pixel() as u64;
    assert!(
        result.peak_memory_bytes <= budget.saturating_mul(4),
        "peak memory {} is more than four times the {budget}-byte budget",
        result.peak_memory_bytes
    );
    assert!(
        result.peak_memory_bytes < canvas,
        "peak memory {} reached the full {canvas}-byte canvas, so nothing here \
         was streamed and the bound above passed on an engine that read the \
         image in one piece",
        result.peak_memory_bytes
    );

    // And the archive it streamed has to be a readable archive, or "bounded
    // memory" is being proved by a run that wrote nothing.
    let reader = Reader::try_open(&path).expect("open the streamed archive");
    assert_eq!(
        reader.header().addressed_tiles_count,
        plan.total_tile_count(),
        "the streamed archive does not address every tile the run produced"
    );
}

/// A directory that inflates past the ceiling has to be refused, not inflated.
///
/// # The hole this closes
///
/// Every other cell in this file measures what a *well-formed* archive costs to
/// read. None of them measures what a hostile one costs, and the difference is
/// not academic: an adversarial review deleted the reader's gzip ceiling
/// outright, `MAX_DIRECTORY_BYTES` and the `take` that enforces it, and all
/// thirty tests in the PMTiles suite stayed green. The one thing standing
/// between a hostile archive and an unbounded inflate was guarded by nothing,
/// in the file whose whole subject is that a hostile header cannot make a
/// reader pull gigabytes.
///
/// A gzip stream of zeros is about a thousand to one, so a root directory of a
/// few kilobytes inflates to several mebibytes. The archive is otherwise valid:
/// a real header, a real length, and a root that starts with the gzip magic the
/// reader expects, so it gets all the way to the decompressor before anything
/// says no.
///
/// The counting reader is the other half. Refusing is only interesting if the
/// refusal happens before the bytes are pulled, so the test asserts the reader
/// asked for the root once and stopped.
#[test]
fn a_directory_that_inflates_past_the_ceiling_is_refused_rather_than_inflated() {
    use libviprs::pmtiles::PmTilesError;
    use libviprs::pmtiles::header::{Compression, Header, TileType};
    use libviprs::pmtiles::reader::{MAX_DIRECTORY_BYTES, MAX_ROOT_SPAN};

    // Past MAX_DIRECTORY_BYTES and, compressed, still inside MAX_ROOT_SPAN.
    // Both halves matter. The first time I wrote this the bomb compressed to
    // 16 KB, the root then ran past the 16384-byte span the format allows, and
    // the reader refused it for that instead: the test passed with the
    // decompression ceiling deleted. A test that passes for the wrong reason is
    // the thing this whole PR is about.
    let bomb_plain = vec![0u8; 8 * 1024 * 1024];
    let bomb = Compression::Gzip
        .compress(&bomb_plain)
        .expect("gzip a run of zeros");
    assert!(
        bomb_plain.len() > MAX_DIRECTORY_BYTES,
        "the bomb inflates to {} bytes and the ceiling is {MAX_DIRECTORY_BYTES}, \
         so there is nothing for the reader to refuse",
        bomb_plain.len()
    );
    assert!(
        127 + bomb.len() as u64 <= MAX_ROOT_SPAN,
        "the bomb compresses to {} bytes, so the root runs past the \
         {MAX_ROOT_SPAN}-byte span the format allows and the reader refuses it \
         for that rather than for inflating. That refusal happens with the \
         decompression ceiling deleted, so this test would pass without it.",
        bomb.len()
    );

    let metadata = Compression::Gzip
        .compress(br#"{"name":"bomb"}"#)
        .expect("gzip the metadata");

    let mut header = Header {
        root_offset: 127,
        root_length: bomb.len() as u64,
        metadata_offset: 127 + bomb.len() as u64,
        metadata_length: metadata.len() as u64,
        leaf_directories_offset: 127 + bomb.len() as u64 + metadata.len() as u64,
        leaf_directories_length: 0,
        tile_data_offset: 127 + bomb.len() as u64 + metadata.len() as u64,
        tile_data_length: 0,
        addressed_tiles_count: 0,
        tile_entries_count: 0,
        tile_contents_count: 0,
        clustered: true,
        internal_compression: Compression::Gzip,
        tile_compression: Compression::None,
        tile_type: TileType::Png,
        min_zoom: 0,
        max_zoom: 0,
        center_zoom: 0,
        ..Header::default()
    };
    header.set_bounds_degrees(-180.0, -85.0, 180.0, 85.0);
    header.set_center_degrees(0.0, 0.0);

    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(&bomb);
    bytes.extend_from_slice(&metadata);
    let size = bytes.len() as u64;

    let counting = CountingRangeReader::new(WholeArchive { bytes, size });
    let opened = Reader::try_new(counting);
    let Err(refused) = opened else {
        panic!(
            "a root directory that inflates to {} MiB was accepted. The reader's \
             decompression ceiling is the only thing between a hostile archive \
             and an unbounded inflate.",
            bomb_plain.len() / (1024 * 1024)
        );
    };
    assert!(
        matches!(refused, PmTilesError::DecompressionLimit { .. }),
        "the archive was refused, and not for inflating past the ceiling: \
         {refused:?}. Any other refusal means this cell is measuring something \
         else and would survive the ceiling being deleted."
    );
}

/// The whole archive in memory, served by range.
struct WholeArchive {
    bytes: Vec<u8>,
    size: u64,
}

impl RangeReader for WholeArchive {
    fn read_range(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let at = usize::try_from(offset).map_err(std::io::Error::other)?;
        let end = at
            .checked_add(len)
            .ok_or_else(|| std::io::Error::other("overflow"))?;
        if end > self.bytes.len() {
            return Err(std::io::Error::other("past the end"));
        }
        Ok(self.bytes[at..end].to_vec())
    }

    fn size(&self) -> std::io::Result<Option<u64>> {
        Ok(Some(self.size))
    }
}

/// A varint that overflows a `u64` has to be refused, not wrapped.
///
/// The twin of the cell above, and it was open for the same reason: both
/// goldens are well formed, so no committed byte ever reaches this branch, and
/// deleting the guard left the whole PMTiles suite green.
///
/// Every offset and length in a directory arrives as one of these. A decoder
/// that wraps instead of refusing turns a hostile ten-byte varint into a small
/// number that passes every bounds check downstream, which is the shape where a
/// refusal one layer up stops being a refusal at all.
///
/// This is the one fixture in this file with no oracle behind it, and that is
/// fine: the claim is that these bytes do not decode, not that they decode to
/// something in particular. The positive control is the line above it, a
/// ten-byte varint that is exactly representable and has to come back.
#[test]
fn a_varint_that_overflows_a_u64_is_refused_rather_than_wrapped() {
    use libviprs::pmtiles::varint::{MAX_UVARINT_LEN, decode_uvarint, encode_uvarint};

    // The control. u64::MAX is ten bytes and its last byte is the bare 1 the
    // format allows, so it has to round-trip.
    let mut largest = Vec::new();
    encode_uvarint(u64::MAX, &mut largest);
    assert_eq!(
        largest.len(),
        MAX_UVARINT_LEN,
        "u64::MAX should encode to {MAX_UVARINT_LEN} bytes"
    );
    assert_eq!(
        decode_uvarint(&largest, 0).expect("u64::MAX decodes"),
        (u64::MAX, MAX_UVARINT_LEN),
        "the largest representable varint has to come back, or the refusal \
         below is refusing everything rather than refusing an overflow"
    );

    // The same shape with 2 in the top byte, which is one bit past a u64.
    let mut overflowing = largest.clone();
    *overflowing.last_mut().expect("ten bytes") = 2;
    let refused = decode_uvarint(&overflowing, 0);
    assert!(
        refused.is_err(),
        "a ten-byte varint carrying bit 64 decoded to {refused:?} instead of \
         being refused. It wraps, and every offset and length in a directory \
         arrives through this decoder, so a wrapped value passes every bounds \
         check downstream."
    );

    // And a run of continuation bytes that never ends.
    let never_ends = vec![0xFFu8; MAX_UVARINT_LEN + 4];
    assert!(
        decode_uvarint(&never_ends, 0).is_err(),
        "a varint of {} continuation bytes was accepted",
        never_ends.len()
    );
}
