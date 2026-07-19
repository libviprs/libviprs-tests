//! CLI-DIFFERENTIAL suite — colour family (the Wave-2 colour lane,
//! CLI_CONTRACT.md §7, OP_MAP.md colour section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/colour/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the bands / morphology reference cells
//! exactly — including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! Every colour op outputs a **non-RGB interpretation** (LAB/XYZ/scRGB float, a
//! float ΔE, or a re-profiled device image), so every command is oracle class
//! **BOUNDED-TOL** at a MEASURED tolerance — EXCEPT `dECMC`, which is
//! **GOLDEN-ONLY**: the core computes the published CMC(1:1) ΔE while vips
//! approximates dECMC as Euclidean distance in its CMC uniform space, a
//! DIFFERENT formula (measured max-abs-diff ~297), so there is no cross-oracle
//! and its reference is a viprs-generated regression pin (see the test).
//!
//! Measured max-abs-diff per case (author host, vips 8.18.4, arm64):
//!
//! | case | carrier | tol | measured |
//! |---|---|---|---|
//! | `colourspace … lab`   | `.v` float | 1e-4 | 4.6e-5 |
//! | `colourspace … xyz`   | `.v` float | 1e-4 | 1.5e-5 |
//! | `colourspace … scrgb` | `.v` float | 1e-4 | 1.0e-6 |
//! | `colourspace … lab` → PNG (#36) | PNG uchar | 1 (≤1 LSB) | 0 |
//! | `colourspace icc_pcs_lab.v … srgb` → PNG (#36 non-round-trip) | PNG uchar | 1 (≤1 LSB) | 1 |
//! | `colourspace --source-space lab` → PNG | PNG uchar | 1 (≤1 LSB) | 1 |
//! | `dE76` | `.v` float | 1e-4 | 6.5e-5 |
//! | `dE00` | `.v` float | 1e-4 | 6.5e-5 |
//! | `dECMC` (GOLDEN-ONLY) | `.v` float | 1e-3 | 0 (viprs self-pin) |
//! | `icc_import` | `.v` float | 0.35 | 0.303 |
//! | `icc_export` | PNG uchar | 2 (≤2 LSB) | 0 |
//! | `icc_export --depth 16` | 16-bit PNG | 16 (≤16 LSB) | 13 |
//! | `icc_transform` | PNG uchar | 2 (≤2 LSB) | 0 |
//!
//! The `.v` carrier is mandatory for the float LAB/XYZ/scRGB/ΔE/Lab-PCS outputs
//! (`CLI_CONTRACT.md` §2); the sRGB / device uchar targets go to PNG, which
//! also exercises the interpretation-aware `→ sRGB` conversion in `io::save`
//! (libviprs-cli #36) — `viprs colourspace … lab out.png` must reproduce vips's
//! own LAB→sRGB pngsave.
//!
//! If the `libviprs-cli` sibling is not checked out (the default CI `test` job
//! and the Docker gate clone only the core counterpart), every test SKIPS with
//! a clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs, run_viprs_ok};

use tempfile::TempDir;

/// BOUNDED-TOL for the float LAB/XYZ/scRGB colourspace outputs and the dE76 /
/// dE00 metrics (`CLI_CONTRACT.md` §5, colour round-trips): the core routes
/// through the same D65 XYZ hub as vips but with independently-rounded matrices,
/// so the two agree to well under 1e-4 (measured ≤6.5e-5).
const FLOAT_TOL: f64 = 1e-4;

/// BOUNDED-TOL ≤1 LSB for the interpretation-aware PNG saves (`colourspace … lab`
/// written to an integer sink runs the vips-style LAB→sRGB conversion, #36; and
/// the `--source-space` override): uchar, measured 0 / 1.
const UCHAR_1LSB: f64 = 1.0;

/// BOUNDED-TOL ≤2 LSB for the device-space ICC round trips (`icc_export`,
/// `icc_transform`): the matrix-shaper sRGB profile reproduces vips's device
/// output EXACTLY here (measured 0); the ≤2-LSB band is margin for a
/// cross-CMS / cross-arch rounding wobble (OP_MAP.md colour ICC caveat).
const UCHAR_2LSB: f64 = 2.0;

/// BOUNDED-TOL for the 16-bit `icc_export --depth 16` device output: at 16-bit
/// precision the native moxcms engine and vips's lcms2 diverge by ~13/65535 on
/// the matrix-shaper sRGB profile (the 8-bit path rounds that away to 0) — a
/// real, measured cross-CMS BOUNDED-TOL, NOT a bug. Compared in 16-bit sample
/// space (0..65535); measured 13, banded to ≤16 LSB for cross-arch margin.
const U16_16LSB: f64 = 16.0;

/// BOUNDED-TOL for `icc_import`'s Lab PCS output: the moxcms native ICC engine
/// and vips's lcms2 evaluate the matrix-shaper sRGB profile on the same exact
/// path but land ~0.3 Lab units apart (measured 0.303) — a real, documented
/// moxcms-vs-lcms2 matrix-shaper divergence, NOT a CLI bug. Still discriminating:
/// a broken import (wrong profile / space) diverges by tens of Lab units.
const ICC_IMPORT_TOL: f64 = 0.35;

/// GOLDEN-ONLY tolerance for `dECMC`: the reference is a viprs-generated pin, so
/// the comparison is viprs-vs-viprs and self-identical on the author host (0);
/// the small band absorbs cross-arch libm ULPs in the float ΔECMC pipeline.
const GOLDEN_TOL: f64 = 1e-3;

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

// Common committed inputs (tests/fixtures/cli/colour/).
const RGB: &str = "colour/rgb.png";
const RGB2: &str = "colour/rgb2.png";
const SRGB_ICC: &str = "colour/sRGB.icc";
const ICC_PCS_LAB: &str = "colour/icc_pcs_lab.v";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// colourspace — S1. Real round-trips through the D65 XYZ hub: sRGB -> LAB / XYZ
// / scRGB float, carried as `.v` (BOUNDED-TOL 1e-4). Distinct target spaces so
// each route is genuinely exercised (an identity op would fail the float tol).
// ---------------------------------------------------------------------------

#[test]
fn colourspace_srgb_to_lab_matches_vips_bounded_tol() {
    if skip_if_no_cli("colourspace_lab") {
        return;
    }
    let out = out_path("colourspace_lab.v");
    run_viprs_ok(&["colourspace", &fx(RGB), out.to_str().unwrap(), "lab"]);
    decode_compare(
        &out,
        &cli_fixture("colour/colourspace_lab_expected.v"),
        FLOAT_TOL,
    );
}

#[test]
fn colourspace_srgb_to_xyz_matches_vips_bounded_tol() {
    if skip_if_no_cli("colourspace_xyz") {
        return;
    }
    let out = out_path("colourspace_xyz.v");
    run_viprs_ok(&["colourspace", &fx(RGB), out.to_str().unwrap(), "xyz"]);
    decode_compare(
        &out,
        &cli_fixture("colour/colourspace_xyz_expected.v"),
        FLOAT_TOL,
    );
}

#[test]
fn colourspace_srgb_to_scrgb_matches_vips_bounded_tol() {
    if skip_if_no_cli("colourspace_scrgb") {
        return;
    }
    let out = out_path("colourspace_scrgb.v");
    run_viprs_ok(&["colourspace", &fx(RGB), out.to_str().unwrap(), "scrgb"]);
    decode_compare(
        &out,
        &cli_fixture("colour/colourspace_scrgb_expected.v"),
        FLOAT_TOL,
    );
}

// ---------------------------------------------------------------------------
// colourspace #36 — interpretation-aware PNG save. `colourspace … lab` written
// to an INTEGER sink: vips's pngsave converts the LAB result to sRGB before
// encoding, and io::save must do the same (not cast the raw Lab channels). uchar
// ≤1 LSB (measured 0). This is the differential that pins libviprs-cli #36.
// ---------------------------------------------------------------------------

#[test]
fn colourspace_lab_to_png_runs_interpretation_aware_save() {
    if skip_if_no_cli("colourspace_lab_png") {
        return;
    }
    let out = out_path("colourspace_lab.png");
    // A `.png` (integer) sink on a LAB result: io::save must colourspace-convert
    // LAB -> sRGB the way vips's foreign saver does (#36), NOT cast raw Lab.
    run_viprs_ok(&["colourspace", &fx(RGB), out.to_str().unwrap(), "lab"]);
    decode_compare(
        &out,
        &cli_fixture("colour/colourspace_lab_png_expected.png"),
        UCHAR_1LSB,
    );
}

// ---------------------------------------------------------------------------
// colourspace #36 (non-round-trip discriminator) — a GENUINELY Lab-tagged input
// (icc_pcs_lab.v, a D50 Lab PCS image) converted to sRGB and written to PNG. The
// plain #36 case above happens to round-trip to the input (its reference equals
// rgb.png), so an identity/no-op colourspace would still pass it; here the
// reference DIFFERS from the input, so a raw-cast / no-op colourspace would
// garble the output. This makes the PNG path discriminate the colourspace
// transform itself, not just the interpretation-aware save. uchar ≤1 LSB
// (measured 1).
// ---------------------------------------------------------------------------

#[test]
fn colourspace_lab_input_to_png_discriminates_the_transform() {
    if skip_if_no_cli("colourspace_lab_input_png") {
        return;
    }
    let out = out_path("colourspace_lab_input.png");
    run_viprs_ok(&[
        "colourspace",
        &fx(ICC_PCS_LAB),
        out.to_str().unwrap(),
        "srgb",
    ]);
    decode_compare(
        &out,
        &cli_fixture("colour/colourspace_lab_input_png_expected.png"),
        UCHAR_1LSB,
    );
}

// ---------------------------------------------------------------------------
// colourspace --source-space — force the sRGB-tagged input to be read as LAB,
// then convert to sRGB. Genuinely discriminating: if --source-space were ignored
// the op would collapse to the srgb->srgb identity (255 apart from this result).
// uchar ≤1 LSB (measured 1).
// ---------------------------------------------------------------------------

#[test]
fn colourspace_source_space_override_matches_vips() {
    if skip_if_no_cli("colourspace_srcspace") {
        return;
    }
    let out = out_path("colourspace_srcspace.png");
    run_viprs_ok(&[
        "colourspace",
        &fx(RGB),
        out.to_str().unwrap(),
        "srgb",
        "--source-space",
        "lab",
    ]);
    decode_compare(
        &out,
        &cli_fixture("colour/colourspace_srcspace_expected.png"),
        UCHAR_1LSB,
    );
}

// ---------------------------------------------------------------------------
// dE76 / dE00 — S2 two-image ΔE -> float `.v` (BOUNDED-TOL 1e-4). Two DISTINCT
// sRGB inputs, so ΔE is non-vacuous (a==b would give ΔE≡0). dE00 pins the
// libvips vips_col_dE00 hue-wrap parity.
// ---------------------------------------------------------------------------

#[test]
fn de76_matches_vips_bounded_tol() {
    if skip_if_no_cli("dE76") {
        return;
    }
    let out = out_path("dE76.v");
    run_viprs_ok(&["dE76", &fx(RGB), &fx(RGB2), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("colour/dE76_expected.v"), FLOAT_TOL);
}

#[test]
fn de00_matches_vips_bounded_tol() {
    if skip_if_no_cli("dE00") {
        return;
    }
    let out = out_path("dE00.v");
    run_viprs_ok(&["dE00", &fx(RGB), &fx(RGB2), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("colour/dE00_expected.v"), FLOAT_TOL);
}

// ---------------------------------------------------------------------------
// dECMC — GOLDEN-ONLY (no vips oracle). vips computes Euclidean distance in its
// CMC uniform space; the core computes the published CMC(1:1) ΔE — a DIFFERENT
// formula (measured max-abs-diff ~297; vips range [13,311] vs core [8,100]). The
// reference is generated by `viprs` itself and this test is a REGRESSION PIN,
// not a vips comparison.
// ---------------------------------------------------------------------------

#[test]
fn decmc_golden_regression_pin() {
    if skip_if_no_cli("dECMC") {
        return;
    }
    let out = out_path("dECMC.v");
    run_viprs_ok(&["dECMC", &fx(RGB), &fx(RGB2), out.to_str().unwrap()]);
    // GOLDEN-ONLY: there is NO vips oracle here (vips's dECMC is a different
    // formula). The committed reference is a viprs-generated pin.
    decode_compare(&out, &cli_fixture("colour/dECMC_golden.v"), GOLDEN_TOL);
}

// ---------------------------------------------------------------------------
// icc_import — S1 device -> Lab PCS `.v`. Matrix-shaper sRGB profile. BOUNDED
// -TOL at the measured moxcms-vs-lcms2 divergence (~0.31 Lab units).
// ---------------------------------------------------------------------------

#[test]
fn icc_import_srgb_matrix_shaper_matches_vips_bounded_tol() {
    if skip_if_no_cli("icc_import") {
        return;
    }
    let out = out_path("icc_import.v");
    run_viprs_ok(&[
        "icc_import",
        &fx(RGB),
        out.to_str().unwrap(),
        "--input-profile",
        &fx(SRGB_ICC),
        "--intent",
        "relative",
    ]);
    decode_compare(
        &out,
        &cli_fixture("colour/icc_import_lab_expected.v"),
        ICC_IMPORT_TOL,
    );
}

// ---------------------------------------------------------------------------
// icc_export — S1 Lab PCS -> device PNG. The committed `icc_pcs_lab.v` (a real
// D50 Lab PCS image) is fed to BOTH sides. Matrix-shaper round trip matches vips
// EXACTLY (measured 0); compared at ≤2 LSB.
// ---------------------------------------------------------------------------

#[test]
fn icc_export_srgb_matrix_shaper_matches_vips() {
    if skip_if_no_cli("icc_export") {
        return;
    }
    let out = out_path("icc_export.png");
    run_viprs_ok(&[
        "icc_export",
        &fx(ICC_PCS_LAB),
        out.to_str().unwrap(),
        "--output-profile",
        &fx(SRGB_ICC),
        "--intent",
        "relative",
        "--depth",
        "8",
    ]);
    decode_compare(
        &out,
        &cli_fixture("colour/icc_export_expected.png"),
        UCHAR_2LSB,
    );
}

// ---------------------------------------------------------------------------
// icc_export --depth 16 — the 16-bit device-output path. `--depth 16` is
// clap-valid AND core-realised (only 10/12/14 land in ColourError::UnsupportedDepth);
// at 16-bit precision moxcms vs lcms2 diverge by ~13/65535 (the 8-bit path rounds
// that to 0), a measured cross-CMS BOUNDED-TOL. 16-bit PNG, ≤16 LSB (measured 13).
// ---------------------------------------------------------------------------

#[test]
fn icc_export_depth_16_matches_vips_bounded_tol() {
    if skip_if_no_cli("icc_export_d16") {
        return;
    }
    let out = out_path("icc_export_d16.png");
    run_viprs_ok(&[
        "icc_export",
        &fx(ICC_PCS_LAB),
        out.to_str().unwrap(),
        "--output-profile",
        &fx(SRGB_ICC),
        "--intent",
        "relative",
        "--depth",
        "16",
    ]);
    decode_compare(
        &out,
        &cli_fixture("colour/icc_export_d16_expected.png"),
        U16_16LSB,
    );
}

// ---------------------------------------------------------------------------
// icc_transform — S1 device -> device PNG in one step (positional output profile
// + `--input-profile`, both real vips flags). sRGB->sRGB round trip matches vips
// EXACTLY (measured 0); compared at ≤2 LSB. Exercises the composed
// import+export path (the core's try_icc_transform has no input-profile param).
// ---------------------------------------------------------------------------

#[test]
fn icc_transform_srgb_round_trip_matches_vips() {
    if skip_if_no_cli("icc_transform") {
        return;
    }
    let out = out_path("icc_transform.png");
    run_viprs_ok(&[
        "icc_transform",
        &fx(RGB),
        out.to_str().unwrap(),
        &fx(SRGB_ICC),
        "--input-profile",
        &fx(SRGB_ICC),
        "--intent",
        "relative",
    ]);
    decode_compare(
        &out,
        &cli_fixture("colour/icc_transform_expected.png"),
        UCHAR_2LSB,
    );
}

// ---------------------------------------------------------------------------
// Error / bounds rejection (CLI_CONTRACT.md §8): op failure -> exit 1 with a
// viprs-side message substring; usage / bad enum / out-of-range -> exit 2 (clap).
// Never a panic/abort. We assert only the viprs-side behaviour (never vips text).
// ---------------------------------------------------------------------------

#[test]
fn colourspace_unsupported_route_exits_1() {
    if skip_if_no_cli("colourspace_unsupported") {
        return;
    }
    // `labq` parses (it is a vips space nickname) but has no core colourspace
    // route, so the op fails with exit 1 — NOT a panic.
    let out = out_path("colourspace_labq.v");
    let res = run_viprs(&["colourspace", &fx(RGB), out.to_str().unwrap(), "labq"]);
    assert!(
        !res.status.success(),
        "colourspace to an unsupported route must exit non-zero"
    );
    let code = res.status.code();
    assert_eq!(code, Some(1), "an op error is exit 1 (got {code:?})");
}

#[test]
fn icc_import_without_a_profile_exits_1() {
    if skip_if_no_cli("icc_import_noprofile") {
        return;
    }
    // No --input-profile and the PNG carries no embedded profile: the op fails
    // with exit 1 (NoProfile), not a panic.
    let out = out_path("icc_import_noprofile.v");
    let res = run_viprs(&["icc_import", &fx(RGB), out.to_str().unwrap()]);
    assert!(
        !res.status.success(),
        "icc_import with no available profile must exit non-zero"
    );
    assert_eq!(res.status.code(), Some(1), "an op error is exit 1");
}

#[test]
fn icc_transform_without_input_profile_exits_1() {
    if skip_if_no_cli("icc_transform_noprofile") {
        return;
    }
    // No --input-profile and rgb.png carries no decoder-readable embedded
    // profile: the composed import step (try_icc_import_with with input_profile
    // = None, reading the embedded profile) fails NoProfile -> exit 1, NOT a
    // panic. This exercises the embedded-profile (input_profile = None) arm of
    // the unified icc_transform composition (libviprs-cli colour fix #1) — the
    // only path that reaches try_icc_import_with(None) inside icc_transform.
    let out = out_path("icc_transform_noprofile.png");
    let res = run_viprs(&[
        "icc_transform",
        &fx(RGB),
        out.to_str().unwrap(),
        &fx(SRGB_ICC),
    ]);
    assert!(
        !res.status.success(),
        "icc_transform with no available input profile must exit non-zero"
    );
    assert_eq!(res.status.code(), Some(1), "an op error is exit 1");
}

#[test]
fn icc_export_in_range_but_core_unsupported_depth_exits_1() {
    if skip_if_no_cli("icc_export_depth_10") {
        return;
    }
    // --depth 10 is INSIDE clap's 8..=16 range (so NOT a usage error), but the
    // core only realises 8 or 16 -> ColourError::UnsupportedDepth -> exit 1
    // (op error), NOT the clap usage exit 2 and NOT a panic. This pins the
    // distinction between the clap-usage rejection (exit 2, --depth 4) and the
    // core-op rejection (exit 1, an in-range-but-unsupported --depth).
    let out = out_path("icc_export_depth10.png");
    let res = run_viprs(&[
        "icc_export",
        &fx(ICC_PCS_LAB),
        out.to_str().unwrap(),
        "--output-profile",
        &fx(SRGB_ICC),
        "--depth",
        "10",
    ]);
    assert!(!res.status.success());
    assert_eq!(
        res.status.code(),
        Some(1),
        "an in-range but core-unsupported --depth is an op error (exit 1), not a usage error"
    );
}

#[test]
fn colourspace_rejects_an_unknown_space_with_usage_exit_2() {
    if skip_if_no_cli("colourspace_bad_enum") {
        return;
    }
    let out = out_path("colourspace_bad.v");
    let res = run_viprs(&[
        "colourspace",
        &fx(RGB),
        out.to_str().unwrap(),
        "not-a-space",
    ]);
    assert!(!res.status.success());
    assert_eq!(
        res.status.code(),
        Some(2),
        "an unknown enum value is a clap usage error (exit 2)"
    );
}

#[test]
fn icc_export_rejects_an_out_of_range_depth_with_usage_exit_2() {
    if skip_if_no_cli("icc_export_bad_depth") {
        return;
    }
    let out = out_path("icc_export_bad.png");
    // vips's declared depth range is 8..=16; a depth below the minimum is a
    // clap value-parser (usage) rejection, exit 2 — never a panic.
    let res = run_viprs(&[
        "icc_export",
        &fx(ICC_PCS_LAB),
        out.to_str().unwrap(),
        "--output-profile",
        &fx(SRGB_ICC),
        "--depth",
        "4",
    ]);
    assert!(!res.status.success());
    assert_eq!(
        res.status.code(),
        Some(2),
        "an out-of-range --depth is a clap usage error (exit 2)"
    );
}
