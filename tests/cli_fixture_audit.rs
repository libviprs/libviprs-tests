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
    // #558 discriminating inputs + the mask that breaks any tolerance.
    "convolution/noise64.png",
    "convolution/boxsum1147.png",
    "convolution/eye16.v",
    "convolution/hostile.mat",
    // gaussmat / logmat (S5 matrix creators → float `.v`).
    "convolution/gaussmat_int_expected.v",
    "convolution/gaussmat_sep_expected.v",
    "convolution/gaussmat_float_expected.v",
    "convolution/logmat_int_expected.v",
    "convolution/logmat_float_expected.v",
    // conv / convsep (matrix-file mask): integer (EXACT, VIPS_NOVECTOR=1
    // reference — issue #558) PNG + float `.v`.
    "convolution/conv_blur_int_expected.png",
    "convolution/conv_sobel_float_expected.v",
    "convolution/convsep_int_expected.png",
    "convolution/convsep_float_expected.v",
    // #558 discriminators: a window summing to 1147 (C 127, vector 128, floor
    // 127) and a mask libvips's own accuracy gate accepts on which the paths
    // differ by 57.
    "convolution/conv_boxsum1147_int_expected.png",
    "convolution/conv_hostile_int_expected.png",
    // #558 regime pins: scale-1 mask (vector path runs, agrees anyway) and a
    // ushort input (vector path gated off at convi.c:1151).
    "convolution/conv_sobel_int_expected.png",
    "convolution/conv_ushort_int_expected.v",
    // compass: max/integer (EXACT, scale-1 mask) PNG + sum/float `.v`.
    "convolution/compass_max_int_expected.png",
    "convolution/compass_sum_float_expected.v",
    "convolution/compass_sum_int_expected.v",
    // gaussblur: integer (EXACT, VIPS_NOVECTOR=1 reference) PNG + float `.v`,
    // plus the #558 sigma sweep — 1.6 (vector-path gap 4, the case a tol of 1
    // would fail), 0.8 (gap 2), and 0.6 (intize declines the mask, gap 0).
    "convolution/gaussblur_int_expected.png",
    "convolution/gaussblur_s1.6_int_expected.png",
    "convolution/gaussblur_s0.8_int_expected.png",
    "convolution/gaussblur_s0.6_int_expected.png",
    "convolution/gaussblur_float_expected.v",
    // sharpen (LabS unsharp, ≤1 LSB — issue #581, NOT #558) PNG.
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
    // ---- mosaicing family (cli_mosaicing_diff.rs) ----
    // Common inputs: merge textures (distinct seeds) + an sRGB for the
    // format-mismatch error case; mosaic crops of a common noise scene (large,
    // as the tie-point search needs 3 strips × 20 contrast windows); and the
    // viprs-minted globalbalance input carrying the join-tree blob.
    "mosaicing/merge_ref.png",
    "mosaicing/merge_sec.png",
    "mosaicing/merge_rgb.png",
    "mosaicing/merge_rgb_sec.png",
    "mosaicing/mosaic_h_ref.png",
    "mosaicing/mosaic_h_sec.png",
    "mosaicing/mosaic_v_ref.png",
    "mosaicing/mosaic_v_sec.png",
    "mosaicing/balance_input.v",
    // merge (EXACT): horizontal (lr) + vertical (tb) feathered blend, the RGB
    // multi-band path, and the positive-dx insert-fallback (no-blend) path.
    "mosaicing/merge_h_expected.png",
    "mosaicing/merge_v_expected.png",
    "mosaicing/merge_rgb_expected.png",
    "mosaicing/merge_fallback_expected.png",
    // mosaic (EXACT): horizontal (lr) + vertical (tb) tie-point search.
    "mosaicing/mosaic_h_expected.png",
    "mosaicing/mosaic_v_expected.png",
    // globalbalance (GOLDEN-ONLY): viprs merge → globalbalance pipeline pin.
    "mosaicing/balance_expected.v",
    // ---- create family (cli_create_diff.rs) ----
    // Common input: a buildlut control-point matrix (read by both sides), plus a
    // >=3-point non-collinear matrix pinning multi-segment interpolation.
    "create/buildlut_points.mat",
    "create/buildlut_points3.mat",
    // EXACT (tol 0): black (+--bands), xyz (vips uint→float).
    "create/black_expected.v",
    "create/black_bands3_expected.v",
    "create/xyz_expected.v",
    // BOUNDED-TOL float creators: eye (+--uchar), zone, sines, buildlut, tonelut.
    "create/eye_expected.v",
    "create/eye_uchar_expected.png",
    "create/zone_expected.v",
    "create/sines_expected.v",
    "create/buildlut_expected.v",
    "create/buildlut3_expected.v",
    "create/tonelut_expected.v",
    // BOUNDED-TOL frequency-domain masks (ideal/gaussian/butterworth/fractal),
    // incl. --nodc and --uchar --optical flag paths.
    "create/mask_ideal_expected.v",
    "create/mask_ideal_nodc_expected.v",
    "create/mask_ideal_ring_expected.v",
    "create/mask_ideal_band_expected.v",
    "create/mask_gaussian_expected.v",
    "create/mask_gaussian_nodc_expected.v",
    "create/mask_gaussian_ring_expected.v",
    "create/mask_gaussian_band_expected.v",
    "create/mask_butterworth_expected.v",
    "create/mask_butterworth_uchar_expected.png",
    "create/mask_butterworth_optical_expected.v",
    "create/mask_butterworth_ring_expected.v",
    "create/mask_butterworth_band_expected.v",
    "create/mask_fractal_expected.v",
    // BOUNDED-TOL signed distance fields (every shape).
    "create/sdf_circle_expected.v",
    "create/sdf_box_expected.v",
    "create/sdf_line_expected.v",
    "create/sdf_rounded_expected.v",
    // GOLDEN-ONLY viprs pins (no vips oracle: PRNG / Pango differ even seeded).
    "create/gaussnoise_golden.v",
    "create/perlin_golden.v",
    "create/worley_golden.v",
    "create/fractsurf_golden.v",
    "create/text_golden.png",
    // ---- draw family (cli_draw_diff.rs) — ALL GOLDEN-ONLY ----
    // Common inputs (built with vips as a deterministic coordinate function,
    // consumed only as `viprs` inputs; there is NO vips oracle for draw ops).
    "draw/rgb.png",
    "draw/flood.png",
    "draw/mask.png",
    "draw/sub.png",
    "draw/smudge.png",
    // 16-bit (Gray16) draw target for the 16-bit ink encode/draw pin.
    "draw/gray16.v",
    // References — every one minted by `viprs` itself (GOLDEN-ONLY regression
    // pins; `draw_*` are in-place mutators the vips CLI discards).
    "draw/draw_circle_golden.png",
    "draw/draw_circle_fill_golden.png",
    "draw/draw_rect_golden.png",
    "draw/draw_rect_fill_golden.png",
    "draw/draw_line_golden.png",
    "draw/draw_flood_golden.png",
    "draw/draw_flood_blob_golden.png",
    "draw/draw_mask_golden.png",
    "draw/draw_smudge_golden.png",
    "draw/draw_image_golden.png",
    "draw/draw_rect_16bit_golden.v",
    // ---- resample family (cli_resample_diff.rs) ----
    // Common inputs: a 2-D gradient (grad), a 2-D sRGB (rgb), a float 2-band
    // coordinate map (index.v, a 2× zoom) for mapim, and a float 1-band
    // high-frequency carrier (hf.v) for the unrounded resize cell.
    "resample/grad.png",
    "resample/rgb.png",
    "resample/index.v",
    "resample/hf.v",
    // shrink / shrinkh / shrinkv (box shrink).
    "resample/shrink_expected.png",
    "resample/shrinkh_expected.png",
    "resample/shrinkv_expected.png",
    // reduce (lanczos3 default + cubic enum) / reduceh / reducev.
    "resample/reduce_lanczos3_expected.png",
    "resample/reduce_cubic_expected.png",
    "resample/reduceh_expected.png",
    "resample/reducev_expected.png",
    // resize (half, --vscale, upscale→affine path, --kernel nearest).
    "resample/resize_half_expected.png",
    "resample/resize_vscale_expected.png",
    "resample/resize_vscale_float_expected.v",
    "resample/resize_up_expected.png",
    "resample/resize_nearest_expected.png",
    // affine (bilinear, held at 1 LSB by libviprs#733's BILINEAR_INT weights;
    // bicubic, 2 LSB until libviprs#702 and 1 until libviprs#704, EXACT now).
    "resample/affine_bilinear_expected.png",
    "resample/affine_bicubic_expected.png",
    // similarity (--angle / --scale) / rotate.
    "resample/similarity_angle_expected.png",
    "resample/similarity_scale_expected.png",
    "resample/rotate_expected.png",
    // mapim (S2: index is a 2nd input) bilinear + bicubic.
    "resample/mapim_bilinear_expected.png",
    "resample/mapim_bicubic_expected.png",
    // thumbnail (FILENAME input + non-square --crop + --linear) / thumbnail_image
    // (decoded variant).
    "resample/thumbnail_expected.png",
    "resample/thumbnail_crop_expected.png",
    "resample/thumbnail_linear_expected.png",
    "resample/thumbnail_image_expected.png",
    // ---- aritha family (cli_aritha_diff.rs) — arith part-A lane ----
    // Common inputs: agray (Gray8 ramp), afloat (float crossing zero, for
    // abs/sign/round), content/content2 (find_trim), pzero (profile), point
    // (the two hough golden pins).
    "aritha/agray.png",
    "aritha/afloat.v",
    "aritha/content.png",
    "aritha/content2.png",
    "aritha/pzero.png",
    "aritha/point.png",
    // S3 scalars (numeric compare): avg/deviate + min/max (+ --x --y position).
    "aritha/avg_expected.txt",
    "aritha/deviate_expected.txt",
    "aritha/min_expected.txt",
    "aritha/max_expected.txt",
    "aritha/min_xy_expected.txt",
    "aritha/max_xy_expected.txt",
    // find_trim: default (white bg) + --background 0 (black bg), 4 ints each.
    "aritha/find_trim_expected.txt",
    "aritha/find_trim_bg_expected.txt",
    // stats/measure: float matrix .v (vips double cast to float; stats cropped
    // to the 6 core columns).
    "aritha/stats_expected.v",
    "aritha/measure_expected.v",
    // profile/project: ushort .v two-output pairs.
    "aritha/profile_cols_expected.v",
    "aritha/profile_rows_expected.v",
    "aritha/project_cols_expected.v",
    "aritha/project_rows_expected.v",
    // const/linear: float linear + uchar linear + remainder + pow.
    "aritha/linear_expected.v",
    "aritha/linear_uchar_expected.png",
    "aritha/remainder_const_expected.png",
    "aritha/math2_const_pow_expected.v",
    // unary/rounding on float: abs/sign + round ceil/floor (EXACT vips oracle);
    // round rint is now EXACT against a real vips oracle (core #494 rounds
    // half-to-even, matching vips's C `rint`; see cli_aritha_diff.rs) — the
    // former GOLDEN-ONLY `round_rint_golden.v` pin is retired.
    "aritha/abs_expected.v",
    "aritha/sign_expected.v",
    "aritha/round_ceil_expected.v",
    "aritha/round_floor_expected.v",
    "aritha/round_rint_expected.v",
    // clamp (uchar).
    "aritha/clamp_expected.png",
    // hough: GOLDEN-ONLY viprs regression pins (no vips oracle — the core Hough
    // numerics diverge structurally from vips; see cli_aritha_diff.rs).
    "aritha/hough_line_golden.v",
    "aritha/hough_circle_golden.v",
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
    // ---- iocleanup family (cli_iocleanup_diff.rs) — io save-path #37/#38 ----
    // Common inputs: 8×8 8-bit rgb/gray + the same scaled to 16-bit (>255
    // samples) so a downcast is caught on value as well as format-class.
    "iocleanup/rgb.png",
    "iocleanup/gray.png",
    "iocleanup/rgb16.v",
    "iocleanup/gray16.v",
    // #38 PNM byte-exact references: P6/P5 × 8/16-bit.
    "iocleanup/flip_ppm_expected.ppm",
    "iocleanup/flip_pgm_expected.pgm",
    "iocleanup/copy_ppm16_expected.ppm",
    "iocleanup/copy_pgm16_expected.pgm",
    // #37 16-bit PNG save-path reference (decode-compare).
    "iocleanup/copy_png16_expected.png",
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
