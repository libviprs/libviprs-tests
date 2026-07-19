//! CLI-DIFFERENTIAL suite — matrix family (a per-family Wave-2 lane,
//! CLI_CONTRACT.md §7, OP_MAP.md matrix section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed matrix inputs under
//! `tests/fixtures/cli/matrix/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the bands / morphology reference cells
//! exactly — including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! # A matrix-in / matrix-out family
//!
//! Unlike the image families, the input is a **matrix FILE** (vips text-matrix
//! format), consumed identically by both the vips reference generator and
//! `viprs` (which loads it through the shared `MatFile` loader into a one-band
//! `Interpretation::Matrix` raster). Both commands are oracle class
//! **BOUNDED-TOL**, NOT EXACT: the core computes in f64 but stores results as
//! **f32** (libvips stores double), so the reference is the vips double result
//! **cast to float** (`vips cast … float`) and the compare is f32-vs-f32. The
//! core is a faithful port of libvips `matrixinvert.c` / `invertlut.c`
//! (identical operation order), so the two f64 computations agree and the
//! f32-cast results match to within f32 rounding: the measured max-abs-diff is
//! exactly `0` for `matrixinvert` (both paths) and `5.96e-8` — one f32 ULP at
//! 1.0 — for `invertlut` (its tail extrapolates to 1.0, where the f32 round of
//! vips's double differs from the core's f32 by a single LSB). [`MATRIX_TOL`]
//! (1e-6) sits ~16x above that measured f32 ULP. The 1e-9 f64 tol the OP_MAP
//! note assumed is unreachable with an f32 carrier (see the wave report open
//! question); the honest, measured f32 tol is used here.
//!
//! Coverage is DISCRIMINATING (a no-op / identity op FAILS): `matrixinvert` on a
//! non-identity matrix yields a visibly different inverse (an identity op would
//! diverge wholesale), the 3×3 exercises the **direct cofactor** path (n<4) and
//! the 4×4 the **PLU decomposition** path (n≥4); `invertlut` yields a 256×1 (or
//! 64×1) LUT from a 3×3 input (an identity op would mismatch on dimensions).
//! Bounds rejection (singular / non-square / out-of-range / too-few-columns
//! matrices, and a sub-range `--size`) is exercised as exit-1 / exit-2 error
//! cases, never a panic (CLI_CONTRACT.md §8).
//!
//! If the `libviprs-cli` sibling is not checked out, every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs, run_viprs_ok};

use tempfile::TempDir;

/// BOUNDED-TOL for both matrix ops (CLI_CONTRACT.md §5): an f32-carrier epsilon.
/// The core stores f64 results as f32; the reference is the vips double result
/// cast to float, so the compare is f32-vs-f32. The faithful port makes the two
/// agree to within f32 rounding — the measured max-abs-diff is 0 for
/// `matrixinvert` and 5.96e-8 (one f32 ULP at 1.0) for `invertlut`. `1e-6` sits
/// ~16x above that measured f32 ULP: tight enough to catch a real regression,
/// loose enough for f32 rounding (a value libvips could never reach with a
/// double carrier, but the honest bound for an f32 one — the OP_MAP 1e-9 was
/// written assuming double storage).
const MATRIX_TOL: f64 = 1e-6;

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

/// Write a small vips text-matrix to a fresh temp `.mat` file and return its
/// path. Used ONLY for the bounds-rejection error cases (singular / non-square /
/// out-of-range / too-few-columns), whose inputs need no committed fixture.
fn tmp_mat(name: &str, contents: &str) -> PathBuf {
    let path = out_path(name);
    std::fs::write(&path, contents).expect("write temp .mat");
    path
}

// Common committed inputs (tests/fixtures/cli/matrix/).
const M3: &str = "matrix/m3.mat";
const M4: &str = "matrix/m4.mat";
const LUT: &str = "matrix/lut.mat";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// matrixinvert — S1, direct cofactor path (3×3, n<4).
// ---------------------------------------------------------------------------

#[test]
fn matrixinvert_3x3_direct_matches_vips_bounded_tol() {
    if skip_if_no_cli("matrixinvert3") {
        return;
    }
    let out = out_path("matrixinvert3.v");
    run_viprs_ok(&["matrixinvert", &fx(M3), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("matrix/matrixinvert3_expected.v"),
        MATRIX_TOL,
    );
}

// ---------------------------------------------------------------------------
// matrixinvert — S1, PLU decomposition path (4×4, n≥4). The 3×3 above runs the
// direct cofactor formulas; this 4×4 runs the Numerical-Recipes PLU path with
// scaled partial pivoting — a distinct code path the 3×3 cannot exercise.
// ---------------------------------------------------------------------------

#[test]
fn matrixinvert_4x4_plu_matches_vips_bounded_tol() {
    if skip_if_no_cli("matrixinvert4") {
        return;
    }
    let out = out_path("matrixinvert4.v");
    run_viprs_ok(&["matrixinvert", &fx(M4), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("matrix/matrixinvert4_expected.v"),
        MATRIX_TOL,
    );
}

// ---------------------------------------------------------------------------
// invertlut — S1, default size (256). A 3×3 measured-points matrix (column 0 =
// input level, columns 1/2 = two bands' responses, all in 0..=1) → a 256×1
// 2-band LUT. An identity op would mismatch on dimensions (3×3 vs 256×1).
// ---------------------------------------------------------------------------

#[test]
fn invertlut_default_matches_vips_bounded_tol() {
    if skip_if_no_cli("invertlut") {
        return;
    }
    let out = out_path("invertlut.v");
    run_viprs_ok(&["invertlut", &fx(LUT), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("matrix/invertlut_expected.v"),
        MATRIX_TOL,
    );
}

// ---------------------------------------------------------------------------
// invertlut — S1, explicit --size 64 (the invertlut_size fold). Same input, a
// different LUT resolution: 64×1 instead of 256×1, so the --size flag path is
// genuinely exercised (a wrong size gives a dimension mismatch).
// ---------------------------------------------------------------------------

#[test]
fn invertlut_size64_matches_vips_bounded_tol() {
    if skip_if_no_cli("invertlut_size64") {
        return;
    }
    let out = out_path("invertlut64.v");
    run_viprs_ok(&["invertlut", &fx(LUT), out.to_str().unwrap(), "--size", "64"]);
    decode_compare(
        &out,
        &cli_fixture("matrix/invertlut_size64_expected.v"),
        MATRIX_TOL,
    );
}

// ---------------------------------------------------------------------------
// Bounds rejection (CLI_CONTRACT.md §8): a bad matrix / size is a typed nonzero
// exit with a viprs-side message, NEVER a panic. We assert only the viprs-side
// substring (never vips stderr) per §7.
// ---------------------------------------------------------------------------

#[test]
fn matrixinvert_singular_is_error_not_panic() {
    if skip_if_no_cli("matrixinvert_singular") {
        return;
    }
    // Linearly dependent rows → singular (the direct-path TOO_SMALL threshold).
    let mat = tmp_mat("singular.mat", "2 2\n1 2\n2 4\n");
    let out = out_path("singular_out.v");
    let output = run_viprs(&["matrixinvert", mat.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "a singular matrix must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("singular"),
        "expected a 'singular' message, got: {stderr}"
    );
}

#[test]
fn matrixinvert_non_square_is_error_not_panic() {
    if skip_if_no_cli("matrixinvert_nonsquare") {
        return;
    }
    let mat = tmp_mat("nonsquare.mat", "3 2\n1 2 3\n4 5 6\n");
    let out = out_path("nonsquare_out.v");
    let output = run_viprs(&["matrixinvert", mat.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "a non-square matrix must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("non-square"),
        "expected a 'non-square' message, got: {stderr}"
    );
}

#[test]
fn invertlut_out_of_range_is_error_not_panic() {
    if skip_if_no_cli("invertlut_out_of_range") {
        return;
    }
    // An element outside 0..=1 (1.5) is rejected by the core, matching vips.
    let mat = tmp_mat("oor.mat", "2 2\n0.1 1.5\n0.2 0.4\n");
    let out = out_path("oor_out.v");
    let output = run_viprs(&["invertlut", mat.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "an out-of-range LUT element must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("range"),
        "expected an out-of-range message, got: {stderr}"
    );
}

#[test]
fn invertlut_too_few_columns_is_error_not_panic() {
    if skip_if_no_cli("invertlut_too_few_columns") {
        return;
    }
    // A single column has an input level but no measured response.
    let mat = tmp_mat("narrow.mat", "1 2\n0.1\n0.2\n");
    let out = out_path("narrow_out.v");
    let output = run_viprs(&["invertlut", mat.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(
        !output.status.success(),
        "a single-column matrix must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("column"),
        "expected a too-few-columns message, got: {stderr}"
    );
}

#[test]
fn invertlut_size_below_one_is_usage_error() {
    if skip_if_no_cli("invertlut_size_below_one") {
        return;
    }
    // vips's minimum --size is 1; clap rejects 0 at parse time → usage exit 2.
    let out = out_path("badsize_out.v");
    let output = run_viprs(&["invertlut", &fx(LUT), out.to_str().unwrap(), "--size", "0"]);
    assert!(
        !output.status.success(),
        "a --size below 1 must be a usage error"
    );
}
