#![cfg(feature = "pdfium")]

// All assertions live inside a single `#[test]` function on purpose.
//
// pdfium-render holds process-wide FFI state inside libpdfium.so. With
// `--test-threads=1`, libtest still runs each `#[test]` function on its
// own worker invocation: each invocation creates and drops `PdfDocument`
// values that release pdfium-internal resources. A second test invocation
// in the same process then trips a double-free / SIGABRT (locally) or
// hangs in the C library (on the CI runner).
//
// libviprs already serializes Pdfium creation behind `OnceLock<Pdfium>`
// (see `libviprs::pdf::init_pdfium`), so the singleton itself is safe.
// What is not safe is repeatedly tearing down `PdfDocument` instances
// across multiple top-level tests in one binary. Wrapping every smoke
// check in a single test function keeps document lifetimes nested and
// avoids the cross-test corruption.
//
// Direct, raw-Pdfium smoke checks live in `pdfium_system_check.rs` and
// run only via `--ignored`.

use libviprs::pdf::{render_page_pdfium, render_page_pdfium_budgeted};
use std::path::Path;

const FIXTURE_PDF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");

/// Default DPI matching libvips.
const DEFAULT_DPI: u32 = 72;

/// Default pixel budget for budgeted rendering tests (100 megapixels).
const DEFAULT_MAX_PIXELS: u64 = 100_000_000;

#[test]
fn libviprs_pdfium_render_paths() {
    // render_page_pdfium produces a valid Raster.
    {
        let raster =
            render_page_pdfium(Path::new(FIXTURE_PDF), 1, 72).expect("render_page_pdfium failed");
        assert!(
            raster.width() > 100,
            "Rendered raster too narrow: {}",
            raster.width()
        );
        assert!(
            raster.height() > 100,
            "Rendered raster too short: {}",
            raster.height()
        );
        assert_eq!(raster.format(), libviprs::PixelFormat::Rgba8);
    }

    // Invalid page number returns a typed error.
    {
        let result = render_page_pdfium(Path::new(FIXTURE_PDF), 999, 72);
        assert!(result.is_err(), "Expected error for out-of-range page");
    }

    // Missing file returns a typed error.
    {
        let result = render_page_pdfium(Path::new("/nonexistent.pdf"), 1, 72);
        assert!(result.is_err(), "Expected error for missing file");
    }

    // Budgeted render at default DPI when budget is generous.
    {
        let result =
            render_page_pdfium_budgeted(Path::new(FIXTURE_PDF), 1, DEFAULT_DPI, DEFAULT_MAX_PIXELS)
                .expect("budgeted render failed");
        assert!(!result.capped, "Should not be capped at {DEFAULT_DPI} DPI");
        assert_eq!(result.dpi_used, DEFAULT_DPI);
        assert!(result.raster.width() > 100);
        assert!(result.raster.height() > 100);
        assert_eq!(result.raster.format(), libviprs::PixelFormat::Rgba8);
    }

    // Budgeted render caps DPI when the pixel budget is small.
    {
        let tiny_budget: u64 = 100 * 100; // 10,000 pixels
        let result =
            render_page_pdfium_budgeted(Path::new(FIXTURE_PDF), 1, DEFAULT_DPI, tiny_budget)
                .expect("budgeted render failed");
        assert!(result.capped, "Should be capped with tiny budget");
        assert!(
            result.dpi_used < DEFAULT_DPI,
            "DPI should be reduced from {DEFAULT_DPI}, got {}",
            result.dpi_used
        );
        let total_pixels = result.raster.width() as u64 * result.raster.height() as u64;
        assert!(
            total_pixels <= tiny_budget,
            "Output {total_pixels} pixels exceeds budget {tiny_budget}"
        );
    }

    // Budgeted render matches unbounded render when the budget is generous.
    {
        let unbounded =
            render_page_pdfium(Path::new(FIXTURE_PDF), 1, DEFAULT_DPI).expect("render failed");
        let budgeted =
            render_page_pdfium_budgeted(Path::new(FIXTURE_PDF), 1, DEFAULT_DPI, DEFAULT_MAX_PIXELS)
                .expect("budgeted render failed");
        assert_eq!(unbounded.width(), budgeted.raster.width());
        assert_eq!(unbounded.height(), budgeted.raster.height());
        assert_eq!(unbounded.data(), budgeted.raster.data());
    }

    // Budgeted render handles invalid page number gracefully.
    {
        let result = render_page_pdfium_budgeted(
            Path::new(FIXTURE_PDF),
            999,
            DEFAULT_DPI,
            DEFAULT_MAX_PIXELS,
        );
        assert!(result.is_err(), "Expected error for out-of-range page");
    }

    // Budgeted render handles missing file gracefully.
    {
        let result = render_page_pdfium_budgeted(
            Path::new("/nonexistent.pdf"),
            1,
            DEFAULT_DPI,
            DEFAULT_MAX_PIXELS,
        );
        assert!(result.is_err(), "Expected error for missing file");
    }
}
