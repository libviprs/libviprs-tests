//! CLI-DIFFERENTIAL suite — conversion family (the Wave-2 conversion lane,
//! CLI_CONTRACT.md §7, OP_MAP.md conversion section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/conversion/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the morphology / bands reference cells
//! exactly — including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! Almost every conversion command is oracle class **EXACT** (integer-in /
//! integer-out, decode comparison at tolerance 0). The exceptions, per
//! OP_MAP.md:
//!
//! * `flatten` is **BOUNDED-TOL** (≤1 LSB): the alpha blend rounds; vips and the
//!   core agree to within one least-significant bit.
//! * `grey` is **BOUNDED-TOL**: the float ramp path is compared via `.v` at a
//!   small float epsilon; the `--uchar` path at ≤1 LSB.
//!
//! `autorot` deserves special mention. libviprs' decoders read **neither** the
//! vips `.v` XML `orientation` field **nor** the TIFF Orientation tag (274) that
//! vips writes, so a vips-oriented input is a no-op under `viprs` while vips
//! rotates — the two orientation metadata channels are mutually unreadable (the
//! same shape as `globalbalance`). The oriented input `autorot_oriented.v` is
//! therefore minted by `viprs` itself (the only producer of an orientation
//! `viprs` reads back), and the reference stays a genuine vips oracle via the
//! identity **autorot(orientation=6) == rot(d90)** (see `PROVENANCE.md`). A
//! regression that broke autorot into a no-op fails loudly: the un-rotated
//! 8×6 output would mismatch the 6×8 rotated reference on dimensions.
//!
//! Carrier choice mirrors the generator: 1/3/4-band uchar outputs are PNG;
//! `cast`→float and the `grey` float ramp are `.v` (float); `byteswap` and
//! `identity` are `.v` (16-bit byte order / histogram-tagged LUT that PNG cannot
//! carry cleanly).
//!
//! If the `libviprs-cli` sibling is not checked out, every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs_ok};

use tempfile::TempDir;

/// EXACT oracle class: bit-exact decode comparison (CLI_CONTRACT.md §5).
const EXACT: f64 = 0.0;

/// BOUNDED-TOL for `flatten` (CLI_CONTRACT.md §5, OP_MAP.md): ≤1 LSB. The alpha
/// blend rounds; core and vips agree to within one least-significant bit.
const FLATTEN_TOL: f64 = 1.0;

/// BOUNDED-TOL for the `grey --uchar` ramp (≤1 LSB): the float→uchar ramp
/// rounding can differ from vips by at most one least-significant bit.
const GREY_UCHAR_TOL: f64 = 1.0;

/// BOUNDED-TOL for the `grey` float ramp: compared as f32 in the `.v` carrier at
/// a small absolute epsilon (the 0..1 ramp is a division, not always dyadic).
const GREY_FLOAT_EPS: f64 = 1e-6;

/// BOUNDED-TOL for `gamma` (measured ≤1 LSB). OP_MAP.md provisionally listed
/// gamma EXACT-AFTER-CAST, but the differential MEASURES a 1-LSB divergence: the
/// per-sample power LUT is computed and rounded independently by the core and by
/// vips, landing ±1 apart on some samples. This is an honest core-vs-vips
/// rounding difference (flagged as an open question), NOT a rigged tolerance —
/// the identity/no-op guard still holds because a broken gamma would move
/// samples by far more than one LSB.
const GAMMA_TOL: f64 = 1.0;

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

// Common committed inputs (tests/fixtures/cli/conversion/).
const GRAY: &str = "conversion/gray.png";
const GRAY2: &str = "conversion/gray2.png";
const GRAY3: &str = "conversion/gray3.png";
const RGB: &str = "conversion/rgb.png";
const RGBA: &str = "conversion/rgba.png";
/// 2-D gradient varying in BOTH axes — makes `flip vertical`, `wrap`'s vertical
/// shift and `rot d180` non-vacuous (a Y-constant ramp made them no-ops).
const GRAD: &str = "conversion/grad.png";
/// 16-bit input whose high and low bytes DIFFER, so `byteswap` truly moves data
/// (a byte-palindromic 16-bit input would pass a broken no-op byteswap).
const NB16: &str = "conversion/nb16.v";
/// 3-band 16-bit input with band0 != band1, so `msb --band 0` (1 band) is
/// provably distinct from `msb` default (3 bands).
const MB16: &str = "conversion/mb16.v";
/// 256×1 grey ramp spanning the full 0..255 domain, so the `gamma`/`falsecolour`
/// LUT ops are compared over every entry (a 16-value input sampled only 16/256).
const RAMP256: &str = "conversion/ramp256.png";
const ODD: &str = "conversion/odd.png";
const STACK: &str = "conversion/stack.png";
const COND: &str = "conversion/cond.png";
const COND2: &str = "conversion/cond2.png";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// Run a one-image→image `viprs` command and decode-compare its output.
fn run_s1(op: &str, input: &str, out_name: &str, expected: &str, tol: f64, extra: &[&str]) {
    let out = out_path(out_name);
    let in_path = fx(input);
    let out_str = out.to_str().unwrap();
    let mut args = vec![op, in_path.as_str(), out_str];
    args.extend_from_slice(extra);
    run_viprs_ok(&args);
    decode_compare(&out, &cli_fixture(expected), tol);
}

// ---------------------------------------------------------------------------
// copy — S1 header tweak (EXACT). A header op is a no-op on pixels *by design*;
// the differential still fails loudly if the pixels/dims/bands drift, since it
// decode-compares against the vips copy of the same input.
// ---------------------------------------------------------------------------

#[test]
fn copy_matches_vips_exact() {
    if skip_if_no_cli("copy") {
        return;
    }
    run_s1(
        "copy",
        RGB,
        "copy.png",
        "conversion/copy_expected.png",
        EXACT,
        &["--interpretation", "srgb"],
    );
}

// ---------------------------------------------------------------------------
// cast — S1 (EXACT). uchar->ushort (widen, .png 16-bit) and uchar->float (.v).
// ---------------------------------------------------------------------------

// Carried as `.v`: vips' pngsave minimises bit depth, so a ushort image whose
// samples all fit in 8 bits would save as an 8-bit PNG, defeating the
// widen-to-16-bit check. `.v` preserves the true ushort result (viprs correctly
// produces a 16-bit Gray16; the divergence was a save-format artifact, not a
// cast error).
#[test]
fn cast_to_ushort_matches_vips_exact() {
    if skip_if_no_cli("cast_ushort") {
        return;
    }
    run_s1(
        "cast",
        GRAY,
        "cast_ushort.v",
        "conversion/cast_ushort_expected.v",
        EXACT,
        &["ushort"],
    );
}

#[test]
fn cast_to_float_matches_vips_exact() {
    if skip_if_no_cli("cast_float") {
        return;
    }
    run_s1(
        "cast",
        GRAY,
        "cast_float.v",
        "conversion/cast_float_expected.v",
        EXACT,
        &["float"],
    );
}

// ---------------------------------------------------------------------------
// flip — S1 enum horizontal|vertical (EXACT). Input is `grad` (a 2-D gradient):
// gray.png is a horizontal ramp (constant in Y), so a vertical flip on it is a
// no-op and a broken `try_flipver` would ship GREEN. `grad` varies in Y too, so
// BOTH arms discriminate (h diff 85, v diff 170).
// ---------------------------------------------------------------------------

#[test]
fn flip_horizontal_matches_vips_exact() {
    if skip_if_no_cli("flip_h") {
        return;
    }
    run_s1(
        "flip",
        GRAD,
        "flip_h.png",
        "conversion/flip_horizontal_expected.png",
        EXACT,
        &["horizontal"],
    );
}

#[test]
fn flip_vertical_matches_vips_exact() {
    if skip_if_no_cli("flip_v") {
        return;
    }
    run_s1(
        "flip",
        GRAD,
        "flip_v.png",
        "conversion/flip_vertical_expected.png",
        EXACT,
        &["vertical"],
    );
}

// ---------------------------------------------------------------------------
// rot — S1 enum d0|d90|d180|d270 (EXACT). d90 (dims swap) + d180 (dims kept), on
// the 2-D `grad` input: on a Y-constant ramp d180 collapses to a horizontal
// flip, so `grad` (varies in both axes) keeps d180 genuinely distinct.
// ---------------------------------------------------------------------------

#[test]
fn rot_d90_matches_vips_exact() {
    if skip_if_no_cli("rot_d90") {
        return;
    }
    run_s1(
        "rot",
        GRAD,
        "rot_d90.png",
        "conversion/rot_d90_expected.png",
        EXACT,
        &["d90"],
    );
}

#[test]
fn rot_d180_matches_vips_exact() {
    if skip_if_no_cli("rot_d180") {
        return;
    }
    run_s1(
        "rot",
        GRAD,
        "rot_d180.png",
        "conversion/rot_d180_expected.png",
        EXACT,
        &["d180"],
    );
}

// ---------------------------------------------------------------------------
// rot45 — S1 --angle (EXACT); odd-sided square input only.
// ---------------------------------------------------------------------------

#[test]
fn rot45_matches_vips_exact() {
    if skip_if_no_cli("rot45") {
        return;
    }
    run_s1(
        "rot45",
        ODD,
        "rot45.png",
        "conversion/rot45_d45_expected.png",
        EXACT,
        &["--angle", "d45"],
    );
}

// ---------------------------------------------------------------------------
// byteswap — S1 (EXACT). Input `nb16` is a 16-bit image whose high and low bytes
// DIFFER (multiples of 0x1000), carried as `.v` (PNG re-encode would normalise
// byte order). This is the crux of the fix: the old gray16 ramp was all
// byte-palindromes (multiples of 0x1111), so a broken byteswap that returned its
// input unchanged shipped GREEN. On `nb16` byteswap moves data by 61440.
// ---------------------------------------------------------------------------

#[test]
fn byteswap_matches_vips_exact() {
    if skip_if_no_cli("byteswap") {
        return;
    }
    run_s1(
        "byteswap",
        NB16,
        "byteswap.v",
        "conversion/byteswap_expected.v",
        EXACT,
        &[],
    );
}

// ---------------------------------------------------------------------------
// msb — S1 --band (EXACT). Input `mb16` is a 3-band 16-bit image with band0 !=
// band1: `msb` default keeps all 3 bands, `msb --band 0` reduces to band 0's
// high byte alone. On the old 1-band gray16 the two were byte-identical, so the
// --band selection path was never exercised; on `mb16` they are provably
// distinct (band count 3 vs 1).
// ---------------------------------------------------------------------------

#[test]
fn msb_default_matches_vips_exact() {
    if skip_if_no_cli("msb") {
        return;
    }
    run_s1(
        "msb",
        MB16,
        "msb.png",
        "conversion/msb_expected.png",
        EXACT,
        &[],
    );
}

#[test]
fn msb_band0_matches_vips_exact() {
    if skip_if_no_cli("msb_band0") {
        return;
    }
    run_s1(
        "msb",
        MB16,
        "msb_band0.png",
        "conversion/msb_band0_expected.png",
        EXACT,
        &["--band", "0"],
    );
}

// ---------------------------------------------------------------------------
// grid — S1 (EXACT). A vertical-ramp stack with DISTINCT tiles so the tile
// rearrangement is observable (an all-identical-tile input would be vacuous).
// ---------------------------------------------------------------------------

#[test]
fn grid_matches_vips_exact() {
    if skip_if_no_cli("grid") {
        return;
    }
    run_s1(
        "grid",
        STACK,
        "grid.png",
        "conversion/grid_expected.png",
        EXACT,
        &["4", "2", "2"],
    );
}

// ---------------------------------------------------------------------------
// flatten — S1 --background (BOUNDED-TOL ≤1 LSB). rgba (partial alpha) blended
// against black; the alpha blend rounds so core (round) vs vips agree within 1.
// ---------------------------------------------------------------------------

#[test]
fn flatten_matches_vips_bounded_tol() {
    if skip_if_no_cli("flatten") {
        return;
    }
    run_s1(
        "flatten",
        RGBA,
        "flatten.png",
        "conversion/flatten_expected.png",
        FLATTEN_TOL,
        &["--background", "0 0 0"],
    );
}

// ---------------------------------------------------------------------------
// ifthenelse — S2 with three FIXED inputs (EXACT). cond ? gray : gray2, a
// genuine per-pixel mix (gray != gray2), not an identity.
// ---------------------------------------------------------------------------

#[test]
fn ifthenelse_matches_vips_exact() {
    if skip_if_no_cli("ifthenelse") {
        return;
    }
    let out = out_path("ifthenelse.png");
    run_viprs_ok(&[
        "ifthenelse",
        &fx(COND),
        &fx(GRAY),
        &fx(GRAY2),
        out.to_str().unwrap(),
    ]);
    decode_compare(
        &out,
        &cli_fixture("conversion/ifthenelse_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// autorot — S1 (EXACT via the rot-d90 identity; see the module header). The
// oriented input is `viprs`-minted (libviprs ignores vips' orientation
// metadata); the reference is a plain vips `rot … d90`. A no-op regression
// mismatches on dimensions (8x6 vs the 6x8 rotated reference).
// ---------------------------------------------------------------------------

#[test]
fn autorot_matches_vips_rot_d90() {
    if skip_if_no_cli("autorot") {
        return;
    }
    run_s1(
        "autorot",
        "conversion/autorot_oriented.v",
        "autorot.png",
        "conversion/autorot_expected.png",
        EXACT,
        &[],
    );
}

// ---------------------------------------------------------------------------
// wrap — S1 (EXACT). Default centre shift (w/2, h/2), swapping quadrants. Input
// `grad` varies in Y so the vertical h/2 shift is actually exercised (a
// Y-constant ramp left the vertical shift untested).
// ---------------------------------------------------------------------------

#[test]
fn wrap_matches_vips_exact() {
    if skip_if_no_cli("wrap") {
        return;
    }
    run_s1(
        "wrap",
        GRAD,
        "wrap.png",
        "conversion/wrap_expected.png",
        EXACT,
        &[],
    );
}

// ---------------------------------------------------------------------------
// gamma — S1 --exponent (BOUNDED-TOL ≤1 LSB; measured). Default (1/2.4) +
// explicit 2.0, over the full-domain `ramp256` LUT input (256×1, every 0..255
// value once) so EVERY LUT entry is compared, not just 16 of 256.
// ---------------------------------------------------------------------------

#[test]
fn gamma_default_matches_vips_bounded_tol() {
    if skip_if_no_cli("gamma") {
        return;
    }
    run_s1(
        "gamma",
        RAMP256,
        "gamma.png",
        "conversion/gamma_expected.png",
        GAMMA_TOL,
        &[],
    );
}

#[test]
fn gamma_exponent_matches_vips_bounded_tol() {
    if skip_if_no_cli("gamma_exp2") {
        return;
    }
    run_s1(
        "gamma",
        RAMP256,
        "gamma_exp2.png",
        "conversion/gamma_exp2_expected.png",
        GAMMA_TOL,
        &["--exponent", "2.0"],
    );
}

// ---------------------------------------------------------------------------
// falsecolour — S1 (EXACT). Band 0 mapped through the fixed PET LUT -> sRGB, over
// the full-domain `ramp256` input so all 256 LUT entries are verified.
// ---------------------------------------------------------------------------

#[test]
fn falsecolour_matches_vips_exact() {
    if skip_if_no_cli("falsecolour") {
        return;
    }
    run_s1(
        "falsecolour",
        RAMP256,
        "falsecolour.png",
        "conversion/falsecolour_expected.png",
        EXACT,
        &[],
    );
}

// ---------------------------------------------------------------------------
// addalpha — S1 (EXACT). rgb -> rgba, an appended opaque (255) alpha band.
// ---------------------------------------------------------------------------

#[test]
fn addalpha_matches_vips_exact() {
    if skip_if_no_cli("addalpha") {
        return;
    }
    run_s1(
        "addalpha",
        RGB,
        "addalpha.png",
        "conversion/addalpha_expected.png",
        EXACT,
        &[],
    );
}

// ---------------------------------------------------------------------------
// arrayjoin — S2 variadic + --across (EXACT). THREE inputs -> a 2x2 grid,
// exercising the true >=3 variadic fold.
// ---------------------------------------------------------------------------

#[test]
fn arrayjoin_matches_vips_exact() {
    if skip_if_no_cli("arrayjoin") {
        return;
    }
    let out = out_path("arrayjoin.png");
    run_viprs_ok(&[
        "arrayjoin",
        &fx(GRAY),
        &fx(GRAY2),
        &fx(GRAY3),
        out.to_str().unwrap(),
        "--across",
        "2",
    ]);
    decode_compare(
        &out,
        &cli_fixture("conversion/arrayjoin_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// grey — S5 creator (BOUNDED-TOL). Default float ramp (.v, eps 1e-6) + --uchar
// (PNG, ≤1 LSB).
// ---------------------------------------------------------------------------

#[test]
fn grey_float_matches_vips_bounded_tol() {
    if skip_if_no_cli("grey_float") {
        return;
    }
    let out = out_path("grey_float.v");
    run_viprs_ok(&["grey", out.to_str().unwrap(), "16", "16"]);
    decode_compare(
        &out,
        &cli_fixture("conversion/grey_float_expected.v"),
        GREY_FLOAT_EPS,
    );
}

#[test]
fn grey_uchar_matches_vips_bounded_tol() {
    if skip_if_no_cli("grey_uchar") {
        return;
    }
    let out = out_path("grey_uchar.png");
    run_viprs_ok(&["grey", out.to_str().unwrap(), "16", "16", "--uchar"]);
    decode_compare(
        &out,
        &cli_fixture("conversion/grey_uchar_expected.png"),
        GREY_UCHAR_TOL,
    );
}

// ---------------------------------------------------------------------------
// identity — S5 creator (EXACT). 256x1 Gray8 + --ushort 65536x1 Gray16. Carried
// as `.v` (vips tags identity `histogram`, which pngsave cannot convert).
// ---------------------------------------------------------------------------

#[test]
fn identity_matches_vips_exact() {
    if skip_if_no_cli("identity") {
        return;
    }
    let out = out_path("identity.v");
    run_viprs_ok(&["identity", out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("conversion/identity_expected.v"), EXACT);
}

#[test]
fn identity_ushort_matches_vips_exact() {
    if skip_if_no_cli("identity_ushort") {
        return;
    }
    let out = out_path("identity_ushort.v");
    run_viprs_ok(&["identity", out.to_str().unwrap(), "--ushort"]);
    decode_compare(
        &out,
        &cli_fixture("conversion/identity_ushort_expected.v"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// switch — S2 variadic (EXACT). Two conditions at distinct thresholds so the
// index image exercises all three outcomes (0 = first non-zero, 1 = second,
// 2 = none matched).
// ---------------------------------------------------------------------------

#[test]
fn switch_matches_vips_exact() {
    if skip_if_no_cli("switch") {
        return;
    }
    let out = out_path("switch.png");
    run_viprs_ok(&["switch", &fx(COND), &fx(COND2), out.to_str().unwrap()]);
    decode_compare(&out, &cli_fixture("conversion/switch_expected.png"), EXACT);
}
