//! CLI-DIFFERENTIAL suite — morphology family (the Wave-1 reference family,
//! CLI_CONTRACT.md §7 / §10, OP_MAP.md morphology section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed input
//! (`tests/fixtures/cli/morphology/input.png`) and decode-compares its output
//! against the COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The
//! suite NEVER runs vips: references are generated offline by
//! `tools/gen_cli_expected.sh` and committed. This cell is the PATTERN every
//! later per-family differential wave copies.
//!
//! All five morphology ops are oracle class **EXACT** (OP_MAP.md): decode /
//! scalar comparison at tolerance 0.
//!
//! If the `libviprs-cli` sibling is not checked out (the default CI `test` job
//! and the Docker gate clone only the core counterpart), every test SKIPS with
//! a clear message rather than failing — the dedicated `cli-differential` CI
//! job lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{
    SCALAR_INT_EXACT, SCALAR_S3_REL_EPS, cli_available, cli_fixture, compare_scalar,
    decode_compare, parse_scalar, read_scalar_fixture, run_viprs_ok,
};

use tempfile::TempDir;

/// EXACT oracle class: bit-exact decode comparison (CLI_CONTRACT.md §5). Applies
/// to the image→image morphology ops (integer-in / integer-out).
const EXACT: f64 = 0.0;

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
///
/// When `$VIPRS_REQUIRE_CLI=1` (the dedicated `cli-differential` CI job) an
/// absent sibling instead PANICS inside [`cli_available`], so this never returns
/// `true` there — a would-be silent skip becomes a hard failure.
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

/// Absolute path to a fresh output file inside a **process-lifetime** temp dir.
///
/// A single [`TempDir`] is created once and reused for every case (each gets a
/// distinct filename), so the directory is reclaimed by its `Drop` at process
/// exit — the previous per-call `mem::forget(TempDir)` leaked one temp dir per
/// invocation for the whole run.
fn out_path(name: &str) -> PathBuf {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
    dir.path().join(name)
}

const INPUT: &str = "morphology/input.png";
const MASK: &str = "morphology/cross.mat";
/// Structuring element exercising all three trit values {0, 128, 255}: a `0`
/// (must-be-zero) cell that `cross.mat` lacks (F7 coverage).
const MASK_CORNER: &str = "morphology/corner.mat";
/// Multi-level (≥3 distinct gray) input for the rank order-statistic semantics
/// (`cross`/binary input only differs on 2 levels) (F8 coverage).
const INPUT_GRAY: &str = "morphology/input_gray.png";

#[test]
fn morph_erode_matches_vips_exact() {
    if skip_if_no_cli("morph_erode") {
        return;
    }
    let input = cli_fixture(INPUT);
    let mask = cli_fixture(MASK);
    let out = out_path("erode.png");
    run_viprs_ok(&[
        "morph",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        mask.to_str().unwrap(),
        "erode",
    ]);
    decode_compare(
        &out,
        &cli_fixture("morphology/morph_erode_expected.png"),
        EXACT,
    );
}

#[test]
fn morph_dilate_matches_vips_exact() {
    if skip_if_no_cli("morph_dilate") {
        return;
    }
    let input = cli_fixture(INPUT);
    let mask = cli_fixture(MASK);
    let out = out_path("dilate.png");
    run_viprs_ok(&[
        "morph",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        mask.to_str().unwrap(),
        "dilate",
    ]);
    decode_compare(
        &out,
        &cli_fixture("morphology/morph_dilate_expected.png"),
        EXACT,
    );
}

#[test]
fn rank_median_matches_vips_exact() {
    if skip_if_no_cli("rank_median") {
        return;
    }
    let input = cli_fixture(INPUT);
    let out = out_path("rank.png");
    // Median of a 3×3 window = sorted index 4.
    run_viprs_ok(&[
        "rank",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        "3",
        "3",
        "4",
    ]);
    decode_compare(
        &out,
        &cli_fixture("morphology/rank_median_expected.png"),
        EXACT,
    );
}

#[test]
fn countlines_horizontal_matches_vips_exact() {
    if skip_if_no_cli("countlines_horizontal") {
        return;
    }
    let input = cli_fixture(INPUT);
    let stdout = run_viprs_ok(&["countlines", input.to_str().unwrap(), "horizontal"]);
    let expected = read_scalar_fixture("morphology/countlines_horizontal_expected.txt");
    // `countlines` returns a floating mean, NOT an integer count: use the S3
    // relative epsilon, not tol-0 (which only survives here because 0.578125 is
    // dyadic). See SCALAR_S3_REL_EPS.
    compare_scalar(&stdout, expected, SCALAR_S3_REL_EPS);
}

#[test]
fn countlines_vertical_matches_vips_exact() {
    if skip_if_no_cli("countlines_vertical") {
        return;
    }
    let input = cli_fixture(INPUT);
    let stdout = run_viprs_ok(&["countlines", input.to_str().unwrap(), "vertical"]);
    let expected = read_scalar_fixture("morphology/countlines_vertical_expected.txt");
    compare_scalar(&stdout, expected, SCALAR_S3_REL_EPS);
}

#[test]
fn labelregions_mask_and_count_match_vips_exact() {
    if skip_if_no_cli("labelregions") {
        return;
    }
    let input = cli_fixture(INPUT);
    // 16-bit TIFF carrier: vips emits an INT-band mask that does not round-trip
    // through PNG / the .v decoder; viprs writes the Gray16 label mask here.
    let out = out_path("label.tif");
    let stdout = run_viprs_ok(&[
        "labelregions",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
    ]);

    // S4: decode-compare the labelled mask …
    decode_compare(
        &out,
        &cli_fixture("morphology/labelregions_mask_expected.tif"),
        EXACT,
    );
    // … AND compare the printed segment count (integer-exact).
    let expected_segments = read_scalar_fixture("morphology/labelregions_segments_expected.txt");
    let got_segments = parse_scalar(&stdout);
    // Segment count is a genuine INTEGER: bit-exact (SCALAR_INT_EXACT), unlike
    // the floating `countlines` mean above.
    compare_scalar(&stdout, expected_segments, SCALAR_INT_EXACT);
    assert_eq!(
        got_segments, expected_segments,
        "labelregions segment count mismatch: got {got_segments}, expected {expected_segments}",
    );
}

// ---------------------------------------------------------------------------
// F7 — trit coverage: a mask with a must-be-ZERO (0) cell, so erode/dilate
// exercise all three structuring-element values {0, 128, 255}. `cross.mat`
// uses only 128/255.
// ---------------------------------------------------------------------------

#[test]
fn morph_erode_corner_mask_matches_vips_exact() {
    if skip_if_no_cli("morph_erode_corner") {
        return;
    }
    let input = cli_fixture(INPUT);
    let mask = cli_fixture(MASK_CORNER);
    let out = out_path("erode_corner.png");
    run_viprs_ok(&[
        "morph",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        mask.to_str().unwrap(),
        "erode",
    ]);
    decode_compare(
        &out,
        &cli_fixture("morphology/morph_erode_corner_expected.png"),
        EXACT,
    );
}

#[test]
fn morph_dilate_corner_mask_matches_vips_exact() {
    if skip_if_no_cli("morph_dilate_corner") {
        return;
    }
    let input = cli_fixture(INPUT);
    let mask = cli_fixture(MASK_CORNER);
    let out = out_path("dilate_corner.png");
    run_viprs_ok(&[
        "morph",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        mask.to_str().unwrap(),
        "dilate",
    ]);
    decode_compare(
        &out,
        &cli_fixture("morphology/morph_dilate_corner_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// F8 — rank on a MULTI-LEVEL (≥3 distinct gray) input, plus a non-median index,
// pinning the order-statistic index semantics that a 2-level input cannot.
// ---------------------------------------------------------------------------

#[test]
fn rank_median_multilevel_matches_vips_exact() {
    if skip_if_no_cli("rank_median_multilevel") {
        return;
    }
    let input = cli_fixture(INPUT_GRAY);
    let out = out_path("rank_gray_median.png");
    // Median of a 3×3 window = sorted index 4.
    run_viprs_ok(&[
        "rank",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        "3",
        "3",
        "4",
    ]);
    decode_compare(
        &out,
        &cli_fixture("morphology/rank_gray_median_expected.png"),
        EXACT,
    );
}

#[test]
fn rank_nonmedian_index_matches_vips_exact() {
    if skip_if_no_cli("rank_nonmedian_index") {
        return;
    }
    let input = cli_fixture(INPUT_GRAY);
    let out = out_path("rank_gray_max.png");
    // Index 8 of a 3×3 window = the window MAXIMUM (a dilation), distinct from
    // the median: pins that INDEX selects the order statistic, not a fixed stat.
    run_viprs_ok(&[
        "rank",
        input.to_str().unwrap(),
        out.to_str().unwrap(),
        "3",
        "3",
        "8",
    ]);
    decode_compare(
        &out,
        &cli_fixture("morphology/rank_gray_max_expected.png"),
        EXACT,
    );
}
