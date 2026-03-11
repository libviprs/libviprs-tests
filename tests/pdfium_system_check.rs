//! Manual system check: verify PDFium is installed and ABI-compatible.
//!
//! Run with: cargo test --features pdfium -- --ignored pdfium_system
//!
//! This test is `#[ignore]`d by default because it's a diagnostic/manual check,
//! not a CI gate. It reports whether the PDFium shared library is findable,
//! loadable, and ABI-compatible with pdfium-render 0.8.x.

#![cfg(feature = "pdfium")]

use pdfium_render::prelude::*;
use std::path::Path;

const FIXTURE_PDF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/blueprint.pdf"
);

/// The minimum pdfium-render crate version we depend on.
const PDFIUM_RENDER_VERSION: &str = "0.8";

/// Check that libpdfium is installed and can be loaded from system paths.
///
/// Run manually with:
///   cargo test --features pdfium -- --ignored pdfium_system_check
#[test]
#[ignore]
fn pdfium_system_check() {
    // 1. Report expected library name for this platform
    let lib_name = Pdfium::pdfium_platform_library_name();
    println!("--- PDFium System Check ---");
    println!("Platform library name: {}", lib_name.to_string_lossy());
    println!(
        "pdfium-render crate version: {} (requires >= {})",
        env!("CARGO_PKG_VERSION"),
        PDFIUM_RENDER_VERSION
    );

    // 2. Try to bind to the system library
    let bindings = match Pdfium::bind_to_system_library() {
        Ok(b) => {
            println!("Library loaded: OK");
            b
        }
        Err(e) => {
            panic!(
                "FAILED: PDFium library not found on this system.\n\
                 Error: {e}\n\
                 \n\
                 To install PDFium:\n\
                 - macOS:  Download from https://github.com/niclasvaneyk/pdfium-apple-silicon/releases\n\
                           or https://github.com/niclasvaneyk/pdfium-macos-x64/releases\n\
                           and place libpdfium.dylib in /usr/local/lib/\n\
                 - Linux:  Download from https://github.com/niclasvaneyk/pdfium-linux-x64/releases\n\
                           and place libpdfium.so in /usr/local/lib/\n\
                 - Or set LD_LIBRARY_PATH / DYLD_LIBRARY_PATH to the directory containing the library."
            );
        }
    };

    // 3. Initialize Pdfium and verify basic functionality
    let pdfium = Pdfium::new(bindings);

    // 4. Test: can it open a PDF?
    let doc = match pdfium.load_pdf_from_file(FIXTURE_PDF, None) {
        Ok(d) => {
            println!("Open test PDF: OK");
            d
        }
        Err(e) => {
            panic!(
                "FAILED: PDFium loaded but cannot open PDF files.\n\
                 Error: {e}\n\
                 This may indicate an ABI mismatch between the installed library \
                 and pdfium-render {PDFIUM_RENDER_VERSION}."
            );
        }
    };

    // 5. Test: can it read page info?
    let pages = doc.pages();
    let page_count = pages.len();
    println!("Page count: {page_count}");
    assert!(page_count >= 1, "Expected at least 1 page in fixture PDF");

    let page = pages.get(0).expect("failed to get first page");
    let width = page.width().value;
    let height = page.height().value;
    println!("Page 1 dimensions: {width:.1} x {height:.1} pts");

    // 6. Test: can it render a page to bitmap? (exercises the render pipeline)
    let config = PdfRenderConfig::new()
        .set_target_width(100)
        .set_maximum_height(100);

    match page.render_with_config(&config) {
        Ok(bitmap) => {
            let img = bitmap.as_image();
            let rgba = img.to_rgba8();
            println!(
                "Render test (100px): OK ({}x{} RGBA)",
                rgba.width(),
                rgba.height()
            );
        }
        Err(e) => {
            panic!(
                "FAILED: PDFium loaded and opened PDF, but rendering failed.\n\
                 Error: {e}\n\
                 This may indicate an ABI mismatch or a PDFium build without \
                 rendering support."
            );
        }
    }

    // 7. Test: does libviprs::render_page_pdfium work end-to-end?
    match libviprs::pdf::render_page_pdfium(Path::new(FIXTURE_PDF), 1, 150) {
        Ok(raster) => {
            println!(
                "libviprs render_page_pdfium: OK ({}x{} {:?})",
                raster.width(),
                raster.height(),
                raster.format()
            );
        }
        Err(e) => {
            panic!(
                "FAILED: libviprs::render_page_pdfium returned error: {e}"
            );
        }
    }

    println!("--- All checks passed ---");
}

/// Quick check that common install locations are searched.
/// Reports where PDFium might be found (informational only).
#[test]
#[ignore]
fn pdfium_library_search_paths() {
    println!("--- PDFium Library Search ---");
    println!(
        "Expected filename: {}",
        Pdfium::pdfium_platform_library_name().to_string_lossy()
    );

    let search_paths: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/usr/local/lib",
            "/opt/homebrew/lib",
            "/usr/lib",
        ]
    } else if cfg!(target_os = "linux") {
        &[
            "/usr/local/lib",
            "/usr/lib",
            "/usr/lib/x86_64-linux-gnu",
        ]
    } else {
        &[]
    };

    let lib_name = Pdfium::pdfium_platform_library_name();
    let mut found = false;
    for dir in search_paths {
        let path = Path::new(dir).join(&lib_name);
        if path.exists() {
            println!("  FOUND: {}", path.display());
            found = true;
        } else {
            println!("  not at: {}", path.display());
        }
    }

    // Also check LD_LIBRARY_PATH / DYLD_LIBRARY_PATH
    let env_var = if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "LD_LIBRARY_PATH"
    };
    if let Ok(paths) = std::env::var(env_var) {
        for dir in paths.split(':') {
            let path = Path::new(dir).join(&lib_name);
            if path.exists() {
                println!("  FOUND ({env_var}): {}", path.display());
                found = true;
            }
        }
    }

    if !found {
        println!("\n  PDFium library NOT found in any standard location.");
        println!("  The pdfium feature tests will fail at runtime.");
    }
}
