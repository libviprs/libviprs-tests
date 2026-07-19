//! CLI-DIFFERENTIAL suite — mosaicing family (Wave-2 mosaicing lane,
//! CLI_CONTRACT.md §7, OP_MAP.md mosaicing section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/mosaicing/` and decode-compares its output against the
//! COMMITTED reference at the op's §5 tolerance. The suite NEVER runs vips:
//! references are generated offline by `tools/gen_cli_expected.sh` and
//! committed. This cell copies the bands / morphology reference cells exactly
//! (skip-guard / `VIPRS_REQUIRE_CLI` discipline).
//!
//! `merge` and `mosaic` are oracle class **EXACT** (decode comparison at
//! tolerance 0). They are the family whose integer feather ramp (`merge`) and
//! **discrete tie-point search** (`mosaic`) must agree with vips bit-for-bit or
//! the whole output diverges — so these tests double as the pin on that
//! agreement (empirically verified max-abs-diff 0). `globalbalance` is
//! **GOLDEN-ONLY**: the core reads the `mosaic-join-tree` metadata blob only
//! viprs merge/mosaic outputs carry, whereas vips's globalbalance reads its own
//! filename-based image history — the two channels are mutually unreadable, so
//! there is NO vips cross-oracle. Its reference is a committed viprs
//! `merge → globalbalance` pipeline pin (tol 0 against a deterministic run).
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

/// EXACT oracle class: bit-exact decode comparison (CLI_CONTRACT.md §5). Applies
/// to `merge` and `mosaic` (integer feather / discrete search, no rounding gap).
const EXACT: f64 = 0.0;

/// GOLDEN-ONLY regression pin: `globalbalance` has no vips cross-oracle, so its
/// reference is a committed deterministic viprs run and the compare is bit-exact
/// (tol 0) — a change in the balance output is a real regression, not drift.
const GOLDEN: f64 = 0.0;

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

// Common committed inputs (tests/fixtures/cli/mosaicing/).
const MERGE_REF: &str = "mosaicing/merge_ref.png";
const MERGE_SEC: &str = "mosaicing/merge_sec.png";
const MERGE_RGB: &str = "mosaicing/merge_rgb.png";
const MOSAIC_H_REF: &str = "mosaicing/mosaic_h_ref.png";
const MOSAIC_H_SEC: &str = "mosaicing/mosaic_h_sec.png";
const MOSAIC_V_REF: &str = "mosaicing/mosaic_v_ref.png";
const MOSAIC_V_SEC: &str = "mosaicing/mosaic_v_sec.png";
const BALANCE_INPUT: &str = "mosaicing/balance_input.v";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// merge — S2, feathered raised-cosine blend. EXACT (integer truncating blend).
// A negative DX/DY is the natural left-right / top-bottom join displacement;
// `viprs` accepts it directly (clap allow_negative_numbers) where vips's CLI
// needs a `--` separator, but both compute the SAME displacement.
// ---------------------------------------------------------------------------

#[test]
fn merge_horizontal_matches_vips_exact() {
    if skip_if_no_cli("merge_horizontal") {
        return;
    }
    let out = out_path("merge_h.png");
    // horizontal, dx -28: sec placed at x=28 over a 40-wide ref → 68x32 union.
    run_viprs_ok(&[
        "merge",
        &fx(MERGE_REF),
        &fx(MERGE_SEC),
        out.to_str().unwrap(),
        "horizontal",
        "-28",
        "0",
    ]);
    decode_compare(&out, &cli_fixture("mosaicing/merge_h_expected.png"), EXACT);
}

#[test]
fn merge_vertical_matches_vips_exact() {
    if skip_if_no_cli("merge_vertical") {
        return;
    }
    let out = out_path("merge_v.png");
    // vertical, dy -22: the tb blend path (a distinct code path from lr).
    run_viprs_ok(&[
        "merge",
        &fx(MERGE_REF),
        &fx(MERGE_SEC),
        out.to_str().unwrap(),
        "vertical",
        "0",
        "-22",
    ]);
    decode_compare(&out, &cli_fixture("mosaicing/merge_v_expected.png"), EXACT);
}

// ---------------------------------------------------------------------------
// mosaic — S2, discrete tie-point search then merge. EXACT. This is the
// family's headline pin: the 3-strip contrast search + normalised-correlation
// refinement must recover the SAME integer offset vips does, or the output
// diverges wholesale. Horizontal (lr) and vertical (tb) exercise both search
// paths and both direction-enum variants.
// ---------------------------------------------------------------------------

#[test]
fn mosaic_horizontal_matches_vips_exact() {
    if skip_if_no_cli("mosaic_horizontal") {
        return;
    }
    let out = out_path("mosaic_h.png");
    // Tie point = the true correspondence sec(40,75) == ref(50,75); the full
    // discrete search still runs and must agree with vips bit-for-bit.
    run_viprs_ok(&[
        "mosaic",
        &fx(MOSAIC_H_REF),
        &fx(MOSAIC_H_SEC),
        out.to_str().unwrap(),
        "horizontal",
        "50",
        "75",
        "40",
        "75",
    ]);
    decode_compare(&out, &cli_fixture("mosaicing/mosaic_h_expected.png"), EXACT);
}

#[test]
fn mosaic_vertical_matches_vips_exact() {
    if skip_if_no_cli("mosaic_vertical") {
        return;
    }
    let out = out_path("mosaic_v.png");
    // sec(75,40) == ref(75,50); the tb search path.
    run_viprs_ok(&[
        "mosaic",
        &fx(MOSAIC_V_REF),
        &fx(MOSAIC_V_SEC),
        out.to_str().unwrap(),
        "vertical",
        "75",
        "50",
        "75",
        "40",
    ]);
    decode_compare(&out, &cli_fixture("mosaicing/mosaic_v_expected.png"), EXACT);
}

// ---------------------------------------------------------------------------
// globalbalance — S1, GOLDEN-ONLY. The input carries the join-tree blob only
// viprs merge writes (and it survives the `.v` JSON trailer), so the CLI reads
// it back and rebalances. NO vips oracle; the reference is a deterministic viprs
// run. Output is always float → the native `.v` container.
// ---------------------------------------------------------------------------

#[test]
fn globalbalance_matches_golden_pin() {
    if skip_if_no_cli("globalbalance") {
        return;
    }
    let out = out_path("balance.v");
    run_viprs_ok(&["globalbalance", &fx(BALANCE_INPUT), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("mosaicing/balance_expected.v"), GOLDEN);
}

// ---------------------------------------------------------------------------
// Error paths (CLI_CONTRACT.md §8): nonzero exit + a viprs-side message
// substring, never a vips stderr text compare, never a panic/abort.
// ---------------------------------------------------------------------------

#[test]
fn merge_rejects_format_mismatch() {
    if skip_if_no_cli("merge_format_mismatch") {
        return;
    }
    let out = out_path("merge_mismatch.png");
    // A Gray8 REF and an sRGB (3-band) SEC cannot merge: the core requires one
    // shared pixel format. Exit 1, a "pixel formats differ" message, no panic.
    let res = run_viprs(&[
        "merge",
        &fx(MERGE_REF),
        &fx(MERGE_RGB),
        out.to_str().unwrap(),
        "horizontal",
        "-28",
        "0",
    ]);
    assert!(
        !res.status.success(),
        "merge of mismatched formats must fail (exit nonzero)"
    );
    assert_eq!(res.status.code(), Some(1), "op failure is exit 1 (§8)");
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("pixel formats differ"),
        "expected a format-mismatch message, got stderr: {stderr}"
    );
}

#[test]
fn globalbalance_rejects_input_without_join_metadata() {
    if skip_if_no_cli("globalbalance_no_metadata") {
        return;
    }
    let out = out_path("balance_bad.v");
    // A plain PNG was not built by merge/mosaic, so it carries no join-tree blob.
    // Exit 1, a "no mosaic metadata" message, never a panic.
    let res = run_viprs(&["globalbalance", &fx(MERGE_REF), out.to_str().unwrap()]);
    assert!(
        !res.status.success(),
        "globalbalance on an input without mosaic metadata must fail"
    );
    assert_eq!(res.status.code(), Some(1), "op failure is exit 1 (§8)");
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("no mosaic metadata"),
        "expected a no-mosaic-metadata message, got stderr: {stderr}"
    );
}
