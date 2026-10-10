//! A live libvips with a PDF loader as a test oracle (libviprs#1199).
//!
//! `vipsheader` and `vips` are looked up on `$PATH`, or at `$VIPSHEADER` and
//! `$VIPS`. The image built from `tools/Dockerfile.libvips-pdfium` has both,
//! with pdfium as the PDF backend. Without them the tests print a `SKIP` line
//! and return; `VIPRS_REQUIRE_VIPS=1` turns that into a failure.

use std::path::Path;
use std::process::Command;

fn bin(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

pub fn vipsheader_bin() -> String {
    bin("VIPSHEADER", "vipsheader")
}

pub fn vips_bin() -> String {
    bin("VIPS", "vips")
}

/// `vipsheader -f <field> <file>`, parsed as a number. `Err` carries what
/// vipsheader said, so a caller can tell "no PDF loader" from a mismatch.
pub fn header_field(file: &str, field: &str) -> Result<u32, String> {
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

/// Whether libvips can load a PDF, asked once per process.
pub fn pdf_loader_available(probe_pdf: &str) -> Result<(), String> {
    static AVAILABLE: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    AVAILABLE
        .get_or_init(|| header_field(&format!("{probe_pdf}[dpi=72]"), "width").map(|_| ()))
        .clone()
}

/// Skip guard: `true` (with a printed reason) when there is no libvips with a
/// PDF loader to ask. Panics instead under `VIPRS_REQUIRE_VIPS=1`.
pub fn skip_if_no_vips_pdf(test: &str, probe_pdf: &str) -> bool {
    let reason = match pdf_loader_available(probe_pdf) {
        Ok(()) => return false,
        Err(e) => e,
    };
    if std::env::var("VIPRS_REQUIRE_VIPS").as_deref() == Ok("1") {
        panic!("{test}: VIPRS_REQUIRE_VIPS=1 but libvips cannot load a PDF: {reason}");
    }
    eprintln!(
        "SKIP {test}: no libvips with a PDF loader ({reason}). Put vipsheader and vips on PATH, \
         set $VIPSHEADER / $VIPS, or run the image built from tools/Dockerfile.libvips-pdfium."
    );
    true
}

/// Render `pdf` at `dpi` straight to a PNG at `out`, flattened onto white, with
/// libvips streaming it (no full-size buffer here). `strip` leaves out
/// metadata, so the file carries nothing but pixels.
pub fn render_to_png(pdf: &Path, dpi: u32, out: &Path) -> Result<(), String> {
    let src = format!("{}[dpi={dpi}]", pdf.display());
    let bands = header_field(&src, "bands")?;
    let dst = format!("{}[strip,compression=3]", out.display());
    let args: Vec<&str> = if bands == 4 {
        vec!["flatten", &src, &dst, "--background", "255"]
    } else {
        vec!["copy", &src, &dst]
    };
    let o = Command::new(vips_bin())
        .args(&args)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", vips_bin()))?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

pub fn version() -> String {
    Command::new(vipsheader_bin())
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// What libvips rendered: the size and band count `pdfload` reports, and the
/// pixels after flattening any alpha onto white.
pub struct VipsRender {
    pub width: u32,
    pub height: u32,
    /// Bands `pdfload` itself produces, before flattening.
    pub bands: u32,
    /// Interleaved RGB bytes, `width * height * 3`.
    pub rgb: Vec<u8>,
}

/// Render page 1 of `pdf` at `dpi` with `pdfload[dpi=N]`.
///
/// The raw bytes come out through two `vips` calls into `scratch`:
/// `vips flatten` composites any alpha onto white (background 255) and writes a
/// native `.v` image, then `vips rawsave` dumps that as headerless interleaved
/// bytes. The flattened image is 3-band RGB whatever `pdfload` produced.
pub fn render(pdf: &Path, dpi: u32, scratch: &Path, tag: &str) -> Result<VipsRender, String> {
    let src = format!("{}[dpi={dpi}]", pdf.display());
    let (width, height, bands) = (
        header_field(&src, "width")?,
        header_field(&src, "height")?,
        header_field(&src, "bands")?,
    );
    let flat = scratch.join(format!("{tag}.v"));
    let raw = scratch.join(format!("{tag}.raw"));
    let run = |args: &[&str]| -> Result<(), String> {
        let out = Command::new(vips_bin())
            .args(args)
            .output()
            .map_err(|e| format!("cannot run {}: {e}", vips_bin()))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
        }
    };
    let flat_s = flat.to_str().unwrap();
    let raw_s = raw.to_str().unwrap();
    if bands == 4 {
        run(&["flatten", &src, flat_s, "--background", "255"])?;
    } else {
        run(&["copy", &src, flat_s])?;
    }
    run(&["rawsave", flat_s, raw_s])?;
    let rgb = std::fs::read(&raw).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&flat);
    let _ = std::fs::remove_file(&raw);
    if rgb.len() != (width as usize) * (height as usize) * 3 {
        return Err(format!(
            "{} raw bytes for {width}x{height}x3 ({bands} bands before flatten)",
            rgb.len()
        ));
    }
    Ok(VipsRender {
        width,
        height,
        bands,
        rgb,
    })
}

/// The invocation the references are made with, recorded in the manifest.
pub const INVOCATION: &str = "vipsheader -f width|height|bands '<pdf>[dpi=N]'; \
committed rasters: vips flatten '<pdf>[dpi=N]' tmp.v --background 255 (only when pdfload gives 4 bands), \
vips rawsave tmp.v tmp.raw, raw RGB PNG-encoded by the test crate (image crate, Best, Adaptive); \
on-demand rasters: vips flatten '<pdf>[dpi=N]' 'out.png[strip,compression=3]' --background 255";
