//! `viprs pyramid`'s pipeline controls and `viprs verify`, end to end
//! (libviprs/libviprs-cli#66).
//!
//! The core has had PMTiles layouts, a dedupe budget, ordered emission,
//! resume, retry, cancellation, progress events, region runs and three verify
//! entry points for a while, and none of it had a flag. This file drives the
//! real `viprs` binary against committed fixtures and holds the CLI to what
//! the library already promises.
//!
//! # Bytes, not exit codes
//!
//! Same rule as `cli_pmtiles.rs`: a cell that asserts exit 0 passes when the
//! command did nothing. So every claim here is checked on an artifact, and
//! every one has a negative control next to it:
//!
//! * the ordered arrival archive is compared byte for byte with the tile-id
//!   archive, and the unordered one is shown to differ, with the header's
//!   `clustered` byte read both ways;
//! * the interrupted run is shown to be incomplete before `--resume` is shown
//!   to finish it to the same bytes an uninterrupted run wrote;
//! * `viprs verify` is shown to pass a clean output before it is shown to fail
//!   one with a single flipped byte, and the failure has to name the tile;
//! * the event stream is counted against the plan, and `--events none` is
//!   shown to print nothing at all.
//!
//! # Inputs
//!
//! Everything starts from committed fixtures: `canonical_input.png` (256x256
//! RGBA) and `extracted_blueprint_portrait.png` (3300x5024 Gray8). The binary
//! is built `--no-default-features` (CLI_CONTRACT.md §7), which has no PDFium,
//! so the PDF fixtures would test a render path this binary does not carry;
//! the rasters extracted from them are what this file reads instead.
//!
//! The two cells that need a feature, the object-store sink and the failure
//! flags it lets us drive, sit behind this crate's `object-store-sink` feature
//! (which `s3` aliases, so `--features s3` runs them too) and build their own
//! `viprs --features s3` into a target directory of their own, so they never
//! replace the shared binary. The stub store is the one output a run does not
//! wipe first, so it is where a tile that can never land gets planted.
//!
//! # What the core does not record yet
//!
//! Neither the manifest nor the archive metadata records `--centre` or
//! `--drop-blanks` (libviprs#1161), so `viprs verify` takes both flags and
//! the verify cells hold it to them. When the core records them the flags
//! become redundant, and these cells are where that shows.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::cli::{cli_available, run_viprs, viprs_bin};

use libviprs::planner::TileCoord;
use libviprs::sink_pmtiles::tile_coord_to_zxy;
use libviprs::{Layout, PmTilesPyramidReader, PyramidPlan, PyramidPlanner, PyramidReader};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Skip guard and plumbing
// ---------------------------------------------------------------------------

/// `true` (with a printed reason) when the CLI sibling is absent. Under
/// `$VIPRS_REQUIRE_CLI=1` [`cli_available`] panics instead, so the
/// cli-differential job cannot go green by not running.
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

fn s(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

/// A committed fixture under `tests/fixtures/`.
fn fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    assert!(
        path.is_file(),
        "missing committed fixture {}",
        path.display()
    );
    path
}

/// Copy a committed fixture into `dir` as `<stem>.png`.
///
/// The copy is what the CLI reads, so an archive's metadata name and a
/// default output name come from a stem this file chose rather than from the
/// fixture's.
fn stage(dir: &Path, name: &str, stem: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("create the staging directory");
    let dst = dir.join(format!("{stem}.png"));
    std::fs::copy(fixture(name), &dst).expect("stage the fixture");
    dst
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Run `viprs` and require exit 0, returning the output for inspection.
fn ok(args: &[&str]) -> Output {
    let out = run_viprs(args);
    assert!(
        out.status.success(),
        "viprs {args:?} exited {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status,
        stdout_of(&out),
        stderr_of(&out),
    );
    out
}

/// Run `viprs` and require the exact exit code `code`.
fn exits(code: i32, args: &[&str]) -> Output {
    let out = run_viprs(args);
    assert_eq!(
        out.status.code(),
        Some(code),
        "viprs {args:?} should exit {code}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        stdout_of(&out),
        stderr_of(&out),
    );
    out
}

/// Run `viprs` and require the exact exit code `code` within `limit`, killing
/// it and failing the cell if it runs longer. For the cells whose failure mode
/// is a run that never ends, where waiting on it would hang the suite instead
/// of failing it.
fn exits_within(code: i32, limit: std::time::Duration, args: &[&str]) -> Output {
    use std::io::Read as _;
    let mut child = Command::new(viprs_bin())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn viprs");
    let drain = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    let stdout = drain(Box::new(child.stdout.take().expect("piped stdout")));
    let stderr = drain(Box::new(child.stderr.take().expect("piped stderr")));
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll viprs") {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("viprs {args:?} was still running after {limit:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let out = Output {
        status,
        stdout: stdout.join().expect("the stdout reader"),
        stderr: stderr.join().expect("the stderr reader"),
    };
    assert_eq!(
        out.status.code(),
        Some(code),
        "viprs {args:?} should exit {code}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        stdout_of(&out),
        stderr_of(&out),
    );
    out
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The PMTiles v3 header's `clustered` byte. Offset 96 is fixed by the spec:
/// seven bytes of magic, the version, eleven little-endian u64s, then this.
fn clustered_byte(archive: &Path) -> u8 {
    let bytes = read(archive);
    assert!(
        bytes.len() > 127 && &bytes[..7] == b"PMTiles",
        "{} is not a PMTiles v3 archive",
        archive.display()
    );
    bytes[96]
}

/// Every file under `dir`, keyed by its `/`-separated relative path.
///
/// The resume bookkeeping (`.libviprs-job.*`) is returned separately. It is
/// the run's state rather than the pyramid, and an uninterrupted run and a
/// resumed one are allowed to differ there and nowhere else.
fn tree(dir: &Path) -> (BTreeMap<String, Vec<u8>>, BTreeSet<String>) {
    fn walk(
        root: &Path,
        dir: &Path,
        files: &mut BTreeMap<String, Vec<u8>>,
        jobs: &mut BTreeSet<String>,
    ) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("list {}: {e}", dir.display()))
            .map(|e| e.expect("dir entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(root, &path, files, jobs);
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .expect("under the root")
                .to_string_lossy()
                .replace('\\', "/");
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if name.starts_with(".libviprs-job") {
                jobs.insert(rel);
            } else {
                files.insert(rel, read(&path));
            }
        }
    }
    let mut files = BTreeMap::new();
    let mut jobs = BTreeSet::new();
    walk(dir, dir, &mut files, &mut jobs);
    (files, jobs)
}

/// The tile files of a tree, which is [`tree`] without the sidecars a sink
/// writes beside the tiles.
fn tiles_only(files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Vec<u8>> {
    files
        .iter()
        .filter(|(rel, _)| rel.ends_with(".png") || rel.ends_with(".raw"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// The deepest Deep Zoom level directory in a tree.
fn top_level(dir: &Path) -> u32 {
    std::fs::read_dir(dir)
        .expect("list the tree")
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .max()
        .expect("a Deep Zoom tree has numbered level directories")
}

/// XOR one byte of `path` at `at`, returning what it was.
fn flip_byte(path: &Path, at: usize) -> u8 {
    let mut bytes = read(path);
    let was = bytes[at];
    bytes[at] ^= 0xFF;
    std::fs::write(path, &bytes).expect("write the damaged copy");
    was
}

/// Number of byte positions at which two equal-length buffers differ.
fn differing_bytes(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len(), "lengths differ");
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

/// Every parsed event line on stdout. Panics on a line that is not a JSON
/// object with a string `event`, because the claim is one parseable line per
/// event, not most of them.
fn json_events(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let v: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("event line is not JSON ({e}): {line:?}"));
            assert!(
                v.get("event").and_then(|e| e.as_str()).is_some(),
                "event line has no string `event` field: {line:?}"
            );
            v
        })
        .collect()
}

fn coord_of(event: &serde_json::Value) -> (u32, u32, u32) {
    let n = |k: &str| {
        event
            .get(k)
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| panic!("event has no numeric {k:?}: {event}")) as u32
    };
    (n("level"), n("col"), n("row"))
}

fn events_named<'a>(events: &'a [serde_json::Value], name: &str) -> Vec<&'a serde_json::Value> {
    events
        .iter()
        .filter(|e| e["event"].as_str() == Some(name))
        .collect()
}

/// Decode a PNG into RGBA8 with the `image` crate, outside the CLI.
fn decode_rgba(path: &Path) -> image::RgbaImage {
    image::open(path)
        .unwrap_or_else(|e| panic!("decode {}: {e}", path.display()))
        .to_rgba8()
}

// ---------------------------------------------------------------------------
// PMTiles layout and ordered emission (libviprs#1145)
// ---------------------------------------------------------------------------

/// The pyramid every layout cell builds: the canonical 256x256 RGBA input at a
/// 32-pixel tile, which in XYZ is nine zoom levels and 90 tiles, enough that
/// the cascade order (full resolution first) and tile id order (zoom 0 first)
/// are nothing alike.
fn layout_run(dir: &Path, extra: &[&str]) -> PathBuf {
    let input = stage(dir, "canonical_input.png", "canonical");
    let archive = dir.join("canonical.pmtiles");
    let mut args = vec!["pyramid", s(&input), s(&archive), "--tile-size", "32"];
    args.extend_from_slice(extra);
    ok(&args);
    assert!(archive.is_file(), "no archive at {}", archive.display());
    archive
}

/// An archive cut into the parts PMTiles v3 defines: the header fields, and
/// the four sections the header points at.
struct Sections {
    header: Vec<u8>,
    root: Vec<u8>,
    metadata: Vec<u8>,
    leaves: Vec<u8>,
    tile_data: Vec<u8>,
}

impl Sections {
    fn read(archive: &Path) -> Self {
        let bytes = read(archive);
        assert!(bytes.len() >= 127 && &bytes[..7] == b"PMTiles");
        let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        let section =
            |off: usize, len: usize| bytes[u64_at(off)..u64_at(off) + u64_at(len)].to_vec();
        Sections {
            header: bytes[..127].to_vec(),
            root: section(8, 16),
            metadata: section(24, 32),
            leaves: section(40, 48),
            tile_data: section(56, 64),
        }
    }

    /// The header with the three section offsets and `clustered` zeroed, which
    /// is all a layout is allowed to move.
    fn header_without_framing(&self) -> Vec<u8> {
        let mut h = self.header.clone();
        for at in [24, 40, 56] {
            h[at..at + 8].fill(0);
        }
        h[96] = 0;
        h
    }
}

/// The #1145 bar, through the CLI: an arrival archive whose tiles were emitted
/// in tile id order holds exactly the tile-id archive's content, and both say
/// they are clustered.
///
/// # Whole-file identity is not the bar, and cannot be
///
/// libviprs#1145 asked for a whole-file match and the core showed it is
/// unreachable by construction (`an_ordered_arrival_run_stores_what_the_tile_id_
/// layout_would_have` in the core's `tests/pmtiles_sink.rs`): `arrival`
/// reserves the first 16384 bytes for the header and root and writes the
/// metadata and leaves after the tile data, `tile-id` writes them before it.
/// Where the sections sit *is* the layout. So this cell holds the CLI to what
/// the core settled on: every section byte for byte, and every header field
/// except the three section offsets and `clustered`. That is every byte of
/// content in the file.
///
/// It also runs the ordered arrival build twice at four workers and requires
/// the two files to be identical outright, which is the determinism ordered
/// emission exists to buy and which a race would break.
#[test]
fn an_ordered_arrival_run_matches_the_tile_id_run_section_for_section() {
    if skip_if_no_cli("an_ordered_arrival_run_matches_the_tile_id_run_section_for_section") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    // Both at four workers: the archive's metadata records the concurrency,
    // so holding everything else fixed means holding that fixed too.
    let tile_id = layout_run(
        &tmp.path().join("tile-id"),
        &["--pmtiles-layout", "tile-id", "--concurrency", "4"],
    );
    let ordered_args = [
        "--pmtiles-layout",
        "arrival",
        "--ordered-emission",
        "--concurrency",
        "4",
    ];
    let ordered = layout_run(&tmp.path().join("arrival-ordered"), &ordered_args);
    let again = layout_run(&tmp.path().join("arrival-ordered-again"), &ordered_args);

    assert_eq!(
        clustered_byte(&tile_id),
        1,
        "the tile-id layout always earns clustered"
    );
    assert_eq!(
        clustered_byte(&ordered),
        1,
        "tiles fed in tile id order earn clustered under Arrival too"
    );

    let (a, b) = (Sections::read(&tile_id), Sections::read(&ordered));
    assert!(
        !a.tile_data.is_empty(),
        "the positive control: an empty archive matches anything"
    );
    assert!(a.tile_data == b.tile_data, "the tile data regions differ");
    assert!(a.root == b.root, "the root directories differ");
    assert!(a.leaves == b.leaves, "the leaf directories differ");
    assert!(a.metadata == b.metadata, "the metadata differs");
    assert_eq!(
        a.header_without_framing(),
        b.header_without_framing(),
        "the headers differ in something other than where the sections sit"
    );
    assert_eq!(
        read(&ordered),
        read(&again),
        "two ordered arrival runs at four workers wrote different archives"
    );
}

/// The negative control for the cell above: without ordered emission the
/// arrival archive is laid out in cascade order, so its tile data differs from
/// the tile-id archive's and it says honestly that it is not clustered. It is
/// still the same pyramid, which is checked tile by tile so "differs" cannot
/// mean "is broken".
#[test]
fn an_unordered_arrival_run_differs_and_is_not_clustered() {
    if skip_if_no_cli("an_unordered_arrival_run_differs_and_is_not_clustered") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let tile_id = layout_run(
        &tmp.path().join("tile-id"),
        &["--pmtiles-layout", "tile-id"],
    );
    let arrival = layout_run(
        &tmp.path().join("arrival"),
        &["--pmtiles-layout", "arrival"],
    );

    assert_eq!(clustered_byte(&tile_id), 1);
    assert_eq!(
        clustered_byte(&arrival),
        0,
        "an arrival archive fed in cascade order must not claim to be clustered"
    );
    let (a, b) = (Sections::read(&tile_id), Sections::read(&arrival));
    assert_eq!(a.tile_data.len(), b.tile_data.len());
    assert!(
        a.tile_data != b.tile_data,
        "the unordered arrival run wrote its tile data in tile id order, so the \
         layout flag or the emission order did not reach the writer"
    );

    let plan = PyramidPlanner::new(256, 256, 32, 0, Layout::Xyz)
        .unwrap()
        .plan();
    let a = PmTilesPyramidReader::try_open(&tile_id).expect("open the tile-id archive");
    let b = PmTilesPyramidReader::try_open(&arrival).expect("open the arrival archive");
    let mut compared = 0;
    for coord in plan.tile_coords() {
        let x = a
            .tile(coord)
            .expect("read")
            .expect("tile-id archive has the tile");
        let y = b
            .tile(coord)
            .expect("read")
            .expect("arrival archive has the tile");
        assert_eq!(x, y, "the two layouts hold different bytes at {coord:?}");
        compared += 1;
    }
    assert_eq!(compared, plan.total_tile_count());
}

/// Leaving the flag off is the tile-id layout, which is what every archive
/// this command wrote before the flag existed looks like.
#[test]
fn the_default_pmtiles_layout_is_tile_id() {
    if skip_if_no_cli("the_default_pmtiles_layout_is_tile_id") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let default = layout_run(&tmp.path().join("default"), &[]);
    let tile_id = layout_run(
        &tmp.path().join("tile-id"),
        &["--pmtiles-layout", "tile-id"],
    );
    assert_eq!(read(&default), read(&tile_id));
}

/// The trap the issue names: `--layout` picks the pyramid scheme, a different
/// enum from the PMTiles layout, and its help has to say so.
#[test]
fn layout_help_points_at_pmtiles_layout_and_says_it_is_not_that() {
    if skip_if_no_cli("layout_help_points_at_pmtiles_layout_and_says_it_is_not_that") {
        return;
    }
    let out = ok(&["pyramid", "--help"]);
    let help = stdout_of(&out);
    let start = help
        .find("--layout <")
        .unwrap_or_else(|| panic!("no --layout in pyramid --help:\n{help}"));
    let rest = &help[start + 1..];
    let end = rest.find("\n  -").unwrap_or(rest.len());
    let block = &help[start..start + 1 + end];
    assert!(
        block.contains("not the PMTiles layout") && block.contains("--pmtiles-layout"),
        "--layout's help must say it is not the PMTiles layout and point at \
         --pmtiles-layout:\n{block}"
    );
    assert!(
        help.contains("--pmtiles-layout <"),
        "pyramid --help has no --pmtiles-layout:\n{help}"
    );
}

/// A budget below what one dedupe set costs is refused with exit 2 and
/// nothing written, rather than clamped to the floor in silence. The floor
/// itself is accepted, which is the control that the refusal is about the
/// number and not about the flag.
#[test]
fn a_dedupe_budget_below_the_floor_is_refused_not_clamped() {
    if skip_if_no_cli("a_dedupe_budget_below_the_floor_is_refused_not_clamped") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");

    let refused = tmp.path().join("refused.pmtiles");
    let out = exits(
        2,
        &[
            "pyramid",
            s(&input),
            s(&refused),
            "--dedupe-memory-bytes",
            "519",
        ],
    );
    let err = stderr_of(&out);
    assert!(
        err.contains("--dedupe-memory-bytes") && err.contains("520"),
        "the refusal must name the flag and the floor:\n{err}"
    );
    assert!(
        !refused.exists(),
        "a refused run wrote {}",
        refused.display()
    );

    let floor = tmp.path().join("floor.pmtiles");
    ok(&[
        "pyramid",
        s(&input),
        s(&floor),
        "--dedupe-memory-bytes",
        "520",
    ]);
    assert!(floor.is_file(), "the floor itself must be accepted");
}

/// Flag combinations with no meaning are usage errors (exit 2), not runs that
/// quietly ignore half of what was asked.
///
/// Exit 2 alone proves nothing here: clap exits 2 for a flag it has never
/// heard of, so a binary without any of these flags would pass. Each refusal
/// therefore has to be about the combination, which means stderr must name the
/// flag and must not be clap's "unexpected argument".
#[test]
fn meaningless_pipeline_combinations_are_usage_errors() {
    if skip_if_no_cli("meaningless_pipeline_combinations_are_usage_errors") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let tree = tmp.path().join("tree");
    let tree_arg = s(&tree).to_string();

    let cases: [(&str, Vec<&str>); 7] = [
        (
            "--pmtiles-layout",
            vec![
                "--storage",
                "directory",
                &tree_arg,
                "--pmtiles-layout",
                "arrival",
            ],
        ),
        (
            "--retries",
            vec!["--retries", "2", "--on-failure", "fail-fast"],
        ),
        ("--retry-backoff-ms", vec!["--retry-backoff-ms", "10"]),
        ("--region", vec!["--region", "1,2,3"]),
        ("--events", vec!["--events", "xml"]),
        // --drop-blanks leaves blanks out and --skip-blank writes a
        // placeholder for each, so asking for both has no meaning.
        ("--drop-blanks", vec!["--drop-blanks", "--skip-blank"]),
        // --skip-failed carries on past a tile and --fail-fast stops at it.
        ("--skip-failed", vec!["--skip-failed", "--fail-fast"]),
    ];
    for (flag, extra) in cases {
        let mut args = vec!["pyramid", s(&input)];
        args.extend(extra);
        let err = stderr_of(&exits(2, &args));
        assert!(
            !err.contains("unexpected argument"),
            "{flag} is not a flag this viprs knows, so the exit 2 is not the refusal \
             under test:\n{err}"
        );
        assert!(
            err.contains(flag),
            "the refusal does not name {flag}:\n{err}"
        );
    }
    assert!(!tree.exists(), "a usage error wrote {}", tree.display());
}

// ---------------------------------------------------------------------------
// SIGINT, SIGKILL and --resume
// ---------------------------------------------------------------------------

/// What every interrupt cell builds: the portrait raster at a 128-pixel tile,
/// which is over a thousand tiles, into a tree, on one worker. One worker
/// keeps exactly one tile in flight, so "at most one tile rewritten" below is
/// a bound rather than a hope.
const INTERRUPT_ARGS: [&str; 6] = [
    "--storage",
    "directory",
    "--tile-size",
    "128",
    "--concurrency",
    "1",
];

/// Wall-clock ceiling on one interrupted run. A run that ignores the signal
/// must fail the cell, not hang the suite.
const INTERRUPT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// The uninterrupted run the interrupt cells compare against: every file, and
/// the tiles on their own.
fn interrupt_reference(
    input: &Path,
    dir: &Path,
) -> (BTreeMap<String, Vec<u8>>, BTreeMap<String, Vec<u8>>) {
    let mut args = vec!["pyramid", s(input), s(dir)];
    args.extend_from_slice(&INTERRUPT_ARGS);
    ok(&args);
    let (files, _) = tree(dir);
    let tiles = tiles_only(&files);
    assert!(
        tiles.len() > 200,
        "the reference pyramid has {} tiles, too few to interrupt part way",
        tiles.len()
    );
    (files, tiles)
}

/// How an interrupted run ended.
#[cfg(unix)]
struct Interrupted {
    status: std::process::ExitStatus,
    stderr: String,
}

/// Start a checkpoint-every-tile run into `out_dir`, send `signal` once the
/// event stream has reported `after` completed tiles, and wait for it to end.
///
/// stderr is drained on a thread of its own and stdout is read through a
/// channel with a deadline, so neither a full pipe nor a run that never stops
/// can hang the cell.
#[cfg(unix)]
fn interrupt_after(input: &Path, out_dir: &Path, signal: &str, after: usize) -> Interrupted {
    use std::io::Read as _;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::Instant;

    let mut child = Command::new(viprs_bin())
        .args(["pyramid", s(input), s(out_dir)])
        .args(INTERRUPT_ARGS)
        .args(["--checkpoint-every", "1", "--events", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn viprs pyramid");
    let pid = child.id().to_string();

    let mut err_pipe = child.stderr.take().expect("piped stderr");
    let stderr = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = err_pipe.read_to_string(&mut text);
        text
    });
    let out_pipe = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out_pipe).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + INTERRUPT_DEADLINE;
    let mut completed = 0usize;
    let mut signalled = false;
    loop {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(line) => {
                if line.contains("\"tile_completed\"") {
                    completed += 1;
                }
                if completed >= after && !signalled {
                    let status = Command::new("kill")
                        .args([format!("-{signal}"), pid.clone()])
                        .status()
                        .expect("run kill");
                    assert!(status.success(), "kill -{signal} {pid} failed");
                    signalled = true;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "viprs pyramid was still running after {INTERRUPT_DEADLINE:?} \
                     ({completed} tiles reported, signalled: {signalled})"
                );
            }
        }
    }
    let status = child.wait().expect("wait for the interrupted run");
    let stderr = stderr.join().expect("the stderr reader");
    assert!(
        signalled,
        "the run finished after {completed} tile events, before {after}, so it was never \
         interrupted\n--- stderr ---\n{stderr}"
    );
    Interrupted { status, stderr }
}

/// The interrupted tree holds a checkpoint and some of the tiles but not all
/// of them, so the resume that follows has something to prove. Returns the
/// tiles it holds.
fn assert_part_way(
    out_dir: &Path,
    reference_tiles: &BTreeMap<String, Vec<u8>>,
) -> BTreeMap<String, Vec<u8>> {
    let (files, jobs) = tree(out_dir);
    let tiles = tiles_only(&files);
    assert!(
        jobs.iter().any(|j| j.ends_with(".libviprs-job.json")),
        "the interrupted run left no checkpoint, so there is nothing to resume: {jobs:?}"
    );
    assert!(
        !tiles.is_empty() && tiles.len() < reference_tiles.len(),
        "the interrupted tree holds {} of {} tiles; it must be part way",
        tiles.len(),
        reference_tiles.len()
    );
    tiles
}

/// `--resume` into `out_dir`, requiring exit 0.
fn resume(input: &Path, out_dir: &Path) -> Output {
    let mut args = vec!["pyramid", s(input), s(out_dir)];
    args.extend_from_slice(&INTERRUPT_ARGS);
    args.extend_from_slice(&["--resume", "--events", "json"]);
    ok(&args)
}

/// The finished tree is the uninterrupted one: the same files, byte for byte,
/// and the same `.dzi` beside it. The resume bookkeeping is left out, since
/// it describes the run rather than the pyramid.
fn assert_same_tree(out_dir: &Path, reference: &Path, reference_files: &BTreeMap<String, Vec<u8>>) {
    let (files, _) = tree(out_dir);
    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        reference_files.keys().collect::<Vec<_>>(),
        "the resumed tree holds a different set of files from the uninterrupted one"
    );
    for (rel, bytes) in reference_files {
        assert!(
            &files[rel] == bytes,
            "{rel} differs between the resumed and the uninterrupted run"
        );
    }
    assert_eq!(
        read(&reference.with_extension("dzi")),
        read(&out_dir.with_extension("dzi")),
        "the .dzi sidecars differ"
    );
}

/// Ctrl-C mid-run exits 130 (the shell's own number for SIGINT, and the
/// README's), says how to finish, and leaves a job `--resume` finishes to the
/// tree an uninterrupted run writes, byte for byte.
///
/// The interrupted run checkpoints after every tile, so it leaves a checkpoint
/// naming exactly what it finished. The signal goes out only after the event
/// stream has reported forty completed tiles, so "mid-way" is observed rather
/// than hoped for, and the tree is shown to be incomplete before the resume
/// runs. Exit 130 exactly, not just non-zero: a panic (101) after the hint
/// would pass a non-zero check.
#[cfg(unix)]
#[test]
fn sigint_exits_130_and_leaves_a_job_that_resume_finishes_byte_identical() {
    if skip_if_no_cli("sigint_exits_130_and_leaves_a_job_that_resume_finishes_byte_identical") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "extracted_blueprint_portrait.png", "portrait");
    let reference = tmp.path().join("reference");
    let out_dir = tmp.path().join("resumed");
    let (reference_files, reference_tiles) = interrupt_reference(&input, &reference);

    let interrupted = interrupt_after(&input, &out_dir, "INT", 40);
    assert_eq!(
        interrupted.status.code(),
        Some(130),
        "Ctrl-C must exit 130, got {}\n--- stderr ---\n{}",
        interrupted.status,
        interrupted.stderr
    );
    assert!(
        interrupted.stderr.contains("--resume"),
        "an interrupted run should say how to finish it:\n{}",
        interrupted.stderr
    );
    let partial_tiles = assert_part_way(&out_dir, &reference_tiles);

    // The tiles the interrupted run finished must be left exactly where they
    // are: a resume that regenerated everything would also end on the right
    // bytes, so the evidence that it resumed is that it did not touch them.
    // (The core never emits `tile_skipped_on_resume`, libviprs#1161, so the
    // event stream cannot say this.)
    let mtime = |rel: &str| {
        std::fs::metadata(out_dir.join(rel))
            .and_then(|m| m.modified())
            .unwrap_or_else(|e| panic!("stat {rel}: {e}"))
    };
    let before: BTreeMap<String, std::time::SystemTime> = partial_tiles
        .keys()
        .map(|rel| (rel.clone(), mtime(rel)))
        .collect();
    let resumed = resume(&input, &out_dir);
    let completed = events_named(&json_events(&stdout_of(&resumed)), "tile_completed").len();
    assert!(completed > 0, "the resumed run reported no work at all");
    // At most one tile may have been rewritten: the one in flight when the
    // signal landed can be on disk without having reached the checkpoint yet,
    // and a single worker has only one tile in flight.
    let rewritten: Vec<&String> = before
        .iter()
        .filter(|(rel, when)| mtime(rel) != **when)
        .map(|(rel, _)| rel)
        .collect();
    assert!(
        rewritten.len() <= 1,
        "--resume rewrote {} of the {} tiles the interrupted run had finished, so it \
         regenerated rather than resumed: {rewritten:?}",
        rewritten.len(),
        before.len()
    );
    assert_same_tree(&out_dir, &reference, &reference_files);
}

/// `kill -9` gives the run no chance to tidy up: no handler, no last
/// checkpoint, whatever tile was in flight cut off wherever it was. The
/// checkpoint and the tiles are written through a temporary file and a
/// rename, so what is on disk is still a job `--resume` can finish, and the
/// finished tree has to be the uninterrupted one with no debris beside it.
#[cfg(unix)]
#[test]
fn sigkill_leaves_a_job_that_resume_finishes_byte_identical() {
    use std::os::unix::process::ExitStatusExt as _;

    if skip_if_no_cli("sigkill_leaves_a_job_that_resume_finishes_byte_identical") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "extracted_blueprint_portrait.png", "portrait");
    let reference = tmp.path().join("reference");
    let out_dir = tmp.path().join("killed");
    let (reference_files, reference_tiles) = interrupt_reference(&input, &reference);

    let killed = interrupt_after(&input, &out_dir, "KILL", 40);
    assert_eq!(
        killed.status.signal(),
        Some(9),
        "the run must have died by SIGKILL, got {}\n--- stderr ---\n{}",
        killed.status,
        killed.stderr
    );
    assert_part_way(&out_dir, &reference_tiles);

    resume(&input, &out_dir);
    assert_same_tree(&out_dir, &reference, &reference_files);
}

/// A checkpoint cut off half way (a full disk, a copy that died) must never
/// be trusted. `--resume` either refuses it with exit 1 and says what is
/// wrong, or ignores it and regenerates, in which case the tree has to come
/// out byte-identical. A panic, or a resume that believes half a list, is the
/// failure.
#[cfg(unix)]
#[test]
fn a_truncated_checkpoint_is_refused_or_regenerated_never_trusted() {
    if skip_if_no_cli("a_truncated_checkpoint_is_refused_or_regenerated_never_trusted") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "extracted_blueprint_portrait.png", "portrait");
    let reference = tmp.path().join("reference");
    let out_dir = tmp.path().join("truncated");
    let (reference_files, reference_tiles) = interrupt_reference(&input, &reference);

    let interrupted = interrupt_after(&input, &out_dir, "INT", 40);
    assert_eq!(
        interrupted.status.code(),
        Some(130),
        "{}",
        interrupted.stderr
    );
    assert_part_way(&out_dir, &reference_tiles);
    let (_, jobs) = tree(&out_dir);
    let checkpoint = jobs
        .iter()
        .find(|j| j.ends_with(".libviprs-job.json"))
        .map(|rel| out_dir.join(rel))
        .expect("a checkpoint");
    let bytes = read(&checkpoint);
    std::fs::write(&checkpoint, &bytes[..bytes.len() / 2]).expect("truncate the checkpoint");

    let mut args = vec!["pyramid", s(&input), s(&out_dir)];
    args.extend_from_slice(&INTERRUPT_ARGS);
    args.push("--resume");
    let out = run_viprs(&args);
    let err = stderr_of(&out);
    assert!(
        !err.contains("panicked"),
        "a truncated checkpoint panicked viprs:\n{err}"
    );
    match out.status.code() {
        Some(0) => assert_same_tree(&out_dir, &reference, &reference_files),
        Some(1) => assert!(
            err.contains("checkpoint") || err.contains(".libviprs-job"),
            "the refusal must say it is about the checkpoint:\n{err}"
        ),
        other => panic!("--resume over a truncated checkpoint exited {other:?}:\n{err}"),
    }
}

// ---------------------------------------------------------------------------
// viprs verify
// ---------------------------------------------------------------------------

/// The `Tiles: N` line `viprs verify` prints on success.
fn verified_count(stdout: &str) -> u64 {
    let line = stdout
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("Tiles:"))
        .unwrap_or_else(|| panic!("no `Tiles:` line in verify output:\n{stdout}"));
    line.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("`Tiles:{line}` is not a count"))
}

/// An archive: clean passes and counts every planned tile; one flipped byte
/// inside one tile's payload fails, exit 1, naming that tile.
#[test]
fn verify_passes_a_clean_archive_and_names_the_tile_with_a_flipped_byte() {
    if skip_if_no_cli("verify_passes_a_clean_archive_and_names_the_tile_with_a_flipped_byte") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let clean = tmp.path().join("clean.pmtiles");
    ok(&["pyramid", s(&input), s(&clean), "--tile-size", "64"]);
    let plan = PyramidPlanner::new(256, 256, 64, 0, Layout::Xyz)
        .unwrap()
        .plan();

    let out = ok(&["verify", s(&clean)]);
    assert_eq!(verified_count(&stdout_of(&out)), plan.total_tile_count());

    // A full-resolution tile, (1, 2) on the top level, named the way PMTiles
    // names it. Its payload has to occur exactly once in the file, or flipping
    // it would damage some other tile as well.
    let top = plan.levels.last().expect("a plan has levels").level;
    let coord = TileCoord::new(top, 1, 2);
    let (z, x, y) = tile_coord_to_zxy(coord).expect("an xyz coordinate has a z/x/y");
    let name = format!("{z}/{x}/{y}");
    let payload = PmTilesPyramidReader::try_open(&clean)
        .unwrap()
        .tile(coord)
        .unwrap()
        .unwrap_or_else(|| panic!("the archive holds {name}"));
    let bytes = read(&clean);
    let hits: Vec<usize> = bytes
        .windows(payload.len())
        .enumerate()
        .filter(|(_, w)| *w == payload.as_slice())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "tile {name}'s payload is not unique in the archive"
    );

    let damaged = tmp.path().join("damaged.pmtiles");
    std::fs::copy(&clean, &damaged).unwrap();
    flip_byte(&damaged, hits[0] + payload.len() / 2);
    assert_eq!(differing_bytes(&read(&clean), &read(&damaged)), 1);

    let out = exits(1, &["verify", s(&damaged)]);
    let err = stderr_of(&out);
    assert!(
        err.contains(&name),
        "verify must name the damaged tile {name}:\n{err}"
    );
}

/// A loose tree with per-tile checksums: clean passes, and one flipped byte in
/// one tile file fails naming that file.
#[test]
fn verify_passes_a_clean_tree_and_names_the_tile_with_a_flipped_byte() {
    if skip_if_no_cli("verify_passes_a_clean_tree_and_names_the_tile_with_a_flipped_byte") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let dir = tmp.path().join("tree");
    ok(&[
        "pyramid",
        s(&input),
        s(&dir),
        "--storage",
        "directory",
        "--tile-size",
        "64",
        "--checksum",
    ]);
    let plan = PyramidPlanner::new(256, 256, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let out = ok(&["verify", s(&dir)]);
    assert_eq!(verified_count(&stdout_of(&out)), plan.total_tile_count());

    let rel = format!("{}/1_1.png", top_level(&dir));
    let tile = dir.join(&rel);
    let before = read(&tile);
    flip_byte(&tile, before.len() / 2);
    assert_eq!(differing_bytes(&before, &read(&tile)), 1);

    let out = exits(1, &["verify", s(&dir)]);
    let err = stderr_of(&out);
    assert!(
        err.contains(&rel),
        "verify must name the damaged tile {rel}:\n{err}"
    );
}

/// `--source` re-renders the pyramid from the input and compares, which is the
/// only check that can tell a tree was made from a different image. Raw tiles,
/// because an encoded tile cannot be compared against a re-render byte for
/// byte. The control is the same input with one pixel changed, and the
/// failure has to name the one tile that pixel lands in.
///
/// The tree records its source digest (`--manifest-source-hash`), which is
/// what lets verify re-render a different file at all (libviprs-cli#103): the
/// run folds the input's digest into the plan hash its checkpoint records, and
/// the checkpoint holds only the hash. A tree that did not record it still
/// fails a different source, but as "made from a different source" without
/// naming a tile, and says what to write the tree with.
#[test]
fn verify_with_source_catches_a_tree_made_from_a_different_image() {
    if skip_if_no_cli("verify_with_source_catches_a_tree_made_from_a_different_image") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let dir = tmp.path().join("tree");
    ok(&[
        "pyramid",
        s(&input),
        s(&dir),
        "--storage",
        "directory",
        "--tile-size",
        "64",
        "--format",
        "raw",
        "--checksum",
        "--manifest-source-hash",
    ]);

    let out = ok(&["verify", s(&dir), "--source", s(&input)]);
    assert!(verified_count(&stdout_of(&out)) > 0);

    // One pixel, inside top-level tile 2_3, nudged.
    let mut other = decode_rgba(&input);
    let px = other.get_pixel_mut(150, 200);
    px.0[0] = px.0[0].wrapping_add(17);
    let other_path = tmp.path().join("other.png");
    other.save(&other_path).expect("write the altered input");

    let out = exits(1, &["verify", s(&dir), "--source", s(&other_path)]);
    let err = stderr_of(&out);
    let top = top_level(&dir);
    assert!(
        err.contains(&format!("{top}/2_3")),
        "verify --source must name the tile the changed pixel is in ({top}/2_3):\n{err}"
    );

    // Without a recorded digest the different source is still refused, and
    // the message says how to get a tree verify can name the tile for.
    let plain = tmp.path().join("unrecorded");
    ok(&[
        "pyramid",
        s(&input),
        s(&plain),
        "--storage",
        "directory",
        "--tile-size",
        "64",
        "--format",
        "raw",
        "--checksum",
    ]);
    ok(&["verify", s(&plain), "--source", s(&input)]);
    let err = stderr_of(&exits(
        1,
        &["verify", s(&plain), "--source", s(&other_path)],
    ));
    assert!(
        err.contains("made from a different source") && err.contains("--manifest-source-hash"),
        "a tree without a recorded digest must say it was made from a different source and \
         point at --manifest-source-hash:\n{err}"
    );
}

/// A tree written with `--checksum`, for the verify cells that damage one.
fn checksummed_tree(tmp: &Path) -> (PathBuf, PyramidPlan) {
    let input = stage(tmp, "canonical_input.png", "canonical");
    let dir = tmp.join("tree");
    ok(&[
        "pyramid",
        s(&input),
        s(&dir),
        "--storage",
        "directory",
        "--tile-size",
        "64",
        "--checksum",
    ]);
    let plan = PyramidPlanner::new(256, 256, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    (dir, plan)
}

/// A deleted tile and an emptied one are each named, with exit 1. Neither is
/// a flipped byte, so these hold the presence pass and the size pass to
/// account, not only the checksum pass.
#[test]
fn verify_names_a_deleted_tile_and_an_empty_tile_in_a_tree() {
    if skip_if_no_cli("verify_names_a_deleted_tile_and_an_empty_tile_in_a_tree") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let (dir, plan) = checksummed_tree(tmp.path());
    let out = ok(&["verify", s(&dir)]);
    assert_eq!(verified_count(&stdout_of(&out)), plan.total_tile_count());

    let top = top_level(&dir);
    let deleted = format!("{top}/2_1.png");
    std::fs::remove_file(dir.join(&deleted)).expect("delete a tile");
    let err = stderr_of(&exits(1, &["verify", s(&dir)]));
    assert!(
        err.contains(&deleted) && err.contains("missing"),
        "verify must name the deleted tile {deleted} as missing:\n{err}"
    );
    assert!(
        err.contains("--drop-blanks"),
        "a missing tile on a verify without --drop-blanks should say what else it could \
         mean:\n{err}"
    );

    let emptied = format!("{top}/0_3.png");
    std::fs::write(dir.join(&emptied), b"").expect("empty a tile");
    let err = stderr_of(&exits(1, &["verify", s(&dir)]));
    assert!(
        err.contains(&emptied) && err.contains("empty"),
        "verify must name the empty tile {emptied}:\n{err}"
    );
}

/// The manifest copies a tree carries, beside it and inside it, whichever
/// exist. verify reads the first; a hostile edit goes into both so the cell
/// does not depend on which.
fn manifest_copies(dir: &Path) -> Vec<PathBuf> {
    let sibling = dir.with_file_name(format!(
        "{}.manifest.json",
        dir.file_name().unwrap().to_string_lossy()
    ));
    let copies: Vec<PathBuf> = [sibling, dir.join("manifest.json")]
        .into_iter()
        .filter(|p| p.is_file())
        .collect();
    assert!(!copies.is_empty(), "{} has no manifest", dir.display());
    copies
}

fn edit_manifests(dir: &Path, edit: impl Fn(&mut serde_json::Value)) {
    for path in manifest_copies(dir) {
        let mut m: serde_json::Value =
            serde_json::from_slice(&read(&path)).expect("manifest.json parses");
        edit(&mut m);
        std::fs::write(&path, serde_json::to_vec_pretty(&m).unwrap()).expect("write it back");
    }
}

/// A manifest is input like any other. One that claims a source far bigger
/// than the tree, one with a tile size no plan can have, and one that is not
/// JSON at all are each exit 1 with a message, quickly, and never a panic.
/// The oversized one is where the problem cap earns its keep: millions of
/// missing tiles are reported as fifty and a line saying there are more.
/// Each run gets a minute, so a verify without the cap fails the cell rather
/// than hanging the suite walking a trillion tiles.
#[test]
fn verify_refuses_hostile_manifests_with_exit_1_and_never_panics() {
    if skip_if_no_cli("verify_refuses_hostile_manifests_with_exit_1_and_never_panics") {
        return;
    }
    let limit = std::time::Duration::from_secs(60);

    let tmp = TempDir::new().unwrap();
    let (dir, _) = checksummed_tree(&tmp.path().join("huge"));
    edit_manifests(&dir, |m| {
        m["source"]["width"] = 1_000_000.into();
        m["source"]["height"] = 1_000_000.into();
    });
    let err = stderr_of(&exits_within(1, limit, &["verify", s(&dir)]));
    let named = err.lines().filter(|l| l.contains("is missing")).count();
    assert_eq!(
        named, 50,
        "a manifest naming millions of tiles must be reported as 50 problems:\n{err}"
    );
    assert!(
        err.contains("stopped after 50 problems"),
        "the report must say it stopped looking:\n{err}"
    );

    let (dir, _) = checksummed_tree(&tmp.path().join("zero"));
    edit_manifests(&dir, |m| m["generation"]["tile_size"] = 0.into());
    let err = stderr_of(&exits_within(1, limit, &["verify", s(&dir)]));
    assert!(
        err.contains("no plan can describe"),
        "a tile size of 0 must be refused as a plan that cannot exist:\n{err}"
    );

    let (dir, _) = checksummed_tree(&tmp.path().join("garbage"));
    for path in manifest_copies(&dir) {
        std::fs::write(&path, b"{\"version\": 1, \"generation\": [").unwrap();
    }
    let err = stderr_of(&exits_within(1, limit, &["verify", s(&dir)]));
    assert!(
        err.contains("manifest"),
        "an unreadable manifest must be named:\n{err}"
    );
}

/// The canonical image in the top-left corner of a white canvas twice its
/// size, so a quarter of the full-resolution tiles have content and the rest
/// are blank.
fn sparse_input(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let src = decode_rgba(&fixture("canonical_input.png"));
    let mut canvas = image::RgbaImage::from_pixel(512, 512, image::Rgba([255, 255, 255, 255]));
    image::imageops::replace(&mut canvas, &src, 0, 0);
    let input = dir.join("sparse.png");
    canvas.save(&input).expect("write the sparse input");
    input
}

/// A `--drop-blanks` tree leaves its blank tiles out on purpose, and its
/// manifest says so (`skip_blanks`, libviprs#1162), so verify passes it with
/// no flag (libviprs-cli#85). It still catches a kept tile that goes
/// missing, which is the control that dropped blanks did not simply switch
/// the presence check off. A manifest from before the field reads as no
/// blanks dropped, so for one of those verify fails the dropped blanks as
/// missing, says what that could mean, and passes once told `--drop-blanks`.
#[test]
fn verify_reads_drop_blanks_from_a_tree_and_still_catches_a_lost_tile() {
    if skip_if_no_cli("verify_reads_drop_blanks_from_a_tree_and_still_catches_a_lost_tile") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = sparse_input(tmp.path());
    let dir = tmp.path().join("dropped");
    ok(&[
        "pyramid",
        s(&input),
        s(&dir),
        "--storage",
        "directory",
        "--tile-size",
        "128",
        "--drop-blanks",
        "--checksum",
    ]);

    for told in [&[][..], &["--drop-blanks"][..]] {
        let mut args = vec!["verify", s(&dir)];
        args.extend_from_slice(told);
        let out = ok(&args);
        assert!(
            stdout_of(&out).contains("blank tiles dropped"),
            "the summary should say blanks were allowed for ({told:?}):\n{}",
            stdout_of(&out)
        );
    }
    // The re-render skips a dropped blank too (libviprs#1174).
    for told in [&[][..], &["--drop-blanks"][..]] {
        let mut args = vec!["verify", s(&dir), "--source", s(&input)];
        args.extend_from_slice(told);
        let out = ok(&args);
        assert!(
            stdout_of(&out).contains("re-render"),
            "--source on a --drop-blanks tree was not re-rendered ({told:?}):\n{}",
            stdout_of(&out)
        );
    }

    // A manifest from before `skip_blanks` existed: verify has to be told.
    let old = tmp.path().join("old");
    ok(&[
        "pyramid",
        s(&input),
        s(&old),
        "--storage",
        "directory",
        "--tile-size",
        "128",
        "--drop-blanks",
        "--checksum",
    ]);
    edit_manifests(&old, |m| {
        m["generation"]
            .as_object_mut()
            .unwrap()
            .remove("skip_blanks");
    });
    let err = stderr_of(&exits(1, &["verify", s(&old)]));
    assert!(
        err.contains("missing") && err.contains("--drop-blanks"),
        "verify of a tree whose manifest predates skip_blanks must call the dropped blanks \
         missing and point at the flag:\n{err}"
    );
    ok(&["verify", s(&old), "--drop-blanks"]);
    ok(&["verify", s(&old), "--drop-blanks", "--source", s(&input)]);

    // Top-level tile 0_0 is the canonical image itself, so it is kept.
    let kept = format!("{}/0_0.png", top_level(&dir));
    std::fs::remove_file(dir.join(&kept)).expect("delete a kept tile");
    for told in [&[][..], &["--drop-blanks"][..]] {
        let mut args = vec!["verify", s(&dir)];
        args.extend_from_slice(told);
        let err = stderr_of(&exits(1, &args));
        assert!(
            err.contains(&kept),
            "verify must still name a kept tile that went missing ({kept}, {told:?}):\n{err}"
        );
    }
}

/// The archive half of the cell above: the same input into PMTiles with
/// `--drop-blanks` records it in the archive's `vnd.libviprs` metadata, so
/// verify passes it told or not.
#[test]
fn verify_reads_drop_blanks_from_an_archive() {
    if skip_if_no_cli("verify_reads_drop_blanks_from_an_archive") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = sparse_input(tmp.path());
    let archive = tmp.path().join("dropped.pmtiles");
    ok(&[
        "pyramid",
        s(&input),
        s(&archive),
        "--tile-size",
        "128",
        "--drop-blanks",
    ]);
    let full = tmp.path().join("full.pmtiles");
    ok(&["pyramid", s(&input), s(&full), "--tile-size", "128"]);
    assert!(
        read(&archive).len() < read(&full).len(),
        "--drop-blanks dropped nothing from the archive, so this cell proves nothing"
    );

    let out = ok(&["verify", s(&archive)]);
    assert!(
        stdout_of(&out).contains("blank tiles dropped"),
        "the summary should say blanks were allowed for:\n{}",
        stdout_of(&out)
    );
    ok(&["verify", s(&archive), "--drop-blanks"]);
    ok(&["verify", s(&full)]);
}

/// A non-square input, so `--centre` moves the image on the XYZ canvas.
fn wide_input(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let src = decode_rgba(&fixture("canonical_input.png"));
    let wide = image::imageops::crop_imm(&src, 0, 64, 256, 96).to_image();
    let input = dir.join("wide.png");
    wide.save(&input).expect("write the wide input");
    input
}

/// A `--centre` pyramid is checked against the centred plan, archive and
/// tree alike, and the count it reports is the centred plan's. The manifest
/// and the archive record the centring (libviprs#1162), so verify needs no
/// flag; `--centre` is only for a pyramid written before that.
///
/// The centred plan names the same tiles as the uncentred one for every
/// layout `pyramid` accepts with `--centre` (measured: google and xyz,
/// 200x70 and 256x96 at a 64-pixel tile), so only `--source` sees the
/// difference. The re-render lays a source out on a centred grid
/// (libviprs#1163), so the tree is also re-rendered here, told and untold,
/// and once more from a manifest that predates `centre`, where it takes the
/// flag to pass.
#[test]
fn verify_checks_a_centred_pyramid_against_the_centred_grid() {
    if skip_if_no_cli("verify_checks_a_centred_pyramid_against_the_centred_grid") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = wide_input(tmp.path());
    let centred = |layout: Layout| {
        PyramidPlanner::new(256, 96, 64, 0, layout)
            .unwrap()
            .with_centre(true)
            .plan()
            .total_tile_count()
    };

    let archive = tmp.path().join("centred.pmtiles");
    ok(&[
        "pyramid",
        s(&input),
        s(&archive),
        "--tile-size",
        "64",
        "--centre",
    ]);
    let out = ok(&["verify", s(&archive), "--centre"]);
    assert_eq!(verified_count(&stdout_of(&out)), centred(Layout::Xyz));

    for (layout, name) in [(Layout::Xyz, "xyz"), (Layout::Google, "google")] {
        let dir = tmp.path().join(format!("centred-{name}"));
        ok(&[
            "pyramid",
            s(&input),
            s(&dir),
            "--storage",
            "directory",
            "--layout",
            name,
            "--tile-size",
            "64",
            "--centre",
            "--checksum",
        ]);
        for told in [&[][..], &["--centre"][..]] {
            let mut args = vec!["verify", s(&dir), "--source", s(&input)];
            args.extend_from_slice(told);
            let out = ok(&args);
            assert_eq!(verified_count(&stdout_of(&out)), centred(layout), "{name}");
            assert!(
                stdout_of(&out).contains("re-render"),
                "the {name} tree was not re-rendered ({told:?}):\n{}",
                stdout_of(&out)
            );
        }
        let out = ok(&["verify", s(&dir)]);
        assert_eq!(verified_count(&stdout_of(&out)), centred(layout), "{name}");
        assert!(
            stdout_of(&out).contains("against its checksum"),
            "the {name} tree's tiles were not checked against their checksums:\n{}",
            stdout_of(&out)
        );
    }
}

/// `--source` re-renders the pyramid, and neither `--drop-blanks` nor
/// `--centre` is refused beside it any more (libviprs-cli#85): the re-render
/// skips a dropped blank (libviprs#1174) and lays a source out on a centred
/// grid (libviprs#1163). On this plain tree `--drop-blanks` changes nothing,
/// so it passes, and the centred re-render does not match, which is exit 1
/// for the input, not exit 2 for the command line.
#[test]
fn verify_source_takes_drop_blanks_and_centre() {
    if skip_if_no_cli("verify_source_takes_drop_blanks_and_centre") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let (dir, _) = checksummed_tree(tmp.path());
    let input = tmp.path().join("canonical.png");
    ok(&["verify", s(&dir), "--source", s(&input)]);
    ok(&["verify", s(&dir), "--source", s(&input), "--drop-blanks"]);
    let err = stderr_of(&exits(
        1,
        &["verify", s(&dir), "--source", s(&input), "--centre"],
    ));
    assert!(
        !err.contains("cannot be combined"),
        "--centre --source must not be refused any more:\n{err}"
    );
}

// ---------------------------------------------------------------------------
// --events
// ---------------------------------------------------------------------------

/// One parseable JSON line per event, and the completed tiles are exactly the
/// plan's tiles: the same count, and no coordinate twice.
#[test]
fn events_json_is_one_line_per_event_and_completes_every_planned_tile() {
    if skip_if_no_cli("events_json_is_one_line_per_event_and_completes_every_planned_tile") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let archive = tmp.path().join("canonical.pmtiles");
    let out = ok(&[
        "pyramid",
        s(&input),
        s(&archive),
        "--tile-size",
        "32",
        "--events",
        "json",
    ]);
    let events = json_events(&stdout_of(&out));

    // The JSON lines are a public contract: each one carries the schema
    // version, and its name comes from a fixed table rather than from a
    // `Debug` rendering that could change under it. The table is the core's
    // events plus the one line the CLI adds itself, the closing `summary`.
    const NAMES: [&str; 20] = [
        "summary",
        "source_load_started",
        "source_loaded",
        "plan_created",
        "level_started",
        "tile_completed",
        "tile_failed",
        "tile_skipped_on_resume",
        "retry_attempted",
        "level_completed",
        "strip_rendered",
        "batch_started",
        "batch_completed",
        "strip_dispatched",
        "strip_executor_done",
        "worker_joined",
        "worker_left",
        "memory_snapshot",
        "checkpoint_flushed",
        "finished",
    ];
    for event in &events {
        assert_eq!(
            event.get("v").and_then(|v| v.as_u64()),
            Some(1),
            "every event line carries \"v\":1: {event}"
        );
        let name = event["event"].as_str().unwrap();
        assert!(
            NAMES.contains(&name),
            "{name:?} is not one of the documented event names: {event}"
        );
    }
    for name in [
        "level_started",
        "level_completed",
        "tile_completed",
        "summary",
    ] {
        assert!(
            !events_named(&events, name).is_empty(),
            "a whole run reports {name}"
        );
    }

    let plan = PyramidPlanner::new(256, 256, 32, 0, Layout::Xyz)
        .unwrap()
        .plan();
    let completed = events_named(&events, "tile_completed");
    assert_eq!(
        completed.len() as u64,
        plan.total_tile_count(),
        "tile_completed events must equal the plan's tile count"
    );
    let seen: BTreeSet<_> = completed.iter().map(|e| coord_of(e)).collect();
    let planned: BTreeSet<_> = plan
        .tile_coords()
        .map(|c| (c.level, c.col, c.row))
        .collect();
    assert_eq!(
        seen, planned,
        "the completed coordinates are not the plan's coordinates"
    );
}

/// The default prints no events, and `text` prints one line per event too.
#[test]
fn events_none_is_silent_and_text_is_one_line_per_event() {
    if skip_if_no_cli("events_none_is_silent_and_text_is_one_line_per_event") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let plan = PyramidPlanner::new(256, 256, 32, 0, Layout::Xyz)
        .unwrap()
        .plan();

    let none = ok(&[
        "pyramid",
        s(&input),
        s(&tmp.path().join("a.pmtiles")),
        "--tile-size",
        "32",
    ]);
    assert!(
        none.stdout.is_empty(),
        "no --events must print nothing on stdout, got:\n{}",
        stdout_of(&none)
    );

    let text = ok(&[
        "pyramid",
        s(&input),
        s(&tmp.path().join("b.pmtiles")),
        "--tile-size",
        "32",
        "--events",
        "text",
    ]);
    let completed = stdout_of(&text)
        .lines()
        .filter(|l| l.starts_with("tile_completed "))
        .count() as u64;
    assert_eq!(completed, plan.total_tile_count());
}

// ---------------------------------------------------------------------------
// --region, --drop-blanks, --manifest-source-hash
// ---------------------------------------------------------------------------

/// A region run is crop-then-pyramid, so it is byte-identical to a plain run
/// over the input cropped beforehand, and not to a run over the whole input.
#[test]
fn a_region_run_equals_a_run_over_the_cropped_input() {
    if skip_if_no_cli("a_region_run_equals_a_run_over_the_cropped_input") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(&tmp.path().join("full"), "canonical_input.png", "canonical");
    let (x, y, w, h) = (32u32, 16u32, 160u32, 128u32);

    let crop = image::imageops::crop_imm(&decode_rgba(&input), x, y, w, h).to_image();
    std::fs::create_dir_all(tmp.path().join("crop")).unwrap();
    let cropped = tmp.path().join("crop/canonical.png");
    crop.save(&cropped).expect("write the cropped input");

    let region = tmp.path().join("full/out.pmtiles");
    ok(&[
        "pyramid",
        s(&input),
        s(&region),
        "--tile-size",
        "64",
        "--region",
        &format!("{x},{y},{w},{h}"),
    ]);
    let precropped = tmp.path().join("crop/out.pmtiles");
    ok(&["pyramid", s(&cropped), s(&precropped), "--tile-size", "64"]);
    let whole = tmp.path().join("whole.pmtiles");
    ok(&["pyramid", s(&input), s(&whole), "--tile-size", "64"]);

    assert_eq!(
        read(&region),
        read(&precropped),
        "a region run is not crop-then-pyramid"
    );
    assert_ne!(read(&region), read(&whole), "--region did nothing");
}

/// `--drop-blanks` drops uniform tiles altogether, which is not what the older
/// `--skip-blank` does (that one writes a placeholder per blank tile). The
/// input is [`sparse_input`], so a quarter of the full-resolution tiles have
/// content and the rest are blank.
#[test]
fn drop_blanks_drops_blank_tiles_and_keeps_the_rest_byte_identical() {
    if skip_if_no_cli("drop_blanks_drops_blank_tiles_and_keeps_the_rest_byte_identical") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = sparse_input(tmp.path());

    let full = tmp.path().join("full");
    let skipped = tmp.path().join("skipped");
    let placeholder = tmp.path().join("placeholder");
    let base = ["--storage", "directory", "--tile-size", "128"];
    for (dir, extra) in [
        (&full, None),
        (&skipped, Some("--drop-blanks")),
        (&placeholder, Some("--skip-blank")),
    ] {
        let mut args = vec!["pyramid", s(&input), s(dir)];
        args.extend_from_slice(&base);
        args.extend(extra);
        ok(&args);
    }
    let full_tiles = tiles_only(&tree(&full).0);
    let skipped_tiles = tiles_only(&tree(&skipped).0);
    let placeholder_tiles = tiles_only(&tree(&placeholder).0);

    assert!(
        skipped_tiles.len() < full_tiles.len(),
        "--drop-blanks dropped nothing ({} of {} tiles)",
        skipped_tiles.len(),
        full_tiles.len()
    );
    assert_eq!(
        placeholder_tiles.len(),
        full_tiles.len(),
        "--skip-blank keeps a file per tile, which is the difference between the two flags"
    );
    for (rel, bytes) in &skipped_tiles {
        assert_eq!(
            Some(bytes),
            full_tiles.get(rel),
            "{rel} changed under --drop-blanks"
        );
    }
    for (rel, bytes) in &full_tiles {
        if skipped_tiles.contains_key(rel) {
            continue;
        }
        let img = image::load_from_memory(bytes)
            .unwrap_or_else(|e| panic!("decode {rel}: {e}"))
            .to_rgba8();
        let first = img.pixels().next().copied();
        assert!(
            img.pixels().all(|p| Some(*p) == first),
            "--drop-blanks dropped {rel}, which is not blank"
        );
    }
}

/// `--manifest-source-hash` records the BLAKE3 of the source file's bytes in
/// the manifest, which is what the core documents `bytes_hash` to be ("the raw
/// source bytes"), and leaving it off records nothing. The digest of the
/// decoded pixels is a different number, and it is checked not to be the one
/// recorded, so the cell cannot pass on the old semantics. An input read from
/// stdin leaves no file to hash and is refused as a usage error.
#[test]
fn manifest_source_hash_records_the_source_digest() {
    if skip_if_no_cli("manifest_source_hash_records_the_source_digest") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let expected = blake3::hash(&read(&input)).to_hex().to_string();
    let raster = libviprs::decode_file(&input).expect("decode the input");
    let pixels = blake3::hash(raster.data()).to_hex().to_string();
    assert_ne!(
        expected, pixels,
        "the two digests must differ to tell them apart"
    );

    let manifest = |dir: &Path| -> serde_json::Value {
        serde_json::from_slice(&read(&dir.join("manifest.json"))).expect("manifest.json parses")
    };
    let with = tmp.path().join("with");
    ok(&[
        "pyramid",
        s(&input),
        s(&with),
        "--storage",
        "directory",
        "--checksum",
        "--manifest-source-hash",
    ]);
    assert_eq!(
        manifest(&with)["source"]["bytes_hash"].as_str(),
        Some(expected.as_str()),
        "the manifest does not carry the source digest"
    );

    let without = tmp.path().join("without");
    ok(&[
        "pyramid",
        s(&input),
        s(&without),
        "--storage",
        "directory",
        "--checksum",
    ]);
    assert!(manifest(&without)["source"]["bytes_hash"].is_null());

    let stdin = tmp.path().join("stdin");
    let out = Command::new(viprs_bin())
        .args([
            "pyramid",
            "-",
            s(&stdin),
            "--storage",
            "directory",
            "--manifest-source-hash",
        ])
        .stdin(std::fs::File::open(&input).unwrap())
        .output()
        .expect("run viprs");
    assert_eq!(out.status.code(), Some(2), "{}", stderr_of(&out));
    assert!(
        stderr_of(&out).contains("--manifest-source-hash") && stderr_of(&out).contains("stdin"),
        "the refusal must name the flag and stdin:\n{}",
        stderr_of(&out)
    );
    assert!(!stdin.exists(), "a refused run wrote {}", stdin.display());
}

// ---------------------------------------------------------------------------
// The object-store sink, and the failure flags it lets us drive
// ---------------------------------------------------------------------------

/// The shared binary is built without the `s3` feature, and a feature a build
/// left out is exit 1 everywhere (README, "Exit codes"), naming the feature.
/// The stub store's directory flag is a test seam and stays out of the help.
#[test]
fn an_s3_sink_without_the_feature_exits_1_and_the_stub_flag_is_hidden() {
    if skip_if_no_cli("an_s3_sink_without_the_feature_exits_1_and_the_stub_flag_is_hidden") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let out = run_viprs(&[
        "pyramid",
        s(&input),
        "--sink",
        "s3://tiles/run-1",
        "--tile-size",
        "64",
    ]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a missing feature is an operational failure, exit 1:\n{}",
        stderr_of(&out)
    );
    assert!(
        stderr_of(&out).contains("`s3` feature"),
        "the refusal must name the feature to rebuild with:\n{}",
        stderr_of(&out)
    );

    let help = stdout_of(&ok(&["pyramid", "--help"]));
    assert!(
        !help.contains("--object-store-root"),
        "--object-store-root is a hidden test seam, not a flag to point a job at:\n{help}"
    );
}

/// `viprs` built with `--features s3`, into a target directory of its own so
/// the shared `--no-default-features` binary every other cell runs is never
/// replaced.
#[cfg(feature = "object-store-sink")]
fn viprs_s3_bin() -> PathBuf {
    use std::sync::OnceLock;
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let cli = common::cli::cli_dir();
        let target = cli.join("target/pipeline-s3");
        let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
            .args([
                "build",
                "--release",
                "--no-default-features",
                "--features",
                "s3",
                "--bin",
                "viprs",
            ])
            .arg("--manifest-path")
            .arg(cli.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(&target)
            .env_remove("RUSTFLAGS")
            .status()
            .expect("spawn cargo build --features s3");
        assert!(
            status.success(),
            "building viprs --features s3 failed: {status}"
        );
        target.join("release/viprs")
    })
    .clone()
}

#[cfg(feature = "object-store-sink")]
fn run_s3(args: &[&str]) -> Output {
    Command::new(viprs_s3_bin())
        .args(args)
        .output()
        .expect("run viprs")
}

/// `--sink s3://bucket/prefix` against the local stub store writes the same
/// tiles a directory run writes, under the bucket and prefix it was given.
/// Without a store to write to it is refused rather than pretending to upload.
#[cfg(feature = "object-store-sink")]
#[test]
fn the_object_store_sink_writes_the_directory_tiles_into_the_stub_store() {
    if skip_if_no_cli("the_object_store_sink_writes_the_directory_tiles_into_the_stub_store") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let store = tmp.path().join("store");

    let refused = run_s3(&[
        "pyramid",
        s(&input),
        "--sink",
        "s3://tiles/run-1",
        "--tile-size",
        "64",
    ]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr_of(&refused));
    assert!(stderr_of(&refused).contains("--object-store-root"));

    let out = run_s3(&[
        "pyramid",
        s(&input),
        "--sink",
        "s3://tiles/run-1",
        "--object-store-root",
        s(&store),
        "--tile-size",
        "64",
    ]);
    assert!(out.status.success(), "{}", stderr_of(&out));

    let dir = tmp.path().join("tree");
    ok(&[
        "pyramid",
        s(&input),
        s(&dir),
        "--storage",
        "directory",
        "--tile-size",
        "64",
    ]);
    let expected = tiles_only(&tree(&dir).0);
    let uploaded = tiles_only(&tree(&store.join("tiles/run-1/canonical_files")).0);
    assert!(!expected.is_empty());
    assert_eq!(
        uploaded.keys().collect::<Vec<_>>(),
        expected.keys().collect::<Vec<_>>(),
        "the store holds a different set of keys from the directory run"
    );
    assert_eq!(
        uploaded, expected,
        "the uploaded tiles differ from the directory tiles"
    );
}

/// A stub store with a directory squatting where one tile goes, so every
/// write of that tile fails and every other write lands. The store is the
/// one output the run never wipes first, which is what makes it the place to
/// plant a failure. Returns the store, the input, the reference tiles a clean
/// run writes and the key of the tile that cannot land.
#[cfg(feature = "object-store-sink")]
fn store_with_a_blocked_tile(
    tmp: &Path,
    name: &str,
) -> (PathBuf, PathBuf, BTreeMap<String, Vec<u8>>, String) {
    let input = stage(tmp, "canonical_input.png", "canonical");
    let reference = tmp.join("reference");
    ok(&[
        "pyramid",
        s(&input),
        s(&reference),
        "--storage",
        "directory",
        "--tile-size",
        "64",
    ]);
    let reference_tiles = tiles_only(&tree(&reference).0);
    // An interior full-resolution tile, so neither the first write nor the
    // last is the one that fails.
    let blocked = format!("{}/1_1.png", top_level(&reference));
    let store = tmp.join(name);
    let squatter = store.join("tiles/run-1/canonical_files").join(&blocked);
    std::fs::create_dir_all(&squatter).expect("plant the squatting directory");
    std::fs::write(squatter.join("keep"), b"x").expect("and make it non-empty");
    (store, input, reference_tiles, blocked)
}

#[cfg(feature = "object-store-sink")]
fn run_into_store(store: &Path, input: &Path, extra: &[&str]) -> Output {
    let mut args = vec![
        "pyramid",
        s(input),
        "--sink",
        "s3://tiles/run-1",
        "--object-store-root",
        s(store),
        "--tile-size",
        "64",
    ];
    args.extend_from_slice(extra);
    run_s3(&args)
}

/// `--retries N` retries and then FAILS the run, exit 1, the same as no flag
/// at all; it never quietly skips. Only `--skip-failed` turns a tile that
/// keeps failing into a hole, and then the run carries on, writes every other
/// tile exactly as a clean run would, and still exits 1 saying how many it
/// skipped, because the output has a hole in it. With both, the retries are
/// counted before the skip.
#[cfg(feature = "object-store-sink")]
#[test]
fn retries_then_fail_and_only_skip_failed_skips_with_exit_1() {
    if skip_if_no_cli("retries_then_fail_and_only_skip_failed_skips_with_exit_1") {
        return;
    }
    let tmp = TempDir::new().unwrap();

    for (name, extra) in [
        ("plain", vec![]),
        ("retries", vec!["--retries", "2", "--retry-backoff-ms", "1"]),
        ("fail-fast", vec!["--fail-fast"]),
    ] {
        let (store, input, _, blocked) = store_with_a_blocked_tile(&tmp.path().join(name), name);
        let out = run_into_store(&store, &input, &extra);
        let err = stderr_of(&out);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{name}: a tile that never lands must fail the run:\n{err}"
        );
        assert!(
            !err.contains("were skipped"),
            "{name}: only --skip-failed may skip a tile:\n{err}"
        );
        assert!(
            err.contains(&blocked) || err.contains("generating pyramid"),
            "{name}: the failure must say what failed:\n{err}"
        );
    }

    for (name, extra, retries) in [
        ("skip", vec!["--skip-failed", "--events", "json"], None),
        (
            "retry-skip",
            vec![
                "--retries",
                "2",
                "--retry-backoff-ms",
                "1",
                "--skip-failed",
                "--events",
                "json",
            ],
            Some(2),
        ),
    ] {
        let (store, input, reference, blocked) =
            store_with_a_blocked_tile(&tmp.path().join(name), name);
        let out = run_into_store(&store, &input, &extra);
        let err = stderr_of(&out);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{name}: a run that skipped a tile has a hole and must exit 1:\n{err}"
        );
        assert!(
            err.contains("1 tiles were skipped"),
            "{name}: the run must say how many tiles it skipped:\n{err}"
        );
        if let Some(n) = retries {
            assert!(
                err.contains(&format!("Retries: {n},")),
                "{name}: the {n} retries must be counted before the skip:\n{err}"
            );
        }

        let failed = events_named(&json_events(&stdout_of(&out)), "tile_failed")
            .into_iter()
            .map(coord_of)
            .collect::<Vec<_>>();
        let top = blocked.split('/').next().unwrap().parse::<u32>().unwrap();
        assert_eq!(
            failed,
            vec![(top, 1, 1)],
            "{name}: exactly the blocked tile is reported failed"
        );

        let mut want = reference;
        want.remove(&blocked);
        let got = tiles_only(&tree(&store.join("tiles/run-1/canonical_files")).0);
        assert_eq!(
            got.keys().collect::<Vec<_>>(),
            want.keys().collect::<Vec<_>>(),
            "{name}: every tile but the blocked one must have landed"
        );
        assert!(
            got == want,
            "{name}: the tiles that landed differ from a clean run"
        );
    }
}
