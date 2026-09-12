//! Shared helpers for the PMTiles black-box suite (libviprs-tests#202).
//!
//! Five test binaries use this: `phase_pmtiles.rs` for cross-backend
//! equivalence, `pmtiles_interop.rs` for the committed go-pmtiles goldens,
//! `pmtiles_bounded.rs` for the offset arithmetic and the bounded-read proof,
//! `pmtiles_sweep.rs` for the whole-grid comparison against the reference, and
//! `cli_pmtiles.rs` for the CLI-driven cells.
//!
//! # Why this is not registered in `tests/common/mod.rs`
//!
//! It looks like it belongs there, beside `cli` and `dzsave_expected`, and the
//! plan for this issue said to put it there. It cannot go there, and the reason
//! is worth a paragraph rather than a shrug.
//!
//! `tests/common/mod.rs` is pulled in by `mod common;` at the top of nearly
//! every test file in this repo, so anything it declares is compiled into every
//! one of those binaries. This module names `libviprs::pmtiles` and
//! `libviprs::PyramidReader`, which arrive with EPIC F. Registering it would
//! mean that on any counterpart without them, all ~140 test binaries fail to
//! compile rather than the three that are actually about PMTiles, and a CI run
//! where everything is red carries no signal about anything.
//!
//! So each consumer pulls it in by path instead:
//!
//! ```ignore
//! #[path = "common/pmtiles.rs"]
//! mod pmtiles_support;
//! ```
//!
//! Cargo only treats top-level `tests/*.rs` as integration targets, so a file
//! under `tests/common/` that nothing declares is simply never compiled.
//!
//! # What is pinned here, and against what
//!
//! Everything under `tests/fixtures/pmtiles/` came out of the real
//! `protomaps/go-pmtiles` v1.31.2. `PROVENANCE.md` beside those files says how.
//! The loaders below read them at run time and verify the sha256 of the bytes
//! they just read, because a number transcribed into a Rust `const` is
//! indistinguishable from an invented one, and "fixing" a red test by editing
//! the expectation is then invisible.
//!
//! Three ways a loader like this quietly stops checking anything, and what it
//! does about each:
//!
//! * **the file is the wrong file.** Every read verifies a pinned digest and
//!   panics on a mismatch.
//! * **the parse comes back empty.** A loop over zero rows passes every
//!   assertion inside it, so every accessor refuses an empty result and names
//!   what it was looking for rather than returning a `Vec` of length zero.
//! * **the fixture came from a different tool.** Every vector load asserts the
//!   `produced_by` block names the pinned release and commit.

#![allow(dead_code)]

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use libviprs::PyramidReader;
use libviprs::planner::PyramidPlan;
use libviprs::pmtiles::{Compression, Entry, Header, PmTilesError, RangeReader, directory};
use serde_json::Value;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// The pin
// ---------------------------------------------------------------------------

/// The go-pmtiles release every fixture here was produced by.
pub const ORACLE_RELEASE_TAG: &str = "v1.31.2";
/// The go-pmtiles source commit behind [`ORACLE_RELEASE_TAG`].
pub const ORACLE_SOURCE_COMMIT: &str = "a3e4951ea6a0477b784c27c1dcbfd9c130878c5a";

/// sha256 of `go-pmtiles_1.31.2_Linux_x86_64.tar.gz`, which is what CI
/// downloads, because `ubuntu-latest` runners are x86_64.
pub const GO_PMTILES_LINUX_X86_64_SHA256: &str =
    "3ed7dbf4ec2e6dfe5e25b6f70d1ffc932729f93c86db353bf514dd71010a312f";
/// sha256 of `go-pmtiles_1.31.2_Linux_arm64.tar.gz`, which is what the local
/// container mirror downloads.
pub const GO_PMTILES_LINUX_ARM64_SHA256: &str =
    "f8bd47e7ea866863489cad588fbaf2f31f42e5821f7a03f009b3769f05801cb1";

// ---------------------------------------------------------------------------
// The committed fixtures
// ---------------------------------------------------------------------------

/// The plain golden: zoom 0 to 2, 21 distinct tiles, one root directory.
pub const RASTER_GOLDEN: &str = "raster-z0z2.pmtiles";
/// sha256 of [`RASTER_GOLDEN`].
pub const RASTER_GOLDEN_SHA256: &str =
    "e2ed5e64f3c29efa3ec3b679ec5f1b06569c1b234c6eea762fb9f02fc23e9c12";

/// The duplicate golden: runs and repeated non-adjacent offsets, both shapes.
pub const DUPES_GOLDEN: &str = "dupes-z0z3.pmtiles";
/// sha256 of [`DUPES_GOLDEN`].
pub const DUPES_GOLDEN_SHA256: &str =
    "bfc9db4c6ce6a04194e02b3d4815814adb05209f1aaba8591e4e1332f6e56a27";

/// The leaf golden: 6 real leaf directories over 21844 entries.
pub const LEAVES_GOLDEN: &str = "leaves-z0z7.pmtiles";
/// sha256 of [`LEAVES_GOLDEN`].
pub const LEAVES_GOLDEN_SHA256: &str =
    "fe5c9636be61abc60046d7f13837f8a3efb20ce3c38303644dac0cbec8248b8d";

/// The distinct golden: a payload per tile, so nothing about it is symmetric.
///
/// `leaves-z0z7` was the only fixture with leaf directories and it cannot see
/// any of it. Its two payloads alternate by `(x + y) % 2` and every plausible
/// tile-id mistake preserves that parity at every zoom, so all 21845 cells come
/// back identical under five wrong conventions, measured. Its leaves all start
/// at offset 0 as well, which makes the wrong leaf-entry base the identity.
/// This one has 19843 distinct payloads and leaves starting at 49164, 98324,
/// 147497 and 196597, so both mistakes move bytes.
pub const DISTINCT_GOLDEN: &str = "distinct-z0z7.pmtiles";
/// sha256 of [`DISTINCT_GOLDEN`].
pub const DISTINCT_GOLDEN_SHA256: &str =
    "a32dce77a93a304dbd27b80d72160b29455446931b477b5b1b7a84155ecd2dd5";

/// The header golden: vector tiles, gzip payloads, and nothing symmetric.
///
/// The other four all carry the symmetric whole-world bounds, a centre of
/// `0, 0`, a minimum zoom of 0, and `tile_type` and the two compressions at
/// neighbouring byte values, so a lat/lon transposition, a swapped pair of
/// compression bytes and a dropped minimum zoom are all the identity in them.
pub const HEADER_MVT_GOLDEN: &str = "header-mvt-z2z4.pmtiles";
/// sha256 of [`HEADER_MVT_GOLDEN`].
pub const HEADER_MVT_GOLDEN_SHA256: &str =
    "d15ece2e0cd6517817529a96fc6767e8df448a197e59bcabf6c1456c3bc3eba7";

/// Every committed golden, with its digest. Used so a test can sweep all of
/// them rather than naming them one at a time and quietly dropping one.
pub const GOLDENS: &[(&str, &str)] = &[
    (RASTER_GOLDEN, RASTER_GOLDEN_SHA256),
    (DUPES_GOLDEN, DUPES_GOLDEN_SHA256),
    (LEAVES_GOLDEN, LEAVES_GOLDEN_SHA256),
    (DISTINCT_GOLDEN, DISTINCT_GOLDEN_SHA256),
    (HEADER_MVT_GOLDEN, HEADER_MVT_GOLDEN_SHA256),
];

/// The goldens `vectors/tiles.json` carries hand-picked rows for.
///
/// The two added later are covered by `vectors/sweep.json` instead, which holds
/// every cell rather than a chosen few, so there is nothing for them to have a
/// row in.
pub const TILES_VECTOR_GOLDENS: &[(&str, &str)] = &[
    (RASTER_GOLDEN, RASTER_GOLDEN_SHA256),
    (DUPES_GOLDEN, DUPES_GOLDEN_SHA256),
    (LEAVES_GOLDEN, LEAVES_GOLDEN_SHA256),
];

/// sha256 of `vectors/tiles.json`.
pub const TILES_JSON_SHA256: &str =
    "efaebeee9399d9e1e6e0395059caf38c659f134353442bfe29fd0065e5ae6581";
/// sha256 of `vectors/header.json`.
pub const HEADER_JSON_SHA256: &str =
    "c815b35da91b639c6daace98551dfa9e2e5ff07c561537fd3e50a3f747e9908e";
/// sha256 of `vectors/show.json`.
pub const SHOW_JSON_SHA256: &str =
    "42dfdd7ef763885ce61eb4128db7ae472d29997f0429abe018d141540fd1536a";
/// sha256 of `vectors/sweep.json`.
pub const SWEEP_JSON_SHA256: &str =
    "719cbc55d3a6ca1432786b4bf546d5525d6a1a484da824c3e931f573ad662df9";

/// `tests/fixtures/pmtiles/`, absolute.
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("pmtiles")
}

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// sha256 of some bytes, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Read a fixture and check its sha256 before handing the bytes back.
pub fn read_checked(relative: &str, want_sha256: &str) -> Vec<u8> {
    let path = fixtures_dir().join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read the committed fixture {}: {e}", path.display()));
    let got = sha256_hex(&bytes);
    assert_eq!(
        got,
        want_sha256,
        "{} is not the file these tests were pinned against. Either the fixture \
         was edited, which is never the fix for a red interop test, or it was \
         regenerated, which needs PROVENANCE.md updating with it.",
        path.display()
    );
    bytes
}

/// The path to a golden, after checking the bytes on disk are the bytes the
/// digest names.
///
/// Returns a path rather than bytes because the reader under test opens a file:
/// checking the digest and then handing over the path is the closest a caller
/// can get to "this exact file".
pub fn golden_path(name: &str, want_sha256: &str) -> PathBuf {
    let _ = read_checked(name, want_sha256);
    fixtures_dir().join(name)
}

/// One of the vector files, parsed, digest-checked, with `produced_by`
/// confirmed to name the pinned release, commit and both tarball digests.
///
/// The tarball digests are asserted here and not only in CI on purpose: the
/// digest CI verifies with `sha256sum -c` and the digest the fixtures were
/// produced under are the same number, and a guard that reads it from one place
/// cannot notice them drifting apart.
pub fn vectors(name: &str, want_sha256: &str) -> Value {
    let bytes = read_checked(&format!("vectors/{name}"), want_sha256);
    let value: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("vectors/{name} is not JSON: {e}"));
    let produced = value.get("produced_by").unwrap_or_else(|| {
        panic!("vectors/{name} has no produced_by block, so nothing says which tool wrote it")
    });
    for (key, want) in [
        ("release_tag", ORACLE_RELEASE_TAG),
        ("source_commit", ORACLE_SOURCE_COMMIT),
        ("linux_amd64_tarball_sha256", GO_PMTILES_LINUX_X86_64_SHA256),
        ("linux_arm64_tarball_sha256", GO_PMTILES_LINUX_ARM64_SHA256),
    ] {
        assert_eq!(
            produced.get(key).and_then(Value::as_str),
            Some(want),
            "vectors/{name} produced_by.{key} does not match the pin these tests \
             and .github/workflows/ci.yml share"
        );
    }
    value
}

/// The whole of `vectors/tiles.json`.
pub fn tiles_vectors() -> Value {
    vectors("tiles.json", TILES_JSON_SHA256)
}

/// The whole of `vectors/header.json`.
pub fn header_vectors() -> Value {
    vectors("header.json", HEADER_JSON_SHA256)
}

/// The whole of `vectors/show.json`.
pub fn show_vectors() -> Value {
    vectors("show.json", SHOW_JSON_SHA256)
}

/// The whole of `vectors/sweep.json`.
pub fn sweep_vectors() -> Value {
    vectors("sweep.json", SWEEP_JSON_SHA256)
}

/// One field as `pmtiles show` printed it, as a string.
pub fn oracle_shown(vectors: &Value, archive: &str, field: &str) -> String {
    let fields = archive_block(vectors, archive)
        .get("fields")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{archive} has no fields block in show.json"));
    fields
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            let have: Vec<&str> = fields.keys().map(String::as_str).collect();
            panic!("{archive}'s show output has no string {field:?}; it has {have:?}")
        })
}

/// One field as `pmtiles show` printed it, as the numbers it rendered.
pub fn oracle_shown_numbers(vectors: &Value, archive: &str, field: &str) -> Vec<f64> {
    let fields = archive_block(vectors, archive)
        .get("fields")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{archive} has no fields block in show.json"));
    fields
        .get(field)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{archive}'s show output has no numeric {field:?}"))
        .iter()
        .map(|v| {
            v.as_f64()
                .unwrap_or_else(|| panic!("{archive}: {field} holds {v:?}, which is not a number"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Reading rows out of the vectors
// ---------------------------------------------------------------------------

/// One `(z, x, y)` row as `pmtiles tile` answered it.
#[derive(Debug, Clone)]
pub struct TileRow {
    pub z: u8,
    pub x: u32,
    pub y: u32,
    /// The CLI's exit status for that invocation.
    pub exit_code: i64,
    /// Bytes written. Zero on an `absent` row, which is how go-pmtiles reports
    /// "this archive does not hold that tile".
    pub length: usize,
    /// sha256 of what was written, empty on a zero-length row.
    pub sha256: String,
    /// The offset of the directory entry covering this tile, relative to the
    /// tile data section, as the reference read it. `None` when the tile is
    /// served by a run, in which case the covering entry carries a lower tile
    /// id and this row is not the place to look it up.
    pub directory_entry_offset: Option<u64>,
    /// Whether an entry addresses this tile directly. `false` means a run
    /// serves it, which is the duplicate shape a reader can get wrong while
    /// looking correct on everything else.
    pub directly_addressed: bool,
}

fn archive_block<'a>(vectors: &'a Value, archive: &str) -> &'a Value {
    let archives = vectors
        .get("archives")
        .and_then(Value::as_object)
        .expect("the vector file has no archives block");
    archives.get(archive).unwrap_or_else(|| {
        let have: Vec<&str> = archives.keys().map(String::as_str).collect();
        panic!("the vector file has no rows for {archive:?}; it has {have:?}")
    })
}

/// Every archive the vector file carries rows for, in file order.
///
/// A test sweeps this rather than naming the three archives itself, so a
/// fixture that stops being covered shows up as a missing file rather than as
/// nothing at all.
pub fn vector_archive_names(vectors: &Value) -> Vec<String> {
    let archives = vectors
        .get("archives")
        .and_then(Value::as_object)
        .expect("the vector file has no archives block");
    assert!(
        !archives.is_empty(),
        "the vector file's archives block is empty, so every sweep over it \
         would be a loop that runs zero times"
    );
    archives.keys().cloned().collect()
}

/// The `golden_sha256` the vector file records for an archive.
///
/// This is the independent half of the fixture check: [`read_checked`] compares
/// the file on disk against a digest in this source file, and this compares it
/// against a digest the oracle's dump program wrote. Two sources have to agree.
pub fn vector_golden_sha256(vectors: &Value, archive: &str) -> String {
    archive_block(vectors, archive)
        .get("golden_sha256")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{archive} has no golden_sha256 in the vector file"))
        .to_string()
}

/// Rows from one named section (`hits`, `absent`, `out_of_range`) of one
/// archive in `tiles.json`.
///
/// Panics on a missing section or an empty one. Both are the same failure from
/// the caller's point of view, a loop that runs zero times, and both have to be
/// loud rather than green.
pub fn tile_rows(vectors: &Value, archive: &str, section: &str) -> Vec<TileRow> {
    let block = archive_block(vectors, archive);
    let array = block
        .get(section)
        .and_then(Value::as_array)
        .unwrap_or_else(|| {
            let have: Vec<&str> = block
                .as_object()
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            panic!("{archive} has no array section {section:?}; it has {have:?}")
        });

    let rows: Vec<TileRow> = array
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let num = |key: &str| -> u64 {
                row.get(key)
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| panic!("{archive}.{section}[{index}] has no unsigned {key}"))
            };
            TileRow {
                z: u8::try_from(num("z")).expect("a zoom fits in a u8"),
                x: u32::try_from(num("x")).expect("an x fits in a u32"),
                y: u32::try_from(num("y")).expect("a y fits in a u32"),
                exit_code: row
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .unwrap_or_else(|| panic!("{archive}.{section}[{index}] has no exit_code")),
                length: usize::try_from(num("length")).expect("a length fits in a usize"),
                sha256: row
                    .get("sha256")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| panic!("{archive}.{section}[{index}] has no sha256"))
                    .to_string(),
                directory_entry_offset: row.get("directory_entry_offset").and_then(Value::as_u64),
                directly_addressed: row
                    .get("directly_addressed_by_an_entry")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }
        })
        .collect();

    assert!(
        !rows.is_empty(),
        "{archive}.{section} parsed to zero rows, so whatever loops over it \
         asserts nothing"
    );
    rows
}

/// One field of the reference implementation's own decoding of an archive's
/// header, out of `header.json`.
///
/// These are `pmtiles.DeserializeHeader`'s answers for the same 127 bytes our
/// decoder reads, so a disagreement is a disagreement with the reference rather
/// than with a number somebody typed into a test.
pub fn oracle_header_u64(vectors: &Value, archive: &str, field: &str) -> u64 {
    let decoded = archive_block(vectors, archive)
        .get("decoded_by_pmtiles_DeserializeHeader")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{archive} has no decoded_by_pmtiles_DeserializeHeader block"));
    let raw = decoded.get(field).unwrap_or_else(|| {
        let have: Vec<&str> = decoded.keys().map(String::as_str).collect();
        panic!("{archive}'s decoded header has no {field:?}; it has {have:?}")
    });
    // `clustered` comes back as a JSON bool because that is what Go's encoder
    // does with a Go bool. It is a byte in the header, so it compares as one.
    match raw {
        Value::Bool(b) => u64::from(*b),
        other => other.as_u64().unwrap_or_else(|| {
            panic!("{archive}'s {field:?} is {other:?}, which is not a number or a bool")
        }),
    }
}

/// The same, for a field the reference decodes as signed. The four bounds and
/// the two centre positions are `i32` in the header and negative in every
/// golden that covers the whole world, so reading them through
/// [`oracle_header_u64`] panics rather than comparing.
pub fn oracle_header_i64(vectors: &Value, archive: &str, field: &str) -> i64 {
    let decoded = archive_block(vectors, archive)
        .get("decoded_by_pmtiles_DeserializeHeader")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{archive} has no decoded_by_pmtiles_DeserializeHeader block"));
    decoded
        .get(field)
        .and_then(Value::as_i64)
        .unwrap_or_else(|| {
            let have: Vec<&str> = decoded.keys().map(String::as_str).collect();
            panic!("{archive}'s decoded header has no signed {field:?}; it has {have:?}")
        })
}

/// Every field name the reference reported for `archive`, so a comparison can
/// be held to covering all of them.
///
/// A header test that names the fields it checks silently stops covering each
/// new one, and this header has 25. Thirteen of them were compared and the
/// twelve that were not are where a big-endian `i32` or a lat/lon transposition
/// hides: measured, both leave the whole PMTiles suite green.
pub fn oracle_header_fields(vectors: &Value, archive: &str) -> Vec<String> {
    archive_block(vectors, archive)
        .get("decoded_by_pmtiles_DeserializeHeader")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{archive} has no decoded_by_pmtiles_DeserializeHeader block"))
        .keys()
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Counting what a reader actually touches
// ---------------------------------------------------------------------------

/// A [`RangeReader`] that counts what passes through it.
///
/// Wraps any other reader and records the number of calls, the total bytes
/// asked for, and the largest single request. The claim it exists to measure is
/// that a PMTiles read is proportional to the index plus the payload rather than
/// to the archive, and that is a measurement rather than an assertion only
/// because `Reader` is generic over this trait.
#[derive(Debug)]
pub struct CountingRangeReader<R> {
    inner: R,
    calls: AtomicUsize,
    bytes: AtomicU64,
    largest: AtomicU64,
}

impl<R: RangeReader> CountingRangeReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            calls: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
            largest: AtomicU64::new(0),
        }
    }

    /// How many `read_range` calls have happened.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    /// How many bytes have been asked for in total.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// The largest single request.
    pub fn largest_read(&self) -> u64 {
        self.largest.load(Ordering::Relaxed)
    }

    /// Zero the counters, so a caller can measure one operation rather than
    /// everything since the archive was opened.
    pub fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.bytes.store(0, Ordering::Relaxed);
        self.largest.store(0, Ordering::Relaxed);
    }
}

impl<R: RangeReader> RangeReader for CountingRangeReader<R> {
    fn read_range(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(len as u64, Ordering::Relaxed);
        self.largest.fetch_max(len as u64, Ordering::Relaxed);
        self.inner.read_range(offset, len)
    }

    fn size(&self) -> io::Result<Option<u64>> {
        self.inner.size()
    }
}

// ---------------------------------------------------------------------------
// A sparse archive nobody has to store
// ---------------------------------------------------------------------------

/// A [`RangeReader`] over an archive whose tile data sits past 4 GiB without
/// 4 GiB of anything existing.
///
/// The prefix (header, root directory, metadata) is held in memory at its real
/// position, and the tile payloads are held at their real absolute offsets in a
/// sparse list. Every other byte of the archive is a hole: a read that lands in
/// one fails rather than returning zeros, which turns "the reader wandered
/// somewhere it should not" from a silent pass into a named error.
///
/// This is how the `u64` arithmetic gets a test. A reader that truncates an
/// offset to 32 bits computes a position inside the hole and fails here, where
/// against a small archive it would compute the right answer by accident.
#[derive(Debug)]
pub struct SparseArchive {
    prefix: Vec<u8>,
    payloads: Vec<(u64, Vec<u8>)>,
    size: u64,
}

impl SparseArchive {
    pub fn new(prefix: Vec<u8>, payloads: Vec<(u64, Vec<u8>)>, size: u64) -> Self {
        Self {
            prefix,
            payloads,
            size,
        }
    }
}

impl RangeReader for SparseArchive {
    fn read_range(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let end = offset
            .checked_add(len as u64)
            .ok_or_else(|| io::Error::other("range overflows u64"))?;
        if end > self.size {
            return Err(io::Error::other(format!(
                "read of {len} at {offset} runs past the {} byte archive",
                self.size
            )));
        }
        if end <= self.prefix.len() as u64 {
            let at = offset as usize;
            return Ok(self.prefix[at..at + len].to_vec());
        }
        for (at, bytes) in &self.payloads {
            if offset == *at && len == bytes.len() {
                return Ok(bytes.clone());
            }
            if offset >= *at && end <= at + bytes.len() as u64 {
                let from = (offset - at) as usize;
                return Ok(bytes[from..from + len].to_vec());
            }
        }
        Err(io::Error::other(format!(
            "read of {len} bytes at {offset} lands in a hole: this archive holds \
             a {}-byte prefix and {} payload(s) at {:?}, and nothing in between. \
             A reader asking here has computed a position that is not where any \
             tile is.",
            self.prefix.len(),
            self.payloads.len(),
            self.payloads.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
        )))
    }

    fn size(&self) -> io::Result<Option<u64>> {
        Ok(Some(self.size))
    }
}

/// Assemble a one-root-directory archive whose tile data starts at
/// `tile_data_offset`, returning the [`SparseArchive`] that serves it.
///
/// `tiles` are `(tile_id, payload)` pairs, laid out consecutively from
/// `tile_data_offset`. Nothing between the prefix and the first payload is ever
/// materialised, so `tile_data_offset` can be any `u64` the format allows.
pub fn sparse_archive(
    tile_data_offset: u64,
    tiles: &[(u64, Vec<u8>)],
) -> Result<SparseArchive, PmTilesError> {
    let metadata = br#"{"name":"sparse","format":"png"}"#.to_vec();
    let metadata_stored = Compression::Gzip.compress(&metadata)?;

    let mut entries = Vec::new();
    let mut payloads = Vec::new();
    let mut relative: u64 = 0;
    for (tile_id, payload) in tiles {
        entries.push(Entry {
            tile_id: *tile_id,
            offset: relative,
            length: u32::try_from(payload.len()).expect("a test payload fits in a u32"),
            run_length: 1,
        });
        payloads.push((tile_data_offset + relative, payload.clone()));
        relative += payload.len() as u64;
    }
    let tile_data_length = relative;

    let root_stored = Compression::Gzip.compress(&directory::serialize_entries(&entries)?)?;

    let root_offset = libviprs::pmtiles::header::HEADER_BYTES as u64;
    let metadata_offset = root_offset + root_stored.len() as u64;

    let header = Header {
        root_offset,
        root_length: root_stored.len() as u64,
        metadata_offset,
        metadata_length: metadata_stored.len() as u64,
        leaf_directories_offset: metadata_offset + metadata_stored.len() as u64,
        leaf_directories_length: 0,
        tile_data_offset,
        tile_data_length,
        addressed_tiles_count: entries.len() as u64,
        tile_entries_count: entries.len() as u64,
        tile_contents_count: entries.len() as u64,
        clustered: true,
        internal_compression: Compression::Gzip,
        tile_compression: Compression::None,
        tile_type: libviprs::pmtiles::TileType::Png,
        min_zoom: 0,
        max_zoom: 31,
        min_lon_e7: -1_800_000_000,
        min_lat_e7: -850_511_287,
        max_lon_e7: 1_800_000_000,
        max_lat_e7: 850_511_287,
        center_zoom: 0,
        center_lon_e7: 0,
        center_lat_e7: 0,
    };

    let mut prefix = header.encode().to_vec();
    prefix.extend_from_slice(&root_stored);
    prefix.extend_from_slice(&metadata_stored);

    Ok(SparseArchive::new(
        prefix,
        payloads,
        tile_data_offset + tile_data_length,
    ))
}

// ---------------------------------------------------------------------------
// Making an archive comparable to a directory tree
// ---------------------------------------------------------------------------

/// Read every tile the plan declares out of `reader`, keyed by the relative path
/// the directory backend would have written it to.
///
/// This is the shape `common::dzsave_expected::assert_tiles_pixel_equal_tol`
/// wants (`&[(String, Vec<u8>)]`, sorted by path), which is also what
/// `collect_files` returns for a directory tree. So the two backends become
/// comparable without either side learning about the other.
///
/// Tiles the reader answers `None` for are left out rather than pushed as empty
/// bytes, so a backend that holds nothing produces an empty vector and the
/// caller's count assertion fires. Filling holes with empties would make an
/// empty archive compare equal to an empty tree, which is the vacuous pass this
/// whole exercise is about avoiding.
pub fn materialise(
    reader: &dyn PyramidReader,
    plan: &PyramidPlan,
    ext: &str,
) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for coord in plan.tile_coords() {
        let Some(rel) = plan.tile_path(coord, ext) else {
            panic!(
                "the plan has no path for {coord:?}, which it produced itself from \
                 tile_coords(), so the two disagree"
            );
        };
        let tile = reader
            .tile(coord)
            .unwrap_or_else(|e| panic!("reading {rel} back: {e}"));
        if let Some(bytes) = tile {
            out.push((rel, bytes));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Compare two tile sets by decoded pixels, whatever they are encoded as.
///
/// `common::dzsave_expected::assert_tiles_pixel_equal_tol` is the repo's
/// comparator and the right one to reach for, but it calls `decode_png`
/// unconditionally, so it panics on a JPEG tile with `InvalidSignature` rather
/// than comparing it. That is not a bug in it, the ported `dzsave` cells it was
/// written for are all PNG, but it does mean an archive cell that covers more
/// than one encoding needs this.
///
/// The rule is the same one, and it is the rule this whole repo runs on: decode
/// and compare pixels, never compare compressed bytes. Both sides here happen
/// to share an encoder, so a byte comparison would pass too, which is exactly
/// why it would be the wrong test: it would keep passing right up until one
/// side gained an encoder setting, and then it would fail for a reason that is
/// not a difference in the image.
pub fn assert_decoded_tiles_equal(
    expected: &[(String, Vec<u8>)],
    actual: &[(String, Vec<u8>)],
    context: &str,
) {
    assert_eq!(
        expected.len(),
        actual.len(),
        "{context}: tile count mismatch: {} vs {}",
        expected.len(),
        actual.len()
    );
    assert!(
        !expected.is_empty(),
        "{context}: both sides are empty, and two empty sets are equal, so this \
         comparison asserts nothing"
    );
    for ((want_path, want_bytes), (got_path, got_bytes)) in expected.iter().zip(actual.iter()) {
        assert_eq!(want_path, got_path, "{context}: tile path mismatch");
        let want = image::load_from_memory(want_bytes)
            .unwrap_or_else(|e| panic!("{context}: cannot decode expected {want_path}: {e}"));
        let got = image::load_from_memory(got_bytes)
            .unwrap_or_else(|e| panic!("{context}: cannot decode actual {got_path}: {e}"));
        assert_eq!(
            (want.width(), want.height()),
            (got.width(), got.height()),
            "{context}: {want_path} is {}x{} one side and {}x{} the other",
            want.width(),
            want.height(),
            got.width(),
            got.height()
        );
        let want_px = want.into_bytes();
        let got_px = got.into_bytes();
        assert_eq!(
            want_px.len(),
            got_px.len(),
            "{context}: {want_path} decodes to a different number of samples on \
             each side, so the two are not the same image even before the values \
             are compared"
        );
        if let Some(at) = want_px.iter().zip(got_px.iter()).position(|(a, b)| a != b) {
            panic!(
                "{context}: {want_path} differs at sample {at}: {} vs {}",
                want_px[at], got_px[at]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Proving a probe set can fail
// ---------------------------------------------------------------------------

/// The tile-id conventions a writer could plausibly land on by misreading the
/// spec, and the arithmetic for each.
///
/// This is deliberately a second implementation rather than a call into
/// `libviprs::pmtiles`. Its whole job is to say which coordinates a mistake
/// would move, and a copy of the code under test cannot answer that: it would
/// report that nothing moves, which is the answer that makes a probe set look
/// fine.
///
/// The spec's Hilbert step is `rotate`: when `ry == 0`, reflect the quadrant if
/// `rx == 1`, then swap x and y. Each variant below drops or alters one piece
/// of that, which is the shape of every published implementation error I could
/// find. `NoReflect` in particular is the one that matters most here, because
/// it preserves `(x + y) % 2` at every zoom, so a fixture whose payloads
/// alternate on that parity cannot see it at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrongConvention {
    /// The whole rotation dropped.
    NoRotation,
    /// Reflection kept, the x/y swap dropped.
    NoSwap,
    /// Swap kept, the corner reflection dropped.
    NoReflect,
    /// x and y exchanged going in, which is what a column/row mix-up produces.
    Transpose,
}

impl WrongConvention {
    /// Every variant, so a sweep cannot quietly drop one.
    pub const ALL: &'static [WrongConvention] = &[
        WrongConvention::NoRotation,
        WrongConvention::NoSwap,
        WrongConvention::NoReflect,
        WrongConvention::Transpose,
    ];

    /// The name this variant reports itself by in a failure.
    pub fn name(self) -> &'static str {
        match self {
            WrongConvention::NoRotation => "no rotation",
            WrongConvention::NoSwap => "no x/y swap",
            WrongConvention::NoReflect => "no corner reflection",
            WrongConvention::Transpose => "x and y exchanged",
        }
    }
}

fn hilbert_rotate(n: u64, x: &mut u64, y: &mut u64, rx: u64, ry: u64, reflect: bool, swap: bool) {
    if ry == 0 {
        if rx == 1 && reflect {
            *x = n - 1 - *x;
            *y = n - 1 - *y;
        }
        if swap {
            std::mem::swap(x, y);
        }
    }
}

/// The position within the zoom level, under `wrong` or, for `None`, correctly.
///
/// The level base is left off because every comparison here is within one
/// zoom, and leaving it off keeps this from looking like a tile-id function
/// anything should call.
fn hilbert_d(z: u8, x: u32, y: u32, wrong: Option<WrongConvention>) -> u64 {
    let n: u64 = 1 << z;
    let (mut tx, mut ty) = match wrong {
        Some(WrongConvention::Transpose) => (u64::from(y), u64::from(x)),
        _ => (u64::from(x), u64::from(y)),
    };
    let mut d: u64 = 0;
    let mut s: u64 = n >> 1;
    while s > 0 {
        let rx = u64::from((tx & s) > 0);
        let ry = u64::from((ty & s) > 0);
        d += s * s * ((3 * rx) ^ ry);
        match wrong {
            Some(WrongConvention::NoRotation) => {}
            Some(WrongConvention::NoSwap) => {
                hilbert_rotate(n, &mut tx, &mut ty, rx, ry, true, false);
            }
            Some(WrongConvention::NoReflect) => {
                hilbert_rotate(n, &mut tx, &mut ty, rx, ry, false, true);
            }
            _ => hilbert_rotate(n, &mut tx, &mut ty, rx, ry, true, true),
        }
        s >>= 1;
    }
    d
}

/// For every cell of a `side` x `side` zoom level, the cell whose payload a
/// correct reader finds there when the writer used `wrong`.
///
/// A writer with the wrong convention stores `(x, y)` at `wrong(x, y)`. A
/// correct reader asking for `(x, y)` looks at `correct(x, y)` and finds
/// whichever coordinate the writer sent there. Where that is `(x, y)` itself
/// the mistake is invisible at that cell, however many tiles the archive holds.
pub fn what_a_correct_reader_finds(
    z: u8,
    side: u32,
    wrong: WrongConvention,
) -> std::collections::HashMap<(u32, u32), (u32, u32)> {
    let mut placed = std::collections::HashMap::new();
    for y in 0..side {
        for x in 0..side {
            placed.insert(hilbert_d(z, x, y, Some(wrong)), (x, y));
        }
    }
    let mut found = std::collections::HashMap::with_capacity(placed.len());
    for y in 0..side {
        for x in 0..side {
            let at = placed[&hilbert_d(z, x, y, None)];
            found.insert((x, y), at);
        }
    }
    found
}

/// The cells of a `side` x `side` zoom level whose bytes a writer using
/// `wrong` would put somewhere a correct reader does not look.
pub fn cells_a_wrong_convention_moves(z: u8, side: u32, wrong: WrongConvention) -> Vec<(u32, u32)> {
    let mut moved: Vec<(u32, u32)> = what_a_correct_reader_finds(z, side, wrong)
        .into_iter()
        .filter(|(asked, found)| asked != found)
        .map(|(asked, _)| asked)
        .collect();
    moved.sort_unstable();
    moved
}

/// Refuse a probe set that no tile-id mistake would show up in.
///
/// This is the check the epic's own external interop cell was missing. It
/// probed `(0,0)`, `(1,0)`, `(0,1)` and the far corner of a 4x4 level, and all
/// four are fixed points of all four permutations above, so the one place an
/// outside implementation checked our writer could not fail for the reason it
/// existed. A probe set is worth having only if some wrong answer lands in it.
pub fn assert_probes_discriminate(z: u8, side: u32, probes: &[(u32, u32)], what: &str) {
    for wrong in WrongConvention::ALL {
        let moved = cells_a_wrong_convention_moves(z, side, *wrong);
        if moved.is_empty() {
            // Nothing to catch at this zoom: z0 has one cell and every
            // permutation is the identity on it.
            continue;
        }
        let caught: Vec<(u32, u32)> = probes
            .iter()
            .copied()
            .filter(|p| moved.contains(p))
            .collect();
        assert!(
            !caught.is_empty(),
            "{what}: none of the {} probed cells at zoom {z} is one of the {} \
             cells that `{}` moves, so this comparison passes whether or not \
             the archive was written with that convention. The cells it would \
             catch are {:?}",
            probes.len(),
            moved.len(),
            wrong.name(),
            &moved[..moved.len().min(12)],
        );
    }
}
