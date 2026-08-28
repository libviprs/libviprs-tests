//! Regression repro for libviprs/libviprs follow-up to #287/#288 (PR #337).
//!
//! PR #337 made `reduce_axis` premultiply colour by alpha for *every* alpha
//! format, unconditionally. That is correct for the averaging kernels (it stops
//! transparent RGB bleeding into opaque neighbours), but the **Nearest** kernel
//! is a single-tap pick: it does no averaging, so wrapping it in a
//! premultiply -> un-premultiply round-trip can only corrupt the straight-alpha
//! RGB of semi-transparent pixels. Because `premultiply_raster` premultiplies
//! into a *same-bit-depth integer* raster, the intermediate colour is requantised
//! (`round(c * a / max)` then `round(c' * max / a)`), which is lossy for low
//! alpha.
//!
//! A single-tap nearest pick must therefore be *exact*: every output pixel must
//! be byte-identical to the source pixel it selected. `vips resize <rgba>
//! --kernel nearest` preserves the exact sampled pixels; this is that invariant
//! stated directly (no libvips needed at runtime, no fixture).
//!
//! Proven RED against the buggy core (the premultiply round-trip perturbs the
//! low-alpha RGB); GREEN once the Nearest single-tap kernel skips premultiply.

use libviprs::{PixelFormat, Raster, ReduceKernel, ResizeOptions};

/// Four semi-transparent RGBA8 colours whose straight-alpha RGB does **not**
/// survive a premultiply -> un-premultiply integer round-trip. Each corrupts to
/// a value that is not equal to any colour in this palette, so "output pixel is
/// one of these four" is a faithful proxy for "the exact source pixel survived".
///
/// Round-trip images under the buggy path (max = 255):
///   (200,100, 50,10) -> (204,102, 51,10)
///   ( 30,220,140, 3) -> (  0,255,170, 3)
///   (170, 90,240, 7) -> (182, 73,255, 7)
///   ( 90,200, 60, 5) -> (102,204, 51, 5)
/// none of which is a member of the palette.
const PALETTE: [[u8; 4]; 4] = [
    [200, 100, 50, 10],
    [30, 220, 140, 3],
    [170, 90, 240, 7],
    [90, 200, 60, 5],
];

/// A `w` x `h` RGBA8 raster tiled with `PALETTE` on a 2x2 lattice, so *every*
/// pixel is one of the four fragile semi-transparent colours.
fn semi_transparent_raster(w: u32, h: u32) -> Raster {
    let mut data = Vec::with_capacity((w * h) as usize * 4);
    for y in 0..h {
        for x in 0..w {
            let idx = (x % 2) as usize + 2 * (y % 2) as usize;
            data.extend_from_slice(&PALETTE[idx]);
        }
    }
    Raster::new(w, h, PixelFormat::Rgba8, data).unwrap()
}

/// Assert every output pixel is byte-identical to one of the source palette
/// colours. A Nearest single-tap pick can only ever return an exact source
/// pixel; the buggy premultiply round-trip returns requantised colours that are
/// absent from the palette.
fn assert_exact_pick(out: &Raster, label: &str) {
    let data = out.data();
    assert_eq!(out.format(), PixelFormat::Rgba8, "{label}: format changed");
    assert_eq!(data.len() % 4, 0, "{label}: not whole RGBA pixels");
    for (i, px) in data.chunks_exact(4).enumerate() {
        let pixel = [px[0], px[1], px[2], px[3]];
        assert!(
            PALETTE.contains(&pixel),
            "{label}: output pixel {i} = {pixel:?} is not an exact source sample; \
             the Nearest single-tap kernel must not premultiply/round-trip \
             semi-transparent RGB (follow-up to #287/#288)",
        );
    }
}

#[test]
fn reduceh_nearest_preserves_exact_alpha_pixels() {
    let src = semi_transparent_raster(4, 4);
    let out = src.reduceh(2.0, "nearest");
    assert_eq!(out.width(), 2);
    assert_eq!(out.height(), 4);
    assert_exact_pick(&out, "reduceh 2.0 nearest");
}

#[test]
fn reducev_nearest_preserves_exact_alpha_pixels() {
    let src = semi_transparent_raster(4, 4);
    let out = src.reducev(2.0, "nearest");
    assert_eq!(out.width(), 4);
    assert_eq!(out.height(), 2);
    assert_exact_pick(&out, "reducev 2.0 nearest");
}

#[test]
fn reduce_nearest_preserves_exact_alpha_pixels() {
    let src = semi_transparent_raster(4, 4);
    let out = src.reduce(2.0, 2.0, "nearest");
    assert_eq!(out.width(), 2);
    assert_eq!(out.height(), 2);
    assert_exact_pick(&out, "reduce 2.0x2.0 nearest");
}

#[test]
fn reduce_nearest_fractional_preserves_exact_alpha_pixels() {
    // A fractional factor still routes through `reduce_axis` with a single-tap
    // mask, so the pick must remain exact.
    let src = semi_transparent_raster(6, 6);
    let out = src.reduce(1.5, 1.5, "nearest");
    assert_exact_pick(&out, "reduce 1.5x1.5 nearest");
}

#[test]
fn resize_nearest_preserves_exact_alpha_pixels() {
    // `vips_resize` with the nearest kernel subsamples the integer part then
    // reduces the fractional residual through `reduce_axis`; the residual pass
    // must not corrupt the exact pick either.
    let src = semi_transparent_raster(8, 8);
    let opts = ResizeOptions::default().with_kernel(ReduceKernel::Nearest);
    let out = src.resize_with(0.4, opts);
    assert_exact_pick(&out, "resize 0.4 nearest");
}
