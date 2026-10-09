//! Blank, lopdf-built PDFs for the page-size tests (libviprs#1199).
//!
//! Every page carries a 4 pt black frame hugging the edge of its visible box,
//! so a test can tell whether the raster covers the whole page (ink on the
//! first and last row and column) or came out a pixel short or padded.
//!
//! Also holds the size table and the libvips size formula the tests compare
//! against. The formula lives here, written out longhand, on purpose: it must
//! not call `PageSizing`, or the parity tests would only prove the core agrees
//! with itself.

use lopdf::{Document, Object, Stream, dictionary};

/// Thickness of the frame drawn on every page, in points.
pub const FRAME_PT: f64 = 4.0;

/// The page sizes the issue measures, in points. A4 to A0 use the ISO sizes as
/// the issue lists them (non-integer, which is where the old arithmetic broke).
pub const SIZES: &[(&str, f64, f64)] = &[
    ("Letter", 612.0, 792.0),
    ("Tabloid", 792.0, 1224.0),
    ("Tabloid landscape", 1224.0, 792.0),
    ("ARCH A", 648.0, 864.0),
    ("ARCH B", 864.0, 1296.0),
    ("ARCH C", 1296.0, 1728.0),
    ("ARCH D", 1728.0, 2592.0),
    ("ARCH E", 2592.0, 3456.0),
    ("ANSI D", 1584.0, 2448.0),
    ("Legal", 612.0, 1008.0),
    ("A4", 595.276, 841.89),
    ("A3", 841.89, 1190.551),
    ("A2", 1190.551, 1683.78),
    ("A1", 1683.78, 2383.937),
    ("A0", 2383.937, 3370.394),
    ("600x800", 600.0, 800.0),
    ("700x1000", 700.0, 1000.0),
];

/// The DPIs the issue sweeps.
pub const DPIS: &[u32] = &[72, 96, 150, 300, 600];

/// The size libvips gives a page, as `foreign/pdfiumload.c` computes it:
/// `total_scale = dpi / 72.0` in double, then `rint(page_dim * total_scale)`.
///
/// `rint` is round to nearest with ties to even, which is `round_ties_even`
/// here and not `f64::round`. The page dimension is the float pdfium reports
/// (`FPDF_GetPageWidthF`), widened to double, so it goes through `f32` first;
/// the PDF text we write is parsed to `f32` by pdfium too.
pub fn libvips_dims(w_pt: f64, h_pt: f64, dpi: u32) -> (u32, u32) {
    let scale = dpi as f64 / 72.0;
    let w = (w_pt as f32 as f64 * scale).round_ties_even();
    let h = (h_pt as f32 as f64 * scale).round_ties_even();
    (w as u32, h as u32)
}

/// One blank page, described by what the tests vary.
#[derive(Clone, Debug)]
pub struct PageSpec {
    /// Visible width and height in points (before `/Rotate`).
    pub w: f64,
    pub h: f64,
    /// Lower-left corner of the visible box. Non-zero tests MediaBox origins.
    pub origin: (f64, f64),
    /// `/Rotate` on the page.
    pub rotate: Option<i64>,
    /// When set, the MediaBox is made bigger than the visible box and the
    /// visible box is carried by `/CropBox` instead.
    pub crop_box: bool,
    /// `/UserUnit`, which pdfium, libvips and pdftoppm all ignore.
    pub user_unit: Option<f64>,
}

impl PageSpec {
    pub fn new(w: f64, h: f64) -> Self {
        Self {
            w,
            h,
            origin: (0.0, 0.0),
            rotate: None,
            crop_box: false,
            user_unit: None,
        }
    }

    pub fn rotate(mut self, r: i64) -> Self {
        self.rotate = Some(r);
        self
    }

    pub fn origin(mut self, x: f64, y: f64) -> Self {
        self.origin = (x, y);
        self
    }

    pub fn crop_box(mut self) -> Self {
        self.crop_box = true;
        self
    }

    pub fn user_unit(mut self, u: f64) -> Self {
        self.user_unit = Some(u);
        self
    }
}

fn real_box(b: [f64; 4]) -> Object {
    Object::Array(b.iter().map(|v| Object::Real(*v as f32)).collect())
}

/// Serialise `spec` to PDF bytes.
pub fn build_blank_pdf(spec: &PageSpec) -> Vec<u8> {
    let (x0, y0) = spec.origin;
    let (w, h) = (spec.w, spec.h);
    let visible = [x0, y0, x0 + w, y0 + h];

    // Frame: four rectangles hugging the visible box.
    let t = FRAME_PT;
    let content = format!(
        "q 0 g {x0} {y0} {w} {t} re f {x0} {top} {w} {t} re f \
         {x0} {y0} {t} {h} re f {right} {y0} {t} {h} re f Q",
        top = y0 + h - t,
        right = x0 + w - t,
    );

    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));

    let mut page = dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => dictionary! {},
    };
    if spec.crop_box {
        // A MediaBox that is clearly larger than the page, so a renderer that
        // reads it instead of the CropBox gets the wrong size.
        page.set(
            "MediaBox",
            real_box([x0 - 20.0, y0 - 30.0, x0 + w + 40.0, y0 + h + 10.0]),
        );
        page.set("CropBox", real_box(visible));
    } else {
        page.set("MediaBox", real_box(visible));
    }
    if let Some(r) = spec.rotate {
        page.set("Rotate", Object::Integer(r));
    }
    if let Some(u) = spec.user_unit {
        page.set("UserUnit", Object::Real(u as f32));
    }
    let page_id = doc.add_object(page);
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1i64,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).expect("serialise blank pdf");
    out
}

/// Write `spec` into `dir` under `name` and return the path.
pub fn write_blank_pdf(dir: &std::path::Path, name: &str, spec: &PageSpec) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, build_blank_pdf(spec)).expect("write blank pdf");
    path
}

/// File-name-safe slug of a size label.
pub fn slug(label: &str) -> String {
    label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
