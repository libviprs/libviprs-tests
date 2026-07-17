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
                let diff = (ev as i16 - av as i16).unsigned_abs() as u8;
                if diff > global_max_diff {
                    global_max_diff = diff;
                    max_diff_path = act_path.clone();
                }
                assert!(
                    diff <= tolerance,
                    "{context}: pixel mismatch at {act_path} row={y} col={} \
                     vips={ev} libviprs={av} diff={diff} tolerance={tolerance}",
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
        "{context}: max pixel diff = {global_max_diff} (at {max_diff_path}), tolerance = {tolerance}"
    );
    global_max_diff
}
