//! CLI-DIFFERENTIAL suite — core family (`add` / `getpoint`, the Wave-2 core
//! lane; CLI_CONTRACT.md §1/§2/§7/§8, OP_MAP.md core section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/core/` and compares its output against the COMMITTED
//! vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER runs vips:
//! references are generated offline by `tools/gen_cli_expected.sh` and
//! committed. This cell copies the morphology / bands reference cells exactly —
//! including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! `add` and `getpoint` are the two `raster_ops.rs` ops the frozen contract
//! hard-codes. `add` is EXACT-AFTER-CAST (uchar+uchar widens to ushort; the
//! output is integer so the §2 save-cast is a no-op and the differential
//! compares at tol 0). `getpoint` is EXACT (integer pixel values compared
//! numerically, never as text). The `add` outputs are 16-bit ushort rasters
//! carried as the native `.v` container, which both vips and the libviprs
//! decoder round-trip losslessly (a 16-bit PNG's encode path differs between the
//! two libraries).
//!
//! Beyond the happy path, the cell pins the §8 error contract: neither op has a
//! `try_*` core twin (both PANIC on bad input), so the handlers pre-validate and
//! must exit 1 with a message — never abort. Those cases assert a nonzero exit
//! plus a viprs-side message substring only (never vips stderr text).
//!
//! If the `libviprs-cli` sibling is not checked out (the default CI `test` job
//! and the Docker gate clone only the core counterpart), every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{
    SCALAR_INT_EXACT, cli_available, cli_fixture, compare_scalar, decode_compare,
    read_scalar_fixture, run_viprs, run_viprs_ok,
};

use tempfile::TempDir;

/// EXACT oracle class: bit-exact decode comparison (CLI_CONTRACT.md §5). `add`
/// is EXACT-AFTER-CAST but its integer output makes the save-cast a no-op, so
/// the differential compares at tolerance 0.
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

// Common committed inputs (tests/fixtures/cli/core/).
const ADD_A: &str = "core/add_a.png";
const ADD_B: &str = "core/add_b.png";
const GRAY_A: &str = "core/gray_a.png";
const GRAY_B: &str = "core/gray_b.png";
const FLOAT_IN: &str = "core/getpoint_float.v";
const FLOAT_ND_IN: &str = "core/getpoint_float_nd.v";
const U16_A: &str = "core/u16_a.v";
const U16_B: &str = "core/u16_b.v";

/// Numeric epsilon for the NON-DYADIC float `getpoint` case. core prints
/// `f64::to_string` of the widened f32 (e.g. `0.10000000149011612`); vips's own
/// getpoint text form for a non-dyadic float is NOT a contracted surface (§9) and
/// can differ by value class / vips build. This documented `1e-6` tol proves the
/// numeric float-parse + eps compare (NOT a dyadic text match) is what carries a
/// float getpoint case (CLI_CONTRACT.md §3; stdout text is §9 out-of-scope).
const FLOAT_TEXT_EPS: f64 = 1e-6;

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// Parse a whitespace-separated vips numeric vector (`getpoint` prints
/// `148 81 147`) as `f64`s, never comparing text (CLI_CONTRACT.md §3).
fn parse_vec(s: &str) -> Vec<f64> {
    s.split_whitespace()
        .map(|t| {
            t.parse::<f64>()
                .unwrap_or_else(|e| panic!("token {t:?} in {s:?} is not a float: {e}"))
        })
        .collect()
}

/// Compare two per-band pixel vectors element-wise within an absolute tolerance
/// (`0.0` = bit-exact), with a matching band count.
fn compare_vec(got: &[f64], expected: &[f64], tol: f64) {
    assert_eq!(
        got.len(),
        expected.len(),
        "band-count mismatch: got {got:?}, expected {expected:?}"
    );
    for (i, (g, e)) in got.iter().zip(expected).enumerate() {
        assert!(
            (g - e).abs() <= tol,
            "band {i} mismatch: got {g}, expected {e} (|Δ| {} > tol {tol})\n  \
             got = {got:?}\n  expected = {expected:?}",
            (g - e).abs(),
        );
    }
}

// ---------------------------------------------------------------------------
// add — S2 (fixed binary). Two DISTINCT RGB inputs whose per-band sums exceed
// 255, so the 8→16-bit widening is exercised (a broken add that stayed 8-bit
// would clip at 255). Output is 16-bit ushort, carried as `.v`.
// ---------------------------------------------------------------------------

#[test]
fn add_rgb_matches_vips_exact() {
    if skip_if_no_cli("add_rgb") {
        return;
    }
    let out = out_path("add_rgb.v");
    run_viprs_ok(&["add", &fx(ADD_A), &fx(ADD_B), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("core/add_rgb_expected.v"), EXACT);
}

#[test]
fn add_gray_matches_vips_exact() {
    if skip_if_no_cli("add_gray") {
        return;
    }
    // Single-band Gray8 + Gray8 -> Gray16 (the 1-band widening path).
    let out = out_path("add_gray.v");
    run_viprs_ok(&["add", &fx(GRAY_A), &fx(GRAY_B), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("core/add_gray_expected.v"), EXACT);
}

// ---------------------------------------------------------------------------
// getpoint — S3 image→stdout-scalar. Numeric compare (never text).
// ---------------------------------------------------------------------------

#[test]
fn getpoint_gray_matches_vips_exact() {
    if skip_if_no_cli("getpoint_gray") {
        return;
    }
    // 1-band pixel -> a single scalar: use compare_scalar (integer-exact).
    let stdout = run_viprs_ok(&["getpoint", &fx(GRAY_A), "5", "2"]);
    let expected = read_scalar_fixture("core/getpoint_gray_expected.txt");
    compare_scalar(&stdout, expected, SCALAR_INT_EXACT);
}

#[test]
fn getpoint_rgb_matches_vips_exact() {
    if skip_if_no_cli("getpoint_rgb") {
        return;
    }
    // 3-band pixel -> a vector: parse both sides and compare element-wise.
    let stdout = run_viprs_ok(&["getpoint", &fx(ADD_A), "5", "2"]);
    let expected_text = std::fs::read_to_string(cli_fixture("core/getpoint_rgb_expected.txt"))
        .expect("read getpoint_rgb reference");
    let got = parse_vec(&stdout);
    assert_eq!(got.len(), 3, "expected a 3-band pixel, got {got:?}");
    compare_vec(&got, &parse_vec(&expected_text), EXACT);
}

#[test]
fn getpoint_float_matches_vips_exact() {
    if skip_if_no_cli("getpoint_float") {
        return;
    }
    // Reads FLOAT samples (a constant [0.5, 1.25, 2.5] .v image): proves getpoint
    // decodes f32 samples, not just uchar. Values are dyadic, so bit-exact.
    let stdout = run_viprs_ok(&["getpoint", &fx(FLOAT_IN), "0", "0"]);
    let expected_text = std::fs::read_to_string(cli_fixture("core/getpoint_float_expected.txt"))
        .expect("read getpoint_float reference");
    let got = parse_vec(&stdout);
    assert_eq!(got.len(), 3, "expected a 3-band float pixel, got {got:?}");
    compare_vec(&got, &parse_vec(&expected_text), EXACT);
}

#[test]
fn getpoint_float_nondyadic_matches_vips_within_eps() {
    if skip_if_no_cli("getpoint_float_nd") {
        return;
    }
    // NON-DYADIC float [0.1, 0.2, 0.3]: none stores exactly in f32. core prints
    // f64::to_string of the widened f32; vips's getpoint text form for a
    // non-dyadic float is NOT a contracted surface (§9). This case DE-RIGS the
    // deliberately-dyadic getpoint_float fixture by proving the numeric
    // float-parse + epsilon compare (NOT a bit-exact dyadic text match) is what
    // carries a float getpoint case (CLI_CONTRACT.md §3). The numeric compare at
    // FLOAT_TEXT_EPS is the real oracle, robust to any text-format difference.
    let stdout = run_viprs_ok(&["getpoint", &fx(FLOAT_ND_IN), "0", "0"]);
    let expected_text = std::fs::read_to_string(cli_fixture("core/getpoint_float_nd_expected.txt"))
        .expect("read getpoint_float_nd reference");
    let got = parse_vec(&stdout);
    assert_eq!(got.len(), 3, "expected a 3-band float pixel, got {got:?}");
    compare_vec(&got, &parse_vec(&expected_text), FLOAT_TEXT_EPS);
}

// ---------------------------------------------------------------------------
// §8 error contract — neither op has a try_* core twin (both PANIC on bad
// input), so the handlers must pre-validate to a typed exit-1 error, NEVER an
// abort (exit 101). Assert a nonzero exit + a viprs-side message substring only.
// ---------------------------------------------------------------------------

#[test]
fn add_rejects_channel_mismatch_without_panicking() {
    if skip_if_no_cli("add_channel_mismatch") {
        return;
    }
    // 3-band RGB + 1-band gray: core's add asserts on the channel-count mismatch;
    // the handler must turn that into exit 1, not an abort.
    //
    // NOTE (documented SUBSET, not a parity claim): vips `add` BAND-BROADCASTS a
    // 1-band operand across a multi-band one (`vips add rgb gray` → per-band
    // rgb+gray, exit 0). Core requires EQUAL band counts and cannot broadcast
    // without a core change, so the CLI keeps this exit-1 rejection as a
    // documented limitation — moving toward vips broadcasting would be a feature,
    // NOT a regression against this test. Band-broadcast parity is a core-side
    // follow-up, not baked into the CLI contract here.
    let out = out_path("add_mismatch.v");
    let res = run_viprs(&["add", &fx(ADD_A), &fx(GRAY_A), out.to_str().unwrap()]);
    assert!(
        !res.status.success(),
        "add of a 3-band and a 1-band image must fail"
    );
    assert_ne!(
        res.status.code(),
        Some(101),
        "add must exit 1 (typed error), never abort (101) on a channel mismatch"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("channel-count mismatch"),
        "expected a channel-mismatch message, got stderr: {stderr}"
    );
}

#[test]
fn add_rejects_float_input_without_panicking() {
    if skip_if_no_cli("add_float") {
        return;
    }
    // Float inputs are unsupported (a later arithmetic batch adds float
    // addition); core's add PANICS on them, so the handler must exit 1.
    let out = out_path("add_float.v");
    let res = run_viprs(&["add", &fx(FLOAT_IN), &fx(FLOAT_IN), out.to_str().unwrap()]);
    assert!(!res.status.success(), "add of float inputs must fail");
    assert_ne!(
        res.status.code(),
        Some(101),
        "add must exit 1 (typed error), never abort (101) on a float input"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("float"),
        "expected a float-unsupported message, got stderr: {stderr}"
    );
}

#[test]
fn add_rejects_16bit_input_without_panicking() {
    if skip_if_no_cli("add_16bit") {
        return;
    }
    // Two 16-bit (ushort) inputs whose per-pixel sum (40000+40000 = 80000)
    // OVERFLOWS ushort. vips `add` promotes ushort→uint and returns 80000, but
    // core keeps the input at 16-bit and SATURATES at 65535 — a SILENTLY WRONG
    // result. The CLI must therefore REJECT 16-bit inputs (exit 1), never emit a
    // wrong 65535 "success" (finding #1: uchar-only fixtures structurally hid
    // this divergence). This asserts the reject, so a future author cannot
    // silently re-open the saturating path.
    let out = out_path("add_u16.v");
    let res = run_viprs(&["add", &fx(U16_A), &fx(U16_B), out.to_str().unwrap()]);
    assert!(
        !res.status.success(),
        "add of two 16-bit inputs must fail (core would saturate at 65535, not \
         promote like vips) — never a wrong success"
    );
    assert_ne!(
        res.status.code(),
        Some(101),
        "add must exit 1 (typed error), never abort (101) on a 16-bit input"
    );
    // Guard against a wrong 65535/80000 success leaking to stdout/the output file.
    assert!(
        !out.exists(),
        "add must not write an output for a rejected 16-bit input"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("16-bit"),
        "expected a 16-bit-unsupported message, got stderr: {stderr}"
    );
}

#[test]
fn getpoint_rejects_out_of_bounds_without_panicking() {
    if skip_if_no_cli("getpoint_oob") {
        return;
    }
    // gray_a is 8×8; (9999, 0) is out of bounds. core's getpoint asserts
    // `x < width`, so the handler must exit 1 rather than abort.
    let res = run_viprs(&["getpoint", &fx(GRAY_A), "9999", "0"]);
    assert!(
        !res.status.success(),
        "getpoint at an out-of-bounds coordinate must fail"
    );
    assert_ne!(
        res.status.code(),
        Some(101),
        "getpoint must exit 1 (typed error), never abort (101) out of bounds"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("out of bounds"),
        "expected an out-of-bounds message, got stderr: {stderr}"
    );
}
