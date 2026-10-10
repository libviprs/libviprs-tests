//! The `viprs` CLI sizes and renders a PDF page the way libvips does
//! (libviprs#1199, libviprs-cli#107).
//!
//! This is the CLI half of the page-size suite. The library half is
//! `pdfium_page_size_exact.rs`, `page_size_libvips_pdfium_parity.rs` and
//! `page_size_libvips_pixel_parity.rs`, and this file reads the very same
//! committed PDFs under `tests/fixtures/page_size/`, the same
//! `libvips_reference.json` and stored libvips PNGs, the same on-demand
//! generation for the big sheets, and the same `PixelTolerance`, all through
//! `tests/common`. Nothing is copied: the matrix, the manifest, the tolerance,
//! the landmark probe and the big-sheet row-chunk comparison are the library
//! suite's own helpers.
//!
//! The real binary is driven through four doors that take `--dpi` and
//! `--page-sizing`: `pdf info --dpi`, `plan`, `pdf extract` and `pyramid
//! --render`. What each one is held to:
//!
//! * sizes: for every sheet and DPI in the matrix the size the CLI reports is
//!   `PageSizing::Exact` and is the size libvips reported, from the manifest.
//!   `pdf info` and `plan` for the whole matrix (no render), `pyramid` (the
//!   `.dzi` it writes) and `pdf extract` (the PNG it writes) for every sheet
//!   that has a stored raster;
//! * pixels: the extracted PNG against the stored libvips raster, size and
//!   band count first, then within `PixelTolerance` (`VIPRS_PIXEL_MAX_*`),
//!   one-pixel shift and stretch controls that must fall outside it, and the
//!   landmark positions. The sheets bigger than Tabloid at 300 dpi are made on
//!   demand with libvips exactly as the library suite does (`VIPRS_REQUIRE_VIPS=1`
//!   turns a missing libvips into a failure) and compared in row chunks;
//! * the default is `exact` and `--page-sizing legacy-truncated` (alias
//!   `legacy`) keeps the 0.5.x sizes: Letter at 300 dpi is 2550x3300 by default
//!   and 2549x3299 with the flag, through every door.
//!
//! `VIPRS_VERIFY_LIBVIPS_REFERENCE=1` additionally asks a live `vipsheader`
//! and requires the CLI to agree with it.
//!
//! Needs a `viprs` built with the CLI's default `pdfium` feature and a
//! libpdfium, like `cli_pdf_geo_plan.rs`: `$VIPRS_BIN` (else this builds the
//! sibling with default features) and `$PDFIUM_PATH`. Without them the cells
//! print SKIP; `VIPRS_REQUIRE_CLI=1` and `VIPRS_REQUIRE_PDFIUM=1` make that a
//! failure.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::blank_pdf::fixture_pdf;
use common::cli::{cli_available, viprs_bin, viprs_bin_for};
use common::libvips_reference::{self, PngRows, Reference, matrix, reference_for};
use common::page_probe::{compare_big, dark, landmark_points, peak_rss_mb, probe};
use common::pixel_tolerance::{
    PixelStats, PixelTolerance, compare_rgb, shifted_overlap, stretched_one_pixel,
};
use common::vips_oracle::{self, skip_if_no_vips_pdf};
use libviprs::PageSizing;

// ---------------------------------------------------------------------------
// The binary
// ---------------------------------------------------------------------------

/// The pdfium-enabled `viprs`, or `None` (with a printed reason) when this
/// machine cannot run the cells.
fn viprs(test: &str) -> Option<PathBuf> {
    if !cli_available() {
        eprintln!(
            "SKIP {test}: libviprs-cli sibling not checked out (set $VIPRS_CLI_DIR / $VIPRS_BIN)."
        );
        return None;
    }
    if std::env::var_os("PDFIUM_PATH").is_none() {
        if std::env::var("VIPRS_REQUIRE_PDFIUM").is_ok_and(|v| v == "1") {
            panic!(
                "VIPRS_REQUIRE_PDFIUM=1 but $PDFIUM_PATH is unset: {test} would skip and \
                 report a false green."
            );
        }
        eprintln!("SKIP {test}: no $PDFIUM_PATH (needs libpdfium and a pdfium-enabled viprs).");
        return None;
    }
    Some(if std::env::var_os("VIPRS_BIN").is_some() {
        viprs_bin()
    } else {
        viprs_bin_for(true, &[])
    })
}

/// Run `viprs` and return stdout, panicking with both streams on failure.
fn run(bin: &Path, args: &[&str]) -> String {
    let out = Command::new(bin)
        .args(args)
        .env_remove("VIPRS_PDF_PASSWORD")
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", bin.display()));
    assert!(
        out.status.success(),
        "viprs {args:?} failed ({}):\n{}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn sizing_args<'a>(sizing: Option<&'a str>, mut args: Vec<&'a str>) -> Vec<&'a str> {
    if let Some(s) = sizing {
        args.extend(["--page-sizing", s]);
    }
    args
}

fn parse_wxh(s: &str) -> (u32, u32) {
    let (w, h) = s
        .split_once('x')
        .unwrap_or_else(|| panic!("not WxH: {s:?}"));
    (w.trim().parse().unwrap(), h.trim().parse().unwrap())
}

/// `viprs pdf info --dpi`: the size it prints for page 1.
fn info_dims(bin: &Path, pdf: &Path, dpi: u32, sizing: Option<&str>) -> (u32, u32) {
    let dpi = dpi.to_string();
    let args = sizing_args(
        sizing,
        vec!["pdf", "info", pdf.to_str().unwrap(), "--dpi", &dpi],
    );
    let out = run(bin, &args);
    let line = out
        .lines()
        .find(|l| l.contains(" px at "))
        .unwrap_or_else(|| panic!("no size line in pdf info output:\n{out}"));
    parse_wxh(
        line.trim()
            .trim_start_matches("->")
            .split(" px at ")
            .next()
            .unwrap(),
    )
}

/// `viprs plan --dpi`: the `Image: WxH` line.
fn plan_dims(bin: &Path, pdf: &Path, dpi: u32, sizing: Option<&str>) -> (u32, u32) {
    let dpi = dpi.to_string();
    let args = sizing_args(sizing, vec!["plan", pdf.to_str().unwrap(), "--dpi", &dpi]);
    let out = run(bin, &args);
    let line = out
        .lines()
        .find_map(|l| l.strip_prefix("Image: "))
        .unwrap_or_else(|| panic!("no Image line in plan output:\n{out}"));
    parse_wxh(line)
}

/// `viprs pyramid --render --dpi`: the size in the `.dzi` it writes.
fn pyramid_dims(bin: &Path, pdf: &Path, dpi: u32, sizing: Option<&str>) -> (u32, u32) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("tiles");
    let dpi = dpi.to_string();
    let args = sizing_args(
        sizing,
        vec![
            "pyramid",
            pdf.to_str().unwrap(),
            out.to_str().unwrap(),
            "--storage",
            "directory",
            "--render",
            "--dpi",
            &dpi,
            "--tile-size",
            "1024",
        ],
    );
    run(bin, &args);
    let dzi = std::fs::read_to_string(dir.path().join("tiles.dzi")).expect("pyramid wrote no .dzi");
    let attr = |name: &str| -> u32 {
        let at = dzi
            .find(name)
            .unwrap_or_else(|| panic!("no {name} in {dzi}"));
        dzi[at + name.len()..]
            .trim_start_matches("=\"")
            .split('"')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    (attr("Width"), attr("Height"))
}

/// `viprs pdf extract --dpi` to a PNG in `dir`.
fn extract(bin: &Path, pdf: &Path, dpi: u32, sizing: Option<&str>, dir: &Path) -> PathBuf {
    let png = dir.join("page.png");
    let dpi = dpi.to_string();
    let args = sizing_args(
        sizing,
        vec![
            "pdf",
            "extract",
            pdf.to_str().unwrap(),
            png.to_str().unwrap(),
            "--dpi",
            &dpi,
        ],
    );
    run(bin, &args);
    png
}

/// The page the sizing sees: `/Rotate` 90 and 270 swap the axes.
fn page_pts(e: &libvips_reference::Entry) -> (f64, f64) {
    if matches!(e.spec.rotate, Some(90 | 270)) {
        (e.spec.h as f32 as f64, e.spec.w as f32 as f64)
    } else {
        (e.spec.w as f32 as f64, e.spec.h as f32 as f64)
    }
}

// ---------------------------------------------------------------------------
// Sizes
// ---------------------------------------------------------------------------

/// `pdf info --dpi` and `plan` for every sheet at every DPI: exact is the
/// libvips size and the library's `PageSizing::Exact`; `legacy-truncated` is
/// the library's `LegacyTruncated`, and is not the same thing for some sheets.
#[test]
fn cli_reports_the_libvips_size_for_every_sheet_and_dpi() {
    let Some(bin) = viprs("cli_reports_the_libvips_size_for_every_sheet_and_dpi") else {
        return;
    };
    let manifest = libvips_reference::load();
    let (mut checked, mut legacy_differs) = (0, 0);
    let mut failures = Vec::new();
    for e in matrix() {
        // `pdf info` and `plan` size from the MediaBox, so a CropBox page is
        // covered by the ignored test below (libviprs/libviprs#1209).
        if e.spec.crop_box {
            continue;
        }
        let rec = manifest.recorded(&e);
        let want = (rec.width, rec.height);
        let (pw, ph) = page_pts(&e);
        let label = format!("{} @ {} dpi", e.label, e.dpi);
        let pdf = fixture_pdf(&e.spec);
        assert_eq!(
            PageSizing::Exact.pixel_dims(pw, ph, e.dpi),
            want,
            "{label}: the library's Exact disagrees with the recorded libvips size"
        );
        let legacy_want = PageSizing::LegacyTruncated.pixel_dims(pw, ph, e.dpi);
        legacy_differs += usize::from(legacy_want != want);

        for (door, got) in [
            ("pdf info (default)", info_dims(&bin, &pdf, e.dpi, None)),
            (
                "pdf info exact",
                info_dims(&bin, &pdf, e.dpi, Some("exact")),
            ),
            ("plan (default)", plan_dims(&bin, &pdf, e.dpi, None)),
            ("plan exact", plan_dims(&bin, &pdf, e.dpi, Some("exact"))),
        ] {
            if got != want {
                failures.push(format!("{label}: {door} {got:?}, libvips {want:?}"));
            }
        }
        for (door, got) in [
            (
                "pdf info legacy-truncated",
                info_dims(&bin, &pdf, e.dpi, Some("legacy-truncated")),
            ),
            (
                "plan legacy-truncated",
                plan_dims(&bin, &pdf, e.dpi, Some("legacy-truncated")),
            ),
        ] {
            if got != legacy_want {
                failures.push(format!(
                    "{label}: {door} {got:?}, library LegacyTruncated {legacy_want:?}"
                ));
            }
        }
        checked += 1;
    }
    assert!(checked >= 160, "only {checked} entries checked");
    assert!(
        legacy_differs >= 10,
        "legacy-truncated and exact agree on all but {legacy_differs} entries, so this \
         cannot tell the two flags apart"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `pdf info --dpi` and `plan` for the CropBox sheet say what the render
/// produces and libvips reports. They do not yet: `pdf_info` reads the
/// MediaBox (libviprs/libviprs#1209). Un-ignore with the fix.
#[test]
#[ignore = "libviprs/libviprs#1209: pdf_info ignores the CropBox"]
fn cli_info_and_plan_follow_the_cropbox_like_render() {
    let Some(bin) = viprs("cli_info_and_plan_follow_the_cropbox_like_render") else {
        return;
    };
    let manifest = libvips_reference::load();
    let mut failures = Vec::new();
    for e in matrix().iter().filter(|e| e.spec.crop_box) {
        let rec = manifest.recorded(e);
        let want = (rec.width, rec.height);
        let pdf = fixture_pdf(&e.spec);
        for (door, got) in [
            ("pdf info", info_dims(&bin, &pdf, e.dpi, None)),
            ("plan", plan_dims(&bin, &pdf, e.dpi, None)),
        ] {
            if got != want {
                failures.push(format!(
                    "{} @ {} dpi: {door} {got:?}, libvips {want:?}",
                    e.label, e.dpi
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The default is `exact`, and `legacy-truncated` (and its `legacy` alias)
/// keeps the 0.5.x size, through every door: Letter at 300 dpi.
#[test]
fn default_is_exact_and_legacy_truncated_keeps_the_old_size() {
    let Some(bin) = viprs("default_is_exact_and_legacy_truncated_keeps_the_old_size") else {
        return;
    };
    let letter = common::blank_pdf::PageSpec::new(612.0, 792.0);
    let pdf = fixture_pdf(&letter);
    let dir = tempfile::tempdir().unwrap();
    type Door<'a> = (&'a str, &'a dyn Fn(Option<&str>) -> (u32, u32));
    let doors: [Door; 4] = [
        ("pdf info", &|s| info_dims(&bin, &pdf, 300, s)),
        ("plan", &|s| plan_dims(&bin, &pdf, 300, s)),
        ("pyramid", &|s| pyramid_dims(&bin, &pdf, 300, s)),
        ("pdf extract", &|s| {
            let png = extract(&bin, &pdf, 300, s, dir.path());
            let rows = PngRows::open(&png);
            (rows.width, rows.height)
        }),
    ];
    for (door, dims) in doors {
        assert_eq!(dims(None), (2550, 3300), "{door}: the default is exact");
        assert_eq!(
            dims(Some("exact")),
            (2550, 3300),
            "{door}: --page-sizing exact"
        );
        assert_eq!(
            dims(Some("legacy-truncated")),
            (2549, 3299),
            "{door}: --page-sizing legacy-truncated is the 0.5.x size"
        );
        assert_eq!(
            dims(Some("legacy")),
            (2549, 3299),
            "{door}: `legacy` is an alias of legacy-truncated"
        );
    }
    // The old size is what the library's variant gives too.
    assert_eq!(
        PageSizing::LegacyTruncated.pixel_dims(612.0, 792.0, 300),
        (2549, 3299)
    );
    assert_eq!(
        PageSizing::Exact.pixel_dims(612.0, 792.0, 300),
        (2550, 3300)
    );
}

/// `--page-sizing` only means something next to `--dpi` on the commands that
/// require it, so it is refused rather than silently ignored.
#[test]
fn page_sizing_without_dpi_is_refused_by_pdf_extract() {
    let Some(bin) = viprs("page_sizing_without_dpi_is_refused_by_pdf_extract") else {
        return;
    };
    let pdf = fixture_pdf(&common::blank_pdf::PageSpec::new(612.0, 792.0));
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(&bin)
        .args(["pdf", "extract", pdf.to_str().unwrap()])
        .arg(dir.path().join("x.png"))
        .args(["--page-sizing", "exact"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "--page-sizing without --dpi was accepted"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--dpi"),
        "the refusal does not name --dpi:\n{err}"
    );
}

// ---------------------------------------------------------------------------
// Rendered sizes and pixels
// ---------------------------------------------------------------------------

/// Every stored raster: `pyramid` writes the libvips size, and `pdf extract`
/// writes a PNG of the libvips width, height and band count whose pixels are
/// within tolerance, outside tolerance when shifted or stretched by a pixel,
/// and whose landmarks sit on the same pixels.
#[test]
fn extracted_raster_matches_libvips_size_pixels_controls_and_landmarks() {
    let Some(bin) = viprs("extracted_raster_matches_libvips_size_pixels_controls_and_landmarks")
    else {
        return;
    };
    let tol = PixelTolerance::from_env();
    let manifest = libvips_reference::load();
    eprintln!(
        "reference: {} / {}",
        manifest.libvips_version(),
        manifest.pdfium()
    );
    let (mut ran, mut failures) = (0, Vec::new());
    for e in matrix().iter().filter(|e| e.raster) {
        let label = format!("{} @ {}", e.label, e.dpi);
        let rec = manifest.recorded(e);
        let pdf = fixture_pdf(&e.spec);

        let piramid = pyramid_dims(&bin, &pdf, e.dpi, None);
        if piramid != (rec.width, rec.height) {
            failures.push(format!(
                "{label}: pyramid wrote {piramid:?}, libvips {:?}",
                (rec.width, rec.height)
            ));
        }

        let dir = tempfile::tempdir().unwrap();
        let png = extract(&bin, &pdf, e.dpi, None, dir.path());
        let mut rows = PngRows::open(&png);
        // The size check first: width, height and bands (pdfload's 4, RGBA,
        // against the CLI's RGBA8 PNG).
        assert_eq!(
            (rec.width, rec.height, rec.bands),
            (rows.width, rows.height, rows.bands),
            "{label}: libvips (width, height, bands) vs the CLI's PNG"
        );
        let (w, h) = (rows.width, rows.height);
        let mut ours = Vec::with_capacity(w as usize * h as usize * 3);
        while let Some((_, chunk)) = rows.next_chunk() {
            ours.extend_from_slice(chunk);
        }
        assert!(
            rows.opaque,
            "{label}: the CLI's raster is not opaque, so comparing RGB would hide it"
        );
        let vips = rec.raster.expect("matrix says this entry has a raster");

        let s = compare_rgb(&ours, &vips, w, &tol);
        eprintln!("STATS {label:<22} {w}x{h}  {}", s.summary());
        if !s.within(&tol) {
            failures.push(format!("{label}: {}", s.summary()));
        }

        // The tolerance must not hide an off-by-one.
        let mut control = |what: &str, s: PixelStats| {
            eprintln!("CONTROL {label:<22} {what:<8} {}", s.summary());
            if s.within(&tol) {
                failures.push(format!(
                    "{label} {what} fits inside the tolerance: {}",
                    s.summary()
                ));
            }
        };
        for (what, d) in [("shift x", (1, 0)), ("shift y", (0, 1))] {
            let (a, b, ow) = shifted_overlap(&ours, &vips, (w, h), d);
            control(what, compare_rgb(&a, &b, ow, &tol));
        }
        let stretched = stretched_one_pixel(&vips, (w, h));
        control("stretch", compare_rgb(&ours, &stretched, w, &tol));

        let pt = landmark_points((e.spec.w, e.spec.h), e.spec.rotate.unwrap_or(0), (w, h));
        let o = probe(&|x, y| dark(&ours, w, x, y), pt, &label);
        let v = probe(&|x, y| dark(&vips, w, x, y), pt, &label);
        eprintln!("LANDMARK {label:<22} CLI {o:?} libvips {v:?}");
        if o != v {
            failures.push(format!("{label}: landmarks {o:?} (CLI) vs {v:?} (libvips)"));
        }
        ran += 1;
    }
    assert!(ran >= 50, "only {ran} stored rasters compared");
    assert!(
        failures.is_empty(),
        "outside {tol:?}:\n{}",
        failures.join("\n")
    );
}

/// The sheets bigger than Tabloid at 300 dpi, and the 150 dpi rasters over the
/// cap, against libvips rasters made on demand (see the library suite). The
/// CLI's PNG and the reference are read in row chunks and never held whole.
#[test]
fn big_sheets_from_the_cli_match_the_on_demand_libvips_reference() {
    let Some(bin) = viprs("big_sheets_from_the_cli_match_the_on_demand_libvips_reference") else {
        return;
    };
    let tol = PixelTolerance::from_env();
    let manifest = libvips_reference::load();
    let started = std::time::Instant::now();
    let (mut compared, mut skipped, mut failures) = (0, 0, Vec::new());
    for e in matrix().iter().filter(|e| e.on_demand) {
        let label = format!("{} @ {}", e.label, e.dpi);
        let (rec, recorded_hash) = manifest.on_demand(e);
        let path = match reference_for(e, &recorded_hash) {
            Reference::Ready(p) => p,
            Reference::Skip(msg) => {
                eprintln!("SKIP {msg}");
                skipped += 1;
                continue;
            }
        };
        let t = std::time::Instant::now();
        let dir = tempfile::tempdir().unwrap();
        let png = extract(&bin, &fixture_pdf(&e.spec), e.dpi, None, dir.path());
        let mut rows = PngRows::open(&png);
        assert_eq!(
            (rec.width, rec.height, rec.bands),
            (rows.width, rows.height, rows.bands),
            "{label}: libvips (width, height, bands) vs the CLI's PNG"
        );
        let (w, h) = (rows.width, rows.height);
        let big = compare_big(
            &label,
            &path,
            (e.spec.w, e.spec.h),
            e.spec.rotate.unwrap_or(0),
            (w, h),
            &tol,
            3,
            &mut |y0, n, buf| {
                let (cy, chunk) = rows.next_chunk().expect("the CLI's PNG ended early");
                assert_eq!(cy, y0, "{label}: chunks out of step");
                assert_eq!(chunk.len(), n as usize * w as usize * 3);
                buf.extend_from_slice(chunk);
            },
        );
        assert_eq!(
            big.ref_hash, recorded_hash,
            "{label}: libvips output differs from the recorded reference (the stored raster no longer hashes to the manifest)"
        );
        assert!(rows.opaque, "{label}: the CLI's raster is not opaque");
        eprintln!(
            "BIG {label:<24} {w}x{h}  {}  landmarks {} ({:.1}s, peak {} MB)",
            big.stats.summary(),
            if big.ours_landmarks == big.ref_landmarks {
                "same"
            } else {
                "MOVED"
            },
            t.elapsed().as_secs_f64(),
            peak_rss_mb()
        );
        if !big.stats.within(&tol) {
            failures.push(format!("{label}: {}", big.stats.summary()));
        }
        if big.ours_landmarks != big.ref_landmarks {
            failures.push(format!(
                "{label}: landmarks {:?} (CLI) vs {:?} (libvips)",
                big.ours_landmarks, big.ref_landmarks
            ));
        }
        compared += 1;
    }
    eprintln!(
        "BIG total: {compared} compared, {skipped} skipped, {:.1}s",
        started.elapsed().as_secs_f64()
    );
    assert!(
        failures.is_empty(),
        "outside {tol:?}:\n{}",
        failures.join("\n")
    );
}

/// Opt-in: the CLI against a live `vipsheader`, to show the manifest the other
/// cells lean on has not drifted from the libvips it was made with.
#[test]
fn cli_sizes_still_match_a_live_vipsheader() {
    if std::env::var("VIPRS_VERIFY_LIBVIPS_REFERENCE").as_deref() != Ok("1") {
        eprintln!(
            "SKIP cli_sizes_still_match_a_live_vipsheader: set VIPRS_VERIFY_LIBVIPS_REFERENCE=1 to run vipsheader"
        );
        return;
    }
    let Some(bin) = viprs("cli_sizes_still_match_a_live_vipsheader") else {
        return;
    };
    let entries = matrix();
    let probe_pdf = fixture_pdf(&entries[0].spec);
    if skip_if_no_vips_pdf(
        "cli_sizes_still_match_a_live_vipsheader",
        probe_pdf.to_str().unwrap(),
    ) {
        return;
    }
    eprintln!("live: {}", vips_oracle::version());
    for e in entries.into_iter().filter(|e| !e.spec.crop_box) {
        let pdf = fixture_pdf(&e.spec);
        let src = format!("{}[dpi={}]", pdf.display(), e.dpi);
        let live = (
            vips_oracle::header_field(&src, "width").unwrap(),
            vips_oracle::header_field(&src, "height").unwrap(),
        );
        assert_eq!(
            info_dims(&bin, &pdf, e.dpi, None),
            live,
            "{} @ {} dpi: the CLI disagrees with a live vipsheader",
            e.label,
            e.dpi
        );
    }
}
