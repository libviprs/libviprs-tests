#![cfg(feature = "ported_tests")]

//! Audit test: verifies that all fixture files required by the ported test suite
//! exist in `libvips-reference-tests/test-suite/images/`.
//!
//! Run with: `cargo test --features ported_tests fixture_audit`
//!
//! A single summary test at the end fails with a detailed report if any files
//! are missing, listing every affected test function.

use std::path::{Path, PathBuf};

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> PathBuf {
    Path::new(REF_IMAGES).join(name)
}

// ---- Fixture dependency registry ----
//
// Each entry: (fixture filename, &[test functions that use it])
// Sorted by fixture name for easy maintenance.

const FIXTURE_DEPS: &[(&str, &[&str])] = &[
    ("1.webp", &[
        "ported_foreign::test_webp",
    ]),
    ("CMU-1-Small-Region.svs", &[
        "ported_foreign::test_openslideload",
    ]),
    ("ISO_12233-reschart.pdf", &[
        "ported_foreign::test_pdf_reference_reschart",
    ]),
    ("MARBLES.BMP", &[
        "ported_foreign::test_magickload",
    ]),
    ("WFPC2u5780205r_c0fx.fits", &[
        "ported_foreign::test_fitsload",
    ]),
    ("avg152T1_LR_nifti.nii.gz", &[
        "ported_foreign::test_niftiload",
    ]),
    ("avif-orientation-6.avif", &[
        "ported_foreign::test_heifload",
    ]),
    ("cd1.1.jpg", &[
        "ported_mosaicing::test_lrmerge",
        "ported_mosaicing::test_lrmosaic",
        "ported_mosaicing::test_mosaic",
        "ported_mosaicing::test_tbmerge",
        "ported_mosaicing::test_tbmosaic",
    ]),
    ("cd1.2.jpg", &[
        "ported_mosaicing::test_lrmerge",
        "ported_mosaicing::test_lrmosaic",
        "ported_mosaicing::test_mosaic",
    ]),
    ("cd2.1.jpg", &[
        "ported_mosaicing::test_mosaic",
        "ported_mosaicing::test_tbmerge",
        "ported_mosaicing::test_tbmosaic",
    ]),
    ("cd2.2.jpg", &[
        "ported_mosaicing::test_mosaic",
    ]),
    ("cd3.1.jpg", &[
        "ported_mosaicing::test_mosaic",
    ]),
    ("cd3.2.jpg", &[
        "ported_mosaicing::test_mosaic",
    ]),
    ("cd4.1.jpg", &[
        "ported_mosaicing::test_mosaic",
    ]),
    ("cd4.2.jpg", &[
        "ported_mosaicing::test_mosaic",
    ]),
    ("cmyktest.pdf", &[
        "ported_foreign::test_pdf_cmyk",
    ]),
    ("cogs.gif", &[
        "ported_foreign::test_magickload",
    ]),
    ("dicom_test_image.dcm", &[
        "ported_foreign::test_magickload",
    ]),
    ("dispose-background.gif", &[
        "ported_foreign::test_gifload_animation_dispose_background",
    ]),
    ("dispose-background.png", &[
        "ported_foreign::test_gifload_animation_dispose_background",
    ]),
    ("dispose-previous.gif", &[
        "ported_foreign::test_gifload_animation_dispose_previous",
    ]),
    ("dispose-previous.png", &[
        "ported_foreign::test_gifload_animation_dispose_previous",
    ]),
    ("favicon.ico", &[
        "ported_foreign::test_magickload",
    ]),
    ("garden.gif", &[
        "ported_foreign::test_gifload_frame_error",
    ]),
    ("indexed.png", &[
        "ported_foreign::test_png_load_palette",
    ]),
    ("logo.svg", &[
        "ported_foreign::test_magickload",
    ]),
    ("multi-channel-z-series.ome.tif", &[
        "ported_foreign::test_tiff_multipage",
    ]),
    ("ojpeg-strip.tif", &[
        "ported_foreign::test_tiff_ojpeg",
    ]),
    ("ojpeg-tile.tif", &[
        "ported_foreign::test_tiff_ojpeg",
        "ported_foreign::test_tiff_tile",
    ]),
    ("rgba-correct.ppm", &[
        "ported_foreign::test_ppm",
    ]),
    ("rgba.png", &[
        "ported_conversion::test_smartcrop_rgba",
        "ported_conversion::test_smartcrop_rgba_premultiplied",
        "ported_foreign::test_png_load_8bit",
        "ported_foreign::test_png_load_rgba",
    ]),
    ("sRGB.icm", &[
        "ported_colour::test_icc",
        "ported_infrastructure::test_keep_custom_profile",
    ]),
    ("sample-xyb.jpg", &[
        "ported_resample::test_thumbnail_icc",
    ]),
    ("sample.cur", &[
        "ported_foreign::test_magickload",
    ]),
    ("sample.exr", &[
        "ported_foreign::test_openexrload",
    ]),
    ("sample.hdr", &[
        "ported_foreign::test_analyzeload",
        "ported_foreign::test_rad",
    ]),
    ("sample.jpg", &[
        "ported_colour::test_cmyk",
        "ported_colour::test_icc",
        "ported_connection::test_connection_csv",
        "ported_connection::test_connection_dz",
        "ported_connection::test_connection_matrix",
        "ported_connection::test_connection_ppm",
        "ported_connection::test_connection_tiff",
        "ported_iofuncs::test_get_fields",
        "ported_connection::test_image_new_from_source_file",
        "ported_connection::test_image_new_from_source_memory",
        "ported_connection::test_image_write_to_target_file",
        "ported_connection::test_image_write_to_target_memory",
        "ported_iofuncs::test_revalidate",
        "ported_connection::test_source_new_from_file",
        "ported_conversion::test_autorot",
        "ported_conversion::test_smartcrop",
        "ported_conversion::test_smartcrop_attention",
        "ported_convolution::test_compass",
        "ported_convolution::test_conv",
        "ported_convolution::test_convsep",
        "ported_convolution::test_fastcor",
        "ported_convolution::test_gaussblur",
        "ported_convolution::test_sharpen",
        "ported_convolution::test_spcor",
        "ported_foreign::test_avifsave",
        "ported_foreign::test_avifsave_chroma",
        "ported_foreign::test_avifsave_exif",
        "ported_foreign::test_avifsave_icc",
        "ported_foreign::test_avifsave_lossless",
        "ported_foreign::test_avifsave_q",
        "ported_foreign::test_avifsave_tune",
        "ported_foreign::test_dz_layout_deepzoom",
        "ported_foreign::test_dz_region",
        "ported_foreign::test_heicsave_8_to_16",
        "ported_foreign::test_jp2ksave",
        "ported_foreign::test_jpeg_autorot",
        "ported_foreign::test_jpeg_load_dimensions",
        "ported_foreign::test_jpeg_load_from_memory",
        "ported_foreign::test_jpeg_load_pixel_values",
        "ported_foreign::test_jpeg_save_exif",
        "ported_foreign::test_jpeg_save_icc",
        "ported_foreign::test_jpeg_save_quality",
        "ported_foreign::test_jpeg_save_subsample",
        "ported_foreign::test_jpeg_sequential",
        "ported_foreign::test_jpeg_shrink_on_load",
        "ported_foreign::test_jpegsave_exif",
        "ported_foreign::test_jpegsave_exif_2_3_ascii",
        "ported_foreign::test_jpegsave_exif_2_3_ascii_2",
        "ported_foreign::test_jxlsave",
        "ported_foreign::test_magickload",
        "ported_foreign::test_magicksave",
        "ported_foreign::test_png_load_interlaced",
        "ported_foreign::test_vips",
        "ported_histogram::test_hist_entropy",
        "ported_histogram::test_hist_equal",
        "ported_histogram::test_hist_local",
        "ported_histogram::test_percent",
        "ported_histogram::test_stdif",
        "ported_infrastructure::test_cli_rotate",
        "ported_infrastructure::test_cli_thumbnail",
        "ported_infrastructure::test_fd_leak_jpeg",
        "ported_infrastructure::test_keep_custom_profile",
        "ported_infrastructure::test_keep_icc",
        "ported_infrastructure::test_keep_none",
        "ported_infrastructure::test_keep_xmp",
        "ported_iofuncs::test_new_from_image",
        "ported_infrastructure::test_pipeline_stall",
        "ported_infrastructure::test_progress_cancel",
        "ported_infrastructure::test_seq_no_temps",
        "ported_infrastructure::test_seq_shrink_no_temps",
        "ported_infrastructure::test_seq_thumbnail",
        "ported_infrastructure::test_threading_consistency",
        "ported_resample::test_affine",
        "ported_resample::test_mapim",
        "ported_resample::test_reduce",
        "ported_resample::test_resize_rounding",
        "ported_resample::test_rotate",
        "ported_resample::test_shrink",
        "ported_resample::test_shrink_average",
        "ported_resample::test_similarity",
        "ported_resample::test_similarity_scale",
        "ported_resample::test_thumbnail",
        "ported_resample::test_thumbnail_icc",
    ]),
    ("sample.mat", &[
        "ported_foreign::test_matload",
    ]),
    ("sample.png", &[
        "ported_foreign::test_heicsave_16_to_12",
        "ported_foreign::test_heicsave_16_to_8",
        "ported_foreign::test_jp2ksave",
        "ported_foreign::test_png_exif",
        "ported_foreign::test_png_icc",
        "ported_foreign::test_png_load_16bit_reference",
        "ported_foreign::test_png_load_dimensions",
        "ported_foreign::test_png_save_compression",
        "ported_foreign::test_png_save_interlace",
        "ported_foreign::test_png_save_palette",
        "ported_infrastructure::test_fd_leak_png",
    ]),
    ("sample.tif", &[
        "ported_foreign::test_tiff_bigtiff",
        "ported_foreign::test_tiff_load_dimensions",
        "ported_foreign::test_tiff_load_pixels",
        "ported_foreign::test_tiff_save_ccitt",
        "ported_foreign::test_tiff_save_deflate",
        "ported_foreign::test_tiff_save_jpeg",
        "ported_foreign::test_tiff_save_lzw",
        "ported_foreign::test_tiff_strip",
        "ported_foreign::test_tiffjp2k",
        "ported_infrastructure::test_fd_leak_tiff",
    ]),
    ("silicongraphics.sgi", &[
        "ported_foreign::test_magickload",
    ]),
    ("subsampled.tif", &[
        "ported_foreign::test_tiff_subsampled",
    ]),
    ("targa.tga", &[
        "ported_foreign::test_magickload",
    ]),
    ("trans-x.gif", &[
        "ported_foreign::test_gifload",
        "ported_foreign::test_gifsave",
        "ported_foreign::test_magicksave",
    ]),
    ("truncated.gif", &[
        "ported_foreign::test_gifload_truncated",
    ]),
    ("truncated.jpg", &[
        "ported_foreign::test_truncated",
    ]),
    ("ultra-hdr.jpg", &[
        "ported_foreign::test_uhdrload",
        "ported_foreign::test_uhdr_dzsave",
        "ported_foreign::test_uhdr_thumbnail",
        "ported_foreign::test_uhdr_thumbnail_crop",
        "ported_foreign::test_uhdrsave",
        "ported_foreign::test_uhdrsave_gainmap_scale_factor",
        "ported_foreign::test_uhdrsave_roundtrip",
        "ported_foreign::test_uhdrsave_roundtrip_hdr",
        "ported_resample::test_thumbnail_uhdr_linear",
    ]),
    ("world.jp2", &[
        "ported_foreign::test_jp2kload",
    ]),
];

// ---------------------------------------------------------------------------
// Individual existence tests — one per fixture file
// ---------------------------------------------------------------------------

macro_rules! fixture_exists_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            let path = ref_image($file);
            assert!(
                path.exists(),
                "fixture missing: {} (expected at {})",
                $file,
                path.display()
            );
        }
    };
}

fixture_exists_test!(fixture_marbles_bmp, "MARBLES.BMP");
fixture_exists_test!(fixture_1_webp, "1.webp");
fixture_exists_test!(fixture_cmu_svs, "CMU-1-Small-Region.svs");
fixture_exists_test!(fixture_iso_12233_pdf, "ISO_12233-reschart.pdf");
fixture_exists_test!(fixture_wfpc2_fits, "WFPC2u5780205r_c0fx.fits");
fixture_exists_test!(fixture_nifti, "avg152T1_LR_nifti.nii.gz");
fixture_exists_test!(fixture_avif_orientation_6, "avif-orientation-6.avif");
fixture_exists_test!(fixture_cd1_1_jpg, "cd1.1.jpg");
fixture_exists_test!(fixture_cd1_2_jpg, "cd1.2.jpg");
fixture_exists_test!(fixture_cd2_1_jpg, "cd2.1.jpg");
fixture_exists_test!(fixture_cd2_2_jpg, "cd2.2.jpg");
fixture_exists_test!(fixture_cd3_1_jpg, "cd3.1.jpg");
fixture_exists_test!(fixture_cd3_2_jpg, "cd3.2.jpg");
fixture_exists_test!(fixture_cd4_1_jpg, "cd4.1.jpg");
fixture_exists_test!(fixture_cd4_2_jpg, "cd4.2.jpg");
fixture_exists_test!(fixture_cmyktest_pdf, "cmyktest.pdf");
fixture_exists_test!(fixture_cogs_gif, "cogs.gif");
fixture_exists_test!(fixture_dicom, "dicom_test_image.dcm");
fixture_exists_test!(fixture_dispose_background_gif, "dispose-background.gif");
fixture_exists_test!(fixture_dispose_background_png, "dispose-background.png");
fixture_exists_test!(fixture_dispose_previous_gif, "dispose-previous.gif");
fixture_exists_test!(fixture_dispose_previous_png, "dispose-previous.png");
fixture_exists_test!(fixture_favicon_ico, "favicon.ico");
fixture_exists_test!(fixture_garden_gif, "garden.gif");
fixture_exists_test!(fixture_indexed_png, "indexed.png");
fixture_exists_test!(fixture_logo_svg, "logo.svg");
fixture_exists_test!(fixture_ome_tif, "multi-channel-z-series.ome.tif");
fixture_exists_test!(fixture_ojpeg_strip_tif, "ojpeg-strip.tif");
fixture_exists_test!(fixture_ojpeg_tile_tif, "ojpeg-tile.tif");
fixture_exists_test!(fixture_rgba_correct_ppm, "rgba-correct.ppm");
fixture_exists_test!(fixture_rgba_png, "rgba.png");
fixture_exists_test!(fixture_srgb_icm, "sRGB.icm");
fixture_exists_test!(fixture_sample_cur, "sample.cur");
fixture_exists_test!(fixture_sample_xyb_jpg, "sample-xyb.jpg");
fixture_exists_test!(fixture_sample_exr, "sample.exr");
fixture_exists_test!(fixture_sample_hdr, "sample.hdr");
fixture_exists_test!(fixture_sample_jpg, "sample.jpg");
fixture_exists_test!(fixture_sample_mat, "sample.mat");
fixture_exists_test!(fixture_sample_png, "sample.png");
fixture_exists_test!(fixture_sample_tif, "sample.tif");
fixture_exists_test!(fixture_silicongraphics_sgi, "silicongraphics.sgi");
fixture_exists_test!(fixture_subsampled_tif, "subsampled.tif");
fixture_exists_test!(fixture_targa_tga, "targa.tga");
fixture_exists_test!(fixture_trans_x_gif, "trans-x.gif");
fixture_exists_test!(fixture_truncated_gif, "truncated.gif");
fixture_exists_test!(fixture_truncated_jpg, "truncated.jpg");
fixture_exists_test!(fixture_ultra_hdr_jpg, "ultra-hdr.jpg");
fixture_exists_test!(fixture_world_jp2, "world.jp2");

// ---------------------------------------------------------------------------
// Summary test — fails with a full report if anything is missing
// ---------------------------------------------------------------------------

#[test]
fn fixture_audit_summary() {
    let mut missing: Vec<(&str, &[&str])> = Vec::new();

    for &(file, tests) in FIXTURE_DEPS {
        if !ref_image(file).exists() {
            missing.push((file, tests));
        }
    }

    if missing.is_empty() {
        return;
    }

    let total_files = missing.len();
    let total_tests: usize = missing.iter().map(|(_, t)| t.len()).sum();

    let mut report = format!(
        "\n\n=== FIXTURE AUDIT FAILED ===\n\
         {total_files} fixture file(s) missing, blocking {total_tests} test(s).\n\
         Reference images dir: {REF_IMAGES}\n\n"
    );

    for (file, tests) in &missing {
        report.push_str(&format!("  MISSING: {file}\n"));
        report.push_str(&format!("    used by {} test(s):\n", tests.len()));
        for t in *tests {
            report.push_str(&format!("      - {t}\n"));
        }
        report.push('\n');
    }

    report.push_str("To fix: add the missing files to the images directory,\n\
                      or update the Rust test to use the correct filename.\n\
                      See TESTS_PLAN.md § 'Missing Test Fixtures' for known renames.\n");

    panic!("{report}");
}
