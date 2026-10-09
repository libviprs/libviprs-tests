//! libviprs sizes a PDF page exactly as libvips does (libviprs#1199).
//!
//! Sits beside `rotation_libvips_pdfium_parity.rs`, which compares pixels to
//! committed goldens a libvips+pdfium build produced. This one asks the live
//! libvips instead: `vipsheader` on `page.pdf[dpi=N]` gives the width and
//! height `pdfload` would render, and `PageSizing::Exact` has to say the same
//! for every page in the size table at every DPI in the sweep. A table of
//! numbers I typed in myself would only prove I copied the same formula twice.
//!
//! # The oracle
//!
//! `vipsheader` from a libvips with a PDF loader (pdfium or poppler, the two
//! round alike). It is looked up on `$PATH`, or at `$VIPSHEADER`. The image
//! `tools/Dockerfile.libvips-pdfium` builds has it, and so does a stock
//! `libvips-tools` package with poppler. The default CI job has neither, so
//! the test prints a `SKIP` and returns, the same convention the CLI-driven
//! suites use for a missing sibling. `VIPRS_REQUIRE_VIPS=1` turns that skip
//! into a failure for a job that is meant to have the oracle.
//!
//! # Where it has run
//!
//! - libvips 8.18.4 built with the pdfium backend (`tools/Dockerfile.libvips-pdfium`),
//!   linux/arm64 in a container, 2026-10-09, run against the core's
//!   `fix/1199-page-size-matches-libvips` working tree with libpdfium
//!   pdfium-8085 swapped in over the image's 8054 (libvips and the test binary
//!   load the same `libpdfium.so`): every size and DPI in the table, and the
//!   rotated pages, equal.
//! - Native x64 on MARS: see the PR for that run.
//!
//! It does not need the `pdfium` feature: sizing is pure arithmetic, and the
//! rendered rasters are checked against the same table in
//! `pdfium_page_size_exact.rs`.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::blank_pdf::{DPIS, PageSpec, SIZES, libvips_dims, slug, write_blank_pdf};
use libviprs::PageSizing;

fn vipsheader_bin() -> String {
    std::env::var("VIPSHEADER").unwrap_or_else(|_| "vipsheader".to_string())
}

/// `vipsheader -f <field> <file>`, parsed as a number. `Err` carries what
/// vipsheader said, so the caller can tell "no PDF loader" from a real mismatch.
fn header_field(file: &str, field: &str) -> Result<u32, String> {
    let out = Command::new(vipsheader_bin())
        .args(["-f", field, file])
        .output()
        .map_err(|e| format!("cannot run {}: {e}", vipsheader_bin()))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .map_err(|e| {
            format!(
                "not a number {:?}: {e}",
                String::from_utf8_lossy(&out.stdout)
            )
        })
}

/// Skip guard: `true` (with a printed reason) when there is no libvips with a
/// PDF loader to ask. Panics instead under `VIPRS_REQUIRE_VIPS=1`.
fn skip_if_no_vips_pdf(test: &str, probe_pdf: &str) -> bool {
    let reason = match header_field(&format!("{probe_pdf}[dpi=72]"), "width") {
        Ok(_) => return false,
        Err(e) => e,
    };
    if std::env::var("VIPRS_REQUIRE_VIPS").as_deref() == Ok("1") {
        panic!("{test}: VIPRS_REQUIRE_VIPS=1 but libvips cannot load a PDF: {reason}");
    }
    eprintln!(
        "SKIP {test}: no libvips with a PDF loader ({reason}). Put vipsheader on PATH, \
         set $VIPSHEADER, or run the image built from tools/Dockerfile.libvips-pdfium."
    );
    true
}

fn vips_version() -> String {
    Command::new(vipsheader_bin())
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn write_pages() -> (tempfile::TempDir, Vec<(&'static str, f64, f64, PathBuf)>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let pages = SIZES
        .iter()
        .map(|&(label, w, h)| {
            let path = write_blank_pdf(
                dir.path(),
                &format!("{}.pdf", slug(label)),
                &PageSpec::new(w, h),
            );
            (label, w, h, path)
        })
        .collect();
    (dir, pages)
}

#[test]
fn exact_sizing_equals_libvips_pdfload_for_the_size_table() {
    let (_dir, pages) = write_pages();
    let probe = pages[0].3.to_str().unwrap();
    if skip_if_no_vips_pdf(
        "exact_sizing_equals_libvips_pdfload_for_the_size_table",
        probe,
    ) {
        return;
    }
    eprintln!("oracle: {}", vips_version());

    let mut checked = 0;
    for (label, w, h, path) in &pages {
        for &dpi in DPIS {
            let file = format!("{}[dpi={dpi}]", path.to_str().unwrap());
            let vw =
                header_field(&file, "width").unwrap_or_else(|e| panic!("{label} @ {dpi}: {e}"));
            let vh =
                header_field(&file, "height").unwrap_or_else(|e| panic!("{label} @ {dpi}: {e}"));

            // The longhand oracle first: if this fails the test's own formula
            // is wrong and the Exact comparison below means nothing.
            assert_eq!(
                libvips_dims(*w, *h, dpi),
                (vw, vh),
                "{label} @ {dpi} dpi: the test oracle disagrees with libvips pdfload",
            );
            let exact = PageSizing::Exact.pixel_dims(*w as f32 as f64, *h as f32 as f64, dpi);
            assert_eq!(
                exact,
                (vw, vh),
                "{label} @ {dpi} dpi: PageSizing::Exact disagrees with libvips pdfload",
            );
            checked += 1;
        }
    }
    assert_eq!(checked, SIZES.len() * DPIS.len());
}

/// Rotation swaps the axes in libvips too, and `/Rotate` is read from the page
/// the same way, so the swapped size must agree as well.
#[test]
fn exact_sizing_equals_libvips_pdfload_on_rotated_pages() {
    let (dir, pages) = write_pages();
    let probe = pages[0].3.to_str().unwrap();
    if skip_if_no_vips_pdf(
        "exact_sizing_equals_libvips_pdfload_on_rotated_pages",
        probe,
    ) {
        return;
    }
    for &(label, w, h) in &[
        ("Letter", 612.0, 792.0),
        ("A3", 841.89, 1190.551),
        ("A4", 595.276, 841.89),
    ] {
        for rotate in [90, 270] {
            let path = write_blank_pdf(
                dir.path(),
                &format!("rot{rotate}_{}.pdf", slug(label)),
                &PageSpec::new(w, h).rotate(rotate),
            );
            for &dpi in DPIS {
                let file = format!("{}[dpi={dpi}]", path.to_str().unwrap());
                let vw = header_field(&file, "width").unwrap();
                let vh = header_field(&file, "height").unwrap();
                assert_eq!(
                    PageSizing::Exact.pixel_dims(h as f32 as f64, w as f32 as f64, dpi),
                    (vw, vh),
                    "{label} /Rotate {rotate} @ {dpi} dpi",
                );
            }
        }
    }
}
