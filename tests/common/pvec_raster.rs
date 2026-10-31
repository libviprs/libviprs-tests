//! Shared by the `pvec_raster_*` suites and by `gen_source_rasters.rs` (libviprs-tests#305).
//!
//! Two jobs. The first is the **fixtures**: every file under `tests/fixtures/pvec-raster/` is made
//! by a function here, so the committed bytes can be regenerated and a test proves they were
//! (`gen_source_rasters.rs`). The second is the **plumbing** the suites share: a drawing for the
//! raster to ride (PVec's WRT-01 wants an entity), the producer's pyramid written into it, and a
//! few tile helpers.
//!
//! The fixture bytes depend on the lockfile (the `image` PNG encoder, the `lopdf` writer and the
//! PVec writer), so `SHA256SUMS` is a digest of what this tree makes, and a bump of any of them
//! shows up as a failing regeneration test rather than as a silent change of the inputs.

#![allow(dead_code)]

use std::path::PathBuf;

use image::ImageEncoder;
use libviprs::{PixelFormat, Raster};
use sha2::{Digest, Sha256};

/// Where the committed fixtures live.
pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pvec-raster")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// The sizes the issue names
// ---------------------------------------------------------------------------

/// The scan: odd sizes on purpose, so the last column and row of tiles pad.
pub const SCAN_W: u32 = 1537;
pub const SCAN_H: u32 = 1031;
/// The blank quarter of the scan starts here (a multiple of 256, so whole tiles are blank).
pub const BLANK_X: u32 = 768;
pub const BLANK_Y: u32 = 512;
pub const PHOTO: (u32, u32) = (700, 500);
pub const SHEET: (u32, u32) = (600, 400);
/// The clip baked into the sheet's alpha, as `visible_box` (`x0 y0 x1 y1`, exclusive end). Asymmetric
/// on purpose: 37 columns off the left, 37 off the right, but 11 rows off the top and 20 off the bottom.
pub const SHEET_VISIBLE: [u32; 4] = [37, 11, 563, 380];
/// The test vector of `RASTER.md` section 5.
pub const VECTOR_SIZE: (u32, u32) = (1024, 512);
pub const VECTOR_PLACEMENT: [f64; 6] = [0.5, 0.25, 0.125, -0.5, 100.0, 50.0];
/// The world file's image, and the placement its two readings give.
pub const POINT_SIZE: (u32, u32) = (300, 200);

// ---------------------------------------------------------------------------
// Generators. All deterministic, none uses a clock or a random seed.
// ---------------------------------------------------------------------------

fn lcg(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *state >> 8
}

/// The scan: a paper tone, a ruled grid, a diagonal, some dotted detail, and a blank (pure 255)
/// quarter at the bottom right. 1,537 by 1,031.
pub fn scan_gray() -> Raster {
    let (w, h) = (SCAN_W, SCAN_H);
    let mut seed = 0x5CA9_u32;
    let mut data = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let blank = x >= BLANK_X && y >= BLANK_Y;
            let v = if blank {
                255
            } else if x % 97 == 0 || y % 89 == 0 {
                20
            } else if (i64::from(x) - i64::from(y) * 14 / 10).abs() < 2 {
                64
            } else if lcg(&mut seed).is_multiple_of(41) {
                90
            } else {
                // Paper: a faint, position-dependent tone so no two tiles are the same bytes.
                (236 + (x / 7 + y / 11) % 9) as u8
            };
            data[(y * w + x) as usize] = v;
        }
    }
    Raster::new(w, h, PixelFormat::Gray8, data).expect("a gray raster")
}

/// The scan thresholded to black and white. Stored as a real 1-bit PNG, and read back as the
/// eight-bit raster of the same two values (the producer cuts eight-bit samples).
pub fn scan_bilevel() -> Raster {
    let s = scan_gray();
    let data = s
        .data()
        .iter()
        .map(|&v| if v < 128 { 0 } else { 255 })
        .collect();
    Raster::new(s.width(), s.height(), PixelFormat::Gray8, data).expect("a gray raster")
}

/// An RGB gradient with a little structure, for the JPEG tiles.
pub fn photo_rgb() -> Raster {
    let (w, h) = PHOTO;
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let ring = ((x as f64 - 350.0).hypot(y as f64 - 250.0) / 6.0).sin();
            data.push((x * 255 / (w - 1)) as u8);
            data.push((y * 255 / (h - 1)) as u8);
            data.push((128.0 + 100.0 * ring) as u8);
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).expect("an rgb raster")
}

/// An RGBA sheet: opaque colour inside [`SHEET_VISIBLE`], alpha 0 (and colour 0) outside, which is
/// what a clip baked into alpha looks like. The clip is a rectangle with one chamfered corner so it
/// is not symmetric either way.
pub fn sheet_rgba() -> Raster {
    let (w, h) = SHEET;
    let [x0, y0, x1, y1] = SHEET_VISIBLE;
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let inside = x >= x0 && x < x1 && y >= y0 && y < y1
                // chamfer the top-left corner by 20 pixels, inside the box
                && !(x - x0 + y - y0 < 20);
            if inside {
                data.extend_from_slice(&[
                    (x * 255 / (w - 1)) as u8,
                    (y * 255 / (h - 1)) as u8,
                    ((x ^ y) & 0xFF) as u8,
                    if (x + y) % 50 < 5 { 128 } else { 255 },
                ]);
            } else {
                data.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Raster::new(w, h, PixelFormat::Rgba8, data).expect("an rgba raster")
}

/// The bounding box of the non-transparent pixels, `x0 y0 x1 y1` with the end exclusive.
pub fn alpha_box(r: &Raster) -> [u32; 4] {
    let (w, h) = (r.width(), r.height());
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if r.data()[((y * w + x) * 4 + 3) as usize] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    [x0, y0, x1, y1]
}

/// The 1,024 by 512 image of `RASTER.md`'s test vector: a gray ramp with a corner marker, so a
/// flipped or transposed placement could not read back the same.
pub fn vector_gray() -> Raster {
    let (w, h) = VECTOR_SIZE;
    let mut data = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push(if x < 40 && y < 40 {
                0
            } else {
                ((x / 4 + y / 2) % 251) as u8
            });
        }
    }
    Raster::new(w, h, PixelFormat::Gray8, data).expect("a gray raster")
}

/// The image the world file belongs to.
pub fn point_gray() -> Raster {
    let (w, h) = POINT_SIZE;
    let data = (0..w * h).map(|i| ((i % w) ^ (i / w)) as u8).collect();
    Raster::new(w, h, PixelFormat::Gray8, data).expect("a gray raster")
}

/// A world file (`.pgw`): the pixel size, the rotation terms, and the coordinates of the **centre**
/// of the upper-left pixel, which is the `PixelIsPoint` reading. The same numbers read as a corner
/// would be half a pixel off, and the placement tests pin the difference.
pub const WORLD_FILE: &str = "0.5\n0.0\n0.0\n-0.5\n100.25\n199.75\n";

/// The six numbers of a world file, in the file's own order `A D B E C F` turned into
/// [`libviprs::GeoTransform`] order `[a b c d e f]`.
pub fn parse_world_file(text: &str) -> [f64; 6] {
    let v: Vec<f64> = text
        .lines()
        .map(|l| l.trim().parse().expect("a number"))
        .collect();
    assert_eq!(v.len(), 6, "a world file has six lines");
    [v[0], v[2], v[4], v[1], v[3], v[5]]
}

// ---- encoding fixtures -----------------------------------------------------

pub fn png_of(r: &Raster) -> Vec<u8> {
    let color = match r.format() {
        PixelFormat::Gray8 => image::ExtendedColorType::L8,
        PixelFormat::Rgb8 => image::ExtendedColorType::Rgb8,
        PixelFormat::Rgba8 => image::ExtendedColorType::Rgba8,
        other => panic!("not a fixture format: {other:?}"),
    };
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(r.data(), r.width(), r.height(), color)
        .expect("png encode");
    out
}

/// A true 1-bit PNG of a two-valued gray raster.
pub fn png_1bit(r: &Raster) -> Vec<u8> {
    let (w, h) = (r.width() as usize, r.height() as usize);
    let stride = w.div_ceil(8);
    let mut rows = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            if r.data()[y * w + x] >= 128 {
                rows[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::One);
        let mut writer = enc.write_header().expect("png header");
        writer.write_image_data(&rows).expect("png data");
    }
    out
}

/// Decode a PNG fixture to the raster the producer takes. A 1-bit file comes back as eight-bit
/// gray with the values 0 and 255.
pub fn raster_of_png(bytes: &[u8]) -> Raster {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).expect("a png");
    match img {
        image::DynamicImage::ImageLuma8(g) => {
            Raster::new(g.width(), g.height(), PixelFormat::Gray8, g.into_raw()).unwrap()
        }
        image::DynamicImage::ImageRgb8(g) => {
            Raster::new(g.width(), g.height(), PixelFormat::Rgb8, g.into_raw()).unwrap()
        }
        image::DynamicImage::ImageRgba8(g) => {
            Raster::new(g.width(), g.height(), PixelFormat::Rgba8, g.into_raw()).unwrap()
        }
        other => panic!("fixture decoded to {:?}", other.color()),
    }
}

/// Load a committed PNG fixture.
pub fn load(name: &str) -> Raster {
    let path = dir().join(name);
    raster_of_png(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

// ---- the drawing the rasters ride --------------------------------------------

/// The small vector drawing every raster rides: a closed outline, one entity (WRT-01), in a sheet
/// large enough for every fixture's placement.
pub fn drawing_writer() -> pvec_facade::PvecWriter {
    use pvec_facade::{Bounds2, Curve, EntityId, Manifest, PvecWriter};
    use pvec_geometry::{Point2, Polyline};
    let mut w = PvecWriter::new(Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 {
            min: [-1000.0, -1000.0],
            max: [4000.0, 4000.0],
        },
        capabilities: Vec::new(),
        producer_id: "libviprs-tests-305".into(),
        producer_version: "1".into(),
    })
    .expect("a manifest");
    let ring = Polyline::new(vec![
        Point2 { x: 0.0, y: 0.0 },
        Point2 { x: 1500.0, y: 0.0 },
        Point2 {
            x: 1500.0,
            y: 1000.0,
        },
        Point2 { x: 0.0, y: 1000.0 },
        Point2 { x: 0.0, y: 0.0 },
    ])
    .expect("a ring");
    w.push(EntityId::new(0).unwrap(), &Curve::Polyline(ring))
        .expect("the outline");
    w
}

/// The drawing on its own: the raster-free twin of every file the suites write.
pub fn drawing_pvec() -> Vec<u8> {
    drawing_writer().finish().expect("the drawing finishes")
}

// ---- the three-image PDF ------------------------------------------------------

/// A one-page PDF with three images: two image XObjects (a 4 by 4 gray one and a 3 by 2 RGB one)
/// and one inline image (2 by 2 gray). Built in memory, so the pixels are the ones written here.
pub fn three_image_pdf() -> Vec<u8> {
    use lopdf::{Dictionary, Document, Object, Stream, dictionary};
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let gray: Vec<u8> = (0..16).map(|i| i * 16).collect();
    let rgb: Vec<u8> = (0..18).map(|i| i * 14).collect();
    let img = |w: i64, h: i64, cs: &str, data: Vec<u8>| {
        Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => w, "Height" => h,
                "ColorSpace" => cs, "BitsPerComponent" => 8,
            },
            data,
        )
    };
    let im1 = doc.add_object(img(4, 4, "DeviceGray", gray));
    let im2 = doc.add_object(img(3, 2, "DeviceRGB", rgb));
    let mut content = Vec::new();
    content.extend_from_slice(b"q 100 0 0 100 50 600 cm /Im1 Do Q\n");
    content.extend_from_slice(b"q 150 0 0 100 200 600 cm /Im2 Do Q\n");
    content.extend_from_slice(b"q 60 0 0 60 400 600 cm\nBI /W 2 /H 2 /CS /DeviceGray /BPC 8 ID ");
    content.extend_from_slice(&[0x00, 0x55, 0xAA, 0xFF]);
    content.extend_from_slice(b" EI Q\n");
    let content = doc.add_object(Stream::new(Dictionary::new(), content));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => content,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => im1, "Im2" => im2 } },
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![Object::from(page)], "Count" => 1,
        }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("pdf");
    out
}

// ---- the whole set --------------------------------------------------------------

/// Every committed fixture as (file name, bytes), in the order `SHA256SUMS` lists them.
pub fn all() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("scan_gray_1537x1031.png", png_of(&scan_gray())),
        ("scan_1bit_1537x1031.png", png_1bit(&scan_bilevel())),
        ("photo_rgb_700x500.png", png_of(&photo_rgb())),
        ("sheet_rgba_600x400.png", png_of(&sheet_rgba())),
        ("vector_gray_1024x512.png", png_of(&vector_gray())),
        ("point_gray_300x200.png", png_of(&point_gray())),
        ("point_gray_300x200.pgw", WORLD_FILE.as_bytes().to_vec()),
        ("three_images.pdf", three_image_pdf()),
        ("drawing.pvec", drawing_pvec()),
    ]
}

/// `SHA256SUMS` as `sha256sum -c` reads it.
pub fn sums(files: &[(&'static str, Vec<u8>)]) -> String {
    files
        .iter()
        .map(|(name, bytes)| format!("{}  {name}\n", sha256_hex(bytes)))
        .collect()
}

// ---------------------------------------------------------------------------
// Plumbing the suites share
// ---------------------------------------------------------------------------

/// A tile edge of 256, a Google plan over the whole image.
pub fn plan(w: u32, h: u32, edge: u32) -> libviprs::PyramidPlan {
    libviprs::PyramidPlanner::new(w, h, edge, 0, libviprs::Layout::Google)
        .expect("a plan")
        .plan()
}

/// The pixels of tile `(col, row)` of a decoded tile image, cropped to the image, row by row.
pub fn crop(tile: &[u8], edge: u32, bpp: usize, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for row in 0..h {
        let start = ((y + row) * edge + x) as usize * bpp;
        out.extend_from_slice(&tile[start..start + w as usize * bpp]);
    }
    out
}

// ---- interchange ---------------------------------------------------------------

/// The scan (blank quarter not stored, attribution set, the sheared placement of `RASTER.md`)
/// written into the drawing: the pyramid and the `.pvec` bytes. Shared by the interchange suites.
#[cfg(feature = "pvec-raster")]
pub fn interchange_pvec() -> (libviprs::raster_pvec::pyramid::RasterPyramid, Vec<u8>) {
    use libviprs::raster_pvec::container::write_rasters;
    use libviprs::raster_pvec::pyramid::{
        RasterPlacement, RasterPyramidOptions, produce_raster_pyramid,
    };
    let raster = load("scan_gray_1537x1031.png");
    let mut options = RasterPyramidOptions::new(RasterPlacement::Explicit([
        0.5, 0.25, 0.125, -0.5, 100.0, 50.0,
    ]));
    options.skip_blanks = true;
    options.attribution = Some("Scan of the ground floor, 1987".to_string());
    let pyramid =
        produce_raster_pyramid(&raster, &plan(1537, 1031, 256), &options).expect("pyramid");
    let mut w = drawing_writer();
    write_rasters(&mut w, std::slice::from_ref(&pyramid)).expect("write");
    (pyramid, w.finish().expect("finish"))
}

/// The PMTiles archive pvec-interchange exports for raster 0 of a `.pvec`.
#[cfg(feature = "pvec-raster")]
pub fn export_raster_0(pvec: &[u8]) -> Vec<u8> {
    use pvec_container::reader::{ContainerReader, ReaderConfig};
    use pvec_container::source::MemorySource;
    let r = ContainerReader::open(MemorySource::new(pvec.to_vec()), ReaderConfig::default())
        .expect("the file opens");
    pvec_interchange::pmtiles::export_raster(&r, 0).expect("the export")
}
