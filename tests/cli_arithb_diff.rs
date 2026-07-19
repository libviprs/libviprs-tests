//! CLI-DIFFERENTIAL suite — arithmetic **part B** (the Wave-2 arith-b lane,
//! CLI_CONTRACT.md §7, OP_MAP.md arithmetic section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/arithb/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the bands/morphology reference cells exactly,
//! including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! Oracle classes are HONEST — each tolerance is the max-abs-diff MEASURED
//! against vips 8.18.4 on the committed inputs (not a class picked to make a
//! rigged input pass):
//!
//! * **EXACT / EAC, tol 0**: `subtract` (float diff, PNG save-cast clips
//!   negatives), `multiply` (ushort `.v`), `divide` (float `.v`), `minpair`,
//!   `maxpair`, `sum` (>=3 variadic, ushort `.v`), `relational(_const)`,
//!   `boolean(_const)`.
//! * **FOURIER / float, eps 1e-6**: `math` (sin/cos/atan), `math2 atan2`,
//!   `complexform`, `complex` (polar/rect/conj), `complexget` (real/imag).
//!   Measured 0 on the committed inputs; the 1e-6 eps is f32-rounding headroom.
//! * **BOUNDED-TOL** (a genuine, documented core-vs-vips divergence, NOT hidden
//!   by input choice):
//!   - `scale` (linear + `--log`): ≤1 LSB, log path transcendental (measured 0).
//!   - `recomb`: 1 LSB — core rounds & saturates per output band into the input
//!     depth while vips computes in float and casts once (measured 1). A
//!     documented deviation from OP_MAP's EAC prediction (core issue #491).
//!   - `premultiply` / `unpremultiply`: ≤1 LSB post-cast (vips emits float, core
//!     rounds into the uchar depth; measured 1).
//!   - `stdif`: 6 — core CLIPS the sliding window at the image border while vips
//!     MIRRORS, so a genuinely 2-D input (the `eye` zone-plate) diverges in the
//!     border ring (interior exact). Measured 6 on eye 3x3. A documented
//!     edge-handling divergence (core issue #490).
//!   - `math2 pow`: 1 — the single `pow(0,0)` sample diverges (Rust
//!     `f64::powf(0,0)=1` vs libvips `0`). The 0^0 edge is EXERCISED (base and
//!     exponent both span 0), not hidden (core issue #489).
//!
//! If the `libviprs-cli` sibling is not checked out, every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! (with `VIPRS_REQUIRE_CLI=1`) lays the CLI down and actually exercises these
//! (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs_ok};

use tempfile::TempDir;

/// EXACT / EXACT-AFTER-CAST: bit-exact decode comparison (CLI_CONTRACT.md §5).
const EXACT: f64 = 0.0;

/// FOURIER / float ops (`math`, `math2 atan2`, complex family): f32-rounding
/// headroom eps (measured max-abs-diff was 0 on the committed inputs).
const FLOAT_EPS: f64 = 1e-6;

/// `scale` (linear + log): ≤1 LSB (log path transcendental; measured 0).
const SCALE_TOL: f64 = 1.0;

/// `recomb`: 1 LSB — per-band round-and-saturate vs vips float-then-cast.
const RECOMB_TOL: f64 = 1.0;

/// `premultiply` / `unpremultiply`: ≤1 LSB post-cast.
const PREMULT_TOL: f64 = 1.0;

/// `stdif`: border clip-vs-mirror divergence, measured 6 on eye 3x3.
const STDIF_TOL: f64 = 6.0;

/// `math2 pow`: the single `pow(0,0)` sample (Rust 1 vs vips 0).
const POW_TOL: f64 = 1.0;

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

// Common committed inputs (tests/fixtures/cli/arithb/).
const A: &str = "arithb/a.png";
const B: &str = "arithb/b.png";
const C: &str = "arithb/c.png";
const RGB: &str = "arithb/rgb.png";
const RGBA: &str = "arithb/rgba.png";
const SMALL: &str = "arithb/small.png";
const SMALL2: &str = "arithb/small2.png";
const EYE: &str = "arithb/eye.png";
const MAT: &str = "arithb/recomb.mat";
const CPX_IN: &str = "arithb/complex_in.v";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// Convenience: the output temp path as an owned `String`.
fn op(name: &str) -> String {
    out_path(name).to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Binary (S2): subtract / multiply / divide / minpair / maxpair.
// ---------------------------------------------------------------------------

#[test]
fn subtract_matches_vips_exact() {
    if skip_if_no_cli("subtract") {
        return;
    }
    // a - b straddles 0 across the image; the float diff's PNG save-cast clips
    // negatives to 0, exactly as vips's signed-short → PNG path.
    let out = op("subtract.png");
    run_viprs_ok(&["subtract", &fx(A), &fx(B), &out]);
    decode_compare(
        &out_path("subtract.png"),
        &cli_fixture("arithb/subtract_expected.png"),
        EXACT,
    );
}

#[test]
fn multiply_matches_vips_exact() {
    if skip_if_no_cli("multiply") {
        return;
    }
    // uchar * uchar widens to ushort in both core and vips → carried as `.v`.
    let out = op("multiply.v");
    run_viprs_ok(&["multiply", &fx(A), &fx(B), &out]);
    decode_compare(
        &out_path("multiply.v"),
        &cli_fixture("arithb/multiply_expected.v"),
        EXACT,
    );
}

#[test]
fn divide_matches_vips_exact() {
    if skip_if_no_cli("divide") {
        return;
    }
    // b is strictly > 0 (no div-by-zero); the float quotient matches bit-exactly
    // via the `.v` carrier (the uchar save-cast path instead diverges by 1 LSB).
    let out = op("divide.v");
    run_viprs_ok(&["divide", &fx(A), &fx(B), &out]);
    decode_compare(
        &out_path("divide.v"),
        &cli_fixture("arithb/divide_expected.v"),
        EXACT,
    );
}

#[test]
fn minpair_matches_vips_exact() {
    if skip_if_no_cli("minpair") {
        return;
    }
    let out = op("minpair.png");
    run_viprs_ok(&["minpair", &fx(A), &fx(B), &out]);
    decode_compare(
        &out_path("minpair.png"),
        &cli_fixture("arithb/minpair_expected.png"),
        EXACT,
    );
}

#[test]
fn maxpair_matches_vips_exact() {
    if skip_if_no_cli("maxpair") {
        return;
    }
    let out = op("maxpair.png");
    run_viprs_ok(&["maxpair", &fx(A), &fx(B), &out]);
    decode_compare(
        &out_path("maxpair.png"),
        &cli_fixture("arithb/maxpair_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// sum — S2 variadic with THREE inputs (the >=3 fold io::inputs_and_out drives).
// ---------------------------------------------------------------------------

#[test]
fn sum_three_inputs_matches_vips_exact() {
    if skip_if_no_cli("sum") {
        return;
    }
    // a + b + c exercises the true >=3 variadic array (the 2-input case would
    // run one accumulation; three runs the fold). vips promotes to UINT; the
    // reference is cast to ushort to match core's ushort output (sum <= 543).
    let out = op("sum.v");
    run_viprs_ok(&["sum", &fx(A), &fx(B), &fx(C), &out]);
    decode_compare(
        &out_path("sum.v"),
        &cli_fixture("arithb/sum_expected.v"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// relational / relational_const — enum variants that actually differ.
// ---------------------------------------------------------------------------

#[test]
fn relational_more_matches_vips_exact() {
    if skip_if_no_cli("relational_more") {
        return;
    }
    let out = op("relational_more.png");
    run_viprs_ok(&["relational", &fx(A), &fx(B), &out, "more"]);
    decode_compare(
        &out_path("relational_more.png"),
        &cli_fixture("arithb/relational_more_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_less_matches_vips_exact() {
    if skip_if_no_cli("relational_less") {
        return;
    }
    // `less` is the complement of `more` here — a distinct, non-vacuous enum.
    let out = op("relational_less.png");
    run_viprs_ok(&["relational", &fx(A), &fx(B), &out, "less"]);
    decode_compare(
        &out_path("relational_less.png"),
        &cli_fixture("arithb/relational_less_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_const_more_matches_vips_exact() {
    if skip_if_no_cli("relational_const_more") {
        return;
    }
    let out = op("relational_const_more.png");
    run_viprs_ok(&["relational_const", &fx(A), &out, "more", "128"]);
    decode_compare(
        &out_path("relational_const_more.png"),
        &cli_fixture("arithb/relational_const_more_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// boolean / boolean_const — enum variants (and the const shift path).
// ---------------------------------------------------------------------------

#[test]
fn boolean_eor_matches_vips_exact() {
    if skip_if_no_cli("boolean_eor") {
        return;
    }
    let out = op("boolean_eor.png");
    run_viprs_ok(&["boolean", &fx(A), &fx(B), &out, "eor"]);
    decode_compare(
        &out_path("boolean_eor.png"),
        &cli_fixture("arithb/boolean_eor_expected.png"),
        EXACT,
    );
}

#[test]
fn boolean_and_matches_vips_exact() {
    if skip_if_no_cli("boolean_and") {
        return;
    }
    let out = op("boolean_and.png");
    run_viprs_ok(&["boolean", &fx(A), &fx(B), &out, "and"]);
    decode_compare(
        &out_path("boolean_and.png"),
        &cli_fixture("arithb/boolean_and_expected.png"),
        EXACT,
    );
}

#[test]
fn boolean_const_and_matches_vips_exact() {
    if skip_if_no_cli("boolean_const_and") {
        return;
    }
    let out = op("boolean_const_and.png");
    run_viprs_ok(&["boolean_const", &fx(A), &out, "and", "200"]);
    decode_compare(
        &out_path("boolean_const_and.png"),
        &cli_fixture("arithb/boolean_const_and_expected.png"),
        EXACT,
    );
}

#[test]
fn boolean_const_lshift_matches_vips_exact() {
    if skip_if_no_cli("boolean_const_lshift") {
        return;
    }
    // The const-shift path (u32 count): a << 2, truncated into the uchar depth.
    let out = op("boolean_const_lshift.png");
    run_viprs_ok(&["boolean_const", &fx(A), &out, "lshift", "2"]);
    decode_compare(
        &out_path("boolean_const_lshift.png"),
        &cli_fixture("arithb/boolean_const_lshift_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// Windowed: scale (--log flag path) / stdif / recomb / (un)premultiply.
// ---------------------------------------------------------------------------

#[test]
fn scale_linear_matches_vips_bounded_tol() {
    if skip_if_no_cli("scale_linear") {
        return;
    }
    let out = op("scale.png");
    run_viprs_ok(&["scale", &fx(RGB), &out]);
    decode_compare(
        &out_path("scale.png"),
        &cli_fixture("arithb/scale_expected.png"),
        SCALE_TOL,
    );
}

#[test]
fn scale_log_matches_vips_bounded_tol() {
    if skip_if_no_cli("scale_log") {
        return;
    }
    // The `--log` flag path (transcendental log-scaling curve).
    let out = op("scale_log.png");
    run_viprs_ok(&["scale", &fx(RGB), &out, "--log"]);
    decode_compare(
        &out_path("scale_log.png"),
        &cli_fixture("arithb/scale_log_expected.png"),
        SCALE_TOL,
    );
}

#[test]
fn stdif_matches_vips_bounded_tol() {
    if skip_if_no_cli("stdif") {
        return;
    }
    // On the 2-D `eye` input the sliding-window statistics diverge at the border
    // (core clips, vips mirrors) by up to 6 raw units; the interior is exact.
    // A documented core-vs-vips edge-handling divergence (core issue #490; see the module header).
    let out = op("stdif.png");
    run_viprs_ok(&["stdif", &fx(EYE), &out, "3", "3"]);
    decode_compare(
        &out_path("stdif.png"),
        &cli_fixture("arithb/stdif_expected.png"),
        STDIF_TOL,
    );
}

#[test]
fn recomb_matches_vips_bounded_tol() {
    if skip_if_no_cli("recomb") {
        return;
    }
    // Matrix FILE arg via the shared matfile loader. BOUNDED-TOL 1: core rounds
    // & saturates per output band into the uchar depth while vips computes in
    // float and casts once (a documented deviation from OP_MAP's EAC prediction; core issue #491).
    let out = op("recomb.png");
    run_viprs_ok(&["recomb", &fx(RGB), &out, &fx(MAT)]);
    decode_compare(
        &out_path("recomb.png"),
        &cli_fixture("arithb/recomb_expected.png"),
        RECOMB_TOL,
    );
}

#[test]
fn premultiply_matches_vips_bounded_tol() {
    if skip_if_no_cli("premultiply") {
        return;
    }
    let out = op("premultiply.png");
    run_viprs_ok(&["premultiply", &fx(RGBA), &out]);
    decode_compare(
        &out_path("premultiply.png"),
        &cli_fixture("arithb/premultiply_expected.png"),
        PREMULT_TOL,
    );
}

#[test]
fn unpremultiply_matches_vips_bounded_tol() {
    if skip_if_no_cli("unpremultiply") {
        return;
    }
    let out = op("unpremultiply.png");
    run_viprs_ok(&["unpremultiply", &fx(RGBA), &out]);
    decode_compare(
        &out_path("unpremultiply.png"),
        &cli_fixture("arithb/unpremultiply_expected.png"),
        PREMULT_TOL,
    );
}

// ---------------------------------------------------------------------------
// Math (float → .v): math (sin/cos/atan) / math2 (atan2/pow).
// ---------------------------------------------------------------------------

#[test]
fn math_sin_matches_vips_float() {
    if skip_if_no_cli("math_sin") {
        return;
    }
    let out = op("math_sin.v");
    run_viprs_ok(&["math", &fx(A), &out, "sin"]);
    decode_compare(
        &out_path("math_sin.v"),
        &cli_fixture("arithb/math_sin_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn math_cos_matches_vips_float() {
    if skip_if_no_cli("math_cos") {
        return;
    }
    let out = op("math_cos.v");
    run_viprs_ok(&["math", &fx(A), &out, "cos"]);
    decode_compare(
        &out_path("math_cos.v"),
        &cli_fixture("arithb/math_cos_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn math_atan_matches_vips_float() {
    if skip_if_no_cli("math_atan") {
        return;
    }
    let out = op("math_atan.v");
    run_viprs_ok(&["math", &fx(A), &out, "atan"]);
    decode_compare(
        &out_path("math_atan.v"),
        &cli_fixture("arithb/math_atan_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn math2_atan2_matches_vips_float() {
    if skip_if_no_cli("math2_atan2") {
        return;
    }
    let out = op("math2_atan2.v");
    run_viprs_ok(&["math2", &fx(A), &fx(B), &out, "atan2"]);
    decode_compare(
        &out_path("math2_atan2.v"),
        &cli_fixture("arithb/math2_atan2_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn math2_pow_matches_vips_bounded_tol() {
    if skip_if_no_cli("math2_pow") {
        return;
    }
    // small (0..15) ^ small2 (0..3): both span 0 so the pow(0,0) edge is present
    // — Rust's f64::powf(0,0)=1 vs libvips 0, the single divergent sample (tol 1,
    // core issue #489; NOT hidden by excluding 0 from the input).
    let out = op("math2_pow.v");
    run_viprs_ok(&["math2", &fx(SMALL), &fx(SMALL2), &out, "pow"]);
    decode_compare(
        &out_path("math2_pow.v"),
        &cli_fixture("arithb/math2_pow_expected.v"),
        POW_TOL,
    );
}

// ---------------------------------------------------------------------------
// Complex (FOURIER float band-pairs, .v): complexform / complex / complexget.
// ---------------------------------------------------------------------------

#[test]
fn complexform_matches_vips_fourier() {
    if skip_if_no_cli("complexform") {
        return;
    }
    // Two reals → (re,im) f32 band pairs. The vips reference is a header-relabel
    // reinterpret of vips's complex output to a 2-band float `.v` (same bytes).
    let out = op("complexform.v");
    run_viprs_ok(&["complexform", &fx(A), &fx(B), &out]);
    decode_compare(
        &out_path("complexform.v"),
        &cli_fixture("arithb/complexform_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn complex_polar_matches_vips_fourier() {
    if skip_if_no_cli("complex_polar") {
        return;
    }
    let out = op("complex_polar.v");
    run_viprs_ok(&["complex", &fx(CPX_IN), &out, "polar"]);
    decode_compare(
        &out_path("complex_polar.v"),
        &cli_fixture("arithb/complex_polar_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn complex_rect_matches_vips_fourier() {
    if skip_if_no_cli("complex_rect") {
        return;
    }
    let out = op("complex_rect.v");
    run_viprs_ok(&["complex", &fx(CPX_IN), &out, "rect"]);
    decode_compare(
        &out_path("complex_rect.v"),
        &cli_fixture("arithb/complex_rect_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn complex_conj_matches_vips_fourier() {
    if skip_if_no_cli("complex_conj") {
        return;
    }
    let out = op("complex_conj.v");
    run_viprs_ok(&["complex", &fx(CPX_IN), &out, "conj"]);
    decode_compare(
        &out_path("complex_conj.v"),
        &cli_fixture("arithb/complex_conj_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn complexget_real_matches_vips_fourier() {
    if skip_if_no_cli("complexget_real") {
        return;
    }
    let out = op("complexget_real.v");
    run_viprs_ok(&["complexget", &fx(CPX_IN), &out, "real"]);
    decode_compare(
        &out_path("complexget_real.v"),
        &cli_fixture("arithb/complexget_real_expected.v"),
        FLOAT_EPS,
    );
}

#[test]
fn complexget_imag_matches_vips_fourier() {
    if skip_if_no_cli("complexget_imag") {
        return;
    }
    let out = op("complexget_imag.v");
    run_viprs_ok(&["complexget", &fx(CPX_IN), &out, "imag"]);
    decode_compare(
        &out_path("complexget_imag.v"),
        &cli_fixture("arithb/complexget_imag_expected.v"),
        FLOAT_EPS,
    );
}
