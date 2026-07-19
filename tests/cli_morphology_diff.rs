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

use common::cli::{
    cli_available, cli_fixture, compare_scalar, decode_compare, parse_scalar, read_scalar_fixture,
    run_viprs_ok,
};

/// EXACT oracle class: bit-exact decode / scalar comparison (CLI_CONTRACT.md §5).
const EXACT: f64 = 0.0;

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
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

/// Absolute path to a fresh output file inside a per-test temp dir. The
/// `TempDir` is leaked so the path stays valid for the whole test; the OS
/// reclaims it (tests write tiny 64×64 rasters).
fn out_path(name: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join(name);
    std::mem::forget(dir);
    path
}

const INPUT: &str = "morphology/input.png";
const MASK: &str = "morphology/cross.mat";

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
    compare_scalar(&stdout, expected, EXACT);
}

#[test]
fn countlines_vertical_matches_vips_exact() {
    if skip_if_no_cli("countlines_vertical") {
        return;
    }
    let input = cli_fixture(INPUT);
    let stdout = run_viprs_ok(&["countlines", input.to_str().unwrap(), "vertical"]);
    let expected = read_scalar_fixture("morphology/countlines_vertical_expected.txt");
    compare_scalar(&stdout, expected, EXACT);
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
    assert_eq!(
        got_segments, expected_segments,
        "labelregions segment count mismatch: got {got_segments}, expected {expected_segments}",
    );
}
