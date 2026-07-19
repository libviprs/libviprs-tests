//! CLI-DIFFERENTIAL suite — freqfilt (Fourier) family (the Wave-2 Fourier lane,
//! CLI_CONTRACT.md §7, OP_MAP.md freqfilt section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/freqfilt/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the bands/morphology reference cells exactly
//! — including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! **Every freqfilt op is oracle class FOURIER** (CLI_CONTRACT.md §5): the
//! outputs are FFT-derived floats compared as f64 band-pairs from the native
//! `.v` container at a MEASURED absolute epsilon (the core runs each transform
//! in f64 through pure-Rust `rustfft` and stores f32; the vips oracle runs FFTW
//! in double and stores dpcomplex/double — the two agree to the f32
//! quantisation floor, so tol 0 would be a latent false-red on a cross-platform
//! rustfft build). The epsilons are sized above each op's f32-quantisation floor
//! (peak magnitude → f32 ULP): fwfft ~127 → 1e-2; invfft ~3.3e4 → 5e-2; phasecor
//! / roundtrip ~127 → 1e-2. `spectrum` and `freqmult` fold their float result
//! back to a uchar raster: `spectrum` matches bit-for-bit (tol 0); `freqmult`'s
//! float round-trip cast back to uchar rounds ±1 vs vips (BOUNDED-TOL ≤1 LSB, a
//! genuine measured divergence — the same shape as the bands `bandmean` and
//! conversion `gamma` cases).
//!
//! Measured viprs-vs-vips max-abs-diff (release build, vips 8.18.4): fwfft
//! 1.1e-16, invfft 2.8e-14, invfft --real 0, roundtrip 3.8e-6, phasecor 0
//! (float `.v`); spectrum 0, freqmult 1 (uchar PNG) — all comfortably inside the
//! tolerances below.
//!
//! **Carrier note** (CLI_CONTRACT.md §2): the libviprs `.v` decoder accepts only
//! uchar/ushort/float band formats and REJECTS vips `dpcomplex`(10) /
//! `double`(8), so every complex/real vips reference is normalised OFFLINE to an
//! f32 `.v` (complex → 2-band `[re, im]` via `complexget`+`bandjoin`+`cast
//! float`; real → `cast float`). See `tools/gen_cli_expected.sh` and
//! PROVENANCE.md. A single committed complex INPUT cannot feed both sides (the
//! two complex carriers are mutually unreadable), so the complex-input code path
//! is driven by the invfft(fwfft(in)) --real ROUND-TRIP instead.
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

// -- FOURIER tolerances (CLI_CONTRACT.md §5): a small absolute eps per op, sized
//    above its f32-quantisation floor (peak magnitude → f32 ULP), NOT tol 0. See
//    the module header for the measured max-abs-diffs.

/// `fwfft` float `.v` (2-band, peak ~127). Measured 1.1e-16.
const FWFFT_EPS: f64 = 1e-2;
/// `invfft` float `.v` (2-band complex / 1-band real, peak ~3.3e4 — f32 ULP
/// ~4e-3 there). Measured ≤2.8e-14; the 5e-2 eps clears a cross-platform f32
/// boundary flip while a real invfft bug is off by O(1e3).
const INVFFT_EPS: f64 = 5e-2;
/// `phasecor` float `.v` (1-band, peak ~O(256)). Measured 0.
const PHASECOR_EPS: f64 = 1e-2;
/// invfft(fwfft(in)) --real round-trip float `.v` (1-band, recovers the input,
/// peak ~127). Measured 3.8e-6.
const ROUNDTRIP_EPS: f64 = 1e-2;
/// `spectrum` uchar PNG — matches vips bit-for-bit. Measured 0.
const SPECTRUM_TOL: f64 = 0.0;
/// `freqmult` uchar PNG — the float round-trip cast back to uchar rounds ±1 vs
/// vips (BOUNDED-TOL ≤1 LSB). Measured 1.
const FREQMULT_TOL: f64 = 1.0;

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

// Common committed inputs (tests/fixtures/cli/freqfilt/).
const IN: &str = "freqfilt/in.png";
const MASK: &str = "freqfilt/mask.v";
const SHIFTED: &str = "freqfilt/shifted.png";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// fwfft — S1; forward FFT (real in -> complex spectrum, 2-band f32 .v).
// ---------------------------------------------------------------------------

#[test]
fn fwfft_matches_vips_fourier() {
    if skip_if_no_cli("fwfft") {
        return;
    }
    let out = out_path("fwfft.v");
    run_viprs_ok(&["fwfft", &fx(IN), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("freqfilt/fwfft_expected.v"), FWFFT_EPS);
}

// ---------------------------------------------------------------------------
// invfft — S1; inverse FFT (real in -> complex out, 2-band f32 .v). vips casts
// the real input to complex; the core transforms the real band — identical.
// ---------------------------------------------------------------------------

#[test]
fn invfft_real_input_complex_output_matches_vips_fourier() {
    if skip_if_no_cli("invfft") {
        return;
    }
    let out = out_path("invfft.v");
    run_viprs_ok(&["invfft", &fx(IN), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("freqfilt/invfft_expected.v"), INVFFT_EPS);
}

// ---------------------------------------------------------------------------
// invfft --real — S1 flag path; inverse FFT keeping the real part (1-band f32).
// This is the folded `invfft_real` op (vips `invfft --real`, verified).
// ---------------------------------------------------------------------------

#[test]
fn invfft_real_matches_vips_fourier() {
    if skip_if_no_cli("invfft_real") {
        return;
    }
    let out = out_path("invfft_real.v");
    run_viprs_ok(&["invfft", &fx(IN), out.to_str().unwrap(), "--real"]);
    decode_compare(
        &out,
        &cli_fixture("freqfilt/invfft_real_expected.v"),
        INVFFT_EPS,
    );
}

// ---------------------------------------------------------------------------
// invfft round-trip — invfft(fwfft(in)) --real recovers the input AND is the
// ONLY case exercising the core's COMPLEX-input path: `viprs invfft` reads its
// OWN Fourier-stamped 2-band f32 `.v` (is_fourier_complex == true). A single
// committed complex INPUT cannot feed both sides (vips dpcomplex vs libviprs
// f32-pairs are mutually unreadable), so each side chains its own carrier.
// ---------------------------------------------------------------------------

#[test]
fn invfft_roundtrip_complex_input_path_matches_vips_fourier() {
    if skip_if_no_cli("invfft_roundtrip") {
        return;
    }
    // Step 1: viprs fwfft -> a Fourier-stamped 2-band f32 .v (viprs's OWN
    // complex carrier).
    let fourier = out_path("rt_fourier.v");
    run_viprs_ok(&["fwfft", &fx(IN), fourier.to_str().unwrap()]);
    // Step 2: viprs invfft --real reads that complex .v back (complex-input
    // path) and recovers the real image.
    let out = out_path("rt_real.v");
    run_viprs_ok(&[
        "invfft",
        fourier.to_str().unwrap(),
        out.to_str().unwrap(),
        "--real",
    ]);
    decode_compare(
        &out,
        &cli_fixture("freqfilt/roundtrip_expected.v"),
        ROUNDTRIP_EPS,
    );
}

// ---------------------------------------------------------------------------
// freqmult — S2 (2-image); multiply by a low-pass mask in Fourier space, cast
// back to uchar. mask_ideal 0.3 changes the gradient by max-abs 114 (a no-op /
// identity freqmult would FAIL — the bands `bandmean` non-vacuous lesson).
// BOUNDED-TOL ≤1 LSB: the float round-trip cast back to uchar rounds ±1 vs vips.
// ---------------------------------------------------------------------------

#[test]
fn freqmult_matches_vips_bounded_tol() {
    if skip_if_no_cli("freqmult") {
        return;
    }
    let out = out_path("freqmult.png");
    run_viprs_ok(&["freqmult", &fx(IN), &fx(MASK), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("freqfilt/freqmult_expected.png"),
        FREQMULT_TOL,
    );
}

// ---------------------------------------------------------------------------
// spectrum — S1; displayable log-scaled power spectrum (uchar, DC centred).
// Matches vips bit-for-bit (tol 0).
// ---------------------------------------------------------------------------

#[test]
fn spectrum_matches_vips_exact() {
    if skip_if_no_cli("spectrum") {
        return;
    }
    let out = out_path("spectrum.png");
    run_viprs_ok(&["spectrum", &fx(IN), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("freqfilt/spectrum_expected.png"),
        SPECTRUM_TOL,
    );
}

// ---------------------------------------------------------------------------
// phasecor — S2 (2-image); phase correlation of `in` and its (3,2)-shifted copy
// (the peak sits at the translation — a discriminating, non-origin surface).
// ---------------------------------------------------------------------------

#[test]
fn phasecor_matches_vips_fourier() {
    if skip_if_no_cli("phasecor") {
        return;
    }
    let out = out_path("phasecor.v");
    run_viprs_ok(&["phasecor", &fx(IN), &fx(SHIFTED), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("freqfilt/phasecor_expected.v"),
        PHASECOR_EPS,
    );
}

// ---------------------------------------------------------------------------
// Error cases (CLI_CONTRACT.md §8): op failure -> nonzero exit + a viprs-side
// message substring (never a panic/abort, never vips stderr text).
// ---------------------------------------------------------------------------

/// A committed freqfilt-owned image smaller than `in` (8×8 vs 16×16): the
/// self-contained wrong-size input for the dimension-mismatch error cases.
const SMALL: &str = "freqfilt/small.png";

/// Run `viprs` and return (success, combined stderr) — for the error cases.
fn run_viprs_err(args: &[&str]) -> (bool, String) {
    let out = common::cli::run_viprs(args);
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn freqmult_rejects_mismatched_mask_size() {
    if skip_if_no_cli("freqmult_mismatch") {
        return;
    }
    // The mask must match the image size; an 8×8 mask against the 16×16 input is
    // a typed exit-1 error (FreqfiltError::DimensionMismatch), never a panic.
    let out = out_path("freqmult_err.png");
    let (ok, err) = run_viprs_err(&["freqmult", &fx(IN), &fx(SMALL), out.to_str().unwrap()]);
    assert!(!ok, "a mask size mismatch must exit nonzero, got success");
    assert!(
        err.contains("mismatch"),
        "expected a viprs-side dimension-mismatch message, got: {err}"
    );
}

#[test]
fn phasecor_rejects_mismatched_input_sizes() {
    if skip_if_no_cli("phasecor_mismatch") {
        return;
    }
    // `in` (16×16) vs `small` (8×8): different sizes ->
    // FreqfiltError::DimensionMismatch -> exit 1 + a viprs-side message.
    let out = out_path("phasecor_err.v");
    let (ok, err) = run_viprs_err(&["phasecor", &fx(IN), &fx(SMALL), out.to_str().unwrap()]);
    assert!(!ok, "a size mismatch must exit nonzero, got success");
    assert!(
        err.contains("mismatch"),
        "expected a viprs-side dimension-mismatch message, got: {err}"
    );
}

#[test]
fn missing_input_exits_nonzero() {
    if skip_if_no_cli("missing_input") {
        return;
    }
    let out = out_path("missing.v");
    let (ok, err) = run_viprs_err(&["fwfft", "/definitely/not/here.png", out.to_str().unwrap()]);
    assert!(!ok, "a missing input must exit nonzero");
    assert!(
        err.contains("failed to load"),
        "expected a viprs-side load-failure message, got: {err}"
    );
}
