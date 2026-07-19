//! Follow-up to libviprs/libviprs#339 (tracked by #350): PR #339 made each
//! arithmetic op's *output* allocation fallible (`alloc_op_output`, issue
//! #280) but left the far-larger *scratch* buffers inside `try_hough_circle`
//! and `try_stdif` as infallible `vec![..]` allocations, which call
//! `handle_alloc_error` and abort (SIGABRT) the whole process on an
//! attacker-influenced size — exactly the remote-DoS abort #280 set out to
//! remove (issues #433 / #434 / #435).
//!
//! The dominant hole is `try_hough_circle`'s vote accumulator
//! `acc = vec![0u32; w * h * radii]`: `radii` (= `max_radius - min_radius + 1`)
//! is caller-controlled up to `u16::MAX`, so a modest raster with a wide
//! radius range requests a multi-hundred-terabyte `vec!` that aborts before
//! the now-fallible output allocation is ever reached. These tests drive the
//! actual op (not just the `alloc_op_output` constructor boundary) at an
//! over-capacity scratch size and assert it returns a typed `Err` / panics
//! rather than aborting. Against the pristine tree they abort the process
//! (RED); once the scratch is routed through the fallible path they degrade to
//! `Err` (GREEN).
//!
//! `try_stdif` has the identical defect in its two `f64` integral-image
//! scratch buffers and is fixed by the same fallible helper, but its scratch
//! is only ~16x its input, so the input needed to overflow it (~8 TiB) exceeds
//! the 8 GiB `DEFAULT_MAX_ALLOC_BYTES` construction budget and cannot be built
//! in a test — the `hough_circle` accumulator, whose depth is caller-scaled,
//! is the case a test can actually drive.
//!
//! The abort-safety assertion is a pure behavioural contract with no libvips
//! analogue (vips-differential is N/A there). Alongside it we pin a
//! vips-differential reference for `Raster::mul`: libvips `multiply` on two
//! `uchar` images promotes to `ushort` with value `a*b`, which is exactly
//! what `mul` produces (`binary_map` widens to 16-bit, `a*b <= 65535` so no
//! saturation). The reference was produced offline with
//!   `vips multiply input_l.png input_r.png multiply_expected.png`
//! (vips-8.18.4) and committed under
//! `tests/fixtures/arithmetic_large_input_fallible_alloc_expected/`.

use libviprs::{PixelFormat, Raster};

/// A raster whose `hough_circle` vote accumulator is far larger than any host
/// can address. A 24000×24000 Gray8 raster (~576 MB, well within the 8 GiB
/// construction budget) with a full `1..=65535` radius range makes
/// `acc = 24000 * 24000 * 65535 * 4 bytes ≈ 137 TiB`, above the ~64 TiB point
/// at which the allocator refuses the reservation outright (rather than
/// lazily over-committing it), so the failure is fast and deterministic: the
/// pristine infallible `vec![0u32; …]` aborts, the fallible scratch returns
/// `Err`.
const OVERSIZE_DIM: u32 = 24_000;
const RADIUS_MIN: u32 = 1;
const RADIUS_MAX: u32 = 65_535;

/// Directory holding the committed input fixtures and the offline libvips
/// reference output for the differential `mul` check.
fn fixture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arithmetic_large_input_fallible_alloc_expected")
}

/// Load a committed 8-bit RGB PNG fixture as an `Rgb8` [`Raster`], reading it
/// through the `image` crate so the differential compares libviprs' `mul`
/// against libvips on byte-identical inputs.
fn load_rgb8(name: &str) -> Raster {
    let img = image::open(fixture_dir().join(name))
        .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
        .to_rgb8();
    let (w, h) = img.dimensions();
    Raster::new(w, h, PixelFormat::Rgb8, img.into_raw()).expect("fixture raster is well-formed")
}

/// Read a native-endian `u16` sample stream out of a raster buffer.
fn samples_u16(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
        .collect()
}

/// `try_hough_circle` with a caller-controlled radius range must allocate its
/// vote accumulator fallibly: an over-capacity scratch size returns a typed
/// `Err` (routed through `RasterError::AllocationFailed`), never a process
/// abort through `handle_alloc_error`.
#[test]
fn hough_circle_oversize_scratch_returns_typed_error_not_abort() {
    let raster = Raster::zeroed(OVERSIZE_DIM, OVERSIZE_DIM, PixelFormat::Gray8)
        .expect("oversize Gray8 raster is within the construction budget");
    // radii = 65535 is exactly the u16::MAX band ceiling, so `format_for`
    // succeeds and execution reaches the accumulator allocation — the buffer
    // this test is about — rather than short-circuiting on TooManyBands.
    let result = raster.try_hough_circle(RADIUS_MIN, RADIUS_MAX);
    assert!(
        result.is_err(),
        "oversized hough_circle scratch must return Err, not a raster"
    );
}

/// The same fallible-scratch guarantee, driven through the panicking wrapper:
/// `hough_circle` must surface the over-capacity scratch as a panic (its only
/// error channel) rather than aborting the process. A panic unwinds and is
/// catchable; an abort is not.
#[test]
fn hough_circle_oversize_scratch_panics_not_aborts() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = std::panic::catch_unwind(|| {
        let raster = Raster::zeroed(OVERSIZE_DIM, OVERSIZE_DIM, PixelFormat::Gray8)
            .expect("oversize Gray8 raster is within the construction budget");
        raster.hough_circle(RADIUS_MIN, RADIUS_MAX)
    });
    std::panic::set_hook(prev);
    assert!(
        caught.is_err(),
        "oversized hough_circle scratch must panic (unwindable), not abort"
    );
}

/// vips-differential reference for `Raster::mul`: on byte-identical `uchar`
/// inputs, libviprs' `mul` must match libvips `multiply` sample-for-sample
/// (both promote to `ushort` and store `a*b`). Guards that the fallible-scratch
/// rework left the arithmetic result untouched.
#[test]
fn mul_matches_libvips_multiply_reference() {
    let left = load_rgb8("input_l.png");
    let right = load_rgb8("input_r.png");

    let product = left.mul(&right);
    assert_eq!(
        product.format(),
        PixelFormat::Rgb16,
        "mul of two 8-bit rasters promotes to 16-bit"
    );

    let expected = image::open(fixture_dir().join("multiply_expected.png"))
        .expect("read multiply_expected.png")
        .to_rgb16();
    let (ew, eh) = expected.dimensions();
    assert_eq!(
        (product.width(), product.height()),
        (ew, eh),
        "output dimensions match the libvips reference"
    );

    let got = samples_u16(product.data());
    let want: Vec<u16> = expected.into_raw();
    assert_eq!(
        got, want,
        "mul must equal the libvips multiply reference sample-for-sample"
    );
}
