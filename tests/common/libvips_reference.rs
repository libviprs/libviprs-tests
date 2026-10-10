//! The committed libvips reference for the page-size tests (libviprs#1199).
//!
//! libvips is not run by a normal test run. It ran once, through the
//! pdfium-backed build, and left two things in `tests/fixtures/page_size/libvips/`:
//!
//! - `libvips_reference.json`: for every (fixture PDF, dpi) in [`matrix`], the
//!   size and band count `pdfload[dpi=N]` reports, plus the PDF's sha256, so a
//!   regenerated fixture that no longer matches is caught. Where the entry also
//!   has a raster, the file name and the sha256 of the stored PNG and of the
//!   raw RGB it holds.
//! - the rasters themselves: `pdfload` flattened onto white, 8-bit RGB, as PNG
//!   written by the `image` crate at `CompressionType::Best` with adaptive
//!   filtering. PNG because the test crate already decodes it and it is
//!   lossless; the encoder writes no timestamp or other metadata, so
//!   regenerating gives the same bytes.
//!
//! Regenerate (needs `vips` and `vipsheader` with a PDF loader on PATH, for
//! example the image built from `tools/Dockerfile.libvips-pdfium`):
//!
//! ```text
//! REGEN_PAGE_SIZE_LIBVIPS=1 VIPRS_REFERENCE_PDFIUM=pdfium-8085 \
//!   cargo test --features pdfium --test page_size_libvips_pixel_parity \
//!   -- --ignored --exact regen_libvips_reference
//! ```
//!
//! Check that the stored references still match what that libvips makes:
//! `VIPRS_VERIFY_LIBVIPS_REFERENCE=1` on `page_size_libvips_pixel_parity` and
//! `page_size_libvips_pdfium_parity`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::blank_pdf::{
    DPIS, PageSpec, SIZES, fixture_dir, fixture_name, fixture_pdf, libvips_dims,
};
use super::vips_oracle;

pub const REGEN_COMMAND: &str = "REGEN_PAGE_SIZE_LIBVIPS=1 VIPRS_REFERENCE_PDFIUM=pdfium-8085 cargo test \
--features pdfium --test page_size_libvips_pixel_parity -- --ignored --exact regen_libvips_reference";

pub fn reference_dir() -> PathBuf {
    fixture_dir().join("libvips")
}

pub fn manifest_path() -> PathBuf {
    reference_dir().join("libvips_reference.json")
}

/// One (fixture page, dpi) the reference covers.
#[derive(Clone)]
pub struct Entry {
    pub label: String,
    pub spec: PageSpec,
    pub dpi: u32,
    /// Whether a raster is committed, not just the size.
    pub raster: bool,
    /// Whether the raster is too big to commit: the test suite makes it with
    /// libvips the first time it is needed, into the git-ignored `generated/`
    /// folder, and checks it against the recorded hash of the decoded pixels.
    pub on_demand: bool,
}

impl Entry {
    pub fn pdf_name(&self) -> String {
        fixture_name(&self.spec)
    }
}

/// Mid-size sheets keep a 150 dpi raster; the rest stop at 72.
const MAX_PX_AT_150: u64 = 20_000_000;
/// The only sheets with a 300 dpi raster. The larger sheets at 300 dpi are
/// covered by the size entries alone, which keeps the references small.
const RASTER_AT_300: &[&str] = &["Letter", "A4", "Tabloid"];

/// Every (page, dpi) in the reference. Sizes for the whole table at every DPI
/// in the sweep and for rotated pages; rasters for the deliberate subset.
pub fn matrix() -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    for &(label, w, h) in SIZES {
        for &dpi in DPIS {
            let px = libvips_dims(w, h, 150);
            let raster = dpi == 72
                || (dpi == 150 && (px.0 as u64) * (px.1 as u64) <= MAX_PX_AT_150)
                || (dpi == 300 && RASTER_AT_300.contains(&label));
            out.push(Entry {
                label: label.into(),
                spec: PageSpec::new(w, h),
                dpi,
                raster,
                on_demand: false,
            });
        }
    }
    for (label, w, h) in [
        ("Letter", 612.0, 792.0),
        ("A3", 841.89, 1190.551),
        ("A4", 595.276, 841.89),
    ] {
        for rotate in [90, 270] {
            for &dpi in DPIS {
                let raster = label == "A3" && rotate == 90 && dpi <= 150;
                out.push(Entry {
                    label: format!("{label} /Rotate {rotate}"),
                    spec: PageSpec::new(w, h).rotate(rotate),
                    dpi,
                    raster,
                    on_demand: false,
                });
            }
        }
    }
    for &dpi in DPIS {
        out.push(Entry {
            label: "A3 CropBox".into(),
            spec: PageSpec::new(841.89, 1190.551).crop_box(),
            dpi,
            raster: dpi <= 150,
            on_demand: false,
        });
    }
    // Too big to commit: every sheet bigger than Tabloid at 300 dpi, and the
    // 150 dpi rasters over the committed size cap. Rotated and CropBox A3 get
    // their 300 dpi raster the same way.
    let tabloid_area = 792.0 * 1224.0;
    for e in &mut out {
        let area = e.spec.w * e.spec.h;
        let big_sheet_300 = e.dpi == 300 && area > tabloid_area * 1.0001;
        let big_150 = e.dpi == 150 && !e.raster && e.spec.rotate.is_none() && !e.spec.crop_box;
        e.on_demand = !e.raster && (big_sheet_300 || big_150);
    }
    out.sort_by_key(|e| (e.pdf_name(), e.dpi));
    out.dedup_by(|b, a| {
        let same = a.pdf_name() == b.pdf_name() && a.dpi == b.dpi;
        if same {
            a.raster |= b.raster;
            a.on_demand = (a.on_demand || b.on_demand) && !a.raster;
        }
        same
    });
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Where an on-demand raster lives once generated.
pub fn generated_path(e: &Entry) -> PathBuf {
    reference_dir().join("generated").join(raster_file_name(e))
}

pub fn raster_file_name(e: &Entry) -> String {
    format!("{}@{}dpi.png", e.pdf_name().trim_end_matches(".pdf"), e.dpi)
}

/// The committed manifest.
pub struct Manifest {
    json: Value,
}

pub fn load() -> Manifest {
    let path = manifest_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{} is missing ({e}). Regenerate with:\n  {REGEN_COMMAND}",
            path.display()
        )
    });
    Manifest {
        json: serde_json::from_str(&text).expect("libvips_reference.json is not JSON"),
    }
}

/// What the manifest records for one (page, dpi).
pub struct Recorded {
    pub width: u32,
    pub height: u32,
    pub bands: u32,
    /// Decoded RGB bytes of the stored raster, when the entry has one.
    pub raster: Option<Vec<u8>>,
}

impl Manifest {
    pub fn libvips_version(&self) -> &str {
        self.json["libvips_version"].as_str().unwrap_or("?")
    }

    pub fn pdfium(&self) -> &str {
        self.json["pdfium"].as_str().unwrap_or("?")
    }

    fn find(&self, e: &Entry) -> &Value {
        let name = e.pdf_name();
        self.json["entries"]
            .as_array()
            .and_then(|a| a.iter().find(|v| v["pdf"] == name.as_str() && v["dpi"] == e.dpi))
            .unwrap_or_else(|| {
                panic!("no manifest entry for {name} at {} dpi ({}). Regenerate with:\n  {REGEN_COMMAND}", e.dpi, e.label)
            })
    }

    /// The recorded size and decoded-pixel hash of an on-demand entry.
    pub fn on_demand(&self, e: &Entry) -> (Recorded, String) {
        let v = self.find(e);
        let pdf = std::fs::read(fixture_pdf(&e.spec)).expect("read fixture pdf");
        assert_eq!(
            v["pdf_sha256"].as_str().unwrap(),
            sha256_hex(&pdf),
            "{}: the fixture PDF changed since the libvips reference was made. Regenerate with:\n  {REGEN_COMMAND}",
            e.pdf_name()
        );
        let hash = v["raster"]["raster_sha256"].as_str().unwrap_or_else(|| {
            panic!(
                "{} @ {}: manifest has no on-demand hash. Regenerate with:\n  {REGEN_COMMAND}",
                e.label, e.dpi
            )
        });
        let num = |k: &str| v[k].as_u64().unwrap() as u32;
        (
            Recorded {
                width: num("width"),
                height: num("height"),
                bands: num("bands"),
                raster: None,
            },
            hash.to_string(),
        )
    }

    /// Look `e` up, check the PDF and the stored raster still hash to what was
    /// recorded, and decode the raster.
    pub fn recorded(&self, e: &Entry) -> Recorded {
        let v = self.find(e);
        let pdf = std::fs::read(fixture_pdf(&e.spec)).expect("read fixture pdf");
        assert_eq!(
            v["pdf_sha256"].as_str().unwrap(),
            sha256_hex(&pdf),
            "{}: the fixture PDF changed since the libvips reference was made. Regenerate with:\n  {REGEN_COMMAND}",
            e.pdf_name()
        );
        let num = |k: &str| v[k].as_u64().unwrap() as u32;
        let raster = if e.raster {
            let r = v["raster"].as_object().unwrap_or_else(|| {
                panic!(
                    "{} @ {}: manifest has no raster. Regenerate with:\n  {REGEN_COMMAND}",
                    e.pdf_name(),
                    e.dpi
                )
            });
            let bytes = std::fs::read(reference_dir().join(r["file"].as_str().unwrap()))
                .unwrap_or_else(|err| {
                    panic!("{}: {err}. Regenerate with:\n  {REGEN_COMMAND}", e.label)
                });
            assert_eq!(
                r["file_sha256"].as_str().unwrap(),
                sha256_hex(&bytes),
                "{}: stored raster is corrupt",
                e.label
            );
            let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
                .expect("decode stored raster")
                .to_rgb8();
            assert_eq!((img.width(), img.height()), (num("width"), num("height")));
            let rgb = img.into_raw();
            assert_eq!(r["raster_sha256"].as_str().unwrap(), sha256_hex(&rgb));
            Some(rgb)
        } else {
            None
        };
        Recorded {
            width: num("width"),
            height: num("height"),
            bands: num("bands"),
            raster,
        }
    }
}

fn encode_png(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Best, FilterType::Adaptive)
        .write_image(rgb, w, h, ExtendedColorType::Rgb8)
        .expect("encode png");
    out
}

/// Run libvips over the whole matrix and write the manifest and rasters.
/// `pdfium` is the build libvips was linked against, recorded as given.
pub fn regenerate(pdfium: &str) {
    let entries = matrix();
    let probe = fixture_pdf(&entries[0].spec);
    assert!(
        !vips_oracle::skip_if_no_vips_pdf("regen_libvips_reference", probe.to_str().unwrap()),
        "regeneration needs a libvips with a PDF loader"
    );
    std::fs::create_dir_all(reference_dir()).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let mut rows = Vec::new();
    for e in &entries {
        let pdf_path = fixture_pdf(&e.spec);
        let pdf_sha = sha256_hex(&std::fs::read(&pdf_path).unwrap());
        let src = format!("{}[dpi={}]", pdf_path.display(), e.dpi);
        let field = |f: &str| {
            vips_oracle::header_field(&src, f).unwrap_or_else(|err| panic!("{}: {err}", e.label))
        };
        let (width, height, bands) = (field("width"), field("height"), field("bands"));
        let mut row = json!({
            "label": e.label, "pdf": e.pdf_name(), "pdf_sha256": pdf_sha, "dpi": e.dpi,
            "width": width, "height": height, "bands": bands,
        });
        if e.raster {
            let r = vips_oracle::render(&pdf_path, e.dpi, scratch.path(), "ref")
                .unwrap_or_else(|err| panic!("{}: {err}", e.label));
            assert_eq!((r.width, r.height, r.bands), (width, height, bands));
            let png = encode_png(&r.rgb, width, height);
            let file = raster_file_name(e);
            std::fs::write(reference_dir().join(&file), &png).unwrap();
            row["raster"] = json!({
                "file": file, "file_sha256": sha256_hex(&png), "raster_sha256": sha256_hex(&r.rgb),
            });
        }
        if e.on_demand {
            let out = generated_path(e);
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            vips_oracle::render_to_png(&pdf_path, e.dpi, &out)
                .unwrap_or_else(|err| panic!("{}: {err}", e.label));
            let (w, h, hash) = hash_png(&out);
            assert_eq!((w, h), (width, height), "{}: png size", e.label);
            row["raster"] = json!({
                "file": format!("generated/{}", raster_file_name(e)),
                "raster_sha256": hash,
                "on_demand": true,
            });
        }
        eprintln!(
            "regen {} @ {}: {width}x{height}x{bands}{}",
            e.label,
            e.dpi,
            if e.raster {
                " +raster"
            } else if e.on_demand {
                " +on-demand"
            } else {
                ""
            }
        );
        rows.push(row);
    }
    let manifest = json!({
        "on_demand_note": "entries with raster.on_demand are not committed; the tests generate them with libvips when missing and check raster_sha256 (the hash of the decoded RGB bytes)",
        "about": "libvips pdfload references for the page-size tests; see tests/common/libvips_reference.rs",
        "libvips_version": vips_oracle::version(),
        "pdfium": pdfium,
        "invocation": vips_oracle::INVOCATION,
        "entries": rows,
    });
    let mut text = serde_json::to_string_pretty(&manifest).unwrap();
    text.push('\n');
    std::fs::write(manifest_path(), text).unwrap();
}

// ---------------------------------------------------------------------------
// Streaming decode and on-demand generation
// ---------------------------------------------------------------------------

/// Rows handed to a streaming consumer at a time.
pub const CHUNK_ROWS: u32 = 64;

/// An 8-bit RGB or RGBA PNG read [`CHUNK_ROWS`] rows at a time, handed out as
/// RGB. The libvips references are RGB; the CLI writes RGBA. Alpha is dropped
/// on the way out and [`PngRows::opaque`] says whether every pixel read so far
/// had alpha 255, so a caller can still insist on an opaque raster.
pub struct PngRows {
    reader: png::Reader<std::io::BufReader<std::fs::File>>,
    pub width: u32,
    pub height: u32,
    /// Samples per pixel in the file: 3 or 4.
    pub bands: u32,
    pub opaque: bool,
    next_row: u32,
    buf: Vec<u8>,
    path: PathBuf,
}

impl PngRows {
    pub fn open(path: &Path) -> PngRows {
        let file = std::io::BufReader::new(
            std::fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
        );
        let reader = png::Decoder::new(file).read_info().expect("png header");
        let info = reader.info();
        let (width, height) = (info.width, info.height);
        let bands = match reader.output_color_type() {
            (png::ColorType::Rgb, png::BitDepth::Eight) => 3,
            (png::ColorType::Rgba, png::BitDepth::Eight) => 4,
            other => panic!("{}: not 8-bit RGB or RGBA ({other:?})", path.display()),
        };
        PngRows {
            reader,
            width,
            height,
            bands,
            opaque: true,
            next_row: 0,
            buf: Vec::with_capacity(width as usize * 3 * CHUNK_ROWS as usize),
            path: path.to_owned(),
        }
    }

    /// The next chunk as `(y0, RGB rows)`, or `None` at the end.
    pub fn next_chunk(&mut self) -> Option<(u32, &[u8])> {
        if self.next_row >= self.height {
            return None;
        }
        let y0 = self.next_row;
        self.buf.clear();
        while self.next_row < self.height && self.next_row - y0 < CHUNK_ROWS {
            let row = self
                .reader
                .next_row()
                .unwrap_or_else(|e| panic!("{}: png row: {e}", self.path.display()))
                .expect("png ended early");
            if self.bands == 3 {
                self.buf.extend_from_slice(row.data());
            } else {
                for p in row.data().chunks_exact(4) {
                    self.opaque &= p[3] == 255;
                    self.buf.extend_from_slice(&p[..3]);
                }
            }
            self.next_row += 1;
        }
        Some((y0, &self.buf))
    }
}

/// Decode the 8-bit RGB PNG at `path` row chunk by row chunk: `f(y0, rows)`
/// gets `rows` full rows starting at row `y0`. Returns the size.
pub fn stream_png(path: &Path, mut f: impl FnMut(u32, &[u8])) -> (u32, u32) {
    let mut rows = PngRows::open(path);
    assert_eq!(rows.bands, 3, "{}: not 8-bit RGB", path.display());
    while let Some((y0, chunk)) = rows.next_chunk() {
        f(y0, chunk);
    }
    (rows.width, rows.height)
}

/// Size and sha256 of the decoded RGB bytes of a PNG, without holding them.
pub fn hash_png(path: &Path) -> (u32, u32, String) {
    let mut hasher = Sha256::new();
    let (w, h) = stream_png(path, |_, rows| hasher.update(rows));
    (
        w,
        h,
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

/// What `reference_for` found.
pub enum Reference {
    /// The verified PNG.
    Ready(PathBuf),
    /// No file and no libvips to make one; the message says how to proceed.
    Skip(String),
}

/// The generated PNG for an on-demand entry. Present means reuse it (the
/// caller checks the decoded hash as it reads). Missing means make it with
/// libvips: render to a private temp file next to the target, check the
/// decoded pixels against the recorded hash, and only then rename it into
/// place, so a killed run or a parallel test never leaves or reads a partial
/// file. With no libvips it is a [`Reference::Skip`], which
/// `VIPRS_REQUIRE_VIPS=1` turns into a panic.
pub fn reference_for(e: &Entry, recorded_hash: &str) -> Reference {
    let target = generated_path(e);
    if target.exists() {
        return Reference::Ready(target);
    }
    let pdf = fixture_pdf(&e.spec);
    if let Err(reason) = vips_oracle::pdf_loader_available(pdf.to_str().unwrap()) {
        let msg = format!(
            "{} @ {} dpi: no generated reference and no libvips PDF loader ({reason}). \
             Put vips on PATH to generate it, or make all of them with:\n  {REGEN_COMMAND}",
            e.label, e.dpi
        );
        if std::env::var("VIPRS_REQUIRE_VIPS").as_deref() == Ok("1") {
            panic!("VIPRS_REQUIRE_VIPS=1 but {msg}");
        }
        return Reference::Skip(msg);
    }
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let tmp = target.with_extension(
        format!(
            "tmp-{}-{:?}.png",
            std::process::id(),
            std::thread::current().id()
        )
        .replace(['(', ')'], ""),
    );
    eprintln!(
        "generating {} with libvips ({})",
        target.display(),
        vips_oracle::version()
    );
    vips_oracle::render_to_png(&pdf, e.dpi, &tmp)
        .unwrap_or_else(|err| panic!("{}: libvips failed: {err}", e.label));
    let (_, _, hash) = hash_png(&tmp);
    if hash != recorded_hash {
        let _ = std::fs::remove_file(&tmp);
        panic!(
            "{} @ {} dpi: libvips output differs from the recorded reference (decoded sha256 {hash}, recorded {recorded_hash}; libvips {}). \
             If this libvips or pdfium is meant to differ, refresh the manifest with:\n  {REGEN_COMMAND}",
            e.label,
            e.dpi,
            vips_oracle::version()
        );
    }
    std::fs::rename(&tmp, &target).unwrap_or_else(|err| panic!("{}: {err}", target.display()));
    Reference::Ready(target)
}
