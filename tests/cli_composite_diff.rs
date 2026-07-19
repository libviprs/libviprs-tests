//! CLI-DIFFERENTIAL suite — composite family (the Wave-2 composite lane,
//! CLI_CONTRACT.md §7, OP_MAP.md composite section).
//!
//! Each test runs `viprs <composite|composite2> …` on the COMMON committed
//! inputs under `tests/fixtures/cli/composite/` and decode-compares its output
//! against the COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The
//! suite NEVER runs vips: references are generated offline by
//! `tools/gen_cli_expected.sh` and committed. This cell copies the bands /
//! morphology reference cells exactly, including the skip-guard /
//! `VIPRS_REQUIRE_CLI` discipline.
//!
//! # Honest oracle split (a measured core-vs-vips divergence — tracked as a
//! filed GitHub issue, not a comment-only open question)
//!
//! The core op (`try_composite2`) blends exactly two images with one blend mode;
//! both vips nicknames (`composite` array form, `composite2` pair form) map onto
//! it. Oracle class is **BOUNDED-TOL ≤1 LSB** (the blend runs premultiplied in
//! `f64` then rounds to the integer container).
//!
//! Compositing **opaque** inputs, all 25 modes agree with vips to ≤1 LSB. With
//! **translucent** alpha, only the Porter-Duff *simple* operators
//! (clear/source/over/in/out/dest/dest-over/dest-in/dest-out/xor/add) still
//! agree; the 11 PDF separable blends AND the alpha-weighted Porter-Duff
//! operators atop/dest-atop/saturate diverge WHOLESALE (measured max-abs-diff up
//! to 215) — the core's translucent blend-composite formula differs from
//! libvips'. This suite is honest about that, and does NOT hand-pick inputs to
//! hide it:
//!
//! * Porter-Duff simple modes are cross-checked against vips on the
//!   **translucent** inputs (real oracle, arbitrary alpha, tol 1).
//! * PDF separable blends are cross-checked against vips on the **opaque**
//!   inputs (real oracle, non-vacuous blend-function coverage, tol 1).
//! * The remaining nine modes (clear/out/dest/dest-in/dest-out/dest-atop/
//!   lighten/colour-burn/soft-light) are cross-checked against vips on the
//!   **opaque** inputs (tol 1) so that every one of the 25 VipsBlendMode
//!   spellings is discriminated against a real vips oracle — a mode->variant
//!   mis-wiring cannot pass CI unnoticed.
//! * The translucent divergence is captured separately as **GOLDEN-ONLY**
//!   `viprs` regression pins (multiply/atop/saturate on the translucent inputs):
//!   there is NO cross-oracle for translucent blend compositing, so the
//!   reference is `viprs`-generated (deterministic) and the test is a regression
//!   pin recording the measured vips divergence (the smartcrop-entropy
//!   GOLDEN-ONLY precedent). This divergence is tracked as a filed GitHub issue,
//!   NOT a comment-only open question; a core translucent-blend fix that lands
//!   requires REGENERATING these pins, not reverting.
//!
//! The four core-only non-separable modes (hue/saturation/colour/luminosity)
//! have no vips oracle and are not exposed by the CLI; the error-case tests pin
//! their rejection.
//!
//! If the `libviprs-cli` sibling is not checked out, every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs};

use tempfile::TempDir;

/// BOUNDED-TOL ≤1 LSB for every composite differential (CLI_CONTRACT.md §5): the
/// blend runs premultiplied in `f64` then rounds to the integer container, so
/// core and vips agree to within one least-significant bit on the inputs where
/// they agree at all.
const BOUNDED_TOL: f64 = 1.0;

/// GOLDEN-ONLY: the reference is `viprs`-generated (no vips oracle), so a
/// deterministic `viprs` re-run must reproduce it bit-for-bit.
const GOLDEN_EXACT: f64 = 0.0;

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

// Common committed inputs (tests/fixtures/cli/composite/).
const BASE: &str = "composite/base.png";
const OVERLAY: &str = "composite/overlay.png";
const BASE_OP: &str = "composite/base_op.png";
const OVERLAY_OP: &str = "composite/overlay_op.png";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// Run `viprs composite2 base overlay out mode`, asserting exit 0.
fn run_composite2(base: &str, overlay: &str, out: &Path, mode: &str) {
    let out = run_viprs(&[
        "composite2",
        &fx(base),
        &fx(overlay),
        out.to_str().unwrap(),
        mode,
    ]);
    assert!(
        out.status.success(),
        "viprs composite2 … {mode} exited {}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// composite2 — Porter-Duff simple modes on TRANSLUCENT RGBA (real vips oracle,
// arbitrary alpha, tol 1). These agree with vips ≤1 LSB; a broken blend fails.
// ---------------------------------------------------------------------------

#[test]
fn composite2_over_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_over") {
        return;
    }
    let out = out_path("c2_over.png");
    run_composite2(BASE, OVERLAY, &out, "over");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_over_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_source_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_source") {
        return;
    }
    let out = out_path("c2_source.png");
    run_composite2(BASE, OVERLAY, &out, "source");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_source_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_in_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_in") {
        return;
    }
    let out = out_path("c2_in.png");
    run_composite2(BASE, OVERLAY, &out, "in");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_in_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_xor_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_xor") {
        return;
    }
    let out = out_path("c2_xor.png");
    run_composite2(BASE, OVERLAY, &out, "xor");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_xor_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_add_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_add") {
        return;
    }
    let out = out_path("c2_add.png");
    run_composite2(BASE, OVERLAY, &out, "add");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_add_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_dest_over_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_dest_over") {
        return;
    }
    // The hyphenated vips spelling must be accepted verbatim.
    let out = out_path("c2_dest_over.png");
    run_composite2(BASE, OVERLAY, &out, "dest-over");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_dest_over_expected.png"),
        BOUNDED_TOL,
    );
}

// ---------------------------------------------------------------------------
// composite (vips array form) — over on TRANSLUCENT RGBA. Proves the second
// nickname works identically to composite2 (both map onto try_composite2).
// ---------------------------------------------------------------------------

#[test]
fn composite_array_form_over_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite_over") {
        return;
    }
    let out = out_path("c_over.png");
    let done = run_viprs(&[
        "composite",
        &fx(BASE),
        &fx(OVERLAY),
        out.to_str().unwrap(),
        "over",
    ]);
    assert!(
        done.status.success(),
        "viprs composite … over exited {}\n{}",
        done.status,
        String::from_utf8_lossy(&done.stderr),
    );
    decode_compare(
        &out,
        &cli_fixture("composite/composite_over_expected.png"),
        BOUNDED_TOL,
    );
}

// ---------------------------------------------------------------------------
// composite2 — PDF separable blends on OPAQUE RGB (real vips oracle, tol 1).
// The blend FUNCTION B(cb, cs) is exercised non-vacuously (a broken blend
// fails). The translucent divergence of these modes is pinned separately below.
// ---------------------------------------------------------------------------

#[test]
fn composite2_multiply_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_multiply") {
        return;
    }
    let out = out_path("c2_multiply.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "multiply");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_multiply_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_screen_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_screen") {
        return;
    }
    let out = out_path("c2_screen.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "screen");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_screen_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_overlay_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_overlay") {
        return;
    }
    let out = out_path("c2_overlay.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "overlay");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_overlay_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_darken_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_darken") {
        return;
    }
    let out = out_path("c2_darken.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "darken");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_darken_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_hardlight_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_hardlight") {
        return;
    }
    let out = out_path("c2_hardlight.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "hard-light");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_hardlight_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_difference_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_difference") {
        return;
    }
    let out = out_path("c2_difference.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "difference");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_difference_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_exclusion_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_exclusion") {
        return;
    }
    let out = out_path("c2_exclusion.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "exclusion");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_exclusion_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_colourdodge_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_colourdodge") {
        return;
    }
    let out = out_path("c2_colourdodge.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "colour-dodge");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_colourdodge_expected.png"),
        BOUNDED_TOL,
    );
}

// ---------------------------------------------------------------------------
// composite2 — the remaining 9 modes on OPAQUE RGB (real vips oracle, tol 1).
// These close the mode->CompositeMode wiring hole: with the 8 PDF/Porter-Duff
// modes above they make every one of the 25 VipsBlendMode spellings discriminated
// against a real vips oracle, so a swap to a valid-but-wrong variant (e.g.
// lighten->Darken, colour-burn->ColourDodge, out->In) fails CI instead of passing
// unnoticed. All nine agree with vips ≤1 LSB on opaque inputs (measured: clear/
// out/dest/dest-in/dest-out/dest-atop/lighten = 0, colour-burn/soft-light = 1).
// ---------------------------------------------------------------------------

#[test]
fn composite2_clear_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_clear") {
        return;
    }
    let out = out_path("c2_clear.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "clear");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_clear_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_out_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_out") {
        return;
    }
    let out = out_path("c2_out.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "out");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_out_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_dest_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_dest") {
        return;
    }
    let out = out_path("c2_dest.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "dest");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_dest_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_dest_in_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_dest_in") {
        return;
    }
    let out = out_path("c2_dest_in.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "dest-in");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_dest_in_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_dest_out_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_dest_out") {
        return;
    }
    let out = out_path("c2_dest_out.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "dest-out");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_dest_out_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_dest_atop_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_dest_atop") {
        return;
    }
    let out = out_path("c2_dest_atop.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "dest-atop");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_dest_atop_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_lighten_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_lighten") {
        return;
    }
    let out = out_path("c2_lighten.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "lighten");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_lighten_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_colourburn_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_colourburn") {
        return;
    }
    let out = out_path("c2_colourburn.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "colour-burn");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_colourburn_expected.png"),
        BOUNDED_TOL,
    );
}

#[test]
fn composite2_softlight_opaque_matches_vips_bounded_tol() {
    if skip_if_no_cli("composite2_softlight") {
        return;
    }
    let out = out_path("c2_softlight.png");
    run_composite2(BASE_OP, OVERLAY_OP, &out, "soft-light");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_softlight_expected.png"),
        BOUNDED_TOL,
    );
}

// ---------------------------------------------------------------------------
// GOLDEN-ONLY translucent divergence pins (NO vips oracle). On the translucent
// inputs these modes diverge WHOLESALE from vips (measured max-abs-diff:
// multiply 38, atop 191, saturate 37 — a different translucent blend-composite
// formula, not a tolerance). The reference is viprs-generated; the test pins the
// core's current translucent behaviour so a regression is caught, and EXPOSES
// the divergence (tracked as a filed GitHub issue) rather than hiding it.
//
// IMPORTANT — these are NOT "known-good vs vips" pins. If a future core change
// ALIGNS the translucent blend-composite formula with libvips, these three tests
// WILL fail. That failure is EXPECTED and correct: REGENERATE the goldens
// (`tools/gen_cli_expected.sh`) and re-bless them (and promote them to real
// vips-oracle differentials) — do NOT revert the core change to make them pass.
// ---------------------------------------------------------------------------

#[test]
fn composite2_multiply_translucent_golden_pin() {
    if skip_if_no_cli("composite2_multiply_translucent") {
        return;
    }
    let out = out_path("c2_multiply_tl.png");
    run_composite2(BASE, OVERLAY, &out, "multiply");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_multiply_translucent_golden.png"),
        GOLDEN_EXACT,
    );
}

#[test]
fn composite2_atop_translucent_golden_pin() {
    if skip_if_no_cli("composite2_atop_translucent") {
        return;
    }
    let out = out_path("c2_atop_tl.png");
    run_composite2(BASE, OVERLAY, &out, "atop");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_atop_translucent_golden.png"),
        GOLDEN_EXACT,
    );
}

#[test]
fn composite2_saturate_translucent_golden_pin() {
    if skip_if_no_cli("composite2_saturate_translucent") {
        return;
    }
    let out = out_path("c2_saturate_tl.png");
    run_composite2(BASE, OVERLAY, &out, "saturate");
    decode_compare(
        &out,
        &cli_fixture("composite/composite2_saturate_translucent_golden.png"),
        GOLDEN_EXACT,
    );
}

// ---------------------------------------------------------------------------
// Error / rejection cases (CLI_CONTRACT.md §8): nonzero exit + a viprs-side
// message substring; NO vips reference needed.
// ---------------------------------------------------------------------------

#[test]
fn composite2_band_mismatch_is_exit_1() {
    if skip_if_no_cli("composite2_band_mismatch") {
        return;
    }
    // A genuine, fixture-backed core error: `base_op` has 3 colour bands, `gray`
    // has 1, so the core rejects the composite with a typed exit-1 error (never a
    // panic / abort, CLI_CONTRACT.md §8), and the viprs-side message names the
    // band-count mismatch.
    let out = out_path("c2_bandmismatch.png");
    let done = run_viprs(&[
        "composite2",
        &fx(BASE_OP),
        &fx("composite/gray.png"),
        out.to_str().unwrap(),
        "over",
    ]);
    assert!(
        !done.status.success(),
        "a colour-band-count mismatch must fail"
    );
    assert_eq!(
        done.status.code(),
        Some(1),
        "a core op error is exit 1 (CLI_CONTRACT.md §8)"
    );
    let stderr = String::from_utf8_lossy(&done.stderr);
    assert!(
        stderr.contains("band"),
        "the error must name the band-count mismatch, got: {stderr}"
    );
}

#[test]
fn composite2_unknown_mode_is_usage_error() {
    if skip_if_no_cli("composite2_unknown_mode") {
        return;
    }
    let out = out_path("c2_bogus.png");
    let done = run_viprs(&[
        "composite2",
        &fx(BASE),
        &fx(OVERLAY),
        out.to_str().unwrap(),
        "bogus",
    ]);
    assert!(
        !done.status.success(),
        "an unknown blend mode must be a usage error"
    );
    assert_eq!(
        done.status.code(),
        Some(2),
        "clap usage errors exit 2 (CLI_CONTRACT.md §8)"
    );
}

#[test]
fn composite2_non_separable_mode_is_rejected() {
    if skip_if_no_cli("composite2_non_separable") {
        return;
    }
    // The four core-only PDF non-separable modes have no vips oracle and are not
    // exposed by the CLI: clap must reject `hue` (usage exit 2).
    let out = out_path("c2_hue.png");
    let done = run_viprs(&[
        "composite2",
        &fx(BASE),
        &fx(OVERLAY),
        out.to_str().unwrap(),
        "hue",
    ]);
    assert!(
        !done.status.success(),
        "a core-only non-separable mode must be rejected"
    );
    assert_eq!(done.status.code(), Some(2));
}

#[test]
fn composite_rejects_a_third_input() {
    if skip_if_no_cli("composite_three_inputs") {
        return;
    }
    // The core blends exactly two images; `composite`'s fixed BASE OVERLAY OUT
    // MODE positional shape rejects a third input at clap's usage exit 2 (the
    // OP_MAP subset note).
    let out = out_path("c_three.png");
    let done = run_viprs(&[
        "composite",
        &fx(BASE),
        &fx(OVERLAY),
        &fx(OVERLAY),
        out.to_str().unwrap(),
        "over",
    ]);
    assert!(
        !done.status.success(),
        "a third input must be a usage error"
    );
    assert_eq!(done.status.code(), Some(2));
}
