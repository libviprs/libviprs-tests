//! Drawing-heavy, lopdf-built PDFs for the page-size tests (libviprs#1199).
//!
//! The name is a leftover: nothing here is blank any more. By default every
//! page gets a deterministic sheet scaled to its box (see [`Content::Rich`]):
//! a 2 pt border flush with the box edge, corner ticks, a light grid with
//! heavier major lines, dozens of vector shapes, grey and coloured fills,
//! hatching, dashed lines, arrows and dimension lines, base-14 text from 6 to
//! 48 pt, and a title block with ruled cells bottom-right.
//!
//! Two landmarks are asymmetric on purpose, so a flip, a rotation or a shift
//! shows up as the wrong pixel being dark: a solid triangle in the top-left
//! corner only, and a solid square in the title block's bottom-right cell
//! (the sheet number sits in that block too and nowhere else). [`landmarks`]
//! says where both are, in page points from the lower-left of the visible box.
//!
//! Also holds the size table and the libvips size formula the tests compare
//! against. The formula lives here, written out longhand, on purpose: it must
//! not call `PageSizing`, or the parity tests would only prove the core agrees
//! with itself.

use std::fmt::Write as _;

use lopdf::{Document, Object, Stream, dictionary};

/// Thickness of the border drawn on every page, flush with the box edge.
pub const FRAME_PT: f64 = 2.0;

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
    ("Letter landscape", 792.0, 612.0),
    ("Legal landscape", 1008.0, 612.0),
    ("ARCH E1", 2160.0, 3024.0),
    ("ARCH D landscape", 2592.0, 1728.0),
    ("ANSI C", 1224.0, 1584.0),
    ("ANSI C landscape", 1584.0, 1224.0),
    ("ANSI E", 2448.0, 3168.0),
    ("B4", 708.661, 1000.630),
    ("B3", 1000.630, 1417.323),
    ("A4", 595.276, 841.89),
    ("A3", 841.89, 1190.551),
    ("A3 landscape", 1190.551, 841.89),
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
    /// What is drawn on the page.
    pub content: Content,
}

/// What a generated page carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Content {
    /// The full drawing described in the module docs.
    #[default]
    Rich,
    /// Only the border, for the odd test that wants the cheapest page.
    BorderOnly,
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
            content: Content::Rich,
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

    pub fn content(mut self, c: Content) -> Self {
        self.content = c;
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

    let mut content = String::new();
    if spec.crop_box {
        // Black everywhere in the MediaBox, then the visible box wiped white:
        // a renderer that reads the MediaBox instead of the CropBox shows it.
        let (mx, my, mw, mh) = (x0 - 20.0, y0 - 30.0, w + 60.0, h + 40.0);
        let _ = write!(
            content,
            "q 0 g {mx} {my} {mw} {mh} re f 1 g {x0} {y0} {w} {h} re f Q "
        );
    }
    let _ = write!(content, "q 1 0 0 1 {x0} {y0} cm ");
    match spec.content {
        Content::Rich => rich_sheet(&mut content, w, h),
        Content::BorderOnly => border(&mut content, w, h),
    }
    content.push_str("Q");

    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut stream = Stream::new(dictionary! {}, content.into_bytes());
    let _ = stream.compress();
    let content_id = doc.add_object(stream);
    let font =
        |name: &str| dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => name };
    let resources = dictionary! {
        "Font" => dictionary! {
            "F1" => font("Helvetica"),
            "F2" => font("Helvetica-Bold"),
            "F3" => font("Times-Roman"),
            "F4" => font("Courier"),
        },
    };

    let mut page = dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
        "Resources" => resources,
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

/// Where the committed PDFs live.
pub fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/page_size")
}

/// File name for `spec`, derived from everything that changes its bytes.
pub fn fixture_name(spec: &PageSpec) -> String {
    format!(
        "{}x{}_o{}_{}_r{}_c{}_u{}_{:?}.pdf",
        spec.w,
        spec.h,
        spec.origin.0,
        spec.origin.1,
        spec.rotate.map_or("-".to_string(), |r| r.to_string()),
        u8::from(spec.crop_box),
        spec.user_unit.map_or("-".to_string(), |u| u.to_string()),
        spec.content,
    )
}

/// Path of the committed PDF for `spec`. The PDFs are generated once and live
/// in `tests/fixtures/page_size/`; a test run only reads them. Missing means
/// the spec is new: run the suite once with `REGEN_PAGE_SIZE_FIXTURES=1` to
/// write it (that also rewrites the rest, byte for byte, since the generator
/// is deterministic), then commit the file.
pub fn fixture_pdf(spec: &PageSpec) -> std::path::PathBuf {
    let path = fixture_dir().join(fixture_name(spec));
    if std::env::var_os("REGEN_PAGE_SIZE_FIXTURES").is_some() {
        std::fs::create_dir_all(fixture_dir()).expect("create fixture dir");
        let bytes = build_blank_pdf(spec);
        if std::fs::read(&path).ok().as_deref() != Some(&bytes) {
            std::fs::write(&path, bytes).expect("write fixture");
        }
    } else {
        assert!(
            path.exists(),
            "{} is missing: run once with REGEN_PAGE_SIZE_FIXTURES=1 and commit it",
            path.display()
        );
    }
    path
}

/// File-name-safe slug of a size label.
pub fn slug(label: &str) -> String {
    label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

// ---------------------------------------------------------------------------
// The drawing
// ---------------------------------------------------------------------------

/// Where the two asymmetric landmarks are, in points from the lower-left corner
/// of the visible box (before `/Rotate`).
#[derive(Clone, Copy, Debug)]
pub struct Landmarks {
    /// Leg length of the solid triangle in the top-left corner. Its vertices
    /// are `(0, h)`, `(tri, h)` and `(0, h - tri)`.
    pub tri: f64,
    /// Centre of the solid square in the title block's bottom-right cell.
    pub square_centre: (f64, f64),
    /// Side of that square.
    pub square: f64,
}

/// Margin between the page edge and the title block.
const BLOCK_MARGIN: f64 = 14.0;

pub fn landmarks(w: f64, h: f64) -> Landmarks {
    let short = w.min(h);
    let tri = (0.1 * short).max(30.0).min(short / 3.0);
    let (bw, bh) = (0.32 * w, 0.14 * h);
    let (bx, by) = (w - BLOCK_MARGIN - bw, BLOCK_MARGIN);
    Landmarks {
        tri,
        square_centre: (bx + 0.75 * bw, by + bh / 6.0),
        square: 0.5 * (bw / 2.0).min(bh / 3.0),
    }
}

fn border(c: &mut String, w: f64, h: f64) {
    // Stroked inside the box so its outer edge is the box edge: 2 pt of ink on
    // the first and last row and column, none lost to the clip.
    let _ = write!(
        c,
        "q 0 G {FRAME_PT} w {} {} {} {} re S Q ",
        FRAME_PT / 2.0,
        FRAME_PT / 2.0,
        w - FRAME_PT,
        h - FRAME_PT
    );
}

fn text(c: &mut String, font: u8, size: f64, x: f64, y: f64, s: &str) {
    let _ = write!(c, "BT /F{font} {size:.2} Tf {x:.2} {y:.2} Td ({s}) Tj ET ");
}

fn ellipse(c: &mut String, cx: f64, cy: f64, rx: f64, ry: f64, op: &str) {
    const K: f64 = 0.552_284_749_8;
    let _ = write!(
        c,
        "{:.2} {:.2} m {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c \
         {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {op} ",
        cx + rx,
        cy,
        cx + rx,
        cy + K * ry,
        cx + K * rx,
        cy + ry,
        cx,
        cy + ry,
        cx - K * rx,
        cy + ry,
        cx - rx,
        cy + K * ry,
        cx - rx,
        cy,
        cx - rx,
        cy - K * ry,
        cx - K * rx,
        cy - ry,
        cx,
        cy - ry,
        cx + K * rx,
        cy - ry,
        cx + rx,
        cy - K * ry,
        cx + rx,
        cy,
    );
}

/// Filled arrowhead at `(x, y)` pointing along `angle` (radians).
fn arrowhead(c: &mut String, x: f64, y: f64, angle: f64, len: f64) {
    let (s, co) = angle.sin_cos();
    let (bx, by) = (x - len * co, y - len * s);
    let (nx, ny) = (-s * len * 0.35, co * len * 0.35);
    let _ = write!(
        c,
        "{x:.2} {y:.2} m {:.2} {:.2} l {:.2} {:.2} l f ",
        bx + nx,
        by + ny,
        bx - nx,
        by - ny
    );
}

/// A dimension line from `(x1, y1)` to `(x2, y2)` with arrowheads and a label.
fn dimension(c: &mut String, p1: (f64, f64), p2: (f64, f64), label: &str, size: f64) {
    let _ = write!(
        c,
        "q 0 g 0 G 0.6 w {:.2} {:.2} m {:.2} {:.2} l S ",
        p1.0, p1.1, p2.0, p2.1
    );
    let ang = (p2.1 - p1.1).atan2(p2.0 - p1.0);
    let head = size * 1.2;
    arrowhead(c, p2.0, p2.1, ang, head);
    arrowhead(c, p1.0, p1.1, ang + std::f64::consts::PI, head);
    let (mx, my) = ((p1.0 + p2.0) / 2.0, (p1.1 + p2.1) / 2.0);
    text(c, 4, size, mx + size * 0.4, my + size * 0.4, label);
    c.push_str("Q ");
}

fn rich_sheet(c: &mut String, w: f64, h: f64) {
    let short = w.min(h);
    let size = |frac: f64, lo: f64, hi: f64| (frac * short).clamp(lo, hi);
    let lm = landmarks(w, h);
    let (bw, bh) = (0.32 * w, 0.14 * h);
    let (bx, by) = (w - BLOCK_MARGIN - bw, BLOCK_MARGIN);

    // Light grid every half inch or 5% of the short side, whichever is larger,
    // heavier major lines every fifth.
    let step = (0.05 * short).max(36.0);
    let mut minor = String::new();
    let mut major = String::new();
    let (mut i, mut x) = (0u32, 0.0);
    while x <= w {
        let dst = if i % 5 == 0 { &mut major } else { &mut minor };
        let _ = write!(dst, "{x:.2} 0 m {x:.2} {h:.2} l ");
        i += 1;
        x = i as f64 * step;
    }
    let (mut j, mut y) = (0u32, 0.0);
    while y <= h {
        let dst = if j % 5 == 0 { &mut major } else { &mut minor };
        let _ = write!(dst, "0 {y:.2} m {w:.2} {y:.2} l ");
        j += 1;
        y = j as f64 * step;
    }
    let _ = write!(c, "q 0.25 w 0.88 G {minor} S 0.6 w 0.55 G {major} S Q ");

    // Fills: a grey region, then colour.
    let _ = write!(
        c,
        "q 0.8 g {:.2} {:.2} {:.2} {:.2} re f ",
        0.28 * w,
        0.62 * h,
        0.2 * w,
        0.16 * h
    );
    let _ = write!(
        c,
        "0.85 0.1 0.1 rg {:.2} {:.2} {:.2} {:.2} re f ",
        0.52 * w,
        0.62 * h,
        0.14 * w,
        0.12 * h
    );
    let _ = write!(
        c,
        "0.1 0.6 0.2 rg {:.2} {:.2} {:.2} {:.2} re f ",
        0.28 * w,
        0.40 * h,
        0.12 * w,
        0.14 * h
    );
    c.push_str("0.15 0.3 0.85 rg ");
    ellipse(c, 0.62 * w, 0.48 * h, 0.08 * w, 0.07 * h, "f");
    c.push_str("0.95 0.8 0.1 rg ");
    ellipse(c, 0.42 * w, 0.30 * h, 0.05 * short, 0.05 * short, "f");
    let _ = write!(
        c,
        "0.95 0.5 0.1 rg {:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l f Q ",
        0.7 * w,
        0.30 * h,
        0.78 * w,
        0.34 * h,
        0.74 * w,
        0.42 * h,
        0.68 * w,
        0.38 * h
    );

    // Diagonal hatching clipped to a box.
    let (hx, hy, hw, hh) = (0.46 * w, 0.36 * h, 0.14 * w, 0.12 * h);
    let _ = write!(c, "q {hx:.2} {hy:.2} {hw:.2} {hh:.2} re W n 0.3 w 0 G ");
    let hs = (0.012 * short).max(4.0);
    let mut d = -hh;
    while d < hw {
        let _ = write!(
            c,
            "{:.2} {:.2} m {:.2} {:.2} l ",
            hx + d,
            hy,
            hx + d + hh,
            hy + hh
        );
        d += hs;
    }
    let _ = write!(c, "S Q q 0.4 w 0 G {hx:.2} {hy:.2} {hw:.2} {hh:.2} re S Q ");

    // Thirty small shapes from a fixed-seed generator, in the middle band.
    let mut seed: u64 = 0x5EED_1199;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 33) as f64) / ((1u64 << 31) as f64)
    };
    for n in 0..30 {
        let (sx, sy) = (0.22 * w + next() * 0.56 * w, 0.24 * h + next() * 0.34 * h);
        let r = (0.008 + next() * 0.02) * short;
        let (cr, cg, cb) = (next(), next(), next());
        let _ = write!(c, "q {cr:.2} {cg:.2} {cb:.2} rg 0 G 0.4 w ");
        match n % 5 {
            0 => {
                let _ = write!(c, "{:.2} {:.2} {:.2} {:.2} re B ", sx, sy, 2.0 * r, r);
            }
            1 => ellipse(c, sx, sy, r, r * 0.7, "B"),
            2 => {
                let _ = write!(
                    c,
                    "{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l S ",
                    sx,
                    sy,
                    sx + r,
                    sy + r,
                    sx + 2.0 * r,
                    sy - r * 0.5,
                    sx + 3.0 * r,
                    sy + r
                );
            }
            3 => {
                let _ = write!(
                    c,
                    "[{:.1} {:.1}] 0 d {:.2} {:.2} m {:.2} {:.2} l S ",
                    r * 0.4,
                    r * 0.3,
                    sx,
                    sy,
                    sx + 4.0 * r,
                    sy + r
                );
            }
            _ => {
                let _ = write!(
                    c,
                    "{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l b ",
                    sx,
                    sy,
                    sx + 2.0 * r,
                    sy,
                    sx + r,
                    sy + 1.6 * r
                );
            }
        }
        c.push_str("Q ");
    }

    // Dashed lines and a polyline.
    let _ = write!(
        c,
        "q 0.8 w 0.2 0.2 0.2 RG [8 4] 0 d {:.2} {:.2} m {:.2} {:.2} l S [2 3] 0 d {:.2} {:.2} m {:.2} {:.2} l S Q ",
        0.24 * w,
        0.58 * h,
        0.76 * w,
        0.58 * h,
        0.24 * w,
        0.26 * h,
        0.76 * w,
        0.26 * h
    );
    let _ = write!(
        c,
        "q 1.2 w 0.5 0 0.5 RG {:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l {:.2} {:.2} l S Q ",
        0.22 * w,
        0.82 * h,
        0.30 * w,
        0.86 * h,
        0.38 * w,
        0.80 * h,
        0.46 * w,
        0.86 * h,
        0.54 * w,
        0.82 * h
    );

    // Arrow and north arrow.
    let a = 0.06 * short;
    let _ = write!(
        c,
        "q 0 g 0 G 1 w {:.2} {:.2} m {:.2} {:.2} l S ",
        0.56 * w,
        0.20 * h,
        0.56 * w + a * 2.0,
        0.20 * h
    );
    arrowhead(c, 0.56 * w + a * 2.0, 0.20 * h, 0.0, a * 0.5);
    let (nx, ny) = (0.82 * w, 0.68 * h);
    ellipse(c, nx, ny, a * 0.6, a * 0.6, "S");
    let _ = write!(
        c,
        "{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l f Q ",
        nx,
        ny + a * 0.9,
        nx - a * 0.25,
        ny - a * 0.3,
        nx + a * 0.25,
        ny - a * 0.3
    );
    text(
        c,
        2,
        size(0.02, 8.0, 24.0),
        nx - a * 0.12,
        ny + a * 1.0,
        "N",
    );

    // Dimension lines.
    let dim = size(0.014, 6.0, 14.0);
    dimension(
        c,
        (0.28 * w, 0.20 * h + 20.0),
        (0.52 * w, 0.20 * h + 20.0),
        "12'-6\"",
        dim,
    );
    dimension(
        c,
        (0.90 * w, 0.30 * h),
        (0.90 * w, 0.62 * h),
        "20'-0\"",
        dim,
    );

    // Text: title, lorem lines, notes, legend.
    text(
        c,
        2,
        size(0.06, 10.0, 48.0),
        0.12 * w,
        0.92 * h,
        "SITE PLAN",
    );
    text(
        c,
        3,
        size(0.025, 8.0, 24.0),
        0.12 * w,
        0.88 * h,
        "LOREM IPSUM DOLOR SIT AMET",
    );
    text(
        c,
        1,
        size(0.012, 6.0, 12.0),
        0.12 * w,
        0.855 * h,
        "CONSECTETUR ADIPISCING ELIT SED DO EIUSMOD",
    );
    let ns = size(0.012, 6.0, 12.0);
    text(c, 2, ns * 1.2, 0.04 * w, 0.78 * h, "NOTES");
    for (k, line) in [
        "1. LOREM IPSUM DOLOR",
        "2. SIT AMET CONSECTETUR",
        "3. SED DO EIUSMOD TEMPOR",
        "4. INCIDIDUNT UT LABORE",
        "5. ET DOLORE MAGNA ALIQUA",
    ]
    .iter()
    .enumerate()
    {
        text(c, 4, ns, 0.04 * w, 0.75 * h - k as f64 * ns * 1.5, line);
    }
    let lx = 0.72 * w;
    for (k, (name, rgb)) in [
        ("WALL", "0 0 0"),
        ("DOOR", "0.85 0.1 0.1"),
        ("WINDOW", "0.15 0.3 0.85"),
        ("FIXTURE", "0.1 0.6 0.2"),
    ]
    .iter()
    .enumerate()
    {
        let ly = 0.89 * h - k as f64 * ns * 1.6;
        let _ = write!(
            c,
            "q {rgb} rg {lx:.2} {ly:.2} {:.2} {:.2} re f Q ",
            ns * 1.6,
            ns
        );
        text(c, 4, ns, lx + ns * 2.2, ly + ns * 0.15, name);
    }

    // Title block: a ruled table, two columns by three rows.
    let (cw, ch) = (bw / 2.0, bh / 3.0);
    let _ = write!(c, "q 0 G 0.8 w {bx:.2} {by:.2} {bw:.2} {bh:.2} re S 0.4 w ");
    let _ = write!(
        c,
        "{:.2} {by:.2} m {:.2} {:.2} l ",
        bx + cw,
        bx + cw,
        by + bh
    );
    for r in 1..3 {
        let _ = write!(
            c,
            "{bx:.2} {:.2} m {:.2} {:.2} l ",
            by + r as f64 * ch,
            bx + bw,
            by + r as f64 * ch
        );
    }
    c.push_str("S Q ");
    let ts = (ch * 0.3).clamp(5.0, 14.0);
    text(
        c,
        1,
        ts,
        bx + 4.0,
        by + 2.0 * ch + ch * 0.35,
        "PLACEHOLDER SITE",
    );
    text(c, 1, ts, bx + cw + 4.0, by + 2.0 * ch + ch * 0.35, "REV A");
    text(c, 3, ts, bx + 4.0, by + ch + ch * 0.35, "DRAWN LOREM");
    text(c, 2, ts * 1.4, bx + cw + 4.0, by + ch + ch * 0.3, "A-101");
    text(c, 4, ts, bx + 4.0, by + ch * 0.35, "SCALE 1:100");
    let (sx, sy) = lm.square_centre;
    let _ = write!(
        c,
        "q 0 g {:.2} {:.2} {:.2} {:.2} re f Q ",
        sx - lm.square / 2.0,
        sy - lm.square / 2.0,
        lm.square,
        lm.square
    );

    // Top-left landmark triangle.
    let _ = write!(
        c,
        "q 0 g 0 {h:.2} m {:.2} {h:.2} l 0 {:.2} l f Q ",
        lm.tri,
        h - lm.tri
    );

    // Corner ticks, 4 pt in from each edge and 12 pt long.
    let mut t = String::new();
    for (cx, dx) in [(0.0, 1.0), (w, -1.0)] {
        for (cy, dy) in [(0.0, 1.0), (h, -1.0)] {
            let _ = write!(
                t,
                "{cx:.2} {:.2} m {:.2} {:.2} l {:.2} {cy:.2} m {:.2} {:.2} l ",
                cy + dy * 4.0,
                cx + dx * 12.0,
                cy + dy * 4.0,
                cx + dx * 4.0,
                cx + dx * 4.0,
                cy + dy * 12.0
            );
        }
    }
    let _ = write!(c, "q 0.5 w 0 G {t} S Q ");

    // Border last, over everything.
    border(c, w, h);
}
