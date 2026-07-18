//! Repro cells for the PR #335 review follow-ups tracked by issue #346.
//!
//! PR #335 added ink channel-count validation only to the `Raster::draw_*` /
//! `try_draw_*` wrappers, but two equally-public routes stayed defective:
//!
//! 1. The generic [`Raster::draw`]`(&op)` entry point and the draw-op
//!    constructors ([`Circle`], [`Rectangle`], [`Mask`], ...) are an *unguarded*
//!    route to the exact #294 corruption: a wrong-width ink is cycled/truncated
//!    verbatim through `put_pixel`, silently painting channels from the wrong
//!    bytes (e.g. an `Rgba8` alpha band from the red byte). The infallible
//!    `draw(&op)` path cannot return an error, so the intended contract is the
//!    same documented panic the `draw_*` wrappers already raise.
//! 2. The *fallible* [`Raster::try_draw_mask`] still panics on a 32-bit-float
//!    raster (`RgbaF32` / `FloatF32`) even with correct-width ink, because
//!    `Mask::apply` decodes samples as unsigned integers and hits the
//!    float-unsupported panic arm. A fallible entry point must return a typed
//!    `DrawError`, not unwind.
//!
//! Both cells are RED against the counterpart core pinned in `COUNTERPART_REV`
//! and go GREEN once core validates ink on the op-application path and makes
//! `try_draw_mask` fallible on float rasters. No libvips at runtime: these are
//! pure in-memory contract checks over the public draw API.

use std::panic::{AssertUnwindSafe, catch_unwind};

use libviprs::{Circle, Mask, PixelFormat, Raster, Rectangle};

/// A single-band 8-bit stencil that is fully opaque, so the mask blend would
/// actually touch every covered pixel (isolating the ink/format defect rather
/// than a no-op mask).
fn opaque_mask(w: u32, h: u32) -> Raster {
    let mut m = Raster::zeroed(w, h, PixelFormat::Gray8).unwrap();
    m.draw_rect_filled(&[255], 0, 0, w as i32, h as i32);
    m
}

/// (1a) `Raster::draw(&Rectangle::filled(..))` (the `put_pixel` ink mechanism)
/// with a 3-byte RGB ink on a 4-byte `Rgba8` pixel must NOT silently corrupt:
/// the guarded wrappers reject this, so the raw op path must guard it too. The
/// only contract available on the infallible `draw(&op)` entry point is a
/// panic, so we assert the call unwinds instead of cycling the ink.
///
/// RED against unfixed core: the op cycles `[10,20,30]` across the 4-byte pixel
/// (alpha painted from red) and returns normally, so no panic is observed.
#[test]
fn draw_op_rectangle_path_guards_wrong_width_ink() {
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let mut rgba = Raster::zeroed(4, 4, PixelFormat::Rgba8).unwrap();
        rgba.draw(&Rectangle::filled(&[10, 20, 30], 0, 0, 2, 2));
    }))
    .is_err();
    assert!(
        panicked,
        "draw(&Rectangle::filled) with a 3-byte ink on an Rgba8 raster must be \
         guarded (documented panic), not cycle the ink and corrupt the alpha band"
    );
}

/// (1b) The same defect through the *other* ink mechanism: `Circle` also paints
/// via `put_pixel`, and directly constructing + applying it bypasses the
/// wrapper's `check_ink`. A 1-byte ink on a 2-byte `Gray16` pixel would repeat
/// the low byte; the raw path must guard it.
///
/// RED against unfixed core: no panic, the ink is cycled into both bytes.
#[test]
fn draw_op_circle_path_guards_wrong_width_ink() {
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let mut g16 = Raster::zeroed(4, 4, PixelFormat::Gray16).unwrap();
        g16.draw(&Circle::filled(&[200], 2, 2, 1));
    }))
    .is_err();
    assert!(
        panicked,
        "draw(&Circle::filled) with a 1-byte ink on a Gray16 raster must be \
         guarded (documented panic), not repeat the low byte across both channels"
    );
}

/// (1c) The `ink_pixel` mechanism (used by `Mask`) is the third unguarded
/// route: `draw(&Mask::new(..))` with a wrong-width ink on a non-float raster
/// materialises the ink by cycling it to the pixel width. The raw op path must
/// guard it like the wrappers do.
///
/// RED against unfixed core: no panic; the 3-byte ink is cycled to 4 bytes and
/// blended, corrupting the alpha band.
#[test]
fn draw_op_mask_path_guards_wrong_width_ink() {
    let mask = opaque_mask(2, 2);
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let mut rgba = Raster::zeroed(4, 4, PixelFormat::Rgba8).unwrap();
        rgba.draw(&Mask::new(&[10, 20, 30], &mask, 0, 0));
    }))
    .is_err();
    assert!(
        panicked,
        "draw(&Mask::new) with a 3-byte ink on an Rgba8 raster must be guarded \
         (documented panic), not cycle the ink through the blend"
    );
}

/// (2) `try_draw_mask` is the *fallible* mask entry point, yet on a 32-bit-float
/// raster it panics deep inside the unsigned-sample decode instead of returning
/// `Err`. The ink here is correct width (4 x f32 = 16 bytes), so the only reason
/// to fail is the unsupported float format, which must surface as a typed
/// `DrawError` — never a panic — from a `try_` method.
///
/// RED against unfixed core: `Mask::apply` -> `channel_at` hits the float panic
/// arm, unwinding the call rather than returning `Err`.
#[test]
fn try_draw_mask_returns_err_on_float_raster_instead_of_panicking() {
    let mask = opaque_mask(2, 2);
    // Correct-width ink for RgbaF32 (4 channels * 4 bytes) so ink length is not
    // the reason for failure.
    let ink: Vec<u8> = [0.0f32; 4].iter().flat_map(|f| f.to_ne_bytes()).collect();

    // The call itself must not panic: run it under catch_unwind so a panicking
    // (unfixed) core is reported as a clear assertion failure, and a fixed core
    // yields a `Result` we can assert on.
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let mut im = Raster::zeroed(2, 2, PixelFormat::RgbaF32).unwrap();
        im.try_draw_mask(&ink, &mask, 0, 0)
    }));

    match outcome {
        Err(_) => panic!(
            "try_draw_mask panicked on an RgbaF32 raster; a fallible `try_` entry \
             point must return a typed DrawError for an unsupported float format"
        ),
        Ok(Ok(())) => panic!(
            "try_draw_mask unexpectedly succeeded on an RgbaF32 raster; the mask \
             blend decodes unsigned samples and cannot support float targets"
        ),
        Ok(Err(err)) => {
            // GREEN: a typed error naming the unsupported float format.
            let msg = err.to_string().to_lowercase();
            assert!(
                msg.contains("float"),
                "expected a typed float-unsupported DrawError, got: {err}"
            );
        }
    }
}
