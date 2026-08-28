//! CLI-DIFFERENTIAL suite — resample family (the Wave-2 resample lane,
//! CLI_CONTRACT.md §7, OP_MAP.md resample section).
//!
//! Each test runs `viprs <op> …` on the COMMON committed inputs under
//! `tests/fixtures/cli/resample/` and decode-compares its output against the
//! COMMITTED vips 8.18.4 reference at the op's §5 tolerance. The suite NEVER
//! runs vips: references are generated offline by `tools/gen_cli_expected.sh`
//! and committed. This cell copies the bands / morphology reference cells
//! exactly — including the skip-guard / `VIPRS_REQUIRE_CLI` discipline.
//!
//! **Most resample ops are now bit-exact against the oracle**, which is not what
//! this header said a landing ago. Three quantisations closed in sequence:
//! `table_offset` rounds the sub-pixel offset onto libvips' 65-entry grid at
//! both the reduce and the bicubic call sites (libviprs#668), the `uchar`
//! bicubic reads `vips_bicubic_matrixi` as 12-bit fixed point the way
//! `bicubic_unsigned_int_tab` does (libviprs#704), and the float bicubic
//! narrows each row sum through `cubic_float<float>` the way vips does
//! (libviprs#705). Measured against core `ed958d5`, **seventeen of the 23
//! comparison cells here read 0** and six read 1.
//!
//! **The six that read 1 are two named divergences, not slack**, and both are
//! carrier effects that go to 0 on the identical content promoted to float:
//!
//! * `affine` bilinear, both `similarity` cells and `rotate` carry
//!   libviprs#733. vips sends an integer carrier to `BILINEAR_INT`, whose four
//!   weights are truncated to 12-bit fixed point; the core keeps f64 because
//!   adopting the quantisation would introduce 6.28 LSB of new error on a
//!   16-bit carrier where there is currently none. That is a decision with a
//!   measurement behind it, recorded in `src/resample.rs`.
//! * `reduceh` and `reducev` carry libviprs#777, the `matrixs` fixed-point copy
//!   of the reduce mask vips uses on the integer carriers. That one is
//!   undecided rather than decided, and it is the last quantisation in the
//!   family with no answer yet.
//!
//! **What this header said before, and why it mattered.** It said the core
//! computed the masks in `f64` per output position "so the two agree to ≤1
//! LSB", and `AFFINE_BICUBIC_TOL = 2.0` said the same thing again about the
//! interpolator. That is the divergence libviprs#668 turned out to be, written
//! down as a design decision in two places across two repos, and it is a fair
//! part of why the bug lasted. Both are gone.
//!
//! **What is pinned, and what deliberately is not.** A cell earns [`EXACT`] by
//! being separated from 1 by a mutation of the thing it guards, never by
//! happening to measure 0, because a pin that no mutation moves buys no
//! falsifiability and costs a vips-version tripwire (libviprs#723). Eight
//! mutations of a scratch core at `ed958d5`, and the only rows that move:
//!
//! | cell | tol | `ed958d5` | r1 | r2 | r3 | r4 | r5 | r6 | m704 | m705 |
//! |---|---|---|---|---|---|---|---|---|---|---|
//! | `resize 0.5 --vscale 0.75`, uchar | 0 | 0 | 0 | 0 | 0 | 1 | 0 | 1 | 0 | 0 |
//! | `resize 0.5 --vscale 0.75`, float | 0 | 0 | .697 | .697 | 0 | 2.09 | 0 | 2.10 | 0 | 0 |
//! | `resize 2.0` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | **1** | 0 |
//! | `affine … bicubic` | 0 | 0 | 3 | 0 | 3 | 7 | 0 | 7 | **1** | 0 |
//! | `mapim … bicubic` | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | **1** | 0 |
//!
//! r1 reverts libviprs#668 outright, r2 its `reduce_axis` call site only, r3 its
//! bicubic call sites only, r4 and r6 coarsen the 1/64 offset grid to 1/32 and
//! 1/16, r5 truncates where `table_offset` floors, m704 stops the `uchar`
//! bicubic taking the fixed-point arm, m705 stops the float rows narrowing.
//! Every other cell reads the same in all nine columns, so the twelve remaining
//! zeroes stay at [`BT1`] and the six ones stay there too.
//!
//! **m705 moves nothing here**, which is a hole rather than a result: #705 only
//! fires on the float arm of the bicubic interpolator, every bicubic cell in
//! this file is `uchar`, and the one float cell goes through `reduce_axis`.
//! libviprs#780 is that hole.
//!
//! **r5 moves nothing either, and that one is fine.** floor and truncate differ
//! only on a negative source coordinate, which these ops never produce; the
//! core's own `resize_quantises_a_negative_source_coordinate_the_way_vips_shifted_space_does`
//! fails at 0.1227 under r5, so the arm has a guard, just not one a CLI
//! differential could carry.
//!
//! **The 23rd comparison cell is the reduce half of libviprs#668**
//! (libviprs#724). No uchar cell can see that half and no tolerance can give
//! one sight: on the committed PNG the two binaries are byte-identical, because
//! the divergence is under half an LSB before it is rounded into a byte. So
//! `resize_vscale_float_matches_vips_exactly` runs the same
//! `resize 0.5 --vscale 0.75` on the float `hf.v` carrier against a committed
//! float `.v` reference, and there the same mutation is worth 0.697 of a unit
//! on data spanning 0..250. The two halves of #668 now separate cleanly:
//! reverting the `reduce_axis` call site reddens that cell and nothing else,
//! reverting the bicubic call sites reddens `affine … bicubic` and nothing
//! else, and reverting both reddens both.
//!
//! Inputs are DISCRIMINATING (an identity / no-op op would FAIL): `grad.png` is a
//! 2-D gradient varying in BOTH axes (so `shrinkv`/`reducev`/`rot` are
//! non-vacuous), `index.v` maps every output to HALF its source coordinate (a
//! real 2× zoom, so `mapim` moves data rather than reproducing its input), and
//! `hf.v` holds `(i*37 + 11) mod 251` at `i = y*32 + x`, 251 distinct values in
//! 0..250 that are high-frequency in both axes so a fraction-of-a-pixel shift in
//! the resampling offset shows up as whole units instead of averaging out. The
//! `thumbnail --crop` case fits a NON-square 16×8 box so centre-crop actually
//! removes pixels — its reference is distinct from the no-crop 16×16 fixtures, so
//! a dropped / ignored `--crop` FAILS (a square box on a square source would crop
//! nothing and be vacuous).
//!
//! **Oracle scope — non-alpha only.** The ≤1 LSB BOUNDED-TOL contract holds for
//! inputs WITHOUT alpha. The core `reduce`/`shrink`/`resize` premultiply alpha
//! before resampling (a documented, intentional divergence from bare
//! `vips_reduce`/`vips_shrink`, which do not), so on an RGBA / GrayA input these
//! ops diverge from vips WHOLESALE — well beyond ≤1 LSB (measured max-abs-diff 4
//! for `shrink 2 2` on a 4-band sRGB ramp). That is deliberate and OUT of this
//! oracle class; every case here uses the no-alpha `grad`/`rgb` carriers.
//!
//! If the `libviprs-cli` sibling is not checked out (the default CI `test` job
//! and the Docker gate clone only the core counterpart), every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{cli_available, cli_fixture, decode_compare, run_viprs, run_viprs_ok};

use tempfile::TempDir;

/// BOUNDED-TOL ≤1 LSB (CLI_CONTRACT.md §5). Six cells are held here and every
/// one of them is a carrier effect that goes to 0 on the identical content
/// promoted to float, so this is not slack, it is two named divergences:
///
/// * `affine` bilinear, `similarity --angle`, `similarity --scale` and `rotate`
///   carry libviprs#733. `SWITCH_INTERPOLATE` sends a `uchar` carrier to
///   `BILINEAR_INT`, whose four weights are truncated to 12-bit fixed point,
///   and the core deliberately keeps `f64` there: adopting the quantisation
///   would introduce a mean error of 6.28 LSB on a 16-bit carrier where there
///   is currently none. That decision is recorded in `src/resample.rs`, so this
///   1 is permanent until that changes.
/// * `reduceh` and `reducev` carry libviprs#777, the `matrixs` fixed-point copy
///   of the reduce mask that vips uses on the integer carriers and the core
///   does not. That one is undecided rather than decided.
///
/// It is NOT a general slack: everything else in this file measures 0 and is
/// pinned at [`EXACT`].
const BT1: f64 = 1.0;

/// Bit-exact. Five cells are held here, and the test for admitting one is not
/// "it measures 0" but "a mutation of the thing it guards separates 0 from 1 on
/// it" (libviprs#723). Seventeen cells measure 0 against core `ed958d5`; these
/// five are the ones that earn the pin:
///
/// * `resize 0.5 --vscale 0.75` on `rgb.png`, which a resampling offset rounded
///   onto the wrong grid moves to 1 (libviprs#723).
/// * the same op on the float `hf.v`, where reverting libviprs#668's reduce
///   half is worth 0.697 of a unit (libviprs#724).
/// * `resize 2.0`, `affine … bicubic` and `mapim … bicubic`, which all read 1
///   again with libviprs#704 reverted. They measured 1 themselves until that
///   landed, so [`BT1`] on them now absorbs exactly the fix they exist to guard
///   (libviprs#753).
///
/// The other twelve zeroes stay at [`BT1`], because no mutation in the table in
/// this module's header separates 0 from 1 on them: pinning them would buy no
/// falsifiability and cost a vips-version tripwire.
const EXACT: f64 = 0.0;

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
/// Under `$VIPRS_REQUIRE_CLI=1` an absent sibling PANICS inside [`cli_available`]
/// instead, so a would-be silent skip in the dedicated job is a hard failure.
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

// Common committed inputs (tests/fixtures/cli/resample/).
const GRAD: &str = "resample/grad.png";
const RGB: &str = "resample/rgb.png";
const INDEX: &str = "resample/index.v";
const HF: &str = "resample/hf.v";

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// Convenience: absolute string path of a temp output file.
fn op(name: &str) -> String {
    out_path(name).to_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// shrink / shrinkh / shrinkv — S1 box shrink (2-D gradient → non-vacuous).
// ---------------------------------------------------------------------------

#[test]
fn shrink_matches_vips_bounded_tol() {
    if skip_if_no_cli("shrink") {
        return;
    }
    let out = op("shrink.png");
    run_viprs_ok(&["shrink", &fx(GRAD), &out, "2", "2"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/shrink_expected.png"),
        BT1,
    );
}

#[test]
fn shrinkh_matches_vips_bounded_tol() {
    if skip_if_no_cli("shrinkh") {
        return;
    }
    let out = op("shrinkh.png");
    run_viprs_ok(&["shrinkh", &fx(GRAD), &out, "2"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/shrinkh_expected.png"),
        BT1,
    );
}

#[test]
fn shrinkv_matches_vips_bounded_tol() {
    if skip_if_no_cli("shrinkv") {
        return;
    }
    let out = op("shrinkv.png");
    run_viprs_ok(&["shrinkv", &fx(GRAD), &out, "2"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/shrinkv_expected.png"),
        BT1,
    );
}

// ---------------------------------------------------------------------------
// reduce / reduceh / reducev — S1 kernel downsample (--kernel enum variants).
// ---------------------------------------------------------------------------

#[test]
fn reduce_lanczos3_matches_vips_bounded_tol() {
    if skip_if_no_cli("reduce_lanczos3") {
        return;
    }
    let out = op("reduce_l3.png");
    // No --kernel: viprs default lanczos3 == vips default.
    run_viprs_ok(&["reduce", &fx(RGB), &out, "2", "2"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/reduce_lanczos3_expected.png"),
        BT1,
    );
}

#[test]
fn reduce_cubic_matches_vips_bounded_tol() {
    if skip_if_no_cli("reduce_cubic") {
        return;
    }
    let out = op("reduce_cubic.png");
    // --kernel cubic exercises a DISTINCT kernel (a different mask than lanczos3).
    run_viprs_ok(&["reduce", &fx(RGB), &out, "2", "2", "--kernel", "cubic"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/reduce_cubic_expected.png"),
        BT1,
    );
}

#[test]
fn reduceh_matches_vips_bounded_tol() {
    if skip_if_no_cli("reduceh") {
        return;
    }
    let out = op("reduceh.png");
    // MEASURED 1, and it is a carrier effect: the same content promoted to float
    // reads 0. vips reduces an integer carrier through the `matrixs` fixed-point
    // copy of the mask where the core stays in f64 (libviprs#777), which is the
    // last quantisation in this family still undecided.
    run_viprs_ok(&["reduceh", &fx(GRAD), &out, "2"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/reduceh_expected.png"),
        BT1,
    );
}

#[test]
fn reducev_matches_vips_bounded_tol() {
    if skip_if_no_cli("reducev") {
        return;
    }
    let out = op("reducev.png");
    // MEASURED 1, same `matrixs` divergence as `reduceh` (libviprs#777).
    run_viprs_ok(&["reducev", &fx(GRAD), &out, "2"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/reducev_expected.png"),
        BT1,
    );
}

// ---------------------------------------------------------------------------
// resize — S1 scale (downscale, --vscale flag, upscale→affine path, --kernel).
// ---------------------------------------------------------------------------

#[test]
fn resize_half_matches_vips_bounded_tol() {
    if skip_if_no_cli("resize_half") {
        return;
    }
    let out = op("resize_half.png");
    run_viprs_ok(&["resize", &fx(RGB), &out, "0.5"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/resize_half_expected.png"),
        BT1,
    );
}

#[test]
fn resize_vscale_matches_vips_exactly() {
    if skip_if_no_cli("resize_vscale") {
        return;
    }
    let out = op("resize_vscale.png");
    // --vscale gives the two axes DIFFERENT scales (a non-square resize), and
    // 0.75 is the only NON-DYADIC factor in this file: 32x32 to 16x24, vertical
    // residual shrink 4/3, an offset that lands off the 1/64 grid at every
    // output row. MEASURED 0. It is pinned at 0 rather than at BT1 because a
    // resampling offset rounded onto the wrong grid moves it to 1
    // (libviprs#723), so ≤1 here absorbs the one regression it exists to catch.
    run_viprs_ok(&["resize", &fx(RGB), &out, "0.5", "--vscale", "0.75"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/resize_vscale_expected.png"),
        EXACT,
    );
}

#[test]
fn resize_vscale_float_matches_vips_exactly() {
    if skip_if_no_cli("resize_vscale_float") {
        return;
    }
    let out = op("resize_vscale_float.v");
    // The SAME op as `resize_vscale_matches_vips_exactly` on a FLOAT carrier,
    // and the only cell in this file that can see the reduce half of
    // libviprs#668. 32x32 to 16x24: horizontal 0.5 is dyadic and lands every
    // offset on the 1/64 grid, the vertical residual shrink is 4/3 and misses
    // it at nearly every output row, so `reduce_axis` is the whole difference
    // between the two binaries here.
    //
    // On the uchar twin that difference is under half an LSB and rounds away,
    // which is why the PNG cell reads 0 before libviprs#702, after it, and with
    // it reverted (libviprs#723). Unrounded it is 0.697 of a unit on data
    // spanning 0..250, four orders of magnitude above the f32 accumulation
    // noise, so EXACT here is a cell that separates a correct core from a
    // reverted one instead of one that measures nothing.
    //
    // EXACT on a float convolution is portable, not lucky. The reference is
    // committed bytes, so vips does not run here and its own FP contraction is
    // already baked in; the only thing that varies between the arm64 host this
    // was measured on and the x86-64 `cli-differential` runner is the CORE, and
    // that side cannot vary. Rust never contracts `a * b + c` into an FMA and
    // LLVM will not reassociate a float sum without fast-math, so `reduce_axis`
    // accumulates the same f64 products in the same order everywhere, and
    // `src/resample.rs` has no `target_arch` / `target_feature` / `std::arch` in
    // it. If this cell ever reddens, the core moved.
    run_viprs_ok(&["resize", &fx(HF), &out, "0.5", "--vscale", "0.75"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/resize_vscale_float_expected.v"),
        EXACT,
    );
}

#[test]
fn resize_up_matches_vips_exactly() {
    if skip_if_no_cli("resize_up") {
        return;
    }
    let out = op("resize_up.png");
    // An UPSCALE exercises the affine enlargement path (distinct from the reduce
    // downscale path the other resize cases take), and on a `uchar` carrier that
    // path is the fixed-point bicubic. MEASURED 0 since libviprs#704, 1 before
    // it and 1 again with it reverted, so EXACT is what makes this cell able to
    // tell the two apart and BT1 was absorbing it (libviprs#753).
    run_viprs_ok(&["resize", &fx(GRAD), &out, "2.0"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/resize_up_expected.png"),
        EXACT,
    );
}

#[test]
fn resize_nearest_matches_vips_bounded_tol() {
    if skip_if_no_cli("resize_nearest") {
        return;
    }
    let out = op("resize_nearest.png");
    // --kernel nearest takes the integer-subsample resize path.
    run_viprs_ok(&["resize", &fx(RGB), &out, "0.5", "--kernel", "nearest"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/resize_nearest_expected.png"),
        BT1,
    );
}

// ---------------------------------------------------------------------------
// affine — S1 matrix transform. Both interpolators measure 1 LSB. Bicubic used
// to measure 2, and that 2 was libviprs#668 (see header).
// ---------------------------------------------------------------------------

#[test]
fn affine_bilinear_matches_vips_bounded_tol() {
    if skip_if_no_cli("affine_bilinear") {
        return;
    }
    let out = op("affine_bilinear.png");
    // No --interpolate: viprs default bilinear == vips's affine interpolator default.
    // MEASURED 1, and it is a carrier effect: the same content promoted to float
    // reads 0. `SWITCH_INTERPOLATE` sends a `uchar` carrier to `BILINEAR_INT`,
    // whose four weights are truncated to 12-bit fixed point, and the core keeps
    // f64 there on purpose (libviprs#733 measures adopting it at 6.28 LSB of new
    // error on a 16-bit carrier). So this 1 is a decision, not slack, and it is
    // the same 1 the two `similarity` cells and `rotate` carry.
    run_viprs_ok(&["affine", &fx(RGB), &out, "1.5 0 0 1.5"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/affine_bilinear_expected.png"),
        BT1,
    );
}

#[test]
fn affine_bicubic_matches_vips_exactly() {
    if skip_if_no_cli("affine_bicubic") {
        return;
    }
    let out = op("affine_bicubic.png");
    // This used to be the ONE resample case that exceeded 1 LSB, at a measured
    // 2, and the reason given for it was the divergence libviprs#668 turned out
    // to be. Two landings closed it. libviprs#702 rounded the bicubic offset
    // onto vips's 1/64 grid and took it from 2 to 1; libviprs#704 ported
    // `vips_bicubic_matrixi`, the 12-bit fixed-point coefficients
    // `bicubic_unsigned_int_tab` reads on a `uchar` carrier, and took it from 1
    // to 0.
    //
    // So EXACT here is three assertions in one, each with a mutation behind it:
    // 1 with libviprs#704 reverted, 3 with libviprs#702's bicubic call sites
    // reverted, 7 with the offset grid coarsened to 1/32. BT1 would pass the
    // first of those, which is why it moved (libviprs#753).
    run_viprs_ok(&[
        "affine",
        &fx(RGB),
        &out,
        "1.5 0 0 1.5",
        "--interpolate",
        "bicubic",
    ]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/affine_bicubic_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// similarity / rotate — S1 rotate + scale.
// ---------------------------------------------------------------------------

#[test]
fn similarity_angle_matches_vips_bounded_tol() {
    if skip_if_no_cli("similarity_angle") {
        return;
    }
    let out = op("similarity_angle.png");
    // MEASURED 1, the `BILINEAR_INT` divergence (libviprs#733).
    run_viprs_ok(&["similarity", &fx(RGB), &out, "--angle", "30"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/similarity_angle_expected.png"),
        BT1,
    );
}

#[test]
fn similarity_scale_matches_vips_bounded_tol() {
    if skip_if_no_cli("similarity_scale") {
        return;
    }
    let out = op("similarity_scale.png");
    // MEASURED 1, the `BILINEAR_INT` divergence (libviprs#733).
    run_viprs_ok(&["similarity", &fx(RGB), &out, "--scale", "1.5"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/similarity_scale_expected.png"),
        BT1,
    );
}

#[test]
fn rotate_matches_vips_bounded_tol() {
    if skip_if_no_cli("rotate") {
        return;
    }
    let out = op("rotate.png");
    // MEASURED 1, the `BILINEAR_INT` divergence (libviprs#733).
    run_viprs_ok(&["rotate", &fx(RGB), &out, "30"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/rotate_expected.png"),
        BT1,
    );
}

// ---------------------------------------------------------------------------
// mapim — S2, the index is a SECOND input (float .v). vips order `in out index`.
// ---------------------------------------------------------------------------

#[test]
fn mapim_bilinear_matches_vips_bounded_tol() {
    if skip_if_no_cli("mapim_bilinear") {
        return;
    }
    let out = op("mapim_bilinear.png");
    run_viprs_ok(&["mapim", &fx(RGB), &out, &fx(INDEX)]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/mapim_bilinear_expected.png"),
        BT1,
    );
}

#[test]
fn mapim_bicubic_matches_vips_exactly() {
    if skip_if_no_cli("mapim_bicubic") {
        return;
    }
    let out = op("mapim_bicubic.png");
    // MEASURED 0 since libviprs#704, and 1 with it reverted. `index.v` is an
    // exact 2x zoom so every offset lands on the 1/64 grid and no mutation of
    // the OFFSET moves this cell at all; the fixed-point coefficients are the
    // only thing it can see, and BT1 could not see them (libviprs#753).
    run_viprs_ok(&[
        "mapim",
        &fx(RGB),
        &out,
        &fx(INDEX),
        "--interpolate",
        "bicubic",
    ]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/mapim_bicubic_expected.png"),
        EXACT,
    );
}

// ---------------------------------------------------------------------------
// thumbnail — S1; the FIRST arg is a source FILENAME (not a decoded raster).
// thumbnail_image — the decoded-image variant.
// ---------------------------------------------------------------------------

#[test]
fn thumbnail_matches_vips_bounded_tol() {
    if skip_if_no_cli("thumbnail") {
        return;
    }
    let out = op("thumbnail.png");
    run_viprs_ok(&["thumbnail", &fx(RGB), &out, "16"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/thumbnail_expected.png"),
        BT1,
    );
}

#[test]
fn thumbnail_crop_matches_vips_bounded_tol() {
    if skip_if_no_cli("thumbnail_crop") {
        return;
    }
    let out = op("thumbnail_crop.png");
    // DISCRIMINATING crop: a NON-square target (16×8) so centre-cropping a square
    // source actually removes pixels — the reference is DISTINCT from every
    // no-crop 16×16 fixture, so a build that ignored / dropped --crop would FAIL.
    // (A square 16 box on a square source crops nothing — the vacuous identity
    // case the adversarial review flagged.) `--crop centre` is the literal vips
    // spelling, now accepted by the optional-value flag arity (bare `--crop`
    // means the same `centre`).
    run_viprs_ok(&[
        "thumbnail",
        &fx(RGB),
        &out,
        "16",
        "--height",
        "8",
        "--crop",
        "centre",
    ]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/thumbnail_crop_expected.png"),
        BT1,
    );
}

#[test]
fn thumbnail_linear_matches_vips_bounded_tol() {
    if skip_if_no_cli("thumbnail_linear") {
        return;
    }
    let out = op("thumbnail_linear.png");
    // --linear routes to the linear-light reduce core entry point (distinct from
    // the default sRGB-space reduce), directly covering that path against vips.
    run_viprs_ok(&["thumbnail", &fx(RGB), &out, "16", "--linear"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/thumbnail_linear_expected.png"),
        BT1,
    );
}

#[test]
fn thumbnail_image_matches_vips_bounded_tol() {
    if skip_if_no_cli("thumbnail_image") {
        return;
    }
    let out = op("thumbnail_image.png");
    run_viprs_ok(&["thumbnail_image", &fx(RGB), &out, "16"]);
    decode_compare(
        &PathBuf::from(&out),
        &cli_fixture("resample/thumbnail_image_expected.png"),
        BT1,
    );
}

// ---------------------------------------------------------------------------
// Bounds / error cases (CLI_CONTRACT.md §8): nonzero exit + a viprs-side message
// substring, never a panic. Never asserts vips stderr.
// ---------------------------------------------------------------------------

#[test]
fn shrink_rejects_a_sub_unit_factor_with_exit_1() {
    if skip_if_no_cli("shrink_bad_factor") {
        return;
    }
    // A factor below vips's minimum (1.0) is a typed core error → exit 1, not a
    // panic. clap parses 0.5 as a valid f64, so the rejection is the core's.
    let out = op("shrink_bad.png");
    let res = run_viprs(&["shrink", &fx(GRAD), &out, "0.5", "0.5"]);
    assert!(
        !res.status.success(),
        "a sub-unit shrink factor must be rejected (nonzero exit)"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("factor"),
        "expected a viprs-side 'factor' error, got stderr: {stderr:?}"
    );
}

#[test]
fn shrinkh_rejects_a_zero_factor_with_nonzero_exit() {
    if skip_if_no_cli("shrinkh_bad_factor") {
        return;
    }
    // vips's minimum shrinkh factor is 1; the `1..` value_parser rejects 0 at
    // parse time (clap usage error, exit 2), never a panic.
    let out = op("shrinkh_bad.png");
    let res = run_viprs(&["shrinkh", &fx(GRAD), &out, "0"]);
    assert!(
        !res.status.success(),
        "a zero shrinkh factor must be rejected (nonzero exit)"
    );
}
