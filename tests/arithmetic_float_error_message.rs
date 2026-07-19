//! Follow-up to libviprs/libviprs#339 (tracked by #350): the float-unsupported
//! diagnostic must name the failing op *exactly once*.
//!
//! `ArithmeticError::FloatUnsupported` embeds the op name in its `Display`
//! ("sub does not support float rasters yet ..."), and the panicking op forms
//! route the error through `expect_arith`, which prefixes `"<op>: "` for the
//! other, op-less error variants. For `FloatUnsupported` that prefix doubled
//! the op name, producing "sub: sub does not support float rasters yet ...".
//!
//! These tests pin the diagnostic down to a single occurrence of the op name.
//! They drive only the crate's public panicking surface (`Raster::mul`,
//! `Raster::add_vec`) on a float raster, which the integer arithmetic ops
//! reject, and inspect the caught panic payload. They add no shared-file edits
//! and require no feature flags.
//!
//! (Image-image `sub` used to be one of the rejecting ops; issue #282
//! promoted it to a float raster so it now *accepts* float input, so the
//! image-image case here drives `mul`, which keeps the integer contract and
//! its float-input guard.)

use libviprs::{PixelFormat, Raster};

/// A zeroed single-band 32-bit float raster. The mutating integer arithmetic
/// ops (`mul`, `add_vec`, ...) reject float input, so these rasters exercise
/// the `FloatUnsupported` diagnostic path.
fn float_raster() -> Raster {
    let fmt = PixelFormat::with_channels(1, 4).expect("FloatF32(1) is a valid format");
    Raster::zeroed(2, 2, fmt).expect("2x2 float raster allocates")
}

/// Run `body`, expected to panic, and return its panic message as a `String`
/// with the default hook silenced so the intentional panic does not spam test
/// output (mirrors the core crate's own panic-containment tests).
fn caught_panic_message(body: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let payload = std::panic::catch_unwind(body).expect_err("body was expected to panic");
    std::panic::set_hook(prev);
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("panic payload is a string")
        .to_owned()
}

/// The image-image `mul` on a float raster must panic with the op name once,
/// not "mul: mul ...". Guards the `binary_map` -> `FloatUnsupported` path.
#[test]
fn mul_float_panic_names_op_exactly_once() {
    let (a, b) = (float_raster(), float_raster());
    let msg = caught_panic_message(move || {
        let _ = a.mul(&b);
    });
    assert_eq!(
        msg.matches("mul").count(),
        1,
        "mul() float-unsupported panic must name the op exactly once, got: {msg:?}"
    );
    assert!(
        msg.contains("does not support float rasters yet"),
        "unexpected panic message: {msg:?}"
    );
}

/// The per-band `add_vec` on a float raster must likewise name the op once,
/// not "add_vec: add_vec ...". Guards the `vec_map` -> `FloatUnsupported` path.
#[test]
fn add_vec_float_panic_names_op_exactly_once() {
    let a = float_raster();
    let msg = caught_panic_message(move || {
        let _ = a.add_vec(&[1.0]);
    });
    assert_eq!(
        msg.matches("add_vec").count(),
        1,
        "add_vec() float-unsupported panic must name the op exactly once, got: {msg:?}"
    );
}
