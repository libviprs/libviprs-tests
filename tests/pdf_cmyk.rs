//! Test CMYK FlateDecode PDF extraction path.
//!
//! The CMYK → RGB conversion in libviprs handles DeviceCMYK color space
//! PDFs common in print/AEC workflows. This test constructs a minimal
//! CMYK PDF using lopdf to exercise the path end-to-end.
//!
//! TODO: Add a real-world CMYK PDF fixture to tests/fixtures/ for more
//! comprehensive coverage (e.g., a scanned blueprint from a print shop).

use libviprs::{PixelFormat, extract_page_image, pdf_info};
use std::io::Write;

/// Build a minimal valid PDF containing a single CMYK FlateDecode image.
fn create_cmyk_pdf(width: u32, height: u32) -> Vec<u8> {
    // CMYK pixel data: 4 bytes per pixel (C, M, Y, K)
    let pixel_count = width as usize * height as usize;
    let mut cmyk_data = Vec::with_capacity(pixel_count * 4);
    for y in 0..height {
        for x in 0..width {
            // Create a gradient pattern
            let c = ((x * 255) / width.max(1)) as u8;
            let m = ((y * 255) / height.max(1)) as u8;
            let y_val = (((x + y) * 127) / (width + height).max(1)) as u8;
            let k = 0u8; // No black
            cmyk_data.extend_from_slice(&[c, m, y_val, k]);
        }
    }

    // Compress with flate2
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&cmyk_data).unwrap();
    let compressed = encoder.finish().unwrap();

    // Build a minimal PDF by hand
    let mut pdf = Vec::new();
    let mut offsets = Vec::new();

    // Header
    pdf.extend_from_slice(b"%PDF-1.4\n");

    // Object 1: Catalog
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    // Object 2: Pages
    offsets.push(pdf.len());
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    // Object 3: Page
    offsets.push(pdf.len());
    let page = format!(
        "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Resources << /XObject << /Im0 4 0 R >> >> >>\nendobj\n"
    );
    pdf.extend_from_slice(page.as_bytes());

    // Object 4: Image XObject (CMYK FlateDecode)
    offsets.push(pdf.len());
    let stream_dict = format!(
        "4 0 obj\n<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        compressed.len()
    );
    pdf.extend_from_slice(stream_dict.as_bytes());
    pdf.extend_from_slice(&compressed);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    // Cross-reference table
    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n");
    pdf.extend_from_slice(format!("0 {}\n", offsets.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }

    // Trailer
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            offsets.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );

    pdf
}

#[test]
fn extract_cmyk_image_from_synthetic_pdf() {
    let pdf_bytes = create_cmyk_pdf(16, 16);

    // Write to temp file
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cmyk_test.pdf");
    std::fs::write(&path, &pdf_bytes).unwrap();

    // Verify PDF is parseable
    let info = pdf_info(&path).expect("failed to parse synthetic CMYK PDF");
    assert_eq!(info.page_count, 1);
    assert!(
        info.pages[0].has_images,
        "Page should have an image XObject"
    );

    // Extract the image — this exercises the CMYK → RGB path
    let raster = extract_page_image(&path, 1).expect("failed to extract CMYK image");

    // Should be converted to RGB8
    assert_eq!(raster.format(), PixelFormat::Rgb8);
    assert_eq!(raster.width(), 16);
    assert_eq!(raster.height(), 16);

    // Verify pixel values: first pixel is C=0,M=0,Y=0,K=0 → R=255,G=255,B=255
    let data = raster.data();
    assert_eq!(data[0], 255, "R channel for zero CMYK should be 255");
    assert_eq!(data[1], 255, "G channel for zero CMYK should be 255");
    assert_eq!(data[2], 255, "B channel for zero CMYK should be 255");
}

#[test]
fn cmyk_full_black_converts_correctly() {
    // Single-pixel CMYK with K=255 → should produce black (0,0,0)
    let pdf_bytes = create_single_pixel_cmyk_pdf(0, 0, 0, 255);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cmyk_black.pdf");
    std::fs::write(&path, &pdf_bytes).unwrap();

    let raster = extract_page_image(&path, 1).expect("failed to extract CMYK image");
    let data = raster.data();
    assert_eq!(data[0], 0, "R should be 0 for full black");
    assert_eq!(data[1], 0, "G should be 0 for full black");
    assert_eq!(data[2], 0, "B should be 0 for full black");
}

#[test]
fn cmyk_pure_cyan_converts_correctly() {
    // C=255, M=0, Y=0, K=0 → R=0, G=255, B=255
    let pdf_bytes = create_single_pixel_cmyk_pdf(255, 0, 0, 0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cmyk_cyan.pdf");
    std::fs::write(&path, &pdf_bytes).unwrap();

    let raster = extract_page_image(&path, 1).expect("failed to extract CMYK image");
    let data = raster.data();
    assert_eq!(data[0], 0, "R should be 0 for pure cyan");
    assert_eq!(data[1], 255, "G should be 255 for pure cyan");
    assert_eq!(data[2], 255, "B should be 255 for pure cyan");
}

/// Helper: create a PDF with a single 1x1 CMYK pixel.
fn create_single_pixel_cmyk_pdf(c: u8, m: u8, y: u8, k: u8) -> Vec<u8> {
    let cmyk_data = vec![c, m, y, k];

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&cmyk_data).unwrap();
    let compressed = encoder.finish().unwrap();

    let mut pdf = Vec::new();
    let mut offsets = Vec::new();

    pdf.extend_from_slice(b"%PDF-1.4\n");

    offsets.push(pdf.len());
    pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");

    offsets.push(pdf.len());
    pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");

    offsets.push(pdf.len());
    pdf.extend_from_slice(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Resources << /XObject << /Im0 4 0 R >> >> >>\nendobj\n");

    offsets.push(pdf.len());
    let stream_dict = format!(
        "4 0 obj\n<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceCMYK /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        compressed.len()
    );
    pdf.extend_from_slice(stream_dict.as_bytes());
    pdf.extend_from_slice(&compressed);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");

    let xref_offset = pdf.len();
    pdf.extend_from_slice(b"xref\n");
    pdf.extend_from_slice(format!("0 {}\n", offsets.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for off in &offsets {
        pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
    }

    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            offsets.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );

    pdf
}
