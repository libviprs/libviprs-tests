//! Offline generator for the canonical fixture set.
//!
//! Run with `cargo test --test gen_canonicals -- --ignored` to
//! regenerate the canonical fixtures committed under
//! `tests/fixtures/canonical*.{pdf,png}`. The generator is
//! deterministic — running it twice produces byte-identical output —
//! and the resulting fixtures are checked in. Tests in the regular
//! suite consume the committed bytes; they do **not** invoke this
//! generator.
//!
//! # Why a canonical set
//!
//! Most invariant tests in this crate (engine determinism, builder
//! commutativity, resume / retry / sink contract tests) historically
//! constructed input rasters via per-file helpers like
//! `gradient_raster(w, h)`. That left ~38 distinct generators
//! scattered across ~31 test files, each a potential source of input
//! drift, and none of them committed alongside their corresponding
//! expected outputs. The canonical set replaces the scattered
//! helpers with a small pinned input the test suite can rely on:
//!
//! - **`canonical_input.png`** — 256 × 256 RGBA8 raster, deterministic
//!   four-quadrant gradient. Replaces `gradient_raster` for invariant
//!   tests that need *some* multi-channel non-uniform input.
//! - **`canonical.pdf`** — A4 PDF (595 × 842 pt) with a simple
//!   four-quadrant fill. Smallest PDF that exercises pdfium's
//!   form-data render path with non-trivial content.
//! - **`canonical_solid_white.pdf`** — A4 PDF with an empty content
//!   stream, defaulting to the bitmap clear colour. Drives the
//!   blank-tile and solid-white tolerance paths from a checked-in
//!   PDF rather than a synthetic `Raster::new(.., vec![255; …])`.
//! - **`canonical_rotated_90.pdf`** / **`canonical_rotated_180.pdf`** /
//!   **`canonical_rotated_270.pdf`** — `canonical.pdf` with the page's
//!   `/Rotate` value flipped. Replaces the runtime-`lopdf::save`
//!   round-trip in `pdfium_streaming_synthetic_rotations.rs` with
//!   checked-in inputs that don't depend on `lopdf`'s save format being
//!   stable. The four fixtures (`canonical.pdf` at `/Rotate 0` plus the
//!   three rotated variants) cover every branch of the rotation matrix.
//!
//! Three additional canonicals (`canonical_blank.pdf`,
//! `canonical_identical_halves.pdf`, `canonical_cmyk.pdf`) are
//! tracked as follow-up work — they need PDF colour-space and
//! drawing-primitive plumbing that isn't worth the complexity in
//! this first pass when the existing canonical set already covers
//! the highest-traffic invariant tests.

use std::io::Write;
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::write::ZlibEncoder;
use lopdf::{Document, Object, ObjectId, Stream, dictionary};

const A4_W_PT: f32 = 595.0;
const A4_H_PT: f32 = 842.0;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
#[ignore = "fixture generator; run via `cargo test --test gen_canonicals -- --ignored` to rebuild"]
fn generate_canonical_fixtures() {
    let dir = fixtures_dir();
    write_canonical_input_png(&dir.join("canonical_input.png"));
    write_canonical_pdf(&dir.join("canonical.pdf"), Rotate::None);
    write_canonical_pdf(&dir.join("canonical_rotated_90.pdf"), Rotate::Quarter);
    write_canonical_pdf(&dir.join("canonical_rotated_180.pdf"), Rotate::Half);
    write_canonical_pdf(
        &dir.join("canonical_rotated_270.pdf"),
        Rotate::ThreeQuarter,
    );
    write_solid_white_pdf(&dir.join("canonical_solid_white.pdf"));

    // CMYK fixtures consumed by `tests/pdf_cmyk.rs`. Each PDF embeds a
    // single DeviceCMYK FlateDecode image XObject of the requested dims;
    // the test extracts the image and asserts the CMYK→RGB conversion.
    write_cmyk_pdf(
        &dir.join("canonical_cmyk.pdf"),
        16,
        16,
        cmyk_gradient_bytes(16, 16),
    );
    write_cmyk_pdf(
        &dir.join("canonical_cmyk_black_1x1.pdf"),
        1,
        1,
        vec![0, 0, 0, 255],
    );
    write_cmyk_pdf(
        &dir.join("canonical_cmyk_cyan_1x1.pdf"),
        1,
        1,
        vec![255, 0, 0, 0],
    );
}

// ---------------------------------------------------------------------------
// Raster: deterministic 256x256 four-quadrant gradient.
//
// Each quadrant is 128x128 with one dominant channel that varies linearly
// from 0..256 across the quadrant. Reading the byte at (x, y) channel c
// yields a value that's a function purely of (x, y, c) — the file is
// reproducible bit-for-bit.
// ---------------------------------------------------------------------------

fn write_canonical_input_png(path: &Path) {
    use image::{ImageBuffer, Rgba};

    const SIZE: u32 = 256;
    let half = SIZE / 2;
    let mut img: ImageBuffer<Rgba<u8>, Vec<u8>> = ImageBuffer::new(SIZE, SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let in_left = x < half;
            let in_top = y < half;
            // Per-quadrant ramp 0..255 across the quadrant's own width.
            let ramp_x = ((x % half) * 255 / (half - 1)) as u8;
            let ramp_y = ((y % half) * 255 / (half - 1)) as u8;
            let pixel = match (in_top, in_left) {
                // top-left: red ramp on x
                (true, true) => Rgba([ramp_x, 0, 0, 255]),
                // top-right: green ramp on x
                (true, false) => Rgba([0, ramp_x, 0, 255]),
                // bottom-left: blue ramp on y
                (false, true) => Rgba([0, 0, ramp_y, 255]),
                // bottom-right: yellow ramp on x+y, deterministic mix
                (false, false) => Rgba([ramp_x, ramp_y, 0, 255]),
            };
            img.put_pixel(x, y, pixel);
        }
    }
    img.save(path)
        .unwrap_or_else(|e| panic!("save canonical_input.png: {e}"));
}

// ---------------------------------------------------------------------------
// PDFs: minimal A4 documents constructed via lopdf.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Rotate {
    None,
    Quarter,
    Half,
    ThreeQuarter,
}

impl Rotate {
    fn degrees(self) -> i64 {
        match self {
            Rotate::None => 0,
            Rotate::Quarter => 90,
            Rotate::Half => 180,
            Rotate::ThreeQuarter => 270,
        }
    }
}

/// PDF content stream that fills the A4 page with four solid-colour
/// quadrants. Origin is bottom-left in PDF user space; quadrants are
/// each `(A4_W_PT / 2) × (A4_H_PT / 2)`. Colours are PDF RGB
/// (0.0 – 1.0).
fn four_quadrant_content_stream() -> Vec<u8> {
    let half_w = A4_W_PT / 2.0;
    let half_h = A4_H_PT / 2.0;
    // PDF operators:
    //   r g b rg     — set non-stroking colour
    //   x y w h re   — append rectangle to path
    //   f            — fill path
    let stream = format!(
        // Bottom-left blue
        "0 0 1 rg 0 0 {half_w} {half_h} re f \
         0 1 0 rg {half_w} 0 {half_w} {half_h} re f \
         1 0 0 rg 0 {half_h} {half_w} {half_h} re f \
         1 1 0 rg {half_w} {half_h} {half_w} {half_h} re f"
    );
    stream.into_bytes()
}

fn write_canonical_pdf(path: &Path, rotate: Rotate) {
    let content = four_quadrant_content_stream();
    write_a4_pdf(path, Some(content), rotate);
}

fn write_solid_white_pdf(path: &Path) {
    write_a4_pdf(path, None, Rotate::None);
}

fn write_a4_pdf(path: &Path, content_stream: Option<Vec<u8>>, rotate: Rotate) {
    let mut doc = Document::with_version("1.7");

    // The pages tree node — its id is reserved up front so the page
    // dict can reference its parent.
    let pages_id = doc.new_object_id();

    let mut page_dict = dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(A4_W_PT),
            Object::Real(A4_H_PT),
        ],
        "Resources" => dictionary! {},
    };
    if let Some(bytes) = content_stream {
        let stream = Stream::new(dictionary! {}, bytes);
        let content_id = doc.add_object(stream);
        page_dict.set("Contents", content_id);
    }
    if rotate.degrees() != 0 {
        page_dict.set("Rotate", Object::Integer(rotate.degrees()));
    }
    let page_id = doc.add_object(page_dict);

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![Object::Reference(page_id)],
        "Count" => 1,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    doc.compress();
    doc.save(path)
        .unwrap_or_else(|e| panic!("save {}: {e}", path.display()));

    // Sanity: the file must be reloadable and report the rotation we
    // wrote. Cheap drift detection at generation time.
    let reloaded = Document::load(path)
        .unwrap_or_else(|e| panic!("reload {} for self-check: {e}", path.display()));
    let pages_map = reloaded.get_pages();
    let &first_page_id = pages_map
        .get(&1)
        .unwrap_or_else(|| panic!("{} missing page 1", path.display()));
    let actual_rotate = page_rotate(&reloaded, first_page_id);
    assert_eq!(
        actual_rotate.rem_euclid(360),
        rotate.degrees().rem_euclid(360),
        "{}: round-trip /Rotate diverged",
        path.display()
    );
}

/// Walk a page's `/Parent` chain to resolve the inherited `/Rotate`,
/// returning `0` when none is found. Equivalent to libviprs's private
/// `pdf::resolve_rotate` but inlined here so the fixture generator
/// has no dependency on libviprs's internals.
fn page_rotate(doc: &Document, page_id: ObjectId) -> i64 {
    let mut current = page_id;
    for _ in 0..64 {
        let Ok(obj) = doc.get_object(current) else {
            return 0;
        };
        let Ok(dict) = obj.as_dict() else {
            return 0;
        };
        if let Ok(rotate_obj) = dict.get(b"Rotate") {
            if let Ok(Object::Integer(v)) = resolve(doc, rotate_obj) {
                return *v;
            }
            if let Ok(Object::Real(v)) = resolve(doc, rotate_obj) {
                return *v as i64;
            }
        }
        let parent_id = match dict.get(b"Parent") {
            Ok(Object::Reference(id)) => *id,
            _ => return 0,
        };
        if parent_id == current {
            return 0;
        }
        current = parent_id;
    }
    0
}

fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Result<&'a Object, lopdf::Error> {
    match obj {
        Object::Reference(id) => doc.get_object(*id),
        other => Ok(other),
    }
}

// ---------------------------------------------------------------------------
// CMYK image fixtures — drive the libviprs CMYK → RGB extraction path
// (`extract_page_image`) tested in `tests/pdf_cmyk.rs`. Each file is a
// single-page PDF containing one DeviceCMYK FlateDecode image XObject in
// `/Resources/XObject/Im0`; there is no `/Contents` stream because
// `extract_page_image` pulls the embedded image directly rather than
// rendering the page.
// ---------------------------------------------------------------------------

/// Reproduce the gradient pattern that `pdf_cmyk.rs` historically built
/// inline, so the migrated test sees the same bytes coming out the other
/// end of the CMYK → RGB conversion: C ramps with x, M ramps with y, Y
/// ramps with x+y, K stays zero.
fn cmyk_gradient_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let c = ((x * 255) / width.max(1)) as u8;
            let m = ((y * 255) / height.max(1)) as u8;
            let y_val = (((x + y) * 127) / (width + height).max(1)) as u8;
            out.extend_from_slice(&[c, m, y_val, 0]);
        }
    }
    out
}

fn write_cmyk_pdf(path: &Path, width: u32, height: u32, cmyk_bytes: Vec<u8>) {
    assert_eq!(
        cmyk_bytes.len(),
        width as usize * height as usize * 4,
        "{}: expected {} CMYK bytes for {}x{}, got {}",
        path.display(),
        width as usize * height as usize * 4,
        width,
        height,
        cmyk_bytes.len(),
    );

    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();

    // Pre-compress with flate2 and set `/Filter /FlateDecode` on the
    // stream dict ourselves rather than relying on `Document::compress()`
    // — that whole-doc pass skips small streams (the 1×1 fixtures end up
    // uncompressed otherwise), and libviprs's CMYK extraction path keys
    // on the FlateDecode filter being present.
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&cmyk_bytes).expect("zlib write");
    let compressed = encoder.finish().expect("zlib finish");

    let image_stream = Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => width as i64,
            "Height" => height as i64,
            "ColorSpace" => "DeviceCMYK",
            "BitsPerComponent" => 8,
            "Filter" => "FlateDecode",
        },
        compressed,
    );
    let image_id = doc.add_object(image_stream);

    // Page MediaBox matches the image dims so pdfium / libviprs see a 1:1
    // page-to-image mapping (the existing pdf_cmyk.rs test relied on this
    // — it asserted raster.width()/height() equal the image dims).
    let page_dict = dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(width as f32),
            Object::Real(height as f32),
        ],
        "Resources" => dictionary! {
            "XObject" => dictionary! {
                "Im0" => Object::Reference(image_id),
            },
        },
    };
    let page_id = doc.add_object(page_dict);

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![Object::Reference(page_id)],
        "Count" => 1,
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    doc.compress();
    doc.save(path)
        .unwrap_or_else(|e| panic!("save {}: {e}", path.display()));

    // Sanity: reload and confirm an image XObject of the right dims is
    // actually present. Cheap drift detection — if lopdf changes how
    // image XObjects round-trip, this fails at generation time rather
    // than confusing pdf_cmyk.rs with a malformed fixture.
    let reloaded = Document::load(path)
        .unwrap_or_else(|e| panic!("reload {} for self-check: {e}", path.display()));
    let (got_w, got_h) = first_image_xobject_dims(&reloaded).unwrap_or_else(|| {
        panic!(
            "{}: no image XObject found after round-trip",
            path.display()
        )
    });
    assert_eq!(
        (got_w, got_h),
        (width as i64, height as i64),
        "{}: round-trip image dims diverged",
        path.display()
    );
}

/// Find the first image XObject in `doc` and return its (width, height).
/// Walks every object looking for a Stream whose dict has /Subtype /Image.
fn first_image_xobject_dims(doc: &Document) -> Option<(i64, i64)> {
    for obj in doc.objects.values() {
        let Object::Stream(stream) = obj else {
            continue;
        };
        let dict = &stream.dict;
        let Ok(subtype) = dict.get(b"Subtype") else {
            continue;
        };
        let Object::Name(name) = subtype else {
            continue;
        };
        if name != b"Image" {
            continue;
        }
        let w = dict.get(b"Width").ok().and_then(|o| match o {
            Object::Integer(v) => Some(*v),
            _ => None,
        })?;
        let h = dict.get(b"Height").ok().and_then(|o| match o {
            Object::Integer(v) => Some(*v),
            _ => None,
        })?;
        return Some((w, h));
    }
    None
}
