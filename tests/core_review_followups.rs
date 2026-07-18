//! Vips-differential tests for the core review follow-ups
//! (libviprs/libviprs#321, #322, #323). Each exercises a newly-added core API
//! against a checked-in `vips`-generated reference, mirroring the golden-test
//! approach in `ported_foreign`: grab a committed fixture, produce the
//! reference output with the equivalent libvips operation offline, then assert
//! libviprs reproduces it. No libvips at runtime.
#![cfg(feature = "ported_tests")]

use std::path::Path;

use libviprs::{
    EngineBuilder, EngineKind, FsSink, Layout, PyramidPlanner, decode_file,
    extract_page_image_with_background_typed,
};

mod common;

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

/// #321: `EngineBuilder::run_region` must reproduce the vips crop-then-pyramid
/// reference `region_expected/` (vips crop 0,0,100,100 then dzsave, tile 64).
#[test]
fn region_builder_matches_vips_crop_dzsave() {
    let src = decode_file(&fixture("canonical_input.png")).unwrap();
    let (left, top, rw, rh) = (0u32, 0u32, 100u32, 100u32);
    let plan = PyramidPlanner::new(rw, rh, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("region_builder_out");
    let sink = FsSink::new(base.clone(), plan.clone());
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .run_region(left, top, rw, rh)
        .unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());

    let expected = common::dzsave_expected::collect_files(&fixture("region_expected"), "png");
    let actual = common::dzsave_expected::collect_files(&base, "png");
    common::dzsave_expected::assert_tiles_pixel_equal_tol(&expected, &actual, "region-builder", 0);
}

/// #322: `PyramidPlanner::with_iiif_id` must place the caller-supplied base URL
/// into `info.json` `@id` exactly as vips does (vips writes
/// `https://example.com/iiif/<name>`), and the tiles must match vips' IIIF
/// output under `iiif_id_expected/`.
#[test]
fn iiif_id_configurable_matches_vips() {
    let src = decode_file(&fixture("canonical_input.png")).unwrap();
    // vips wrote this exact @id into the reference info.json.
    let vips_id = "https://example.com/iiif/iiif_id_expected";
    let plan = PyramidPlanner::new(src.width(), src.height(), 128, 0, Layout::Iiif)
        .unwrap()
        .with_iiif_id(vips_id)
        .plan();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("iiif_id_out");
    let sink = FsSink::new(base.clone(), plan.clone());
    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .run()
        .unwrap();

    let expected_dir = fixture("iiif_id_expected");
    let expected = common::dzsave_expected::collect_files(&expected_dir, "png");
    let actual = common::dzsave_expected::collect_files(&base, "png");
    common::dzsave_expected::assert_tiles_pixel_equal_tol(&expected, &actual, "iiif-id", 0);

    let json = std::fs::read_to_string(base.join("info.json")).unwrap();
    let expected_json = std::fs::read_to_string(expected_dir.join("info.json")).unwrap();
    for needle in [
        format!("\"@id\": \"{vips_id}\""),
        "\"width\": 256".to_string(),
        "\"height\": 256".to_string(),
    ] {
        assert!(
            json.contains(&needle),
            "libviprs info.json missing {needle}: {json}"
        );
        assert!(
            expected_json.contains(&needle),
            "vips info.json missing {needle}"
        );
    }
}

/// #323: `extract_page_image_with_background_typed` (typed `BackgroundColor`)
/// must render byte-identically to the vips `pdfload --background` reference
/// (the same `pdf_bg_red_expected.png` the loosely-typed test uses).
#[test]
#[cfg_attr(
    not(feature = "pdfium"),
    ignore = "needs pdfium to render a PDF page with a background fill"
)]
fn pdf_background_typed_matches_vips() {
    let red = extract_page_image_with_background_typed(
        &fixture("canonical_solid_white.pdf"),
        1,
        [255u8, 0, 0, 255],
    )
    .unwrap();
    let reference = decode_file(&fixture("pdf_bg_red_expected.png")).unwrap();
    assert_eq!(
        (red.width(), red.height()),
        (reference.width(), reference.height())
    );
    let diff = red.max_diff(&reference);
    assert!(
        diff <= 1.0,
        "typed background render vs vips differ by {diff}"
    );
}
