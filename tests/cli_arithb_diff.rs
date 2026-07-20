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
//! * **EXACT / EAC, tol 0**: `subtract` (core saturates at 0; a-b PLUS an a>=b
//!   case with no clip dead-zone), `multiply` (ushort `.v`), `divide` (float
//!   `.v`), `minpair`, `maxpair`, `sum` (>=3 variadic, ushort `.v`),
//!   `relational(_const)` (ALL SIX enum arms), `boolean` (and/or/eor),
//!   `boolean_const` (and/or/eor/lshift/rshift).
//! * **FOURIER / float, eps 1e-6**: `math` (ALL SIXTEEN arms — sin/cos/atan on
//!   a.png, the other 13 on the in-domain float inputs), `math2 atan2`,
//!   `complexform`, `complex` (polar/rect/conj), `complexget` (real/imag).
//!   Measured 0 on the committed inputs; the 1e-6 eps is f32-rounding headroom.
//! * **EXACT, tol 0 (upgraded from BOUNDED-TOL by the core parity fixes)**:
//!   - `recomb`: core issue #491 recomputes the band recombination in f32 and
//!     truncates once (as vips does), so integer-in/integer-out is now bit-exact
//!     (was ≤1 LSB per-band round-and-saturate).
//!   - `stdif`: core issue #490 switched the sliding-window border to
//!     edge-replicate, so the WHOLE image — border ring included — matches vips
//!     on the 2-D `eye` zone-plate (was whole-image 6 / interior 0 from a
//!     core-clip-vs-vips-mirror border divergence; the old separate interior-only
//!     crop is retired).
//!   - `math2 pow` / `math2 wop`: core issue #489 made `pow(0, ≤0) = 0` (was Rust
//!     `f64::powf(0,0)=1` vs libvips `0` on the single 0^0 sample), so both match
//!     vips bit-for-bit. The 0^0 edge is still EXERCISED (base and exponent both
//!     span 0), not hidden.
//! * **BOUNDED-TOL** (a genuine, documented core-vs-vips divergence, NOT hidden
//!   by input choice):
//!   - `scale` (linear + `--log`): ≤1 LSB, log path transcendental (measured 0).
//!   - `premultiply` / `unpremultiply`: ≤1 LSB post-cast (vips emits float, core
//!     rounds into the uchar depth; measured 1).
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

/// `premultiply` / `unpremultiply`: ≤1 LSB post-cast.
const PREMULT_TOL: f64 = 1.0;

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
/// `a` rotated d90 (vertical ramp, same 0..255 value set): equal to `a` on the
/// x==y diagonal, so the relational enum arms are non-vacuously discriminated.
const AVERT: &str = "arithb/avert.png";
/// Float input in (0.1, 0.95) — in-domain for the transcendental math ops.
const MSMALL: &str = "arithb/msmall.v";
/// Float input in [1, 6] — the `acosh` domain (>=1), which `msmall` (<1) cannot cover.
const MACOSH: &str = "arithb/macosh.v";

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
fn subtract_pos_matches_vips_exact() {
    if skip_if_no_cli("subtract_pos") {
        return;
    }
    // a - small: a (horizontal ramp 0..255) >= small (horizontal ramp 0..15) at
    // EVERY pixel, so the difference is non-negative across the whole image — no
    // clip dead-zone. The a-b case pins ~half the pixels at 0 (both core and vips
    // saturate a<b to 0; core's try_sub itself saturates, so a .v carrier could
    // not recover them), leaving that region unable to distinguish a bug. This
    // case exercises the full subtraction range (review finding 4).
    let out = op("subtract_pos.png");
    run_viprs_ok(&["subtract", &fx(A), &fx(SMALL), &out]);
    decode_compare(
        &out_path("subtract_pos.png"),
        &cli_fixture("arithb/subtract_pos_expected.png"),
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

// The remaining four relational arms on (a, avert): avert = a rotated d90, so
// equality holds on the x==y diagonal (16 pixels). A non-empty equality set makes
// each arm distinct — lesseq != less, moreeq != more, equal != noteq — so a
// dispatch swap in any of these arms fails (review finding 1).

#[test]
fn relational_equal_matches_vips_exact() {
    if skip_if_no_cli("relational_equal") {
        return;
    }
    let out = op("relational_equal.png");
    run_viprs_ok(&["relational", &fx(A), &fx(AVERT), &out, "equal"]);
    decode_compare(
        &out_path("relational_equal.png"),
        &cli_fixture("arithb/relational_equal_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_noteq_matches_vips_exact() {
    if skip_if_no_cli("relational_noteq") {
        return;
    }
    let out = op("relational_noteq.png");
    run_viprs_ok(&["relational", &fx(A), &fx(AVERT), &out, "noteq"]);
    decode_compare(
        &out_path("relational_noteq.png"),
        &cli_fixture("arithb/relational_noteq_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_lesseq_matches_vips_exact() {
    if skip_if_no_cli("relational_lesseq") {
        return;
    }
    // Differs from `less` exactly on the equality diagonal — a lesseq<->less swap
    // is caught here.
    let out = op("relational_lesseq.png");
    run_viprs_ok(&["relational", &fx(A), &fx(AVERT), &out, "lesseq"]);
    decode_compare(
        &out_path("relational_lesseq.png"),
        &cli_fixture("arithb/relational_lesseq_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_moreeq_matches_vips_exact() {
    if skip_if_no_cli("relational_moreeq") {
        return;
    }
    // Differs from `more` exactly on the equality diagonal — a moreeq<->more swap
    // is caught here.
    let out = op("relational_moreeq.png");
    run_viprs_ok(&["relational", &fx(A), &fx(AVERT), &out, "moreeq"]);
    decode_compare(
        &out_path("relational_moreeq.png"),
        &cli_fixture("arithb/relational_moreeq_expected.png"),
        EXACT,
    );
}

// The remaining four relational_const arms against C=7 on small.png (samples are
// EXACTLY 0..15, so `== 7` is a non-empty column): lesseq != less, equal != noteq
// (review finding 1).

#[test]
fn relational_const_equal_matches_vips_exact() {
    if skip_if_no_cli("relational_const_equal") {
        return;
    }
    let out = op("relational_const_equal.png");
    run_viprs_ok(&["relational_const", &fx(SMALL), &out, "equal", "7"]);
    decode_compare(
        &out_path("relational_const_equal.png"),
        &cli_fixture("arithb/relational_const_equal_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_const_noteq_matches_vips_exact() {
    if skip_if_no_cli("relational_const_noteq") {
        return;
    }
    let out = op("relational_const_noteq.png");
    run_viprs_ok(&["relational_const", &fx(SMALL), &out, "noteq", "7"]);
    decode_compare(
        &out_path("relational_const_noteq.png"),
        &cli_fixture("arithb/relational_const_noteq_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_const_lesseq_matches_vips_exact() {
    if skip_if_no_cli("relational_const_lesseq") {
        return;
    }
    let out = op("relational_const_lesseq.png");
    run_viprs_ok(&["relational_const", &fx(SMALL), &out, "lesseq", "7"]);
    decode_compare(
        &out_path("relational_const_lesseq.png"),
        &cli_fixture("arithb/relational_const_lesseq_expected.png"),
        EXACT,
    );
}

#[test]
fn relational_const_moreeq_matches_vips_exact() {
    if skip_if_no_cli("relational_const_moreeq") {
        return;
    }
    let out = op("relational_const_moreeq.png");
    run_viprs_ok(&["relational_const", &fx(SMALL), &out, "moreeq", "7"]);
    decode_compare(
        &out_path("relational_const_moreeq.png"),
        &cli_fixture("arithb/relational_const_moreeq_expected.png"),
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

#[test]
fn boolean_or_matches_vips_exact() {
    if skip_if_no_cli("boolean_or") {
        return;
    }
    // The third boolean arm (and|or|eor) — an or<->and/eor swap is caught since
    // all three are now tested (review finding 1).
    let out = op("boolean_or.png");
    run_viprs_ok(&["boolean", &fx(A), &fx(B), &out, "or"]);
    decode_compare(
        &out_path("boolean_or.png"),
        &cli_fixture("arithb/boolean_or_expected.png"),
        EXACT,
    );
}

#[test]
fn boolean_const_or_matches_vips_exact() {
    if skip_if_no_cli("boolean_const_or") {
        return;
    }
    let out = op("boolean_const_or.png");
    run_viprs_ok(&["boolean_const", &fx(A), &out, "or", "200"]);
    decode_compare(
        &out_path("boolean_const_or.png"),
        &cli_fixture("arithb/boolean_const_or_expected.png"),
        EXACT,
    );
}

#[test]
fn boolean_const_eor_matches_vips_exact() {
    if skip_if_no_cli("boolean_const_eor") {
        return;
    }
    let out = op("boolean_const_eor.png");
    run_viprs_ok(&["boolean_const", &fx(A), &out, "eor", "200"]);
    decode_compare(
        &out_path("boolean_const_eor.png"),
        &cli_fixture("arithb/boolean_const_eor_expected.png"),
        EXACT,
    );
}

#[test]
fn boolean_const_rshift_matches_vips_exact() {
    if skip_if_no_cli("boolean_const_rshift") {
        return;
    }
    // The SECOND shift arm (a >> 2). With lshift also tested, an rshift-maps-to-
    // lshift bug fails (review finding 1).
    let out = op("boolean_const_rshift.png");
    run_viprs_ok(&["boolean_const", &fx(A), &out, "rshift", "2"]);
    decode_compare(
        &out_path("boolean_const_rshift.png"),
        &cli_fixture("arithb/boolean_const_rshift_expected.png"),
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
fn stdif_matches_vips_exact() {
    if skip_if_no_cli("stdif") {
        return;
    }
    // EXACT (tol 0) across the WHOLE image, including the border ring. Core issue
    // #490 switched the sliding-window border handling to edge-replicate,
    // matching vips 8.18.4 on the 2-D `eye` input (previously the core CLIPPED
    // while vips MIRRORED, diverging up to 6 raw units in the 1px border ring —
    // so the old suite carried a whole-image tol-6 case plus a separate
    // interior-only tol-0 crop; both collapse to a single whole-image tol-0
    // comparison now).
    let out = op("stdif.png");
    run_viprs_ok(&["stdif", &fx(EYE), &out, "3", "3"]);
    decode_compare(
        &out_path("stdif.png"),
        &cli_fixture("arithb/stdif_expected.png"),
        EXACT,
    );
}

#[test]
fn recomb_matches_vips_exact() {
    if skip_if_no_cli("recomb") {
        return;
    }
    // Matrix FILE arg via the shared matfile loader. EXACT (tol 0): core issue
    // #491 recomputes the band recombination in f32 and truncates once (as vips
    // does), so the integer-in/integer-out result is now bit-exact (previously
    // per-band round-and-saturate diverged ≤1 LSB from vips's float-then-cast).
    let out = op("recomb.png");
    run_viprs_ok(&["recomb", &fx(RGB), &out, &fx(MAT)]);
    decode_compare(
        &out_path("recomb.png"),
        &cli_fixture("arithb/recomb_expected.png"),
        EXACT,
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
// Math (float → .v): ALL 16 math arms + all 3 math2 arms (review finding 1).
// sin/cos/atan on a.png (degrees, defined over 0..255); the other 13 on the
// in-domain float inputs (msmall in (0.1,0.95); macosh in [1,6] for acosh).
// Each arm has its OWN vips reference, so a log<->log10, exp<->exp10 or
// sinh<->cosh dispatch swap fails. Measured max-abs-diff 0 for all math arms.
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
fn math_remaining_thirteen_arms_match_vips_float() {
    if skip_if_no_cli("math_remaining") {
        return;
    }
    // The 13 math arms sin/cos/atan do NOT cover — each compared against its own
    // vips reference so a copy-paste dispatch swap (log<->log10, exp<->exp10,
    // sinh<->cosh, …) fails (review finding 1). tan/asin/acos/log/log10/exp/exp10/
    // sinh/cosh/tanh/asinh/atanh run on msmall (0.1..0.95, in-domain and O(1));
    // acosh runs on macosh (>=1). All measured 0; FLOAT_EPS is f32 headroom.
    let cases: &[(&str, &str)] = &[
        ("tan", MSMALL),
        ("asin", MSMALL),
        ("acos", MSMALL),
        ("log", MSMALL),
        ("log10", MSMALL),
        ("exp", MSMALL),
        ("exp10", MSMALL),
        ("sinh", MSMALL),
        ("cosh", MSMALL),
        ("tanh", MSMALL),
        ("asinh", MSMALL),
        ("atanh", MSMALL),
        ("acosh", MACOSH),
    ];
    for (opname, input) in cases {
        let out = op(&format!("math_{opname}.v"));
        run_viprs_ok(&["math", &fx(input), &out, opname]);
        decode_compare(
            &out_path(&format!("math_{opname}.v")),
            &cli_fixture(&format!("arithb/math_{opname}_expected.v")),
            FLOAT_EPS,
        );
    }
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
fn math2_pow_matches_vips_exact() {
    if skip_if_no_cli("math2_pow") {
        return;
    }
    // small (0..15) ^ small2 (0..3): both span 0 so the pow(0,0) edge is present
    // and EXERCISED (not hidden by excluding 0 from the input). Core issue #489
    // made pow(0, ≤0) = 0, matching libvips (previously Rust's f64::powf(0,0)=1
    // diverged on that single sample), so the whole `.v` now matches vips 8.18.4
    // bit-for-bit — compared at tol 0.
    let out = op("math2_pow.v");
    run_viprs_ok(&["math2", &fx(SMALL), &fx(SMALL2), &out, "pow"]);
    decode_compare(
        &out_path("math2_pow.v"),
        &cli_fixture("arithb/math2_pow_expected.v"),
        EXACT,
    );
}

#[test]
fn math2_wop_matches_vips_exact() {
    if skip_if_no_cli("math2_wop") {
        return;
    }
    // The third math2 arm. wop = right^left, so `math2 small2 small wop` =
    // small^small2 (same magnitude as pow) and shares the pow(0,0) edge at x=0.
    // Core issue #489 made pow(0, ≤0) = 0 (previously Rust f64::powf(0,0)=1),
    // matching libvips, so this now matches vips 8.18.4 bit-for-bit (tol 0). A
    // pow<->wop swap is caught since both are tested (review finding 1).
    let out = op("math2_wop.v");
    run_viprs_ok(&["math2", &fx(SMALL2), &fx(SMALL), &out, "wop"]);
    decode_compare(
        &out_path("math2_wop.v"),
        &cli_fixture("arithb/math2_wop_expected.v"),
        EXACT,
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
