//! Raster fixtures through the producer into a `.pvec`, and back out through PVec's reader
//! (libviprs-tests#305, EPIC R-L, libviprs/libviprs#1329).
//!
//! The library tests of libviprs/libviprs#1328 prove the producer against a synthetic raster inside
//! the crate. This is the other thing: a built artefact, real files under it (committed with
//! provenance in `tests/fixtures/pvec-raster/`), and the claims of ADR 0018 run end to end:
//!
//! * every tile PVec's reader returns is digest-equal to the tile the producer made;
//! * the six coefficients of a composed placement come back as the same bits;
//! * the decoded pixels of every lossless tile equal the source's, at tolerance 0;
//! * the JPEG fixture's tiles are byte-equal, with no claim about pixels (lossy is the producer's
//!   choice, ADR 0018 "Effect elsewhere" item 3);
//! * a blank tile is not stored, and the padded margin of an edge tile is the background, outside
//!   the `visible_box`;
//! * the three-image PDF writes three rasters and the count of image constructs is written plus
//!   refused (libviprs/libviprs#1332).
//!
//! Needs `--features pvec-raster`.

#![cfg(feature = "pvec-raster")]

#[path = "common/pvec_raster.rs"]
mod fx;

use libviprs::raster_pvec::container::write_rasters;
use libviprs::raster_pvec::placement::SourceCalibration;
use libviprs::raster_pvec::pyramid::{
    RasterCodec, RasterPlacement, RasterPyramid, RasterPyramidOptions, produce_raster_pyramid,
};
use libviprs::{GeoTransform, PixelOrigin, Raster, TileFormat};
use pvec_facade::unstable::{PixelFormat as Channels, RasterId};
use pvec_facade::{PvecReader, PvecWriter};

fn bits(m: [f64; 6]) -> [u64; 6] {
    m.map(f64::to_bits)
}

fn explicit(m: [f64; 6]) -> RasterPyramidOptions {
    RasterPyramidOptions::new(RasterPlacement::Explicit(m))
}

/// Cut `raster` and write it into the drawing; the pyramid and the file.
fn through_pvec(
    raster: &Raster,
    edge: u32,
    options: &RasterPyramidOptions,
) -> (RasterPyramid, PvecReader) {
    let pyramid = produce_raster_pyramid(
        raster,
        &fx::plan(raster.width(), raster.height(), edge),
        options,
    )
    .expect("the producer cuts the fixture");
    let mut w: PvecWriter = fx::drawing_writer();
    write_rasters(&mut w, std::slice::from_ref(&pyramid)).expect("PVec takes the pyramid");
    let reader = PvecReader::open(w.finish().expect("finish")).expect("PVec opens its own file");
    (pyramid, reader)
}

fn id0() -> RasterId {
    RasterId::new(0).unwrap()
}

/// The digest comparison, as a function so its control can call it too.
fn same_digest(a: &[u8], b: &[u8]) -> bool {
    fx::sha256_hex(a) == fx::sha256_hex(b)
}

fn assert_every_tile_digest_equal(p: &RasterPyramid, reader: &PvecReader, what: &str) {
    let mut n = 0;
    for (l, c, r, planned) in p.tiles() {
        let read = reader
            .raster_tile(id0(), l, c, r)
            .unwrap_or_else(|e| panic!("{what} {l}/{c}/{r}: {e:?}"))
            .unwrap_or_else(|| panic!("{what}: tile {l}/{c}/{r} is missing"));
        assert!(
            same_digest(&read, planned),
            "{what}: tile {l}/{c}/{r} differs from the planner's"
        );
        n += 1;
    }
    assert!(n > 0, "{what}: the pyramid has tiles");
}

// ---------------------------------------------------------------------------
// Digests
// ---------------------------------------------------------------------------

#[test]
fn every_tile_of_every_lossless_fixture_reads_back_digest_equal_to_the_planners() {
    for (name, edge) in [
        ("scan_gray_1537x1031.png", 256),
        ("scan_gray_1537x1031.png", 512),
        ("scan_1bit_1537x1031.png", 256),
        ("photo_rgb_700x500.png", 256),
        ("sheet_rgba_600x400.png", 256),
        ("vector_gray_1024x512.png", 512),
    ] {
        let raster = fx::load(name);
        let (p, reader) =
            through_pvec(&raster, edge, &explicit([1.0, 0.0, 0.0, -1.0, 0.0, 1000.0]));
        assert_every_tile_digest_equal(&p, &reader, &format!("{name} @{edge}"));
    }
}

#[test]
fn a_tile_with_one_flipped_byte_fails_the_digest_comparison() {
    // The control: the comparison above is not one that passes whatever it is given.
    let raster = fx::load("scan_gray_1537x1031.png");
    let (p, reader) = through_pvec(&raster, 256, &explicit([1.0, 0.0, 0.0, -1.0, 0.0, 1000.0]));
    let (l, c, r, planned) = p.tiles().next().unwrap();
    let read = reader.raster_tile(id0(), l, c, r).unwrap().unwrap();
    assert!(same_digest(&read, planned));
    let mut flipped = read.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x01;
    assert!(!same_digest(&flipped, planned));
}

// ---------------------------------------------------------------------------
// Placement: bit-equal after the composition
// ---------------------------------------------------------------------------

fn calibration(metres_to_native: GeoTransform, metres_per_unit: f64) -> SourceCalibration {
    SourceCalibration::Affine {
        native_crs: "EPSG:2056".to_string(),
        metres_to_native,
        metres_per_unit,
    }
}

fn composed(t: [f64; 6], origin: PixelOrigin, cal: SourceCalibration) -> RasterPyramidOptions {
    RasterPyramidOptions::new(RasterPlacement::Composed {
        pixel_to_native: GeoTransform::new(t[0], t[1], t[2], t[3], t[4], t[5]),
        pixel_origin: origin,
        raster_crs: "EPSG:2056".to_string(),
        calibration: cal,
    })
}

#[test]
fn the_rasters_document_test_vector_comes_back_with_the_same_bits_and_the_same_corners() {
    // RASTER.md section 5: every number is exact in binary, so the corners are exact.
    let raster = fx::load("vector_gray_1024x512.png");
    let (_, reader) = through_pvec(&raster, 512, &explicit(fx::VECTOR_PLACEMENT));
    let pl = &reader.raster_placements().unwrap()[0];
    assert_eq!(bits(pl.pixel_to_source), bits(fx::VECTOR_PLACEMENT));
    assert_eq!((pl.width, pl.height), (1024, 512));
    assert_eq!(pl.visible_box, [0, 0, 1024, 512]);
    let [a, b, c, d, tx, ty] = pl.pixel_to_source;
    let at = |px: f64, py: f64| (a * px + b * py + tx, c * px + d * py + ty);
    assert_eq!(at(0.0, 0.0), (100.0, 50.0));
    assert_eq!(at(1024.0, 0.0), (612.0, 178.0));
    assert_eq!(at(1024.0, 512.0), (740.0, -78.0));
    assert_eq!(at(0.0, 512.0), (228.0, -206.0));
    assert_eq!(at(0.5, 0.5), (100.375, 49.8125));
    // Negative determinant: an upright image in y-up space.
    assert_eq!(a * d - b * c, -0.28125);
}

#[test]
fn a_world_file_is_a_half_pixel_off_a_corner_and_the_composition_says_which() {
    let text = std::fs::read_to_string(fx::dir().join("point_gray_300x200.pgw")).unwrap();
    let world = fx::parse_world_file(&text);
    let raster = fx::load("point_gray_300x200.png");
    let identity = || calibration(GeoTransform::new(1.0, 0.0, 0.0, 0.0, 1.0, 0.0), 1.0);

    // PixelIsPoint (a world file): the numbers name the centre of the upper-left pixel, so the
    // corner is half a pixel up and to the left: 100.25 - 0.25 and 199.75 + 0.25.
    let (_, reader) = through_pvec(
        &raster,
        256,
        &composed(world, PixelOrigin::Centre, identity()),
    );
    let point = reader.raster_placements().unwrap()[0].pixel_to_source;
    assert_eq!(bits(point), bits([0.5, 0.0, 0.0, -0.5, 100.0, 200.0]));

    // PixelIsArea: the same six numbers are the corner already, and nothing shifts.
    let (_, reader) = through_pvec(
        &raster,
        256,
        &composed(world, PixelOrigin::Corner, identity()),
    );
    let area = reader.raster_placements().unwrap()[0].pixel_to_source;
    assert_eq!(bits(area), bits([0.5, 0.0, 0.0, -0.5, 100.25, 199.75]));
    assert_ne!(point, area, "the half-pixel shift is the whole difference");
}

#[test]
fn a_calibration_is_composed_through_its_inverse_and_the_unit_scale() {
    // The drawing is in millimetres whose origin sits at (90, 150) in the raster's CRS.
    let raster = fx::load("point_gray_300x200.png");
    let cal = calibration(GeoTransform::new(1.0, 0.0, 90.0, 0.0, 1.0, 150.0), 0.001);
    let (_, reader) = through_pvec(
        &raster,
        256,
        &composed(
            [0.5, 0.0, 100.0, 0.0, -0.5, 200.0],
            PixelOrigin::Corner,
            cal,
        ),
    );
    let m = reader.raster_placements().unwrap()[0].pixel_to_source;
    // (native - (90, 150)) / 0.001 mm: 0.5 m a pixel is 500 mm, the origin (10, 50) m is (10000, 50000).
    assert_eq!(bits(m), bits([500.0, 0.0, 0.0, -500.0, 10_000.0, 50_000.0]));
}

#[test]
fn a_crs_mismatch_is_a_typed_refusal_and_nothing_is_written() {
    let raster = fx::load("point_gray_300x200.png");
    let options = RasterPyramidOptions::new(RasterPlacement::Composed {
        pixel_to_native: GeoTransform::new(0.5, 0.0, 100.0, 0.0, -0.5, 200.0),
        pixel_origin: PixelOrigin::Corner,
        raster_crs: "EPSG:4326".to_string(),
        calibration: calibration(GeoTransform::new(1.0, 0.0, 0.0, 0.0, 1.0, 0.0), 1.0),
    });
    let err = produce_raster_pyramid(&raster, &fx::plan(300, 200, 256), &options)
        .expect_err("a placement never resamples");
    assert!(format!("{err:?}").contains("CrsMismatch"), "{err:?}");
}

// ---------------------------------------------------------------------------
// Decoded pixels, tolerance 0
// ---------------------------------------------------------------------------

/// The decoded pixels of every stored tile of the full resolution level, cut back to the image,
/// against the source; the margin of an edge tile is returned for the caller to judge.
fn assert_full_level_pixels_equal_the_source(
    raster: &Raster,
    p: &RasterPyramid,
    reader: &PvecReader,
    bpp: usize,
    what: &str,
) -> Vec<Vec<u8>> {
    let top = u32::from(p.levels) - 1;
    let (cols, rows) = p.grid(top).expect("the top level has a grid");
    let edge = p.tile_edge;
    let (w, h) = (raster.width(), raster.height());
    let mut margins = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            let Some(bytes) = reader.raster_tile(id0(), top, col, row).unwrap() else {
                continue;
            };
            let img = image::load_from_memory(&bytes).expect("a lossless tile decodes");
            let tile = match bpp {
                1 => img.to_luma8().into_raw(),
                3 => img.to_rgb8().into_raw(),
                _ => img.to_rgba8().into_raw(),
            };
            assert_eq!(
                tile.len(),
                (edge * edge) as usize * bpp,
                "{what}: a full tile"
            );
            let (x, y) = (col * edge, row * edge);
            let (cw, ch) = ((w - x).min(edge), (h - y).min(edge));
            let got = fx::crop(&tile, edge, bpp, 0, 0, cw, ch);
            let mut want = Vec::new();
            for sy in y..y + ch {
                let start = (sy * w + x) as usize * bpp;
                want.extend_from_slice(&raster.data()[start..start + cw as usize * bpp]);
            }
            assert!(
                got == want,
                "{what}: tile {col},{row} differs from the source"
            );
            // The margin: everything in the tile outside the image.
            for ty in 0..edge {
                for tx in 0..edge {
                    if tx >= cw || ty >= ch {
                        let at = (ty * edge + tx) as usize * bpp;
                        margins.push(tile[at..at + bpp].to_vec());
                    }
                }
            }
        }
    }
    margins
}

#[test]
fn the_decoded_pixels_of_every_lossless_tile_equal_the_sources_at_tolerance_zero() {
    for (name, bpp) in [
        ("scan_gray_1537x1031.png", 1),
        ("scan_1bit_1537x1031.png", 1),
        ("photo_rgb_700x500.png", 3),
        ("sheet_rgba_600x400.png", 4),
    ] {
        let raster = fx::load(name);
        let (p, reader) = through_pvec(&raster, 256, &explicit([1.0, 0.0, 0.0, -1.0, 0.0, 1000.0]));
        assert_eq!(p.tile_codec, RasterCodec::Png);
        assert_full_level_pixels_equal_the_source(&raster, &p, &reader, bpp, name);
        // And the overview levels: what PVec returns decodes to the pixels of the producer's tile.
        for (l, c, r, planned) in p.tiles() {
            let read = reader.raster_tile(id0(), l, c, r).unwrap().unwrap();
            assert!(
                image::load_from_memory(&read).unwrap().as_bytes()
                    == image::load_from_memory(planned).unwrap().as_bytes(),
                "{name}: level {l} tile {c},{r}"
            );
        }
    }
}

#[test]
fn the_one_bit_fixture_is_two_valued_and_stays_two_valued() {
    let raster = fx::load("scan_1bit_1537x1031.png");
    assert!(raster.data().iter().all(|&v| v == 0 || v == 255));
    assert!(raster.data().contains(&0));
}

#[test]
fn a_bilevel_png_tile_is_stored_one_bit_and_decodes_to_the_same_pixels() {
    // The producer cuts eight-bit samples, so a 1-bit tile is the encoder's `encode_png_bilevel`
    // handed to PVec directly with bit_depth 1 (ADR 0018, "the legal table").
    use pvec_facade::unstable::{Band, Placement, TileCodec, TileIn};
    let scan = fx::load("scan_1bit_1537x1031.png");
    let tile_px: Vec<u8> = (0..256usize)
        .flat_map(|y| scan.data()[y * 1537..y * 1537 + 256].to_vec())
        .collect();
    let tile = Raster::new(256, 256, libviprs::PixelFormat::Gray8, tile_px.clone()).unwrap();
    let bytes = tile.encode_png_bilevel(128).expect("a bilevel png");
    let mut w = fx::drawing_writer();
    w.rasters(&[Placement {
        pixel_to_source: [1.0, 0.0, 0.0, -1.0, 0.0, 256.0],
        width: 256,
        height: 256,
        visible_box: [0, 0, 256, 256],
        tile_edge: 256,
        levels: 1,
        tile_codec: TileCodec::Png,
        pixel_format: Channels::Gray,
        bit_depth: 1,
        opacity: 255,
        band: Band::Underlay,
        layer_index: None,
        source_digest: None,
        attribution: String::new(),
    }])
    .expect("a one-bit placement");
    w.raster_tiles(
        id0(),
        vec![TileIn {
            level: 0,
            col: 0,
            row: 0,
            bytes: bytes.clone(),
        }],
    )
    .expect("the sniffer accepts a 1-bit tile");
    let reader = PvecReader::open(w.finish().unwrap()).unwrap();
    let read = reader.raster_tile(id0(), 0, 0, 0).unwrap().unwrap();
    assert_eq!(read, bytes);
    assert_eq!(
        reader.raster_placements().unwrap()[0].bit_depth,
        1,
        "stored as one bit"
    );
    assert_eq!(
        image::load_from_memory(&read)
            .unwrap()
            .to_luma8()
            .into_raw(),
        tile_px
    );
}

// ---------------------------------------------------------------------------
// JPEG: bytes, not pixels
// ---------------------------------------------------------------------------

#[test]
fn the_jpeg_fixtures_tiles_are_byte_equal_and_make_no_claim_about_pixels() {
    let raster = fx::load("photo_rgb_700x500.png");
    let mut options = explicit([1.0, 0.0, 0.0, -1.0, 0.0, 500.0]);
    options.tile_format = TileFormat::Jpeg { quality: 85 };
    let (p, reader) = through_pvec(&raster, 256, &options);
    assert_eq!(p.tile_codec, RasterCodec::Jpeg);
    let pl = &reader.raster_placements().unwrap()[0];
    assert_eq!(pl.pixel_format, Channels::Rgb);
    assert_every_tile_digest_equal(&p, &reader, "photo jpeg");
    for (l, c, r, planned) in p.tiles() {
        // Byte for byte, not merely the same digest function applied twice.
        assert_eq!(
            reader.raster_tile(id0(), l, c, r).unwrap().as_deref(),
            Some(planned)
        );
        // The bytes are a JPEG (SOI ... EOI), which is all that is claimed of them.
        assert_eq!(&planned[..2], &[0xFF, 0xD8]);
        assert_eq!(&planned[planned.len() - 2..], &[0xFF, 0xD9]);
    }
}

// ---------------------------------------------------------------------------
// Sparse tiles, padding and the visible box
// ---------------------------------------------------------------------------

fn scan_with_blanks_skipped() -> (Raster, RasterPyramid, PvecReader) {
    let raster = fx::load("scan_gray_1537x1031.png");
    let mut options = explicit([1.0, 0.0, 0.0, -1.0, 0.0, 1031.0]);
    options.skip_blanks = true;
    let (p, reader) = through_pvec(&raster, 256, &options);
    (raster, p, reader)
}

#[test]
fn a_blank_tile_is_not_stored_and_a_stored_tile_is_never_blank() {
    let (raster, p, reader) = scan_with_blanks_skipped();
    let top = u32::from(p.levels) - 1;
    // 1,537 by 1,031 at 256 is 7 by 5 tiles. The blank quarter starts at (768, 512), so the whole
    // tiles columns 3 to 5, rows 2 and 3 lie inside it.
    assert_eq!(p.grid(top), Some((7, 5)));
    for row in 2..4 {
        for col in 3..6 {
            assert_eq!(
                reader.raster_tile(id0(), top, col, row).unwrap(),
                None,
                "blank tile {col},{row} is stored"
            );
        }
    }
    // Stored tiles keep their pixels, and a tile with ink in it is there.
    assert!(reader.raster_tile(id0(), top, 0, 0).unwrap().is_some());
    assert!(p.report.not_stored_tiles >= 6, "{:?}", p.report);
    // Everything stored still decodes to the source (tolerance 0), blanks or not.
    assert_full_level_pixels_equal_the_source(&raster, &p, &reader, 1, "scan, blanks skipped");
    // Fewer tiles are stored than the full grid.
    let stored = p.tiles().count() as u64;
    let all: u64 = p
        .plan
        .levels
        .iter()
        .map(|l| u64::from(l.width.div_ceil(256)) * u64::from(l.height.div_ceil(256)))
        .sum();
    assert_eq!(stored + p.report.not_stored_tiles, all);
}

#[test]
fn the_padded_edge_tiles_margin_is_the_background_and_the_visible_box_excludes_it() {
    let raster = fx::load("scan_gray_1537x1031.png");
    let (p, reader) = through_pvec(&raster, 256, &explicit([1.0, 0.0, 0.0, -1.0, 0.0, 1031.0]));
    let margin = assert_full_level_pixels_equal_the_source(&raster, &p, &reader, 1, "scan");
    // 7 columns of 256 over 1,537 is 255 margin columns; 5 rows over 1,031 is 249 margin rows.
    assert_eq!(margin.len(), 7 * 256 * 5 * 256 - 1537 * 1031);
    assert!(
        margin.iter().all(|px| px == &[255]),
        "the margin is the (white) background"
    );
    let pl = &reader.raster_placements().unwrap()[0];
    assert_eq!(pl.visible_box, [0, 0, 1537, 1031]);
    assert!(pl.visible_box[2] < 7 * 256 && pl.visible_box[3] < 5 * 256);
}

#[test]
fn a_baked_clip_is_alpha_zero_and_the_visible_box_it_implies_is_stored() {
    let raster = fx::load("sheet_rgba_600x400.png");
    // The clip's box is what the alpha says, and what the fixture documents.
    assert_eq!(fx::alpha_box(&raster), fx::SHEET_VISIBLE);
    let mut pyramid = produce_raster_pyramid(
        &raster,
        &fx::plan(600, 400, 256),
        &explicit([1.0, 0.0, 0.0, -1.0, 0.0, 400.0]),
    )
    .unwrap();
    pyramid.visible_box = fx::SHEET_VISIBLE;
    let mut w = fx::drawing_writer();
    write_rasters(&mut w, std::slice::from_ref(&pyramid)).unwrap();
    let reader = PvecReader::open(w.finish().unwrap()).unwrap();
    let pl = &reader.raster_placements().unwrap()[0];
    assert_eq!(pl.visible_box, fx::SHEET_VISIBLE);
    assert_eq!(pl.pixel_format, Channels::Rgba);
    // Outside the box every decoded pixel is fully transparent: the clip is in the pixels.
    let top = u32::from(pyramid.levels) - 1;
    let [x0, y0, x1, y1] = fx::SHEET_VISIBLE;
    for row in 0..2 {
        for col in 0..3 {
            let bytes = reader.raster_tile(id0(), top, col, row).unwrap().unwrap();
            let rgba = image::load_from_memory(&bytes).unwrap().to_rgba8();
            for (tx, ty, px) in rgba.enumerate_pixels() {
                let (x, y) = (col * 256 + tx, row * 256 + ty);
                if x < 600 && y < 400 && (x < x0 || x >= x1 || y < y0 || y >= y1) {
                    assert_eq!(px.0[3], 0, "pixel {x},{y} outside the clip is visible");
                }
            }
        }
    }
}

#[test]
fn a_footprint_outside_the_visible_box_is_refused_by_pvec_not_stored() {
    // The control for the box above: PVec holds the box to the image.
    let raster = fx::load("sheet_rgba_600x400.png");
    let mut pyramid = produce_raster_pyramid(
        &raster,
        &fx::plan(600, 400, 256),
        &explicit([1.0, 0.0, 0.0, -1.0, 0.0, 400.0]),
    )
    .unwrap();
    pyramid.visible_box = [0, 0, 601, 400];
    let mut w = fx::drawing_writer();
    assert!(write_rasters(&mut w, std::slice::from_ref(&pyramid)).is_err());
}

// ---------------------------------------------------------------------------
// The three-image PDF
// ---------------------------------------------------------------------------

#[test]
fn the_three_image_pdf_writes_three_rasters_and_the_count_is_written_plus_refused() {
    use libviprs::pvec::pdf_image::{PdfImageOptions, bake_page_images};
    use libviprs::pvec::pdf_vector::{PdfVectorLimits, extract_page_paths_from_bytes};

    let bytes = std::fs::read(fx::dir().join("three_images.pdf")).unwrap();
    let page = extract_page_paths_from_bytes(&bytes, 1, &PdfVectorLimits::default()).unwrap();
    let bake = bake_page_images(&page, &PdfImageOptions::default());

    // Two XObjects and one inline image: the report counts three constructs.
    assert_eq!(bake.report.images, 3, "{:?}", bake.report);
    assert_eq!(
        bake.report.images,
        bake.report.written + bake.report.refused_total(),
        "nothing is dropped silently"
    );
    assert_eq!(bake.written.len(), 3, "refused: {:?}", bake.refused);
    assert!(bake.refused.is_empty());

    let mut w = fx::drawing_writer();
    write_rasters(&mut w, bake.written.iter().map(|b| &b.pyramid)).unwrap();
    let reader = PvecReader::open(w.finish().unwrap()).unwrap();
    let placements = reader.raster_placements().unwrap();
    assert_eq!(placements.len(), 3);

    // RasterId follows draw order: the two XObjects, then the inline image.
    assert_eq!(placements[0].pixel_format, Channels::Gray);
    assert_eq!((placements[0].width, placements[0].height), (4, 4));
    assert_eq!(placements[1].pixel_format, Channels::Rgb);
    assert_eq!((placements[1].width, placements[1].height), (3, 2));
    assert_eq!((placements[2].width, placements[2].height), (2, 2));
    // Placement of the first: CTM [100 0 0 100 50 600] over 4 by 4 is [25 0 0 -25 50 700].
    assert_eq!(
        bits(placements[0].pixel_to_source),
        bits([25.0, 0.0, 0.0, -25.0, 50.0, 700.0])
    );
    // The inline image's four samples come back exactly.
    let tile = reader
        .raster_tile(RasterId::new(2).unwrap(), 0, 0, 0)
        .unwrap()
        .expect("a stored tile");
    let g = image::load_from_memory(&tile).unwrap().to_luma8();
    let px: Vec<u8> = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .iter()
        .map(|&(x, y)| g.get_pixel(x, y).0[0])
        .collect();
    assert_eq!(px, vec![0x00, 0x55, 0xAA, 0xFF]);
}
