//! Reproduces the istracked/server #95 / PR #96 streaming panic.
//!
//! Mirrors the call sequence in `ensure_tiles_generated` from
//! `istracked/server/src/rest_api/sheets.rs` on PR branch
//! `libviprs_streaming`:
//!
//!     pdf_info(path) -> width_pts, height_pts
//!     width_px  = (width_pts / 72 * dpi) as u32
//!     height_px = (height_pts / 72 * dpi) as u32
//!     PdfiumStripSource::new(path, 1, dpi, width_px, height_px)
//!     PyramidPlanner::new(width_px, height_px, 256, 0, Layout::Google)
//!     StreamingConfig { memory_budget_bytes: 1_000_000, .. }
//!     generate_pyramid_streaming(...)
//!
//! Reported panic (libviprs 0.1.4):
//!     thread '...' panicked at libviprs-0.1.4/src/streaming.rs:840:51:
//!     range end index 1009732 out of range for slice of length 953568
//!
//! Hypothesis: `PdfiumStripSource::width()/height()` returns the user-
//! supplied prediction, but `extract_strip` returns a strip sized from
//! the actual pdfium-rendered raster. When pdfium honours a page-rotation
//! hint (or rounds dimensions differently from the caller's prediction),
//! the streaming engine's per-row copy in `obtain_canvas_strip` writes
//! `actual_width * bpp` bytes per row into a destination buffer striped
//! at `predicted_width * bpp`, producing the slice OOB.

#![cfg(feature = "pdfium")]

use std::path::Path;

use libviprs::pdf::{pdf_info, render_page_pdfium};
use libviprs::streaming::generate_pyramid_streaming;
use libviprs::{
    EngineConfig, Layout, MemorySink, PdfiumStripSource, PyramidPlanner, StreamingConfig,
    observe::NoopObserver,
};

const FIXTURE_PDF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");

const TILE_SIZE: u32 = 256;
// PR #96 sets `let dpi = 300` in source, but the runtime that reported the
// panic used 72 DPI. The dimension-mismatch bug shape is identical at any
// DPI (rotation swaps width/height regardless), and 72 DPI keeps
// blueprint.pdf at ~16 MP × 4 bpp = ~64 MB, well inside the 4 GB container
// budget. Lower DPI just makes the test cheaper to run.
const DPI: u32 = 72;
const BACKGROUND_RGB: [u8; 3] = [0xE7, 0xE7, 0xE7];
const MEMORY_BUDGET_BYTES_1MB: u64 = 1_000_000;
const MEMORY_BUDGET_BYTES_2MB: u64 = 2_000_000;

/// Compute the predicted pixel dimensions exactly the way PR #96 does,
/// using `(width_pts / 72.0 * dpi) as u32` truncation.
fn predicted_dims(path: &Path, dpi: u32) -> (u32, u32) {
    let info = pdf_info(path).expect("pdf_info failed");
    let page = &info.pages[0];
    let w = (page.width_pts / 72.0 * dpi as f64) as u32;
    let h = (page.height_pts / 72.0 * dpi as f64) as u32;
    (w, h)
}

/// Diagnostic: print predicted dims vs. what pdfium actually renders.
/// If they differ, that is the off-by-row that wedges the streaming engine.
#[test]
fn diagnostic_predicted_vs_actual_dims() {
    let (pw, ph) = predicted_dims(Path::new(FIXTURE_PDF), DPI);
    let raster =
        render_page_pdfium(Path::new(FIXTURE_PDF), 1, DPI).expect("render_page_pdfium failed");
    let (aw, ah) = (raster.width(), raster.height());

    eprintln!("blueprint.pdf @ {DPI} DPI:");
    eprintln!("  predicted: {pw} x {ph}");
    eprintln!("  actual:    {aw} x {ah}");
    eprintln!(
        "  delta:     dw={} dh={}",
        aw as i64 - pw as i64,
        ah as i64 - ph as i64,
    );

    // We don't fail this test — it's purely diagnostic. The streaming
    // tests below will panic if the delta matters.
}

/// Replicates PR #96's code path verbatim with a 1MB memory budget on the
/// blueprint.pdf fixture. If `streaming.rs` panics here, we have a
/// bug-in-libviprs reproduction without needing the original failing PDF.
///
/// Marked `#[ignore]` because it intentionally panics — opt in with
/// `cargo test --test streaming_pdfium_repro -- --ignored` to reproduce.
/// Remove the attribute once libviprs#41 is fixed so this becomes a
/// regression guard.
#[test]
#[ignore = "intentionally panics; reproduces libviprs#41"]
fn pdfium_streaming_blueprint_1mb_budget_replicates_pr96() {
    let path = Path::new(FIXTURE_PDF);
    let (width_px, height_px) = predicted_dims(path, DPI);

    let strip_src = PdfiumStripSource::new(path, 1, DPI, width_px, height_px);
    let planner = PyramidPlanner::new(width_px, height_px, TILE_SIZE, 0, Layout::Google)
        .expect("PyramidPlanner::new failed");
    let plan = planner.plan();

    let sink = MemorySink::new();
    let mut engine = EngineConfig::default();
    engine.background_rgb = BACKGROUND_RGB;
    let config = StreamingConfig {
        memory_budget_bytes: MEMORY_BUDGET_BYTES_1MB,
        engine,
    };

    // If this panics with "range end index N out of range for slice of
    // length M" inside libviprs::streaming, we have reproduced #95/#96.
    let result = generate_pyramid_streaming(&strip_src, &plan, &sink, &config, &NoopObserver)
        .expect("generate_pyramid_streaming failed (non-panic error)");

    assert_eq!(
        result.tiles_produced,
        plan.total_tile_count(),
        "tile count mismatch (produced != planned)"
    );
}

/// Same call sequence at the 2MB budget that the PR author tried after
/// the 1MB budget panicked. Matches the report that doubling the budget
/// did not change the panic. Same `#[ignore]` rationale as the 1MB test.
#[test]
#[ignore = "intentionally panics; reproduces libviprs#41"]
fn pdfium_streaming_blueprint_2mb_budget_replicates_pr96_followup() {
    let path = Path::new(FIXTURE_PDF);
    let (width_px, height_px) = predicted_dims(path, DPI);

    let strip_src = PdfiumStripSource::new(path, 1, DPI, width_px, height_px);
    let planner = PyramidPlanner::new(width_px, height_px, TILE_SIZE, 0, Layout::Google)
        .expect("PyramidPlanner::new failed");
    let plan = planner.plan();

    let sink = MemorySink::new();
    let mut engine = EngineConfig::default();
    engine.background_rgb = BACKGROUND_RGB;
    let config = StreamingConfig {
        memory_budget_bytes: MEMORY_BUDGET_BYTES_2MB,
        engine,
    };

    let result = generate_pyramid_streaming(&strip_src, &plan, &sink, &config, &NoopObserver)
        .expect("generate_pyramid_streaming failed (non-panic error)");
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

/// Control: feed the *actual* rendered dimensions (from `render_page_pdfium`)
/// to both `PdfiumStripSource` and `PyramidPlanner`. If this passes while
/// the predicted-dims tests above panic, the bug is the prediction-vs-actual
/// dimension mismatch (and the fix in istracked/server is to render first
/// and use `raster.width()/height()`, mirroring the pre-PR code path).
#[test]
fn pdfium_streaming_blueprint_actual_dims_should_pass() {
    let path = Path::new(FIXTURE_PDF);
    let raster = render_page_pdfium(path, 1, DPI).expect("render_page_pdfium failed");
    let (width_px, height_px) = (raster.width(), raster.height());
    drop(raster); // PdfiumStripSource will re-render lazily.

    let strip_src = PdfiumStripSource::new(path, 1, DPI, width_px, height_px);
    let planner = PyramidPlanner::new(width_px, height_px, TILE_SIZE, 0, Layout::Google)
        .expect("PyramidPlanner::new failed");
    let plan = planner.plan();

    let sink = MemorySink::new();
    let mut engine = EngineConfig::default();
    engine.background_rgb = BACKGROUND_RGB;
    let config = StreamingConfig {
        memory_budget_bytes: MEMORY_BUDGET_BYTES_1MB,
        engine,
    };

    let result = generate_pyramid_streaming(&strip_src, &plan, &sink, &config, &NoopObserver)
        .expect("generate_pyramid_streaming failed (non-panic error)");
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}
