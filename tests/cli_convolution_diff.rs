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
//! * **EXACT (tol 0) — integer convolution on uchar** (`conv`, `convsep`,
//!   `gaussblur`), against references generated with `VIPS_NOVECTOR=1`. These
//!   carried a ≤1 LSB tolerance until issue #558, and that tolerance was wrong
//!   twice over.
//!
//!   Its justification said vips shifts by `>> sexp` where the core divides
//!   `(sum + scale/2) / scale`. But **`sexp` is the ORC variable**, and it is
//!   never set in a Highway build, so the comment described a code path that
//!   is not in the binary it was measured against. And "at most one LSB" is
//!   contradicted by this very fixture: `gaussblur … 1.6` on `eye.png` moves
//!   256 samples by up to **4**, and `convsep … sep.mat` by **4**. Both are
//!   now pinned below, EXACTLY, so nobody rediscovers this by widening a
//!   number.
//!
//!   What actually differs is that libvips has **two** integer-convolution
//!   implementations on uchar, and they convolve with **different
//!   coefficients**. `vips_convi_gen` is the portable C loop libvips's own
//!   documentation names as the specification (`convi.c:1271-1284`: "For UCHAR
//!   images, vips_convi uses a fast vector path based on half-float
//!   arithmetic. **This can produce slightly different results.** Disable the
//!   vector path with --vips-novector or VIPS_NOVECTOR …"), and it is also
//!   what libvips falls back to whenever its own accuracy gate declines a
//!   mask. libviprs implements it, so `VIPS_NOVECTOR=1 vips` reproduces the
//!   core byte for byte and the references are generated that way.
//!
//!   The rounding mode is NOT the mechanism, and
//!   `conv_box_window_sums_to_1147_matches_vips_exact` is the proof: on a
//!   window summing to 1147 the C path gives `(1147 + 4) / 9 = 127`, floor
//!   gives 127 as well, and the vector path gives `(57 * 1147 + 256) >> 9 =
//!   128` because it is filtering with `57/512`, not `1/9`.
//!
//!   Nobody has ever bounded the gap between the two paths, so there is no
//!   honest tolerance to write. `vips_convi_intize`'s check
//!   (`convi.c:1096-1113`) is often read as bounding it at 2; it does not —
//!   it compares the requantised mask against exact real arithmetic at one
//!   grey level on a flat field, which constrains `sum(w_hat - w)` and says
//!   nothing about per-pixel error `sum((w_hat - w) * p)`.
//!   `conv_hostile_mask_matches_vips_exact` uses a mask that gate **accepts**
//!   and on which the two paths differ by **57**.
//! * **BOUNDED-TOL ≤1 LSB (tol 1) — `sharpen` ONLY.** This is a separate
//!   tolerance with a separate cause and it must not be folded back into the
//!   one above. `sharpen` convolves the L of LabS, which is 16-bit, and the
//!   vector path is gated on `BandFmt == VIPS_FORMAT_UCHAR` (`convi.c:1151`),
//!   so `sharpen` takes the portable C path on **both** libvips builds
//!   (`VIPS_INFO=1` reports "convi: using C path"). #558 cannot be its
//!   mechanism. The remaining deviation is a real libviprs bug, issue #581,
//!   which the shared constant was concealing.
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
//! * **Regime pins (tol 0)** — three regimes exist, not two, and nothing on the
//!   API surface tells a test author which one a mask is in, so the suite now
//!   pins the boundaries instead of relying on them silently:
//!   `conv_ushort_integer_matches_vips_exact` (the vector path is gated off
//!   entirely for 16-bit, `convi.c:1151`),
//!   `gaussblur_sigma_0_6_matches_vips_exact` (`vips_convi_intize` declines the
//!   3x1 scale-30 mask as "too inaccurate", so libvips runs the C path itself),
//!   and `conv_scale_1_mask_integer_matches_vips_exact` (the vector path RUNS
//!   and still agrees, because `intize` is the identity at scale 1).
//!
//! Every input is DISCRIMINATING: `eye.png` is a high-frequency zone-plate, so a
//! box blur moves it by up to 59 and compass edge-detection by 254 — a
//! broken/identity op fails loudly, far above the ≤1 tolerances. Since #558
//! three inputs are also chosen to separate libvips's two integer-convolution
//! paths deliberately rather than by luck: `noise64.png` (a hostile mask
//! reaches 2 on the zone-plate and **57** here), `boxsum1147.png` (every
//! window sums to 1147), and `eye16.v` (the ushort regime).
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

/// BOUNDED-TOL ≤1 LSB for **`sharpen` and nothing else** (issue #581).
///
/// This used to be shared with integer `conv`/`gaussblur`, and sharing it was
/// hiding two unrelated causes under one number. Integer convolution on uchar
/// is now EXACT against a `VIPS_NOVECTOR=1` reference (#558); what is left
/// here is `sharpen`'s own deviation, which #558 demonstrably cannot explain:
/// `sharpen` runs its unsharp mask on the L of LabS, which is 16-bit, and
/// `vips_convi`'s vector path is gated on `BandFmt == VIPS_FORMAT_UCHAR`
/// (`convi.c:1151`), so both libvips builds take the portable C path here —
/// `VIPS_INFO=1` says "convi: using C path" on the plain binary.
///
/// Measured max-abs-diff 1 on this fixture and 44 of 256 samples deviating.
/// Do NOT reuse this constant for another op: give the next deviation its own
/// name and its own explanation, or the same conflation happens again.
const SHARPEN_LSB: f64 = 1.0;

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

// #558 discriminating inputs and mask. Each one separates libvips's two
// integer-convolution paths on purpose; the gaps in the comments are measured
// on vips 8.18.4 (targets NEON_BF16 NEON) and recorded in PROVENANCE.md.
/// 64x64 `gaussnoise --seed 42`. High entropy is what makes a hostile mask
/// hostile: the same mask reaches 2 on `eye.png` and 57 here.
const NOISE64: &str = "convolution/noise64.png";
/// 3x3, eight 127s around a 131, so every replicated window sums to 1147.
const BOXSUM1147: &str = "convolution/boxsum1147.png";
/// 16x16 **ushort** zone-plate: the regime where `convi.c:1151` gates the
/// vector path off entirely.
const EYE16: &str = "convolution/eye16.v";
/// `[45 -17 -25 / -33 -15 -34 / 55 53 -26]`, scale 3. `vips_convi_intize`
/// ACCEPTS this mask and the two libvips paths still differ by 57.
const HOSTILE: &str = "convolution/hostile.mat";

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
fn conv_blur_integer_matches_vips_exact() {
    // EXACT (was ≤1 LSB before #558). The reference is generated with
    // VIPS_NOVECTOR=1, i.e. against `vips_convi_gen` — the portable C loop
    // libvips's own docs name as the specification and the core implements.
    // Against a default `vips` this same case differs on 72 of 256 samples.
    // Non-vacuous: a box blur of the zone-plate moves pixels by up to 59.
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
        EXACT,
    );
}

#[test]
fn conv_box_window_sums_to_1147_matches_vips_exact() {
    // The discriminator that kills "it is just the rounding mode". Every
    // replicated 3x3 window of this fixture sums to 1147, and:
    //
    //   C path      (1147 + 9/2) / 9       = 127
    //   floor       floor(1147 / 9)        = 127   <- SAME
    //   vector path (57 * 1147 + 256) >> 9 = 128   <- different COEFFICIENTS
    //
    // The vector path is filtering with 57/512 = 0.111328, not 1/9. So this
    // case cannot be reconciled by changing how the core rounds; the two
    // libvips paths are computing different convolutions.
    if skip_if_no_cli("conv_boxsum1147_int") {
        return;
    }
    let out = out_path("conv_boxsum1147_int.png");
    run_viprs_ok(&[
        "conv",
        &fx(BOXSUM1147),
        out.to_str().unwrap(),
        &fx(BLUR),
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/conv_boxsum1147_int_expected.png"),
        EXACT,
    );
}

#[test]
fn conv_hostile_mask_matches_vips_exact() {
    // The bound-breaking regression. `vips_convi_intize` ACCEPTS this mask —
    // VIPS_INFO=1 reports "convi: using vector path" — and the two libvips
    // paths still differ by 57 of 255 on this input. That is why integer conv
    // has no honest tolerance: the gate (convi.c:1096-1113) is a DC-gain check
    // against exact real arithmetic at one grey level on a flat field. It
    // constrains sum(w_hat - w); per-pixel error is sum((w_hat - w) * p), and
    // nothing bounds that.
    //
    // If this test ever fails, the reference was regenerated against the wrong
    // libvips. Do not add a tolerance.
    if skip_if_no_cli("conv_hostile_int") {
        return;
    }
    let out = out_path("conv_hostile_int.png");
    run_viprs_ok(&[
        "conv",
        &fx(NOISE64),
        out.to_str().unwrap(),
        &fx(HOSTILE),
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/conv_hostile_int_expected.png"),
        EXACT,
    );
}

#[test]
fn conv_scale_1_mask_integer_matches_vips_exact() {
    // Regime pin 1 of 3: a scale-1 mask is exact on BOTH libvips paths. The
    // vector path really does run here (VIPS_INFO=1: "convi: using vector
    // path"); it agrees because `vips_convi_intize` is the identity when the
    // mask already divides by 1, so both kernels convolve with the same
    // coefficients. The suite has always leaned on this — it is why the
    // scale-1 compass case was EXACT while conv was not — and never tested it.
    if skip_if_no_cli("conv_sobel_int") {
        return;
    }
    let out = out_path("conv_sobel_int.png");
    run_viprs_ok(&[
        "conv",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(SOBEL),
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/conv_sobel_int_expected.png"),
        EXACT,
    );
}

#[test]
fn conv_ushort_integer_matches_vips_exact() {
    // Regime pin 2 of 3: 16-bit input, where the divergence cannot happen at
    // all. `vips_convi` gates its vector path on
    // `BandFmt == VIPS_FORMAT_UCHAR` (convi.c:1151), so every libvips build
    // runs the C path on a ushort image and libviprs cannot be wrong about it.
    // The same blur.mat on the uchar `eye.png` differs between the paths.
    if skip_if_no_cli("conv_ushort_int") {
        return;
    }
    let out = out_path("conv_ushort_int.v");
    run_viprs_ok(&[
        "conv",
        &fx(EYE16),
        out.to_str().unwrap(),
        &fx(BLUR),
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/conv_ushort_int_expected.v"),
        EXACT,
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
// convsep — S1, separable (1xN) matrix-file mask.
// ---------------------------------------------------------------------------

#[test]
fn convsep_integer_matches_vips_exact() {
    // New with #558, and it is the case that most embarrasses the old ≤1 LSB
    // story: integer convsep was never covered at all, and on this same
    // eye.png the two libvips paths differ by 4. The scale-10 mask is applied
    // twice, so each pass's requantisation error compounds.
    if skip_if_no_cli("convsep_int") {
        return;
    }
    let out = out_path("convsep_int.png");
    run_viprs_ok(&[
        "convsep",
        &fx(EYE),
        out.to_str().unwrap(),
        &fx(SEP),
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/convsep_int_expected.png"),
        EXACT,
    );
}

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
fn gaussblur_integer_matches_vips_exact() {
    // EXACT (was ≤1 LSB before #558), against a VIPS_NOVECTOR=1 reference.
    // Non-vacuous — blurs the zone-plate.
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
        EXACT,
    );
}

#[test]
fn gaussblur_sigma_1_6_integer_matches_vips_exact() {
    // The case that proves the old tolerance was luck rather than a bound.
    // Sigma 1.5 (above) differs between the two libvips paths by 1, which the
    // ≤1 tolerance absorbed. Sigma 1.6 differs by 4 on 256 samples of this
    // same fixture, so the identical test at the next sigma would have failed
    // the moment anybody wrote it. The separable gaussmat's scale goes 64 at
    // sigma 1.4 (a power of two, hence exact) to 70 at 1.6.
    if skip_if_no_cli("gaussblur_s1.6_int") {
        return;
    }
    let out = out_path("gaussblur_s1.6_int.png");
    run_viprs_ok(&[
        "gaussblur",
        &fx(EYE),
        out.to_str().unwrap(),
        "1.6",
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussblur_s1.6_int_expected.png"),
        EXACT,
    );
}

#[test]
fn gaussblur_sigma_0_8_integer_matches_vips_exact() {
    // Sigma 0.8 on the high-entropy input: separable gaussmat scale 38, the
    // two libvips paths differ on 1550 of 4096 samples by up to 2. Pinned
    // because sigma 1.4 — the DEFAULT, and the value every convenience call
    // uses — is the one sigma whose separable mask has a power-of-two scale,
    // so a suite pinned only at the default sees none of this. (The 2D
    // gaussmat at 1.4 has scale 216 and is NOT lucky; the luck belongs to the
    // separable mask alone.)
    if skip_if_no_cli("gaussblur_s0.8_int") {
        return;
    }
    let out = out_path("gaussblur_s0.8_int.png");
    run_viprs_ok(&[
        "gaussblur",
        &fx(NOISE64),
        out.to_str().unwrap(),
        "0.8",
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussblur_s0.8_int_expected.png"),
        EXACT,
    );
}

#[test]
fn gaussblur_sigma_0_6_matches_vips_exact() {
    // Regime pin 3 of 3: the mask libvips itself refuses. At sigma 0.6 the
    // separable gaussmat is 3x1 with scale 30, and `vips_convi_intize` bails —
    // VIPS_INFO=1 prints "vips_convi_intize: too inaccurate" followed by
    // "convi: using C path" — so the vectorised binary runs the same kernel
    // libviprs does.
    //
    // NB this is a property of THIS mask, not of "sigma <= 0.6": at sigma 0.5
    // the separable mask is 1x1 with scale 20 and libvips runs the VECTOR
    // path, agreeing only because a 1x1 requantises exactly. And the 2D
    // gaussmat at the same sigma 0.6 (3x3, scale 44) takes the vector path
    // and differs on 4065 of 4096 samples by up to 4 — the 2D and the
    // separable mask for one sigma land on opposite sides of the predicate.
    // There is no sigma threshold; there is a per-mask predicate, and nothing
    // on the API surface says which side you are on.
    if skip_if_no_cli("gaussblur_s0.6_int") {
        return;
    }
    let out = out_path("gaussblur_s0.6_int.png");
    run_viprs_ok(&[
        "gaussblur",
        &fx(EYE),
        out.to_str().unwrap(),
        "0.6",
        "--precision",
        "integer",
    ]);
    decode_compare(
        &out,
        &cli_fixture("convolution/gaussblur_s0.6_int_expected.png"),
        EXACT,
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
// sharpen — S1, LabS unsharp mask. m1/m2 nonzero so it truly sharpens.
//
// This is the ONLY remaining tolerance in the cell, it has its own constant,
// and it is NOT issue #558. sharpen convolves the L of LabS, which is 16-bit,
// and `vips_convi`'s vector path is gated on `BandFmt == VIPS_FORMAT_UCHAR`
// (convi.c:1151), so both libvips builds take the portable C path here —
// confirmed with VIPS_INFO=1. 44 of 256 samples deviate; that is a real
// libviprs bug, tracked as issue #581, and until #558 it was hiding under a
// constant shared with integer conv.
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
    decode_compare(
        &out,
        &cli_fixture("convolution/sharpen_expected.png"),
        SHARPEN_LSB,
    );
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
