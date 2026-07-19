//! Integration test: the single-axis coordinate ceiling (libvips
//! `VIPS_MAX_COORD`) is a per-decode [`DecodeLimits::max_coord`] knob and
//! nothing else — there is no process-global to set.
//!
//! Regression guard for libviprs#462. The crate briefly carried inert,
//! deprecated process-global shims — `get_max_coord` / `set_max_coord` /
//! `init_from_env` over a `MAX_COORD` static — retained only until the
//! in-tree ported cells migrated off them. They have been removed. The
//! ceiling now lives solely on [`DecodeLimits`], built with
//! [`DecodeLimits::with_max_coord`] and enforced by every decoder before
//! any pixel allocation.
//!
//! There is no pixel-diff oracle here: the finding is the removal of a
//! deprecated API plus the rejection / error-path property of its
//! replacement, so the assertions are the typed `Err` on an over-ceiling
//! decode and the `Ok` at/under it. The oversized input is a native `.v`
//! image encoded in-process from a [`Raster`], so no external fixture or
//! `vips` oracle is required (the `.v` header geometry is exactly what we
//! over-constrain).

use libviprs::imageio::DEFAULT_MAX_COORD;
use libviprs::source::{DecodeLimits, SourceError, decode_bytes_with_limits};
use libviprs::{PixelFormat, Raster};

/// Encode a single-band `width`×`height` image as a native `.v` byte
/// buffer whose declared header geometry the decoders then re-check
/// against the caller's [`DecodeLimits`].
fn vips_bytes(width: u32, height: u32) -> Vec<u8> {
    Raster::zeroed(width, height, PixelFormat::Gray8)
        .expect("raster construction")
        .encode_vips()
        .expect("encode .v")
}

/// The per-decode ceiling built with [`DecodeLimits::with_max_coord`]
/// rejects an over-ceiling declared width with the typed
/// [`SourceError::CoordLimitExceeded`], and admits an under-ceiling image —
/// the behaviour the removed `set_max_coord` process-global only ever
/// claimed (it was inert). This is the migration target for the former
/// `test_cli_max_coord_flag` shell port.
#[test]
fn with_max_coord_is_the_only_ceiling_knob() {
    let tight = DecodeLimits::default().with_max_coord(1000);

    // A 2000 px width past the ceiling rejects before allocation, carrying
    // the offending geometry and the configured ceiling.
    let err = decode_bytes_with_limits(&vips_bytes(2000, 1), tight)
        .expect_err("over-ceiling width must reject");
    assert!(
        matches!(
            err,
            SourceError::CoordLimitExceeded {
                width: 2000,
                height: 1,
                max_coord: 1000,
            }
        ),
        "expected typed CoordLimitExceeded, got {err}"
    );

    // A 500 px width under the ceiling decodes cleanly.
    let raster = decode_bytes_with_limits(&vips_bytes(500, 500), tight)
        .expect("under-ceiling image must decode");
    assert_eq!((raster.width(), raster.height()), (500, 500));
}

/// The former `init_from_env` shim read `VIPS_MAX_COORD` into a global; the
/// migration reads the variable in the caller and folds it into a
/// [`DecodeLimits`]. This is the replacement path for the former
/// `test_cli_max_coord_env` shell port. Runs as one serialised test so the
/// process environment is mutated and restored in order.
#[test]
fn env_var_ceiling_is_read_by_the_caller_into_decode_limits() {
    // SAFETY: test-local mutation of this process's environment; the
    // variable is removed again before the test returns.
    unsafe { std::env::set_var("VIPS_MAX_COORD", "500") };

    let ceiling: u32 = std::env::var("VIPS_MAX_COORD")
        .ok()
        .and_then(|v| v.parse().ok())
        .expect("VIPS_MAX_COORD parses");
    let limits = DecodeLimits::default().with_max_coord(ceiling);

    unsafe { std::env::remove_var("VIPS_MAX_COORD") };

    assert_eq!(limits.max_coord, 500);
    // The caller-built ceiling enforces just like any other: a 600 px width
    // over the env-derived 500 rejects.
    let err = decode_bytes_with_limits(&vips_bytes(600, 1), limits)
        .expect_err("width over the env-derived ceiling must reject");
    assert!(
        matches!(err, SourceError::CoordLimitExceeded { max_coord: 500, .. }),
        "expected typed CoordLimitExceeded at the env-derived ceiling, got {err}"
    );
}

/// With no override, [`DecodeLimits::max_coord`] carries the libvips
/// `VIPS_MAX_COORD` default, so an in-bounds decode is unaffected — the same
/// default the removed `MAX_COORD` static once held.
#[test]
fn default_ceiling_matches_the_libvips_default() {
    assert_eq!(DecodeLimits::default().max_coord, DEFAULT_MAX_COORD);

    let raster = decode_bytes_with_limits(&vips_bytes(2000, 1), DecodeLimits::default())
        .expect("in-bounds decode under default limits must succeed");
    assert_eq!((raster.width(), raster.height()), (2000, 1));
}
