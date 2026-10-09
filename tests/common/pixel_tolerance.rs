//! How far two renders of the same page may differ, and how to measure it.
//!
//! Two paths rasterise the same page with the same pdfium: libvips (page
//! flags `FPDF_ANNOT | FPDF_REVERSE_BYTE_ORDER`, then a form-fill pass) and
//! libviprs (its own flags). Antialiasing and colour handling differ in the
//! last bits, so byte equality is not the contract; a bounded error is.
//!
//! A pixel is "bad" when any channel differs by more than `max_channel_delta`.
//! A comparison passes when the bad fraction is at most `max_bad_fraction` and
//! the mean absolute error over all channels is at most `max_mean_abs_error`.
//! The defaults come from the measurements recorded in
//! `tests/page_size_libvips_pixel_parity.rs` (measured worst case 1e-5 bad
//! and 0.0014 MAE, headroom about 50x and 14x; a one-pixel error measures at
//! least 0.0065 bad and 0.91 MAE, so it is 13x and 45x outside); a different pdfium build can
//! loosen them without editing code through `VIPRS_PIXEL_MAX_DELTA`,
//! `VIPRS_PIXEL_MAX_BAD_FRACTION` and `VIPRS_PIXEL_MAX_MAE`.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelTolerance {
    pub max_channel_delta: u8,
    pub max_bad_fraction: f64,
    pub max_mean_abs_error: f64,
}

impl Default for PixelTolerance {
    fn default() -> Self {
        Self {
            max_channel_delta: DEFAULT_MAX_DELTA,
            max_bad_fraction: DEFAULT_MAX_BAD_FRACTION,
            max_mean_abs_error: DEFAULT_MAX_MAE,
        }
    }
}

pub const DEFAULT_MAX_DELTA: u8 = 48;
pub const DEFAULT_MAX_BAD_FRACTION: f64 = 0.0005;
pub const DEFAULT_MAX_MAE: f64 = 0.02;

impl PixelTolerance {
    /// The defaults, overridden by the `VIPRS_PIXEL_*` environment variables.
    pub fn from_env() -> Self {
        fn var<T: std::str::FromStr>(name: &str) -> Option<T> {
            let v = std::env::var(name).ok()?;
            Some(
                v.parse()
                    .unwrap_or_else(|_| panic!("{name}={v:?} does not parse")),
            )
        }
        let d = Self::default();
        Self {
            max_channel_delta: var("VIPRS_PIXEL_MAX_DELTA").unwrap_or(d.max_channel_delta),
            max_bad_fraction: var("VIPRS_PIXEL_MAX_BAD_FRACTION").unwrap_or(d.max_bad_fraction),
            max_mean_abs_error: var("VIPRS_PIXEL_MAX_MAE").unwrap_or(d.max_mean_abs_error),
        }
    }
}

/// What a comparison of two RGB buffers measured.
#[derive(Clone, Copy, Debug)]
pub struct PixelStats {
    pub bad_fraction: f64,
    pub mean_abs_error: f64,
    /// Fraction of pixels whose three channels are byte-identical.
    pub identical_fraction: f64,
    pub max_delta: u8,
    /// Pixel `(x, y)` of the first largest delta.
    pub max_at: (u32, u32),
}

impl PixelStats {
    pub fn within(&self, tol: &PixelTolerance) -> bool {
        self.bad_fraction <= tol.max_bad_fraction && self.mean_abs_error <= tol.max_mean_abs_error
    }

    pub fn summary(&self) -> String {
        format!(
            "bad {:.5} mae {:.4} identical {:.4} max delta {} at ({}, {})",
            self.bad_fraction,
            self.mean_abs_error,
            self.identical_fraction,
            self.max_delta,
            self.max_at.0,
            self.max_at.1
        )
    }
}

/// Row-wise accumulation of the statistics, so a big raster is compared in
/// chunks and never needs two full copies in memory.
#[derive(Default)]
pub struct StatsAcc {
    pixels: u64,
    bad: u64,
    same: u64,
    sum: u64,
    max_delta: u8,
    max_at: (u32, u32),
}

impl StatsAcc {
    /// Add `rows` of `a` (`a_stride` bytes per pixel, 3 or 4, extra bytes
    /// ignored) against `b` (RGB), starting at row `y0` of a `width` wide image.
    pub fn add(
        &mut self,
        a: &[u8],
        a_stride: usize,
        b: &[u8],
        width: u32,
        y0: u32,
        tol: &PixelTolerance,
    ) {
        assert_eq!(
            a.len() / a_stride,
            b.len() / 3,
            "chunks differ in pixel count"
        );
        for (i, (pa, pb)) in a.chunks_exact(a_stride).zip(b.chunks_exact(3)).enumerate() {
            let mut worst = 0u8;
            for c in 0..3 {
                let d = pa[c].abs_diff(pb[c]);
                self.sum += d as u64;
                worst = worst.max(d);
            }
            self.same += u64::from(worst == 0);
            self.bad += u64::from(worst > tol.max_channel_delta);
            if worst > self.max_delta {
                self.max_delta = worst;
                self.max_at = ((i as u32) % width, y0 + (i as u32) / width);
            }
        }
        self.pixels += (b.len() / 3) as u64;
    }

    pub fn finish(&self) -> PixelStats {
        let n = self.pixels.max(1) as f64;
        PixelStats {
            bad_fraction: self.bad as f64 / n,
            mean_abs_error: self.sum as f64 / (3.0 * n),
            identical_fraction: self.same as f64 / n,
            max_delta: self.max_delta,
            max_at: self.max_at,
        }
    }
}

/// Compare two interleaved RGB buffers of the same `width` x `height`.
pub fn compare_rgb(a: &[u8], b: &[u8], width: u32, tol: &PixelTolerance) -> PixelStats {
    assert_eq!(a.len(), b.len(), "buffers differ in length");
    let mut acc = StatsAcc::default();
    acc.add(a, 3, b, width, 0, tol);
    acc.finish()
}

/// `rgb` moved `dx`, `dy` pixels (positive is right and down), compared over
/// the part both cover. Returns the two overlapping crops.
pub fn shifted_overlap(
    a: &[u8],
    b: &[u8],
    (w, h): (u32, u32),
    (dx, dy): (i32, i32),
) -> (Vec<u8>, Vec<u8>, u32) {
    let (ow, oh) = (w - dx.unsigned_abs(), h - dy.unsigned_abs());
    let (ax, ay) = (dx.max(0) as u32, dy.max(0) as u32);
    let (bx, by) = ((-dx).max(0) as u32, (-dy).max(0) as u32);
    let (mut ca, mut cb) = (Vec::new(), Vec::new());
    for y in 0..oh {
        let ra = (((ay + y) * w + ax) * 3) as usize;
        let rb = (((by + y) * w + bx) * 3) as usize;
        ca.extend_from_slice(&a[ra..ra + (ow * 3) as usize]);
        cb.extend_from_slice(&b[rb..rb + (ow * 3) as usize]);
    }
    (ca, cb, ow)
}

/// `rgb` stretched one pixel wider (nearest neighbour) and cropped back to
/// `width`: the left edge stays put and the right edge ends up a pixel off, as
/// a page rendered one pixel too wide would.
pub fn stretched_one_pixel(rgb: &[u8], (w, h): (u32, u32)) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len());
    for y in 0..h {
        for x in 0..w {
            // Column x of a w+1 wide nearest-neighbour stretch of the row.
            let sx = (x as u64 * w as u64 / (w as u64 + 1)) as u32;
            let i = ((y * w + sx) * 3) as usize;
            out.extend_from_slice(&rgb[i..i + 3]);
        }
    }
    out
}
