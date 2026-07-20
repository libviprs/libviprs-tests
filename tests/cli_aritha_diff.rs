//! CLI-DIFFERENTIAL suite — aritha family (arith part-A lane, CLI_CONTRACT.md
//! §7, OP_MAP.md arithmetic section: statistics / const-linear / unary-rounding
//! / hough rows).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/aritha/` and compares its output against the COMMITTED
//! vips 8.18.4 reference (or, for the two hough ops, a committed viprs golden
//! pin) at the op's honest §5 tolerance. The suite NEVER runs vips: references
//! are generated offline by `tools/gen_cli_expected.sh` and committed. This cell
//! copies the morphology / bands reference cells' skip-guard + `VIPRS_REQUIRE_CLI`
//! discipline exactly.
//!
//! Honest oracle classes (measured against vips 8.18.4):
//! * `avg` / `deviate` / `min` / `max` / `find_trim` — S3 scalars, compared
//!   numerically (never as text): `min`/`max` are integer-exact on the integer
//!   input; `avg`/`deviate` are a rational mean at the S3 relative epsilon.
//! * `stats` / `measure` — a float matrix `.v` (vips double cast to float; the
//!   core has no f64 pixel format). `stats` drops vips's 4 position columns
//!   (6..10 — a documented core subset), so the reference is cropped to 6
//!   columns. Both measured max-abs-diff 0.
//! * `profile` / `project` — 16-bit `.v` (vips INT/UINT cast to ushort).
//! * `linear` (float `.v` + `--uchar` PNG), `remainder_const` / `clamp` (PNG),
//!   `math2_const pow` / `abs` / `sign` / `round ceil` / `round floor` (float
//!   `.v`) — all measured 0. `sign`'s afloat now includes exactly 0.0, so the
//!   zero→0 branch (not just −1/+1) is exercised.
//! * `round rint` — **EXACT** (tol 0, upgraded from GOLDEN-ONLY): core issue #494
//!   changed the core to round half TO EVEN, matching vips's C `rint` at exact
//!   half-integers on the honest afloat that reaches x.5 (previously the core's
//!   `f64::round`, half away from zero, diverged by 1 at every x.5). The
//!   reference is now a genuine vips oracle (`round_rint_expected.v`).
//! * `hough_line` / `hough_circle` — **GOLDEN-ONLY**: `hough_line`'s distance
//!   binning is now vips-exact (core issue #495 fixed the one-cell offset), but
//!   an INHERENT format/saturation gap remains — the core accumulates into
//!   Gray16 (u16) while vips uses a uint accumulator, so a peak of >65535
//!   collinear votes saturates in the core (no u32 carrier). `hough_circle`
//!   still diverges structurally (a different circle vote model — a single point
//!   yields a core max of 1 vs a vips max of 4). Neither has a full-width vips
//!   oracle, so the references are viprs-generated regression pins compared at
//!   tol 0.
//!
//! If the `libviprs-cli` sibling is not checked out, every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{
    SCALAR_INT_EXACT, SCALAR_S3_REL_EPS, cli_available, cli_fixture, compare_scalar,
    decode_compare, run_viprs, run_viprs_ok,
};

use tempfile::TempDir;

/// EXACT oracle class: bit-exact decode comparison (CLI_CONTRACT.md §5).
const EXACT: f64 = 0.0;

/// A tiny absolute tolerance for the `stats` / `measure` float-matrix `.v`
/// comparisons. The measured max-abs-diff is 0 (the core's f64 accumulation
/// matches vips and casts to the same f32 on this integer input); this small
/// guard only absorbs a possible cross-platform f32 accumulation wobble.
const MATRIX_TOL: f64 = 1e-2;

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
/// Under `$VIPRS_REQUIRE_CLI=1` an absent sibling PANICS in [`cli_available`],
/// so this never returns `true` on the dedicated `cli-differential` job.
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

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// The absolute string path of a fresh temp output.
fn o(name: &str) -> String {
    out_path(name).to_str().unwrap().to_string()
}

// Common committed inputs (tests/fixtures/cli/aritha/).
const AGRAY: &str = "aritha/agray.png";
const AFLOAT: &str = "aritha/afloat.v";

/// Parse every non-empty whitespace-trimmed line of `s` as an `f64`.
fn parse_lines(s: &str) -> Vec<f64> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            l.parse::<f64>()
                .unwrap_or_else(|e| panic!("line {l:?} is not a float: {e}"))
        })
        .collect()
}

/// Compare a multi-line numeric `viprs` stdout against a committed reference
/// file line-for-line, each within a relative epsilon (0 = bit-exact integers).
fn compare_line_vec(stdout: &str, expected_rel: &str, rel_eps: f64) {
    let got = parse_lines(stdout);
    let raw = std::fs::read_to_string(cli_fixture(expected_rel))
        .unwrap_or_else(|e| panic!("cannot read {expected_rel}: {e}"));
    let want = parse_lines(&raw);
    assert_eq!(
        got.len(),
        want.len(),
        "line-count mismatch for {expected_rel}: got {got:?}, want {want:?}"
    );
    for (i, (&g, &w)) in got.iter().zip(&want).enumerate() {
        let denom = w.abs().max(1.0);
        let rel = (g - w).abs() / denom;
        assert!(
            rel <= rel_eps,
            "line {i} mismatch for {expected_rel}: got {g}, want {w} (relative {rel} > {rel_eps})"
        );
    }
}

// ---------------------------------------------------------------------------
// Statistics — S3 scalars.
// ---------------------------------------------------------------------------

#[test]
fn avg_matches_vips() {
    if skip_if_no_cli("avg") {
        return;
    }
    let stdout = run_viprs_ok(&["avg", &fx(AGRAY)]);
    // A rational mean (127.5): compared at the S3 relative epsilon.
    compare_scalar(
        &stdout,
        common::cli::read_scalar_fixture("aritha/avg_expected.txt"),
        SCALAR_S3_REL_EPS,
    );
}

#[test]
fn deviate_matches_vips() {
    if skip_if_no_cli("deviate") {
        return;
    }
    let stdout = run_viprs_ok(&["deviate", &fx(AGRAY)]);
    compare_scalar(
        &stdout,
        common::cli::read_scalar_fixture("aritha/deviate_expected.txt"),
        SCALAR_S3_REL_EPS,
    );
}

#[test]
fn min_matches_vips_int_exact() {
    if skip_if_no_cli("min") {
        return;
    }
    let stdout = run_viprs_ok(&["min", &fx(AGRAY)]);
    // The minimum of an integer image is an exact integer (0).
    compare_scalar(
        &stdout,
        common::cli::read_scalar_fixture("aritha/min_expected.txt"),
        SCALAR_INT_EXACT,
    );
}

#[test]
fn max_matches_vips_int_exact() {
    if skip_if_no_cli("max") {
        return;
    }
    let stdout = run_viprs_ok(&["max", &fx(AGRAY)]);
    compare_scalar(
        &stdout,
        common::cli::read_scalar_fixture("aritha/max_expected.txt"),
        SCALAR_INT_EXACT,
    );
}

#[test]
fn min_x_y_position_matches_vips() {
    if skip_if_no_cli("min_xy") {
        return;
    }
    // vips prints x, then y, then the value (three lines); `minpos` fold.
    let stdout = run_viprs_ok(&["min", &fx(AGRAY), "--x", "--y"]);
    compare_line_vec(&stdout, "aritha/min_xy_expected.txt", SCALAR_INT_EXACT);
}

#[test]
fn max_x_y_position_matches_vips() {
    if skip_if_no_cli("max_xy") {
        return;
    }
    // Discriminating: the max of the horizontal ramp is at x=15 (not 0).
    let stdout = run_viprs_ok(&["max", &fx(AGRAY), "--x", "--y"]);
    compare_line_vec(&stdout, "aritha/max_xy_expected.txt", SCALAR_INT_EXACT);
}

#[test]
fn find_trim_matches_vips() {
    if skip_if_no_cli("find_trim") {
        return;
    }
    // content.png: a black 6×7 block on white at (4,5) → left/top/width/height
    // = 4/5/6/7 (a non-vacuous interior box, not the whole image).
    let stdout = run_viprs_ok(&["find_trim", &fx("aritha/content.png")]);
    compare_line_vec(&stdout, "aritha/find_trim_expected.txt", SCALAR_INT_EXACT);
}

#[test]
fn find_trim_background_flag_matches_vips() {
    if skip_if_no_cli("find_trim_bg") {
        return;
    }
    // content2.png: a white 5×4 block on black; --background 0 makes the white
    // block the content → 3/2/5/4.
    let stdout = run_viprs_ok(&["find_trim", &fx("aritha/content2.png"), "--background", "0"]);
    compare_line_vec(
        &stdout,
        "aritha/find_trim_bg_expected.txt",
        SCALAR_INT_EXACT,
    );
}

// ---------------------------------------------------------------------------
// Statistics — matrix .v (image → image).
// ---------------------------------------------------------------------------

#[test]
fn stats_matches_vips_matrix() {
    if skip_if_no_cli("stats") {
        return;
    }
    let out = o("stats.v");
    run_viprs_ok(&["stats", &fx(AGRAY), &out]);
    // The reference is vips's 6 core columns (cast to float); the 4 position
    // columns vips also emits are a documented core subset gap.
    decode_compare(
        out_path("stats.v").as_path(),
        &cli_fixture("aritha/stats_expected.v"),
        MATRIX_TOL,
    );
}

#[test]
fn measure_matches_vips_matrix() {
    if skip_if_no_cli("measure") {
        return;
    }
    let out = o("measure.v");
    run_viprs_ok(&["measure", &fx(AGRAY), &out, "2", "2"]);
    decode_compare(
        out_path("measure.v").as_path(),
        &cli_fixture("aritha/measure_expected.v"),
        MATRIX_TOL,
    );
}

// ---------------------------------------------------------------------------
// Statistics — S4 two outputs.
// ---------------------------------------------------------------------------

#[test]
fn profile_matches_vips() {
    if skip_if_no_cli("profile") {
        return;
    }
    let cols = o("profile_cols.v");
    let rows = o("profile_rows.v");
    run_viprs_ok(&["profile", &fx("aritha/pzero.png"), &cols, &rows]);
    decode_compare(
        out_path("profile_cols.v").as_path(),
        &cli_fixture("aritha/profile_cols_expected.v"),
        EXACT,
    );
    decode_compare(
        out_path("profile_rows.v").as_path(),
        &cli_fixture("aritha/profile_rows_expected.v"),
        EXACT,
    );
}

#[test]
fn project_matches_vips() {
    if skip_if_no_cli("project") {
        return;
    }
    let cols = o("project_cols.v");
    let rows = o("project_rows.v");
    run_viprs_ok(&["project", &fx(AGRAY), &cols, &rows]);
    decode_compare(
        out_path("project_cols.v").as_path(),
        &cli_fixture("aritha/project_cols_expected.v"),
        EXACT,
    );
    decode_compare(
        out_path("project_rows.v").as_path(),
        &cli_fixture("aritha/project_rows_expected.v"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// Const / linear.
// ---------------------------------------------------------------------------

#[test]
fn linear_float_matches_vips() {
    if skip_if_no_cli("linear_float") {
        return;
    }
    let out = o("linear.v");
    // Scalar (broadcast) 2·in + 10 — non-identity; float out.
    run_viprs_ok(&["linear", &fx(AGRAY), &out, "2", "10"]);
    decode_compare(
        out_path("linear.v").as_path(),
        &cli_fixture("aritha/linear_expected.v"),
        EXACT,
    );
}

#[test]
fn linear_uchar_matches_vips() {
    if skip_if_no_cli("linear_uchar") {
        return;
    }
    let out = o("linear_uchar.png");
    run_viprs_ok(&["linear", &fx(AGRAY), &out, "2", "10", "--uchar"]);
    decode_compare(
        out_path("linear_uchar.png").as_path(),
        &cli_fixture("aritha/linear_uchar_expected.png"),
        EXACT,
    );
}

#[test]
fn remainder_const_matches_vips() {
    if skip_if_no_cli("remainder_const") {
        return;
    }
    let out = o("rem.png");
    run_viprs_ok(&["remainder_const", &fx(AGRAY), &out, "100"]);
    decode_compare(
        out_path("rem.png").as_path(),
        &cli_fixture("aritha/remainder_const_expected.png"),
        EXACT,
    );
}

#[test]
fn math2_const_pow_matches_vips() {
    if skip_if_no_cli("math2_const") {
        return;
    }
    let out = o("pow.v");
    run_viprs_ok(&["math2_const", &fx(AGRAY), &out, "pow", "2"]);
    decode_compare(
        out_path("pow.v").as_path(),
        &cli_fixture("aritha/math2_const_pow_expected.v"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// Unary / rounding (float .v: abs/sign/round exercise the negative/fractional
// domain a uchar input cannot).
// ---------------------------------------------------------------------------

#[test]
fn abs_matches_vips() {
    if skip_if_no_cli("abs") {
        return;
    }
    let out = o("abs.v");
    run_viprs_ok(&["abs", &fx(AFLOAT), &out]);
    decode_compare(
        out_path("abs.v").as_path(),
        &cli_fixture("aritha/abs_expected.v"),
        EXACT,
    );
}

#[test]
fn sign_matches_vips() {
    if skip_if_no_cli("sign") {
        return;
    }
    let out = o("sign.v");
    run_viprs_ok(&["sign", &fx(AFLOAT), &out]);
    decode_compare(
        out_path("sign.v").as_path(),
        &cli_fixture("aritha/sign_expected.v"),
        EXACT,
    );
}

// NOTE: `round rint` is now EXACT against a real vips oracle (core issue #494
// made the core round half-to-even, matching vips's C `rint`) — see the
// `round_rint_matches_vips` test just below. It was formerly a GOLDEN-ONLY viprs
// regression pin (the core's `f64::round`, half away from zero, diverged from
// vips at exact half-integers).

#[test]
fn round_rint_matches_vips() {
    if skip_if_no_cli("round_rint") {
        return;
    }
    // EXACT (tol 0): on the discriminating afloat (which reaches exact
    // half-integers), core issue #494 changed the core to round half TO EVEN
    // (0.5→0, 2.5→2, −2.5→−2), matching vips's C `rint` bit-for-bit — previously
    // the core mapped `f64::round` (half AWAY from zero: 0.5→1, 2.5→3, −2.5→−3),
    // diverging by 1 at every x.5, which forced a GOLDEN-ONLY viprs pin. The
    // reference is now a genuine vips oracle (`round_rint_expected.v`).
    let out = o("round_rint.v");
    run_viprs_ok(&["round", &fx(AFLOAT), &out, "rint"]);
    decode_compare(
        out_path("round_rint.v").as_path(),
        &cli_fixture("aritha/round_rint_expected.v"),
        EXACT,
    );
}

#[test]
fn round_ceil_matches_vips() {
    if skip_if_no_cli("round_ceil") {
        return;
    }
    let out = o("round_ceil.v");
    run_viprs_ok(&["round", &fx(AFLOAT), &out, "ceil"]);
    decode_compare(
        out_path("round_ceil.v").as_path(),
        &cli_fixture("aritha/round_ceil_expected.v"),
        EXACT,
    );
}

#[test]
fn round_floor_matches_vips() {
    if skip_if_no_cli("round_floor") {
        return;
    }
    let out = o("round_floor.v");
    run_viprs_ok(&["round", &fx(AFLOAT), &out, "floor"]);
    decode_compare(
        out_path("round_floor.v").as_path(),
        &cli_fixture("aritha/round_floor_expected.v"),
        EXACT,
    );
}

#[test]
fn clamp_matches_vips() {
    if skip_if_no_cli("clamp") {
        return;
    }
    let out = o("clamp.png");
    run_viprs_ok(&["clamp", &fx(AGRAY), &out, "--min", "50", "--max", "200"]);
    decode_compare(
        out_path("clamp.png").as_path(),
        &cli_fixture("aritha/clamp_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// Hough — GOLDEN-ONLY (viprs regression pin; the core numerics diverge from
// vips — see the module header). tol 0: the committed golden was produced by
// the same deterministic `viprs` binary this suite builds.
// ---------------------------------------------------------------------------

#[test]
fn hough_line_matches_golden_pin() {
    if skip_if_no_cli("hough_line") {
        return;
    }
    let out = o("hough_line.v");
    run_viprs_ok(&["hough_line", &fx("aritha/point.png"), &out]);
    decode_compare(
        out_path("hough_line.v").as_path(),
        &cli_fixture("aritha/hough_line_golden.v"),
        EXACT,
    );
}

#[test]
fn hough_circle_matches_golden_pin() {
    if skip_if_no_cli("hough_circle") {
        return;
    }
    let out = o("hough_circle.v");
    run_viprs_ok(&["hough_circle", &fx("aritha/point.png"), &out, "2", "4"]);
    decode_compare(
        out_path("hough_circle.v").as_path(),
        &cli_fixture("aritha/hough_circle_golden.v"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// Bounds / rejection paths — a bad user input is a typed non-zero exit, NEVER a
// panic/abort (CLI_CONTRACT.md §8).
// ---------------------------------------------------------------------------

#[test]
fn clamp_inverted_bounds_is_rejected() {
    if skip_if_no_cli("clamp_inverted") {
        return;
    }
    // --min > --max would panic the core `clamp` (assert); the CLI must turn it
    // into a clean exit-1, not an abort (exit 134).
    let out = o("clamp_bad.png");
    let res = run_viprs(&["clamp", &fx(AGRAY), &out, "--min", "200", "--max", "50"]);
    assert!(
        !res.status.success(),
        "an inverted --min/--max must be rejected"
    );
    // Exit 1 (op error), never a signal / abort.
    assert_eq!(res.status.code(), Some(1), "expected a clean exit 1");
}

#[test]
fn clamp_nan_bound_is_rejected() {
    if skip_if_no_cli("clamp_nan") {
        return;
    }
    // clap's f64 value_parser accepts "nan". `NaN > hi` is false, so a bare
    // `lo > hi` guard let a NaN bound slip past into the core `assert!(lo <= hi)`
    // and abort with exit 101 (§8 violation). The `!(lo <= hi)` guard rejects it
    // as a clean exit 1. Cover both bounds; NEVER a panic/abort (exit 134).
    for (min_v, max_v) in [("nan", "200"), ("0", "nan")] {
        let out = o("clamp_nan.png");
        let res = run_viprs(&["clamp", &fx(AGRAY), &out, "--min", min_v, "--max", max_v]);
        assert!(
            !res.status.success(),
            "a NaN clamp bound (--min {min_v} --max {max_v}) must be rejected"
        );
        assert_eq!(
            res.status.code(),
            Some(1),
            "expected a clean exit 1 for a NaN bound (--min {min_v} --max {max_v}), \
             not the core assert abort"
        );
    }
}

#[test]
fn linear_per_band_vector_is_rejected() {
    if skip_if_no_cli("linear_vector") {
        return;
    }
    // Multi-element a/b is a per-band linear the core cannot back — a typed
    // exit-1 error, not a silent wrong result.
    let out = o("linear_bad.v");
    let res = run_viprs(&["linear", &fx(AGRAY), &out, "2 3", "1 1"]);
    assert!(
        !res.status.success(),
        "a per-band linear vector must be rejected"
    );
    assert_eq!(res.status.code(), Some(1), "expected a clean exit 1");
}

#[test]
fn math2_const_unsupported_op_is_a_usage_error() {
    if skip_if_no_cli("math2_const_wop") {
        return;
    }
    // Only `pow` is core-backed; `wop` is rejected by clap at parse time (exit 2).
    let out = o("m2_bad.v");
    let res = run_viprs(&["math2_const", &fx(AGRAY), &out, "wop", "2"]);
    assert!(!res.status.success(), "wop is not core-backed");
    assert_eq!(
        res.status.code(),
        Some(2),
        "a bad enum is a usage error (exit 2)"
    );
}
