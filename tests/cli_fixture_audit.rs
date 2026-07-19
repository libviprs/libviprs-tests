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
    // ---- convolution family (cli_convolution_diff.rs) ----
    // Common inputs (eye = high-frequency zone-plate; patch = correlation
    // template) + the three mask files.
    "convolution/eye.png",
    "convolution/patch.png",
    "convolution/blur.mat",
    "convolution/sobel.mat",
    "convolution/sep.mat",
    // gaussmat / logmat (S5 matrix creators → float `.v`).
    "convolution/gaussmat_int_expected.v",
    "convolution/gaussmat_sep_expected.v",
    "convolution/gaussmat_float_expected.v",
    "convolution/logmat_int_expected.v",
    "convolution/logmat_float_expected.v",
    // conv / convsep (matrix-file mask): integer (≤1 LSB) PNG + float `.v`.
    "convolution/conv_blur_int_expected.png",
    "convolution/conv_sobel_float_expected.v",
    "convolution/convsep_float_expected.v",
    // compass: max/integer (EXACT, scale-1 mask) PNG + sum/float `.v`.
    "convolution/compass_max_int_expected.png",
    "convolution/compass_sum_float_expected.v",
    "convolution/compass_sum_int_expected.v",
    // gaussblur: integer (≤1 LSB) PNG + float `.v`.
    "convolution/gaussblur_int_expected.png",
    "convolution/gaussblur_float_expected.v",
    // sharpen (LabS unsharp, ≤1 LSB) PNG.
    "convolution/sharpen_expected.png",
    // spcor (float NCC) + fastcor (uint→float SSD, EXACT) surfaces.
    "convolution/spcor_expected.v",
    "convolution/fastcor_expected.v",
    // ---- matrix family (cli_matrix_diff.rs) ----
    // Common inputs: vips text-matrix files (consumed by both the generator and
    // the `viprs` MatFile loader). m3 = 3x3 (matrixinvert direct cofactor path),
    // m4 = 4x4 (matrixinvert PLU path), lut = 3x3 measured points (invertlut).
    "matrix/m3.mat",
    "matrix/m4.mat",
    "matrix/lut.mat",
    // matrixinvert (BOUNDED-TOL f32): direct 3x3 + PLU 4x4, cast double->float `.v`.
    "matrix/matrixinvert3_expected.v",
    "matrix/matrixinvert4_expected.v",
    // invertlut (BOUNDED-TOL f32): default size 256 + explicit --size 64.
    "matrix/invertlut_expected.v",
    "matrix/invertlut_size64_expected.v",
    // ---- colour family (cli_colour_diff.rs) ----
    // Common inputs: two distinct sRGB images, an sRGB matrix-shaper ICC
    // profile, and a D50 Lab PCS image (the shared icc_export input).
    "colour/rgb.png",
    "colour/rgb2.png",
    "colour/sRGB.icc",
    "colour/icc_pcs_lab.v",
    // colourspace: LAB/XYZ/scRGB float (.v) + the #36 interpretation-aware PNG
    // save + the --source-space override (uchar PNG).
    "colour/colourspace_lab_expected.v",
    "colour/colourspace_xyz_expected.v",
    "colour/colourspace_scrgb_expected.v",
    "colour/colourspace_lab_png_expected.png",
    "colour/colourspace_lab_input_png_expected.png",
    "colour/colourspace_srcspace_expected.png",
    // dE76 / dE00 float ΔE (.v); dECMC is a GOLDEN-ONLY viprs pin (vips computes
    // a different formula — see cli_colour_diff.rs).
    "colour/dE76_expected.v",
    "colour/dE00_expected.v",
    "colour/dECMC_golden.v",
    // ICC (matrix-shaper sRGB): import Lab PCS (.v), export + transform device
    // PNG.
    "colour/icc_import_lab_expected.v",
    "colour/icc_export_expected.png",
    "colour/icc_export_d16_expected.png",
    "colour/icc_transform_expected.png",
    // ---- histogram family (cli_histogram_diff.rs) ----
    // Common image inputs + committed histogram-shaped inputs.
    "histogram/gray.png",
    "histogram/rgb.png",
    "histogram/index.png",
    "histogram/hist.v",
    "histogram/histcum.v",
    "histogram/hist2.v",
    "histogram/lut.v",
    // EXACT count / LUT references (vips uint cast to ushort).
    "histogram/hist_find_expected.v",
    "histogram/hist_find_band_expected.v",
    // --band 2 (diagonal band, triangular histogram distinct from band 0):
    // pins band-index honouring, not just the 1-vs-3-band output shape.
    "histogram/hist_find_band2_expected.v",
    "histogram/hist_find_indexed_expected.v",
    "histogram/hist_find_ndim_expected.v",
    "histogram/hist_cum_expected.v",
    "histogram/hist_norm_expected.v",
    // hist_equal (BOUNDED-TOL ≤1) / maplut (EXACT) — plain b-w → PNG.
    "histogram/hist_equal_expected.png",
    "histogram/maplut_expected.png",
    // S3 scalars: hist_entropy (uniform + non-uniform), hist_ismonotonic (F/T).
    "histogram/hist_entropy_expected.txt",
    "histogram/hist_entropy_cum_expected.txt",
    "histogram/hist_ismonotonic_false_expected.txt",
    "histogram/hist_ismonotonic_true_expected.txt",
    // GOLDEN-ONLY viprs regression pins (no vips oracle — core diverges).
    "histogram/hist_match_golden.v",
    "histogram/hist_plot_golden.v",
    "histogram/hist_local_golden.png",
    "histogram/hist_local_clahe_golden.png",
    "histogram/percent_golden.txt",
    // ---- composite family (cli_composite_diff.rs) ----
    // Common inputs: translucent RGBA base/overlay (varying alpha) + opaque RGB
    // counterparts + a 1-band grey (band-mismatch error case, no reference).
    "composite/base.png",
    "composite/overlay.png",
    "composite/base_op.png",
    "composite/overlay_op.png",
    "composite/gray.png",
    // Porter-Duff simple modes on TRANSLUCENT inputs (real vips oracle, tol 1).
    "composite/composite2_over_expected.png",
    "composite/composite2_source_expected.png",
    "composite/composite2_in_expected.png",
    "composite/composite2_xor_expected.png",
    "composite/composite2_add_expected.png",
    "composite/composite2_dest_over_expected.png",
    // composite (vips array form) over on translucent inputs.
    "composite/composite_over_expected.png",
    // PDF separable blends on OPAQUE inputs (real vips oracle, tol 1).
    "composite/composite2_multiply_expected.png",
    "composite/composite2_screen_expected.png",
    "composite/composite2_overlay_expected.png",
    "composite/composite2_darken_expected.png",
    "composite/composite2_hardlight_expected.png",
    "composite/composite2_difference_expected.png",
    "composite/composite2_exclusion_expected.png",
    "composite/composite2_colourdodge_expected.png",
    // Remaining 9 modes on OPAQUE inputs (real vips oracle, tol 1) — every one of
    // the 25 VipsBlendMode spellings is discriminated, closing the wiring hole.
    "composite/composite2_clear_expected.png",
    "composite/composite2_out_expected.png",
    "composite/composite2_dest_expected.png",
    "composite/composite2_dest_in_expected.png",
    "composite/composite2_dest_out_expected.png",
    "composite/composite2_dest_atop_expected.png",
    "composite/composite2_lighten_expected.png",
    "composite/composite2_colourburn_expected.png",
    "composite/composite2_softlight_expected.png",
    // GOLDEN-ONLY translucent divergence pins (viprs-generated, no vips oracle).
    "composite/composite2_multiply_translucent_golden.png",
    "composite/composite2_atop_translucent_golden.png",
    "composite/composite2_saturate_translucent_golden.png",
    // ---- freqfilt family (cli_freqfilt_diff.rs) ----
    // Common inputs: 2-D gradient `in`, low-pass `mask`, (3,2)-shifted copy, and
    // an 8×8 `small` (the wrong-size dimension-mismatch error input).
    "freqfilt/in.png",
    "freqfilt/mask.v",
    "freqfilt/shifted.png",
    "freqfilt/small.png",
    // fwfft / invfft (complex + --real) / round-trip — complex outputs normalised
    // to 2-band f32 `.v` (band0 re, band1 im); real outputs cast to 1-band f32.
    "freqfilt/fwfft_expected.v",
    "freqfilt/invfft_expected.v",
    "freqfilt/invfft_real_expected.v",
    "freqfilt/roundtrip_expected.v",
    // freqmult / spectrum — uchar PNG (BOUNDED-TOL ≤1 LSB / tol 0).
    "freqfilt/freqmult_expected.png",
    "freqfilt/spectrum_expected.png",
    // phasecor — real correlation surface, 1-band f32 `.v`.
    "freqfilt/phasecor_expected.v",
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
