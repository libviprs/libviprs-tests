//! Behaviour-preserving performance guard for `Raster::colourspace`
//! (libviprs#284).
//!
//! ## The finding
//!
//! `try_colourspace` used to build a full-image `Vec<f64>` staging buffer
//! and call a `from_xyz` helper that returned a *freshly heap-allocated*
//! `Vec<f64>` for **every pixel**. On the crate's flagship colour entry
//! point that is one heap allocation per pixel plus a whole extra
//! full-image f64 vector — a 100 MP image paid 100M pixel-sized
//! allocations and staged a 2.4 GB f64 vector before writing the output.
//!
//! The fix (`from_xyz_into` writing into a reused `[f64; 4]` scratch, the
//! conversion streaming straight into the output byte buffer) is
//! **behaviour-preserving**: the output bytes are identical, so this file
//! cannot pin the fix with an output-equality assertion alone — that
//! assertion is green on both the pristine and the fixed core (the
//! existing vips-reference colour suite, `ported_colour.rs`, is the
//! byte-identity oracle and stays green). What *does* change, and what
//! this file pins, is the **allocation behaviour**: a counting global
//! allocator proves the per-pixel `Vec` is gone.
//!
//! Against the pristine core the per-pixel `Vec<f64>` makes the allocation
//! count during a conversion scale with the pixel count (one allocation
//! per pixel), so [`colourspace_makes_no_per_pixel_allocations`] is RED
//! (≥ 16384 allocations for the 128×128 image below). After the fix the
//! conversion allocates only the output buffer and a couple of small
//! reused scratch vectors — a handful, independent of pixel count — so the
//! test is GREEN.
//!
//! The remaining tests are behaviour-preservation guards against external
//! references (sRGB white → Lab / XYZ) and the extra-band carry-through
//! index arithmetic the streaming rewrite had to reproduce exactly.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use libviprs::{Interpretation, Raster};

// ---------------------------------------------------------------------------
// Counting global allocator
// ---------------------------------------------------------------------------

/// Process-wide count of allocation events (alloc / zeroed / realloc).
/// A `#[global_allocator]` is scoped to this integration-test binary, so
/// it does not perturb any other test crate.
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Run `f`, returning its result and the number of allocation events that
/// occurred during the call.
fn count_allocs<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = ALLOCS.load(Ordering::Relaxed);
    let out = f();
    let after = ALLOCS.load(Ordering::Relaxed);
    (out, after.wrapping_sub(before))
}

// ---------------------------------------------------------------------------
// The per-pixel-allocation guard (RED against pristine core)
// ---------------------------------------------------------------------------

/// A colourspace conversion must not allocate once per pixel.
///
/// The image is 128×128 = 16384 pixels. Against the pristine core the
/// per-pixel `from_xyz` `Vec<f64>` makes the allocation count during the
/// conversion ≥ 16384; the fixed core allocates only the output buffer and
/// a couple of reused scratch vectors — a small constant independent of
/// the pixel count. The 1024 bound sits an order of magnitude below the
/// per-pixel floor, so it is unambiguously RED before the fix and GREEN
/// after, with headroom against any incidental allocation from the
/// parallel test runner.
#[test]
fn colourspace_makes_no_per_pixel_allocations() {
    const W: u32 = 128;
    const H: u32 = 128;
    let pixels = (W * H) as usize;

    // Build the input outside the measured region.
    let src = Raster::constant(W, H, &[200.0, 120.0, 40.0], Interpretation::Srgb);

    let (out, allocs) = count_allocs(|| src.colourspace(Interpretation::Lab));

    // Sanity: the conversion actually produced the expected raster.
    assert_eq!(out.width(), W);
    assert_eq!(out.height(), H);

    assert!(
        allocs < 1024,
        "colourspace(sRGB -> Lab) on {pixels} pixels made {allocs} allocations; \
         a per-pixel Vec allocates once per pixel (>= {pixels}). The conversion \
         must stream into the output buffer with reused scratch (libviprs#284)."
    );
}

/// The same guard through the widest target space (CMYK, four colour
/// bands): it exercises the full `[f64; 4]` scratch slot in
/// `from_xyz_into` and must still be allocation-flat.
#[test]
fn colourspace_to_cmyk_makes_no_per_pixel_allocations() {
    const W: u32 = 128;
    const H: u32 = 128;
    let pixels = (W * H) as usize;

    let src = Raster::constant(W, H, &[200.0, 120.0, 40.0], Interpretation::Srgb);
    let (_out, allocs) = count_allocs(|| src.colourspace(Interpretation::Cmyk));

    assert!(
        allocs < 1024,
        "colourspace(sRGB -> CMYK) on {pixels} pixels made {allocs} allocations; \
         expected a small constant independent of pixel count (libviprs#284)."
    );
}

// ---------------------------------------------------------------------------
// Behaviour-preservation guards (GREEN on both cores)
// ---------------------------------------------------------------------------

/// sRGB white converts to Lab white against an external reference:
/// L≈100, a≈0, b≈0. Independent of libvips, this pins that the streaming
/// rewrite still computes the correct colour, not merely a stable one.
#[test]
fn srgb_white_to_lab_reference() {
    let src = Raster::constant(4, 4, &[255.0, 255.0, 255.0], Interpretation::Srgb);
    let lab = src.colourspace(Interpretation::Lab).getpoint(1, 1);
    assert!(
        (lab[0] - 100.0).abs() < 1e-2,
        "L should be ~100, got {}",
        lab[0]
    );
    assert!(lab[1].abs() < 5e-2, "a should be ~0, got {}", lab[1]);
    assert!(lab[2].abs() < 5e-2, "b should be ~0, got {}", lab[2]);
}

/// sRGB white converts to the D65 white point in XYZ (Y white = 100):
/// [95.047, 100, 108.883], the reference the module documents.
#[test]
fn srgb_white_to_xyz_reference() {
    let src = Raster::constant(4, 4, &[255.0, 255.0, 255.0], Interpretation::Srgb);
    let xyz = src.colourspace(Interpretation::Xyz).getpoint(2, 2);
    assert!(
        (xyz[0] - 95.047).abs() < 5e-2,
        "X should be ~95.047, got {}",
        xyz[0]
    );
    assert!(
        (xyz[1] - 100.0).abs() < 5e-2,
        "Y should be ~100, got {}",
        xyz[1]
    );
    assert!(
        (xyz[2] - 108.883).abs() < 5e-2,
        "Z should be ~108.883, got {}",
        xyz[2]
    );
}

/// Extra bands beyond the colour bands must be carried through unchanged,
/// after the colour bands, at the output depth. This pins the extra-band
/// output-index arithmetic (`out_base + tgt_bands + (c - src_bands)`) the
/// streaming rewrite reproduces in place of the old push order.
#[test]
fn extra_band_is_carried_through_after_colour_bands() {
    // 4-band sRGB source: three colour bands + one extra (e.g. alpha).
    let src = Raster::constant(4, 4, &[255.0, 255.0, 255.0, 42.0], Interpretation::Srgb);
    let out = src.colourspace(Interpretation::Lab);

    // Lab is a three-band space, so the output is Lab (3) + 1 extra = 4.
    assert_eq!(
        out.format().channels(),
        4,
        "extra band must survive the conversion"
    );

    let px = out.getpoint(1, 1);
    // Colour bands: Lab white.
    assert!(
        (px[0] - 100.0).abs() < 1e-2,
        "L should be ~100, got {}",
        px[0]
    );
    assert!(px[1].abs() < 5e-2, "a should be ~0, got {}", px[1]);
    assert!(px[2].abs() < 5e-2, "b should be ~0, got {}", px[2]);
    // Extra band, plain-cast through unchanged.
    assert!(
        (px[3] - 42.0).abs() < 1e-3,
        "extra band should carry 42.0, got {}",
        px[3]
    );
}
