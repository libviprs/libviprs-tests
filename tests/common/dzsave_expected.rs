//! Tile-pyramid comparison helpers for the ported `dzsave` cells.
//!
//! The ported foreign-format tests that enable a DeepZoom layout / sink
//! feature compare libviprs output against a **vips-generated expected
//! fixture** committed under `tests/fixtures/<feature>_expected/`. The
//! reference is produced offline with `vips dzsave` (see each fixture's
//! generation command in `tests/fixtures/README.md`) and checked in, so the
//! tests need no libvips at runtime — a local pre-commit run and CI see the
//! identical committed bytes.
//!
//! This mirrors the tile-diff helpers already used by
//! `blueprint_portrait_pyramid.rs` / `blueprint_mix_pyramid.rs`, centralised
//! here because the six `ported_foreign` enablements all live in one file.

use std::path::Path;

/// Recursively collect `ext` files under `dir` as `(relative_path, bytes)`,
/// sorted by relative path so two trees line up positionally.
pub fn collect_files(dir: &Path, ext: &str) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    collect_files_recursive(dir, dir, ext, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn collect_files_recursive(root: &Path, dir: &Path, ext: &str, out: &mut Vec<(String, Vec<u8>)>) {
    if !dir.is_dir() {
        return;
    }
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_files_recursive(root, &path, ext, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            let bytes = std::fs::read(&path).unwrap();
            out.push((rel, bytes));
        }
    }
}

/// Decode a PNG to `(width, height, raw_pixels)` so comparisons are at the
/// pixel level (differences in PNG encoding never cause failures).
pub fn decode_png(data: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = image::load_from_memory_with_format(data, image::ImageFormat::Png)
        .expect("failed to decode PNG tile");
    let w = img.width();
    let h = img.height();
    (w, h, img.into_bytes())
}

/// Compare two sets of PNG tiles by decoded pixels.
///
/// vips emits edge tiles at their natural (unpadded) size; libviprs pads edge
/// tiles to `tile_size` with white background. This compares the overlapping
/// region within `tolerance` per channel (rounding in the downscaler differs
/// between libvips area-averaging and libviprs), and asserts any libviprs
/// padding is solid white. Returns the maximum per-channel difference seen so
/// callers can log/calibrate.
pub fn assert_tiles_pixel_equal_tol(
    expected_files: &[(String, Vec<u8>)],
    actual_files: &[(String, Vec<u8>)],
    context: &str,
    tolerance: u8,
) -> u8 {
    assert_tiles_within(
        expected_files,
        actual_files,
        context,
        -i16::from(tolerance),
        i16::from(tolerance),
    )
}

/// Compare two sets of PNG tiles from an **RGBA** source, where libviprs is
/// allowed to sit exactly one count ABOVE the vips reference and never below.
///
/// This is not a slackened tolerance, it is the signature of one named
/// divergence, and a sample on the wrong side of it still fails.
///
/// `dzsave` builds each pyramid level with `vips_region_shrink`
/// (`libvips/iofuncs/region.c`). On an image with an alpha band that dispatches
/// to `vips_region_shrink_alpha`, whose `SHRINK_ALPHA_TYPE` macro forms the
/// alpha-weighted colour mean and the averaged alpha in `double` and then
/// stores them through a C cast to the integer sample type, which truncates:
/// `tq[z] = (a1*tp[z] + a2*tp[z+nb] + a3*tp1[z] + a4*tp1[z+nb]) / a;` and
/// `tq[nb - 1] = a / 4;`. Its own no-alpha twin `SHRINK_TYPE_INT` rounds
/// instead, `tq[z] = (tot + 2) >> 2`, so inside libvips a fully opaque RGBA
/// image shrinks half a count darker than its RGB twin. libviprs closed that
/// gap on purpose: `downscale_half_alpha` rounds half-up on both branches
/// (libviprs/libviprs#458, under "Fixed" in that crate's CHANGELOG — "so a
/// fully-opaque RGBA image downscales bit-identically to its RGB twin ...
/// instead of carrying the systematic -0.5 LSB truncation bias").
///
/// So for every shrunk sample vips holds `floor(v)` and libviprs holds
/// `floor(v)` or `floor(v) + 1`: the difference is one-sided and at most one
/// count. Full-resolution tiles are copies, not shrinks, and stay bit-exact.
///
/// Measured on `canonical_input.png` (256x256 RGBA, fully opaque, tile 128,
/// all three layouts plus the ZIP DeepZoom tree): level 8 is bit-identical,
/// every shrunk level lands in `[0, +1]` — 320 differing samples out of 327 680
/// at level 7, halving per level down to 3 at the 1x1 level.
pub fn assert_tiles_pixel_equal_shrink_round_up(
    expected_files: &[(String, Vec<u8>)],
    actual_files: &[(String, Vec<u8>)],
    context: &str,
) -> u8 {
    assert_tiles_within(expected_files, actual_files, context, 0, 1)
}

/// Shared comparison core: every overlapping sample must satisfy
/// `min_diff <= libviprs - vips <= max_diff`, and any libviprs padding beyond
/// the vips tile must be solid white. Returns the largest absolute difference
/// seen.
fn assert_tiles_within(
    expected_files: &[(String, Vec<u8>)],
    actual_files: &[(String, Vec<u8>)],
    context: &str,
    min_diff: i16,
    max_diff: i16,
) -> u8 {
    assert_eq!(
        expected_files.len(),
        actual_files.len(),
        "{context}: tile count mismatch: vips {}, libviprs {}",
        expected_files.len(),
        actual_files.len(),
    );

    let mut global_max_diff: u8 = 0;
    let mut max_diff_path = String::new();

    for ((exp_path, exp_bytes), (act_path, act_bytes)) in
        expected_files.iter().zip(actual_files.iter())
    {
        assert_eq!(
            exp_path, act_path,
            "{context}: tile path mismatch: vips {exp_path}, libviprs {act_path}"
        );
        let (ew, eh, epx) = decode_png(exp_bytes);
        let (aw, ah, apx) = decode_png(act_bytes);

        assert!(
            aw >= ew && ah >= eh,
            "{context}: libviprs tile {act_path} ({aw}x{ah}) smaller than vips ({ew}x{eh})"
        );

        let exp_channels = epx.len() / (ew as usize * eh as usize);
        let act_channels = apx.len() / (aw as usize * ah as usize);
        assert_eq!(
            exp_channels, act_channels,
            "{context}: channel-count mismatch at {act_path}: vips {exp_channels}, libviprs {act_channels}"
        );
        let ch = exp_channels;

        for y in 0..eh as usize {
            let exp_row = y * ew as usize * ch;
            let act_row = y * aw as usize * ch;
            for x in 0..(ew as usize * ch) {
                let ev = epx[exp_row + x];
                let av = apx[act_row + x];
                let signed = i16::from(av) - i16::from(ev);
                let diff = signed.unsigned_abs() as u8;
                if diff > global_max_diff {
                    global_max_diff = diff;
                    max_diff_path = act_path.clone();
                }
                assert!(
                    (min_diff..=max_diff).contains(&signed),
                    "{context}: pixel mismatch at {act_path} row={y} col={} \
                     vips={ev} libviprs={av} diff={signed} allowed={min_diff}..={max_diff}",
                    x / ch
                );
            }
        }

        // Any libviprs padding beyond the vips region must be solid white.
        if aw > ew {
            for y in 0..ah as usize {
                let pad_start = y * aw as usize * ch + ew as usize * ch;
                let pad_end = y * aw as usize * ch + aw as usize * ch;
                assert!(
                    apx[pad_start..pad_end].iter().all(|&b| b == 255),
                    "{context}: right padding not white at row {y} in {act_path}"
                );
            }
        }
        if ah > eh {
            for y in eh as usize..ah as usize {
                let row_start = y * aw as usize * ch;
                let row_end = row_start + aw as usize * ch;
                assert!(
                    apx[row_start..row_end].iter().all(|&b| b == 255),
                    "{context}: bottom padding not white at row {y} in {act_path}"
                );
            }
        }
    }

    eprintln!(
        "{context}: max pixel diff = {global_max_diff} (at {max_diff_path}), \
         allowed = {min_diff}..={max_diff}"
    );
    global_max_diff
}
