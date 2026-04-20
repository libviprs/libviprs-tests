//! Phase 3 hardening: failing integration tests for fuzzy blank-tile detection.
//!
//! TDD (no implementation). These tests exercise the *planned* API:
//!   - `BlankTileStrategy::PlaceholderWithTolerance { max_channel_delta: u8 }`
//!   - `pub fn is_blank_tile_with_tolerance(raster: &Raster, max_channel_delta: u8) -> bool`
//!
//! Detection spec (single simple rule, applied to ALL channels — alpha included):
//!   A tile is considered blank at tolerance `t` iff, for every channel `c` in
//!   the pixel format (including alpha when present), the per-channel range
//!   `max(channel_c) - min(channel_c)` across all pixels is `<= t`.
//!
//! This means:
//!   - Tolerance 0 reduces exactly to "every pixel identical" (the existing
//!     `Placeholder` / `is_blank_tile` behaviour).
//!   - RGB constant + alpha varying by > t is NOT blank (alpha is checked).
//!   - A fully-transparent RGBA tile (A near 0, range <= t) is still blank
//!     only if each *other* channel's range is also <= t. If any channel's
//!     spread exceeds the threshold, the tile is not blank — this keeps the
//!     rule uniform and simple.
//!
//! All tests are expected to FAIL TO COMPILE (or fail at runtime) until the
//! planned API is implemented on the libviprs `feat/phase3-hardening` branch.

use libviprs::engine::{is_blank_tile, is_blank_tile_with_tolerance};
use libviprs::{
    BlankTileStrategy, EngineConfig, Layout, MemorySink, PixelFormat, PyramidPlanner, Raster,
    generate_pyramid,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a raster where every channel of every pixel is in `[255 - jitter, 255]`.
/// Uses a small deterministic hash of the pixel index so the values are varied
/// but stay inside the jitter window.
fn near_white_raster(w: u32, h: u32, fmt: PixelFormat, jitter: u8) -> Raster {
    let bpp = fmt.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            for c in 0..bpp {
                // Deterministic per-(x,y,channel) value within [255-jitter, 255].
                let mix =
                    (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ (c as u32 * 97)) as u8;
                let delta = if jitter == 0 { 0 } else { mix % (jitter + 1) };
                data[off + c] = 255u8.saturating_sub(delta);
            }
        }
    }
    Raster::new(w, h, fmt, data).unwrap()
}

/// Build an RGB gradient raster — a strong horizontal + vertical ramp
/// guaranteed to span the full 0..=255 range on every channel.
fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            // Map x/w and y/h to the full 0..=255 range so the spread is 255.
            let fx = if w > 1 { (x * 255 / (w - 1)) as u8 } else { 0 };
            let fy = if h > 1 { (y * 255 / (h - 1)) as u8 } else { 0 };
            data[off] = fx;
            data[off + 1] = fy;
            data[off + 2] = fx.wrapping_add(fy);
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Construct an RGBA raster with three independent channel generators.
fn rgba_raster(
    w: u32,
    h: u32,
    r: impl Fn(u32, u32) -> u8,
    g: impl Fn(u32, u32) -> u8,
    b: impl Fn(u32, u32) -> u8,
    a: impl Fn(u32, u32) -> u8,
) -> Raster {
    let bpp = PixelFormat::Rgba8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = r(x, y);
            data[off + 1] = g(x, y);
            data[off + 2] = b(x, y);
            data[off + 3] = a(x, y);
        }
    }
    Raster::new(w, h, PixelFormat::Rgba8, data).unwrap()
}

/// Gray8 raster where every pixel value is in `[255 - jitter, 255]`.
fn near_white_gray(w: u32, h: u32, jitter: u8) -> Raster {
    near_white_raster(w, h, PixelFormat::Gray8, jitter)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. `tolerance_zero_equals_exact_match` —
///    With `max_channel_delta == 0` the tolerance helper must agree with the
///    existing exact `is_blank_tile` on every kind of raster we can throw at it.
#[test]
fn tolerance_zero_equals_exact_match() {
    // Solid → both true.
    let solid = near_white_raster(16, 16, PixelFormat::Rgb8, 0);
    assert_eq!(
        is_blank_tile(&solid),
        is_blank_tile_with_tolerance(&solid, 0)
    );
    assert!(is_blank_tile_with_tolerance(&solid, 0));

    // Near-white with small jitter → exact says false, tolerance(0) must also say false.
    let near = near_white_raster(16, 16, PixelFormat::Rgb8, 2);
    assert_eq!(is_blank_tile(&near), is_blank_tile_with_tolerance(&near, 0));
    assert!(!is_blank_tile_with_tolerance(&near, 0));

    // Gradient → both false.
    let grad = gradient_raster(16, 16);
    assert_eq!(is_blank_tile(&grad), is_blank_tile_with_tolerance(&grad, 0));
    assert!(!is_blank_tile_with_tolerance(&grad, 0));
}

/// 2. `near_white_under_tolerance_is_blank` —
///    An RGB tile where every channel of every pixel is within +/- 2 of 255
///    must be reported as blank at tolerance 3.
#[test]
fn near_white_under_tolerance_is_blank() {
    let raster = near_white_raster(32, 32, PixelFormat::Rgb8, 2);
    assert!(
        is_blank_tile_with_tolerance(&raster, 3),
        "per-channel spread <= 2 should be blank at tolerance 3"
    );
    // And definitely not blank via the exact helper (values vary).
    assert!(!is_blank_tile(&raster));
}

/// 3. `near_white_above_tolerance_is_not_blank` —
///    The same raster at tolerance 1 must NOT be reported as blank, because
///    some channels span a range of 2.
#[test]
fn near_white_above_tolerance_is_not_blank() {
    let raster = near_white_raster(32, 32, PixelFormat::Rgb8, 2);
    assert!(
        !is_blank_tile_with_tolerance(&raster, 1),
        "per-channel spread of 2 must not satisfy tolerance 1"
    );
}

/// 4. `gradient_is_not_blank_at_any_reasonable_tolerance` —
///    A strong RGB gradient that spans the full 0..=255 range must not be
///    classified as blank even at tolerance 10.
#[test]
fn gradient_is_not_blank_at_any_reasonable_tolerance() {
    let raster = gradient_raster(64, 64);
    assert!(!is_blank_tile_with_tolerance(&raster, 10));
    // Cross-check a few more points for sanity.
    assert!(!is_blank_tile_with_tolerance(&raster, 0));
    assert!(!is_blank_tile_with_tolerance(&raster, 5));
}

/// 5. `alpha_channel_is_included_in_check` —
///    RGBA raster with constant RGB but alpha varying well beyond the
///    tolerance must NOT be reported as blank; the alpha channel is part of
///    the spec.
#[test]
fn alpha_channel_is_included_in_check() {
    // R=G=B=255, A ramps 0..=255 → alpha spread is 255, way above tolerance.
    let raster = rgba_raster(
        16,
        16,
        |_, _| 255,
        |_, _| 255,
        |_, _| 255,
        |x, _| (x * 17) as u8, // varies a lot
    );
    assert!(
        !is_blank_tile_with_tolerance(&raster, 5),
        "alpha spread must disqualify the tile even when RGB is constant"
    );
}

/// 6. `gray8_tolerance` —
///    Single-channel tolerance works the same way.
#[test]
fn gray8_tolerance() {
    let raster = near_white_gray(24, 24, 4);
    assert!(is_blank_tile_with_tolerance(&raster, 4));
    assert!(is_blank_tile_with_tolerance(&raster, 5));
    assert!(!is_blank_tile_with_tolerance(&raster, 3));
}

/// 7. `rgba8_tolerance_detects_near_transparent_with_colored_pixels` —
///    Chosen spec (documented in the module header): a tile is blank iff the
///    per-channel spread is <= tolerance for EVERY channel.
///
///    Consequence: an RGBA tile whose alpha is uniformly near-zero but whose
///    RGB channels vary wildly is NOT blank under this simple rule. This test
///    pins that behaviour down so that any future "alpha-aware" short-circuit
///    must be an intentional API addition, not a silent regression.
///
///    We additionally check the complementary positive case: when the RGB
///    channels ALSO stay within the tolerance, a near-zero-alpha tile IS
///    considered blank.
#[test]
fn rgba8_tolerance_detects_near_transparent_with_colored_pixels() {
    // Case A: alpha ~ 0 but RGB varies wildly — under the simple uniform rule
    // this is NOT blank.
    let colored_but_transparent = rgba_raster(
        16,
        16,
        |x, _| (x * 31) as u8,         // R varies 0..
        |_, y| (y * 29) as u8,         // G varies 0..
        |x, y| ((x + y) * 23) as u8,   // B varies 0..
        |x, y| ((x ^ y) & 0x01) as u8, // A in {0, 1} → spread 1
    );
    assert!(
        !is_blank_tile_with_tolerance(&colored_but_transparent, 5),
        "simple rule: wildly varying RGB disqualifies the tile even if alpha ~ 0"
    );

    // Case B: all four channels stay within tolerance → blank.
    let uniform_near_transparent = rgba_raster(
        16,
        16,
        |_, _| 0,
        |_, _| 0,
        |_, _| 0,
        |x, _| (x & 0x01) as u8, // A in {0, 1} → spread 1
    );
    assert!(
        is_blank_tile_with_tolerance(&uniform_near_transparent, 2),
        "uniform RGB + tiny alpha spread should be blank under tolerance 2"
    );
}

/// 8. `engine_with_tolerance_writes_placeholder_for_near_white_tiles` —
///    End-to-end: run `generate_pyramid` with a near-white input + the new
///    `PlaceholderWithTolerance { max_channel_delta: 5 }` strategy and assert
///    `tiles_skipped > 0` in the `EngineResult`.
#[test]
fn engine_with_tolerance_writes_placeholder_for_near_white_tiles() {
    let src = near_white_raster(128, 128, PixelFormat::Rgb8, 3);
    let planner = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let sink = MemorySink::new();
    let config = EngineConfig::default().with_blank_tile_strategy(
        BlankTileStrategy::PlaceholderWithTolerance {
            max_channel_delta: 5,
        },
    );

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();

    assert!(
        result.tiles_skipped > 0,
        "expected tolerance-skipped tiles, got tiles_skipped={}",
        result.tiles_skipped
    );
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

/// 9. `determinism_with_tolerance` —
///    Two identical runs of the engine with the same tolerance produce
///    identical `tiles_skipped` counts and the same ordered tile payloads.
#[test]
fn determinism_with_tolerance() {
    let src = near_white_raster(128, 128, PixelFormat::Rgb8, 3);
    let planner = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let config = EngineConfig::default().with_blank_tile_strategy(
        BlankTileStrategy::PlaceholderWithTolerance {
            max_channel_delta: 4,
        },
    );

    let sink_a = MemorySink::new();
    let result_a = generate_pyramid(&src, &plan, &sink_a, &config).unwrap();

    let sink_b = MemorySink::new();
    let result_b = generate_pyramid(&src, &plan, &sink_b, &config).unwrap();

    assert_eq!(
        result_a.tiles_skipped, result_b.tiles_skipped,
        "tiles_skipped must be deterministic across runs"
    );
    assert_eq!(result_a.tiles_produced, result_b.tiles_produced);

    let mut tiles_a = sink_a.tiles();
    let mut tiles_b = sink_b.tiles();
    tiles_a.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    tiles_b.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    assert_eq!(tiles_a.len(), tiles_b.len());
    for (ta, tb) in tiles_a.iter().zip(tiles_b.iter()) {
        assert_eq!(ta.coord, tb.coord);
        assert_eq!(
            ta.data, tb.data,
            "tile bytes must match across runs at {:?}",
            ta.coord
        );
    }
}

/// 10. `property_at_higher_tolerance_never_fewer_blanks` —
///     For a fixed raster, the number of blank tiles (as reported by the
///     engine's `tiles_skipped`) at tolerance `N` is never less than at
///     tolerance `N-1`. Monotonicity of the detector in the tolerance
///     parameter is a correctness invariant.
#[test]
fn property_at_higher_tolerance_never_fewer_blanks() {
    let src = near_white_raster(128, 128, PixelFormat::Rgb8, 6);
    let planner = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let mut last_skipped: u64 = 0;
    for t in 0u8..=8 {
        let sink = MemorySink::new();
        let config = EngineConfig::default().with_blank_tile_strategy(
            BlankTileStrategy::PlaceholderWithTolerance {
                max_channel_delta: t,
            },
        );
        let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();
        assert!(
            result.tiles_skipped >= last_skipped,
            "monotonicity violated: tolerance {} skipped {}, previous {}",
            t,
            result.tiles_skipped,
            last_skipped,
        );
        last_skipped = result.tiles_skipped;
    }

    // Sanity: with a jitter of 6, by tolerance 8 we should have skipped at
    // least one tile somewhere in the pyramid.
    assert!(
        last_skipped > 0,
        "expected at least some tolerance-skipped tiles at tolerance 8"
    );

    // Cross-check the helper directly for a single tile: monotone on one raster.
    let tile = near_white_raster(64, 64, PixelFormat::Rgb8, 4);
    let mut prev = false;
    for t in 0u8..=10 {
        let now = is_blank_tile_with_tolerance(&tile, t);
        if prev {
            assert!(
                now,
                "is_blank_tile_with_tolerance must stay true once it becomes true (tolerance {})",
                t,
            );
        }
        prev = now;
    }
}
