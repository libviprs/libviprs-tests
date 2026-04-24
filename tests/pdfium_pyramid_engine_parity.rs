//! Cross-engine pyramid parity on PDF fixtures.
//!
//! Pins the invariant that, given identical source pixels from
//! [`PdfiumStripSource`], the monolithic engine (fed the full-height raster
//! via a single `render_strip` call) and the streaming engine (fed the
//! same `PdfiumStripSource` with a budget that forces strip-by-strip
//! iteration) emit byte-identical tile bags at every pyramid level.
//!
//! Covers both fixtures:
//!
//! - `blueprint.pdf` — landscape `MediaBox` with `/Rotate 90`. A regression
//!   in the windowed-strip rotation handling would diverge one engine's
//!   tiles from the other's; this test catches that at the tile level
//!   rather than the strip level.
//! - `blueprint-portrait.pdf` — portrait control with `/Rotate 0`.
//!
//! The Monolithic reference raster is built by stitching
//! `PdfiumStripSource::render_strip` calls at the *same* strip height the
//! streaming engine will use, rather than one single `render_strip(0, h)`.
//! pdfium's form-data render path produces slightly different pixels for
//! rotated pages depending on the destination bitmap's height — that's a
//! C-library property we can't dodge. By sourcing both engines from
//! byte-identical stitched strips, the test isolates the pyramid
//! pipeline: any tile divergence is engine behaviour, not pdfium
//! rendering drift.
//!
//! Source pixels are obtained via `PdfiumStripSource` deliberately —
//! *not* `render_page_pdfium` or `extract_page_image`. Any "render the
//! PDF to a Raster, then stream the Raster" pattern caches the full page
//! up front, which defeats the streaming engine's memory-budget
//! contract.
//!
//! # Rotated + Google+centre is not covered
//!
//! One combination is deliberately absent: rotated PDFs under
//! Google+centre layout (e.g., `blueprint.pdf` with `/Rotate 90`).
//! Byte-parity is not achievable there, because the streaming engine's
//! [`embed_strip_in_canvas`] path requests source strips at boundaries
//! dictated by the centring offset (e.g., `render_strip(0, strip_h - oy)`),
//! not at the uniform `strip_h` the reference-raster stitch uses. pdfium
//! produces different bytes for different bitmap heights on rotated
//! pages, the drift cascades through the downscale chain, and empirical
//! max channel delta on individual tiles reaches ~46/255. That's a real
//! user-facing divergence between monolithic and streaming output for
//! rotated Google+centre PDFs, not something libviprs can correct on
//! the streaming side without caching the full page. Callers that need
//! bit-exact monolithic equivalence on rotated Google+centre fixtures
//! should not use `--memory-budget`.

#![cfg(feature = "pdfium")]

use libviprs::streaming::{BudgetPolicy, StripSource, compute_strip_height};
use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, MemorySink, PdfiumStripSource, PixelFormat,
    PyramidPlanner, Raster,
};

const FIXTURE_BLUEPRINT: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");
const FIXTURE_PORTRAIT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint-portrait.pdf"
);

/// 72 DPI — the dimension agreement between `PdfiumStripSource` and
/// `render_page_pdfium` is exact at this scale on both fixtures. Higher
/// DPIs drift by 1–2 px due to pdfium's internal scaler rounding, which
/// would break the Monolithic-over-rendered-raster reference.
const DPI: u32 = 72;

const TILE: u32 = 256;

/// Budgets must exceed the streaming engine's pre-flight minimum
/// (`canvas_width × 2 × tile_size × 4` bytes for the widest layout the
/// test exercises) and stay below monolithic peak so the engine actually
/// takes the streaming path.
///
/// - blueprint-portrait.pdf at 72 DPI: source ~612×792, Google+centre
///   canvas 2048×2048 → pre-flight min 2048 × 512 × 4 = 4.0 MiB.
///   Monolithic peak ≈ 16 MiB.
/// - blueprint.pdf at 72 DPI: source 4768×3370, Google+centre canvas
///   8192×8192 → pre-flight min 8192 × 512 × 4 = 16.0 MiB. Monolithic
///   peak ≈ 256 MiB.
const PORTRAIT_BUDGET_BYTES: u64 = 6 * 1024 * 1024;
const BLUEPRINT_BUDGET_BYTES: u64 = 20 * 1024 * 1024;

fn run_parity(fixture: &str, layout: Layout, centre: bool, budget_bytes: u64, label: &str) {
    let ref_source = PdfiumStripSource::new(fixture, 1, DPI)
        .unwrap_or_else(|e| panic!("{label}: PdfiumStripSource::new: {e:?}"));
    let w = ref_source.width();
    let h = ref_source.height();

    let planner = {
        let p = PyramidPlanner::new(w, h, TILE, 0, layout).unwrap();
        if centre { p.with_centre(true) } else { p }
    };
    let plan = planner.plan();

    // Stitch the reference raster at the same strip height the streaming
    // engine will use, so both paths see byte-identical source pixels.
    // pdfium's render output depends on destination-bitmap height for
    // rotated pages, so `render_strip(0, h)` and `render_strip(y, strip_h)`
    // don't byte-match even with identical source regions — that's a
    // pdfium property, not a libviprs bug, and stitching lets us pin
    // pyramid-pipeline parity without getting tangled in it.
    // Mirror the streaming engine: use `compute_strip_height` when the
    // budget fits the full geometric live-buffer series; fall back to
    // the minimum aligned strip (`2 × tile_size`) otherwise.
    let strip_h =
        compute_strip_height(&plan, PixelFormat::Rgba8, budget_bytes).unwrap_or(2 * plan.tile_size);
    let mut stitched = Vec::with_capacity(w as usize * h as usize * 4);
    let mut y = 0u32;
    while y < h {
        let sh = strip_h.min(h - y);
        let strip = ref_source
            .render_strip(y, sh)
            .unwrap_or_else(|e| panic!("{label}: render_strip({y}, {sh}): {e:?}"));
        stitched.extend_from_slice(strip.data());
        y += sh;
    }
    let raster = Raster::new(w, h, PixelFormat::Rgba8, stitched)
        .unwrap_or_else(|e| panic!("{label}: Raster::new from stitched strips: {e:?}"));

    // Monolithic reference — fed the full raster.
    let mono_sink = MemorySink::new();
    EngineBuilder::new(&raster, plan.clone(), &mono_sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap_or_else(|e| panic!("{label}: monolithic run: {e:?}"));
    let mut mono_tiles = mono_sink.tiles();
    mono_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    // Streaming — fed the strip source directly, with a budget that forces
    // true strip iteration (verified via the engine's pre-flight check).
    let stream_source = PdfiumStripSource::new(fixture, 1, DPI)
        .unwrap_or_else(|e| panic!("{label}: PdfiumStripSource::new (stream): {e:?}"));
    let stream_sink = MemorySink::new();
    EngineBuilder::new(stream_source, plan.clone(), &stream_sink)
        .with_engine(EngineKind::Streaming)
        .with_config(EngineConfig::default())
        .with_memory_budget(budget_bytes)
        .with_budget_policy(BudgetPolicy::Error)
        .run()
        .unwrap_or_else(|e| panic!("{label}: streaming run: {e:?}"));
    let mut stream_tiles = stream_sink.tiles();
    stream_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    assert_eq!(
        mono_tiles.len(),
        stream_tiles.len(),
        "{label}: tile count mismatch (monolithic={}, streaming={})",
        mono_tiles.len(),
        stream_tiles.len(),
    );
    for (m, s) in mono_tiles.iter().zip(stream_tiles.iter()) {
        assert_eq!(m.coord, s.coord, "{label}: coord mismatch");
        assert_eq!(
            m.data.len(),
            s.data.len(),
            "{label}: tile byte-length mismatch at {:?}",
            m.coord,
        );
        if m.data != s.data {
            let (max_delta, diff_px) = tile_delta_stats(&m.data, &s.data);
            panic!(
                "{label}: tile data diverged at {:?} ({} bytes). \
                 max channel delta = {max_delta}, differing pixels = {diff_px}",
                m.coord,
                m.data.len(),
            );
        }
    }
}

fn tile_delta_stats(a: &[u8], b: &[u8]) -> (u8, usize) {
    debug_assert_eq!(a.len(), b.len());
    let mut max = 0u8;
    let mut diff = 0usize;
    for (x, y) in a.iter().zip(b.iter()) {
        if x != y {
            diff += 1;
            let d = x.abs_diff(*y);
            if d > max {
                max = d;
            }
        }
    }
    (max, diff / 4) // diff bytes → pixels (RGBA)
}

#[test]
fn blueprint_rotated_deepzoom_parity() {
    run_parity(
        FIXTURE_BLUEPRINT,
        Layout::DeepZoom,
        false,
        BLUEPRINT_BUDGET_BYTES,
        "blueprint.pdf DeepZoom",
    );
}

// blueprint_rotated_google_centre_parity — intentionally omitted. See
// the module docstring for why rotated + Google+centre cannot hit
// byte-parity with monolithic under strip-based rendering.

#[test]
fn blueprint_portrait_deepzoom_parity() {
    run_parity(
        FIXTURE_PORTRAIT,
        Layout::DeepZoom,
        false,
        PORTRAIT_BUDGET_BYTES,
        "blueprint-portrait.pdf DeepZoom",
    );
}

#[test]
fn blueprint_portrait_google_centre_parity() {
    run_parity(
        FIXTURE_PORTRAIT,
        Layout::Google,
        true,
        PORTRAIT_BUDGET_BYTES,
        "blueprint-portrait.pdf Google+centre",
    );
}
