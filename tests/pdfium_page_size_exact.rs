//! A PDF page rasterises at exactly the size libvips gives it (libviprs#1199).
//!
//! libvips (`foreign/pdfiumload.c`) works out `total_scale = dpi / 72.0` in
//! double, rounds `page_dim * total_scale` to nearest with ties to even, and
//! renders the page stretched into a bitmap of exactly that size. libviprs
//! 0.5.x truncated through `f32` and then let pdfium aspect-fit, so a raster
//! came out 0 to 2 px smaller than the page at that DPI, and which page sizes
//! it hit was a matter of luck (Letter at 300 dpi was 2549x3299, not
//! 2550x3300; A3 was a pixel short on one axis).
//!
//! These tests drive drawing-heavy lopdf PDFs (see `common::blank_pdf`) through every route that reports or
//! produces a page raster and require the libvips size from each:
//!
//! - `render_page_pdfium` and `render_page_pdfium_budgeted`
//! - `PdfiumStripSource` cached (`new`) and streaming (`new_streaming`), plus
//!   `new_with_budget` / `new_streaming_with_budget` under both policies
//! - `StripSource::render_strip` on both modes
//!
//! The size comes from [`common::blank_pdf::libvips_dims`], the libvips formula
//! written out longhand and pinned by name for a handful of sizes, so it is an
//! oracle that does not go through `PageSizing`. The same table is checked
//! against a live libvips in `page_size_libvips_pdfium_parity.rs`.
//!
//! Rendering the 600 dpi row for every size would need gigabytes per raster,
//! so the routes that rasterise the whole page only run on combinations under
//! [`MAX_RENDER_PX`], and the pixel-content checks (edges, landmarks) on the
//! [`REPRESENTATIVE`] sheets only. The streaming probe renders nothing, so it covers the
//! whole table at every DPI.

#![cfg(feature = "pdfium")]

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use common::blank_pdf::{
    DPIS, FRAME_PT, PageSpec, SIZES, landmarks, libvips_dims, slug, write_blank_pdf,
};
use libviprs::streaming::{BudgetPolicy, StripSource};
use libviprs::{
    Layout, PageSizing, PdfiumRenderMode, PdfiumStripSource, PyramidPlanner, Raster,
    render_page_pdfium, render_page_pdfium_budgeted,
};

/// Largest raster, in pixels, a test here rasterises in one piece.
const MAX_RENDER_PX: u64 = 24_000_000;

/// A budget no page in the table comes near, for the `*_with_budget` routes
/// where the budget is not what is being tested.
const HUGE_BUDGET: u64 = 1 << 40;

struct Page {
    label: &'static str,
    w: f64,
    h: f64,
    path: PathBuf,
}

/// The size table as PDFs on disk, written once for the whole binary.
fn pages() -> &'static [Page] {
    static PAGES: OnceLock<(tempfile::TempDir, Vec<Page>)> = OnceLock::new();
    &PAGES
        .get_or_init(|| {
            let dir = tempfile::tempdir().expect("tempdir");
            let pages = SIZES
                .iter()
                .map(|&(label, w, h)| Page {
                    label,
                    w,
                    h,
                    path: write_blank_pdf(
                        dir.path(),
                        &format!("{}.pdf", slug(label)),
                        &PageSpec::new(w, h),
                    ),
                })
                .collect();
            (dir, pages)
        })
        .1
}

fn scratch_dir() -> &'static Path {
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    DIR.get_or_init(|| tempfile::tempdir().expect("tempdir"))
        .path()
}

/// Write a one-off PDF under a unique name.
fn write_spec(name: &str, spec: &PageSpec) -> PathBuf {
    write_blank_pdf(scratch_dir(), &format!("{name}.pdf"), spec)
}

fn px(dims: (u32, u32)) -> u64 {
    dims.0 as u64 * dims.1 as u64
}

fn dims_of(r: &Raster) -> (u32, u32) {
    (r.width(), r.height())
}

fn src_dims(s: &PdfiumStripSource) -> (u32, u32) {
    (s.width(), s.height())
}

// ---------------------------------------------------------------------------
// The oracle itself
// ---------------------------------------------------------------------------

/// The issue pins these by name at 300 dpi; they are the libvips numbers.
#[test]
fn oracle_pins_the_300_dpi_sizes_from_the_issue() {
    let pins: &[(&str, (u32, u32))] = &[
        ("Letter", (2550, 3300)),
        ("Tabloid", (3300, 5100)),
        ("Tabloid landscape", (5100, 3300)),
        ("ARCH A", (2700, 3600)),
        ("ARCH B", (3600, 5400)),
        ("ARCH C", (5400, 7200)),
        ("ARCH D", (7200, 10800)),
        ("ARCH E", (10800, 14400)),
        ("ANSI D", (6600, 10200)),
        ("A4", (2480, 3508)),
        ("A3", (3508, 4961)),
        ("A2", (4961, 7016)),
        ("A1", (7016, 9933)),
        ("A0", (9933, 14043)),
    ];
    for &(label, want) in pins {
        let &(_, w, h) = SIZES.iter().find(|(l, _, _)| *l == label).unwrap();
        assert_eq!(libvips_dims(w, h, 300), want, "{label} at 300 dpi");
    }
}

// ---------------------------------------------------------------------------
// PageSizing
// ---------------------------------------------------------------------------

#[test]
fn exact_is_the_default_sizing() {
    assert_eq!(PageSizing::default(), PageSizing::Exact);
}

#[test]
fn exact_pixel_dims_is_the_libvips_formula_for_the_whole_table() {
    for &(label, w, h) in SIZES {
        for &dpi in DPIS {
            assert_eq!(
                PageSizing::Exact.pixel_dims(w as f32 as f64, h as f32 as f64, dpi),
                libvips_dims(w, h, dpi),
                "{label} at {dpi} dpi",
            );
        }
    }
}

/// `f64::round` rounds ties away from zero and libvips' `rint` rounds them to
/// even, so a width that lands exactly on .5 tells the two apart. 100.5 pt at
/// 72 dpi is 100.5 px: libvips gives 100, and 101.5 gives 102.
#[test]
fn exact_rounds_ties_to_even_like_rint() {
    assert_eq!(PageSizing::Exact.pixel_dims(100.5, 101.5, 72), (100, 102));
    assert_eq!(PageSizing::Exact.pixel_dims(0.5, 2.5, 72), (0, 2));
    assert_eq!(libvips_dims(100.5, 101.5, 72), (100, 102));
}

/// What 0.5.1 rasterises at 300 dpi (`render_page_pdfium`), per page. Letter to
/// ARCH E and A3, A1 are the issue's table (pdfium-8085); the rest were measured
/// on 0.5.1 with pdfium-8054, the build this repo pins. They are what
/// `LegacyTruncated` has to keep producing. ARCH D and ARCH E are the issue's
/// numbers and are only checked where the raster is small enough to allocate.
const LEGACY_RENDERED_300: &[(&str, (u32, u32))] = &[
    ("Letter", (2549, 3299)),
    ("Tabloid", (3299, 5098)),
    ("Tabloid landscape", (5098, 3299)),
    ("ARCH A", (2699, 3599)),
    ("ARCH B", (3599, 5399)),
    ("ARCH C", (5399, 7199)),
    ("ARCH D", (7199, 10799)),
    ("ARCH E", (10799, 14399)),
    ("ANSI D", (6599, 10198)),
    ("Legal", (2550, 4200)),
    ("A4", (2480, 3507)),
    ("A3", (3507, 4959)),
    ("A2", (4960, 7015)),
    ("A1", (7015, 9932)),
    ("600x800", (2500, 3333)),
    ("700x1000", (2916, 4166)),
];

/// What 0.5.1 `new_streaming` reported at 300 dpi: the f32 arithmetic, before
/// pdfium's aspect-fit, so it differs from the rendered size above.
const LEGACY_STREAMING_300: &[(&str, (u32, u32))] = &[
    ("Letter", (2550, 3299)),
    ("Tabloid", (3299, 5100)),
    ("Tabloid landscape", (5100, 3299)),
    ("ARCH A", (2700, 3599)),
    ("ARCH B", (3599, 5400)),
    ("ARCH C", (5400, 7199)),
    ("ANSI D", (6599, 10200)),
    ("Legal", (2550, 4200)),
    ("A4", (2480, 3507)),
    ("A3", (3507, 4960)),
    ("A2", (4960, 7015)),
    ("A1", (7015, 9933)),
    ("600x800", (2500, 3333)),
    ("700x1000", (2916, 4166)),
];

/// Largest raster the legacy table renders. Bigger than [`MAX_RENDER_PX`]
/// because the issue's numbers for ARCH D and ANSI D are worth checking.
const MAX_LEGACY_RENDER_PX: u64 = 80_000_000;

fn size_of(label: &str) -> (f64, f64) {
    let &(_, w, h) = SIZES.iter().find(|(l, _, _)| *l == label).unwrap();
    (w, h)
}

/// `pixel_dims` under `LegacyTruncated` is a 0.5.x size: either the f32
/// arithmetic `new_streaming` reported or the aspect-fitted size pdfium
/// rendered (the core may return either, the issue only says the old pipeline
/// is kept). Never above the libvips size, never more than the 2 px the issue
/// measured under it.
#[test]
fn legacy_truncated_pixel_dims_are_a_050_size() {
    for &(label, streaming) in LEGACY_STREAMING_300 {
        let rendered = LEGACY_RENDERED_300
            .iter()
            .find(|(l, _)| *l == label)
            .unwrap()
            .1;
        let (w, h) = size_of(label);
        let got = PageSizing::LegacyTruncated.pixel_dims(w as f32 as f64, h as f32 as f64, 300);
        assert!(
            got == streaming || got == rendered,
            "{label} at 300 dpi: {got:?} is neither the 0.5.x streaming {streaming:?} nor rendered {rendered:?} size",
        );
    }
    for &(label, w, h) in SIZES {
        for &dpi in DPIS {
            let (w, h) = (w as f32 as f64, h as f32 as f64);
            let exact = PageSizing::Exact.pixel_dims(w, h, dpi);
            let legacy = PageSizing::LegacyTruncated.pixel_dims(w, h, dpi);
            assert!(
                legacy.0 <= exact.0
                    && legacy.1 <= exact.1
                    && exact.0 - legacy.0 <= 2
                    && exact.1 - legacy.1 <= 2,
                "{label} @ {dpi}: legacy {legacy:?} against exact {exact:?}",
            );
        }
    }
}

/// Rasterised with `LegacyTruncated`, a page still comes out the 0.5.x size,
/// including the second rounding pdfium's aspect-fit adds.
#[test]
fn legacy_truncated_render_reproduces_050_sizes() {
    let mut ran = 0;
    for &(label, want) in LEGACY_RENDERED_300 {
        if px(want) > MAX_LEGACY_RENDER_PX {
            continue;
        }
        ran += 1;
        let page = pages().iter().find(|p| p.label == label).unwrap();
        let r = libviprs::render_page_pdfium_with(&page.path, 1, 300, PageSizing::LegacyTruncated)
            .unwrap_or_else(|e| panic!("{label}: legacy render failed: {e:?}"));
        assert_eq!(
            dims_of(&r),
            want,
            "{label}: LegacyTruncated render at 300 dpi"
        );
    }
    assert!(ran >= 14, "legacy table skipped too much ({ran} ran)");
    // Measured on 0.5.1: pdfium's aspect-fit loses a column here as well.
    let letter = pages().iter().find(|p| p.label == "Letter").unwrap();
    let r = libviprs::render_page_pdfium_with(&letter.path, 1, 150, PageSizing::LegacyTruncated)
        .unwrap();
    assert_eq!(dims_of(&r), (1274, 1649));
    // And the default stays the libvips size on the very same page.
    let exact = libviprs::render_page_pdfium_with(&letter.path, 1, 150, PageSizing::Exact).unwrap();
    assert_eq!(dims_of(&exact), (1275, 1650));
}

/// Under `LegacyTruncated` the budgeted render and the cached source come out
/// at the rendered 0.5.x size, and the streaming source reports the arithmetic
/// size 0.5.x streaming did.
#[test]
fn legacy_truncated_budgeted_and_source_routes_keep_their_050_behaviour() {
    for label in ["Letter", "Tabloid", "A3"] {
        let page = pages().iter().find(|p| p.label == label).unwrap();
        let rendered = LEGACY_RENDERED_300
            .iter()
            .find(|(l, _)| *l == label)
            .unwrap()
            .1;
        let arithmetic = LEGACY_STREAMING_300
            .iter()
            .find(|(l, _)| *l == label)
            .unwrap()
            .1;
        let b = libviprs::render_page_pdfium_budgeted_with(
            &page.path,
            1,
            300,
            u64::MAX,
            PageSizing::LegacyTruncated,
        )
        .unwrap();
        assert_eq!(dims_of(&b.raster), rendered, "{label}: budgeted legacy");
        let streaming = strip_source(
            &page.path,
            300,
            PdfiumRenderMode::Streaming,
            PageSizing::LegacyTruncated,
        );
        assert_eq!(
            src_dims(&streaming),
            arithmetic,
            "{label}: streaming legacy"
        );
        assert_eq!(streaming.sizing(), PageSizing::LegacyTruncated);
        let cached = strip_source(
            &page.path,
            300,
            PdfiumRenderMode::CachedFullPage,
            PageSizing::LegacyTruncated,
        );
        assert_eq!(src_dims(&cached), rendered, "{label}: cached legacy");
        assert_eq!(cached.sizing(), PageSizing::LegacyTruncated);
    }
}

/// Build a source at an explicit mode and sizing through the builder.
fn strip_source(
    path: &Path,
    dpi: u32,
    mode: PdfiumRenderMode,
    sizing: PageSizing,
) -> PdfiumStripSource {
    PdfiumStripSource::builder(path, 1, dpi)
        .mode(mode)
        .sizing(sizing)
        .build()
        .unwrap_or_else(|e| panic!("{} {mode:?} {sizing:?} at {dpi}: {e:?}", path.display()))
}

#[test]
fn builder_defaults_to_exact_sizing() {
    let letter = pages().iter().find(|p| p.label == "Letter").unwrap();
    let s = PdfiumStripSource::new_streaming(&letter.path, 1, 300).unwrap();
    assert_eq!(s.sizing(), PageSizing::Exact);
    let s = PdfiumStripSource::new(&letter.path, 1, 72).unwrap();
    assert_eq!(s.sizing(), PageSizing::Exact);
}

// ---------------------------------------------------------------------------
// Probe, planner input and source size, every page at every DPI
// ---------------------------------------------------------------------------

/// The probe is what `new_streaming` reports without rendering anything. It
/// has to be the libvips size, and so does the plan built from it.
#[test]
fn probe_and_planner_input_are_the_libvips_size() {
    for page in pages() {
        for &dpi in DPIS {
            let want = libvips_dims(page.w, page.h, dpi);
            let s = PdfiumStripSource::new_streaming(&page.path, 1, dpi)
                .unwrap_or_else(|e| panic!("{} @ {dpi}: {e:?}", page.label));
            assert_eq!(
                src_dims(&s),
                want,
                "{} @ {dpi}: streaming probe",
                page.label
            );
            assert_eq!(s.dpi(), dpi);

            let plan = PyramidPlanner::new(s.width(), s.height(), 256, 0, Layout::DeepZoom)
                .unwrap()
                .plan();
            assert_eq!(
                (plan.image_width, plan.image_height),
                want,
                "{} @ {dpi}: planner input",
                page.label
            );
            assert_eq!(
                PageSizing::Exact.pixel_dims(page.w as f32 as f64, page.h as f32 as f64, dpi),
                want,
                "{} @ {dpi}: PageSizing::Exact",
                page.label
            );
        }
    }
}

#[test]
fn streaming_with_budget_reports_the_libvips_size_under_both_policies() {
    for page in pages() {
        for &dpi in DPIS {
            let want = libvips_dims(page.w, page.h, dpi);
            let error = PdfiumStripSource::new_streaming_with_budget(
                &page.path,
                1,
                dpi,
                256,
                HUGE_BUDGET,
                BudgetPolicy::Error,
            )
            .unwrap_or_else(|e| panic!("{} @ {dpi} Error: {e:?}", page.label));
            assert_eq!(
                src_dims(&error),
                want,
                "{} @ {dpi}: Error policy",
                page.label
            );
            assert_eq!(error.dpi(), dpi);

            let auto = PdfiumStripSource::new_streaming_with_budget(
                &page.path,
                1,
                dpi,
                256,
                HUGE_BUDGET,
                BudgetPolicy::AutoAdjustDpi { min_dpi: 36 },
            )
            .unwrap_or_else(|e| panic!("{} @ {dpi} Auto: {e:?}", page.label));
            assert_eq!(
                src_dims(&auto),
                want,
                "{} @ {dpi}: AutoAdjustDpi policy",
                page.label
            );
            assert_eq!(
                auto.dpi(),
                dpi,
                "a budget nothing comes near must not lower the dpi"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Rendered rasters
// ---------------------------------------------------------------------------

#[test]
fn every_render_route_returns_the_libvips_size() {
    let mut ran = 0;
    for page in pages() {
        for &dpi in DPIS {
            let want = libvips_dims(page.w, page.h, dpi);
            if px(want) > MAX_RENDER_PX {
                continue;
            }
            ran += 1;
            let label = format!("{} @ {dpi}", page.label);

            let r = render_page_pdfium(&page.path, 1, dpi)
                .unwrap_or_else(|e| panic!("{label}: render_page_pdfium: {e:?}"));
            assert_eq!(dims_of(&r), want, "{label}: render_page_pdfium");

            let b = render_page_pdfium_budgeted(&page.path, 1, dpi, u64::MAX)
                .unwrap_or_else(|e| panic!("{label}: budgeted: {e:?}"));
            assert_eq!(
                dims_of(&b.raster),
                want,
                "{label}: render_page_pdfium_budgeted"
            );
            assert_eq!(b.dpi_used, dpi, "{label}: budgeted dpi");
            assert!(!b.capped, "{label}: budgeted must not cap");

            let cached = PdfiumStripSource::new(&page.path, 1, dpi).unwrap();
            assert_eq!(src_dims(&cached), want, "{label}: cached source");

            for (name, policy) in [
                ("Error", BudgetPolicy::Error),
                ("AutoAdjustDpi", BudgetPolicy::AutoAdjustDpi { min_dpi: 36 }),
            ] {
                let s = PdfiumStripSource::new_with_budget(
                    &page.path,
                    1,
                    dpi,
                    256,
                    HUGE_BUDGET,
                    policy,
                )
                .unwrap_or_else(|e| panic!("{label}: new_with_budget {name}: {e:?}"));
                assert_eq!(src_dims(&s), want, "{label}: new_with_budget {name}");
            }
        }
    }
    assert!(
        ran >= 40,
        "the render cap skipped too much ({ran} combinations ran)"
    );
}

/// `render_strip` hands back a raster as wide as the source says and as tall as
/// asked, at the top, in the middle and at the very bottom, on both modes.
#[test]
fn render_strip_matches_the_reported_size_on_both_modes() {
    for page in pages() {
        for &dpi in DPIS {
            let want = libvips_dims(page.w, page.h, dpi);
            let streaming = PdfiumStripSource::new_streaming(&page.path, 1, dpi).unwrap();
            let cached = if px(want) <= MAX_RENDER_PX {
                Some(PdfiumStripSource::new(&page.path, 1, dpi).unwrap())
            } else {
                None
            };
            let rows = 8.min(want.1);
            for (mode, s) in [("streaming", Some(&streaming)), ("cached", cached.as_ref())] {
                let Some(s) = s else { continue };
                for y in [0, (want.1 - rows) / 2, want.1 - rows] {
                    let strip = s.render_strip(y, rows).unwrap_or_else(|e| {
                        panic!("{} @ {dpi} {mode} strip {y}: {e:?}", page.label)
                    });
                    assert_eq!(
                        dims_of(&strip),
                        (want.0, rows),
                        "{} @ {dpi} {mode}: strip at y={y}",
                        page.label
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Budgets: the libvips size is what the budget is checked against
// ---------------------------------------------------------------------------

#[test]
fn budgeted_render_fits_the_budget_with_the_libvips_size_at_the_dpi_it_chose() {
    for label in ["Letter", "Tabloid", "ARCH D", "ANSI D", "A3", "A1"] {
        let page = pages().iter().find(|p| p.label == label).unwrap();
        for max_pixels in [2_000_000u64, 9_000_000, 25_000_000] {
            let b = render_page_pdfium_budgeted(&page.path, 1, 300, max_pixels)
                .unwrap_or_else(|e| panic!("{label} / {max_pixels}: {e:?}"));
            let full = libvips_dims(page.w, page.h, 300);
            assert_eq!(
                b.capped,
                px(full) > max_pixels,
                "{label} / {max_pixels}: capped only when 300 dpi ({}x{}) does not fit",
                full.0,
                full.1
            );
            assert_eq!(
                dims_of(&b.raster),
                libvips_dims(page.w, page.h, b.dpi_used),
                "{label} / {max_pixels}: size at the dpi it chose ({})",
                b.dpi_used
            );
            assert!(
                px(dims_of(&b.raster)) <= max_pixels,
                "{label} / {max_pixels}: {}x{} is over the budget",
                b.raster.width(),
                b.raster.height()
            );
        }
    }
}

/// `AutoAdjustDpi` settles on a DPI whose worst-case strip fits the budget,
/// measured with the libvips width, and the source then has that width.
#[test]
fn auto_adjust_dpi_budget_is_checked_on_the_libvips_width() {
    for label in ["Letter", "Tabloid", "A3", "A1"] {
        let page = pages().iter().find(|p| p.label == label).unwrap();
        let min_strip = 256u32;
        let budget = 3_000_000u64;
        let s = PdfiumStripSource::new_streaming_with_budget(
            &page.path,
            1,
            600,
            min_strip,
            budget,
            BudgetPolicy::AutoAdjustDpi { min_dpi: 36 },
        )
        .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert!(
            s.dpi() < 600,
            "{label}: 600 dpi cannot fit a {budget} byte strip"
        );
        assert_eq!(
            src_dims(&s),
            libvips_dims(page.w, page.h, s.dpi()),
            "{label}"
        );
        assert!(
            s.width() as u64 * min_strip as u64 * 4 <= budget,
            "{label}: chosen dpi {} still over budget at width {}",
            s.dpi(),
            s.width()
        );
    }
}

/// With `BudgetPolicy::Error` the check runs on the libvips width: a strip that
/// only fitted because the width was truncated is refused.
#[test]
fn error_budget_is_checked_on_the_libvips_width() {
    let letter = pages().iter().find(|p| p.label == "Letter").unwrap();
    let (w, _) = libvips_dims(letter.w, letter.h, 300);
    let exactly = w as u64 * 256 * 4;
    PdfiumStripSource::new_streaming_with_budget(
        &letter.path,
        1,
        300,
        256,
        exactly,
        BudgetPolicy::Error,
    )
    .expect("a budget of exactly one libvips-width strip fits");
    let err = PdfiumStripSource::new_streaming_with_budget(
        &letter.path,
        1,
        300,
        256,
        exactly - 1,
        BudgetPolicy::Error,
    );
    assert!(
        err.is_err(),
        "one byte under the libvips-width strip must be refused"
    );
}

// ---------------------------------------------------------------------------
// Page geometry: rotation, MediaBox origin, CropBox, UserUnit
// ---------------------------------------------------------------------------

/// The sheets the pixel-content checks run on: portrait and landscape, integer
/// and non-integer points, small to large.
const REPRESENTATIVE: &[&str] = &[
    "Letter",
    "Legal",
    "Tabloid landscape",
    "ANSI C",
    "ARCH B",
    "B4",
    "A4",
    "A3",
    "A3 landscape",
];

fn representative() -> impl Iterator<Item = &'static Page> {
    pages().iter().filter(|p| REPRESENTATIVE.contains(&p.label))
}

/// Pages whose shape matters: integer, non-integer and a tie-prone one.
const GEOMETRY_SIZES: &[(&str, f64, f64)] = &[
    ("Letter", 612.0, 792.0),
    ("A3", 841.89, 1190.551),
    ("600x800", 600.0, 800.0),
    ("tie", 100.5, 101.5),
];

/// Check one PDF on every route against `want(dpi)`.
fn check_all_routes(name: &str, path: &Path, want: impl Fn(u32) -> (u32, u32)) {
    for &dpi in DPIS {
        let want = want(dpi);
        let label = format!("{name} @ {dpi}");
        let probe = PdfiumStripSource::new_streaming(path, 1, dpi)
            .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_eq!(src_dims(&probe), want, "{label}: streaming probe");
        if px(want) > MAX_RENDER_PX {
            continue;
        }
        let r = render_page_pdfium(path, 1, dpi).unwrap();
        assert_eq!(dims_of(&r), want, "{label}: render_page_pdfium");
        let cached = PdfiumStripSource::new(path, 1, dpi).unwrap();
        assert_eq!(src_dims(&cached), want, "{label}: cached source");
        let strip = probe.render_strip(0, 4.min(want.1)).unwrap();
        assert_eq!(strip.width(), want.0, "{label}: strip width");
    }
}

#[test]
fn rotate_90_and_270_swap_the_axes() {
    for &(label, w, h) in GEOMETRY_SIZES {
        for rotate in [90, 270] {
            let path = write_spec(
                &format!("rot{rotate}_{}", slug(label)),
                &PageSpec::new(w, h).rotate(rotate),
            );
            check_all_routes(&format!("{label} /Rotate {rotate}"), &path, |dpi| {
                libvips_dims(h, w, dpi)
            });
        }
    }
}

#[test]
fn rotate_180_keeps_the_axes() {
    for &(label, w, h) in GEOMETRY_SIZES {
        let path = write_spec(
            &format!("rot180_{}", slug(label)),
            &PageSpec::new(w, h).rotate(180),
        );
        check_all_routes(&format!("{label} /Rotate 180"), &path, |dpi| {
            libvips_dims(w, h, dpi)
        });
    }
}

#[test]
fn nonzero_mediabox_origin_does_not_change_the_size() {
    for &(label, w, h) in GEOMETRY_SIZES {
        for (ox, oy) in [(100.0, 50.0), (-36.5, -72.25)] {
            let path = write_spec(
                &format!("origin_{}_{ox}_{oy}", slug(label)),
                &PageSpec::new(w, h).origin(ox, oy),
            );
            check_all_routes(&format!("{label} origin ({ox},{oy})"), &path, |dpi| {
                libvips_dims(w, h, dpi)
            });
        }
    }
}

/// pdfium renders the CropBox intersected with the MediaBox, and the MediaBox
/// here is larger than the CropBox on every side.
#[test]
fn cropbox_defines_the_page_size() {
    for &(label, w, h) in GEOMETRY_SIZES {
        let path = write_spec(
            &format!("crop_{}", slug(label)),
            &PageSpec::new(w, h).crop_box(),
        );
        check_all_routes(&format!("{label} CropBox"), &path, |dpi| {
            libvips_dims(w, h, dpi)
        });
    }
}

#[test]
fn cropbox_with_rotate_and_origin_still_swaps_and_sizes_correctly() {
    let (w, h) = (841.89, 1190.551);
    let path = write_spec(
        "crop_rot_origin",
        &PageSpec::new(w, h).crop_box().rotate(90).origin(40.0, 60.0),
    );
    check_all_routes("A3 CropBox /Rotate 90 origin", &path, |dpi| {
        libvips_dims(h, w, dpi)
    });
}

/// `/UserUnit` scales the nominal point in the PDF spec; pdfium, libvips and
/// pdftoppm all ignore it, and so do we.
#[test]
fn user_unit_is_ignored() {
    for &(label, w, h) in GEOMETRY_SIZES {
        for unit in [2.0, 0.5] {
            let path = write_spec(
                &format!("uu_{}_{unit}", slug(label)),
                &PageSpec::new(w, h).user_unit(unit),
            );
            check_all_routes(&format!("{label} UserUnit {unit}"), &path, |dpi| {
                libvips_dims(w, h, dpi)
            });
        }
    }
}

// ---------------------------------------------------------------------------
// The page is stretched into the bitmap: ink on every edge
// ---------------------------------------------------------------------------

fn is_dark(r: &Raster, x: u32, y: u32) -> bool {
    let i = ((y * r.width() + x) * 4) as usize;
    r.data()[i] < 128 && r.data()[i + 1] < 128 && r.data()[i + 2] < 128
}

/// Every page carries a 2 pt black border flush with its edge. If the raster were padded
/// with a blank row or column, or the page shifted, or the right or bottom edge
/// left white (the integer-point truncation a matrix-only render would give a
/// page like A3), an edge row or column would be white.
fn assert_frame_on_every_edge(label: &str, r: &Raster) {
    let (w, h) = dims_of(r);
    // Corners get the frame from two sides, so leave a frame's width of margin
    // for any anti-aliasing there and check everything in between.
    let m = FRAME_PT as u32 * 2;
    assert!(w > 2 * m && h > 2 * m, "{label}: raster too small to check");
    for x in m..w - m {
        assert!(
            is_dark(r, x, 0),
            "{label}: first row is blank at x={x} ({w}x{h})"
        );
        assert!(
            is_dark(r, x, h - 1),
            "{label}: last row is blank at x={x} ({w}x{h})"
        );
    }
    for y in m..h - m {
        assert!(
            is_dark(r, 0, y),
            "{label}: first column is blank at y={y} ({w}x{h})"
        );
        assert!(
            is_dark(r, w - 1, y),
            "{label}: last column is blank at y={y} ({w}x{h})"
        );
    }
}

#[test]
fn frame_reaches_every_edge_of_a_full_page_render() {
    for page in representative() {
        for dpi in [72, 96, 150] {
            if px(libvips_dims(page.w, page.h, dpi)) > MAX_RENDER_PX {
                continue;
            }
            let label = format!("{} @ {dpi}", page.label);
            assert_frame_on_every_edge(&label, &render_page_pdfium(&page.path, 1, dpi).unwrap());
        }
    }
}

#[test]
fn frame_reaches_every_edge_of_the_streaming_strips() {
    for page in representative() {
        for dpi in [72, 150] {
            let want = libvips_dims(page.w, page.h, dpi);
            if px(want) > MAX_RENDER_PX {
                continue;
            }
            let label = format!("{} @ {dpi}", page.label);
            let s = PdfiumStripSource::new_streaming(&page.path, 1, dpi).unwrap();
            // Measure from the source so a wrong size reads as a size mismatch
            // below, not as an index panic in the edge scan.
            let (w, h) = src_dims(&s);
            assert_eq!((w, h), want, "{label}: streaming size");
            // The whole page as one strip, so all four edges are in it.
            assert_frame_on_every_edge(
                &format!("{label} (one strip)"),
                &s.render_strip(0, h).unwrap(),
            );

            // The same page in strips: the first strip holds the top row, the
            // last the bottom row, and each holds both side columns.
            let rows = 64.min(h);
            let first = s.render_strip(0, rows).unwrap();
            let last = s.render_strip(h - rows, rows).unwrap();
            let m = FRAME_PT as u32 * 2;
            for x in m..w - m {
                assert!(is_dark(&first, x, 0), "{label}: first row blank at x={x}");
                assert!(
                    is_dark(&last, x, rows - 1),
                    "{label}: last row blank at x={x}"
                );
            }
            for (strip, lo, hi) in [(&first, m, rows), (&last, 0, rows - m)] {
                for y in lo..hi {
                    assert!(
                        is_dark(strip, 0, y),
                        "{label}: first column blank at strip row {y}"
                    );
                    assert!(
                        is_dark(strip, w - 1, y),
                        "{label}: last column blank at strip row {y}"
                    );
                }
            }
        }
    }
}

#[test]
fn frame_reaches_every_edge_on_rotated_and_cropped_pages() {
    let a3 = (841.89, 1190.551);
    let specs = [
        ("A3 /Rotate 90", PageSpec::new(a3.0, a3.1).rotate(90)),
        ("A3 /Rotate 270", PageSpec::new(a3.0, a3.1).rotate(270)),
        ("A3 origin", PageSpec::new(a3.0, a3.1).origin(100.0, 50.0)),
        ("A3 CropBox", PageSpec::new(a3.0, a3.1).crop_box()),
    ];
    for (label, spec) in specs {
        let path = write_spec(&format!("frame_{}", slug(label)), &spec);
        for dpi in [72, 150] {
            let label = format!("{label} @ {dpi}");
            assert_frame_on_every_edge(&label, &render_page_pdfium(&path, 1, dpi).unwrap());
            let s = PdfiumStripSource::new_streaming(&path, 1, dpi).unwrap();
            assert_frame_on_every_edge(
                &format!("{label} (strip)"),
                &s.render_strip(0, s.height()).unwrap(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Landmarks: nothing flipped, rotated, shifted or stretched
// ---------------------------------------------------------------------------

/// Device position of page point `(x, y)` (points from the lower-left of the
/// visible box) in a `bw` x `bh` bitmap of a `w` x `h` page under `/Rotate rot`.
/// The page is stretched into the bitmap, so each axis has its own scale.
fn device(rot: i64, (w, h): (f64, f64), (bw, bh): (f64, f64), (x, y): (f64, f64)) -> (f64, f64) {
    match rot {
        0 => (x * bw / w, (h - y) * bh / h),
        90 => (y * bw / h, x * bh / w),
        180 => ((w - x) * bw / w, y * bh / h),
        270 => ((h - y) * bw / h, (w - x) * bh / w),
        _ => unreachable!(),
    }
}

fn dark_at(r: &Raster, (x, y): (f64, f64)) -> bool {
    let px = (x.floor().max(0.0) as u32).min(r.width() - 1);
    let py = (y.floor().max(0.0) as u32).min(r.height() - 1);
    is_dark(r, px, py)
}

/// The two landmarks are where the formula puts them, to within a pixel, and
/// their mirror images are not there.
fn assert_landmarks(label: &str, r: &Raster, page: (f64, f64), rot: i64) {
    let (w, h) = page;
    let bm = (r.width() as f64, r.height() as f64);
    let at = |p: (f64, f64)| device(rot, page, bm, p);
    let lm = landmarks(w, h);
    let (cx, cy) = lm.square_centre;

    assert!(
        dark_at(r, at((cx, cy))),
        "{label}: title-block square is missing at its centre"
    );
    for (name, p) in [
        ("left-right flip", (w - cx, cy)),
        ("up-down flip", (cx, h - cy)),
        ("both flips", (w - cx, h - cy)),
    ] {
        assert!(
            !dark_at(r, at(p)),
            "{label}: ink where the square's {name} would be"
        );
    }

    // The corner triangle is in the top-left corner and in no other.
    // 8 pt in from the corner: clear of the border, the ticks and the title
    // block's margin, and inside the triangle (whose legs are at least 30 pt).
    let d = 8.0;
    assert!(
        dark_at(r, at((d, h - d))),
        "{label}: top-left triangle is missing"
    );
    for (name, p) in [
        ("top-right", (w - d, h - d)),
        ("bottom-left", (d, d)),
        ("bottom-right", (w - d, d)),
    ] {
        assert!(
            !dark_at(r, at(p)),
            "{label}: ink in the {name} corner, where only top-left has the triangle"
        );
    }

    // Extent of the square along both device axes: a shift moves its midpoint
    // and a stretch changes its length, each by more than the tolerance.
    let (sx, sy) = if rot % 180 == 0 {
        (bm.0 / w, bm.1 / h)
    } else {
        (bm.0 / h, bm.1 / w)
    };
    let (dx, dy) = at((cx, cy));
    let (px, py) = (dx.floor() as u32, dy.floor() as u32);
    let run = |horizontal: bool| -> (u32, u32) {
        let (mut a, mut b) = if horizontal { (px, px) } else { (py, py) };
        let limit = if horizontal {
            r.width() - 1
        } else {
            r.height() - 1
        };
        let dark = |i: u32| {
            if horizontal {
                is_dark(r, i, py)
            } else {
                is_dark(r, px, i)
            }
        };
        while a > 0 && dark(a - 1) {
            a -= 1;
        }
        while b < limit && dark(b + 1) {
            b += 1;
        }
        (a, b)
    };
    for (axis, horizontal, scale, centre) in [("x", true, sx, dx), ("y", false, sy, dy)] {
        let (a, b) = run(horizontal);
        let len = (b - a + 1) as f64;
        let mid = (a + b + 1) as f64 / 2.0;
        assert!(
            (len - lm.square * scale).abs() <= 1.5,
            "{label}: square is {len} px along {axis}, the formula says {:.2}",
            lm.square * scale
        );
        assert!(
            (mid - centre).abs() <= 1.0,
            "{label}: square midpoint along {axis} is {mid}, the formula says {centre:.2}"
        );
    }
}

#[test]
fn landmarks_sit_where_the_formula_says_on_full_renders_and_strips() {
    for page in representative() {
        for dpi in [72, 150] {
            if px(libvips_dims(page.w, page.h, dpi)) > MAX_RENDER_PX {
                continue;
            }
            let label = format!("{} @ {dpi}", page.label);
            let r = render_page_pdfium(&page.path, 1, dpi).unwrap();
            assert_landmarks(&format!("{label} (render)"), &r, (page.w, page.h), 0);
            let s = PdfiumStripSource::new_streaming(&page.path, 1, dpi).unwrap();
            let strip = s.render_strip(0, s.height()).unwrap();
            assert_landmarks(&format!("{label} (strip)"), &strip, (page.w, page.h), 0);
        }
    }
}

#[test]
fn landmarks_follow_rotation_origin_and_cropbox() {
    for (label, w, h) in [("Letter", 612.0, 792.0), ("A3", 841.89, 1190.551)] {
        let mut specs: Vec<(String, PageSpec, i64)> = [0, 90, 180, 270]
            .into_iter()
            .map(|r| {
                (
                    format!("{label} /Rotate {r}"),
                    PageSpec::new(w, h).rotate(r),
                    r,
                )
            })
            .collect();
        specs.push((
            format!("{label} origin"),
            PageSpec::new(w, h).origin(-36.5, 72.25),
            0,
        ));
        specs.push((
            format!("{label} CropBox"),
            PageSpec::new(w, h).crop_box(),
            0,
        ));
        specs.push((
            format!("{label} CropBox /Rotate 270 origin"),
            PageSpec::new(w, h)
                .crop_box()
                .rotate(270)
                .origin(40.0, 60.0),
            270,
        ));
        for (name, spec, rot) in specs {
            let path = write_spec(&format!("lm_{}", slug(&name)), &spec);
            for dpi in [72, 150] {
                let l = format!("{name} @ {dpi}");
                assert_landmarks(
                    &format!("{l} (render)"),
                    &render_page_pdfium(&path, 1, dpi).unwrap(),
                    (w, h),
                    rot,
                );
                let s = PdfiumStripSource::new_streaming(&path, 1, dpi).unwrap();
                assert_landmarks(
                    &format!("{l} (strip)"),
                    &s.render_strip(0, s.height()).unwrap(),
                    (w, h),
                    rot,
                );
                assert_frame_on_every_edge(&l, &render_page_pdfium(&path, 1, dpi).unwrap());
            }
        }
    }
}

/// The generated sheets are not blank: they carry colour, grey and dark ink,
/// and stay small on disk.
#[test]
fn generated_pages_carry_real_content_and_stay_small() {
    for page in representative() {
        let bytes = std::fs::metadata(&page.path).unwrap().len();
        assert!(bytes < 120_000, "{}: {bytes} bytes on disk", page.label);
        let r = render_page_pdfium(&page.path, 1, 72).unwrap();
        let (mut dark, mut grey, mut colour) = (0u64, 0u64, 0u64);
        for p in r.data().chunks_exact(4) {
            let (a, b, c) = (p[0] as i32, p[1] as i32, p[2] as i32);
            if a < 64 && b < 64 && c < 64 {
                dark += 1;
            } else if (a - b).abs() < 6 && (b - c).abs() < 6 && (140..215).contains(&a) {
                grey += 1;
            } else if (a - b).abs() > 80 || (b - c).abs() > 80 {
                colour += 1;
            }
        }
        let total = (r.width() * r.height()) as u64;
        assert!(
            dark * 1000 > total,
            "{}: too little dark ink ({dark}/{total})",
            page.label
        );
        assert!(
            grey * 100 > total,
            "{}: no grey region ({grey}/{total})",
            page.label
        );
        assert!(
            colour * 200 > total,
            "{}: too little colour ({colour}/{total})",
            page.label
        );
    }
}
