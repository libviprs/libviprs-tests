//! Loaders for the canonical fixture set committed under `tests/fixtures/`.
//!
//! These helpers replace the ~38 per-file `gradient_raster` / `solid_raster`
//! definitions that historically scattered ad-hoc input synthesis across
//! the test suite. The canonical input PNG (`canonical_input.png`) is a
//! checked-in 256×256 RGBA8 four-quadrant gradient; we strip alpha and
//! resize on demand for tests that need other dimensions.
//!
//! # When to use which helper
//!
//! - [`canonical_raster_scaled`] is the right call for any test that
//!   previously called `gradient_raster(w, h)` or `synthetic_raster(w, h)`.
//!   The pixel values change with `(w, h)` but are deterministic per-dim,
//!   which is exactly what those tests needed (a non-uniform multi-channel
//!   input that hashes to a stable value).
//!
//! - [`canonical_solid`] is a uniform-fill helper that intentionally stays
//!   synthetic. Tests that exercise blank-tile detection / placeholder
//!   strategies need uniform input by definition; constructing it from a
//!   PNG would just round-trip a `vec![v; n]`. The helper is centralised
//!   here so the suite has a single source of truth, not because it loads
//!   a fixture.

use std::path::PathBuf;
use std::sync::OnceLock;

use libviprs::{PixelFormat, Raster};

/// Path to `tests/fixtures/canonical_input.png`.
fn canonical_input_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canonical_input.png")
}

/// Decode `canonical_input.png` once per process and cache the RGBA8 bytes.
fn canonical_input_rgba8() -> &'static image::RgbaImage {
    static CACHE: OnceLock<image::RgbaImage> = OnceLock::new();
    CACHE.get_or_init(|| {
        image::open(canonical_input_path())
            .expect("read canonical_input.png")
            .to_rgba8()
    })
}

/// Load `canonical_input.png` as a 256×256 RGB8 [`Raster`] (alpha dropped).
pub fn canonical_raster() -> Raster {
    let rgba = canonical_input_rgba8();
    rgba_to_rgb8_raster(rgba)
}

/// Load `canonical_input.png` and nearest-neighbour resize to `w × h`,
/// returning an RGB8 [`Raster`]. Drop-in replacement for the old
/// `gradient_raster(w, h)` / `synthetic_raster(w, h)` helpers.
///
/// Nearest-neighbour rather than bilinear so the resize is exact and
/// reproducible even on aggressive upscales (no interpolation rounding to
/// chase across platforms). The output bytes change with `(w, h)` but are
/// deterministic per-dimension, which is what the invariant tests need.
pub fn canonical_raster_scaled(w: u32, h: u32) -> Raster {
    let rgba = canonical_input_rgba8();
    let resized = image::imageops::resize(rgba, w, h, image::imageops::FilterType::Nearest);
    rgba_to_rgb8_raster(&resized)
}

/// Uniform-fill RGB8 raster of size `w × h` where every channel is `val`.
///
/// **Intentionally synthetic.** Tests that exercise blank-tile detection
/// or placeholder strategies need uniform input by construction; loading
/// one from disk would just round-trip a `vec![val; …]`. Centralised here
/// so the suite has a single helper rather than 4-plus per-file copies.
pub fn canonical_solid(w: u32, h: u32, val: u8) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let data = vec![val; w as usize * h as usize * bpp];
    Raster::new(w, h, PixelFormat::Rgb8, data).expect("solid raster construction")
}

fn rgba_to_rgb8_raster(rgba: &image::RgbaImage) -> Raster {
    let (w, h) = rgba.dimensions();
    let mut data = Vec::with_capacity(w as usize * h as usize * 3);
    for pixel in rgba.pixels() {
        data.extend_from_slice(&pixel.0[..3]);
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).expect("RGB8 raster from canonical PNG")
}
