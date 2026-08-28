//! vips-differential regression for the premultiplied-alpha resampler
//! follow-ups (libviprs/libviprs #406-#413, #415; part of #348).
//!
//! PR #337 made the averaging resamplers coverage-weight colour by alpha, but
//! the review found the fix incomplete and lossy:
//!
//! * #406 — a mixed downscale-one-axis / upscale-other-axis `resize` still fed
//!   *straight*-alpha colour to the affine enlargement, re-introducing the
//!   transparent-colour bleed on the upscaled axis.
//! * #407 — integer-factor `shrink` / `shrinkh` / `shrinkv` were never
//!   premultiplied at all, so a whole-factor box shrink still bled.
//! * #408/#410/#411/#413 — the premultiply was materialised once *per axis*
//!   into a same-bit-depth **integer** raster, so a two-axis reduce round-tripped
//!   low-alpha colour through straight 8-bit storage and requantised it, wrecking
//!   the colour of near-transparent pixels.
//!
//! The fix premultiplies **once** into a float working buffer around the whole
//! separable pipeline (the `vips_resize` bracket) and un-premultiplies once at
//! the end. libviprs' averaging resamplers therefore match a premultiplied vips
//! pipeline (`vips premultiply | reduce/shrink/resize | vips unpremultiply`),
//! the deliberate divergence documented in the module's *Premultiplied alpha*
//! note.
//!
//! Each case below compares libviprs output against a **committed vips 8.18.4
//! reference** generated offline by exactly that premultiplied pipeline (see
//! `tests/fixtures/README.md` for the commands), so no libvips is needed at
//! runtime. The fixtures use a single opaque colour so the premultiplied colour
//! is coverage-independent: every covered output pixel is the pure opaque colour
//! and the transparent RGB never appears. The straight-alpha bug instead pulls
//! the transparent colour in (the red darkens, the green rises), which blows the
//! colour tolerance.
//!
//! Proven RED against the pre-fix core (integer shrink and the mixed-axis resize
//! bleed; the low-alpha two-axis reduce requantises the colour), GREEN once the
//! premultiply is bracketed once into float around the whole pipeline.

use std::path::{Path, PathBuf};

use libviprs::resize::{downscale_half, downscale_to};
use libviprs::{PixelFormat, Raster, ResizeOptions};

/// Per-colour-channel tolerance against the vips reference. The single-opaque-
/// colour fixtures make the premultiplied colour exact (255,0,0) wherever there
/// is coverage, so only float rounding at the un-premultiply divides separates
/// libviprs from vips; 3/255 is comfortable headroom. The straight-alpha bug
/// pulls transparent colour in with per-channel error in the tens, far outside
/// this band.
const COLOR_TOL: u8 = 3;

/// Alpha-channel tolerance. Alpha *is* kernel-dependent (lanczos / bicubic tap
/// weights and libvips' fixed-point offset quantisation differ from the f64
/// masks here), so the coverage band drifts a few units between the two
/// implementations even when the colour is exact. This is not the property under
/// test (the bleed is a colour phenomenon); a loose band keeps the colour
/// assertion meaningful without pinning an unrelated resampler-parity number.
const ALPHA_TOL: u8 = 6;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Decode an RGBA8 PNG straight into a [`Raster`], preserving the colour under
/// zero-alpha pixels exactly (the `image` crate does not premultiply on decode).
fn load_rgba8(path: &Path) -> Raster {
    let rgba = image::open(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .to_rgba8();
    let (w, h) = rgba.dimensions();
    Raster::new(w, h, PixelFormat::Rgba8, rgba.into_raw()).expect("RGBA8 raster from fixture PNG")
}

/// Assert `actual` matches the committed vips reference PNG within the colour
/// and alpha tolerances, reporting the worst per-channel drift on failure.
fn assert_matches_reference(actual: &Raster, expected_file: &str, label: &str) {
    assert_eq!(
        actual.format(),
        PixelFormat::Rgba8,
        "{label}: output format changed"
    );
    let expected = image::open(fixtures_dir().join(expected_file))
        .unwrap_or_else(|e| panic!("{label}: read reference {expected_file}: {e}"))
        .to_rgba8();
    assert_eq!(
        (actual.width(), actual.height()),
        expected.dimensions(),
        "{label}: dimension mismatch vs vips reference"
    );

    let exp = expected.as_raw();
    let act = actual.data();
    assert_eq!(
        act.len(),
        exp.len(),
        "{label}: byte-length mismatch (actual {}, reference {})",
        act.len(),
        exp.len()
    );

    let mut worst_color = 0u8;
    let mut worst_alpha = 0u8;
    let mut worst_at = (0usize, 0u8, 0u8);
    for (i, (a, e)) in act.chunks_exact(4).zip(exp.chunks_exact(4)).enumerate() {
        // Colour is undefined at fully-transparent pixels (both libviprs and vips
        // zero or leave stray premultiplied residue there), so only the covered
        // pixels — exactly where a transparent-colour bleed would show — constrain
        // the colour bands.
        if e[3] != 0 && a[3] != 0 {
            for band in 0..3 {
                let diff = a[band].abs_diff(e[band]);
                if diff > worst_color {
                    worst_color = diff;
                    worst_at = (i, a[band], e[band]);
                }
            }
        }
        worst_alpha = worst_alpha.max(a[3].abs_diff(e[3]));
    }
    assert!(
        worst_color <= COLOR_TOL,
        "{label}: colour drift {worst_color} > {COLOR_TOL} at pixel {} \
         (libviprs {}, vips {}); the transparent colour is bleeding into opaque \
         neighbours — premultiply must bracket the whole pipeline (#406-#413)",
        worst_at.0,
        worst_at.1,
        worst_at.2,
    );
    assert!(
        worst_alpha <= ALPHA_TOL,
        "{label}: alpha drift {worst_alpha} > {ALPHA_TOL} vs vips reference"
    );
}

/// #407 — integer-factor `shrink` must premultiply. A whole-factor 2x box shrink
/// of the opaque-red / transparent-green checkerboard: pre-fix this ran a plain
/// straight-alpha box mean and bled green into the red (R~127, G~127); the fix
/// brackets the box passes in premultiplied float, so every 2x2 block resolves to
/// pure red at half coverage, matching `vips premultiply | shrink 2 2 |
/// unpremultiply`.
#[test]
fn integer_shrink_premultiplies_like_vips() {
    let src = load_rgba8(
        &fixtures_dir().join("resample_premultiplied_alpha_reference_input/checker_rgba.png"),
    );
    let out = src.shrink(2.0, 2.0);
    assert_matches_reference(
        &out,
        "resample_premultiplied_alpha_reference_expected/checker_shrink2_expected.png",
        "shrink 2x2 (integer box)",
    );
}

/// #408/#410/#411/#413 — a two-axis fractional `reduce` on a low-alpha image must
/// keep colour full-precision. A uniform (200,100,50, alpha=4) image reduced 2x
/// with lanczos3: pre-fix the per-axis integer premultiply raster quantised
/// `round(200*4/255)=3` and un-premultiply amplified it back to ~191, wrecking the
/// colour; the fix keeps the intermediate in float across both axes, preserving
/// (200,100,50), matching `vips premultiply | reduce 2 2 lanczos3 | unpremultiply`.
#[test]
fn low_alpha_two_axis_reduce_matches_vips() {
    let src = load_rgba8(
        &fixtures_dir().join("resample_premultiplied_alpha_reference_input/lowalpha_rgba.png"),
    );
    let out = src.reduce(2.0, 2.0, "lanczos3");
    assert_matches_reference(
        &out,
        "resample_premultiplied_alpha_reference_expected/lowalpha_reduce2_expected.png",
        "reduce 2x2 lanczos3 (low alpha)",
    );
}

/// #406 — a mixed downscale-one-axis / upscale-other-axis `resize` must stay
/// internally consistent. A left-opaque-red / right-transparent-green split
/// resized with horizontal upscale x2 (which crosses the vertical transparency
/// boundary) and vertical downscale x0.5: pre-fix the vertical reduce
/// coverage-weighted but the horizontal affine enlargement then interpolated
/// straight-alpha colour across the boundary (`premultiplied: true` on an
/// un-premultiplied input), darkening the red edge — the exact #287/#288 fringe.
/// The fix premultiplies once around the reduce *and* the affine, keeping the
/// edge pure red, matching `vips premultiply | resize 2.0 --vscale 0.5 |
/// unpremultiply`.
#[test]
fn mixed_axis_resize_matches_vips() {
    let src = load_rgba8(
        &fixtures_dir().join("resample_premultiplied_alpha_reference_input/split_rgba.png"),
    );
    let out = src.resize_with(2.0, ResizeOptions::default().with_vscale(Some(0.5)));
    assert_matches_reference(
        &out,
        "resample_premultiplied_alpha_reference_expected/split_resize_h2_v05_expected.png",
        "resize hscale 2.0 vscale 0.5 (mixed axis)",
    );
}

/// #409 — a large-factor `resize` downscale that triggers the gap-driven integer
/// box pre-shrink inside `reduce_axis` must run that pre-shrink on premultiplied
/// data. resize 0.2 (reduce factor 5, gap 2) box-pre-shrinks by 2 before the
/// lanczos residual; on the premultiplied-float working buffer this stays
/// bleed-free, matching `vips premultiply | resize 0.2 | unpremultiply`.
#[test]
fn box_preshrink_resize_matches_vips() {
    let src = load_rgba8(
        &fixtures_dir().join("resample_premultiplied_alpha_reference_input/checker_rgba.png"),
    );
    let out = src.resize(0.2);
    assert_matches_reference(
        &out,
        "resample_premultiplied_alpha_reference_expected/checker_resize_02_expected.png",
        "resize 0.2 (gap>0 box pre-shrink)",
    );
}

/// #409 — 16-bit alpha weighting. vips' PNG round-trip and the `image` crate's
/// 8-bit decode would lose the 16-bit precision this case exists to check, so it
/// is asserted deterministically instead: a uniform 16-bit low-alpha image has a
/// single opaque colour, so a premultiplied reduce must reproduce that colour
/// exactly (to within 1 LSB of the un-premultiply divide) rather than the
/// requantised value a same-bit-depth integer premultiply would leave.
#[test]
fn rgba16_low_alpha_reduce_preserves_colour() {
    // (40000, 20000, 8000) at alpha = 400 / 65535. A 16-bit integer premultiply
    // would store round(40000*400/65535)=244, then un-premultiply to
    // round(244*65535/400)=39976 — a colour error only the float bracket avoids.
    let color = [40000u16, 20000, 8000, 400];
    let (w, h) = (16u32, 16u32);
    let mut data = Vec::with_capacity((w * h) as usize * 8);
    for _ in 0..w * h {
        for &c in &color {
            data.extend_from_slice(&c.to_ne_bytes());
        }
    }
    let src = Raster::new(w, h, PixelFormat::Rgba16, data).unwrap();
    let out = src.reduce(2.0, 2.0, "lanczos3");
    assert_eq!(out.format(), PixelFormat::Rgba16);
    assert_eq!((out.width(), out.height()), (8, 8));
    for px in out.data().chunks_exact(8) {
        let s = |i: usize| u16::from_ne_bytes([px[2 * i], px[2 * i + 1]]);
        let (r, g, b, a) = (s(0), s(1), s(2), s(3));
        assert!(
            r.abs_diff(color[0]) <= 2
                && g.abs_diff(color[1]) <= 2
                && b.abs_diff(color[2]) <= 2
                && a.abs_diff(color[3]) <= 2,
            "Rgba16 low-alpha reduce lost colour: got ({r},{g},{b},{a}), expected ~{color:?}; \
             the premultiply must bracket into float, not a same-bit-depth integer raster (#411/#413)"
        );
    }
}

/// Panel follow-up — the 2x box halver [`downscale_half`] routes RGBA through
/// `downscale_half_alpha`, which must round its alpha-weighted divides *half-up*
/// to stay consistent with the `downscale_to` round-half-up fix this PR landed.
/// Before the fix it truncated (C `double`→int cast), so a fully-opaque RGBA
/// image downscaled through this box path carried a systematic -0.5 LSB bias
/// versus both the no-alpha box path and `downscale_to` — a silent divergence of
/// at most one level.
///
/// A PNG differential against `vips shrink 2 2` cannot pin this: vips' shrink is a
/// *separable two-stage* box (per-axis `(a+b+1)/2`), so it already differs from a
/// single-stage 4-sample average by up to 1 LSB — the same magnitude as the
/// truncation bug — and any tolerance loose enough to pass the kernel difference
/// swallows the bug. The invariant is instead asserted **exactly** (tolerance 0)
/// as cross-path consistency: for a fully-opaque image the alpha weighting is a
/// no-op, so the alpha box path must produce byte-identical colour to its plain
/// RGB twin and to `downscale_to`. vips 8.18.4 was used only to confirm the
/// rounding *direction* (`shrink 2 2` of an opaque 0.5-average block yields 1,
/// not 0).
mod downscale_half_alpha_rounds_half_up {
    use super::*;

    /// Build a 4x4 fully-opaque RGBA image tiling a 2x2 block whose per-channel
    /// sums land off a clean quarter: R sum 2 (avg 0.5), G sum 10 (2.5), B sum 42
    /// (10.5). Round-half-up resolves each to (1, 3, 11); the old truncation gave
    /// (0, 2, 10). Every 2x2 block is identical, so the 2x2 output is uniform.
    fn opaque_source() -> Raster {
        // even column -> (0, 2, 10), odd column -> (1, 3, 11), alpha always opaque.
        let mut data = Vec::with_capacity(4 * 4 * 4);
        for _y in 0..4u32 {
            for x in 0..4u32 {
                let odd = (x & 1) == 1;
                data.extend_from_slice(&[
                    if odd { 1 } else { 0 },
                    if odd { 3 } else { 2 },
                    if odd { 11 } else { 10 },
                    255,
                ]);
            }
        }
        Raster::new(4, 4, PixelFormat::Rgba8, data).expect("4x4 opaque RGBA8 source")
    }

    /// The alpha box path must round half-up, not truncate.
    #[test]
    fn alpha_box_path_rounds_half_up() {
        let out = downscale_half(&opaque_source()).expect("downscale_half RGBA");
        assert_eq!((out.width(), out.height()), (2, 2));
        for px in out.data().chunks_exact(4) {
            assert_eq!(
                px,
                [1, 3, 11, 255],
                "downscale_half_alpha must round half-up (got {px:?}); truncation would give \
                 [0, 2, 10, 255] — the -0.5 LSB bias the panel flagged"
            );
        }
    }

    /// A fully-opaque RGBA image must downscale to byte-identical colour through
    /// the alpha box path and its plain RGB twin (alpha weighting is a no-op when
    /// every pixel is opaque). This is the exact inconsistency the panel found.
    #[test]
    fn opaque_alpha_path_matches_rgb_twin() {
        let src = opaque_source();
        let rgba_out = downscale_half(&src).expect("downscale_half RGBA");

        // RGB twin: same colour bands, no alpha channel -> no-alpha box path.
        let mut rgb = Vec::with_capacity(4 * 4 * 3);
        for px in src.data().chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        let rgb_src = Raster::new(4, 4, PixelFormat::Rgb8, rgb).expect("4x4 RGB8 twin");
        let rgb_out = downscale_half(&rgb_src).expect("downscale_half RGB");

        for (rgba, rgb) in rgba_out
            .data()
            .chunks_exact(4)
            .zip(rgb_out.data().chunks_exact(3))
        {
            assert_eq!(
                &rgba[..3],
                rgb,
                "opaque RGBA colour must match its RGB twin bit-for-bit through the box halver"
            );
            assert_eq!(rgba[3], 255, "opaque alpha must stay opaque");
        }
    }

    /// The alpha box path and the arbitrary-ratio `downscale_to` (both round
    /// half-up now) must produce byte-identical output for the same 2x halving.
    #[test]
    fn alpha_box_path_matches_downscale_to() {
        let src = opaque_source();
        let box_out = downscale_half(&src).expect("downscale_half RGBA");
        let to_out = downscale_to(&src, 2, 2).expect("downscale_to 2x2");
        assert_eq!(
            box_out.data(),
            to_out.data(),
            "downscale_half_alpha and downscale_to must agree bit-for-bit on a fully-opaque \
             2x halving; a divergence here is the round-half-up/truncate inconsistency"
        );
    }
}
