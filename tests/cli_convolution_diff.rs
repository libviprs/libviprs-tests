//! CLI-DIFFERENTIAL suite — convolution family (the Wave-2 convolution lane,
//! CLI_CONTRACT.md §7, OP_MAP.md convolution section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/convolution/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the bands/morphology reference cells exactly
//! — including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! # Honest oracle classes (MEASURED against vips 8.18.4, not assumed)
//!
//! The differential MEASURES each op rather than trusting OP_MAP.md's
//! provisional class, and the measurements are load-bearing:
//!
//! * **EXACT (tol 0)** — `compass --combine max` at integer precision (its
//!   scale-1 sobel mask needs no coefficient rounding) and `fastcor` (an integer
//!   sum-of-squared-differences). gaussmat/logmat at **integer** precision are
//!   integer-valued matrices and also match exactly.
//! * **BOUNDED-TOL ≤1 LSB (tol 1)** — `conv` and `gaussblur` at **integer**
//!   precision with a **scaled** mask. OP_MAP.md provisionally called these
//!   EXACT; they are NOT. vips's `vips_convi` bakes the mask into a power-of-two
//!   fixed-point form and shifts (`>> sexp`); the core divides
//!   `(sum + scale/2) / scale`. The two round differently by at most one LSB.
//!   The EXACT compass case (scale-1 mask) proves the divergence is the scale
//!   division, not a CLI bug. `sharpen` (a LabS unsharp round trip) is likewise
//!   ≤1 LSB. See the wave report's open questions.
//! * **BOUNDED-TOL float (small eps)** — the large-magnitude float image
//!   surfaces (`conv`/`compass`/`gaussblur` float, `convsep`, measured ≤1.5e-5
//!   on the author Mac — the core's two-pass accumulation order differs from
//!   vips's fused, vectorised pass) at `FLOAT_EPS` = 1e-3, and `spcor` (eps
//!   1e-5). The eps carries the cross-platform libm / FMA drift the
//!   Mac-generated reference cannot see at test time on CI. The
//!   gaussmat/logmat **float** mask creators use a much tighter
//!   `MASK_FLOAT_EPS` = 1e-6 (their ≤1.0 coefficients make 1e-3 far too loose;
//!   the only real drift is cross-libm `exp()` at ~1 ULP), honouring OP_MAP's
//!   1e-9 intent for those two ops.
//! * **EXACT (tol 0), integer 16-bit promotion** — `compass --combine sum` at
//!   **integer** precision promotes uchar inputs to a 16-bit surface (values
//!   here reach 812, above the uchar range) and is bit-exact against vips.
//!
//! Every input is DISCRIMINATING: `eye.png` is a high-frequency zone-plate, so a
//! box blur moves it by up to 59 and compass edge-detection by 254 — a
//! broken/identity op fails loudly, far above the ≤1 tolerances.
//!
//! If the `libviprs-cli` sibling is not checked out (the default CI `test` job
//! and the Docker gate clone only the core counterpart), every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs_ok};

use tempfile::TempDir;

/// EXACT: bit-exact decode comparison (CLI_CONTRACT.md §5). Used for the
/// scale-1 compass, fastcor, and the integer-valued gaussmat/logmat matrices.
const EXACT: f64 = 0.0;

/// BOUNDED-TOL ≤1 LSB: `conv`/`gaussblur` integer-precision with a scaled mask
/// (vips's fixed-point `>> sexp` vs the core's `(sum + scale/2)/scale`), and the
/// `sharpen` LabS round trip. Measured max-abs-diff == 1 on the author Mac.
const LSB: f64 = 1.0;

/// BOUNDED-TOL for the large-magnitude float image surfaces
/// (`conv`/`compass`/`gaussblur` float, `convsep`). Measured ≤1.5e-5 on the
/// author Mac; the eps carries cross-platform libm / accumulation-order drift.
/// These surfaces reach ~1000-2262, so 1e-3 absolute is a tight relative bound.
const FLOAT_EPS: f64 = 1e-3;

/// BOUNDED-TOL for the `gaussmat` / `logmat` **float** mask creators. Their
/// coefficients are all ≤ 1.0, so the shared [`FLOAT_EPS`] = 1e-3 would be 0.1%
/// of full scale — six orders looser than OP_MAP.md's mandated 1e-9 for these
/// two ops, weak enough to hide a systematic ~0.05% transcendental/normalisation
/// error. The only real drift here is cross-libm `exp()` at ~1 ULP (~1e-16), so
/// this tight bound honours OP_MAP's 1e-9 intent with generous headroom.
/// Measured 0 for `gaussmat_float`, ≤1.5e-5 shape headroom retained for logmat.
const MASK_FLOAT_EPS: f64 = 1e-6;

/// BOUNDED-TOL for `spcor` (OP_MAP.md eps 1e-5): the normalised cross-correlation
/// accumulates in a slightly different order than vips. Measured 0 on the Mac.
const SPCOR_EPS: f64 = 1e-5;

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

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

// Common committed inputs / masks (tests/fixtures/cli/convolution/).
const EYE: &str = "convolution/eye.png";
const PATCH: &str = "convolution/patch.png";
const BLUR: &str = "convolution/blur.mat";
const SOBEL: &str = "convolution/sobel.mat";
const SEP: &str = "convolution/sep.mat";

// ---------------------------------------------------------------------------
// gaussmat — S5 creator (matrix → float `.v`).
// ---------------------------------------------------------------------------

#[test]
fn gaussmat_integer_matches_vips_exact() {
    if skip_if_no_cli("gaussmat_int") {
        return;
    }
    let out = out_path("gaussmat_int.v");
    run_viprs_ok(&["gaussmat", out.to_str().unwrap(), "2", "0.2"]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussmat_int_expected.v"),
        EXACT,
    );
}

#[test]
fn gaussmat_separable_matches_vips_exact() {
    if skip_if_no_cli("gaussmat_sep") {
        return;
    }
    let out = out_path("gaussmat_sep.v");
    run_viprs_ok(&["gaussmat", out.to_str().unwrap(), "2", "0.2", "--separable"]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussmat_sep_expected.v"),
        EXACT,
    );
}

#[test]
fn gaussmat_float_matches_vips_bounded_tol() {
    if skip_if_no_cli("gaussmat_float") {
        return;
    }
    let out = out_path("gaussmat_float.v");
    run_viprs_ok(&[
        "gaussmat",
        out.to_str().unwrap(),
        "2",
        "0.2",
        "--precision",
        "float",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussmat_float_expected.v"),
        MASK_FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// logmat — S5 creator.
// ---------------------------------------------------------------------------

#[test]
fn logmat_integer_matches_vips_exact() {
    if skip_if_no_cli("logmat_int") {
        return;
    }
    let out = out_path("logmat_int.v");
    run_viprs_ok(&["logmat", out.to_str().unwrap(), "2", "0.1"]);
    decode_compare(
        &out,
        &cli_fixture("convolution/logmat_int_expected.v"),
        EXACT,
    );
}

#[test]
fn logmat_float_separable_matches_vips_bounded_tol() {
    if skip_if_no_cli("logmat_float") {
        return;
    }
    let out = out_path("logmat_float.v");
    run_viprs_ok(&[
        "logmat",
        out.to_str().unwrap(),
        "2",
        "0.1",
        "--separable",
        "--precision",
        "float",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/logmat_float_expected.v"),
        MASK_FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// conv — S1, matrix-file mask.
// ---------------------------------------------------------------------------

#[test]
fn conv_blur_integer_matches_vips_bounded_tol() {
    // ≤1 LSB, NOT EXACT: the scale-9 box mask hits the fixed-point rounding-scheme
    // gap (vips `>> sexp` vs core `(sum + scale/2)/scale`). Non-vacuous — a box
    // blur of the zone-plate moves pixels by up to 59, far above the tol.
    if skip_if_no_cli("conv_blur_int") {
        return;
    }
    let out = out_path("conv_blur_int.png");
    run_viprs_ok(&[
        "conv",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(BLUR),
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/conv_blur_int_expected.png"),
        LSB,
    );
}

#[test]
fn conv_sobel_float_matches_vips_bounded_tol() {
    if skip_if_no_cli("conv_sobel_float") {
        return;
    }
    let out = out_path("conv_sobel_float.v");
    run_viprs_ok(&[
        "conv",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(SOBEL),
        "--precision",
        "float",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/conv_sobel_float_expected.v"),
        FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// convsep — S1, separable (1xN) matrix-file mask, float precision.
// ---------------------------------------------------------------------------

#[test]
fn convsep_float_matches_vips_bounded_tol() {
    if skip_if_no_cli("convsep_float") {
        return;
    }
    let out = out_path("convsep_float.v");
    run_viprs_ok(&[
        "convsep",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(SEP),
        "--precision",
        "float",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/convsep_float_expected.v"),
        FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// compass — S1, rotating mask. max/integer is EXACT (scale-1 mask); sum/float
// exercises the --combine and --precision flag paths.
// ---------------------------------------------------------------------------

#[test]
fn compass_max_integer_matches_vips_exact() {
    if skip_if_no_cli("compass_max_int") {
        return;
    }
    let out = out_path("compass_max_int.png");
    run_viprs_ok(&[
        "compass",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(SOBEL),
        "--times",
        "4",
        "--angle",
        "d45",
        "--combine",
        "max",
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/compass_max_int_expected.png"),
        EXACT,
    );
}

#[test]
fn compass_sum_float_matches_vips_bounded_tol() {
    if skip_if_no_cli("compass_sum_float") {
        return;
    }
    let out = out_path("compass_sum_float.v");
    run_viprs_ok(&[
        "compass",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(SOBEL),
        "--times",
        "4",
        "--angle",
        "d45",
        "--combine",
        "sum",
        "--precision",
        "float",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/compass_sum_float_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn compass_sum_integer_matches_vips_exact() {
    // EXACT (tol 0): the combine=sum + integer-precision path is the distinct
    // core branch that promotes uchar inputs to a 16-bit surface and saturates
    // (out_fmt = ushort for Sum). Integer arithmetic is deterministic, so it is
    // bit-exact against vips. Non-vacuous — the summed sobel edges reach 812,
    // above the uchar range, exercising the 16-bit promotion the sum/float and
    // max/integer cases never touch. Carrier: native ushort `.v`.
    if skip_if_no_cli("compass_sum_int") {
        return;
    }
    let out = out_path("compass_sum_int.v");
    run_viprs_ok(&[
        "compass",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(SOBEL),
        "--times",
        "4",
        "--angle",
        "d45",
        "--combine",
        "sum",
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/compass_sum_int_expected.v"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// gaussblur — S1.
// ---------------------------------------------------------------------------

#[test]
fn gaussblur_integer_matches_vips_bounded_tol() {
    // ≤1 LSB, same fixed-point rounding-scheme gap as conv (the separable
    // integer Gaussian divides by its scale). Non-vacuous — blurs the zone-plate.
    if skip_if_no_cli("gaussblur_int") {
        return;
    }
    let out = out_path("gaussblur_int.png");
    run_viprs_ok(&[
        "gaussblur",
        &fx(EYE),
        out.to_str().unwrap(),
        "1.5",
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussblur_int_expected.png"),
        LSB,
    );
}

#[test]
fn gaussblur_float_matches_vips_bounded_tol() {
    if skip_if_no_cli("gaussblur_float") {
        return;
    }
    let out = out_path("gaussblur_float.v");
    run_viprs_ok(&[
        "gaussblur",
        &fx(EYE),
        out.to_str().unwrap(),
        "1.5",
        "--precision",
        "float",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussblur_float_expected.v"),
        FLOAT_EPS,
    );
}

// ---------------------------------------------------------------------------
// sharpen — S1, LabS unsharp mask (≤1 LSB). m1/m2 nonzero so it truly sharpens.
// ---------------------------------------------------------------------------

#[test]
fn sharpen_matches_vips_bounded_tol() {
    if skip_if_no_cli("sharpen") {
        return;
    }
    let out = out_path("sharpen.png");
    run_viprs_ok(&[
        "sharpen",
        &fx(EYE),
        out.to_str().unwrap(),
        "--sigma",
        "1",
        "--m1",
        "1",
        "--m2",
        "2",
    ]);
    decode_compare(&out, &cli_fixture("convolution/sharpen_expected.png"), LSB);
}

// ---------------------------------------------------------------------------
// spcor / fastcor — S2 (two image inputs then OUT), correlation surfaces.
// ---------------------------------------------------------------------------

#[test]
fn spcor_matches_vips_bounded_tol() {
    if skip_if_no_cli("spcor") {
        return;
    }
    let out = out_path("spcor.v");
    run_viprs_ok(&["spcor", &fx(EYE), &fx(PATCH), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("convolution/spcor_expected.v"),
        SPCOR_EPS,
    );
}

#[test]
fn fastcor_matches_vips_exact() {
    // Integer sum-of-squared-differences: EXACT (tol 0). vips writes uint; the
    // reference is `vips cast … float`ed and viprs emits float, so the integer
    // values compare exactly.
    if skip_if_no_cli("fastcor") {
        return;
    }
    let out = out_path("fastcor.v");
    run_viprs_ok(&["fastcor", &fx(EYE), &fx(PATCH), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("convolution/fastcor_expected.v"), EXACT);
}
