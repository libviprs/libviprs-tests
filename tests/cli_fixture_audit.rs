//! CLI-differential **fixture-presence** audit (CLI_CONTRACT.md §7).
//!
//! §7 mandates that the harness asserts fixture PRESENCE — never vips presence —
//! so a committed reference that is accidentally deleted (or a `.gitignore` that
//! swallows a `.png`/`.tif`) fails loudly instead of the differential cell
//! silently comparing against a stale or missing file. This audit reads only
//! repository files: it runs under the **default `cargo test`** with no network,
//! no `libviprs-cli` sibling, no CLI build, and no vips oracle — exactly the
//! places the differential cells themselves skip.
//!
//! Every reference produced by `tools/gen_cli_expected.sh` and consumed by
//! `tests/cli_morphology_diff.rs` is listed here; keep the two in lockstep as
//! later families add fixtures.

use std::path::{Path, PathBuf};

/// Root of the committed CLI-differential fixtures.
fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli")
}

/// Every committed reference the CLI-differential suite depends on, relative to
/// [`fixtures_root`]. Grouped by the differential case that consumes it.
const REQUIRED_FIXTURES: &[&str] = &[
    // Provenance record (vips version + exact commands behind every reference).
    "PROVENANCE.md",
    // Inputs + structuring elements.
    "morphology/input.png",
    "morphology/input_gray.png",
    "morphology/cross.mat",
    "morphology/corner.mat",
    // morph erode/dilate — cross mask.
    "morphology/morph_erode_expected.png",
    "morphology/morph_dilate_expected.png",
    // morph erode/dilate — corner mask with a 0 cell (trit coverage).
    "morphology/morph_erode_corner_expected.png",
    "morphology/morph_dilate_corner_expected.png",
    // rank — binary + multi-level, median + non-median index.
    "morphology/rank_median_expected.png",
    "morphology/rank_gray_median_expected.png",
    "morphology/rank_gray_max_expected.png",
    // countlines (S3 scalar).
    "morphology/countlines_horizontal_expected.txt",
    "morphology/countlines_vertical_expected.txt",
    // labelregions (S4: mask + segment count).
    "morphology/labelregions_mask_expected.tif",
    "morphology/labelregions_segments_expected.txt",
    // ---- bands family (cli_bands_diff.rs) ----
    // Common inputs.
    "bands/gray.png",
    "bands/gray2.png",
    "bands/gray3.png",
    "bands/rgb.png",
    "bands/rgba.png",
    // bandjoin: 2-input (S2 variadic) -> 4-band rgba PNG, and 3-input (≥3
    // variadic fold) -> 3-band b-w .v; bandjoin_const (S1 vector) -> 4-band b-w
    // .v (raw-band carriers; see cli_bands_diff.rs).
    "bands/bandjoin_expected.png",
    "bands/bandjoin3_expected.v",
    "bands/bandjoin_const_expected.v",
    // bandfold (.v, b-w multiband) / bandunfold (1-band PNG).
    "bands/bandfold_expected.v",
    "bands/bandunfold_expected.png",
    // bandmean.
    "bands/bandmean_expected.png",
    // bandrank (S2 variadic + --index): median + min.
    "bands/bandrank_median_expected.png",
    "bands/bandrank_min_expected.png",
    // bandbool and|or|eor.
    "bands/bandbool_and_expected.png",
    "bands/bandbool_or_expected.png",
    "bands/bandbool_eor_expected.png",
    // extract_band: single band + --n consecutive bands.
    "bands/extract_band1_expected.png",
    "bands/extract_bandn_expected.png",
    // ---- extract family (cli_extract_diff.rs) ----
    // Common inputs (sub1.png = 1-band insert-broadcast payload).
    "extract/gray.png",
    "extract/rgb.png",
    "extract/sub.png",
    "extract/sub1.png",
    // extract_area / crop (alias).
    "extract/extract_area_expected.png",
    "extract/crop_expected.png",
    // embed: black, copy/repeat/mirror/white extend enum, background vector.
    "extract/embed_black_expected.png",
    "extract/embed_copy_expected.png",
    "extract/embed_repeat_expected.png",
    "extract/embed_mirror_expected.png",
    "extract/embed_white_expected.png",
    "extract/embed_bg_expected.png",
    // gravity: centre + dash-spelled south-east + north-west.
    "extract/gravity_centre_expected.png",
    "extract/gravity_se_expected.png",
    "extract/gravity_nw_expected.png",
    // replicate / zoom / subsample.
    "extract/replicate_expected.png",
    "extract/zoom_expected.png",
    "extract/subsample_expected.png",
    // insert: non-expand + expand (canvas grows) + 1-band-sub bandalike broadcast.
    "extract/insert_expected.png",
    "extract/insert_expand_expected.png",
    "extract/insert_bandalike_expected.png",
    // smartcrop: EXACT geometry + attention + all, and the GOLDEN-ONLY entropy pin.
    "extract/smartcrop_centre_expected.png",
    "extract/smartcrop_low_expected.png",
    "extract/smartcrop_high_expected.png",
    "extract/smartcrop_attention_expected.png",
    "extract/smartcrop_all_expected.png",
    "extract/smartcrop_entropy_golden.png",
    // ---- conversion family (cli_conversion_diff.rs) ----
    // Common inputs.
    "conversion/gray.png",
    "conversion/gray2.png",
    "conversion/gray3.png",
    "conversion/rgb.png",
    "conversion/rgba.png",
    // Discriminating inputs (adversarial-review findings 1/2/3/5): grad (2-D
    // gradient, for flip/rot/wrap), nb16 (non-palindromic 16-bit, for byteswap),
    // mb16 (3-band 16-bit, for msb --band), ramp256 (full 0..255 domain, LUT ops).
    "conversion/grad.png",
    "conversion/nb16.v",
    "conversion/mb16.v",
    "conversion/ramp256.png",
    "conversion/odd.png",
    "conversion/stack.png",
    "conversion/cond.png",
    "conversion/cond2.png",
    // autorot: `viprs`-minted oriented input (libviprs ignores vips'
    // orientation metadata) + the base for the rot-d90 reference.
    "conversion/autorot_base.png",
    "conversion/autorot_oriented.v",
    // References.
    "conversion/copy_expected.png",
    "conversion/cast_ushort_expected.v",
    "conversion/cast_float_expected.v",
    "conversion/flip_horizontal_expected.png",
    "conversion/flip_vertical_expected.png",
    "conversion/rot_d90_expected.png",
    "conversion/rot_d180_expected.png",
    "conversion/rot45_d45_expected.png",
    "conversion/byteswap_expected.v",
    "conversion/msb_expected.png",
    "conversion/msb_band0_expected.png",
    "conversion/grid_expected.png",
    "conversion/flatten_expected.png",
    "conversion/ifthenelse_expected.png",
    "conversion/autorot_expected.png",
    "conversion/wrap_expected.png",
    "conversion/gamma_expected.png",
    "conversion/gamma_exp2_expected.png",
    "conversion/falsecolour_expected.png",
    "conversion/addalpha_expected.png",
    "conversion/arrayjoin_expected.png",
    "conversion/grey_float_expected.v",
    "conversion/grey_uchar_expected.png",
    "conversion/identity_expected.v",
    "conversion/identity_ushort_expected.v",
    "conversion/switch_expected.png",
    // ---- core family (cli_core_diff.rs) ----
    // Common inputs: two distinct sRGB RGB sources + two Gray8 sources (sums
    // > 255, exercising the 8→16-bit widening) + a constant float `.v`.
    "core/add_a.png",
    "core/add_b.png",
    "core/gray_a.png",
    "core/gray_b.png",
    "core/getpoint_float.v",
    // Non-dyadic float input (getpoint numeric-eps de-rig case).
    "core/getpoint_float_nd.v",
    // 16-bit ushort inputs (add 16-bit-reject error case; INPUTS only).
    "core/u16_a.v",
    "core/u16_b.v",
    // add (EXACT-AFTER-CAST, tol 0): rgb + gray, carried as ushort `.v`.
    "core/add_rgb_expected.v",
    "core/add_gray_expected.v",
    // getpoint (S3): rgb vector, gray scalar, float vector, non-dyadic float.
    "core/getpoint_rgb_expected.txt",
    "core/getpoint_gray_expected.txt",
    "core/getpoint_float_expected.txt",
    "core/getpoint_float_nd_expected.txt",
    // ---- arithmetic part-B (cli_arithb_diff.rs) ----
    // Common inputs: a/b/c Gray8 ramps, rgb/rgba, small/small2 (pow 0^0),
    // eye (2-D stdif), the recomb matrix, and the 2-band-float complex input.
    "arithb/a.png",
    "arithb/b.png",
    "arithb/c.png",
    "arithb/rgb.png",
    "arithb/rgba.png",
    "arithb/small.png",
    "arithb/small2.png",
    "arithb/eye.png",
    "arithb/avert.png",
    "arithb/msmall.v",
    "arithb/macosh.v",
    "arithb/recomb.mat",
    "arithb/complex_in.v",
    // Binary: subtract (PNG) + a>=b full-range case, multiply/divide (.v),
    // minpair/maxpair (PNG).
    "arithb/subtract_expected.png",
    "arithb/subtract_pos_expected.png",
    "arithb/multiply_expected.v",
    "arithb/divide_expected.v",
    "arithb/minpair_expected.png",
    "arithb/maxpair_expected.png",
    // sum (>=3 variadic, ushort .v).
    "arithb/sum_expected.v",
    // relational / relational_const (ALL SIX enum arms each).
    "arithb/relational_more_expected.png",
    "arithb/relational_less_expected.png",
    "arithb/relational_equal_expected.png",
    "arithb/relational_noteq_expected.png",
    "arithb/relational_lesseq_expected.png",
    "arithb/relational_moreeq_expected.png",
    "arithb/relational_const_more_expected.png",
    "arithb/relational_const_equal_expected.png",
    "arithb/relational_const_noteq_expected.png",
    "arithb/relational_const_lesseq_expected.png",
    "arithb/relational_const_moreeq_expected.png",
    // boolean (and/or/eor) / boolean_const (and/or/eor/lshift/rshift).
    "arithb/boolean_eor_expected.png",
    "arithb/boolean_and_expected.png",
    "arithb/boolean_or_expected.png",
    "arithb/boolean_const_and_expected.png",
    "arithb/boolean_const_or_expected.png",
    "arithb/boolean_const_eor_expected.png",
    "arithb/boolean_const_lshift_expected.png",
    "arithb/boolean_const_rshift_expected.png",
    // Windowed: scale (linear + log), stdif, recomb, (un)premultiply.
    "arithb/scale_expected.png",
    "arithb/scale_log_expected.png",
    "arithb/stdif_expected.png",
    "arithb/recomb_expected.png",
    "arithb/premultiply_expected.png",
    "arithb/unpremultiply_expected.png",
    // Math (float .v): ALL 16 math arms + math2 atan2/pow/wop.
    "arithb/math_sin_expected.v",
    "arithb/math_cos_expected.v",
    "arithb/math_atan_expected.v",
    "arithb/math_tan_expected.v",
    "arithb/math_asin_expected.v",
    "arithb/math_acos_expected.v",
    "arithb/math_log_expected.v",
    "arithb/math_log10_expected.v",
    "arithb/math_exp_expected.v",
    "arithb/math_exp10_expected.v",
    "arithb/math_sinh_expected.v",
    "arithb/math_cosh_expected.v",
    "arithb/math_tanh_expected.v",
    "arithb/math_asinh_expected.v",
    "arithb/math_atanh_expected.v",
    "arithb/math_acosh_expected.v",
    "arithb/math2_atan2_expected.v",
    "arithb/math2_pow_expected.v",
    "arithb/math2_wop_expected.v",
    // Complex (FOURIER float band-pairs .v): complexform, complex, complexget.
    "arithb/complexform_expected.v",
    "arithb/complex_polar_expected.v",
    "arithb/complex_rect_expected.v",
    "arithb/complex_conj_expected.v",
    "arithb/complexget_real_expected.v",
    "arithb/complexget_imag_expected.v",
];

#[test]
fn every_cli_differential_reference_is_present() {
    let root = fixtures_root();
    let mut missing: Vec<String> = Vec::new();
    for rel in REQUIRED_FIXTURES {
        let path = root.join(rel);
        if !path.is_file() {
            missing.push(rel.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "CLI-differential reference fixture(s) missing under {}:\n  {}\n\
         Regenerate offline with tools/gen_cli_expected.sh (needs the vips oracle) \
         or restore the committed files.",
        root.display(),
        missing.join("\n  "),
    );
}

#[test]
fn required_fixture_list_has_no_duplicates() {
    // A duplicated entry would silently weaken the audit's bookkeeping.
    let mut seen = std::collections::BTreeSet::new();
    for rel in REQUIRED_FIXTURES {
        assert!(seen.insert(*rel), "duplicate fixture entry: {rel}");
    }
}
