//! libviprs sizes a PDF page exactly as libvips does (libviprs#1199).
//!
//! Sits beside `rotation_libvips_pdfium_parity.rs`. The expected sizes are what
//! `pdfload[dpi=N]` reports for every fixture sheet at every DPI in the sweep
//! (and for rotated pages), recorded once in
//! `tests/fixtures/page_size/libvips/libvips_reference.json`. A normal run
//! reads that manifest and needs no libvips: `PageSizing::Exact` has to give
//! the recorded width and height, and the manifest has to be complete for
//! every fixture and DPI covered here (a missing entry fails with the regen
//! command). See `common::libvips_reference` for how it is made.
//!
//! `VIPRS_VERIFY_LIBVIPS_REFERENCE=1` additionally asks a live `vipsheader`
//! and requires it to agree with the manifest and with `PageSizing::Exact`.
//! Without a libvips PDF loader that prints a SKIP; `VIPRS_REQUIRE_VIPS=1`
//! turns the skip into a failure.
//!
//! Where it has run: libvips 8.18.4 with the pdfium backend
//! (`tools/Dockerfile.libvips-pdfium`) and pdfium-8085, linux/arm64, in a
//! container: every entry equal. Native x64 on MARS: see the PR.
//!
//! It does not need the `pdfium` feature: sizing is pure arithmetic, and the
//! rendered rasters are checked in `pdfium_page_size_exact.rs` and
//! `page_size_libvips_pixel_parity.rs`.

mod common;

use common::blank_pdf::{fixture_pdf, libvips_dims};
use common::libvips_reference::{self, matrix};
use common::vips_oracle::{header_field, skip_if_no_vips_pdf};
use libviprs::PageSizing;

#[test]
fn exact_sizing_equals_the_recorded_libvips_size_for_every_fixture_and_dpi() {
    let manifest = libvips_reference::load();
    let mut checked = 0;
    for e in matrix() {
        let rec = manifest.recorded(&e);
        // /Rotate 90 and 270 swap the axes in the page the sizing sees.
        let (w, h) = if matches!(e.spec.rotate, Some(90 | 270)) {
            (e.spec.h, e.spec.w)
        } else {
            (e.spec.w, e.spec.h)
        };
        let label = format!("{} @ {} dpi", e.label, e.dpi);
        // The longhand oracle first: if this fails the test's own formula is
        // wrong and the Exact comparison below means nothing.
        assert_eq!(
            libvips_dims(w, h, e.dpi),
            (rec.width, rec.height),
            "{label}: the test oracle disagrees with the recorded libvips size"
        );
        assert_eq!(
            PageSizing::Exact.pixel_dims(w as f32 as f64, h as f32 as f64, e.dpi),
            (rec.width, rec.height),
            "{label}: PageSizing::Exact disagrees with the recorded libvips size",
        );
        checked += 1;
    }
    assert!(checked >= 170, "only {checked} entries checked");
}

/// Opt-in: the same comparison against a live `vipsheader`, to show the
/// manifest has not drifted from the libvips it was made with.
#[test]
fn recorded_sizes_still_match_a_live_vipsheader() {
    if std::env::var("VIPRS_VERIFY_LIBVIPS_REFERENCE").as_deref() != Ok("1") {
        eprintln!(
            "SKIP recorded_sizes_still_match_a_live_vipsheader: set VIPRS_VERIFY_LIBVIPS_REFERENCE=1 to run vipsheader"
        );
        return;
    }
    let entries = matrix();
    let probe = fixture_pdf(&entries[0].spec);
    if skip_if_no_vips_pdf(
        "recorded_sizes_still_match_a_live_vipsheader",
        probe.to_str().unwrap(),
    ) {
        return;
    }
    let manifest = libvips_reference::load();
    for e in entries {
        let rec = manifest.recorded(&e);
        let file = format!("{}[dpi={}]", fixture_pdf(&e.spec).display(), e.dpi);
        let live = (
            header_field(&file, "width").unwrap(),
            header_field(&file, "height").unwrap(),
        );
        assert_eq!(
            live,
            (rec.width, rec.height),
            "{} @ {} dpi: the manifest drifted from vipsheader",
            e.label,
            e.dpi
        );
    }
}
