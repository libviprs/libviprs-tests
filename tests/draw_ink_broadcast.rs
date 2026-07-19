//! Contract cells for the draw-op ink rules flagged by issues #397 and #399
//! (adversarial-review follow-ups from PR #335 / #294, tracked under #346).
//!
//! Two documentation nits with observable runtime contracts:
//!
//! * **#399** — a shorter ink used to *broadcast*: pre-#294 the draw ops cycled
//!   a too-short ink to fill the pixel, so `&[128]` painted a uniform gray on an
//!   `Rgb8` raster. That silent cycle is the exact #294 channel-corruption route
//!   and is now rejected, but the change surfaced only as an opaque runtime
//!   panic. The rejection must instead *flag* the removed broadcast, and the one
//!   surviving broadcast path — the raw `put_pixel` escape hatch — must keep
//!   cycling a shorter ink.
//! * **#397** — `draw_flood` / `draw_flood_blob` are `draw_*`-named yet
//!   inherently fallible (they also fail with `SeedOutOfBounds`), so on an
//!   ink-width mismatch they return `Err(InkLengthMismatch)` rather than
//!   panicking like the other `draw_*` forms. That documented exception has an
//!   observable contract: the call returns an `Err`, it does not unwind.
//!
//! No libvips at runtime: the draw ops paint in-memory byte buffers and there is
//! no `vips draw_*` CLI equivalent for the panic-vs-error contract or the
//! byte-level broadcast these cells pin, so the assertions are direct in-memory
//! contract checks over the public draw API (`vips_diff_applicable = false`).

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use libviprs::{DrawError, PixelFormat, Raster};

/// Recover the string message from a caught panic payload (`panic!("{e}")`
/// carries either a `&str` or a `String` depending on the format path).
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        String::new()
    }
}

/// #399: a shorter ink on a panicking `draw_*` op is rejected, and the rejection
/// names the removed broadcast so the breaking change is flagged at the call
/// site rather than surfacing as an opaque width panic.
///
/// RED against pristine core: the ink is rejected (the width guard already
/// fires), but the panic message does not mention the broadcast, so the
/// broadcast-signal assertion fails.
#[test]
fn short_ink_on_draw_op_is_rejected_and_flags_the_removed_broadcast() {
    let payload = catch_unwind(AssertUnwindSafe(|| {
        let mut im = Raster::zeroed(4, 4, PixelFormat::Rgb8).unwrap();
        im.draw_rect_filled(&[128], 0, 0, 4, 4);
    }))
    .expect_err("a 1-byte ink on a draw_* op must be rejected, not broadcast");
    let msg = panic_message(payload).to_lowercase();
    assert!(
        msg.contains("does not match"),
        "the rejection must still describe the width mismatch, got: {msg:?}"
    );
    assert!(
        msg.contains("broadcast"),
        "#399: the rejection must flag that a shorter ink is no longer \
         broadcast across channels (breaking change from the pre-#294 cycle), \
         got: {msg:?}"
    );
}

/// #399 (too-long direction): the width guard and its message are symmetric — a
/// too-*long* ink is rejected just like a too-short one, and the rejection still
/// flags the removed broadcast without claiming the ink was "shorter". Guards the
/// directionally-neutral wording of the mismatch message.
#[test]
fn long_ink_on_draw_op_is_rejected_and_flags_the_removed_broadcast() {
    let payload = catch_unwind(AssertUnwindSafe(|| {
        // 4 bytes on a 3-byte Rgb8 pixel: too long, would truncate under the old
        // verbatim write.
        let mut im = Raster::zeroed(4, 4, PixelFormat::Rgb8).unwrap();
        im.draw_rect_filled(&[10, 20, 30, 40], 0, 0, 4, 4);
    }))
    .expect_err("a 4-byte ink on a 3-byte Rgb8 pixel must be rejected, not truncated");
    let msg = panic_message(payload).to_lowercase();
    assert!(
        msg.contains("does not match"),
        "the rejection must still describe the width mismatch, got: {msg:?}"
    );
    assert!(
        msg.contains("broadcast"),
        "#399: the rejection must flag the removed broadcast, got: {msg:?}"
    );
    assert!(
        !msg.contains("a shorter ink is no longer"),
        "#399: the message must not assert the ink was *shorter* — a too-long ink \
         is equally rejected, so the wording must stay directionally neutral, got: {msg:?}"
    );
}

/// The fallible form of a too-long ink surfaces the typed error with the actual
/// `ink_len` / `pixel_bytes`, mirroring the short-ink case for the other direction.
#[test]
fn try_draw_rect_returns_ink_mismatch_for_long_ink() {
    let mut im = Raster::zeroed(4, 4, PixelFormat::Rgb8).unwrap();
    let err = im
        .try_draw_rect(&[10, 20, 30, 40], 0, 0, 4, 4)
        .expect_err("a 4-byte ink on an Rgb8 raster must be a typed error");
    assert!(
        matches!(
            err,
            DrawError::InkLengthMismatch {
                ink_len: 4,
                pixel_bytes: 3,
                ..
            }
        ),
        "expected InkLengthMismatch {{ ink_len: 4, pixel_bytes: 3 }}, got: {err:?}"
    );
}

/// #399: the raw `put_pixel` escape hatch is the one path that still cycles a
/// shorter ink to fill the pixel, so a single-value ink broadcasts to every
/// channel — the documented opt-in replacement for the removed draw-op
/// broadcast. Asserted directly (there is no `vips` equivalent).
#[test]
fn put_pixel_still_broadcasts_a_shorter_ink() {
    let mut im = Raster::zeroed(2, 1, PixelFormat::Rgb8).unwrap();
    im.put_pixel(0, 0, &[128]);
    assert_eq!(
        im.getpoint(0, 0),
        vec![128.0, 128.0, 128.0],
        "put_pixel must cycle a 1-byte ink across all three Rgb8 channels"
    );
    // The untouched neighbour stays zero: the broadcast is confined to (0, 0).
    assert_eq!(im.getpoint(1, 0), vec![0.0, 0.0, 0.0]);
}

/// The fallible form of the rejected op surfaces the typed error rather than
/// panicking, pinning the `ink_len` / `pixel_bytes` the caller is told about.
#[test]
fn try_draw_rect_returns_ink_mismatch_for_short_ink() {
    let mut im = Raster::zeroed(4, 4, PixelFormat::Rgb8).unwrap();
    let err = im
        .try_draw_rect(&[128], 0, 0, 4, 4)
        .expect_err("a 1-byte ink on an Rgb8 raster must be a typed error");
    assert!(
        matches!(
            err,
            DrawError::InkLengthMismatch {
                ink_len: 1,
                pixel_bytes: 3,
                ..
            }
        ),
        "expected InkLengthMismatch {{ ink_len: 1, pixel_bytes: 3 }}, got: {err:?}"
    );
}

/// #397: `draw_flood` / `draw_flood_blob` are `draw_*`-named but inherently
/// fallible, so a short ink comes back as `Err(InkLengthMismatch)` — the call
/// returns, it does not panic like the other `draw_*` wrappers. Run under
/// `catch_unwind` so an (incorrect) unwind is reported as a clear failure.
#[test]
fn draw_flood_wrappers_return_err_on_short_ink_instead_of_panicking() {
    for equal in [false, true] {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut im = Raster::zeroed(8, 8, PixelFormat::Rgb8).unwrap();
            if equal {
                im.draw_flood_blob(&[128], 4, 4)
            } else {
                im.draw_flood(&[128], 4, 4)
            }
        }));
        match outcome {
            Err(_) => panic!(
                "draw_flood{} must return Err(InkLengthMismatch) on a short ink, \
                 not panic like the other draw_* wrappers (#397)",
                if equal { "_blob" } else { "" }
            ),
            Ok(Ok(())) => panic!("a short ink must be rejected, not applied"),
            Ok(Err(e)) => assert!(
                matches!(e, DrawError::InkLengthMismatch { .. }),
                "expected InkLengthMismatch, got: {e:?}"
            ),
        }
    }
}
