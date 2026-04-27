//! Property-based coverage for the streaming-mode public surface.
//!
//! The example-driven tests in `pdfium_streaming_*` cover specific
//! (fixture, dpi, strip_h, y_offset, rotation) tuples. This file
//! exercises the parametric space via `proptest`, asserting properties
//! that must hold for **every** valid combination, not just the ones
//! we hand-picked. Properties tested:
//!
//! - `PageRotation::try_from_degrees ∘ as_degrees` is identity for all
//!   four variants.
//! - `try_from_degrees` accepts `n * 90` for any `i64` (after `rem_euclid`)
//!   and rejects any `n * 90 + k` where `k ∈ {1..89}`.
//! - For any in-range `(y_offset, strip_height)`, `render_strip` returns
//!   a raster with width = source.width() and height = min(strip_height,
//!   source.height() - y_offset). No panics, no malformed dimensions.
//! - For any random rendering of a non-trivial strip, the returned data
//!   length equals exactly `width * height * 4`.

#![cfg(feature = "pdfium")]

mod common;

use common::FIXTURE_PORTRAIT;
use libviprs::PdfiumStripSource;
use libviprs::pdf::PageRotation;
use libviprs::streaming::StripSource;
use proptest::prelude::*;

const PROPTEST_DPI: u32 = 72;

// ---------------------------------------------------------------------------
// PageRotation properties — pure, no pdfium calls, fast cases.
// ---------------------------------------------------------------------------

proptest! {
    /// All four canonical rotations round-trip through degrees.
    #[test]
    fn page_rotation_round_trips_via_degrees(rot_idx in 0u8..=3) {
        let rot = match rot_idx {
            0 => PageRotation::Zero,
            1 => PageRotation::Quarter,
            2 => PageRotation::Half,
            _ => PageRotation::ThreeQuarter,
        };
        prop_assert_eq!(
            PageRotation::try_from_degrees(rot.as_degrees()).unwrap(),
            rot
        );
    }

    /// Any multiple of 90 — including negatives and values ≥360 — is
    /// accepted; the result is normalised to the canonical variant.
    #[test]
    fn page_rotation_accepts_any_multiple_of_90(n in -32i64..32) {
        let degrees = n * 90;
        let result = PageRotation::try_from_degrees(degrees);
        prop_assert!(
            result.is_ok(),
            "try_from_degrees({degrees}) should accept multiples of 90, got {:?}",
            result
        );
    }

    /// Any non-multiple-of-90 between 1 and 89 (after offset) is rejected.
    /// Sample within `[1, 89]` so we can be sure the offset isn't itself
    /// a multiple of 90.
    #[test]
    fn page_rotation_rejects_non_multiples(
        base in -10i64..10,
        offset in 1i64..=89
    ) {
        let degrees = base * 90 + offset;
        match PageRotation::try_from_degrees(degrees) {
            Err(libviprs::pdf::PdfError::UnsupportedRotation(d)) => {
                prop_assert_eq!(d, degrees);
            }
            other => prop_assert!(false, "expected UnsupportedRotation({degrees}), got {:?}", other),
        }
    }
}

// ---------------------------------------------------------------------------
// PdfiumStripSource::render_strip properties — needs pdfium, expensive.
// We sample fewer cases (proptest defaults to 256 cases per #[test]; we
// override to 16 here so the suite stays under a minute).
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        .. ProptestConfig::default()
    })]

    /// `render_strip` never panics for any (y_offset, strip_height) pair
    /// where `y_offset < page_height`. Returned raster width equals the
    /// source's width; returned height is bounded by what was requested.
    /// `data().len() == width * height * 4` (RGBA8 invariant).
    #[test]
    fn render_strip_never_panics_with_valid_inputs(
        // Strip heights in a reasonable range — not just 1px (slow per
        // call) and not larger than the page (the partial-last-strip
        // path is exercised separately in pdfium_streaming_render.rs).
        strip_height in 16u32..=512,
        // y_offset_frac samples a fraction of the page height to get
        // strip starts spread across the page without depending on the
        // page dimensions (which we don't know until we construct the
        // source).
        y_offset_frac in 0.0_f32..=0.95,
    ) {
        let source =
            PdfiumStripSource::new_streaming(FIXTURE_PORTRAIT, 1, PROPTEST_DPI).unwrap();
        let h_total = source.height();
        let y_offset = (h_total as f32 * y_offset_frac) as u32;
        prop_assume!(y_offset < h_total);

        let strip = source.render_strip(y_offset, strip_height).unwrap();

        // Width invariant: always equals source.width().
        prop_assert_eq!(strip.width(), source.width());
        // Height invariant: bounded by both the request and the page.
        let expected_h = strip_height.min(h_total - y_offset);
        prop_assert_eq!(strip.height(), expected_h);
        // RGBA8 byte-length invariant.
        prop_assert_eq!(
            strip.data().len() as u64,
            u64::from(strip.width()) * u64::from(strip.height()) * 4
        );
    }
}
