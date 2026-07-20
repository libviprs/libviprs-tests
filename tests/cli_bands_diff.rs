//! CLI-DIFFERENTIAL suite — bands family (the first per-family Wave-2 lane,
//! CLI_CONTRACT.md §7, OP_MAP.md bands section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/bands/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the morphology reference cell
//! (`cli_morphology_diff.rs`) exactly — including the skip-guard /
//! `VIPRS_REQUIRE_CLI` discipline — and is itself the template later per-family
//! differential waves copy.
//!
//! All bands commands are oracle class **EXACT** (integer-in / integer-out,
//! decode comparison at tolerance 0) — including `bandmean`, which core issue
//! #482 upgraded from BOUNDED-TOL: the core now rounds the per-pixel integer
//! mean to nearest (previously it floored via truncating division, ≤1 LSB below
//! vips's round-to-nearest), so a non-divisible band sum now matches vips
//! bit-for-bit (see the `bandmean` test). References land on a
//! 1 / 3 / 4-band uchar PNG where the interpretation is clean (1-band, or
//! sRGB-tagged 3/4-band); the ops whose output is a **b-w multiband** image
//! (`bandfold`, `bandjoin_const`, and the ≥3-input `bandjoin3` fold) are carried
//! as the native `.v` container instead, because vips's PNG encoder
//! colour-promotes a b-w multiband image (mangling the raw bands) and the
//! libviprs TIFF decoder rejects a 4-band multiband TIFF.
//!
//! If the `libviprs-cli` sibling is not checked out (the default CI `test` job
//! and the Docker gate clone only the core counterpart), every test SKIPS with
//! a clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs_ok};

use tempfile::TempDir;

/// EXACT oracle class: bit-exact decode comparison (CLI_CONTRACT.md §5). Every
/// bands op is integer-in / integer-out — except `bandmean`.
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

/// Absolute path to a fresh output file inside a **process-lifetime** temp dir
/// (one [`TempDir`] reused for every case, reclaimed at process exit).
fn out_path(name: &str) -> PathBuf {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
    dir.path().join(name)
}

// Common committed inputs (tests/fixtures/cli/bands/).
const RGB: &str = "bands/rgb.png";
const RGBA: &str = "bands/rgba.png";
const GRAY: &str = "bands/gray.png";
const GRAY2: &str = "bands/gray2.png";
const GRAY3: &str = "bands/gray3.png";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// bandjoin — S2 variadic (rgb + gray -> 4-band rgba).
// ---------------------------------------------------------------------------

#[test]
fn bandjoin_matches_vips_exact() {
    if skip_if_no_cli("bandjoin") {
        return;
    }
    let out = out_path("bandjoin.png");
    run_viprs_ok(&["bandjoin", &fx(RGB), &fx(GRAY), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("bands/bandjoin_expected.png"), EXACT);
}

// ---------------------------------------------------------------------------
// bandjoin — S2 variadic with THREE inputs (gray + gray2 + gray3 -> 3-band b-w).
// The two-input case above runs run_bandjoin's accumulation loop exactly ONCE;
// this ≥3-input case runs it MORE THAN once, exercising the true variadic fold
// (the S2 template 14 later families copy). Carried as `.v` (b-w multiband).
// ---------------------------------------------------------------------------

#[test]
fn bandjoin_three_inputs_matches_vips_exact() {
    if skip_if_no_cli("bandjoin3") {
        return;
    }
    let out = out_path("bandjoin3.v");
    run_viprs_ok(&[
        "bandjoin",
        &fx(GRAY),
        &fx(GRAY2),
        &fx(GRAY3),
        out.to_str().unwrap(),
    ]);
    decode_compare(&out, &cli_fixture("bands/bandjoin3_expected.v"), EXACT);
}

// ---------------------------------------------------------------------------
// bandjoin_const — S1, multi-element constant vector (gray + "10 20 30").
// The 4-band b-w output is carried as `.v` (vips's PNG encoder would
// colour-promote a b-w multiband image, and libviprs cannot decode a 4-band
// multiband TIFF).
// ---------------------------------------------------------------------------

#[test]
fn bandjoin_const_matches_vips_exact() {
    if skip_if_no_cli("bandjoin_const") {
        return;
    }
    let out = out_path("bandjoin_const.v");
    run_viprs_ok(&[
        "bandjoin_const",
        &fx(GRAY),
        out.to_str().unwrap(),
        "10 20 30",
    ]);
    decode_compare(&out, &cli_fixture("bands/bandjoin_const_expected.v"), EXACT);
}

// ---------------------------------------------------------------------------
// bandfold — S1 --factor (gray, factor 4 -> 4x16 4-band). Carried as `.v` for
// the same b-w-multiband reason as bandjoin_const.
// ---------------------------------------------------------------------------

#[test]
fn bandfold_matches_vips_exact() {
    if skip_if_no_cli("bandfold") {
        return;
    }
    let out = out_path("bandfold.v");
    run_viprs_ok(&[
        "bandfold",
        &fx(GRAY),
        out.to_str().unwrap(),
        "--factor",
        "4",
    ]);
    decode_compare(&out, &cli_fixture("bands/bandfold_expected.v"), EXACT);
}

// ---------------------------------------------------------------------------
// bandunfold — S1 (rgb, default factor = unfold all -> 48x16 1-band). The
// fold/unfold pair: bandfold folds the x axis into bands, bandunfold reverses.
// ---------------------------------------------------------------------------

#[test]
fn bandunfold_matches_vips_exact() {
    if skip_if_no_cli("bandunfold") {
        return;
    }
    let out = out_path("bandunfold.png");
    run_viprs_ok(&["bandunfold", &fx(RGB), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("bands/bandunfold_expected.png"), EXACT);
}

// ---------------------------------------------------------------------------
// bandmean — S1 (rgb -> 1-band per-pixel mean). EXACT (tol 0): rgb.png is a
// bandjoin of three DISTINCT grays, so a per-pixel band sum is generally NOT
// divisible by 3 — a non-vacuous case (the earlier rgb_eq.png had three
// identical bands, making mean == input, both divisible AND arithmetically
// vacuous). Core issue #482 made the core round the integer mean to nearest
// (previously it FLOORED via truncating division, ≤1 LSB below vips), so the two
// now agree bit-for-bit. Compared at tol 0.
// ---------------------------------------------------------------------------

#[test]
fn bandmean_matches_vips_exact() {
    if skip_if_no_cli("bandmean") {
        return;
    }
    let out = out_path("bandmean.png");
    run_viprs_ok(&["bandmean", &fx(RGB), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("bands/bandmean_expected.png"), EXACT);
}

// ---------------------------------------------------------------------------
// bandrank — S2 variadic + --index (3 grays: median default, then min).
// ---------------------------------------------------------------------------

#[test]
fn bandrank_median_matches_vips_exact() {
    if skip_if_no_cli("bandrank_median") {
        return;
    }
    let out = out_path("bandrank_median.png");
    // No --index: viprs default -1 == core median, matching vips's default.
    run_viprs_ok(&[
        "bandrank",
        &fx(GRAY),
        &fx(GRAY2),
        &fx(GRAY3),
        out.to_str().unwrap(),
    ]);
    decode_compare(
        &out,
        &cli_fixture("bands/bandrank_median_expected.png"),
        EXACT,
    );
}

#[test]
fn bandrank_min_index_matches_vips_exact() {
    if skip_if_no_cli("bandrank_min") {
        return;
    }
    let out = out_path("bandrank_min.png");
    // --index 0 selects the per-sample minimum (a distinct order statistic).
    run_viprs_ok(&[
        "bandrank",
        &fx(GRAY),
        &fx(GRAY2),
        &fx(GRAY3),
        out.to_str().unwrap(),
        "--index",
        "0",
    ]);
    decode_compare(&out, &cli_fixture("bands/bandrank_min_expected.png"), EXACT);
}

// ---------------------------------------------------------------------------
// bandbool — S1 enum and|or|eor (rgb -> 1-band bitwise fold).
// ---------------------------------------------------------------------------

#[test]
fn bandbool_and_matches_vips_exact() {
    if skip_if_no_cli("bandbool_and") {
        return;
    }
    let out = out_path("bandbool_and.png");
    run_viprs_ok(&["bandbool", &fx(RGB), out.to_str().unwrap(), "and"]);
    decode_compare(&out, &cli_fixture("bands/bandbool_and_expected.png"), EXACT);
}

#[test]
fn bandbool_or_matches_vips_exact() {
    if skip_if_no_cli("bandbool_or") {
        return;
    }
    let out = out_path("bandbool_or.png");
    run_viprs_ok(&["bandbool", &fx(RGB), out.to_str().unwrap(), "or"]);
    decode_compare(&out, &cli_fixture("bands/bandbool_or_expected.png"), EXACT);
}

#[test]
fn bandbool_eor_matches_vips_exact() {
    if skip_if_no_cli("bandbool_eor") {
        return;
    }
    let out = out_path("bandbool_eor.png");
    run_viprs_ok(&["bandbool", &fx(RGB), out.to_str().unwrap(), "eor"]);
    decode_compare(&out, &cli_fixture("bands/bandbool_eor_expected.png"), EXACT);
}

// ---------------------------------------------------------------------------
// extract_band — S1 BAND + --n (single band, then --n consecutive bands).
// ---------------------------------------------------------------------------

#[test]
fn extract_band_single_matches_vips_exact() {
    if skip_if_no_cli("extract_band_single") {
        return;
    }
    let out = out_path("extract_band1.png");
    // Band 1 (green) of the RGB input -> 1-band gray.
    run_viprs_ok(&["extract_band", &fx(RGB), out.to_str().unwrap(), "1"]);
    decode_compare(
        &out,
        &cli_fixture("bands/extract_band1_expected.png"),
        EXACT,
    );
}

#[test]
fn extract_band_n_matches_vips_exact() {
    if skip_if_no_cli("extract_band_n") {
        return;
    }
    let out = out_path("extract_bandn.png");
    // Bands 1..3 of the RGBA input -> 3-band (--n exercises the count arg).
    run_viprs_ok(&[
        "extract_band",
        &fx(RGBA),
        out.to_str().unwrap(),
        "1",
        "--n",
        "3",
    ]);
    decode_compare(
        &out,
        &cli_fixture("bands/extract_bandn_expected.png"),
        EXACT,
    );
}
