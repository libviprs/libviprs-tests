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
//! The one cell that needs a feature, the object-store sink, sits behind this
//! crate's `object-store-sink` feature (which `s3` aliases, so `--features s3`
//! runs it too) and builds its own `viprs --features s3` into a target
//! directory of its own, so it never replaces the shared binary.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::cli::{cli_available, run_viprs, viprs_bin};

use libviprs::planner::TileCoord;
use libviprs::sink_pmtiles::tile_coord_to_zxy;
use libviprs::{Layout, PmTilesPyramidReader, PyramidPlanner, PyramidReader};

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
fn an_ordered_arrival_run_is_byte_identical_to_the_tile_id_run() {
    if skip_if_no_cli("an_ordered_arrival_run_is_byte_identical_to_the_tile_id_run") {
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

    let cases: [(&str, Vec<&str>); 5] = [
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
// SIGINT and --resume
// ---------------------------------------------------------------------------

/// Ctrl-C mid-run leaves a job `--resume` can finish, and the finished tree is
/// the tree an uninterrupted run writes, byte for byte.
///
/// The interrupted run checkpoints after every tile, so it is slow enough to
/// interrupt and leaves a checkpoint naming exactly what it finished. The
/// signal goes out only after the event stream has reported forty completed
/// tiles, so "mid-way" is observed rather than hoped for, and the tree is
/// shown to be incomplete before the resume runs.
#[test]
fn sigint_leaves_a_job_that_resume_finishes_byte_identical() {
    if skip_if_no_cli("sigint_leaves_a_job_that_resume_finishes_byte_identical") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "extracted_blueprint_portrait.png", "portrait");
    let reference = tmp.path().join("reference");
    let out_dir = tmp.path().join("resumed");
    let common = ["--storage", "directory", "--tile-size", "128"];

    // The uninterrupted run everything is compared against.
    let mut args = vec!["pyramid", s(&input), s(&reference)];
    args.extend_from_slice(&common);
    ok(&args);
    let (reference_files, _) = tree(&reference);
    let reference_tiles = tiles_only(&reference_files);
    assert!(
        reference_tiles.len() > 200,
        "the reference pyramid has {} tiles, too few to interrupt part way",
        reference_tiles.len()
    );

    // The run that gets interrupted.
    let mut child = Command::new(viprs_bin())
        .args(["pyramid", s(&input), s(&out_dir)])
        .args(common)
        .args(["--checkpoint-every", "1", "--events", "json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn viprs pyramid");
    let pid = child.id().to_string();
    let stdout = child.stdout.take().expect("piped stdout");
    let mut completed = 0usize;
    let mut signalled = false;
    for line in BufReader::new(stdout).lines() {
        let line = line.expect("read an event line");
        if line.contains("\"tile_completed\"") {
            completed += 1;
        }
        if completed == 40 && !signalled {
            let status = Command::new("kill")
                .args(["-INT", &pid])
                .status()
                .expect("run kill -INT");
            assert!(status.success(), "kill -INT {pid} failed");
            signalled = true;
        }
    }
    let interrupted = child
        .wait_with_output()
        .expect("wait for the interrupted run");
    assert!(
        signalled,
        "the run finished after {completed} tile events, before 40, so it was never interrupted"
    );
    assert!(
        !interrupted.status.success(),
        "an interrupted run must exit non-zero, got {}\n--- stderr ---\n{}",
        interrupted.status,
        stderr_of(&interrupted)
    );
    assert!(
        stderr_of(&interrupted).contains("--resume"),
        "an interrupted run should say how to finish it:\n{}",
        stderr_of(&interrupted)
    );

    // Mid-way, observed: a checkpoint, and not the whole pyramid.
    let (partial_files, partial_jobs) = tree(&out_dir);
    let partial_tiles = tiles_only(&partial_files);
    assert!(
        partial_jobs
            .iter()
            .any(|j| j.ends_with(".libviprs-job.json")),
        "the interrupted run left no checkpoint, so there is nothing to resume: {partial_jobs:?}"
    );
    assert!(
        !partial_tiles.is_empty() && partial_tiles.len() < reference_tiles.len(),
        "the interrupted tree holds {} of {} tiles; it must be part way",
        partial_tiles.len(),
        reference_tiles.len()
    );
    assert_ne!(partial_tiles, reference_tiles);

    // Resume. The tiles the interrupted run finished must be left exactly
    // where they are: a resume that regenerated everything would also end on
    // the right bytes, so the evidence that it resumed is that it did not
    // touch them. (The core defines a `tile_skipped_on_resume` event and never
    // emits it, so the event stream cannot say this.)
    let mtime = |rel: &str| {
        std::fs::metadata(out_dir.join(rel))
            .and_then(|m| m.modified())
            .unwrap_or_else(|e| panic!("stat {rel}: {e}"))
    };
    let before: BTreeMap<String, std::time::SystemTime> = partial_tiles
        .keys()
        .map(|rel| (rel.clone(), mtime(rel)))
        .collect();
    let mut args = vec!["pyramid", s(&input), s(&out_dir)];
    args.extend_from_slice(&common);
    args.extend_from_slice(&["--resume", "--events", "json"]);
    let resumed = ok(&args);
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

    let (resumed_files, _) = tree(&out_dir);
    assert_eq!(
        resumed_files.keys().collect::<Vec<_>>(),
        reference_files.keys().collect::<Vec<_>>(),
        "the resumed tree holds a different set of files from the uninterrupted one"
    );
    for (rel, bytes) in &reference_files {
        assert!(
            &resumed_files[rel] == bytes,
            "{rel} differs between the resumed and the uninterrupted run"
        );
    }
    assert_eq!(
        read(&reference.with_extension("dzi")),
        read(&out_dir.with_extension("dzi")),
        "the .dzi sidecars differ"
    );
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
// --region, --skip-blanks, --manifest-source-hash
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

/// `--skip-blanks` drops uniform tiles altogether, which is not what the older
/// `--skip-blank` does (that one writes a placeholder per blank tile). The
/// input is the canonical image in the top-left corner of a white canvas
/// twice its size, so a quarter of the full-resolution tiles have content and
/// the rest are blank.
#[test]
fn skip_blanks_drops_blank_tiles_and_keeps_the_rest_byte_identical() {
    if skip_if_no_cli("skip_blanks_drops_blank_tiles_and_keeps_the_rest_byte_identical") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let src = decode_rgba(&fixture("canonical_input.png"));
    let mut canvas = image::RgbaImage::from_pixel(512, 512, image::Rgba([255, 255, 255, 255]));
    image::imageops::replace(&mut canvas, &src, 0, 0);
    let input = tmp.path().join("sparse.png");
    canvas.save(&input).expect("write the sparse input");

    let full = tmp.path().join("full");
    let skipped = tmp.path().join("skipped");
    let placeholder = tmp.path().join("placeholder");
    let base = ["--storage", "directory", "--tile-size", "128"];
    for (dir, extra) in [
        (&full, None),
        (&skipped, Some("--skip-blanks")),
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
        "--skip-blanks dropped nothing ({} of {} tiles)",
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
            "{rel} changed under --skip-blanks"
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
            "--skip-blanks dropped {rel}, which is not blank"
        );
    }
}

/// `--manifest-source-hash` records the BLAKE3 of the decoded source pixels in
/// the manifest, and leaving it off records nothing.
#[test]
fn manifest_source_hash_records_the_source_digest() {
    if skip_if_no_cli("manifest_source_hash_records_the_source_digest") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let raster = libviprs::decode_file(&input).expect("decode the input");
    let expected = blake3::hash(raster.data()).to_hex().to_string();

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
}

// ---------------------------------------------------------------------------
// The object-store sink, under the `s3` cell
// ---------------------------------------------------------------------------

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

/// `--sink s3://bucket/prefix` against the local stub store writes the same
/// tiles a directory run writes, under the bucket and prefix it was given.
/// Without a store to write to it is refused rather than pretending to upload.
#[cfg(feature = "object-store-sink")]
#[test]
fn the_object_store_sink_writes_the_directory_tiles_into_the_stub_store() {
    if skip_if_no_cli("the_object_store_sink_writes_the_directory_tiles_into_the_stub_store") {
        return;
    }
    let bin = viprs_s3_bin();
    let run = |args: &[&str]| Command::new(&bin).args(args).output().expect("run viprs");

    let tmp = TempDir::new().unwrap();
    let input = stage(tmp.path(), "canonical_input.png", "canonical");
    let store = tmp.path().join("store");

    let refused = run(&[
        "pyramid",
        s(&input),
        "--sink",
        "s3://tiles/run-1",
        "--tile-size",
        "64",
    ]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr_of(&refused));
    assert!(stderr_of(&refused).contains("--object-store-root"));

    let out = run(&[
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
