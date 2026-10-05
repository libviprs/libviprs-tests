//! The `viprs` built-in commands end to end, with the exit-code contract
//! (libviprs-tests#232).
//!
//! The op families have their own vips-differential suites (`cli_*_diff.rs`),
//! and several built-ins have suites of their own that go deep on one area:
//! `cli_pmtiles.rs` (the archive against the library), `cli_features.rs`
//! (feature builds and the missing-feature refusal), `cli_pdf_geo_plan.rs`
//! (`pdf`, `geo` and the `plan` queries) and `cli_pyramid_pipeline.rs`
//! (`pyramid`'s pipeline controls and `verify`). This file is the one place
//! that walks every built-in once, and it holds what none of those own:
//!
//! - `pyramid` from a PDF and from a raster, into a tile tree and into a
//!   `.pmtiles`, each checked tile by tile against the committed `vips dzsave`
//!   reference at tolerance 0;
//! - `info` on every format the bare build decodes, its `--json` parsed and
//!   held against what the library decodes in this process;
//! - `plan`, `test-image` and `pmtiles pack`, which have no suite here;
//! - the exit-code contract (README, "Exit codes"; `CLI_CONTRACT.md` §8) for
//!   every built-in: 0 for success, 2 for a usage mistake (a missing required
//!   argument or mode flag included), 1 for an operational failure (a missing
//!   feature included), 130 for Ctrl-C;
//! - the `--no-default-features` binary refusing PDF rendering with its typed
//!   message, exit 1.
//!
//! Every run goes through [`run`], which fails the cell on a panic whatever
//! the exit code: a process killed by a signal, exit 101 (the Rust panic
//! status), or a `panicked at` on stderr. A panic is never assumed absent.
//!
//! The binary is the one `tests/common/cli.rs` builds: `--release
//! --no-default-features`, so there is no pdfium in it, which is exactly the
//! configuration the PDF refusal cells need.

mod common;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::cli::{cli_available, viprs_bin};
use common::dzsave_expected::{assert_tiles_pixel_equal_tol, collect_files};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn skip_if_no_cli(test: &str) -> bool {
    if cli_available() {
        return false;
    }
    eprintln!(
        "SKIP {test}: libviprs-cli sibling not found (set $VIPRS_CLI_DIR or $VIPRS_BIN); \
         under VIPRS_REQUIRE_CLI=1 this is a failure instead"
    );
    true
}

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

fn s(path: &Path) -> &str {
    path.to_str().expect("test paths are UTF-8")
}

/// One finished `viprs` run.
struct Run {
    args: Vec<String>,
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    fn describe(&self) -> String {
        format!(
            "viprs {:?} exited {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
            self.args, self.code, self.stdout, self.stderr
        )
    }
}

/// Fail the cell if `out` shows a panic, whatever else it shows.
///
/// Three ways a panic gets out of a Rust binary, and each is checked rather
/// than trusted to be absent: killed by a signal (an abort, or the stack
/// overflow guard), exit 101 (what the runtime exits with after an unwinding
/// panic in `main`), and the `panicked at` line the default hook prints, which
/// also catches a panic on a worker thread that `main` then turned into some
/// other exit code.
fn assert_no_panic(args: &[String], out: &Output) -> i32 {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let Some(code) = out.status.code() else {
        panic!(
            "viprs {args:?} was killed by a signal ({:?}), which is a crash, not an exit code\n{stderr}",
            out.status
        );
    };
    assert_ne!(
        code, 101,
        "viprs {args:?} exited 101, the status of a panic\n{stderr}"
    );
    assert!(
        !stderr.contains("panicked at") && !stderr.contains("RUST_BACKTRACE"),
        "viprs {args:?} panicked (exit {code}):\n{stderr}"
    );
    code
}

/// Run `viprs` with `args` and check it did not panic.
fn run(args: &[&str]) -> Run {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let out = Command::new(viprs_bin())
        .args(&args)
        .env_remove("VIPRS_PDF_PASSWORD")
        .output()
        .unwrap_or_else(|e| panic!("failed to run viprs {args:?}: {e}"));
    let code = assert_no_panic(&args, &out);
    Run {
        args,
        code,
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// Run and require exit 0.
fn ok(args: &[&str]) -> Run {
    let r = run(args);
    assert_eq!(r.code, 0, "{}", r.describe());
    r
}

/// Run and require `code`, with every needle on stderr.
fn exits(code: i32, args: &[&str], needles: &[&str]) -> Run {
    let r = run(args);
    assert_eq!(r.code, code, "expected exit {code}: {}", r.describe());
    for needle in needles {
        assert!(
            r.stderr.contains(needle),
            "expected `{needle}` on stderr: {}",
            r.describe()
        );
    }
    r
}

fn scratch() -> TempDir {
    tempfile::tempdir().expect("a temp dir")
}

// ---------------------------------------------------------------------------
// pyramid: four routes, each against the committed vips dzsave reference
// ---------------------------------------------------------------------------

/// `blueprint-mix.pdf` holds one embedded 12738x220 RGB image, which is
/// `extracted_blueprint_mix.png`; `blueprint_mix_expected/` is `vips dzsave`
/// of that PNG at tile 256, overlap 0 (tests/fixtures/README.md). The core's
/// own `blueprint_mix_pyramid.rs` matches it at tolerance 0, so the CLI has
/// to as well.
const MIX_PDF: &str = "blueprint-mix.pdf";
const MIX_PNG: &str = "extracted_blueprint_mix.png";
const MIX_EXPECTED: &str = "blueprint_mix_expected";

fn mix_reference() -> Vec<(String, Vec<u8>)> {
    let files = collect_files(&fixture(MIX_EXPECTED), "png");
    assert_eq!(
        files.len(),
        110,
        "tests/fixtures/{MIX_EXPECTED} should hold the 110 dzsave tiles the README describes"
    );
    files
}

/// The `<Size>` and tile fields of a `.dzi` descriptor, which is what a viewer
/// reads; whitespace and attribute order are not part of it.
fn dzi_fields(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for key in ["Format", "Overlap", "TileSize", "Height", "Width"] {
        let needle = format!("{key}=\"");
        let at = xml
            .find(&needle)
            .unwrap_or_else(|| panic!("no {key} in the descriptor:\n{xml}"));
        let rest = &xml[at + needle.len()..];
        let end = rest.find('"').expect("a closing quote");
        out.push((key.to_string(), rest[..end].to_string()));
    }
    out
}

/// `pyramid INPUT DIR --storage directory`, the Deep Zoom tree, compared tile
/// by tile with the reference, and its descriptor with the reference's.
fn assert_tree_matches_reference(input: &Path, context: &str) {
    let tmp = scratch();
    let out = tmp.path().join("mix");
    ok(&["pyramid", s(input), s(&out), "--storage", "directory"]);

    let actual = collect_files(&out, "png");
    let max = assert_tiles_pixel_equal_tol(&mix_reference(), &actual, context, 0);
    assert_eq!(max, 0, "{context}");

    let dzi = std::fs::read_to_string(tmp.path().join("mix.dzi"))
        .unwrap_or_else(|e| panic!("{context}: no mix.dzi beside the tree: {e}"));
    let reference = std::fs::read_to_string(fixture("blueprint_mix_expected.dzi")).unwrap();
    assert_eq!(dzi_fields(&dzi), dzi_fields(&reference), "{context}");
}

/// `pyramid INPUT OUT.pmtiles`, unpacked with `pmtiles extract`, compared tile
/// by tile with the reference.
///
/// The archive's xyz levels are numbered the way Deep Zoom numbers them, so
/// `z/x/y.png` in the archive is `z/x_y.png` in the reference, and the same
/// 110 tiles have to come back. Unpacking through the CLI rather than reading
/// the archive with the library keeps this a statement about what a user of
/// `viprs` gets.
fn assert_archive_matches_reference(archive: &Path, context: &str) {
    let tmp = scratch();
    let tree = tmp.path().join("unpacked");
    let r = ok(&["pmtiles", "extract", s(archive), s(&tree)]);
    assert!(
        r.stdout.contains("Extracted 110 tiles"),
        "{context}: {}",
        r.describe()
    );
    let mut actual: Vec<(String, Vec<u8>)> = collect_files(&tree, "png")
        .into_iter()
        .map(|(rel, bytes)| {
            let parts: Vec<&str> = rel.trim_end_matches(".png").split('/').collect();
            assert_eq!(parts.len(), 3, "{context}: {rel} is not z/x/y.png");
            (format!("{}/{}_{}.png", parts[0], parts[1], parts[2]), bytes)
        })
        .collect();
    actual.sort_by(|a, b| a.0.cmp(&b.0));
    let max = assert_tiles_pixel_equal_tol(&mix_reference(), &actual, context, 0);
    assert_eq!(max, 0, "{context}");

    ok(&["pmtiles", "verify", s(archive)]);
}

#[test]
fn pyramid_from_a_pdf_into_a_tree_matches_vips_dzsave() {
    if skip_if_no_cli("pyramid_from_a_pdf_into_a_tree_matches_vips_dzsave") {
        return;
    }
    assert_tree_matches_reference(&fixture(MIX_PDF), "PDF into a tree");
}

#[test]
fn pyramid_from_a_raster_into_a_tree_matches_vips_dzsave() {
    if skip_if_no_cli("pyramid_from_a_raster_into_a_tree_matches_vips_dzsave") {
        return;
    }
    assert_tree_matches_reference(&fixture(MIX_PNG), "raster into a tree");
}

#[test]
fn pyramid_from_a_pdf_into_an_archive_matches_vips_dzsave() {
    if skip_if_no_cli("pyramid_from_a_pdf_into_an_archive_matches_vips_dzsave") {
        return;
    }
    let tmp = scratch();
    let archive = tmp.path().join("mix.pmtiles");
    ok(&["pyramid", s(&fixture(MIX_PDF)), s(&archive)]);
    assert_archive_matches_reference(&archive, "PDF into an archive");
}

#[test]
fn pyramid_from_a_raster_with_no_output_writes_the_archive_beside_it() {
    if skip_if_no_cli("pyramid_from_a_raster_with_no_output_writes_the_archive_beside_it") {
        return;
    }
    // The default route: no output named, the archive takes the input's name.
    let tmp = scratch();
    let input = tmp.path().join("drawing.png");
    std::fs::copy(fixture(MIX_PNG), &input).unwrap();
    let r = ok(&["pyramid", s(&input)]);
    let archive = tmp.path().join("drawing.pmtiles");
    assert!(
        archive.is_file(),
        "no drawing.pmtiles beside the input: {}",
        r.describe()
    );
    assert_archive_matches_reference(&archive, "raster into the default archive");
}

#[test]
fn pyramid_failures_follow_the_exit_code_contract() {
    if skip_if_no_cli("pyramid_failures_follow_the_exit_code_contract") {
        return;
    }
    let tmp = scratch();
    let png = tmp.path().join("in.png");
    std::fs::copy(fixture("canonical_input.png"), &png).unwrap();
    let garbage = tmp.path().join("garbage.png");
    std::fs::write(&garbage, b"this is not a PNG").unwrap();
    let missing = tmp.path().join("missing.png");
    let archive = tmp.path().join("out.pmtiles");
    let (p, a) = (s(&png), s(&archive));

    // Usage mistakes: 2, and nothing written.
    let usage: &[(&[&str], &[&str])] = &[
        (&["pyramid"], &["required"]),
        (&["pyramid", p, a, "--bogus"], &["--bogus"]),
        (
            &["pyramid", p, "--storage", "directory"],
            &["--storage directory has no output directory"],
        ),
        (
            &["pyramid", p, a, "--layout", "deep-zoom"],
            &["--layout deep-zoom cannot be addressed inside a PMTiles archive"],
        ),
        (
            &["pyramid", p, a, "--format", "raw"],
            &["--format raw has no tile type in a PMTiles archive"],
        ),
        (
            &["pyramid", "-"],
            &["leaves no name to derive the archive from"],
        ),
        (&["pyramid", p, a, "--tile-size", "0"], &["--tile-size"]),
        (&["pyramid", p, a, "--overlap", "256"], &["--overlap"]),
        (&["pyramid", p, a, "--page", "0"], &["--page"]),
        (
            &["pyramid", p, a, "--format", "jpeg", "--quality", "101"],
            &["--quality"],
        ),
    ];
    for (args, needles) in usage {
        exits(2, args, needles);
        assert!(!archive.exists(), "viprs {args:?} wrote {a}");
    }
    let looks_like_dir = format!("{}/", s(&tmp.path().join("tiles")));
    exits(
        2,
        &["pyramid", p, &looks_like_dir],
        &["looks like a directory"],
    );

    // Operational failures: 1.
    exits(1, &["pyramid", s(&missing), a], &["not found"]);
    exits(1, &["pyramid", s(&garbage), a], &["Error"]);
    exits(
        1,
        &["pyramid", s(&fixture(MIX_PDF)), a, "--page", "9"],
        &["page 9 out of range"],
    );
    assert!(!archive.exists(), "a failed pyramid wrote {a}");
}

#[test]
fn a_bare_build_refuses_pdf_rendering_with_the_typed_message() {
    if skip_if_no_cli("a_bare_build_refuses_pdf_rendering_with_the_typed_message") {
        return;
    }
    // tests/common/cli.rs builds --no-default-features, which leaves pdfium
    // out. A feature this build lacks is a 1 (the command line was fine), and
    // the message says which feature and what to do instead.
    let features = ok(&["features"]);
    assert!(
        !features.stdout.lines().any(|l| l.trim() == "pdfium"),
        "these cells need a binary without pdfium, and this one lists it:\n{}",
        features.stdout
    );
    let tmp = scratch();
    let archive = tmp.path().join("r.pmtiles");
    exits(
        1,
        &["pyramid", s(&fixture(MIX_PDF)), s(&archive), "--render"],
        &["--render needs the `pdfium` feature", "omit --render"],
    );
    assert!(!archive.exists());
    let png = tmp.path().join("page.png");
    exits(
        1,
        &[
            "pdf",
            "extract",
            s(&fixture(MIX_PDF)),
            s(&png),
            "--dpi",
            "150",
        ],
        &["--dpi needs the `pdfium` feature"],
    );
    assert!(!png.exists());
    // Without a render option the embedded image still comes out.
    ok(&["pdf", "extract", s(&fixture(MIX_PDF)), s(&png)]);
    let page = image::open(&png).expect("the extracted page decodes");
    assert_eq!((page.width(), page.height()), (12738, 220));
}

// ---------------------------------------------------------------------------
// info: every format the bare build decodes, text and JSON
// ---------------------------------------------------------------------------

/// One committed fixture per format the `--no-default-features` binary
/// decodes. The four codecs behind cargo features (avif, svg, jxl, jp2k) are
/// `cli_features.rs`'s; this is everything that needs no feature.
const INFO_IMAGES: &[(&str, &str)] = &[
    ("png", "canonical_input.png"),
    (
        "jpeg",
        "decode_limits_max_coord_enforcement_expected/oversized_200x100.jpg",
    ),
    (
        "tiff",
        "decode_limits_max_coord_enforcement_expected/oversized_200x100.tif",
    ),
    ("ppm", "cli/iocleanup/flip_ppm_expected.ppm"),
    ("pgm", "cli/iocleanup/flip_pgm_expected.pgm"),
    ("vips .v", "cli/create/xyz_expected.v"),
];

fn info_json(path: &Path) -> serde_json::Value {
    let r = ok(&["info", "--json", s(path)]);
    serde_json::from_str(&r.stdout).unwrap_or_else(|e| {
        panic!(
            "info --json printed something that is not JSON ({e}): {}",
            r.describe()
        )
    })
}

#[test]
fn info_json_agrees_with_the_library_on_every_image_format() {
    if skip_if_no_cli("info_json_agrees_with_the_library_on_every_image_format") {
        return;
    }
    for (format, rel) in INFO_IMAGES {
        let path = fixture(rel);
        let raster = libviprs::decode_file(&path)
            .unwrap_or_else(|e| panic!("the library cannot decode the {format} fixture: {e}"));
        let json = info_json(&path);
        let ctx = format!("{format} ({rel}): {json}");
        assert_eq!(json["v"], 1, "{ctx}");
        assert_eq!(json["kind"], "image", "{ctx}");
        assert_eq!(json["path"], s(&path), "{ctx}");
        assert_eq!(json["width"], raster.width(), "{ctx}");
        assert_eq!(json["height"], raster.height(), "{ctx}");
        assert_eq!(json["format"], format!("{:?}", raster.format()), "{ctx}");
        assert_eq!(json["bytes"], raster.data().len(), "{ctx}");
    }
}

#[test]
fn info_json_agrees_with_the_library_on_a_pdf() {
    if skip_if_no_cli("info_json_agrees_with_the_library_on_a_pdf") {
        return;
    }
    for rel in [MIX_PDF, "canonical_rotated_90.pdf"] {
        let path = fixture(rel);
        let info = libviprs::pdf_info(&path).expect("the library reads the PDF");
        let json = info_json(&path);
        assert_eq!(json["v"], 1, "{rel}: {json}");
        assert_eq!(json["kind"], "pdf", "{rel}: {json}");
        assert_eq!(json["pages"], info.page_count, "{rel}: {json}");
        let sizes = json["page_sizes"].as_array().expect("page_sizes");
        assert_eq!(sizes.len(), info.pages.len(), "{rel}: {json}");
        for (got, want) in sizes.iter().zip(&info.pages) {
            assert_eq!(got["page"], want.page_number, "{rel}: {json}");
            assert_eq!(
                got["width_pts"].as_f64(),
                Some(want.width_pts),
                "{rel}: {json}"
            );
            assert_eq!(
                got["height_pts"].as_f64(),
                Some(want.height_pts),
                "{rel}: {json}"
            );
            assert_eq!(got["has_images"], want.has_images, "{rel}: {json}");
        }
    }
}

#[test]
fn info_text_names_the_dimensions_and_the_format() {
    if skip_if_no_cli("info_text_names_the_dimensions_and_the_format") {
        return;
    }
    let r = ok(&["info", s(&fixture("canonical_input.png"))]);
    assert!(r.stdout.contains("Dimensions: 256x256"), "{}", r.describe());
    assert!(r.stdout.contains("Format: Rgba8"), "{}", r.describe());
    let r = ok(&["info", s(&fixture(MIX_PDF))]);
    assert!(r.stdout.contains("Pages: 1"), "{}", r.describe());
    assert!(r.stdout.contains("(has images)"), "{}", r.describe());
}

#[test]
fn info_failures_follow_the_exit_code_contract() {
    if skip_if_no_cli("info_failures_follow_the_exit_code_contract") {
        return;
    }
    let tmp = scratch();
    let garbage_png = tmp.path().join("garbage.png");
    std::fs::write(&garbage_png, b"not an image").unwrap();
    let garbage_pdf = tmp.path().join("garbage.pdf");
    std::fs::write(&garbage_pdf, b"not a pdf").unwrap();

    exits(2, &["info"], &["required"]);
    exits(2, &["info", s(&garbage_png), "--bogus"], &["--bogus"]);
    exits(
        1,
        &["info", s(&tmp.path().join("missing.png"))],
        &["File not found"],
    );
    exits(1, &["info", s(&garbage_png)], &["Error reading image"]);
    exits(1, &["info", s(&garbage_pdf)], &["Error reading PDF"]);
    exits(1, &["info", s(tmp.path())], &["Error reading image"]);
    // A JSON caller gets nothing on stdout to mistake for an answer.
    for bad in [&garbage_png, &garbage_pdf] {
        let r = exits(1, &["info", "--json", s(bad)], &["Error"]);
        assert!(r.stdout.is_empty(), "{}", r.describe());
    }
}

// ---------------------------------------------------------------------------
// plan
// ---------------------------------------------------------------------------

/// `Levels: N, total tiles: M` from the text output.
fn plan_totals(stdout: &str) -> (u32, u64) {
    let line = stdout
        .lines()
        .find(|l| l.starts_with("Levels:"))
        .unwrap_or_else(|| panic!("no Levels: line in\n{stdout}"));
    let nums: Vec<u64> = line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse().unwrap())
        .collect();
    assert_eq!(nums.len(), 2, "{line}");
    (nums[0] as u32, nums[1])
}

#[test]
fn plan_agrees_with_the_library_planner() {
    if skip_if_no_cli("plan_agrees_with_the_library_planner") {
        return;
    }
    use libviprs::{Layout, PyramidPlanner};
    for (w, h, tile, overlap) in [
        (1000, 500, 256, 0),
        (12738, 220, 256, 0),
        (700, 700, 128, 2),
    ] {
        let plan = PyramidPlanner::new(w, h, tile, overlap, Layout::DeepZoom)
            .unwrap()
            .plan();
        let r = ok(&[
            "plan",
            &w.to_string(),
            "--height",
            &h.to_string(),
            "--tile-size",
            &tile.to_string(),
            "--overlap",
            &overlap.to_string(),
        ]);
        assert!(
            r.stdout.contains(&format!("Image: {w}x{h}")),
            "{}",
            r.describe()
        );
        let total: u64 = plan.levels.iter().map(|l| l.tile_count()).sum();
        assert_eq!(
            plan_totals(&r.stdout),
            (plan.levels.len() as u32, total),
            "{}",
            r.describe()
        );
    }
    // From a file: the dimensions come from the input, and for the PDF from
    // its page at 72 DPI.
    let r = ok(&["plan", s(&fixture("canonical_input.png"))]);
    assert!(r.stdout.contains("Image: 256x256"), "{}", r.describe());
    let r = ok(&["plan", s(&fixture(MIX_PDF))]);
    assert!(r.stdout.contains("Image: 4768x3370"), "{}", r.describe());
}

#[test]
fn plan_failures_follow_the_exit_code_contract() {
    if skip_if_no_cli("plan_failures_follow_the_exit_code_contract") {
        return;
    }
    let tmp = scratch();
    let garbage = tmp.path().join("garbage.png");
    std::fs::write(&garbage, b"not an image").unwrap();

    // A numeric width with no --height is a missing required argument.
    exits(2, &["plan", "1000"], &["--height is required"]);
    exits(2, &["plan"], &["required"]);
    exits(2, &["plan", "0", "--height", "500"], &["width"]);
    exits(2, &["plan", "1000", "--height", "0"], &["--height"]);
    exits(
        2,
        &["plan", "1000", "--height", "500", "--tile-size", "0"],
        &["--tile-size"],
    );
    exits(
        2,
        &["plan", "1000", "--height", "500", "--overlap", "256"],
        &["--overlap"],
    );
    exits(
        2,
        &["plan", "1000", "--height", "500", "--layout", "nope"],
        &["--layout"],
    );

    exits(
        1,
        &["plan", s(&tmp.path().join("missing.png"))],
        &["Not a number or file"],
    );
    exits(1, &["plan", s(&garbage)], &["Error reading image"]);
    exits(
        1,
        &["plan", "4294967295", "--height", "4294967295"],
        &["too large"],
    );
}

// ---------------------------------------------------------------------------
// test-image
// ---------------------------------------------------------------------------

#[test]
fn test_image_writes_the_library_gradient() {
    if skip_if_no_cli("test_image_writes_the_library_gradient") {
        return;
    }
    let tmp = scratch();
    let png = tmp.path().join("gradient.png");
    let r = ok(&["test-image", s(&png), "--width", "97", "--height", "31"]);
    assert!(r.stderr.contains("97x31 Rgb8"), "{}", r.describe());
    let want = libviprs::generate_test_raster(97, 31).unwrap();
    let got = image::open(&png)
        .expect("test-image writes a PNG")
        .to_rgb8();
    assert_eq!((got.width(), got.height()), (97, 31));
    assert_eq!(
        got.as_raw().as_slice(),
        want.data(),
        "the PNG is not the library's gradient"
    );
}

#[test]
fn test_image_failures_follow_the_exit_code_contract() {
    if skip_if_no_cli("test_image_failures_follow_the_exit_code_contract") {
        return;
    }
    let tmp = scratch();
    let png = tmp.path().join("t.png");
    exits(2, &["test-image"], &["required"]);
    exits(2, &["test-image", s(&png), "--width", "0"], &["--width"]);
    exits(2, &["test-image", s(&png), "--height", "0"], &["--height"]);
    exits(2, &["test-image", s(&png), "--width", "-3"], &[]);
    assert!(!png.exists());
    exits(
        1,
        &[
            "test-image",
            s(&tmp.path().join("no/such/dir/t.png")),
            "--width",
            "8",
            "--height",
            "8",
        ],
        &["Error writing file"],
    );
    exits(
        1,
        &[
            "test-image",
            s(&png),
            "--width",
            "100000",
            "--height",
            "100000",
        ],
        &["pixel ceiling"],
    );
    assert!(!png.exists());
}

// ---------------------------------------------------------------------------
// pmtiles: pack (no suite has it), and every subcommand's failures
// ---------------------------------------------------------------------------

#[test]
fn pmtiles_pack_round_trips_a_tree_pyramid_wrote() {
    if skip_if_no_cli("pmtiles_pack_round_trips_a_tree_pyramid_wrote") {
        return;
    }
    let tmp = scratch();
    let tree = tmp.path().join("tree");
    ok(&[
        "pyramid",
        s(&fixture(MIX_PNG)),
        s(&tree),
        "--storage",
        "directory",
        "--layout",
        "xyz",
    ]);
    let packed = tmp.path().join("packed.pmtiles");
    let r = ok(&[
        "pmtiles",
        "pack",
        s(&tree),
        s(&packed),
        "--width",
        "12738",
        "--height",
        "220",
    ]);
    assert!(
        r.stdout.contains("tiles written       : 110"),
        "{}",
        r.describe()
    );
    assert!(
        r.stdout.contains("tiles absent        : 0"),
        "{}",
        r.describe()
    );
    ok(&["pmtiles", "verify", s(&packed)]);
    // And the packed archive is the reference pyramid.
    assert_archive_matches_reference(&packed, "pyramid tree packed");
    // Unpacked again, it is byte for byte the tree it came from.
    let unpacked = tmp.path().join("unpacked");
    ok(&["pmtiles", "extract", s(&packed), s(&unpacked)]);
    assert_eq!(collect_files(&tree, "png"), collect_files(&unpacked, "png"));
}

#[test]
fn pmtiles_failures_follow_the_exit_code_contract() {
    if skip_if_no_cli("pmtiles_failures_follow_the_exit_code_contract") {
        return;
    }
    let tmp = scratch();
    let tree = tmp.path().join("tree");
    ok(&[
        "pyramid",
        s(&fixture("canonical_input.png")),
        s(&tree),
        "--storage",
        "directory",
        "--layout",
        "xyz",
    ]);
    let archive = tmp.path().join("a.pmtiles");
    ok(&["pyramid", s(&fixture("canonical_input.png")), s(&archive)]);
    let not_archive = tmp.path().join("not.pmtiles");
    std::fs::write(&not_archive, b"garbage").unwrap();
    let missing = tmp.path().join("missing.pmtiles");
    let (a, n, m) = (s(&archive), s(&not_archive), s(&missing));
    let packed = tmp.path().join("packed.pmtiles");
    let pk = s(&packed);

    // Usage: a subcommand is required, and pack's plan is a required mode:
    // the manifest or the grid flags, never neither and never both.
    exits(2, &["pmtiles"], &["Usage"]);
    exits(2, &["pmtiles", "info"], &["required"]);
    exits(2, &["pmtiles", "tile", a, "0", "0"], &["required"]);
    exits(2, &["pmtiles", "tile", a, "0", "0", "-1"], &[]);
    exits(
        2,
        &["pmtiles", "pack", s(&tree), pk],
        &["--width/--height were not given"],
    );
    exits(
        2,
        &[
            "pmtiles",
            "pack",
            s(&tree),
            pk,
            "--manifest",
            n,
            "--width",
            "256",
            "--height",
            "256",
        ],
        &["cannot be used with"],
    );
    exits(
        2,
        &[
            "pmtiles",
            "pack",
            s(&tree),
            pk,
            "--width",
            "256",
            "--height",
            "256",
            "--tile-size",
            "0",
        ],
        &["does not describe a pyramid"],
    );
    assert!(!packed.exists());

    // Operational: an input that is not there or not an archive, a tile the
    // archive does not hold, a tree that is not a tree.
    for sub in ["info", "verify"] {
        exits(1, &["pmtiles", sub, n], &["the v3 header is 127 bytes"]);
        exits(1, &["pmtiles", sub, m], &["Error"]);
    }
    exits(1, &["pmtiles", "tile", a, "20", "0", "0"], &["is not in"]);
    exits(1, &["pmtiles", "tile", a, "0", "99", "0"], &["outside"]);
    exits(1, &["pmtiles", "tile", n, "0", "0", "0"], &["Error"]);
    exits(
        1,
        &["pmtiles", "extract", m, s(&tmp.path().join("x"))],
        &["Error"],
    );
    exits(
        1,
        &[
            "pmtiles",
            "pack",
            s(&tmp.path().join("nodir")),
            pk,
            "--width",
            "256",
            "--height",
            "256",
        ],
        &["is not a directory of tiles"],
    );
    exits(
        1,
        &["pmtiles", "pack", s(&tree), pk, "--manifest", m],
        &["Error"],
    );
    assert!(!packed.exists());
}

// ---------------------------------------------------------------------------
// verify, features, pdf, geo: one success and the contract's failures each
// (their own suites go deep; this is the contract walked once per command)
// ---------------------------------------------------------------------------

#[test]
fn verify_follows_the_exit_code_contract() {
    if skip_if_no_cli("verify_follows_the_exit_code_contract") {
        return;
    }
    let tmp = scratch();
    let archive = tmp.path().join("a.pmtiles");
    ok(&["pyramid", s(&fixture("canonical_input.png")), s(&archive)]);
    let r = ok(&["verify", s(&archive)]);
    assert!(r.stdout.contains("Tiles: 9"), "{}", r.describe());

    let not_archive = tmp.path().join("not.pmtiles");
    std::fs::write(&not_archive, b"garbage").unwrap();
    exits(2, &["verify"], &["required"]);
    exits(
        2,
        &[
            "verify",
            s(&archive),
            "--source",
            s(&fixture("canonical_input.png")),
            "--drop-blanks",
        ],
        &["--drop-blanks cannot be combined with --source"],
    );
    exits(
        1,
        &["verify", s(&tmp.path().join("missing.pmtiles"))],
        &["does not exist"],
    );
    exits(1, &["verify", s(&not_archive)], &["Error"]);
    exits(1, &["verify", s(tmp.path())], &["no manifest.json"]);
}

#[test]
fn features_follows_the_exit_code_contract() {
    if skip_if_no_cli("features_follows_the_exit_code_contract") {
        return;
    }
    let r = ok(&["features", "--json"]);
    let json: serde_json::Value = serde_json::from_str(&r.stdout).expect("features --json is JSON");
    assert_eq!(json["v"], 1, "{}", r.describe());
    assert!(json["features"].is_array(), "{}", r.describe());
    exits(2, &["features", "--bogus"], &["--bogus"]);
    exits(2, &["features", "extra"], &[]);
    // A feature this build left out is a 1, naming the feature to rebuild with.
    let tmp = scratch();
    exits(
        1,
        &[
            "pyramid",
            s(&fixture("canonical_input.png")),
            s(&tmp.path().join("o.pmtiles")),
            "--trace-level",
            "debug",
        ],
        &["--features tracing"],
    );
    exits(
        1,
        &[
            "pyramid",
            s(&fixture("canonical_input.png")),
            s(&tmp.path().join("o.pmtiles")),
            "--sink",
            "s3://bucket/prefix",
        ],
        &["--features s3"],
    );
}

#[test]
fn pdf_follows_the_exit_code_contract() {
    if skip_if_no_cli("pdf_follows_the_exit_code_contract") {
        return;
    }
    let r = ok(&["pdf", "info", s(&fixture(MIX_PDF))]);
    assert!(r.stdout.contains("Pages: 1"), "{}", r.describe());
    let r = ok(&["pdf", "rotation", s(&fixture("canonical_rotated_90.pdf"))]);
    assert_eq!(r.stdout.trim(), "90", "{}", r.describe());

    let tmp = scratch();
    let garbage = tmp.path().join("garbage.pdf");
    std::fs::write(&garbage, b"not a pdf").unwrap();
    let png = tmp.path().join("p.png");
    exits(2, &["pdf"], &["Usage"]);
    exits(2, &["pdf", "info"], &["required"]);
    exits(
        2,
        &[
            "pdf",
            "extract",
            s(&fixture(MIX_PDF)),
            s(&png),
            "--page",
            "0",
        ],
        &["pages are numbered from 1"],
    );
    exits(1, &["pdf", "info", s(&garbage)], &["PDF parse error"]);
    exits(
        1,
        &[
            "pdf",
            "extract",
            s(&fixture(MIX_PDF)),
            s(&png),
            "--page",
            "5",
        ],
        &["out of range"],
    );
    assert!(!png.exists());
}

#[test]
fn geo_follows_the_exit_code_contract() {
    if skip_if_no_cli("geo_follows_the_exit_code_contract") {
        return;
    }
    let r = ok(&[
        "geo",
        "pixel-to-geo",
        "10",
        "20",
        "--geo-origin",
        "1,2",
        "--geo-scale",
        "0.5,-0.5",
    ]);
    assert_eq!(r.stdout.trim(), "6,-8", "{}", r.describe());
    // The transform is a required mode: --affine or --geo-origin, one of them.
    exits(
        2,
        &["geo", "pixel-to-geo", "10", "20"],
        &["required", "--affine", "--geo-origin"],
    );
    exits(2, &["geo"], &["Usage"]);
    exits(
        2,
        &["geo", "pixel-to-geo", "10", "20", "--geo-origin", "1,2"],
        &["--geo-scale"],
    );
    exits(
        1,
        &["geo", "geo-to-pixel", "1", "1", "--affine", "0,0,0,0,0,0"],
        &[],
    );
}

// ---------------------------------------------------------------------------
// The contract across the whole binary
// ---------------------------------------------------------------------------

const BUILT_INS: &[&str] = &[
    "pyramid",
    "info",
    "plan",
    "test-image",
    "pmtiles",
    "features",
    "pdf",
    "geo",
    "verify",
];

#[test]
fn every_built_in_has_help_and_the_binary_refuses_nonsense_with_2() {
    if skip_if_no_cli("every_built_in_has_help_and_the_binary_refuses_nonsense_with_2") {
        return;
    }
    let top = ok(&["--help"]);
    for &cmd in BUILT_INS {
        assert!(
            top.stdout.lines().any(|l| l.trim_start().starts_with(cmd)),
            "`viprs --help` does not list {cmd}: {}",
            top.describe()
        );
        let r = ok(&[cmd, "--help"]);
        assert!(r.stdout.contains("Usage:"), "{}", r.describe());
    }
    exits(2, &[], &["Usage"]);
    exits(2, &["no-such-command"], &["unrecognized subcommand"]);
}

#[test]
fn an_op_command_follows_the_same_contract() {
    if skip_if_no_cli("an_op_command_follows_the_same_contract") {
        return;
    }
    // One op family member, to show the built-ins and the ops share the
    // contract: `flip`, whose output cli_iocleanup_diff.rs compares with vips.
    let tmp = scratch();
    let out = tmp.path().join("flipped.png");
    let input = fixture("canonical_input.png");
    ok(&["flip", s(&input), s(&out), "horizontal"]);
    let a = image::open(&input).unwrap().to_rgba8();
    let b = image::open(&out).unwrap().to_rgba8();
    assert_eq!(a.get_pixel(0, 0), b.get_pixel(255, 0));

    exits(2, &["flip", s(&input), s(&out), "sideways"], &["sideways"]);
    exits(2, &["flip", s(&input)], &["required"]);
    exits(
        1,
        &[
            "flip",
            s(&tmp.path().join("missing.png")),
            s(&out),
            "horizontal",
        ],
        &["Error"],
    );
}

/// Ctrl-C mid-run exits 130, says so, and does not panic.
///
/// `cli_pyramid_pipeline.rs` resumes an interrupted job and compares bytes;
/// this cell only holds the exit code to the contract. The input is big enough
/// that the run is still going when the signal lands: the signal goes out once
/// the run has printed its plan, which it does after decoding.
#[test]
fn ctrl_c_exits_130() {
    if skip_if_no_cli("ctrl_c_exits_130") {
        return;
    }
    let tmp = scratch();
    let big = tmp.path().join("big.png");
    ok(&[
        "test-image",
        s(&big),
        "--width",
        "12000",
        "--height",
        "12000",
    ]);
    let archive = tmp.path().join("big.pmtiles");

    let mut child = Command::new(viprs_bin())
        .args(["pyramid", s(&big), s(&archive)])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn viprs");
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let mut all = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line.clone());
            all.push_str(&line);
            all.push('\n');
        }
        all
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let line = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the run printed no Plan: line within 120 s");
        if line.starts_with("Plan:") {
            break;
        }
    }
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("run kill");
    assert!(status.success(), "kill -INT failed");
    let exit = child.wait().expect("wait for viprs");
    let stderr = reader.join().unwrap();
    let out = Output {
        status: exit,
        stdout: Vec::new(),
        stderr: stderr.clone().into_bytes(),
    };
    let code = assert_no_panic(&["pyramid".into(), "<SIGINT>".into()], &out);
    assert_eq!(
        code, 130,
        "Ctrl-C must exit 130 (the run may have finished before the signal \
         landed if it exited 0):\n{stderr}"
    );
    assert!(stderr.contains("Interrupted"), "{stderr}");
}
