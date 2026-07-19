//! vips-differential golden for mixed bit-depth compositing (issues #443–#448,
//! part of #351).
//!
//! The follow-up review of the #289 mixed-depth fix (PR #340) found that
//! `composite2` keyed the 0..65535 vs 0..255 read/write scale on the *raw
//! stored* `meta.interpretation` tag rather than the raster's *resolved*
//! [`Raster::interpretation`]. That tag is `None` for any raster built through
//! the public [`Raster::new`] constructor — exactly what a user decoding a
//! genuine 16-bit PNG produces — so an untagged genuine-16 overlay was read on
//! the 8-bit joint scale: its colours clamped and its half-alpha saturated to
//! fully opaque (the "257:1 blend ratio" / "16-bit alpha ceiling" #289
//! symptoms), even though the crate's own [`Raster::interpretation`] getter
//! already reports `Rgb16` for it.
//!
//! This is a pure public-API, offline vips-differential test: the reference
//! output was produced with `vips composite2` (libvips 8.18.4) and committed,
//! so no libvips is needed at run time. The inputs come from the same PNGs vips
//! consumed. It is RED against the pre-fix core (the untagged 16-bit overlay is
//! read on the 255 scale and washes out — per-channel drift in the tens of
//! thousands) and GREEN once `composite2` keys the scale on the resolved
//! interpretation (gated on the actual 2-byte storage depth).
//!
//! The file also carries the *cross-module* counterpart
//! ([`add_const_promoted_opaque_overlay_stays_visible_over_8bit_base`]): keying
//! the scale on the resolved interpretation must not misread an
//! arithmetic-*promoted* 16-bit container (whose samples stay numerically on
//! 0..255) as a genuine-16 layer. That case has no vips reference — libvips
//! promotes to float, not to a 0..255-in-a-16-bit buffer — so its oracle is the
//! Porter-Duff `Over` identity for an opaque source; see its own doc comment.
//!
//! # Fixture regeneration
//!
//! The three PNGs under `tests/fixtures/composite_mixed_depth_reference_expected/`
//! were produced offline with the vips CLI:
//!
//! ```text
//! # base_rgb8.png     : 2x2 8-bit RGB   {red, green, blue, white}
//! # overlay_rgba16.png: 2x2 16-bit RGBA {half-blue, opaque-yellow,
//! #                                      transparent, half-red}
//! vips composite2 base_rgb8.png overlay_rgba16.png \
//!     expected_over_rgba16.png over
//! ```
//!
//! The 8-bit base channels are deliberately only 0 or 255 so libvips' 8->16
//! promotion (`*257`) is exact and cannot introduce a systematic
//! 255-vs-256-divisor drift; the only residual difference from libviprs is the
//! final integer rounding of the blend, at most 1 unit out of 65535 (see
//! [`TOLERANCE`]).

use libviprs::{CompositeMode, PixelFormat, Raster};
use std::path::PathBuf;

/// Per-channel tolerance in 16-bit units. libviprs and libvips normalise the
/// genuine-16 overlay on the same 0..65535 scale and the 0/255 base on the same
/// 0..255 scale, so the two agree exactly up to the final round-to-nearest of
/// the premultiplied blend — at most 1 unit observed. `2` leaves headroom while
/// still catching the pre-fix bug, whose washed-out output drifts by tens of
/// thousands.
const TOLERANCE: u16 = 2;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/composite_mixed_depth_reference_expected")
        .join(name)
}

/// Decode the committed 8-bit RGB base PNG into an [`PixelFormat::Rgb8`]
/// [`Raster`] — a plain 8-bit sRGB layer, no explicit interpretation tag.
fn base_rgb8() -> Raster {
    let img = image::open(fixture("base_rgb8.png"))
        .expect("open base_rgb8.png")
        .to_rgb8();
    let (w, h) = img.dimensions();
    Raster::new(w, h, PixelFormat::Rgb8, img.into_raw()).expect("Rgb8 base raster")
}

/// Decode the committed 16-bit RGBA overlay PNG into an
/// [`PixelFormat::Rgba16`] [`Raster`]. Crucially it is left **untagged** — the
/// primary [`Raster::new`] construction path a user decoding a 16-bit PNG hits
/// — so the test exercises the untagged genuine-16 case from #443/#444/#445,
/// not the niche explicitly-`.interpretation(Rgb16)`-tagged path.
fn overlay_rgba16() -> Raster {
    let img = image::open(fixture("overlay_rgba16.png"))
        .expect("open overlay_rgba16.png")
        .to_rgba16();
    let (w, h) = img.dimensions();
    let bytes: Vec<u8> = img
        .into_raw()
        .iter()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    Raster::new(w, h, PixelFormat::Rgba16, bytes).expect("Rgba16 overlay raster")
}

/// The committed vips reference output as flat native-endian `u16` samples
/// (RGBA, row-major).
fn expected_samples() -> (u32, u32, Vec<u16>) {
    let img = image::open(fixture("expected_over_rgba16.png"))
        .expect("open expected_over_rgba16.png")
        .to_rgba16();
    let (w, h) = img.dimensions();
    (w, h, img.into_raw())
}

/// Read a [`PixelFormat::Rgba16`] raster's samples as flat native-endian `u16`.
fn raster_samples_u16(r: &Raster) -> Vec<u16> {
    assert_eq!(r.format(), PixelFormat::Rgba16, "expected an Rgba16 result");
    r.data()
        .chunks_exact(2)
        .map(|b| u16::from_ne_bytes([b[0], b[1]]))
        .collect()
}

/// `composite2(Over)` of an 8-bit RGB base and an untagged genuine 16-bit RGBA
/// overlay must match `vips composite2 ... over` pixel-for-pixel (within
/// [`TOLERANCE`]): the overlay is normalised on its own 0..65535 scale and the
/// result is the deeper 16-bit container, so its half-alpha survives and its
/// colours mix at a true ratio instead of the pre-fix 257:1 wash-out.
#[test]
fn untagged_16bit_overlay_over_8bit_base_matches_vips() {
    let base = base_rgb8();
    let overlay = overlay_rgba16();

    let out = base.composite2(&overlay, CompositeMode::Over);
    assert_eq!(
        out.format(),
        PixelFormat::Rgba16,
        "mixed-depth output must land in the deeper 16-bit container"
    );

    let (ew, eh, expected) = expected_samples();
    assert_eq!((out.width(), out.height()), (ew, eh), "dimension mismatch");

    let actual = raster_samples_u16(&out);
    assert_eq!(
        actual.len(),
        expected.len(),
        "sample-count mismatch: libviprs {}, vips {}",
        actual.len(),
        expected.len()
    );

    let mut max_diff = 0u16;
    for (i, (&a, &e)) in actual.iter().zip(expected.iter()).enumerate() {
        let diff = a.abs_diff(e);
        max_diff = max_diff.max(diff);
        assert!(
            diff <= TOLERANCE,
            "sample {i} (px {}, ch {}): libviprs {a} vs vips {e}, diff {diff} > {TOLERANCE} \
             — the untagged genuine-16 overlay was read on the wrong scale",
            i / 4,
            i % 4
        );
    }
    eprintln!("composite_mixed_depth_reference: max per-channel diff = {max_diff} / 65535");
}

/// The cross-module counterpart to the genuine-16 case above, and the
/// regression guard for the fix's own hazard.
///
/// `composite2` keys its 0..65535 vs 0..255 scale on the overlay's *resolved*
/// interpretation. A genuine 16-bit overlay (case above) correctly reads on
/// 65535. But the constant arithmetic ops (`add_const` / `mul_const` /
/// `pow_const` / `add_vec`) *promote* an 8-bit input into a 16-bit container
/// whose samples stay numerically on 0..255 (the crate's promoted-container
/// idiom, standing in for a libvips float). If such a promoted buffer were
/// left resolving to `Rgb16` / `Grey16`, `composite2` would read it on the
/// 65535 scale: a fully-opaque promoted overlay (alpha `255`) would be seen as
/// `255 / 65535 ≈ 0.4 %` opaque and its 0..255 colours as near-black, so it
/// would *vanish* over the base — silent data loss.
///
/// The fix is cross-module: the arithmetic ops stamp the promoted output with
/// the source interpretation (`Srgb` for an `Rgba8` input), so it resolves to
/// a non-genuine-16 space and `composite2` reads it on 0..255. This test locks
/// that in: a fully-opaque `add_const`-promoted overlay `Over` a solid base
/// must *replace* the base (the Porter-Duff `Over` invariant for an opaque
/// source), not disappear.
///
/// There is deliberately no vips reference here: libvips' `linear` / `add`
/// promote to *float*, not to a 0..255-in-a-16-bit-container buffer, so this
/// codifies a libviprs-specific promotion convention. The oracle is instead
/// the exact `Over` identity `opaque_source Over anything == source`.
#[test]
fn add_const_promoted_opaque_overlay_stays_visible_over_8bit_base() {
    // Solid mid-grey 8-bit RGB base (2x1). If the promoted overlay were read
    // on the 65535 scale it would vanish and the output would collapse back to
    // this grey — the assertions below would then fail.
    let base = Raster::new(2, 1, PixelFormat::Rgb8, vec![128, 128, 128, 128, 128, 128])
        .expect("Rgb8 base raster");

    // Fully-opaque 8-bit RGBA overlay, distinct colour (200, 100, 50), alpha
    // 255. `add_const(0.0)` widens it to a 16-bit container without changing
    // the sample values (the promoted-container idiom) — exactly the untagged
    // Rgb16-shaped buffer that regressed before the interpretation stamp.
    let overlay8 = Raster::new(
        2,
        1,
        PixelFormat::Rgba8,
        vec![200, 100, 50, 255, 200, 100, 50, 255],
    )
    .expect("Rgba8 overlay raster");
    let promoted = overlay8.add_const(0.0);
    assert_eq!(
        promoted.format(),
        PixelFormat::Rgba16,
        "add_const must promote the 8-bit overlay into a 16-bit container"
    );

    let out = base.composite2(&promoted, CompositeMode::Over);
    assert_eq!(
        out.format(),
        PixelFormat::Rgba16,
        "mixed-depth output must land in the deeper 16-bit container"
    );

    // Opaque `Over` == source: the promoted overlay must show through on the
    // 0..255 (promoted-container) scale, alpha fully opaque at 255 on that same
    // scale. Under the pre-fix regression the overlay would instead read on the
    // 65535 scale and vanish, leaving the grey base (~32768 per channel) with a
    // 65535 alpha — both far outside this tolerance.
    let s = raster_samples_u16(&out);
    let expected = [200u16, 100, 50, 255];
    for (px, chunk) in s.chunks_exact(4).enumerate() {
        for (ch, (&a, &e)) in chunk.iter().zip(expected.iter()).enumerate() {
            assert!(
                a.abs_diff(e) <= TOLERANCE,
                "px {px} ch {ch}: libviprs {a} vs expected {e} (opaque Over == source) \
                 — a promoted overlay was read on the genuine-16 scale and vanished"
            );
        }
    }
}
