//! The codec round-trip and oracle matrix (libviprs/libviprs-tests#230).
//!
//! One row per codec the core ships, every row on committed fixtures under
//! `tests/fixtures/codec/`, and every reference written by a tool that is not
//! libviprs. The core's own unit tests check each codec against itself, which
//! is exactly the check that cannot see a codec agreeing with its own mistake;
//! every comparison here has vips (or, for the three formats vips cannot
//! write, nibabel, scipy or oiiotool) on the other side.
//!
//! Four kinds of cell:
//!
//! - **decode**: a file vips wrote, decoded by libviprs, compared against
//!   vips's own decode of the same file (`*_ref.png`, or `*_ref.v` for float).
//! - **encode**: where libviprs has an encoder, `src_rgb.png` encoded by
//!   libviprs (`enc/libviprs_*`), decoded offline by vips (`enc/*_vips.*`),
//!   and compared both ways: vips's decode against the source, and libviprs's
//!   decode of a fresh encode against vips's decode.
//! - **limits**: each decoder refuses the same file once a `DecodeLimits`
//!   ceiling sits one below what it needs, with a typed variant, and accepts
//!   it with the ceiling exactly at what it needs, so the boundary is pinned
//!   from both sides.
//! - **pages**: TIFF pages, GIF frames and WebP frames, the page count and
//!   every page against vips's decode of that page.
//!
//! # Tolerances
//!
//! Every lossless row is exact. Every lossy row states its tolerance next to
//! the cell as a [`Tol`], and every lossy codec has a control cell
//! (`*_tolerance_catches_one_quality_step`) that compares libviprs's decode of
//! the file at quality Q against vips's decode of the same source at Q-1 and
//! requires the tolerance to REJECT it. A tolerance loose enough to wave
//! through a one-step quality change is not checking the decoder.
//!
//! # Feature cells
//!
//! `avif`, `svg`, `jxl` and `jp2k` are non-default core features. Their cells
//! always compile (the core keeps every entry point in both builds and answers
//! a typed refusal without the feature), and they run for real only when the
//! core is built with the feature:
//!
//! ```text
//! cargo test --test codec_e2e --features libviprs/avif
//! cargo test --test codec_e2e --features libviprs/svg
//! cargo test --test codec_e2e --features jxl
//! cargo test --test codec_e2e --features libviprs/jp2k
//! ```
//!
//! CI runs all four at once, as `--features "jxl avif svg jp2k"`, with
//! `CODEC_E2E_REQUIRE_FEATURES=all`, so none of them can skip there
//! (`tests/codec_ci_wiring.rs` holds that step in place). The `avif`, `svg`
//! and `jp2k` spellings are this crate's forwarding of the core features
//! (libviprs/libviprs-tests#235); `--features svg` is also what turns on the
//! [`svg_build`] check that stops an svg build reading `Unsupported` as a
//! skip.
//!
//! Without the feature a cell SKIPS, and says so on stderr outside libtest's
//! capture, so the skip shows up in every run rather than only under
//! `--nocapture`. A skip reads to `cargo test` as a pass, so
//! `CODEC_E2E_REQUIRE_FEATURES` (a comma list, or `all`) turns the skip into a
//! failure for a job that means to have the feature on.
//!
//! # Fixtures
//!
//! Generated once by `tools/gen_fixtures.sh codec-vips`, the
//! `generate_libviprs_encodes` cell below, and `tools/gen_fixtures.sh
//! codec-enc`, in that order, inside the image `tools/Dockerfile.codec-oracle`
//! builds. Each file's tool, version and command are in
//! `tests/fixtures/README.md`.

use std::io::Write;
use std::path::PathBuf;

use libviprs::source::{DecodeLimits, SourceError, decode_bytes_with_limits};
use libviprs::{PixelFormat, Raster};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/codec")
        .join(name)
}

fn bytes(name: &str) -> Vec<u8> {
    let path = fx(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Samples, read independently of libviprs
// ---------------------------------------------------------------------------

/// A decoded image as plain samples, so a libviprs [`Raster`], a PNG read by
/// the `image` crate and a `.v` read by [`Img::from_v`] all compare the same
/// way.
#[derive(Clone, Debug)]
struct Img {
    w: u32,
    h: u32,
    bands: usize,
    s: Vec<f64>,
}

impl Img {
    fn from_raster(r: &Raster) -> Img {
        let f = r.format();
        let bands = f.channels();
        let d = r.data();
        let s: Vec<f64> = match (f.bytes_per_channel(), f) {
            (1, PixelFormat::Int8(_)) => d.iter().map(|&b| f64::from(b as i8)).collect(),
            (1, _) => d.iter().map(|&b| f64::from(b)).collect(),
            (2, PixelFormat::Int16(_)) => d
                .chunks_exact(2)
                .map(|c| f64::from(i16::from_ne_bytes([c[0], c[1]])))
                .collect(),
            (2, _) => d
                .chunks_exact(2)
                .map(|c| f64::from(u16::from_ne_bytes([c[0], c[1]])))
                .collect(),
            (4, PixelFormat::RgbaF32 | PixelFormat::FloatF32(_)) => d
                .chunks_exact(4)
                .map(|c| f64::from(f32::from_ne_bytes([c[0], c[1], c[2], c[3]])))
                .collect(),
            (4, PixelFormat::Int32(_)) => d
                .chunks_exact(4)
                .map(|c| f64::from(i32::from_ne_bytes([c[0], c[1], c[2], c[3]])))
                .collect(),
            (4, _) => d
                .chunks_exact(4)
                .map(|c| f64::from(u32::from_ne_bytes([c[0], c[1], c[2], c[3]])))
                .collect(),
            (n, f) => panic!("no sample reader for {n}-byte {f:?}"),
        };
        Img {
            w: r.width(),
            h: r.height(),
            bands,
            s,
        }
    }

    /// A PNG through the `image` crate, never through libviprs.
    fn from_png(name: &str) -> Img {
        let img = image::load_from_memory_with_format(&bytes(name), image::ImageFormat::Png)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let (w, h) = (img.width(), img.height());
        let bands = usize::from(img.color().channel_count());
        let s: Vec<f64> = match img.color().bytes_per_pixel() / img.color().channel_count() {
            1 => img.as_bytes().iter().map(|&b| f64::from(b)).collect(),
            2 => match &img {
                image::DynamicImage::ImageLuma16(i) => {
                    i.as_raw().iter().map(|&v| f64::from(v)).collect()
                }
                image::DynamicImage::ImageLumaA16(i) => {
                    i.as_raw().iter().map(|&v| f64::from(v)).collect()
                }
                image::DynamicImage::ImageRgb16(i) => {
                    i.as_raw().iter().map(|&v| f64::from(v)).collect()
                }
                image::DynamicImage::ImageRgba16(i) => {
                    i.as_raw().iter().map(|&v| f64::from(v)).collect()
                }
                other => panic!("{name}: unexpected 16-bit layout {:?}", other.color()),
            },
            n => panic!("{name}: {n}-byte PNG samples"),
        };
        Img { w, h, bands, s }
    }

    /// A vips `.v` file, read here from its documented 64-byte header rather
    /// than through libviprs's own `.v` decoder, which is one of the things
    /// under test. Only the uncoded sample formats a float reference uses.
    fn from_v(name: &str) -> Img {
        let b = bytes(name);
        // The magic is stored most significant byte first whatever the
        // writer's byte order, and names that order: b6 a6 f2 08 is an Intel
        // (little-endian) file, which is all an x86_64 oracle writes.
        let magic = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        assert_eq!(magic, 0xb6a6_f208, "{name}: not a little-endian .v file");
        let le = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let (w, h, bands, fmt, coding) = (le(4), le(8), le(12) as usize, le(20), le(24));
        assert_eq!(coding, 0, "{name}: coded .v (coding {coding})");
        let n = (w * h) as usize * bands;
        let px = &b[64..];
        let s: Vec<f64> = match fmt {
            0 => px[..n].iter().map(|&v| f64::from(v)).collect(),
            2 => px[..n * 2]
                .chunks_exact(2)
                .map(|c| f64::from(u16::from_le_bytes([c[0], c[1]])))
                .collect(),
            6 => px[..n * 4]
                .chunks_exact(4)
                .map(|c| f64::from(f32::from_le_bytes([c[0], c[1], c[2], c[3]])))
                .collect(),
            other => panic!("{name}: .v band format {other} not handled"),
        };
        Img {
            w: w as u32,
            h: h as u32,
            bands,
            s,
        }
    }

    /// The first `keep` bands, after checking every dropped band holds
    /// `value` everywhere, so dropping one can never hide a difference.
    fn without_constant_bands(&self, keep: usize, value: f64) -> Img {
        assert!(keep <= self.bands);
        let mut s = Vec::with_capacity(self.s.len() / self.bands * keep);
        for px in self.s.chunks_exact(self.bands) {
            s.extend_from_slice(&px[..keep]);
            for &v in &px[keep..] {
                assert_eq!(v, value, "a dropped band is not the constant {value}");
            }
        }
        Img {
            w: self.w,
            h: self.h,
            bands: keep,
            s,
        }
    }
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

/// How far two decodes of the same pixels may sit apart.
#[derive(Clone, Copy, Debug)]
struct Tol {
    /// Largest permitted absolute difference in any one sample.
    max_abs: f64,
    /// Largest permitted mean absolute difference over every sample.
    mean_abs: f64,
}

const EXACT: Tol = Tol {
    max_abs: 0.0,
    mean_abs: 0.0,
};

#[derive(Debug)]
struct Diff {
    max_abs: f64,
    mean_abs: f64,
    differing: usize,
}

impl Diff {
    fn within(&self, tol: Tol) -> bool {
        self.max_abs <= tol.max_abs && self.mean_abs <= tol.mean_abs
    }
}

fn diff(got: &Img, want: &Img) -> Diff {
    let (mut max_abs, mut sum, mut differing) = (0.0f64, 0.0f64, 0usize);
    for (&a, &b) in got.s.iter().zip(&want.s) {
        let d = (a - b).abs();
        if d.is_nan() {
            max_abs = f64::INFINITY;
        }
        if d > 0.0 {
            differing += 1;
        }
        max_abs = max_abs.max(d);
        sum += d;
    }
    Diff {
        max_abs,
        mean_abs: sum / want.s.len() as f64,
        differing,
    }
}

/// `CODEC_E2E_REPORT=1` prints every measured distance, which is how the
/// numbers quoted beside each tolerance were taken.
fn report(cell: &str, d: &Diff, tol: Tol) {
    if std::env::var_os("CODEC_E2E_REPORT").is_some() {
        let _ = writeln!(
            std::io::stderr(),
            "MEASURE {cell}: {} differ, max |d| {} mean |d| {:.5}, tol {tol:?}",
            d.differing,
            d.max_abs,
            d.mean_abs
        );
    }
}

fn assert_close(cell: &str, got: &Img, want: &Img, tol: Tol) {
    assert_eq!(
        (got.w, got.h, got.bands),
        (want.w, want.h, want.bands),
        "{cell}: geometry (w, h, bands) differs from the reference"
    );
    let d = diff(got, want);
    report(cell, &d, tol);
    assert!(
        d.within(tol),
        "{cell}: {} of {} samples differ, max |d| {} mean |d| {:.5}, over the tolerance {tol:?}",
        d.differing,
        want.s.len(),
        d.max_abs,
        d.mean_abs
    );
}

/// The control half of a lossy row: the tolerance has to reject a decode
/// compared against the neighbouring quality's reference.
fn assert_rejects(cell: &str, got: &Img, neighbour: &Img, tol: Tol) {
    assert_eq!(
        (got.w, got.h, got.bands),
        (neighbour.w, neighbour.h, neighbour.bands)
    );
    let d = diff(got, neighbour);
    report(cell, &d, tol);
    assert!(
        !d.within(tol),
        "{cell}: the tolerance {tol:?} accepts a one-step quality change \
         (max |d| {} mean |d| {:.5}), so it cannot tell a decoder bug from a \
         different file",
        d.max_abs,
        d.mean_abs
    );
}

// ---------------------------------------------------------------------------
// Feature cells
// ---------------------------------------------------------------------------

type Decoder = fn(&[u8], DecodeLimits) -> Result<Raster, SourceError>;

/// True when `err` is the typed refusal a core built without `feature`
/// answers. Matched on the variant, never on a message.
fn is_feature_refusal(feature: &str, err: &SourceError) -> bool {
    match feature {
        "avif" => matches!(
            err,
            SourceError::Avif(libviprs::AvifError::FeatureNotEnabled)
        ),
        "jxl" => matches!(err, SourceError::Jxl(libviprs::JxlError::FeatureNotEnabled)),
        "jp2k" => matches!(
            err,
            SourceError::Jp2k(libviprs::Jp2kError::FeatureNotEnabled)
        ),
        // A bare core answers `Unsupported` as its refusal, so only a build
        // without the renderer may read it that way. With the renderer in,
        // the same error is the renderer failing, and that is a failure.
        "svg" => {
            !svg_build()
                && matches!(err, SourceError::Io(e) if e.kind() == std::io::ErrorKind::Unsupported)
        }
        other => panic!("unknown codec feature {other}"),
    }
}

/// An `svg` build must never read an `Unsupported` I/O error as the missing
/// feature: with the renderer compiled in, that error is the renderer failing,
/// and calling it a skip would hide it.
#[test]
fn an_svg_build_never_skips_an_unsupported_error() {
    if !svg_build() {
        let _ = writeln!(
            std::io::stderr(),
            "SKIP codec_e2e::an_svg_build_never_skips_an_unsupported_error: built without \
             `svg`, where Unsupported is the refusal. Run with `--features svg`."
        );
        return;
    }
    let err = SourceError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the renderer gave up",
    ));
    assert!(
        !is_feature_refusal("svg", &err),
        "an svg build treats an Unsupported error from the renderer as a skip"
    );
}

fn is_encode_refusal(feature: &str, err: &libviprs::EncodeError) -> bool {
    matches!(err, libviprs::EncodeError::Unsupported { format } if format == feature)
}

/// Reports a skipped feature cell, or fails it when the run demands the
/// feature. Written straight to the stderr handle so libtest's capture does
/// not swallow it on a passing test.
fn skip(feature: &str, cell: &str) {
    let required = std::env::var("CODEC_E2E_REQUIRE_FEATURES").unwrap_or_default();
    let demanded = required
        .split(',')
        .map(str::trim)
        .any(|f| f == feature || f == "all");
    assert!(
        !demanded,
        "{cell}: the core was built without `{feature}`, and CODEC_E2E_REQUIRE_FEATURES={required} \
         says this run must have it"
    );
    let _ = writeln!(
        std::io::stderr(),
        "SKIP codec_e2e::{cell}: libviprs built without the `{feature}` feature, nothing was \
         checked. Run `cargo test --test codec_e2e --features libviprs/{feature}`."
    );
}

/// Whether this crate was built with its own `svg` feature, which forwards
/// the core's (libviprs/libviprs-tests#235). Before that forwarding lands the
/// feature does not exist here, `cfg!` answers false, and check-cfg would warn
/// about the unknown name, hence the allow.
#[allow(unexpected_cfgs)]
const fn svg_build() -> bool {
    cfg!(feature = "svg")
}

/// Decode `file` for `cell`, or `None` when the cell skipped for a missing
/// feature.
fn decode_for(cell: &str, feature: Option<&str>, decode: Decoder, file: &str) -> Option<Raster> {
    match decode(&bytes(file), DecodeLimits::default()) {
        Ok(r) => Some(r),
        Err(e) if feature.is_some_and(|f| is_feature_refusal(f, &e)) => {
            skip(feature.unwrap(), cell);
            None
        }
        Err(e) => panic!("{cell}: decoding {file} failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Decoders, one per codec, all at the entry point a caller who knows the
// format would use
// ---------------------------------------------------------------------------

fn sniffed(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    decode_bytes_with_limits(b, l)
}
fn gif_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_gif(b, l)
}
fn webp_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_webp(b, l)
}
fn jxl_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_jxl(b, l)
}
fn jp2k_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_jp2k(b, l)
}
fn avif_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_avif(b, l)
}
fn rad_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_radiance(b, l)
}
fn fits_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_fits(b, l)
}
fn exr_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_exr(b, l)
}
fn nifti_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_nifti(b, l)
}
fn mat_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_mat(b, l)
}
/// Analyze is a header and an image file; `b` is the header.
fn analyze_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::analyze::decode_analyze(b, &bytes("analyze.img"), l)
}
fn svg_dec(b: &[u8], l: DecodeLimits) -> Result<Raster, SourceError> {
    libviprs::decode_svg_with_limits(b, libviprs::SvgOptions::default(), l)
}

// ---------------------------------------------------------------------------
// Decode rows: a vips-written file against vips's own decode of it
// ---------------------------------------------------------------------------

/// Decode `file` and compare it with the reference.
fn decode_row(
    cell: &str,
    feature: Option<&str>,
    decode: Decoder,
    file: &str,
    want: &Img,
    tol: Tol,
) -> Option<Img> {
    let got = Img::from_raster(&decode_for(cell, feature, decode, file)?);
    assert_close(cell, &got, want, tol);
    Some(got)
}

fn exact_row(cell: &str, feature: Option<&str>, decode: Decoder, file: &str, reference: &str) {
    decode_row(
        cell,
        feature,
        decode,
        file,
        &Img::from_png(reference),
        EXACT,
    );
}

#[test]
fn decode_png_interlaced_matches_vips() {
    exact_row(
        "decode_png_interlaced_matches_vips",
        None,
        sniffed,
        "png_interlaced.png",
        "png_interlaced_ref.png",
    );
}

#[test]
fn decode_tiff_deflate_matches_vips() {
    exact_row(
        "decode_tiff_deflate_matches_vips",
        None,
        sniffed,
        "tiff_deflate.tif",
        "tiff_deflate_ref.png",
    );
}

#[test]
fn decode_webp_lossless_matches_vips() {
    exact_row(
        "decode_webp_lossless_matches_vips",
        None,
        webp_dec,
        "webp_lossless.webp",
        "webp_lossless_ref.png",
    );
}

#[test]
fn decode_gif_matches_vips() {
    exact_row(
        "decode_gif_matches_vips",
        None,
        gif_dec,
        "gif.gif",
        "gif_ref.png",
    );
}

#[test]
fn decode_ppm_matches_vips() {
    exact_row(
        "decode_ppm_matches_vips",
        None,
        sniffed,
        "ppm.ppm",
        "ppm_ref.png",
    );
}

#[test]
fn decode_vips_native_matches_vips() {
    exact_row(
        "decode_vips_native_matches_vips",
        None,
        sniffed,
        "vips.v",
        "vips_ref.png",
    );
}

#[test]
fn decode_fits_matches_vips() {
    exact_row(
        "decode_fits_matches_vips",
        None,
        fits_dec,
        "fits.fits",
        "fits_ref.png",
    );
}

#[test]
fn decode_mat_matches_vips() {
    exact_row(
        "decode_mat_matches_vips",
        None,
        mat_dec,
        "mat.mat",
        "mat_ref.png",
    );
}

#[test]
fn decode_analyze_matches_vips() {
    exact_row(
        "decode_analyze_matches_vips",
        None,
        analyze_dec,
        "analyze.hdr",
        "analyze_ref.png",
    );
}

/// vips 8.18 has no niftiload in any build, so the reference is the voxels
/// nibabel was handed: `src_gray.png`, x-fastest, no flip.
#[test]
fn decode_nifti_matches_nibabel_voxels() {
    exact_row(
        "decode_nifti_matches_nibabel_voxels",
        None,
        nifti_dec,
        "nifti.nii",
        "src_gray.png",
    );
}

/// Float rows are exact too: RGBE to float and half to float are both
/// fully specified conversions, so two correct readers agree to the bit.
#[test]
fn decode_radiance_matches_vips() {
    decode_row(
        "decode_radiance_matches_vips",
        None,
        rad_dec,
        "rad.hdr",
        &Img::from_v("rad_ref.v"),
        EXACT,
    );
}

/// vips's openexrload always answers four bands; the file oiiotool wrote is
/// RGB, so vips's alpha is the constant 1.0 and is checked and dropped.
#[test]
fn decode_exr_matches_vips() {
    let cell = "decode_exr_matches_vips";
    let want = Img::from_v("exr_half_ref.v");
    let Some(r) = decode_for(cell, None, exr_dec, "exr_half.exr") else {
        return;
    };
    let got = Img::from_raster(&r);
    let want = if got.bands == 3 {
        want.without_constant_bands(3, 1.0)
    } else {
        want
    };
    assert_close(cell, &got, &want, EXACT);
}

#[test]
fn decode_jxl_lossless_matches_vips() {
    exact_row(
        "decode_jxl_lossless_matches_vips",
        Some("jxl"),
        jxl_dec,
        "jxl_lossless.jxl",
        "jxl_lossless_ref.png",
    );
}

#[test]
fn decode_jp2k_lossless_matches_vips() {
    exact_row(
        "decode_jp2k_lossless_matches_vips",
        Some("jp2k"),
        jp2k_dec,
        "jp2k_lossless.jp2",
        "jp2k_lossless_ref.png",
    );
}

/// The SVG embeds `src_rgb.png` at 1:1, so the rasteriser's only job is an
/// identity blit and resvg and librsvg have nothing to disagree about.
#[test]
fn decode_svg_matches_vips() {
    exact_row(
        "decode_svg_matches_vips",
        Some("svg"),
        svg_dec,
        "svg.svg",
        "svg_ref.png",
    );
}

// ---------------------------------------------------------------------------
// Lossy decode rows, each with its tolerance and its control
// ---------------------------------------------------------------------------

/// One lossy codec: the file at quality Q and at Q-1, both written by vips,
/// and the tolerance libviprs's decode of Q must sit inside against vips's
/// decode of Q and outside against vips's decode of Q-1.
struct Lossy {
    feature: Option<&'static str>,
    decode: Decoder,
    q: &'static str,
    q_ref: &'static str,
    neighbour_ref: &'static str,
    tol: Tol,
}

fn lossy_row(cell: &str, l: &Lossy) {
    decode_row(
        cell,
        l.feature,
        l.decode,
        l.q,
        &Img::from_png(l.q_ref),
        l.tol,
    );
}

fn lossy_control(cell: &str, l: &Lossy) {
    let Some(r) = decode_for(cell, l.feature, l.decode, l.q) else {
        return;
    };
    assert_rejects(
        cell,
        &Img::from_raster(&r),
        &Img::from_png(l.neighbour_ref),
        l.tol,
    );
}

/// JPEG at Q76 with 4:4:4 chroma, so no upsampling filter is involved and the
/// only freedom two decoders have is IDCT rounding. Measured on the native
/// x86_64 oracle: 2337 of 196608 samples differ, by at most 2, mean 0.0129.
/// The control (against vips's Q75) measures max 3, mean 0.407.
///
/// Q76 against Q75, because on this smooth source Q75 and Q74 decode to
/// identical pixels (the quantiser entries that differ are ones the image
/// never uses), and a control has to compare two different images.
const JPEG_444: Lossy = Lossy {
    feature: None,
    decode: sniffed,
    q: "jpeg_q76_444.jpg",
    q_ref: "jpeg_q76_444_ref.png",
    neighbour_ref: "jpeg_q75_444_ref.png",
    tol: Tol {
        max_abs: 2.0,
        mean_abs: 0.03,
    },
};

/// JPEG at Q76 with 4:2:0 chroma. libjpeg-turbo's fancy upsampling and the
/// core's decoder may round the interpolated chroma differently, so the bound
/// is wider than 4:4:4's. Measured: 24385 of 196608 samples differ, by at most
/// 4, mean 0.157. The control measures max 4, mean 0.492, so here it is the
/// mean bound that does the catching.
const JPEG_420: Lossy = Lossy {
    feature: None,
    decode: sniffed,
    q: "jpeg_q76_420.jpg",
    q_ref: "jpeg_q76_420_ref.png",
    neighbour_ref: "jpeg_q75_420_ref.png",
    tol: Tol {
        max_abs: 4.0,
        mean_abs: 0.25,
    },
};

/// Lossy WebP at Q75. VP8 reconstruction is bit-exact by specification and
/// the core's chroma upsampler matches libwebp's: measured exact, so the
/// tolerance is [`EXACT`]. The control measures max 12, mean 0.895.
const WEBP_LOSSY: Lossy = Lossy {
    feature: None,
    decode: webp_dec,
    q: "webp_q75.webp",
    q_ref: "webp_q75_ref.png",
    neighbour_ref: "webp_q74_ref.png",
    tol: EXACT,
};

/// Lossy JPEG XL at Q75 (VarDCT). The decoder works in float, and libjxl and
/// jxl-oxide may round the final 8-bit store differently by one level.
/// Measured: 38815 of 196608 samples differ, every one by exactly 1, mean
/// 0.197. The control measures max 41, mean 1.030.
const JXL_LOSSY: Lossy = Lossy {
    feature: Some("jxl"),
    decode: jxl_dec,
    q: "jxl_q75.jxl",
    q_ref: "jxl_q75_ref.png",
    neighbour_ref: "jxl_q74_ref.png",
    tol: Tol {
        max_abs: 1.0,
        mean_abs: 0.3,
    },
};

/// Lossy JPEG 2000 at Q45 (the 9/7 irreversible wavelet). Float wavelet, one
/// level of rounding latitude. Measured: 5 of 196608 samples differ, by 1,
/// mean 0.00003. The control measures max 11, mean 0.0082, so the mean bound
/// sits well under it as well as the max.
const JP2K_LOSSY: Lossy = Lossy {
    feature: Some("jp2k"),
    decode: jp2k_dec,
    q: "jp2k_q45.jp2",
    q_ref: "jp2k_q45_ref.png",
    neighbour_ref: "jp2k_q44_ref.png",
    tol: Tol {
        max_abs: 1.0,
        mean_abs: 0.001,
    },
};

/// AVIF at Q76, 4:4:4. AV1 reconstruction is bit-exact by specification and
/// the core's YCbCr to RGB step matches libheif's: measured exact, so the
/// tolerance is [`EXACT`]. The control measures max 4, mean 0.313. Q76
/// against Q75 rather than Q75 against Q74, because libheif writes the same
/// file at 74 and 75 (one quantiser for both).
const AVIF_LOSSY: Lossy = Lossy {
    feature: Some("avif"),
    decode: avif_dec,
    q: "avif_q76.avif",
    q_ref: "avif_q76_ref.png",
    neighbour_ref: "avif_q75_ref.png",
    tol: EXACT,
};

#[test]
fn decode_jpeg_444_within_tolerance_of_vips() {
    lossy_row("decode_jpeg_444_within_tolerance_of_vips", &JPEG_444);
}
#[test]
fn decode_jpeg_444_tolerance_catches_one_quality_step() {
    lossy_control(
        "decode_jpeg_444_tolerance_catches_one_quality_step",
        &JPEG_444,
    );
}
#[test]
fn decode_jpeg_420_within_tolerance_of_vips() {
    lossy_row("decode_jpeg_420_within_tolerance_of_vips", &JPEG_420);
}
#[test]
fn decode_jpeg_420_tolerance_catches_one_quality_step() {
    lossy_control(
        "decode_jpeg_420_tolerance_catches_one_quality_step",
        &JPEG_420,
    );
}
#[test]
fn decode_webp_lossy_within_tolerance_of_vips() {
    lossy_row("decode_webp_lossy_within_tolerance_of_vips", &WEBP_LOSSY);
}
#[test]
fn decode_webp_lossy_tolerance_catches_one_quality_step() {
    lossy_control(
        "decode_webp_lossy_tolerance_catches_one_quality_step",
        &WEBP_LOSSY,
    );
}
#[test]
fn decode_jxl_lossy_within_tolerance_of_vips() {
    lossy_row("decode_jxl_lossy_within_tolerance_of_vips", &JXL_LOSSY);
}
#[test]
fn decode_jxl_lossy_tolerance_catches_one_quality_step() {
    lossy_control(
        "decode_jxl_lossy_tolerance_catches_one_quality_step",
        &JXL_LOSSY,
    );
}
#[test]
fn decode_jp2k_lossy_within_tolerance_of_vips() {
    lossy_row("decode_jp2k_lossy_within_tolerance_of_vips", &JP2K_LOSSY);
}
#[test]
fn decode_jp2k_lossy_tolerance_catches_one_quality_step() {
    lossy_control(
        "decode_jp2k_lossy_tolerance_catches_one_quality_step",
        &JP2K_LOSSY,
    );
}
#[test]
fn decode_avif_within_tolerance_of_vips() {
    lossy_row("decode_avif_within_tolerance_of_vips", &AVIF_LOSSY);
}
#[test]
fn decode_avif_tolerance_catches_one_quality_step() {
    lossy_control(
        "decode_avif_tolerance_catches_one_quality_step",
        &AVIF_LOSSY,
    );
}

// ---------------------------------------------------------------------------
// Encode rows: libviprs writes, vips reads
// ---------------------------------------------------------------------------

fn src_rgb() -> Raster {
    decode_bytes_with_limits(&bytes("src_rgb.png"), DecodeLimits::default()).expect("src_rgb.png")
}

fn src_rgba() -> Raster {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canonical_input.png");
    let b = std::fs::read(&path).expect("canonical_input.png");
    decode_bytes_with_limits(&b, DecodeLimits::default()).expect("canonical_input.png")
}

fn src_float() -> Raster {
    decode_bytes_with_limits(&bytes("src_float.v"), DecodeLimits::default()).expect("src_float.v")
}

type Encoder = fn(&Raster) -> Result<Vec<u8>, libviprs::EncodeError>;

/// One libviprs encoder, the file it wrote when the fixtures were made, and
/// vips's decode of that file.
struct Enc {
    feature: Option<&'static str>,
    source: fn() -> Raster,
    encode: Encoder,
    decode: Decoder,
    file: &'static str,
    vips: &'static str,
}

fn enc_png(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_png(6)
}
fn enc_tiff(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    Ok(r.tiff_save())
}
fn enc_webp(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_webp(libviprs::webp::SaveOptions::default())
}
fn enc_gif(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_gif(libviprs::gif::SaveOptions::default())
}
fn enc_jxl(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_jxl(libviprs::jxl::SaveOptions::default())
}
fn enc_jp2k(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_jp2k(libviprs::jp2k::SaveOptions::default())
}
fn enc_rad(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_radiance(libviprs::radiance::SaveOptions::default())
}
fn enc_fits(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_fits()
}
fn enc_ppm(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_ppm()
}
fn enc_vips(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_vips().map_err(libviprs::EncodeError::encode)
}
fn enc_jpeg76(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_jpeg(76)
}
fn enc_jpeg75(r: &Raster) -> Result<Vec<u8>, libviprs::EncodeError> {
    r.encode_jpeg(75)
}

const ENC_PNG: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_png,
    decode: sniffed,
    file: "enc/libviprs_png.png",
    vips: "enc/libviprs_png_vips.png",
};
const ENC_TIFF: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_tiff,
    decode: sniffed,
    file: "enc/libviprs_tiff.tif",
    vips: "enc/libviprs_tiff_vips.png",
};
const ENC_WEBP: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_webp,
    decode: webp_dec,
    file: "enc/libviprs_webp.webp",
    vips: "enc/libviprs_webp_vips.png",
};
const ENC_GIF: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_gif,
    decode: gif_dec,
    file: "enc/libviprs_gif.gif",
    vips: "enc/libviprs_gif_vips.png",
};
const ENC_JXL: Enc = Enc {
    feature: Some("jxl"),
    source: src_rgb,
    encode: enc_jxl,
    decode: jxl_dec,
    file: "enc/libviprs_jxl.jxl",
    vips: "enc/libviprs_jxl_vips.png",
};
const ENC_JP2K: Enc = Enc {
    feature: Some("jp2k"),
    source: src_rgba,
    encode: enc_jp2k,
    decode: jp2k_dec,
    file: "enc/libviprs_jp2k.jp2",
    vips: "enc/libviprs_jp2k_vips.png",
};
const ENC_RAD: Enc = Enc {
    feature: None,
    source: src_float,
    encode: enc_rad,
    decode: rad_dec,
    file: "enc/libviprs_rad.hdr",
    vips: "enc/libviprs_rad_vips.v",
};
const ENC_FITS: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_fits,
    decode: fits_dec,
    file: "enc/libviprs_fits.fits",
    vips: "enc/libviprs_fits_vips.png",
};
const ENC_PPM: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_ppm,
    decode: sniffed,
    file: "enc/libviprs_ppm.ppm",
    vips: "enc/libviprs_ppm_vips.png",
};
const ENC_VIPS: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_vips,
    decode: sniffed,
    file: "enc/libviprs_native.v",
    vips: "enc/libviprs_native_vips.png",
};
const ENC_JPEG76: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_jpeg76,
    decode: sniffed,
    file: "enc/libviprs_jpeg_q76.jpg",
    vips: "enc/libviprs_jpeg_q76_vips.png",
};
const ENC_JPEG75: Enc = Enc {
    feature: None,
    source: src_rgb,
    encode: enc_jpeg75,
    decode: sniffed,
    file: "enc/libviprs_jpeg_q75.jpg",
    vips: "enc/libviprs_jpeg_q75_vips.png",
};

const ALL_ENCODERS: &[&Enc] = &[
    &ENC_PNG,
    &ENC_TIFF,
    &ENC_WEBP,
    &ENC_GIF,
    &ENC_JXL,
    &ENC_JP2K,
    &ENC_RAD,
    &ENC_FITS,
    &ENC_PPM,
    &ENC_VIPS,
    &ENC_JPEG76,
    &ENC_JPEG75,
];

fn vips_ref(name: &str) -> Img {
    if name.ends_with(".v") {
        Img::from_v(name)
    } else {
        Img::from_png(name)
    }
}

/// Encode now, or `None` when the cell skipped for a missing feature.
fn encode_for(cell: &str, e: &Enc) -> Option<Vec<u8>> {
    match (e.encode)(&(e.source)()) {
        Ok(b) => Some(b),
        Err(err) if e.feature.is_some_and(|f| is_encode_refusal(f, &err)) => {
            skip(e.feature.unwrap(), cell);
            None
        }
        Err(err) => panic!("{cell}: encoding failed: {err}"),
    }
}

/// A lossless encoder: today's bytes are the committed bytes (so vips's
/// committed decode still speaks for today's encoder), vips read those bytes
/// back as exactly the source, and libviprs reads today's bytes back as
/// exactly what vips read.
fn lossless_encode_row(cell: &str, e: &Enc) {
    let Some(fresh) = encode_for(cell, e) else {
        return;
    };
    assert!(
        fresh == bytes(e.file),
        "{cell}: the encoder no longer writes the committed {} ({} bytes now, {} committed), \
         so vips's decode of it no longer speaks for this encoder. If the change is intended, \
         regenerate with the codec passes of tools/gen_fixtures.sh",
        e.file,
        fresh.len(),
        bytes(e.file).len()
    );
    let vips = vips_ref(e.vips);
    let source = Img::from_raster(&(e.source)());
    assert_close(
        &format!("{cell} (vips decode vs source)"),
        &vips,
        &source,
        EXACT,
    );
    let back = Img::from_raster(&(e.decode)(&fresh, DecodeLimits::default()).expect("decode"));
    assert_close(
        &format!("{cell} (libviprs decode vs vips decode)"),
        &back,
        &vips,
        EXACT,
    );
}

#[test]
fn encode_png_read_back_by_vips() {
    lossless_encode_row("encode_png_read_back_by_vips", &ENC_PNG);
}
#[test]
fn encode_tiff_read_back_by_vips() {
    lossless_encode_row("encode_tiff_read_back_by_vips", &ENC_TIFF);
}
#[test]
fn encode_webp_read_back_by_vips() {
    lossless_encode_row("encode_webp_read_back_by_vips", &ENC_WEBP);
}
#[test]
fn encode_fits_read_back_by_vips() {
    lossless_encode_row("encode_fits_read_back_by_vips", &ENC_FITS);
}
#[test]
fn encode_ppm_read_back_by_vips() {
    lossless_encode_row("encode_ppm_read_back_by_vips", &ENC_PPM);
}
#[test]
fn encode_vips_native_read_back_by_vips() {
    lossless_encode_row("encode_vips_native_read_back_by_vips", &ENC_VIPS);
}
#[test]
fn encode_jxl_read_back_by_vips() {
    lossless_encode_row("encode_jxl_read_back_by_vips", &ENC_JXL);
}
#[test]
fn encode_jp2k_read_back_by_vips() {
    lossless_encode_row("encode_jp2k_read_back_by_vips", &ENC_JP2K);
}

/// The core's JPEG 2000 encoder is a port of openjpeg 2.5.4, and the oracle
/// links exactly that openjpeg, so at `jp2ksave`'s defaults the two must
/// write the same file, byte for byte.
#[test]
fn encode_jp2k_is_byte_identical_to_vips_jp2ksave() {
    let cell = "encode_jp2k_is_byte_identical_to_vips_jp2ksave";
    let Some(fresh) = encode_for(cell, &ENC_JP2K) else {
        return;
    };
    let vips = bytes("jp2k_lossless.jp2");
    let first = fresh.iter().zip(&vips).position(|(a, b)| a != b);
    assert!(
        fresh == vips,
        "{cell}: {} bytes from libviprs, {} from vips jp2ksave --lossless, first difference at {first:?}",
        fresh.len(),
        vips.len()
    );
}

/// Radiance is lossy by construction (RGBE shares one exponent), so the
/// source cannot come back exactly; what has to hold exactly is that vips and
/// libviprs read the same RGBE file to the same floats, and that the encode
/// itself is vips's: within one mantissa step (1/256) of the source, and a
/// mean of at most half that (measured 0.00111).
#[test]
fn encode_radiance_read_back_by_vips() {
    let cell = "encode_radiance_read_back_by_vips";
    let fresh = encode_for(cell, &ENC_RAD).expect("radiance has no feature");
    assert!(
        fresh == bytes(ENC_RAD.file),
        "{cell}: encoder output moved off the committed file"
    );
    let vips = Img::from_v(ENC_RAD.vips);
    let back = Img::from_raster(&rad_dec(&fresh, DecodeLimits::default()).expect("decode"));
    assert_close(
        &format!("{cell} (libviprs decode vs vips decode)"),
        &back,
        &vips,
        EXACT,
    );
    let source = Img::from_raster(&src_float());
    assert_close(
        &format!("{cell} (vips decode vs source)"),
        &vips,
        &source,
        Tol {
            max_abs: 1.0 / 256.0,
            mean_abs: 1.0 / 512.0,
        },
    );
}

/// GIF quantises, so the source does not come back exactly either. vips and
/// libviprs reading the same GIF have to agree to the sample, and the
/// quantised image has to stay near the source on average. No per-sample
/// bound: error diffusion moves single samples a long way on purpose (107
/// levels at worst here), and the mean is what says the palette is right.
#[test]
fn encode_gif_read_back_by_vips() {
    let cell = "encode_gif_read_back_by_vips";
    let fresh = encode_for(cell, &ENC_GIF).expect("gif has no feature");
    assert!(
        fresh == bytes(ENC_GIF.file),
        "{cell}: encoder output moved off the committed file"
    );
    let vips = Img::from_png(ENC_GIF.vips);
    let back = Img::from_raster(&gif_dec(&fresh, DecodeLimits::default()).expect("decode"));
    assert_close(
        &format!("{cell} (libviprs decode vs vips decode)"),
        &back,
        &vips,
        EXACT,
    );
    let source = Img::from_raster(&src_rgb());
    assert_close(
        &format!("{cell} (vips decode vs source)"),
        &vips,
        &source,
        Tol {
            max_abs: 255.0,
            mean_abs: 4.0,
        },
    );
}

/// The libviprs JPEG at Q76, read by vips and by libviprs (Q76 and Q75 for the
/// control, for the reason given on [`JPEG_444`]). Not compared byte
/// for byte with a fresh encode: the core's JPEG encoder is replaced between
/// the suite's pinned core and core main (core #1132), and the decode check
/// is what this cell is about. The fresh encode is held to the source instead.
/// The file is 4:2:0 (`JpegSubsample::Auto` below Q90), so the bound is
/// 4:2:0's. Measured: 25313 of 196608 samples differ, by at most 4, mean
/// 0.163; the control measures max 4, mean 0.536.
const ENC_JPEG_TOL: Tol = Tol {
    max_abs: 4.0,
    mean_abs: 0.25,
};

#[test]
fn encode_jpeg_read_back_by_vips() {
    let cell = "encode_jpeg_read_back_by_vips";
    let committed = Img::from_raster(
        &sniffed(&bytes(ENC_JPEG76.file), DecodeLimits::default()).expect("decode"),
    );
    assert_close(
        cell,
        &committed,
        &Img::from_png(ENC_JPEG76.vips),
        ENC_JPEG_TOL,
    );
    // The fresh encode against the source: no per-sample bound, because the
    // canonical's hard quadrant edges ring at any quality (97 levels at worst
    // here), and a mean that a broken quantiser or colour transform would blow
    // straight through (measured 0.916).
    let fresh = encode_for(cell, &ENC_JPEG76).expect("jpeg has no feature");
    let back = Img::from_raster(&sniffed(&fresh, DecodeLimits::default()).expect("decode"));
    assert_close(
        &format!("{cell} (fresh Q76 encode vs source)"),
        &back,
        &Img::from_raster(&src_rgb()),
        Tol {
            max_abs: 255.0,
            mean_abs: 2.0,
        },
    );
}

#[test]
fn encode_jpeg_tolerance_catches_one_quality_step() {
    let cell = "encode_jpeg_tolerance_catches_one_quality_step";
    let committed = Img::from_raster(
        &sniffed(&bytes(ENC_JPEG76.file), DecodeLimits::default()).expect("decode"),
    );
    assert_rejects(
        cell,
        &committed,
        &Img::from_png(ENC_JPEG75.vips),
        ENC_JPEG_TOL,
    );
}

/// Writes `enc/libviprs_*` from the committed sources. Run once, between the
/// two codec passes of tools/gen_fixtures.sh, with `jxl` and `jp2k` on.
#[test]
#[ignore = "fixture generator, run by hand"]
fn generate_libviprs_encodes() {
    std::fs::create_dir_all(fx("enc")).unwrap();
    for e in ALL_ENCODERS {
        let out = (e.encode)(&(e.source)()).unwrap_or_else(|err| panic!("{}: {err}", e.file));
        std::fs::write(fx(e.file), out).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Decode limits
// ---------------------------------------------------------------------------

/// The typed refusals a limit is allowed to come back as. An allocation
/// failure, a panic, or an untyped decode error is not one of them.
fn is_limit_refusal(e: &SourceError) -> bool {
    e.is_alloc_limit()
        || matches!(
            e,
            SourceError::DimensionLimitExceeded { .. }
                | SourceError::CoordLimitExceeded { .. }
                | SourceError::AllocLimitExceeded { .. }
                | SourceError::PageLimitExceeded { .. }
        )
}

/// Every file the limits cells decode is 256 x 256. The allocation ceiling is
/// probed downward from the decoded raster's own size rather than assumed,
/// because several decoders price more than one buffer (DecodeLimits'
/// `max_alloc_bytes` doc); what is pinned is that some ceiling refuses with a
/// typed variant and the default accepts.
fn limits_row(cell: &str, feature: Option<&str>, decode: Decoder, file: &str) {
    let b = bytes(file);
    let Some(r) = decode_for(cell, feature, decode, file) else {
        return;
    };
    let (w, h) = (r.width(), r.height());
    let px = u64::from(w) * u64::from(h);
    let at = |l: DecodeLimits| decode(&b, l);
    let mut problems = Vec::new();
    let mut refused = |what: &str, l: DecodeLimits| match at(l) {
        Ok(_) => problems.push(format!("{what} one below the {w}x{h} file still decoded")),
        Err(e) if !is_limit_refusal(&e) => {
            problems.push(format!("{what} refused with an untyped error: {e:?}"));
        }
        Err(_) => {}
    };
    let d = DecodeLimits::default();
    refused("max_coord", d.with_max_coord(w.max(h) - 1));
    refused("max_pixels", d.with_max_pixels(px - 1));
    refused(
        "max_alloc_bytes",
        d.with_max_alloc_bytes(r.data().len() as u64 - 1),
    );
    for (what, l) in [
        ("max_coord", d.with_max_coord(w.max(h))),
        ("max_pixels", d.with_max_pixels(px)),
    ] {
        if let Err(e) = at(l) {
            problems.push(format!(
                "{what} exactly at the {w}x{h} file refused it: {e}"
            ));
        }
    }
    assert!(problems.is_empty(), "{cell}:\n  {}", problems.join("\n  "));
}

#[test]
fn limits_png() {
    limits_row("limits_png", None, sniffed, "png_interlaced.png");
}
#[test]
fn limits_jpeg() {
    limits_row("limits_jpeg", None, sniffed, "jpeg_q76_444.jpg");
}
#[test]
fn limits_tiff() {
    limits_row("limits_tiff", None, sniffed, "tiff_deflate.tif");
}
#[test]
fn limits_webp() {
    limits_row("limits_webp", None, webp_dec, "webp_lossless.webp");
}
#[test]
fn limits_gif() {
    limits_row("limits_gif", None, gif_dec, "gif.gif");
}
/// Red against the core: `decode_netpbm` (core `src/textio.rs`) checks only
/// `check_image_alloc`, never `check_coord` or `check_pixels`, so a Netpbm
/// file decodes straight through a `max_coord` or `max_pixels` ceiling it is
/// over. Ignored until the core fix lands and the pin moves to it; run it with
/// `--ignored` to see it fail.
#[test]
#[ignore = "core defect: the Netpbm decoder ignores DecodeLimits::max_coord and max_pixels (the core tracking issue)"]
fn limits_ppm() {
    limits_row("limits_ppm", None, sniffed, "ppm.ppm");
}
/// The live half of the parked `limits_ppm`: it pins the defect as it is
/// today, so the day the core starts honouring `max_coord` and `max_pixels`
/// for Netpbm this goes red and says to un-ignore `limits_ppm` and delete it.
#[test]
fn limits_ppm_defect_is_still_there() {
    let b = bytes("ppm.ppm");
    let r = sniffed(&b, DecodeLimits::default()).expect("ppm.ppm decodes at the defaults");
    let (w, h) = (r.width(), r.height());
    let d = DecodeLimits::default();
    for (what, l) in [
        ("max_coord", d.with_max_coord(w.max(h) - 1)),
        (
            "max_pixels",
            d.with_max_pixels(u64::from(w) * u64::from(h) - 1),
        ),
    ] {
        assert!(
            sniffed(&b, l).is_ok(),
            "the Netpbm decoder now refuses {what} one below the {w}x{h} file, \
             so the core fixed the defect on the core tracking issue: un-ignore \
             limits_ppm and delete this cell"
        );
    }
}
#[test]
fn limits_vips_native() {
    limits_row("limits_vips_native", None, sniffed, "vips.v");
}
#[test]
fn limits_fits() {
    limits_row("limits_fits", None, fits_dec, "fits.fits");
}
#[test]
fn limits_mat() {
    limits_row("limits_mat", None, mat_dec, "mat.mat");
}
#[test]
fn limits_analyze() {
    limits_row("limits_analyze", None, analyze_dec, "analyze.hdr");
}
#[test]
fn limits_nifti() {
    limits_row("limits_nifti", None, nifti_dec, "nifti.nii");
}
#[test]
fn limits_radiance() {
    limits_row("limits_radiance", None, rad_dec, "rad.hdr");
}
#[test]
fn limits_exr() {
    limits_row("limits_exr", None, exr_dec, "exr_half.exr");
}
#[test]
fn limits_jxl() {
    limits_row("limits_jxl", Some("jxl"), jxl_dec, "jxl_lossless.jxl");
}
#[test]
fn limits_jp2k() {
    limits_row("limits_jp2k", Some("jp2k"), jp2k_dec, "jp2k_lossless.jp2");
}
#[test]
fn limits_avif() {
    limits_row("limits_avif", Some("avif"), avif_dec, "avif_q75.avif");
}
/// Red against the core: the SVG rasteriser (core `src/svg.rs`) checks
/// `max_coord` and `max_pixels` and then allocates the pixmap and its
/// demultiplied copy without asking `max_alloc_bytes`, so a caller's
/// allocation budget does not bound an SVG decode. Ignored until the core fix
/// lands and the pin moves to it; run it with `--ignored` to see it fail.
#[test]
#[ignore = "core defect: the SVG rasteriser ignores DecodeLimits::max_alloc_bytes (the core tracking issue)"]
fn limits_svg() {
    limits_row("limits_svg", Some("svg"), svg_dec, "svg.svg");
}
/// The live half of the parked `limits_svg`, the same way round as
/// `limits_ppm_defect_is_still_there`: an allocation budget one byte under the
/// decoded raster still renders, until the core prices the pixmap.
#[test]
fn limits_svg_defect_is_still_there() {
    let Some(r) = decode_for(
        "limits_svg_defect_is_still_there",
        Some("svg"),
        svg_dec,
        "svg.svg",
    ) else {
        return;
    };
    let tight = DecodeLimits::default().with_max_alloc_bytes(r.data().len() as u64 - 1);
    assert!(
        svg_dec(&bytes("svg.svg"), tight).is_ok(),
        "the SVG rasteriser now refuses an allocation budget one byte under its \
         raster, so the core fixed the defect on the core tracking issue: \
         un-ignore limits_svg and delete this cell"
    );
}

// ---------------------------------------------------------------------------
// Multi-page and multi-frame
// ---------------------------------------------------------------------------

const PAGES: u32 = 3;

#[test]
fn pages_tiff() {
    let path = fx("tiff_pages.tif");
    assert_eq!(libviprs::tiff_page_count(&path).expect("page count"), PAGES);
    for p in 0..PAGES {
        let r = libviprs::decode_tiff_page(&path, p).unwrap_or_else(|e| panic!("page {p}: {e}"));
        let reference = format!("tiff_pages_p{p}_ref.png");
        assert_close(
            &format!("pages_tiff page {p}"),
            &Img::from_raster(&r),
            &Img::from_png(&reference),
            EXACT,
        );
    }
    let limited = libviprs::tiff_page_count_with_limits(
        &path,
        DecodeLimits::default().with_max_pages(PAGES - 1),
    );
    assert!(
        matches!(limited, Err(SourceError::PageLimitExceeded { .. })),
        "pages_tiff: a {PAGES}-page file under max_pages {} answered {limited:?}",
        PAGES - 1
    );
}

#[test]
fn pages_gif() {
    let b = bytes("gif_pages.gif");
    let all = libviprs::decode_gif_with(
        &b,
        DecodeLimits::default(),
        libviprs::gif::LoadOptions::default().with_n(-1),
    )
    .expect("every frame");
    assert_eq!(
        all.get_n_pages(),
        PAGES,
        "pages_gif: n-pages of the whole file"
    );
    assert_eq!(
        all.height(),
        256 * PAGES,
        "pages_gif: frames stacked into one raster"
    );
    for p in 0..PAGES {
        let r = libviprs::decode_gif_with(
            &b,
            DecodeLimits::default(),
            libviprs::gif::LoadOptions::default().with_page(p),
        )
        .unwrap_or_else(|e| panic!("frame {p}: {e}"));
        let reference = format!("gif_pages_p{p}_ref.png");
        assert_close(
            &format!("pages_gif frame {p}"),
            &Img::from_raster(&r),
            &Img::from_png(&reference),
            EXACT,
        );
    }
}

#[test]
fn pages_webp() {
    let b = bytes("webp_pages.webp");
    let all = libviprs::decode_webp_with(
        &b,
        DecodeLimits::default(),
        libviprs::webp::LoadOptions::default().with_n(-1),
    )
    .expect("every frame");
    assert_eq!(
        all.get_n_pages(),
        PAGES,
        "pages_webp: n-pages of the whole file"
    );
    assert_eq!(
        all.height(),
        256 * PAGES,
        "pages_webp: frames stacked into one raster"
    );
    for p in 0..PAGES {
        let r = libviprs::decode_webp_with(
            &b,
            DecodeLimits::default(),
            libviprs::webp::LoadOptions::default().with_page(p),
        )
        .unwrap_or_else(|e| panic!("frame {p}: {e}"));
        let reference = format!("webp_pages_p{p}_ref.png");
        assert_close(
            &format!("pages_webp frame {p}"),
            &Img::from_raster(&r),
            &Img::from_png(&reference),
            EXACT,
        );
    }
}
