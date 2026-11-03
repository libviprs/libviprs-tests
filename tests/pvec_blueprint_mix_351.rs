//! libviprs-tests#351: `blueprint-mix.pdf` through the PDF to PVec path.
//!
//! The fixture is one page (3370 x 4768 pt, `/Rotate 270`, so 4768 x 3370 displayed) with CAD
//! linework and a row of 12738 x 220 ICC colour strips, each with a soft mask. It converts the
//! page two ways. The first composes the public pieces the way `docs/pvec-raster-producer.md`
//! describes them, and the last test checks the one-call `PdfConversion` against it:
//!
//! ```text
//! extract_page_paths_from_bytes -> normalize_page -> PvecWriter::push   (vector geometry)
//!                               -> bake_page_images -> write_rasters    (raster blocks)
//! ```
//!
//! and reads the artifact back through PVec's reader. What it pins:
//!
//! * the artifact holds both: vector entities (polylines and cubics, none lost) and raster
//!   blocks (82 of the 89 strips; 7 are refused as fully clipped), all inside the displayed page;
//! * the soft mask rides in alpha (RGBA tiles), the ICC profile is approximated and says so;
//! * placements follow the rotation: the strip maps are anti-diagonal and equal the CTM formula
//!   of `docs/pvec-raster-producer.md` bit for bit;
//! * what the reader returns equals what the writer was given;
//! * two runs give the same bytes;
//! * `PdfConversion` (libviprs/libviprs#1491), the one call that does all of the above and
//!   publishes artifact and report together, writes the same entities and rasters and counts
//!   the same refusals;
//! * every refused or dropped thing has a reason key that a doc names;
//! * peak RSS stays under the 1.5 GiB ceiling of libviprs/libviprs#1064 (Linux only, it reads
//!   `/proc/self/status`; elsewhere the number is not available and the test says so).
//!
//! Time: about a minute per conversion in a release build, nearly all of it in the image bake.
//! Every test is `#[ignore]`d, so no per-PR run pays for it. The nightly workflow runs them with
//! `cargo test --release --features pvec-raster --test pvec_blueprint_mix_351 -- --ignored`, and so
//! can you, with `--nocapture` to see the numbers.

#![cfg(feature = "pvec-raster")]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use libviprs::cad::CadSource;
use libviprs::pvec::fill_policy::FillReport;
use libviprs::pvec::pdf_bezier::normalize_page;
use libviprs::pvec::pdf_image::{PdfImageBake, PdfImageOptions, bake_page_images};
use libviprs::pvec::pdf_vector::{
    PdfPathSegment, PdfVectorLimits, PdfVectorPage, extract_page_paths_from_bytes,
};
use libviprs::pvec::publish::{open_published, report_path};
use libviprs::pvec_convert::PdfConversion;
use libviprs::raster_pvec::container::write_rasters;
use pvec_facade::unstable::{PixelFormat, RasterId};
use pvec_facade::{Bounds2, Curve, EntityId, Manifest, PvecReader, PvecWriter};
use sha2::{Digest, Sha256};

const FIXTURE_SHA256: &str = "a2985c98d56b6c4e128096fed6e132cdcf5f327649f3a9923d55b0aff40a37ad";
const FIXTURE_BYTES: usize = 51_806_940;
/// libviprs/libviprs#1064: 1.5 GiB of process peak RSS.
const RSS_CEILING_KIB: u64 = 1536 * 1024;

fn fixture() -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests/fixtures/blueprint-mix.pdf",
    ]
    .iter()
    .collect();
    let bytes = std::fs::read(&path).expect("tests/fixtures/blueprint-mix.pdf is committed");
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        bytes.len(),
        FIXTURE_BYTES,
        "a different file under this name"
    );
    assert_eq!(digest, FIXTURE_SHA256, "a different file under this name");
    bytes
}

fn peak_rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines()
        .find(|l| l.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// Everything one conversion produced.
struct Run {
    artifact: Vec<u8>,
    page: PdfVectorPage,
    bake: PdfImageBake,
    /// Kinds of curve handed to the writer.
    pushed_polylines: u64,
    pushed_beziers: u64,
    /// Segments PVec refused, by reason.
    vector_refused: BTreeMap<String, u64>,
    fills: FillReport,
    wall: Duration,
    peak_rss_kib: Option<u64>,
}

fn convert() -> Run {
    let t0 = Instant::now();
    let bytes = fixture();
    let page = extract_page_paths_from_bytes(&bytes, 1, &PdfVectorLimits::default())
        .expect("the page's paths and images are extracted");
    drop(bytes);
    let bake = bake_page_images(&page, &PdfImageOptions::default());

    let mut writer = PvecWriter::new(Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 {
            min: [0.0, 0.0],
            max: [page.width, page.height],
        },
        capabilities: Vec::new(),
        producer_id: "libviprs-tests-351".into(),
        producer_version: "1".into(),
    })
    .expect("a manifest");
    let (mut next, mut pushed_polylines, mut pushed_beziers) = (0u32, 0u64, 0u64);
    let mut vector_refused: BTreeMap<String, u64> = BTreeMap::new();
    for path in normalize_page(&page) {
        for e in path.entities {
            let Some(curve) = e.geometry.geometry() else {
                *vector_refused
                    .entry(
                        e.classification
                            .reason()
                            .map_or_else(|| "no-reason".to_owned(), |r| format!("{r:?}")),
                    )
                    .or_insert(0) += 1;
                continue;
            };
            writer
                .push(EntityId::new(next).unwrap(), curve)
                .expect("PVec takes the curve");
            next += 1;
            match curve {
                Curve::Polyline(_) => pushed_polylines += 1,
                Curve::Bezier(_) => pushed_beziers += 1,
                _ => {}
            }
        }
    }
    write_rasters(&mut writer, bake.written.iter().map(|b| &b.pyramid))
        .expect("PVec takes the pyramids");
    let artifact = writer.finish().expect("the artifact seals");

    let mut fills = FillReport::new();
    fills.count_pdf_page(&page);
    Run {
        artifact,
        page,
        bake,
        pushed_polylines,
        pushed_beziers,
        vector_refused,
        fills,
        wall: t0.elapsed(),
        peak_rss_kib: peak_rss_kib(),
    }
}

/// The conversion the read-only tests share (a minute each otherwise).
fn first() -> &'static Run {
    static RUN: OnceLock<Run> = OnceLock::new();
    RUN.get_or_init(convert)
}

fn page_box(run: &Run) -> [f64; 4] {
    [0.0, 0.0, run.page.width, run.page.height]
}

fn inside(b: [f64; 4], x: f64, y: f64) -> bool {
    let eps = 1e-6;
    x >= b[0] - eps && x <= b[2] + eps && y >= b[1] - eps && y <= b[3] + eps
}

/// The box of the page's path points, from the extracted segments.
fn vector_extent(page: &PdfVectorPage) -> [f64; 4] {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut add = |p: [f64; 2]| {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    };
    for e in &page.entities {
        for s in &e.segments {
            match s {
                PdfPathSegment::MoveTo(p) | PdfPathSegment::LineTo(p) => add(*p),
                PdfPathSegment::CurveTo { c1, c2, end } => {
                    add(*c1);
                    add(*c2);
                    add(*end);
                }
                _ => {}
            }
        }
    }
    b
}

fn docs() -> String {
    let dir: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "libviprs", "docs"]
        .iter()
        .collect();
    let mut all = String::new();
    for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let p = entry.unwrap().path();
        if p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("pvec") && n.ends_with(".md"))
        {
            all.push_str(&std::fs::read_to_string(p).unwrap());
        }
    }
    all
}

/// A reason key is documented when a doc names it in code font. `name:argument` keys
/// (`unsupported-filter:LZWDecode`) are documented by their prefix.
fn documented(docs: &str, key: &str) -> bool {
    let stem = key.split(':').next().unwrap();
    docs.contains(&format!("`{stem}"))
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn the_page_is_the_one_the_numbers_below_are_about() {
    let run = first();
    assert_eq!((run.page.width, run.page.height), (4768.0, 3370.0));
    assert_eq!(run.page.rotation_degrees, 270);
    assert_eq!(run.page.images.len(), 89);
    assert!(
        run.page.entities.len() >= 300,
        "{}",
        run.page.entities.len()
    );
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn the_artifact_holds_vector_geometry_and_raster_blocks() {
    let run = first();
    let reader = PvecReader::open(run.artifact.clone()).expect("PVec opens its own file");

    // Vector side: every painted segment became an entity, none was refused.
    assert!(run.vector_refused.is_empty(), "{:?}", run.vector_refused);
    let entities = reader.entities().unwrap();
    assert_eq!(reader.entity_count().unwrap(), entities.len() as u64);
    assert_eq!(
        entities.len() as u64,
        run.pushed_polylines + run.pushed_beziers
    );
    // PVec's container stores a cubic Bezier as a clamped cubic B-spline (four control points,
    // knots 0 0 0 0 1 1 1 1), so the reader hands back a `BSpline` for what the writer got as a
    // `Bezier`. Same control points, no knot was added: that is what "kept as a cubic" means here.
    let (mut polylines, mut beziers) = (0u64, 0u64);
    for (_, c) in &entities {
        match c {
            Curve::Polyline(_) => polylines += 1,
            Curve::Bezier(_) => beziers += 1,
            Curve::BSpline(b) => {
                let dbg = format!("{b:?}");
                assert!(
                    dbg.contains("degree: 3")
                        && dbg.contains("knots: [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]"),
                    "a B-spline that is not a Bezier in disguise: {dbg}"
                );
                beziers += 1;
            }
            other => panic!("a curve kind the PDF side never makes: {other:?}"),
        }
    }
    assert!(polylines >= 900, "polylines: {polylines}");
    assert!(beziers >= 5, "cubics kept as cubics: {beziers}");
    assert_eq!(
        (polylines, beziers),
        (run.pushed_polylines, run.pushed_beziers)
    );

    // Raster side: 82 strips written, 7 refused, the count adds up.
    let placements = reader.raster_placements().unwrap();
    assert_eq!(run.bake.report.images, 89);
    assert_eq!(run.bake.report.written, 82);
    assert_eq!(run.bake.report.refused_total(), 7);
    assert_eq!(placements.len(), 82);
    assert_eq!(
        run.bake.report.images,
        run.bake.report.written + run.bake.report.refused_total(),
        "nothing is dropped silently"
    );
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn every_vector_point_and_raster_footprint_is_inside_the_rotated_page() {
    let run = first();
    let b = page_box(run);
    let v = vector_extent(&run.page);
    assert!(
        inside(b, v[0], v[1]) && inside(b, v[2], v[3]),
        "vector extent {v:?}"
    );
    // Not a sliver in a corner: the linework spans most of the displayed sheet.
    assert!(v[2] - v[0] > 0.5 * run.page.width, "{v:?}");
    assert!(v[3] - v[1] > 0.5 * run.page.height, "{v:?}");

    let reader = PvecReader::open(run.artifact.clone()).unwrap();
    let mut union = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (i, p) in reader.raster_placements().unwrap().iter().enumerate() {
        let m = p.pixel_to_source;
        for (px, py) in [
            (0.0, 0.0),
            (f64::from(p.width), 0.0),
            (0.0, f64::from(p.height)),
            (f64::from(p.width), f64::from(p.height)),
        ] {
            let (x, y) = (m[0] * px + m[2] * py + m[4], m[1] * px + m[3] * py + m[5]);
            assert!(
                inside(b, x, y),
                "raster {i} corner ({x}, {y}) leaves the page"
            );
            union = [
                union[0].min(x),
                union[1].min(y),
                union[2].max(x),
                union[3].max(y),
            ];
        }
    }
    // One drawing: the strips sit on the linework, not beside the sheet.
    let overlap_x = union[2].min(v[2]) - union[0].max(v[0]);
    let overlap_y = union[3].min(v[3]) - union[1].max(v[1]);
    assert!(overlap_x > 0.0 && overlap_y > 0.0, "{union:?} vs {v:?}");
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn raster_placements_follow_the_rotation_and_the_ctm() {
    let run = first();
    let reader = PvecReader::open(run.artifact.clone()).unwrap();
    let placements = reader.raster_placements().unwrap();
    for (w, p) in run.bake.written.iter().zip(&placements) {
        let ctm = run.page.images[w.index].ctm;
        let (pw, ph) = (f64::from(p.width), f64::from(p.height));
        // `docs/pvec-raster-producer.md`: [A/w -C/h B/w -D/h C+E D+F], -0.0 as +0.0.
        let expect = [
            ctm[0] / pw,
            -ctm[2] / ph,
            ctm[1] / pw,
            -ctm[3] / ph,
            ctm[2] + ctm[4],
            ctm[3] + ctm[5],
        ]
        .map(|v| if v == 0.0 { 0.0 } else { v });
        assert_eq!(
            p.pixel_to_source.map(f64::to_bits),
            expect.map(f64::to_bits),
            "image {}",
            w.index
        );
        // A page turned by 270 degrees swaps the axes: pixel x runs up the sheet, pixel rows run
        // across it, and the map has the negative determinant of an upright scan.
        let m = p.pixel_to_source;
        assert!(m[0] == 0.0 && m[3] == 0.0, "{m:?}");
        assert!(m[1] > 0.0 && m[2] > 0.0, "{m:?}");
        assert!(m[0] * m[3] - m[1] * m[2] < 0.0, "{m:?}");
    }
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn rasters_are_rgba_png_pyramids_with_the_soft_mask_in_alpha() {
    let run = first();
    let reader = PvecReader::open(run.artifact.clone()).unwrap();
    let placements = reader.raster_placements().unwrap();

    let mut strips = 0u64;
    let mut sizes: BTreeMap<(u32, u32), u64> = BTreeMap::new();
    for (w, p) in run.bake.written.iter().zip(&placements) {
        // 78 strips of 12738 x 220, and four small pictures (under 1000 pixels across).
        let picture = p.width < 1000;
        assert!(
            picture || (p.width, p.height) == (12738, 220),
            "image {}",
            w.index
        );
        strips += u64::from(!picture);
        *sizes.entry((p.width, p.height)).or_insert(0u64) += 1;
        assert_eq!(p.tile_edge, 256);
        let longest = f64::from(p.width.max(p.height));
        let levels = 1 + (longest / 256.0).log2().ceil() as u8;
        assert_eq!(p.levels, levels, "image {}", w.index);
        assert_eq!(p.bit_depth, 8);
        assert_eq!(format!("{:?}", p.tile_codec), "Png");
        // The soft mask is baked into alpha, so the channels are RGBA, never RGB.
        assert_eq!(p.pixel_format, PixelFormat::Rgba, "image {}", w.index);
        assert!(
            picture || run.page.images[w.index].smask,
            "image {}",
            w.index
        );
        // The ICC profile is not carried; the colour is an sRGB approximation and the report says so.
        let notes: Vec<_> = w.notes.iter().map(|n| n.as_str()).collect();
        assert!(notes.contains(&"colour-space-approximated"), "{notes:?}");
        assert_eq!(p.visible_box, w.pyramid.visible_box);
        assert!(p.visible_box[2] > p.visible_box[0] && p.visible_box[3] > p.visible_box[1]);
    }
    assert_eq!(strips, 78);
    assert_eq!(
        sizes,
        BTreeMap::from([
            ((412, 1843), 1),
            ((683, 396), 2),
            ((683, 397), 1),
            ((12738, 220), 78)
        ])
    );
    assert_eq!(run.bake.report.colour_approximated, 82);
    assert_eq!(run.bake.report.clipped, 82);
    assert_eq!(
        run.bake.report.written_by_filter.get("FlateDecode"),
        Some(&81)
    );
    assert_eq!(run.bake.report.written_by_filter.get("DCTDecode"), Some(&1));
    assert_eq!(run.bake.report.re_encoded_lossy, 1);
    assert_eq!(run.bake.report.interleaved, 81);
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn the_reader_returns_the_tiles_the_producer_cut_and_they_hold_pixels() {
    let run = first();
    let reader = PvecReader::open(run.artifact.clone()).unwrap();
    // Every tile of three strips (first, middle, last) is byte-equal through the reader.
    let n = run.bake.written.len();
    let mut varied_total = 0u64;
    for i in [0, n / 2, n - 1] {
        let pyramid = &run.bake.written[i].pyramid;
        let id = RasterId::new(i as u32).unwrap();
        let mut count = 0u64;
        for (level, col, row, bytes) in pyramid.tiles() {
            let got = reader.raster_tile(id, level, col, row).unwrap();
            assert_eq!(
                got.as_deref(),
                Some(bytes),
                "raster {i} tile {level}/{col}/{row}"
            );
            count += 1;
        }
        assert!(count > 20, "raster {i}: {count} tiles");

        // A tile inside the visible box is 256 x 256 and holds opaque pixels.
        let top = u32::from(pyramid.levels) - 1;
        let vb = pyramid.visible_box;
        let col = (vb[0] + vb[2]) / 2 / pyramid.tile_edge;
        let tile = reader
            .raster_tile(id, top, col, 0)
            .unwrap()
            .expect("a stored tile");
        let img = image::load_from_memory(&tile).unwrap().to_rgba8();
        assert_eq!((img.width(), img.height()), (256, 256));
        assert!(
            img.pixels().filter(|p| p.0[3] == 255).count() > 1000,
            "raster {i}"
        );
        // Somewhere in the strip the drawing is not one flat colour (the lines are ink).
        let varied = pyramid
            .tiles()
            .filter(|t| t.0 == top)
            .any(|(_, _, _, bytes)| {
                let t = image::load_from_memory(bytes).unwrap().to_rgba8();
                let first = *t.get_pixel(0, 0);
                t.pixels()
                    .any(|p| p.0[3] == 255 && first.0[3] == 255 && p.0[..3] != first.0[..3])
            });
        varied_total += u64::from(varied);
    }
    assert!(
        varied_total >= 1,
        "three strips and not a mark on any of them"
    );
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn two_runs_give_the_same_bytes() {
    let a = first();
    let b = convert();
    assert_eq!(a.artifact.len(), b.artifact.len());
    let digest = |v: &[u8]| Sha256::digest(v);
    assert_eq!(digest(&a.artifact), digest(&b.artifact));
    assert_eq!(a.bake.report, b.bake.report);
    // No arm64 baseline machinery exists in this repo for PVec artifacts, so cross-machine
    // equality is not checked here.
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn every_refused_or_dropped_item_has_a_documented_reason_key() {
    let run = first();
    let docs = docs();

    // Images the bake refused.
    assert_eq!(run.bake.refused.len(), 7);
    let mut keys: BTreeMap<String, u64> = BTreeMap::new();
    for r in &run.bake.refused {
        *keys.entry(r.error.reason()).or_insert(0) += 1;
    }
    assert_eq!(
        run.bake.report.refused, keys,
        "the report counts what was refused"
    );
    for key in keys.keys() {
        assert!(documented(&docs, key), "image refusal `{key}` is in no doc");
    }
    // Notes on the images that were written.
    for w in &run.bake.written {
        for n in &w.notes {
            assert!(
                documented(&docs, n.as_str()),
                "note `{}` is in no doc",
                n.as_str()
            );
        }
    }
    // Fills, shadings, patterns and images the policy counts instead of carrying.
    assert!(run.fills.rows().count() > 0);
    for row in run.fills.rows() {
        assert!(
            documented(&docs, row.policy.reason),
            "fill reason `{}` is in no doc",
            row.policy.reason
        );
    }
    // Text is omitted, but counted: four show operators, subset-embedded fonts.
    assert_eq!(run.page.text.pdf.tj, 4);
    assert!(docs.contains("omit text and report the counts"));
    // Segments PVec refused: none, and the map is empty rather than missing.
    assert!(run.vector_refused.is_empty());
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn the_one_call_writes_what_the_composed_pieces_write() {
    let run = first();
    let dir = std::env::temp_dir().join(format!("libviprs-tests-351-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("blueprint-mix.pvec");
    let bytes = fixture();
    let out = PdfConversion::new(CadSource::Bytes(&bytes), &dest)
        .convert()
        .expect("the page converts in one call");
    drop(bytes);

    assert_eq!(out.written, run.pushed_polylines + run.pushed_beziers);
    assert_eq!(out.rasters, 82);
    let pdf = out
        .report
        .pdf
        .as_ref()
        .expect("a PDF report has a pdf section");
    assert_eq!((pdf.page, pdf.images, pdf.rasters), (1, 89, 82));
    assert_eq!(pdf.images_refused, run.bake.report.refused);
    assert_eq!(out.images, run.bake.report);
    assert_eq!(out.report.totals.encountered, out.fidelity.encountered());
    assert_eq!(
        out.report.totals.encountered, out.written,
        "no segment was refused"
    );

    // What was published is what came back, and it opens with PVec's reader.
    let json = std::fs::read_to_string(report_path(&dest)).unwrap();
    assert_eq!(json, out.report.to_json_string().unwrap());
    let reader = open_published(&dest).expect("PVec opens what was published");
    assert_eq!(reader.entity_count().unwrap(), out.written);
    assert_eq!(reader.raster_placements().unwrap().len(), 82);
    // The same placements as the composed run, bit for bit.
    let composed = PvecReader::open(run.artifact.clone()).unwrap();
    let (a, b) = (
        reader.raster_placements().unwrap(),
        composed.raster_placements().unwrap(),
    );
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(
            x.pixel_to_source.map(f64::to_bits),
            y.pixel_to_source.map(f64::to_bits)
        );
        assert_eq!(
            (x.width, x.height, x.visible_box),
            (y.width, y.height, y.visible_box)
        );
    }
    // Every key it counts is one a doc names.
    let docs = docs();
    for key in pdf.images_refused.keys() {
        assert!(documented(&docs, key), "`{key}` is in no doc");
    }
    for key in pdf.fills.keys().chain(pdf.image_notes.keys()) {
        assert!(documented(&docs, key), "`{key}` is in no doc");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
#[ignore = "converts the 51 MB blueprint, about a minute in release; run by nightly.yml"]
fn peak_memory_is_under_the_ceiling_and_the_numbers_are_recorded() {
    let run = first();
    eprintln!(
        "blueprint-mix: {} entities ({} polylines, {} cubics), {} rasters, {} artifact bytes, \
         {:?} wall, peak RSS {:?} KiB, refused {:?}",
        run.pushed_polylines + run.pushed_beziers,
        run.pushed_polylines,
        run.pushed_beziers,
        run.bake.written.len(),
        run.artifact.len(),
        run.wall,
        run.peak_rss_kib,
        run.bake.report.refused
    );
    match run.peak_rss_kib {
        Some(kib) => assert!(
            kib < RSS_CEILING_KIB,
            "peak RSS {kib} KiB is over the 1.5 GiB ceiling of libviprs#1064"
        ),
        None => eprintln!("no /proc/self/status here: peak RSS is not checked on this platform"),
    }
}
