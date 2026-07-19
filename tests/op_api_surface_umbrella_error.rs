//! Op public error/API surface: dual-API completeness (#281) and the
//! crate-level umbrella error (#285).
//!
//! Two review findings, both on the *op* public surface, are pinned here:
//!
//! * **#281** — the integer constant-arithmetic family (`add_const`,
//!   `sub_const`, `mul_const`, `floordiv_const`, `pow_const`, `rem_const`)
//!   shipped *only* the panicking form even though it can fail
//!   data-dependently (it rejects float rasters). Every op that can fail on
//!   caller input must expose a fallible `try_*` twin, and the panicking
//!   form must delegate to it — the crate's existing dual-API convention
//!   (`try_add_vec` / `add_vec`, `try_sub` / `sub`, ...). These tests assert
//!   the new `try_*_const` forms return `Err` exactly where the panicking
//!   twin panics, and that both are pixel-identical on valid integer input.
//!
//! * **#285** — 26 per-module error enums were re-exported flat from the
//!   crate root with no umbrella, so any caller composing op families had to
//!   write bespoke `From` glue. A `#[non_exhaustive]` crate-level [`OpError`]
//!   with `#[from]` conversions for the per-module op enums lets one function
//!   surface errors from several families through a single type. These tests
//!   drive that composition and assert the `From` conversions land in the
//!   right variant.
//!
//! Neither finding is vips-differential: they concern the error-handling and
//! type surface, not pixel output. The happy-path pixel semantics of these
//! ops are already covered (vips-differentially) by `ported_arithmetic.rs`;
//! here we assert only that the fallible surface exists and behaves, so there
//! is no vips reference to generate.

use libviprs::{ArithmeticError, BandError, OpError, PixelFormat, Raster};

/// A zeroed single-band 32-bit float raster. The mutating integer arithmetic
/// ops reject float input, so these rasters exercise the `FloatUnsupported`
/// path (mirrors `arithmetic_float_error_message.rs`).
fn float_raster() -> Raster {
    let fmt = PixelFormat::with_channels(1, 4).expect("FloatF32(1) is a valid format");
    Raster::zeroed(2, 2, fmt).expect("2x2 float raster allocates")
}

/// A small non-uniform single-band 8-bit raster for happy-path parity.
fn int_raster() -> Raster {
    Raster::new(2, 2, PixelFormat::Gray8, vec![10u8, 20, 30, 40]).expect("Gray8 raster")
}

/// Run `body`, expected to panic, with the default hook silenced so the
/// intentional panic does not spam test output.
fn expect_panic(body: impl FnOnce() + std::panic::UnwindSafe) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(body);
    std::panic::set_hook(prev);
    assert!(result.is_err(), "body was expected to panic");
}

// ---------------------------------------------------------------------------
// #281 — the constant-arithmetic family gains fallible twins
// ---------------------------------------------------------------------------

/// Every integer constant op exposes a `try_*` twin that returns
/// `ArithmeticError::FloatUnsupported` on a float raster, naming the op.
#[test]
fn try_const_ops_return_float_unsupported() {
    let f = float_raster();

    let cases: &[(&str, ArithmeticError)] = &[
        ("add_const", f.try_add_const(1.0).unwrap_err()),
        ("sub_const", f.try_sub_const(1.0).unwrap_err()),
        ("mul_const", f.try_mul_const(2.0).unwrap_err()),
        ("floordiv_const", f.try_floordiv_const(2.0).unwrap_err()),
        ("pow_const", f.try_pow_const(2.0).unwrap_err()),
        ("rem_const", f.try_rem_const(2.0).unwrap_err()),
    ];

    for (op, err) in cases {
        match err {
            ArithmeticError::FloatUnsupported { op: got } => assert_eq!(
                got, op,
                "try_{op} must report FloatUnsupported naming the op"
            ),
            other => panic!("try_{op} expected FloatUnsupported, got {other:?}"),
        }
    }
}

/// The panicking twin of each constant op panics exactly where the `try_*`
/// form returns `Err` — the dual-API contract (short = ergonomic-panics,
/// `try_` = fallible).
#[test]
fn const_ops_panic_where_try_forms_err() {
    // Panicking twin panics on float input.
    expect_panic(|| {
        let _ = float_raster().add_const(1.0);
    });
    expect_panic(|| {
        let _ = float_raster().sub_const(1.0);
    });
    expect_panic(|| {
        let _ = float_raster().mul_const(2.0);
    });
    expect_panic(|| {
        let _ = float_raster().floordiv_const(2.0);
    });
    expect_panic(|| {
        let _ = float_raster().pow_const(2.0);
    });
    expect_panic(|| {
        let _ = float_raster().rem_const(2.0);
    });
}

/// On valid integer input the `try_*` twin succeeds and is byte-identical to
/// its panicking form — the panicking form delegates to `try_*`.
#[test]
fn try_const_ops_match_panicking_on_valid_input() {
    let r = int_raster();

    let pairs: [(Raster, Raster); 6] = [
        (r.try_add_const(5.0).unwrap(), r.add_const(5.0)),
        (r.try_sub_const(5.0).unwrap(), r.sub_const(5.0)),
        (r.try_mul_const(3.0).unwrap(), r.mul_const(3.0)),
        (r.try_floordiv_const(3.0).unwrap(), r.floordiv_const(3.0)),
        (r.try_pow_const(2.0).unwrap(), r.pow_const(2.0)),
        (r.try_rem_const(7.0).unwrap(), r.rem_const(7.0)),
    ];

    for (i, (fallible, panicking)) in pairs.iter().enumerate() {
        assert_eq!(
            fallible.format(),
            panicking.format(),
            "case {i}: try_ / panicking formats diverge"
        );
        assert_eq!(
            fallible.data(),
            panicking.data(),
            "case {i}: try_ / panicking pixels diverge"
        );
    }
}

// ---------------------------------------------------------------------------
// #285 — the crate-level umbrella error over op families
// ---------------------------------------------------------------------------

/// A per-module op error converts into the umbrella via `From`, landing in the
/// matching variant.
#[test]
fn op_error_from_conversions_select_the_right_variant() {
    let arith: ArithmeticError = float_raster().try_rem_const(2.0).unwrap_err();
    assert!(
        matches!(OpError::from(arith), OpError::Arithmetic(_)),
        "ArithmeticError must convert into OpError::Arithmetic"
    );

    let band: BandError = int_raster().try_extract_band(99).unwrap_err();
    assert!(
        matches!(OpError::from(band), OpError::Band(_)),
        "BandError must convert into OpError::Band"
    );
}

/// The umbrella lets a single function compose two op families and surface
/// their errors through one `?`-friendly type. Here the arithmetic leg fails
/// first (float input), so the composed result carries the arithmetic
/// variant.
#[test]
fn op_error_composes_multiple_op_families() {
    fn pipeline(r: &Raster) -> Result<Raster, OpError> {
        let scaled = r.try_mul_const(2.0)?; // ArithmeticError -> OpError
        let band = scaled.try_extract_band(0)?; // BandError -> OpError
        Ok(band)
    }

    // Float input trips the arithmetic leg; the error reaches us as OpError
    // without any bespoke mapping glue.
    let err = pipeline(&float_raster()).unwrap_err();
    assert!(
        matches!(err, OpError::Arithmetic(_)),
        "composed pipeline must surface the arithmetic failure through OpError"
    );

    // A band index past the end trips the bands leg on a valid integer input.
    fn extract_only(r: &Raster) -> Result<Raster, OpError> {
        Ok(r.try_extract_band(99)?)
    }
    let err = extract_only(&int_raster()).unwrap_err();
    assert!(
        matches!(err, OpError::Band(_)),
        "composed pipeline must surface the band failure through OpError"
    );
}

/// The umbrella's `Display` delegates to the wrapped per-module error so no
/// diagnostic is lost in the conversion.
#[test]
fn op_error_display_delegates_to_inner() {
    let arith: ArithmeticError = float_raster().try_rem_const(2.0).unwrap_err();
    let inner = arith.to_string();
    let wrapped = OpError::from(float_raster().try_rem_const(2.0).unwrap_err()).to_string();
    assert_eq!(
        wrapped, inner,
        "OpError Display must delegate transparently to the wrapped error"
    );
}
