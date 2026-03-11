#![cfg(feature = "ported_tests")]

//! Ported colour-space tests from libvips `test_colour.py`.
//!
//! These tests exercise colour-space conversions (Lab↔XYZ↔LCH↔CMC↔sRGB↔CMYK),
//! delta-E calculations (dE00, dE76, dECMC), ICC profile transforms, and
//! approximate CMYK↔sRGB round-trips.
//! Tests use the reference JPEG fixture (`sample.jpg`) and synthetic Lab images.

use std::path::Path;

use libviprs::{decode_file, PixelFormat, Raster};

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

#[test]
#[ignore]
/// Colour-space round-trip: convert through a chain of colour spaces and
/// verify the result comes back to the starting point.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Supported colour-space interpretations.
/// pub enum Interpretation {
///     Lab, Xyz, Lch, Cmc, Labs, ScRgb, Hsv, Srgb, Yxy, OkLab, OkLch,
///     Rgb16, Grey16, Bw, Cmyk,
/// }
///
/// /// Convert an image from its current interpretation to the target colour space.
/// /// The image must carry an `interpretation` field so the converter knows the source space.
/// fn Raster::colourspace(&self, target: Interpretation) -> Raster;
///
/// /// Get the current colour-space interpretation of this image.
/// fn Raster::interpretation(&self) -> Interpretation;
///
/// /// Create a constant-colour image: a `w`×`h` image where every pixel has
/// /// the same value (given as a slice of band values).
/// fn Raster::constant(w: u32, h: u32, values: &[f64], interpretation: Interpretation) -> Raster;
///
/// /// Read pixel as f64 values at (x, y).
/// fn Raster::getpoint(&self, x: u32, y: u32) -> Vec<f64>;
/// ```
///
/// ## Test logic (from libvips test_colour.py::test_colourspace)
///
/// 1. Create a 100×100 constant Lab image: [50, 0, 0, 42] (mid-grey with extra band).
/// 2. Convert through each colour space in sequence:
///    Lab → Xyz → Lch → Cmc → Labs → ScRgb → Hsv → Srgb → Yxy → OkLab → OkLch → Lab.
/// 3. After each conversion, assert the interpretation changes correctly.
/// 4. After each conversion, the extra band (42) should be preserved.
/// 5. After the full round-trip, compare start and end pixel at (10,10):
///    all channels should match within threshold 0.1.
/// 6. Go between every pair of colour spaces (start, end) and verify
///    the round-trip to Lab matches.
/// 7. Test Lab→XYZ against Lindbloom reference:
///    Lab [50, 0, 0] → XYZ should be approx [17.5064, 18.4187, 20.0547].
/// 8. Test grey→colour→grey round-trip for mono formats.
/// 9. Test CMYK→colour→CMYK round-trip.
///
/// Reference: test_colour.py::test_colourspace
fn test_colourspace_roundtrip() {
    // Create constant Lab image [50, 0, 0, 42]
    let test = Raster::constant(100, 100, &[50.0, 0.0, 0.0, 42.0], Interpretation::Lab);

    let colour_spaces = [
        Interpretation::Xyz, Interpretation::Lch, Interpretation::Cmc,
        Interpretation::Labs, Interpretation::ScRgb, Interpretation::Hsv,
        Interpretation::Srgb, Interpretation::Yxy, Interpretation::OkLab,
        Interpretation::OkLch, Interpretation::Lab,
    ];

    let mut im = test.clone();
    for &cs in &colour_spaces {
        im = im.colourspace(cs);
        assert_eq!(im.interpretation(), cs);
    }

    // Round-trip should come back close to the original
    let before = test.getpoint(10, 10);
    let after = im.getpoint(10, 10);
    for (b, a) in before.iter().zip(after.iter()) {
        assert!(
            (b - a).abs() < 0.1,
            "Round-trip mismatch: before={b}, after={a}"
        );
    }

    // Test Lab→XYZ against Lindbloom reference for mid-grey
    let xyz = test.colourspace(Interpretation::Xyz);
    let px = xyz.getpoint(10, 10);
    let expected = [17.5064, 18.4187, 20.0547, 42.0];
    for (got, exp) in px.iter().zip(expected.iter()) {
        assert!(
            (got - exp).abs() < 0.01,
            "Lab→XYZ Lindbloom mismatch: got={got}, expected={exp}"
        );
    }
}

#[test]
#[ignore]
/// Mono colour-space conversions (greyscale → colour → greyscale).
///
/// ## Required API
///
/// Same `Raster::colourspace` and `Interpretation` as above.
///
/// ## Test logic (from libvips test_colour.py::test_colourspace, grey section)
///
/// 1. Start with a Lab test image.
/// 2. Convert to mono (B_W / Grey16).
/// 3. Convert through colour spaces and back to mono.
/// 4. Verify the grey value and alpha are preserved within tolerance.
///
/// Reference: test_colour.py::test_colourspace (grey→colour→grey section)
fn test_colourspace_mono() {
    let test = Raster::constant(100, 100, &[50.0, 0.0, 0.0, 42.0], Interpretation::Lab);

    for &mono_fmt in &[Interpretation::Bw, Interpretation::Grey16] {
        let test_grey = test.colourspace(mono_fmt);
        let mut im = test_grey.clone();

        let colour_spaces = [
            Interpretation::Xyz, Interpretation::Lab, Interpretation::Srgb,
            mono_fmt,
        ];
        for &cs in &colour_spaces {
            im = im.colourspace(cs);
            assert_eq!(im.interpretation(), cs);
        }

        let before = test_grey.getpoint(10, 10);
        let after = im.getpoint(10, 10);

        // Alpha should be preserved
        let alpha_diff = (after.last().unwrap() - before.last().unwrap()).abs();
        assert!(alpha_diff < 1.0, "Alpha not preserved in grey round-trip");

        // Grey value tolerance depends on bit depth
        let grey_threshold = if mono_fmt == Interpretation::Grey16 { 30.0 } else { 1.0 };
        let grey_diff = (after[0] - before[0]).abs();
        assert!(
            grey_diff < grey_threshold,
            "Grey value mismatch: before={}, after={}, diff={grey_diff}",
            before[0], after[0]
        );
    }
}

#[test]
#[ignore]
/// CMYK round-trip through colour spaces.
///
/// ## Required API
///
/// Same `Raster::colourspace` with `Interpretation::Cmyk`.
///
/// ## Test logic (from libvips test_colour.py::test_colourspace, CMYK section)
///
/// 1. Start with a Lab test image, convert to CMYK.
/// 2. For each 3-band colour space, convert CMYK→colour→CMYK.
/// 3. Verify the CMYK values match within threshold 10.
///
/// Reference: test_colour.py::test_colourspace (CMYK section)
fn test_colourspace_cmyk() {
    let test = Raster::constant(100, 100, &[50.0, 0.0, 0.0, 42.0], Interpretation::Lab);
    let cmyk = test.colourspace(Interpretation::Cmyk);

    let colour_spaces = [
        Interpretation::Xyz, Interpretation::Lab, Interpretation::Lch,
        Interpretation::Srgb,
    ];

    for &cs in &colour_spaces {
        let im = cmyk.colourspace(cs);
        let im2 = im.colourspace(Interpretation::Cmyk);

        let before = cmyk.getpoint(10, 10);
        let after = im2.getpoint(10, 10);
        for (b, a) in before.iter().zip(after.iter()) {
            assert!(
                (b - a).abs() < 10.0,
                "CMYK round-trip mismatch via {cs:?}: before={b}, after={a}"
            );
        }
    }
}

#[test]
#[ignore]
/// Lab→XYZ conversion verified against Lindbloom reference.
///
/// ## Required API
///
/// Same as above.
///
/// ## Test logic
///
/// 1. Create Lab image [50, 0, 0].
/// 2. Convert to XYZ.
/// 3. Verify pixel matches Lindbloom values: [17.5064, 18.4187, 20.0547].
///
/// Reference: test_colour.py::test_colourspace (Lindbloom section)
fn test_lab_xyz_reference() {
    let test = Raster::constant(100, 100, &[50.0, 0.0, 0.0], Interpretation::Lab);
    let xyz = test.colourspace(Interpretation::Xyz);
    let px = xyz.getpoint(10, 10);

    let expected = [17.5064, 18.4187, 20.0547];
    for (i, (&got, &exp)) in px.iter().zip(expected.iter()).enumerate() {
        assert!(
            (got - exp).abs() < 0.01,
            "Lab→XYZ channel {i}: got={got}, expected={exp}"
        );
    }
}

#[test]
#[ignore]
/// Delta E 2000 (CIEDE2000) colour difference.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Compute CIEDE2000 colour difference between two Lab images.
/// /// Both images must be in Lab interpretation.
/// /// Returns a single-band image of dE00 values (plus any extra bands preserved).
/// fn Raster::de00(&self, other: &Raster) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_colour.py::test_dE00)
///
/// 1. Reference: Lab [50, 10, 20, 42] (100×100 constant image).
/// 2. Sample: Lab [40, -20, 10] (100×100 constant image).
/// 3. Compute dE00.
/// 4. Read pixel at (10, 10): dE00 ≈ 30.238, alpha = 42.
///
/// Reference: test_colour.py::test_dE00
fn test_de00() {
    let reference = Raster::constant(100, 100, &[50.0, 10.0, 20.0, 42.0], Interpretation::Lab);
    let sample = Raster::constant(100, 100, &[40.0, -20.0, 10.0], Interpretation::Lab);

    let difference = reference.de00(&sample);
    let px = difference.getpoint(10, 10);

    assert!(
        (px[0] - 30.238).abs() < 0.01,
        "dE00 should be ~30.238, got {}",
        px[0]
    );
    assert!(
        (px[1] - 42.0).abs() < 0.01,
        "Extra band (alpha) should be 42, got {}",
        px[1]
    );
}

#[test]
#[ignore]
/// Delta E 76 (CIE76) colour difference.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Compute CIE76 colour difference (Euclidean distance in Lab).
/// fn Raster::de76(&self, other: &Raster) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_colour.py::test_dE76)
///
/// 1. Reference: Lab [50, 10, 20, 42].
/// 2. Sample: Lab [40, -20, 10].
/// 3. dE76 at (10,10) ≈ 33.166, alpha = 42.
///
/// Reference: test_colour.py::test_dE76
fn test_de76() {
    let reference = Raster::constant(100, 100, &[50.0, 10.0, 20.0, 42.0], Interpretation::Lab);
    let sample = Raster::constant(100, 100, &[40.0, -20.0, 10.0], Interpretation::Lab);

    let difference = reference.de76(&sample);
    let px = difference.getpoint(10, 10);

    assert!(
        (px[0] - 33.166).abs() < 0.01,
        "dE76 should be ~33.166, got {}",
        px[0]
    );
    assert!(
        (px[1] - 42.0).abs() < 0.01,
        "Extra band should be 42, got {}",
        px[1]
    );
}

#[test]
#[ignore]
/// Delta E CMC colour difference.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Compute CMC colour difference.
/// fn Raster::de_cmc(&self, other: &Raster) -> Raster;
/// ```
///
/// ## Test logic (from libvips test_colour.py::test_dECMC)
///
/// 1. Reference: Lab [50, 10, 20, 42].
/// 2. Sample: Lab [55, 11, 23].
/// 3. dECMC at (10,10) ≈ 4.97 (within 0.5 tolerance), alpha = 42.
///
/// Reference: test_colour.py::test_dECMC
fn test_decmc() {
    let reference = Raster::constant(100, 100, &[50.0, 10.0, 20.0, 42.0], Interpretation::Lab);
    let sample = Raster::constant(100, 100, &[55.0, 11.0, 23.0], Interpretation::Lab);

    let difference = reference.de_cmc(&sample);
    let px = difference.getpoint(10, 10);

    assert!(
        (px[0] - 4.97).abs() < 0.5,
        "dECMC should be ~4.97, got {}",
        px[0]
    );
    assert!(
        (px[1] - 42.0).abs() < 0.01,
        "Extra band should be 42, got {}",
        px[1]
    );
}

#[test]
#[ignore]
/// ICC profile import/export/transform.
///
/// ## Required API
///
/// ```rust,ignore
/// /// Import from device colour space to PCS (Lab or XYZ) using the embedded ICC profile.
/// fn Raster::icc_import(&self) -> Raster;
/// fn Raster::icc_import_with(&self, intent: Intent, input_profile: Option<&Path>, pcs: Option<Pcs>) -> Raster;
///
/// /// Export from PCS to device colour space using an ICC profile.
/// fn Raster::icc_export(&self) -> Raster;
/// fn Raster::icc_export_with(&self, depth: u32, intent: Intent, output_profile: Option<&Path>) -> Raster;
///
/// /// Transform between ICC profiles in one step.
/// fn Raster::icc_transform(&self, output_profile: &Path) -> Raster;
///
/// pub enum Intent { Perceptual, Relative, Saturation, Absolute }
/// pub enum Pcs { Lab, Xyz }
/// ```
///
/// ## Test logic (from libvips test_colour.py::test_icc)
///
/// 1. Load sample.jpg.
/// 2. Import then export: dE76 vs original should be < 6.
/// 3. Import, export at depth=16: should produce 16-bit output.
/// 4. Import with absolute intent, export with absolute intent: dE76 < 6.
/// 5. Import, export with sRGB output profile: dE76 vs colourspace(srgb) < 6.
/// 6. icc_transform to sRGB: dE76 vs colourspace(srgb) < 6.
/// 7. Import with forced sRGB input profile vs default: dE76 > 6 (different profiles).
/// 8. Import with PCS=XYZ: interpretation should be XYZ.
/// 9. Import with default PCS: interpretation should be Lab.
///
/// Reference: test_colour.py::test_icc
fn test_icc_transform() {
    let test = decode_file(&ref_image("sample.jpg")).unwrap();
    let srgb_profile = ref_image("sRGB.icm");

    // Import then export should round-trip
    let imported = test.icc_import();
    let exported = imported.icc_export();
    let de = exported.de76(&test);
    let max_de: f64 = de.max_value();
    assert!(max_de < 6.0, "ICC import+export dE76 should be < 6, got {max_de}");

    // Export at 16-bit depth
    let exported_16 = imported.icc_export_with(16, Intent::Perceptual, None);
    assert_eq!(exported_16.format().bytes_per_channel(), 2, "16-bit export should be 16bpc");

    // With output_profile = sRGB
    let exported_srgb = imported.icc_export_with(8, Intent::Perceptual, Some(&srgb_profile));
    let srgb_conv = imported.colourspace(Interpretation::Srgb);
    let de = exported_srgb.de76(&srgb_conv);
    assert!(de.max_value() < 6.0);

    // ICC transform
    let transformed = test.icc_transform(&srgb_profile);
    let srgb_conv = test.icc_import().colourspace(Interpretation::Srgb);
    let de = transformed.de76(&srgb_conv);
    assert!(de.max_value() < 6.0);

    // Import with XYZ PCS
    let xyz_import = test.icc_import_with(Intent::Perceptual, None, Some(Pcs::Xyz));
    assert_eq!(xyz_import.interpretation(), Interpretation::Xyz);

    // Default import should be Lab
    let lab_import = test.icc_import();
    assert_eq!(lab_import.interpretation(), Interpretation::Lab);
}

#[test]
#[ignore]
/// CMYK→sRGB approximate conversion (without lcms).
///
/// ## Required API
///
/// Same `Raster::colourspace` with Cmyk and Srgb interpretations.
///
/// ## Test logic (from libvips test_colour.py::test_cmyk)
///
/// 1. Load sample.jpg (sRGB).
/// 2. Convert to CMYK, then back to sRGB.
/// 3. Compare pixel at (150, 210): before and after should be close (threshold 10).
///
/// Reference: test_colour.py::test_cmyk
fn test_cmyk_to_srgb() {
    let test = decode_file(&ref_image("sample.jpg")).unwrap();

    let cmyk = test.colourspace(Interpretation::Cmyk);
    let srgb = cmyk.colourspace(Interpretation::Srgb);

    let before = test.getpoint(150, 210);
    let after = srgb.getpoint(150, 210);

    for (b, a) in before.iter().zip(after.iter()) {
        assert!(
            (b - a).abs() < 10.0,
            "CMYK→sRGB round-trip pixel mismatch: before={b}, after={a}"
        );
    }
}
