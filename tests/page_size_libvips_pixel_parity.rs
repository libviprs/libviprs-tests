//! A libviprs page raster matches the libvips one, size first and then pixels
//! (libviprs#1199).
//!
//! `page_size_libvips_pdfium_parity.rs` compares sizes. This renders the
//! committed fixture sheets with libviprs (`render_page_pdfium`,
//! `PageSizing::Exact`, the route the dimension tests use, RGBA8) and compares
//! the result with the libvips raster stored under
//! `tests/fixtures/page_size/libvips/`. **libvips does not run in a normal
//! test run**: the references were made once with the pdfium-backed build and
//! are read back from PNG, see `common::libvips_reference` for how they are
//! made, what the manifest records, and the regen and verify commands.
//!
//! First the size check: identical width, height and band count (the manifest
//! records libvips' `pdfload` bands, 4, against libviprs' RGBA8). That is the
//! assertion that fails against 0.5.x. Only then the bytes, within a
//! [`PixelTolerance`], because the two paths differ in antialiasing, colour
//! handling and flags (libvips renders with `FPDF_ANNOT |
//! FPDF_REVERSE_BYTE_ORDER` plus a form-fill pass, libviprs with its own).
//!
//! # The tolerance, and where its defaults come from
//!
//! Measured on the references (libvips 8.18.4, pdfium-8085, linux/arm64)
//! against the core fix: 59 stored rasters
//! (all sheets at 72 dpi, mid sheets to 150, Letter, A4 and Tabloid at 300, a
//! rotated and a CropBox page), worst bad-pixel fraction 1e-5, worst mean
//! absolute error 0.0014, byte-identical pixels at least 99.97% on every page,
//! largest single delta 185 (ARCH D landscape at 150 dpi, pixel (4001, 463),
//! an antialiased edge). The one-pixel controls measure at least 0.0065 bad
//! and 0.91 MAE. The defaults (delta 48, bad fraction 0.0005, MAE 0.02) sit
//! between the two: about 50x and 14x above the worst real page, 13x and 45x
//! below the weakest control.
//!
//! Three more checks keep the tolerance honest. A control compares the libviprs
//! raster with the stored libvips one shifted by one pixel in x, in y, and
//! stretched one pixel wider, and requires each to be out of tolerance, so it
//! cannot be so loose it hides an off-by-one. The landmarks (title-block
//! square, top-left triangle) have to sit at the same pixels in both rasters.
//!
//! `VIPRS_VERIFY_LIBVIPS_REFERENCE=1` re-runs libvips and checks the stored
//! rasters still match it byte for byte; without a libvips PDF loader that
//! prints a SKIP, and `VIPRS_REQUIRE_VIPS=1` (which only matters in verify
//! mode) turns the skip into a failure.

#![cfg(feature = "pdfium")]

mod common;

use common::blank_pdf::{fixture_pdf, landmarks};
use common::libvips_reference;
use common::pixel_tolerance::{
    PixelStats, PixelTolerance, compare_rgb, shifted_overlap, stretched_one_pixel,
};
use common::vips_oracle::{self, skip_if_no_vips_pdf};
use libviprs::render_page_pdfium;

struct Pair {
    label: String,
    case_w: f64,
    case_h: f64,
    rotate: i64,
    width: u32,
    height: u32,
    /// The stored libvips raster, RGB.
    vips: Vec<u8>,
    /// The libviprs raster, RGB.
    ours: Vec<u8>,
}

/// Run `f` on every stored raster, after the size check.
fn for_each_pair(mut f: impl FnMut(&Pair)) {
    let manifest = libvips_reference::load();
    eprintln!(
        "reference: {} / {}",
        manifest.libvips_version(),
        manifest.pdfium()
    );
    let mut ran = 0;
    for e in libvips_reference::matrix().iter().filter(|e| e.raster) {
        let label = format!("{} @ {}", e.label, e.dpi);
        let rec = manifest.recorded(e);
        let ours = render_page_pdfium(&fixture_pdf(&e.spec), 1, e.dpi)
            .unwrap_or_else(|err| panic!("{label}: libviprs render failed: {err:?}"));

        // The size check: width, height and bands (pdfload's 4, RGBA, against
        // libviprs' RGBA8 raster).
        assert_eq!(
            (rec.width, rec.height, rec.bands),
            (ours.width(), ours.height(), 4),
            "{label}: libvips (width, height, bands) vs libviprs RGBA8",
        );
        assert!(
            ours.data().chunks_exact(4).all(|p| p[3] == 255),
            "{label}: libviprs raster is not opaque, so comparing RGB would hide it"
        );
        let rgb: Vec<u8> = ours
            .data()
            .chunks_exact(4)
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        f(&Pair {
            label,
            case_w: e.spec.w,
            case_h: e.spec.h,
            rotate: e.spec.rotate.unwrap_or(0),
            width: ours.width(),
            height: ours.height(),
            vips: rec.raster.expect("matrix says this entry has a raster"),
            ours: rgb,
        });
        ran += 1;
    }
    assert!(ran >= 50, "only {ran} stored rasters compared");
}

#[test]
fn raster_matches_libvips_within_tolerance() {
    let tol = PixelTolerance::from_env();
    let mut failures = Vec::new();
    for_each_pair(|p| {
        let s = compare_rgb(&p.ours, &p.vips, p.width, &tol);
        eprintln!(
            "STATS {:<22} {}x{}  {}",
            p.label,
            p.width,
            p.height,
            s.summary()
        );
        if !s.within(&tol) {
            failures.push(format!("{}: {}", p.label, s.summary()));
        }
    });
    assert!(
        failures.is_empty(),
        "outside {tol:?}:\n{}",
        failures.join("\n")
    );
}

/// The tolerance must be tight enough that an off-by-one cannot hide in it.
#[test]
fn one_pixel_shift_and_stretch_are_outside_the_tolerance() {
    let tol = PixelTolerance::from_env();
    let mut escaped = Vec::new();
    for_each_pair(|p| {
        let dims = (p.width, p.height);
        let mut check = |what: &str, s: PixelStats| {
            eprintln!("CONTROL {:<22} {what:<8} {}", p.label, s.summary());
            if s.within(&tol) {
                escaped.push(format!("{} {what}: {}", p.label, s.summary()));
            }
        };
        for (what, d) in [("shift x", (1, 0)), ("shift y", (0, 1))] {
            let (a, b, w) = shifted_overlap(&p.ours, &p.vips, dims, d);
            check(what, compare_rgb(&a, &b, w, &tol));
        }
        let stretched = stretched_one_pixel(&p.vips, dims);
        check("stretch", compare_rgb(&p.ours, &stretched, p.width, &tol));
    });
    assert!(
        escaped.is_empty(),
        "a one-pixel error fits inside {tol:?}:\n{}",
        escaped.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Landmarks
// ---------------------------------------------------------------------------

fn dark(rgb: &[u8], w: u32, x: u32, y: u32) -> bool {
    let i = ((y * w + x) * 3) as usize;
    rgb[i] < 128 && rgb[i + 1] < 128 && rgb[i + 2] < 128
}

/// Device position of page point `(x, y)`, as in `pdfium_page_size_exact.rs`.
fn device(rot: i64, (w, h): (f64, f64), (bw, bh): (f64, f64), (x, y): (f64, f64)) -> (f64, f64) {
    match rot {
        0 => (x * bw / w, (h - y) * bh / h),
        90 => (y * bw / h, x * bh / w),
        180 => ((w - x) * bw / w, y * bh / h),
        270 => ((h - y) * bw / h, (w - x) * bh / w),
        _ => unreachable!(),
    }
}

/// The few pixels the landmark checks read: the title-block square's centre
/// `(px, py)`, and where the dark run in the top-left corner starts `(tx, ty)`.
#[derive(Clone, Copy)]
struct LandmarkPoints {
    px: u32,
    py: u32,
    tx: u32,
    ty: u32,
    w: u32,
    h: u32,
}

fn landmark_points(page: (f64, f64), rot: i64, (bw, bh): (u32, u32)) -> LandmarkPoints {
    let bm = (bw as f64, bh as f64);
    let lm = landmarks(page.0, page.1);
    let (cx, cy) = device(rot, page, bm, lm.square_centre);
    let (tx, ty) = device(rot, page, bm, (0.5, page.1 - lm.tri * 0.25));
    LandmarkPoints {
        px: cx.floor() as u32,
        py: cy.floor() as u32,
        tx: (tx.floor() as u32).min(bw - 1),
        ty: (ty.floor() as u32).min(bh - 1),
        w: bw,
        h: bh,
    }
}

/// Pixel extents of the title-block square along x and y, and the length of the
/// dark run at the left end of the row a quarter of the triangle down. `dark`
/// is only asked about row `py`, column `px` and row `ty`.
fn probe(
    dark: &dyn Fn(u32, u32) -> bool,
    pt: LandmarkPoints,
    label: &str,
) -> ((u32, u32), (u32, u32), u32) {
    let ext = |horizontal: bool| {
        let (mut a, mut b) = if horizontal {
            (pt.px, pt.px)
        } else {
            (pt.py, pt.py)
        };
        let limit = if horizontal { pt.w - 1 } else { pt.h - 1 };
        let d = |i: u32| {
            if horizontal {
                dark(i, pt.py)
            } else {
                dark(pt.px, i)
            }
        };
        assert!(d(a), "{label}: no square at its centre");
        while a > 0 && d(a - 1) {
            a -= 1;
        }
        while b < limit && d(b + 1) {
            b += 1;
        }
        (a, b)
    };
    let mut run = 0;
    let mut x = pt.tx;
    while x < pt.w && dark(x, pt.ty) {
        run += 1;
        x += 1;
    }
    (ext(true), ext(false), run)
}

#[test]
fn landmarks_sit_on_the_same_pixels_as_in_libvips() {
    let mut off = Vec::new();
    for_each_pair(|p| {
        let pt = landmark_points((p.case_w, p.case_h), p.rotate, (p.width, p.height));
        let (ox, oy, ot) = probe(&|x, y| dark(&p.ours, p.width, x, y), pt, &p.label);
        let (vx, vy, vt) = probe(&|x, y| dark(&p.vips, p.width, x, y), pt, &p.label);
        eprintln!(
            "LANDMARK {:<22} square x {ox:?}/{vx:?} y {oy:?}/{vy:?} triangle run {ot}/{vt}",
            p.label
        );
        if ox != vx || oy != vy || ot != vt {
            off.push(format!(
                "{}: square x {ox:?} vs {vx:?}, y {oy:?} vs {vy:?}, triangle run {ot} vs {vt}",
                p.label
            ));
        }
    });
    assert!(
        off.is_empty(),
        "landmarks moved (libviprs vs libvips):\n{}",
        off.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Big sheets: references made on demand
// ---------------------------------------------------------------------------

/// Peak resident memory of this process so far, in MB (Linux only).
fn peak_rss_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|t| {
            t.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .map(str::to_owned)
        })
        .and_then(|l| {
            l.split_whitespace()
                .nth(1)
                .and_then(|n| n.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb / 1024)
}

/// Sheets bigger than Tabloid at 300 dpi (and the 150 dpi rasters over the size
/// cap) are too big to commit. The first run with libvips on PATH makes them
/// into the git-ignored `generated/` folder, checked against the recorded hash
/// of the decoded pixels; later runs reuse them. With no libvips and no file
/// the entry is skipped with the regen command (a failure under
/// `VIPRS_REQUIRE_VIPS=1`). The comparison runs in row chunks, so the stored
/// raster is never held whole next to the libviprs one.
#[test]
fn big_sheets_match_the_on_demand_libvips_reference() {
    use common::libvips_reference::{CHUNK_ROWS, Reference, reference_for, stream_png};
    use common::pixel_tolerance::StatsAcc;
    use std::cell::RefCell;
    use std::collections::HashMap;

    let tol = PixelTolerance::from_env();
    let manifest = libvips_reference::load();
    let started = std::time::Instant::now();
    let (mut compared, mut skipped, mut failures) = (0, 0, Vec::new());
    for e in libvips_reference::matrix().iter().filter(|e| e.on_demand) {
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
        let ours = render_page_pdfium(&fixture_pdf(&e.spec), 1, e.dpi)
            .unwrap_or_else(|err| panic!("{label}: libviprs render failed: {err:?}"));
        assert_eq!(
            (rec.width, rec.height, rec.bands),
            (ours.width(), ours.height(), 4),
            "{label}: libvips (width, height, bands) vs libviprs RGBA8",
        );
        let (w, h) = (ours.width(), ours.height());
        let pt = landmark_points((e.spec.w, e.spec.h), e.spec.rotate.unwrap_or(0), (w, h));

        // One pass over the stored raster: statistics, its decoded hash, and
        // the few rows and the column the landmark probe reads.
        let acc = RefCell::new(StatsAcc::default());
        let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
        let rows: RefCell<HashMap<u32, Vec<u8>>> = RefCell::new(HashMap::new());
        let col = RefCell::new(vec![false; h as usize]);
        let mut opaque = true;
        let (rw, rh) = stream_png(&path, |y0, chunk| {
            sha2::Digest::update(&mut hasher, chunk);
            let n = (chunk.len() / (w as usize * 3)) as u32;
            let ours_rows =
                &ours.data()[(y0 as usize * w as usize * 4)..((y0 + n) as usize * w as usize * 4)];
            opaque &= ours_rows.chunks_exact(4).all(|p| p[3] == 255);
            acc.borrow_mut().add(ours_rows, 4, chunk, w, y0, &tol);
            for r in 0..n {
                let y = y0 + r;
                let row = &chunk[(r * w * 3) as usize..((r + 1) * w * 3) as usize];
                let d = |x: u32| {
                    let i = (x * 3) as usize;
                    row[i] < 128 && row[i + 1] < 128 && row[i + 2] < 128
                };
                col.borrow_mut()[y as usize] = d(pt.px);
                if y == pt.py || y == pt.ty {
                    rows.borrow_mut().insert(y, row.to_vec());
                }
            }
        });
        assert_eq!((rw, rh), (w, h), "{label}: stored raster size");
        let hash: String = sha2::Digest::finalize(hasher)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            hash, recorded_hash,
            "{label}: libvips output differs from the recorded reference (the stored raster no longer hashes to the manifest)"
        );
        assert!(opaque, "{label}: libviprs raster is not opaque");

        let s = acc.borrow().finish();
        let (rows, col) = (rows.borrow(), col.borrow());
        let sparse = |x: u32, y: u32| match rows.get(&y) {
            Some(row) => {
                let i = (x * 3) as usize;
                row[i] < 128 && row[i + 1] < 128 && row[i + 2] < 128
            }
            None => {
                assert_eq!(x, pt.px, "landmark probe asked for an unread pixel");
                col[y as usize]
            }
        };
        let our_dark = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            let d = ours.data();
            d[i] < 128 && d[i + 1] < 128 && d[i + 2] < 128
        };
        let ours_lm = probe(&our_dark, pt, &label);
        let ref_lm = probe(&sparse, pt, &label);
        eprintln!(
            "BIG {label:<24} {w}x{h}  {}  landmarks {} ({:.1}s, peak {} MB)",
            s.summary(),
            if ours_lm == ref_lm { "same" } else { "MOVED" },
            t.elapsed().as_secs_f64(),
            peak_rss_mb()
        );
        if !s.within(&tol) {
            failures.push(format!("{label}: {}", s.summary()));
        }
        if ours_lm != ref_lm {
            failures.push(format!("{label}: landmarks {ours_lm:?} vs {ref_lm:?}"));
        }
        compared += 1;
    }
    eprintln!(
        "BIG total: {compared} compared, {skipped} skipped, {:.1}s, peak {} MB, chunk {CHUNK_ROWS} rows",
        started.elapsed().as_secs_f64(),
        peak_rss_mb()
    );
    assert!(
        failures.is_empty(),
        "outside {tol:?}:\n{}",
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Making and checking the reference (the only places libvips runs)
// ---------------------------------------------------------------------------

/// Regenerate the manifest and rasters. Ignored, and it refuses to run without
/// `REGEN_PAGE_SIZE_LIBVIPS=1`, so nothing rewrites the references by accident.
/// The command is in `common::libvips_reference`.
#[test]
#[ignore = "regenerates the committed libvips references; needs libvips, see common::libvips_reference"]
fn regen_libvips_reference() {
    assert!(
        std::env::var("REGEN_PAGE_SIZE_LIBVIPS").as_deref() == Ok("1"),
        "set REGEN_PAGE_SIZE_LIBVIPS=1 to regenerate:\n  {}",
        libvips_reference::REGEN_COMMAND
    );
    let pdfium = std::env::var("VIPRS_REFERENCE_PDFIUM").expect(
        "VIPRS_REFERENCE_PDFIUM (e.g. pdfium-8085) records which pdfium libvips was linked against",
    );
    libvips_reference::regenerate(&pdfium);
}

/// Opt-in drift check: the stored sizes and rasters are still what the libvips
/// on PATH produces. Off unless `VIPRS_VERIFY_LIBVIPS_REFERENCE=1`.
#[test]
fn stored_references_still_match_a_live_libvips() {
    if std::env::var("VIPRS_VERIFY_LIBVIPS_REFERENCE").as_deref() != Ok("1") {
        eprintln!(
            "SKIP stored_references_still_match_a_live_libvips: set VIPRS_VERIFY_LIBVIPS_REFERENCE=1 to run libvips"
        );
        return;
    }
    let entries = libvips_reference::matrix();
    let probe = fixture_pdf(&entries[0].spec);
    if skip_if_no_vips_pdf(
        "stored_references_still_match_a_live_libvips",
        probe.to_str().unwrap(),
    ) {
        return;
    }
    let manifest = libvips_reference::load();
    eprintln!(
        "live: {}  recorded: {}",
        vips_oracle::version(),
        manifest.libvips_version()
    );
    let scratch = tempfile::tempdir().unwrap();
    for e in &entries {
        let rec = manifest.recorded(e);
        let pdf = fixture_pdf(&e.spec);
        let src = format!("{}[dpi={}]", pdf.display(), e.dpi);
        let field = |f: &str| vips_oracle::header_field(&src, f).unwrap();
        assert_eq!(
            (field("width"), field("height"), field("bands")),
            (rec.width, rec.height, rec.bands),
            "{} @ {}: size drifted",
            e.label,
            e.dpi
        );
        if e.on_demand {
            let (_, recorded_hash) = manifest.on_demand(e);
            let png = scratch.path().join("verify.png");
            vips_oracle::render_to_png(&pdf, e.dpi, &png).unwrap();
            let (_, _, hash) = libvips_reference::hash_png(&png);
            assert_eq!(
                hash, recorded_hash,
                "{} @ {}: on-demand raster drifted",
                e.label, e.dpi
            );
            let _ = std::fs::remove_file(&png);
        }
        if let Some(stored) = &rec.raster {
            let live = vips_oracle::render(&pdf, e.dpi, scratch.path(), "verify").unwrap();
            assert!(
                &live.rgb == stored,
                "{} @ {}: raster bytes drifted",
                e.label,
                e.dpi
            );
        }
    }
}
