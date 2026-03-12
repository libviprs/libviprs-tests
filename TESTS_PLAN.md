# Tests Comparison: libvips vs libviprs

## Overall Status

**No tests are missing.** Every Python test in the libvips test suite has a corresponding Rust test in libviprs — either as a direct 1:1 name match, or as documented granular sub-tests of a monolithic libvips test.

| Category | Count |
|---|---|
| Files with exact 1:1 test names | 10 of 13 (arithmetic, connection, conversion, convolution, create, draw, histogram, iofuncs, morphology, mosaicing) |
| Files with documented splits of monolithic tests | 3 (colour, foreign, resample) |
| Rust-only extras (no Python equivalent) | 3 tests (`test_bmp`, `test_shrink_average`, `test_lab_xyz_reference`) |
| Name casing differences (documented) | 4 (`test_de00`/`dE00`, `test_de76`/`dE76`, `test_decmc`/`dECMC`, `test_avifsave_q`/`Q`) |
| Fixtures | All 74 shared; 2 libviprs-only PDFs |

## Test File Mapping

| libvips (Python) | libviprs (Rust) | Recommended Change |
|---|---|---|
| test_arithmetic.py | ported_arithmetic.rs | — |
| test_colour.py | ported_colour.rs | — |
| test_connection.py | ported_connection.rs | — |
| test_conversion.py | ported_conversion.rs | — |
| test_convolution.py | ported_convolution.rs | — |
| test_create.py | ported_create.rs | — |
| test_draw.py | ported_draw.rs | — |
| test_foreign.py | ported_foreign.rs | — |
| test_histogram.py | ported_histogram.rs | — |
| test_iofuncs.py | ported_iofuncs.rs | Done — all 5 tests ported 1:1 |
| test_morphology.py | ported_morphology.rs | — |
| test_mosaicing.py | ported_mosaicing.rs | — |
| test_resample.py | ported_resample.rs | — |
| *(C tests: test_connections.c, test_descriptors.c, test_timeout_gifsave.c, test_timeout_webpsave.c, test_token.c)* | ported_infrastructure.rs | Tests from multiple C sources consolidated into one file |

---

## Test Name Comparison by File

### arithmetic (64 tests — exact match)

All 64 test names match 1:1 between `test_arithmetic.py` and `ported_arithmetic.rs`:
`test_add`, `test_sub`, `test_mul`, `test_div`, `test_floordiv`, `test_pow`, `test_and`, `test_or`, `test_xor`, `test_more`, `test_moreeq`, `test_less`, `test_lesseq`, `test_equal`, `test_noteq`, `test_abs`, `test_lshift`, `test_rshift`, `test_mod`, `test_pos`, `test_neg`, `test_invert`, `test_avg`, `test_deviate`, `test_polar`, `test_rect`, `test_conjugate`, `test_histfind`, `test_histfind_indexed`, `test_histfind_ndim`, `test_hough_circle`, `test_hough_line`, `test_sin`, `test_cos`, `test_tan`, `test_asin`, `test_acos`, `test_atan`, `test_sinh`, `test_cosh`, `test_tanh`, `test_asinh`, `test_acosh`, `test_atanh`, `test_atan2`, `test_log`, `test_log10`, `test_exp`, `test_exp10`, `test_floor`, `test_ceil`, `test_rint`, `test_sign`, `test_max`, `test_min`, `test_measure`, `test_find_trim`, `test_profile`, `test_project`, `test_stats`, `test_sum`, `test_clamp`, `test_minpair`, `test_maxpair`

### colour (6 vs 9 — name mismatches + extras in libviprs)

| libvips | libviprs | Recommended Change |
|---|---|---|
| test_colourspace | test_colourspace_roundtrip | Rename to `test_colourspace` or keep split |
| *(part of test_colourspace)* | test_colourspace_mono | Keep (sub-test split from test_colourspace) |
| *(part of test_colourspace)* | test_colourspace_cmyk | Keep (sub-test split from test_colourspace) |
| — | test_lab_xyz_reference | Keep (extra validation, no libvips equivalent) |
| test_dE00 | test_de00 | Rename to `test_dE00` to match libvips casing |
| test_dE76 | test_de76 | Rename to `test_dE76` to match libvips casing |
| test_dECMC | test_decmc | Rename to `test_dECMC` to match libvips casing |
| test_icc | test_icc | — |
| test_cmyk | test_cmyk | — |

### connection (14 tests — exact 1:1 match)

All 14 test names match 1:1 between `test_connection.py` and `ported_connection.rs`:
`test_source_new_from_file`, `test_image_new_from_source_file`, `test_target_new_to_file`, `test_image_write_to_target_file`, `test_source_new_memory`, `test_image_new_from_source_memory`, `test_target_new_memory`, `test_image_write_to_target_memory`, `test_connection_matrix`, `test_connection_svg`, `test_connection_csv`, `test_connection_ppm`, `test_connection_tiff`, `test_connection_dz`

Note: `test_get_fields` and `test_revalidate` were moved to `ported_iofuncs.rs` to match `test_iofuncs.py`.

### conversion (44 tests — exact match)

All 44 test names match 1:1 between `test_conversion.py` and `ported_conversion.rs`:
`test_cast`, `test_band_and`, `test_band_or`, `test_band_eor`, `test_bandjoin`, `test_bandjoin_const`, `test_addalpha`, `test_bandmean`, `test_bandrank`, `test_copy`, `test_bandfold`, `test_byteswap`, `test_embed`, `test_gravity`, `test_extract`, `test_slice`, `test_crop`, `test_smartcrop`, `test_smartcrop_attention`, `test_smartcrop_rgba`, `test_smartcrop_rgba_premultiplied`, `test_falsecolour`, `test_flatten`, `test_premultiply`, `test_composite`, `test_composite_non_separable`, `test_unpremultiply`, `test_flip`, `test_gamma`, `test_grid`, `test_ifthenelse`, `test_switch`, `test_insert`, `test_arrayjoin`, `test_msb`, `test_recomb`, `test_replicate`, `test_rot45`, `test_rot`, `test_autorot`, `test_scaleimage`, `test_subsample`, `test_zoom`, `test_wrap`

### convolution (7 tests — exact match)

All 7 test names match 1:1: `test_conv`, `test_compass`, `test_convsep`, `test_fastcor`, `test_spcor`, `test_gaussblur`, `test_sharpen`

### create (30 tests — exact match)

All 30 test names match 1:1: `test_black`, `test_buildlut`, `test_eye`, `test_fwfft_small_image`, `test_fractsurf`, `test_gaussmat`, `test_gaussnoise`, `test_grey`, `test_identity`, `test_invertlut`, `test_matrixinvert`, `test_logmat`, `test_mask_butterworth`, `test_mask_butterworth_band`, `test_mask_butterworth_ring`, `test_mask_fractal`, `test_mask_gaussian`, `test_mask_gaussian_band`, `test_mask_gaussian_ring`, `test_mask_gaussian_ring_2`, `test_mask_ideal`, `test_mask_ideal_band`, `test_sines`, `test_text`, `test_tonelut`, `test_xyz`, `test_sdf`, `test_zone`, `test_worley`, `test_perlin`

### draw (8 tests — exact match)

All 8 test names match 1:1: `test_draw_circle`, `test_draw_flood`, `test_draw_flood_out_of_bounds`, `test_draw_image`, `test_draw_line`, `test_draw_mask`, `test_draw_rect`, `test_draw_smudge`

### foreign (56 vs 106 — monolithic tests split into granular tests)

| libvips | libviprs | Recommended Change |
|---|---|---|
| test_jpeg | test_jpeg_load_dimensions | Keep (granular split of monolithic test) |
| *(part of test_jpeg)* | test_jpeg_load_pixel_values | Keep |
| *(part of test_jpeg)* | test_jpeg_load_from_memory | Keep |
| *(part of test_jpeg)* | test_jpeg_shrink_on_load | Keep |
| *(part of test_jpeg)* | test_jpeg_sequential | Keep |
| *(part of test_jpeg)* | test_jpeg_autorot | Keep |
| *(part of test_jpeg)* | test_jpeg_save_quality | Keep |
| *(part of test_jpeg)* | test_jpeg_save_icc | Keep |
| *(part of test_jpeg)* | test_jpeg_save_exif | Keep |
| *(part of test_jpeg)* | test_jpeg_save_subsample | Keep |
| test_jpegsave | test_jpegsave | — (1:1 port added) |
| test_jpegsave_exif | test_jpegsave_exif | — |
| test_jpegsave_exif_2_3_ascii | test_jpegsave_exif_2_3_ascii | — |
| test_jpegsave_exif_2_3_ascii_2 | test_jpegsave_exif_2_3_ascii_2 | — |
| test_truncated | test_truncated | — |
| test_png | test_png_load_dimensions | Keep (granular split) |
| *(part of test_png)* | test_png_load_8bit | Keep |
| *(part of test_png)* | test_png_load_16bit_reference | Keep |
| *(part of test_png)* | test_png_load_16bit | Keep |
| *(part of test_png)* | test_png_load_palette | Keep |
| *(part of test_png)* | test_png_load_rgba | Keep |
| *(part of test_png)* | test_png_load_interlaced | Keep |
| *(part of test_png)* | test_png_save_compression | Keep |
| *(part of test_png)* | test_png_save_interlace | Keep |
| *(part of test_png)* | test_png_save_palette | Keep |
| *(part of test_png)* | test_png_icc | Keep |
| *(part of test_png)* | test_png_exif | Keep |
| test_tiff | test_tiff_load_dimensions | Keep (granular split) |
| *(part of test_tiff)* | test_tiff_load_pixels | Keep |
| *(part of test_tiff)* | test_tiff_strip | Keep |
| *(part of test_tiff)* | test_tiff_tile | Keep |
| *(part of test_tiff)* | test_tiff_low_bitdepth | Keep |
| *(part of test_tiff)* | test_tiff_subsampled | Keep |
| *(part of test_tiff)* | test_tiff_multipage | Keep |
| *(part of test_tiff)* | test_tiff_save_lzw | Keep |
| *(part of test_tiff)* | test_tiff_save_jpeg | Keep |
| *(part of test_tiff)* | test_tiff_save_deflate | Keep |
| *(part of test_tiff)* | test_tiff_save_ccitt | Keep |
| *(part of test_tiff)* | test_tiff_bigtiff | Keep |
| test_tiff_ojpeg | test_tiff_ojpeg | — |
| test_tiffjp2k | test_tiffjp2k | — |
| test_pdfload | test_pdf_page_count | Keep (granular split) |
| *(part of test_pdfload)* | test_pdf_page_dimensions | Keep |
| *(part of test_pdfload)* | test_pdf_extract_image | Keep |
| *(part of test_pdfload)* | test_pdf_page_select | Keep |
| *(part of test_pdfload)* | test_pdf_cmyk | Keep |
| *(part of test_pdfload)* | test_pdf_reference_reschart | Keep |
| *(part of test_pdfload)* | test_pdf_dpi_scale | Keep |
| *(part of test_pdfload)* | test_pdf_background | Keep |
| *(part of test_pdfload)* | test_pdf_password | Keep |
| test_dzsave | test_dz_tile_size | Keep (granular split) |
| *(part of test_dzsave)* | test_dz_overlap | Keep |
| *(part of test_dzsave)* | test_dz_layout_deepzoom | Keep |
| *(part of test_dzsave)* | test_dz_layout_xyz | Keep |
| *(part of test_dzsave)* | test_dz_format_png | Keep |
| *(part of test_dzsave)* | test_dz_format_jpeg | Keep |
| *(part of test_dzsave)* | test_dz_layout_zoomify | Keep |
| *(part of test_dzsave)* | test_dz_layout_iiif | Keep |
| *(part of test_dzsave)* | test_dz_zip | Keep |
| *(part of test_dzsave)* | test_dz_skip_blanks | Keep |
| *(part of test_dzsave)* | test_dz_properties | Keep |
| *(part of test_dzsave)* | test_dz_region | Keep |
| test_vips | test_vips | — |
| test_webp | test_webp | — |
| test_gifload | test_gifload | — |
| test_gifload_animation_dispose_background | test_gifload_animation_dispose_background | — |
| test_gifload_animation_dispose_previous | test_gifload_animation_dispose_previous | — |
| test_gifload_truncated | test_gifload_truncated | — |
| test_gifload_frame_error | test_gifload_frame_error | — |
| test_gifsave | test_gifsave | — |
| test_heifload | test_heifload | — |
| test_avifsave | test_avifsave | — |
| test_avifsave_lossless | test_avifsave_lossless | — |
| test_avifsave_Q | test_avifsave_q | Rename to `test_avifsave_Q` to match libvips casing |
| test_avifsave_chroma | test_avifsave_chroma | — |
| test_avifsave_icc | test_avifsave_icc | — |
| test_avifsave_exif | test_avifsave_exif | — |
| test_avifsave_tune | test_avifsave_tune | — |
| test_heicsave_16_to_12 | test_heicsave_16_to_12 | — |
| test_heicsave_16_to_8 | test_heicsave_16_to_8 | — |
| test_heicsave_8_to_16 | test_heicsave_8_to_16 | — |
| test_jp2kload | test_jp2kload | — |
| test_jp2ksave | test_jp2ksave | — |
| test_jxlsave | test_jxlsave | — |
| test_svgload | test_svgload | — |
| test_fitsload | test_fitsload | — |
| test_openexrload | test_openexrload | — |
| test_openslideload | test_openslideload | — |
| test_matload | test_matload | — |
| test_analyzeload | test_analyzeload | — |
| test_niftiload | test_niftiload | — |
| test_magickload | test_magickload | — |
| test_magicksave | test_magicksave | — |
| test_ppm | test_ppm | — |
| test_rad | test_rad | — |
| test_csv | test_csv | — |
| test_matrix | test_matrix | — |
| test_uhdrload | test_uhdrload | — |
| test_uhdrsave | test_uhdrsave | — |
| test_uhdrsave_roundtrip | test_uhdrsave_roundtrip | — |
| test_uhdrsave_roundtrip_hdr | test_uhdrsave_roundtrip_hdr | — |
| test_uhdrsave_gainmap_scale_factor | test_uhdrsave_gainmap_scale_factor | — |
| test_uhdr_thumbnail | test_uhdr_thumbnail | — |
| test_uhdr_thumbnail_crop | test_uhdr_thumbnail_crop | — |
| test_uhdr_dzsave | test_uhdr_dzsave | — |
| test_fail_on | test_fail_on | — |
| — | test_bmp | No libvips equivalent; keep as extra |

### histogram (12 tests — exact match)

All 12 test names match 1:1: `test_hist_cum`, `test_hist_equal`, `test_hist_ismonotonic`, `test_hist_local`, `test_hist_match`, `test_hist_norm`, `test_hist_plot`, `test_hist_map`, `test_percent`, `test_hist_entropy`, `test_stdif`, `test_case`

### iofuncs (5 tests — exact 1:1 match, now in ported_iofuncs.rs)

| libvips (test_iofuncs.py) | libviprs (ported_iofuncs.rs) | Status |
|---|---|---|
| test_new_from_image | test_new_from_image | Done (moved from ported_infrastructure.rs) |
| test_new_from_memory | test_new_from_memory | Done (added) |
| test_get_fields | test_get_fields | Done (moved from ported_connection.rs) |
| test_write_to_memory | test_write_to_memory | Done (added) |
| test_revalidate | test_revalidate | Done (moved from ported_connection.rs) |

### infrastructure (C test ports, in ported_infrastructure.rs — no libvips Python equivalent)

| Source | libviprs | Notes |
|---|---|---|
| test_keep.sh | test_keep_icc | — |
| test_keep.sh | test_keep_xmp | — |
| test_keep.sh | test_keep_none | — |
| test_keep.sh | test_keep_custom_profile | — |
| test_threading.sh | test_threading_consistency | — |
| test_threading.sh | test_max_threads | — |
| test_seq.sh | test_seq_thumbnail | — |
| test_seq.sh | test_seq_no_temps | — |
| test_seq.sh | test_seq_shrink_no_temps | — |
| test_descriptors.c | test_fd_leak_jpeg | — |
| test_descriptors.c | test_fd_leak_png | — |
| test_descriptors.c | test_fd_leak_tiff | — |
| test_stall.sh | test_pipeline_stall | — |
| (manual) | test_progress_cancel | — |
| test_timeout_gifsave.c | test_timeout_gifsave | — |
| test_timeout_webpsave.c | test_timeout_webpsave | — |
| test_token.c | test_token_parsing | — |
| test_cli.sh | test_cli_thumbnail | — |
| test_cli.sh | test_cli_rotate | — |
| test_cli.sh | test_cli_max_coord_flag | — |
| test_cli.sh | test_cli_max_coord_env | — |

### morphology (5 tests — exact match)

All 5 test names match 1:1: `test_countlines`, `test_labelregions`, `test_erode`, `test_dilate`, `test_rank`

### mosaicing (6 tests — exact match)

All 6 test names match 1:1: `test_lrmerge`, `test_tbmerge`, `test_lrmosaic`, `test_tbmosaic`, `test_mosaic`, `test_globalbalance`

### resample (11 vs 13 — splits + extras)

| libvips | libviprs | Recommended Change |
|---|---|---|
| test_resize | test_resize_quarter | Rename to `test_resize` or keep split |
| *(part of test_resize)* | test_resize_rounding | Keep (sub-test split from test_resize) |
| test_shrink | test_shrink | — |
| — | test_shrink_average | Keep as extra coverage |
| test_affine | test_affine | — |
| test_reduce | test_reduce | — |
| test_thumbnail | test_thumbnail | — |
| test_thumbnail_icc | test_thumbnail_icc | — |
| test_thumbnail_uhdr_linear | test_thumbnail_uhdr_linear | — |
| test_similarity | test_similarity | — |
| test_similarity_scale | test_similarity_scale | — |
| test_rotate | test_rotate | — |
| test_mapim | test_mapim | — |

---

## Fixture Comparison

### Shared fixtures (in both libvips `test/test-suite/images/` and libviprs `tmp/libvips-reference-tests/test-suite/images/`)

All 74 files + rotation/ directory are identical between the two locations:

| Fixture | In libvips | In libviprs (tmp copy) | Recommended Change |
|---|---|---|---|
| 1.webp | Yes | Yes | — |
| 17000x17000.avif | Yes | Yes | — |
| 1bit.tif | Yes | Yes | — |
| 2bit.tif | Yes | Yes | — |
| 4bit.tif | Yes | Yes | — |
| avg152T1_LR_nifti.nii.gz | Yes | Yes | — |
| avif-orientation-6.avif | Yes | Yes | — |
| big-height.webp | Yes | Yes | — |
| blankpage.pdf | Yes | Yes | — |
| blankpage.pdf.png | Yes | Yes | — |
| blankpage.svg | Yes | Yes | — |
| blankpage.svg.png | Yes | Yes | — |
| Bretagne2_1.j2k | Yes | Yes | — |
| Bretagne2_4.j2k | Yes | Yes | — |
| cd1.1.jpg | Yes | Yes | — |
| cd1.2.jpg | Yes | Yes | — |
| cd2.1.jpg | Yes | Yes | — |
| cd2.2.jpg | Yes | Yes | — |
| cd3.1.jpg | Yes | Yes | — |
| cd3.2.jpg | Yes | Yes | — |
| cd4.1.jpg | Yes | Yes | — |
| cd4.2.jpg | Yes | Yes | — |
| CMU-1-Small-Region.svs | Yes | Yes | — |
| cmyktest.pdf | Yes | Yes | — |
| cogs.gif | Yes | Yes | — |
| cogs.png | Yes | Yes | — |
| cramps.gif | Yes | Yes | — |
| dicom_test_image.dcm | Yes | Yes | — |
| dispose-background.gif | Yes | Yes | — |
| dispose-background.png | Yes | Yes | — |
| dispose-previous.gif | Yes | Yes | — |
| dispose-previous.png | Yes | Yes | — |
| favicon.ico | Yes | Yes | — |
| garden.gif | Yes | Yes | — |
| indexed.png | Yes | Yes | — |
| invalid_multiframe.gif | Yes | Yes | — |
| invisible.ico | Yes | Yes | — |
| ISO_12233-reschart.pdf | Yes | Yes | — |
| issue412.jp2 | Yes | Yes | — |
| logo.svg | Yes | Yes | — |
| logo.svg.gz | Yes | Yes | — |
| logo.svgz | Yes | Yes | — |
| looks-like-svg.webp | Yes | Yes | — |
| MARBLES.BMP | Yes | Yes | — |
| multi-channel-z-series.ome.tif | Yes | Yes | — |
| ojpeg-strip.tif | Yes | Yes | — |
| ojpeg-tile.tif | Yes | Yes | — |
| page-box.pdf | Yes | Yes | — |
| rgba-correct.ppm | Yes | Yes | — |
| rgba.png | Yes | Yes | — |
| rotation/ (directory with 0.png, 1-8.jpg) | Yes | Yes | — |
| sample-xyb.jpg | Yes | Yes | — |
| sample.cur | Yes | Yes | — |
| sample.exr | Yes | Yes | — |
| sample.hdr | Yes | Yes | — |
| sample.jpg | Yes | Yes | — |
| sample.mat | Yes | Yes | — |
| sample.png | Yes | Yes | — |
| sample.tif | Yes | Yes | — |
| silicongraphics.sgi | Yes | Yes | — |
| small.bmp | Yes | Yes | — |
| sRGB.icm | Yes | Yes | — |
| subsampled.tif | Yes | Yes | — |
| t00740_tr1_segm.hdr | Yes | Yes | — |
| t00740_tr1_segm.img | Yes | Yes | — |
| targa.tga | Yes | Yes | — |
| trans-x.gif | Yes | Yes | — |
| trans-x.png | Yes | Yes | — |
| truncated.gif | Yes | Yes | — |
| truncated.jpg | Yes | Yes | — |
| truncated.svgz | Yes | Yes | — |
| ultra-hdr.jpg | Yes | Yes | — |
| WFPC2u5780205r_c0fx.fits | Yes | Yes | — |
| world.jp2 | Yes | Yes | — |

### libviprs-only fixtures (in `libviprs-tests/tests/fixtures/`)

| Fixture | In libvips | In libviprs | Recommended Change |
|---|---|---|---|
| blueprint.pdf | No | Yes | Keep (used by pdf_ops tests) |
| password.pdf | No | Yes | Keep (used by test_pdf_password) |

### Summary

- **Shared fixtures**: 74 files + rotation/ directory — exact mirror, no changes needed
- **libviprs-only fixtures**: 2 PDF files for PDF-specific tests — keep as-is
- **libvips-only fixtures**: None (all libvips fixtures are copied to libviprs tmp/)
