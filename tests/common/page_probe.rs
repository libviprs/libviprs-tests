//! Landmark probing shared by the library and CLI page-size pixel suites
//! (libviprs#1199): where the title-block square and the top-left triangle of
//! a generated sheet land in a raster, so a raster can be compared with the
//! libvips one by position as well as by value.

use super::blank_pdf::landmarks;

pub fn dark(rgb: &[u8], w: u32, x: u32, y: u32) -> bool {
    let i = ((y * w + x) * 3) as usize;
    rgb[i] < 128 && rgb[i + 1] < 128 && rgb[i + 2] < 128
}

/// Device position of page point `(x, y)`, as in `pdfium_page_size_exact.rs`.
pub fn device(
    rot: i64,
    (w, h): (f64, f64),
    (bw, bh): (f64, f64),
    (x, y): (f64, f64),
) -> (f64, f64) {
    match rot {
        0 => (x * bw / w, (h - y) * bh / h),
        90 => (y * bw / h, x * bh / w),
        180 => ((w - x) * bw / w, y * bh / h),
        270 => ((h - y) * bw / h, (w - x) * bh / w),
        _ => unreachable!(),
    }
}

/// The few pixels the landmark checks read: the title-block square's centre
/// `(px, py)`, and where the dark run in the top-left corner starts `(tx, ty)`.
#[derive(Clone, Copy)]
pub struct LandmarkPoints {
    pub px: u32,
    pub py: u32,
    pub tx: u32,
    pub ty: u32,
    pub w: u32,
    pub h: u32,
}

pub fn landmark_points(page: (f64, f64), rot: i64, (bw, bh): (u32, u32)) -> LandmarkPoints {
    let bm = (bw as f64, bh as f64);
    let lm = landmarks(page.0, page.1);
    let (cx, cy) = device(rot, page, bm, lm.square_centre);
    let (tx, ty) = device(rot, page, bm, (0.5, page.1 - lm.tri * 0.25));
    LandmarkPoints {
        px: cx.floor() as u32,
        py: cy.floor() as u32,
        tx: (tx.floor() as u32).min(bw - 1),
        ty: (ty.floor() as u32).min(bh - 1),
        w: bw,
        h: bh,
    }
}

/// Pixel extents of the title-block square along x and y, and the length of the
/// dark run at the left end of the row a quarter of the triangle down. `dark`
/// is only asked about row `py`, column `px` and row `ty`.
pub fn probe(
    dark: &dyn Fn(u32, u32) -> bool,
    pt: LandmarkPoints,
    label: &str,
) -> ((u32, u32), (u32, u32), u32) {
    let ext = |horizontal: bool| {
        let (mut a, mut b) = if horizontal {
            (pt.px, pt.px)
        } else {
            (pt.py, pt.py)
        };
        let limit = if horizontal { pt.w - 1 } else { pt.h - 1 };
        let d = |i: u32| {
            if horizontal {
                dark(i, pt.py)
            } else {
                dark(pt.px, i)
            }
        };
        assert!(d(a), "{label}: no square at its centre");
        while a > 0 && d(a - 1) {
            a -= 1;
        }
        while b < limit && d(b + 1) {
            b += 1;
        }
        (a, b)
    };
    let mut run = 0;
    let mut x = pt.tx;
    while x < pt.w && dark(x, pt.ty) {
        run += 1;
        x += 1;
    }
    (ext(true), ext(false), run)
}

/// Peak resident memory of this process so far, in MB (Linux only).
pub fn peak_rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(str::to_owned)
        })
        .and_then(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|n| n.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb / 1024)
}

struct Side {
    rows: std::collections::HashMap<u32, Vec<u8>>,
    col: Vec<bool>,
}

/// What [`compare_big`] found.
pub struct BigOutcome {
    pub stats: super::pixel_tolerance::PixelStats,
    /// sha256 of the decoded reference pixels.
    pub ref_hash: String,
    /// Every pixel of the raster under test had alpha 255 (always true for a
    /// 3-band raster, which has none to check).
    pub opaque: bool,
    /// Landmark extents `(square x, square y, triangle run)` in each raster.
    pub ours_landmarks: ((u32, u32), (u32, u32), u32),
    pub ref_landmarks: ((u32, u32), (u32, u32), u32),
}

/// Compare a raster under test with the stored libvips PNG at `ref_path` in
/// row chunks, so neither is held whole (the big sheets run to 400 MB). The
/// library suite and the CLI suite both come through here.
///
/// `fill(y0, n, buf)` must put the `n` rows starting at `y0` of the raster
/// under test into `buf` (cleared first), `ours_stride` samples per pixel
/// (3 for RGB, 4 for RGBA). The reference is read [`CHUNK_ROWS`] rows at a
/// time, and `fill` is asked for the same rows. One pass gathers the
/// statistics, the decoded-pixel hash, and the few rows and the column the
/// landmark probe reads on each side.
#[allow(clippy::too_many_arguments)] // two rasters, their geometry and the tolerance, all read-only
pub fn compare_big(
    label: &str,
    ref_path: &std::path::Path,
    page: (f64, f64),
    rot: i64,
    (w, h): (u32, u32),
    tol: &super::pixel_tolerance::PixelTolerance,
    ours_stride: usize,
    fill: &mut dyn FnMut(u32, u32, &mut Vec<u8>),
) -> BigOutcome {
    use super::libvips_reference::stream_png;
    use super::pixel_tolerance::StatsAcc;

    let pt = landmark_points(page, rot, (w, h));
    let capture = |side: &mut Side, y0: u32, n: u32, data: &[u8], stride: usize| {
        for r in 0..n {
            let y = y0 + r;
            let row =
                &data[(r as usize * w as usize * stride)..((r as usize + 1) * w as usize * stride)];
            let i = pt.px as usize * stride;
            side.col[y as usize] = row[i] < 128 && row[i + 1] < 128 && row[i + 2] < 128;
            if y == pt.py || y == pt.ty {
                side.rows.insert(y, row.to_vec());
            }
        }
    };
    let new_side = || Side {
        rows: Default::default(),
        col: vec![false; h as usize],
    };
    let (mut ours, mut theirs) = (new_side(), new_side());
    let mut acc = StatsAcc::default();
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    let mut opaque = true;
    let mut buf: Vec<u8> = Vec::new();
    let (rw, rh) = stream_png(ref_path, |y0, chunk| {
        sha2::Digest::update(&mut hasher, chunk);
        let n = (chunk.len() / (w as usize * 3)) as u32;
        buf.clear();
        fill(y0, n, &mut buf);
        assert_eq!(
            buf.len(),
            n as usize * w as usize * ours_stride,
            "{label}: fill gave the wrong number of samples for rows {y0}..{}",
            y0 + n
        );
        if ours_stride == 4 {
            opaque &= buf.chunks_exact(4).all(|p| p[3] == 255);
        }
        acc.add(&buf, ours_stride, chunk, w, y0, tol);
        capture(&mut ours, y0, n, &buf, ours_stride);
        capture(&mut theirs, y0, n, chunk, 3);
    });
    assert_eq!((rw, rh), (w, h), "{label}: stored raster size");
    let ref_hash: String = sha2::Digest::finalize(hasher)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    fn sparse(s: &Side, stride: usize, px: u32) -> impl Fn(u32, u32) -> bool + '_ {
        move |x: u32, y: u32| match s.rows.get(&y) {
            Some(row) => {
                let i = x as usize * stride;
                row[i] < 128 && row[i + 1] < 128 && row[i + 2] < 128
            }
            None => {
                assert_eq!(x, px, "landmark probe asked for an unread pixel");
                s.col[y as usize]
            }
        }
    }
    BigOutcome {
        stats: acc.finish(),
        ref_hash,
        opaque,
        ours_landmarks: probe(&sparse(&ours, ours_stride, pt.px), pt, label),
        ref_landmarks: probe(&sparse(&theirs, 3, pt.px), pt, label),
    }
}
