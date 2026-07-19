//! Integration test: the per-decode [`DecodeLimits::max_coord`] single-axis
//! ceiling is enforced across the raster decoders (PNG / JPEG / TIFF), not
//! just the native `.v` reader.
//!
//! Regression guard for libviprs#349 (findings #424 / #429): `max_coord`
//! was honoured only by the `.v` decoder and silently ignored on the
//! `image`-crate path, so an untrusted PNG/JPEG/TIFF header declaring a
//! dimension past the caller's ceiling decoded anyway. Every raster
//! decoder must now reject an over-ceiling declared dimension with the
//! same typed [`SourceError::CoordLimitExceeded`] the `.v` reader returns,
//! rejecting on the header geometry before the frame is allocated.
//!
//! The oversized input fixtures are real 200×100 single-band images
//! generated offline with the libvips CLI (vips 8.18.4):
//!   vips grey oversized_200x100.png 200 100
//!   vips grey oversized_200x100.jpg 200 100
//!   vips grey tmp.v 200 100 && vips cast tmp.v oversized_200x100.tif uchar
//!
//! There is no pixel-diff oracle here: the finding is a rejection /
//! error-path property, so the assertion is the typed `Err`, and vips is
//! used only to synthesise the (in-bounds) inputs whose *declared* header
//! geometry we then over-constrain.

use std::path::PathBuf;

use libviprs::source::{
    DecodeLimits, SourceError, decode_bytes_with_limits, decode_file_with_limits,
};

const FIXTURE_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/decode_limits_max_coord_enforcement_expected"
);

/// The three raster fixtures, one per `image`-crate decoder, all 200×100.
const RASTER_FIXTURES: &[&str] = &[
    "oversized_200x100.png",
    "oversized_200x100.jpg",
    "oversized_200x100.tif",
];

const FIXTURE_WIDTH: u32 = 200;
const FIXTURE_HEIGHT: u32 = 100;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURE_DIR).join(name)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
}

/// A declared width past the ceiling must reject with the typed
/// [`SourceError::CoordLimitExceeded`] carrying the offending geometry —
/// on every raster decoder, from both the in-memory and file entry points.
#[test]
fn over_ceiling_declared_dimension_rejected_across_raster_decoders() {
    // A ceiling below the fixture width (200) but above its height (100):
    // the width axis alone must trip the guard.
    let tight = DecodeLimits::default().with_max_coord(150);

    for name in RASTER_FIXTURES {
        // (a) In-memory entry point.
        let err = decode_bytes_with_limits(&fixture_bytes(name), tight).expect_err(&format!(
            "{name}: bytes decode must reject over-ceiling width"
        ));
        assert!(
            matches!(
                err,
                SourceError::CoordLimitExceeded {
                    width: FIXTURE_WIDTH,
                    height: FIXTURE_HEIGHT,
                    max_coord: 150,
                }
            ),
            "{name}: expected typed CoordLimitExceeded from decode_bytes_with_limits, got {err}"
        );

        // (b) File entry point exercises the same enforcement.
        let err = decode_file_with_limits(&fixture_path(name), tight).expect_err(&format!(
            "{name}: file decode must reject over-ceiling width"
        ));
        assert!(
            matches!(err, SourceError::CoordLimitExceeded { max_coord: 150, .. }),
            "{name}: expected typed CoordLimitExceeded from decode_file_with_limits, got {err}"
        );
    }
}

/// Boundary: exactly at the ceiling is accepted; one below rejects. Proves
/// the check is `>` (not `>=`) and matches the `.v` reader's boundary.
#[test]
fn max_coord_boundary_is_inclusive_on_raster_decoders() {
    for name in RASTER_FIXTURES {
        let bytes = fixture_bytes(name);

        // The larger axis is the width (200); a ceiling exactly there passes.
        let at = DecodeLimits::default().with_max_coord(FIXTURE_WIDTH);
        let raster = decode_bytes_with_limits(&bytes, at)
            .unwrap_or_else(|e| panic!("{name}: decode at ceiling must succeed, got {e}"));
        assert_eq!(
            (raster.width(), raster.height()),
            (FIXTURE_WIDTH, FIXTURE_HEIGHT)
        );

        // One below the width rejects.
        let below = DecodeLimits::default().with_max_coord(FIXTURE_WIDTH - 1);
        assert!(
            matches!(
                decode_bytes_with_limits(&bytes, below),
                Err(SourceError::CoordLimitExceeded { .. })
            ),
            "{name}: one below the width ceiling must reject"
        );
    }
}

/// The default limits (10,000,000 px per axis) leave a legitimate in-bounds
/// decode completely unaffected on every raster decoder.
#[test]
fn in_bounds_decode_unaffected_by_default_limits() {
    for name in RASTER_FIXTURES {
        let raster = decode_bytes_with_limits(&fixture_bytes(name), DecodeLimits::default())
            .unwrap_or_else(|e| panic!("{name}: default in-bounds decode must succeed, got {e}"));
        assert_eq!(
            (raster.width(), raster.height()),
            (FIXTURE_WIDTH, FIXTURE_HEIGHT)
        );
    }
}

/// A total pixel count above [`DecodeLimits::max_pixels`] is rejected on the
/// declared header geometry — before the output frame is allocated — on
/// every raster decoder, with the typed
/// [`SourceError::DimensionLimitExceeded`]. Regression guard for the
/// pre-allocation `max_pixels` parity between the raster path and the `.v`
/// reader: the fixtures (200×100 = 20,000 px) sit under the default single-
/// axis ceiling, so only the total-pixel budget can trip here.
#[test]
fn over_max_pixels_rejected_pre_allocation_across_raster_decoders() {
    // 20,000 total px for the fixtures; a budget below that must reject
    // while leaving both per-axis dimensions well under `max_coord`.
    const PIXEL_BUDGET: u64 = 1_000;
    let tight = DecodeLimits::default().with_max_pixels(PIXEL_BUDGET);

    for name in RASTER_FIXTURES {
        let err = decode_bytes_with_limits(&fixture_bytes(name), tight)
            .expect_err(&format!("{name}: over-max_pixels decode must reject"));
        assert!(
            matches!(
                err,
                SourceError::DimensionLimitExceeded {
                    width: FIXTURE_WIDTH,
                    height: FIXTURE_HEIGHT,
                    max_pixels: PIXEL_BUDGET,
                }
            ),
            "{name}: expected typed DimensionLimitExceeded pre-allocation, got {err}"
        );
    }
}
