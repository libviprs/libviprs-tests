//! CLI-DIFFERENTIAL suite — create (image-generator) family (a Wave-2
//! per-family lane, CLI_CONTRACT.md §7, OP_MAP.md create section).
//!
//! Each test runs `viprs <op> …` and decode-compares its output against the
//! COMMITTED reference at the op's §5 tolerance. The suite NEVER runs vips:
//! references are generated offline by `tools/gen_cli_expected.sh` and committed.
//! This cell copies the bands/morphology reference cells exactly (the skip-guard
//! / `VIPRS_REQUIRE_CLI` discipline included).
//!
//! Creators are §3 shape **S5** (OUT first, no image input) except `buildlut`,
//! which is **S1** (a matrix `.mat` file input). Oracle classes are MIXED per
//! OP_MAP.md (CLI_CONTRACT.md §5):
//!
//! * **EXACT** (tol 0): `black`, `xyz`, and — measured exact on these inputs —
//!   `tonelut` (a deterministic Gray16 integer curve).
//! * **BOUNDED-TOL** (float trig / exp / pow → f32 carrier): `eye`, `zone`,
//!   `sines`, `buildlut`, `sdf`, and every `mask_*` op. Measured max-abs-diff is
//!   **exactly 0** for every `mask_*` and `buildlut`, and `<= 4e-6` for
//!   `eye`/`zone`/`sdf` (f32 trig/hypot rounding). They are compared with a small
//!   f32 epsilon ([`FLOAT_EPS`]) as an honest upper bound for the class, and the
//!   `--uchar` variants at `<= 1` LSB ([`UCHAR_TOL`]).
//! * **GOLDEN-ONLY** (NO vips oracle — PRNG / Pango differ even seeded):
//!   `gaussnoise`, `perlin`, `worley`, `fractsurf`, `text`. The references are
//!   `viprs`-generated regression pins (deterministic across runs); the tests
//!   pin that output against future regressions and are compared at tol 0.
//!
//! Carriers (CLI_CONTRACT.md §2): float creators → the native `.v` container;
//! `--uchar` variants → PNG; `tonelut` (Gray16) → `.v`. vips `xyz` (uint) and
//! `buildlut` (double) are cast to float in the reference (the libviprs `.v`
//! decoder reads only uchar/ushort/float) — the cast is lossless for integer
//! coordinates / f32-exact LUT entries. See PROVENANCE.md.
//!
//! If the `libviprs-cli` sibling is not checked out, every test SKIPS (unless
//! `$VIPRS_REQUIRE_CLI=1`, which turns the skip into a hard failure — the
//! dedicated `cli-differential` CI job sets it, CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs_ok};

use tempfile::TempDir;

/// EXACT oracle class: bit-exact decode comparison (CLI_CONTRACT.md §5).
const EXACT: f64 = 0.0;

/// BOUNDED-TOL f32 epsilon for the float creators / masks (CLI_CONTRACT.md §5).
/// Measured max-abs-diff is 0 for every `mask_*` and `buildlut`, and `<= 4e-6`
/// for `eye`/`zone`/`sdf` (f32 trig/hypot rounding); `1e-5` is a tight honest
/// upper bound for the class.
const FLOAT_EPS: f64 = 1e-5;

/// BOUNDED-TOL for the `--uchar` creator variants: `<= 1` LSB (the C
/// float-to-uchar cast may differ by one; measured 0 on these fixtures).
const UCHAR_TOL: f64 = 1.0;

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
/// With `$VIPRS_REQUIRE_CLI=1` an absent sibling PANICS inside [`cli_available`]
/// instead, so this never silently skips on the dedicated CI job.
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

/// Absolute path to a fresh output file inside a process-lifetime temp dir.
fn out_path(name: &str) -> PathBuf {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
    dir.path().join(name)
}

/// Absolute string path of a committed fixture under `tests/fixtures/cli/`.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// Run `viprs <op> …` to produce `out`, then decode-compare it against the
/// committed reference at `tol`. `args` are the op + its non-OUT arguments;
/// `out` is spliced into the vips OUT-first position (right after the op name).
fn creator_case(out_name: &str, op_and_tail: &[&str], reference: &str, tol: f64) {
    let out = out_path(out_name);
    // S5: `<op> OUT <tail…>`. op_and_tail[0] is the op; the rest is the tail.
    let (op, tail) = op_and_tail.split_first().expect("op name");
    let mut argv: Vec<&str> = vec![op, out.to_str().unwrap()];
    argv.extend_from_slice(tail);
    run_viprs_ok(&argv);
    decode_compare(&out, &cli_fixture(reference), tol);
}

// ---------------------------------------------------------------------------
// EXACT creators — black (--bands), xyz.
// ---------------------------------------------------------------------------

#[test]
fn black_matches_vips_exact() {
    if skip_if_no_cli("black") {
        return;
    }
    creator_case(
        "black.v",
        &["black", "8", "8"],
        "create/black_expected.v",
        EXACT,
    );
}

#[test]
fn black_bands_matches_vips_exact() {
    if skip_if_no_cli("black_bands") {
        return;
    }
    // --bands 3 exercises the band-count flag (a 3-band Rgb8 result).
    creator_case(
        "black_bands3.v",
        &["black", "8", "8", "--bands", "3"],
        "create/black_bands3_expected.v",
        EXACT,
    );
}

#[test]
fn xyz_matches_vips_exact() {
    if skip_if_no_cli("xyz") {
        return;
    }
    // Coordinate ramp: each pixel is its own [x, y]. Non-trivial (a no-op fails).
    creator_case(
        "xyz.v",
        &["xyz", "16", "16"],
        "create/xyz_expected.v",
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// BOUNDED-TOL float creators — eye (+--uchar), zone, sines.
// ---------------------------------------------------------------------------

#[test]
fn eye_matches_vips_bounded() {
    if skip_if_no_cli("eye") {
        return;
    }
    creator_case(
        "eye.v",
        &["eye", "32", "32"],
        "create/eye_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn eye_uchar_matches_vips_bounded() {
    if skip_if_no_cli("eye_uchar") {
        return;
    }
    // --uchar routes through the PNG integer sink (BOUNDED-TOL <=1 LSB).
    creator_case(
        "eye_uchar.png",
        &["eye", "32", "32", "--uchar"],
        "create/eye_uchar_expected.png",
        UCHAR_TOL,
    );
}

#[test]
fn zone_matches_vips_bounded() {
    if skip_if_no_cli("zone") {
        return;
    }
    creator_case(
        "zone.v",
        &["zone", "32", "32"],
        "create/zone_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn sines_matches_vips_bounded() {
    if skip_if_no_cli("sines") {
        return;
    }
    creator_case(
        "sines.v",
        &["sines", "32", "32"],
        "create/sines_expected.v",
        FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// LUT builders — buildlut (S1 matrix-file), tonelut.
// ---------------------------------------------------------------------------

#[test]
fn buildlut_matches_vips_bounded() {
    if skip_if_no_cli("buildlut") {
        return;
    }
    // S1: the input is a committed control-point matrix, read by BOTH sides.
    let out = out_path("buildlut.v");
    run_viprs_ok(&[
        "buildlut",
        &fx("create/buildlut_points.mat"),
        out.to_str().unwrap(),
    ]);
    decode_compare(&out, &cli_fixture("create/buildlut_expected.v"), FLOAT_EPS);
}

#[test]
fn tonelut_matches_vips_exact() {
    if skip_if_no_cli("tonelut") {
        return;
    }
    // A deterministic Gray16 tone curve; measured exact against vips (tol 0).
    creator_case(
        "tonelut.v",
        &["tonelut"],
        "create/tonelut_expected.v",
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// Frequency-domain masks — ideal (+--nodc), ring/band, gaussian, butterworth
// (+--uchar --optical), fractal.
// ---------------------------------------------------------------------------

#[test]
fn mask_ideal_matches_vips_bounded() {
    if skip_if_no_cli("mask_ideal") {
        return;
    }
    creator_case(
        "mask_ideal.v",
        &["mask_ideal", "64", "64", "0.5"],
        "create/mask_ideal_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_ideal_nodc_matches_vips_bounded() {
    if skip_if_no_cli("mask_ideal_nodc") {
        return;
    }
    // --nodc changes the DC pixel (1.0 -> the formula value): a distinct output.
    creator_case(
        "mask_ideal_nodc.v",
        &["mask_ideal", "64", "64", "0.5", "--nodc"],
        "create/mask_ideal_nodc_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_ideal_ring_matches_vips_bounded() {
    if skip_if_no_cli("mask_ideal_ring") {
        return;
    }
    creator_case(
        "mask_ideal_ring.v",
        &["mask_ideal_ring", "64", "64", "0.5", "0.2"],
        "create/mask_ideal_ring_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_ideal_band_matches_vips_bounded() {
    if skip_if_no_cli("mask_ideal_band") {
        return;
    }
    creator_case(
        "mask_ideal_band.v",
        &["mask_ideal_band", "64", "64", "0.3", "0.3", "0.1"],
        "create/mask_ideal_band_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_gaussian_matches_vips_bounded() {
    if skip_if_no_cli("mask_gaussian") {
        return;
    }
    creator_case(
        "mask_gaussian.v",
        &["mask_gaussian", "64", "64", "0.5", "0.5"],
        "create/mask_gaussian_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_gaussian_ring_matches_vips_bounded() {
    if skip_if_no_cli("mask_gaussian_ring") {
        return;
    }
    creator_case(
        "mask_gaussian_ring.v",
        &["mask_gaussian_ring", "64", "64", "0.5", "0.5", "0.2"],
        "create/mask_gaussian_ring_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_gaussian_band_matches_vips_bounded() {
    if skip_if_no_cli("mask_gaussian_band") {
        return;
    }
    creator_case(
        "mask_gaussian_band.v",
        &["mask_gaussian_band", "64", "64", "0.3", "0.3", "0.1", "0.5"],
        "create/mask_gaussian_band_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_butterworth_matches_vips_bounded() {
    if skip_if_no_cli("mask_butterworth") {
        return;
    }
    creator_case(
        "mask_butterworth.v",
        &["mask_butterworth", "64", "64", "2", "0.5", "0.5"],
        "create/mask_butterworth_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_butterworth_uchar_optical_matches_vips_bounded() {
    if skip_if_no_cli("mask_butterworth_uchar") {
        return;
    }
    // --uchar (PNG sink) + --optical (DC at centre): both flag paths in one case.
    creator_case(
        "mask_butterworth_uchar.png",
        &[
            "mask_butterworth",
            "64",
            "64",
            "2",
            "0.5",
            "0.5",
            "--uchar",
            "--optical",
        ],
        "create/mask_butterworth_uchar_expected.png",
        UCHAR_TOL,
    );
}

#[test]
fn mask_butterworth_ring_matches_vips_bounded() {
    if skip_if_no_cli("mask_butterworth_ring") {
        return;
    }
    creator_case(
        "mask_butterworth_ring.v",
        &[
            "mask_butterworth_ring",
            "64",
            "64",
            "2",
            "0.5",
            "0.5",
            "0.2",
        ],
        "create/mask_butterworth_ring_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_butterworth_band_matches_vips_bounded() {
    if skip_if_no_cli("mask_butterworth_band") {
        return;
    }
    creator_case(
        "mask_butterworth_band.v",
        &[
            "mask_butterworth_band",
            "64",
            "64",
            "2",
            "0.3",
            "0.3",
            "0.1",
            "0.5",
        ],
        "create/mask_butterworth_band_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn mask_fractal_matches_vips_bounded() {
    if skip_if_no_cli("mask_fractal") {
        return;
    }
    creator_case(
        "mask_fractal.v",
        &["mask_fractal", "64", "64", "2.5"],
        "create/mask_fractal_expected.v",
        FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// Signed distance fields — every shape (circle/box/line/rounded-box).
// ---------------------------------------------------------------------------

#[test]
fn sdf_circle_matches_vips_bounded() {
    if skip_if_no_cli("sdf_circle") {
        return;
    }
    creator_case(
        "sdf_circle.v",
        &["sdf", "64", "64", "circle", "--a", "32 32", "--r", "16"],
        "create/sdf_circle_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn sdf_box_matches_vips_bounded() {
    if skip_if_no_cli("sdf_box") {
        return;
    }
    creator_case(
        "sdf_box.v",
        &["sdf", "64", "64", "box", "--a", "10 10", "--b", "50 40"],
        "create/sdf_box_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn sdf_line_matches_vips_bounded() {
    if skip_if_no_cli("sdf_line") {
        return;
    }
    creator_case(
        "sdf_line.v",
        &["sdf", "64", "64", "line", "--a", "10 10", "--b", "50 40"],
        "create/sdf_line_expected.v",
        FLOAT_EPS,
    );
}

#[test]
fn sdf_rounded_box_matches_vips_bounded() {
    if skip_if_no_cli("sdf_rounded_box") {
        return;
    }
    // --corners exercises the rounded-box corner-radius vector.
    creator_case(
        "sdf_rounded.v",
        &[
            "sdf",
            "64",
            "64",
            "rounded-box",
            "--a",
            "10 10",
            "--b",
            "50 40",
            "--corners",
            "20 0 0 0",
        ],
        "create/sdf_rounded_expected.v",
        FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// GOLDEN-ONLY pins — noise (gaussnoise/perlin/worley/fractsurf) + text.
// NO vips oracle: PRNG / Pango differ from vips even seeded, so the reference is
// `viprs`'s own deterministic output and the test is a regression pin.
// ---------------------------------------------------------------------------

#[test]
fn gaussnoise_matches_golden_pin() {
    if skip_if_no_cli("gaussnoise") {
        return;
    }
    // GOLDEN-ONLY: viprs vs the committed viprs pin (no vips oracle exists).
    creator_case(
        "gaussnoise.v",
        &[
            "gaussnoise",
            "16",
            "16",
            "--seed",
            "42",
            "--sigma",
            "10",
            "--mean",
            "128",
        ],
        "create/gaussnoise_golden.v",
        EXACT,
    );
}

#[test]
fn perlin_matches_golden_pin() {
    if skip_if_no_cli("perlin") {
        return;
    }
    creator_case(
        "perlin.v",
        &["perlin", "64", "64", "--seed", "7"],
        "create/perlin_golden.v",
        EXACT,
    );
}

#[test]
fn worley_matches_golden_pin() {
    if skip_if_no_cli("worley") {
        return;
    }
    creator_case(
        "worley.v",
        &["worley", "64", "64", "--seed", "7"],
        "create/worley_golden.v",
        EXACT,
    );
}

#[test]
fn fractsurf_matches_golden_pin() {
    if skip_if_no_cli("fractsurf") {
        return;
    }
    creator_case(
        "fractsurf.v",
        &["fractsurf", "64", "48", "2.5"],
        "create/fractsurf_golden.v",
        EXACT,
    );
}

#[test]
fn text_matches_golden_pin() {
    if skip_if_no_cli("text") {
        return;
    }
    // GOLDEN-ONLY: Pango rendering differs from libviprs, so the reference is
    // viprs's own deterministic Gray8 coverage image.
    creator_case(
        "text.png",
        &["text", "Hi", "--dpi", "72"],
        "create/text_golden.png",
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// Bounds rejection — an out-of-range parameter is a typed nonzero exit with a
// viprs-side message (CLI_CONTRACT.md §8), never a panic and never a wrong
// "success". We assert the viprs message substring only, never vips text.
// ---------------------------------------------------------------------------

#[test]
fn fractsurf_out_of_range_dimension_is_rejected() {
    if skip_if_no_cli("fractsurf_reject") {
        return;
    }
    let out = out_path("fractsurf_bad.v");
    // fractal-dimension 5.0 is outside vips's 2..3; viprs rejects it (exit 1).
    let result = common::cli::run_viprs(&["fractsurf", out.to_str().unwrap(), "16", "16", "5.0"]);
    assert!(
        !result.status.success(),
        "an out-of-range fractal dimension must be a nonzero exit"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("outside the vips range"),
        "expected a viprs range-rejection message, got stderr: {stderr}"
    );
}
