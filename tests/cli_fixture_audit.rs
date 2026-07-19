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
