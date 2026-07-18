//! Follow-up repro for issue #289 (the mixed-depth compositing fix landed in
//! libviprs/libviprs#340 / PR #340).
//!
//! `composite2` writes a mixed-depth result at the genuine 16-bit scale
//! whenever a genuine-16 (`Rgb16`/`Grey16`) input is present — so a
//! half-opaque genuine-16 overlay's alpha correctly survives above the 255
//! ceiling on the FIRST composite. But the result is tagged with the *base's*
//! interpretation (`out.meta = self.meta`), not the genuine-16 compositing
//! space it was actually written at. Chaining that result back through
//! `composite2` therefore re-reads it on the 255 scale and clamps the alpha
//! back to the 255 ceiling — silently re-introducing #289 one hop downstream,
//! and leaving the saved metadata inconsistent with the samples.
//!
//! Pure public-API test. It is RED against the current (buggy) core and turns
//! GREEN once the core follow-up stamps the output interpretation to match its
//! write-back scale.

use libviprs::{CompositeMode, Interpretation, PixelFormat, Raster};

/// A 1x1 genuine 16-bit RGBA raster explicitly tagged [`Interpretation::Rgb16`]
/// (0..65535), as distinct from a promoted-8-bit-in-16-bit buffer (which the
/// crate leaves untagged).
fn genuine_rgba16(colour: [u16; 3], alpha: u16) -> Raster {
    let data: Vec<u8> = [colour[0], colour[1], colour[2], alpha]
        .iter()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    Raster::new(1, 1, PixelFormat::Rgba16, data)
        .unwrap()
        .copy()
        .interpretation(Interpretation::Rgb16)
        .build()
}

/// A genuine-16 composite result must stay usable as a genuine-16 layer when
/// fed back into `composite2`: its alpha (and colour) must survive on the
/// 16-bit scale, not collapse back to the 255 ceiling.
///
/// Step 1 (already correct under #340): compose a genuine-16 half-opaque
/// overlay over an 8-bit base with `Source`. The output is `Rgba16` and its
/// alpha lands near `0x8000` (~32768), far above 255.
///
/// Step 2 (the gap): feed that result back as the overlay of another
/// `Source` composite. If the result carried the genuine-16 tag it was written
/// at, it re-reads on the 65535 scale and its alpha survives (~0x8000). Under
/// the bug it is tagged with the 8-bit base's interpretation, so `composite2`
/// treats it as untagged/non-genuine, re-reads it on the 255 scale, and clamps
/// the alpha to the 255 ceiling.
#[test]
fn genuine_16bit_composite_result_survives_rechain() {
    // Step 1: 8-bit base + genuine-16 half-opaque overlay.
    let base8 = Raster::new(1, 1, PixelFormat::Rgba8, vec![40, 50, 60, 255]).unwrap();
    let overlay = genuine_rgba16([10000, 20000, 30000], 0x8000);
    let mid = base8.composite(&overlay, CompositeMode::Source);
    assert_eq!(
        mid.format(),
        PixelFormat::Rgba16,
        "mid format: {:?}",
        mid.format()
    );
    let mpx = mid.getpoint(0, 0);
    // The #340 fix already honours #289 on the first hop: alpha well above 255.
    assert!(
        mpx[3] > 255.0,
        "first composite alpha must exceed the 255 ceiling: {mpx:?}"
    );
    assert!(
        (mpx[3] - 32768.0).abs() < 256.0,
        "first composite alpha should be ~0x8000: {mpx:?}"
    );

    // Step 2: re-chain `mid` as a genuine-16 overlay via Source over any base.
    let base2 = Raster::new(1, 1, PixelFormat::Rgba8, vec![0, 0, 0, 255]).unwrap();
    let out = base2.composite(&mid, CompositeMode::Source);
    let px = out.getpoint(0, 0);
    assert!(
        px[3] > 255.0,
        "re-chained genuine-16 alpha must survive the 255 ceiling, got {px:?} \
         — the mixed-depth result was written at the 16-bit scale but tagged \
         with the base's non-genuine-16 interpretation, so composite2 re-read \
         it at 255 and re-introduced #289"
    );
    assert!(
        (px[3] - 32768.0).abs() < 256.0,
        "re-chained alpha should stay ~0x8000: {px:?}"
    );
    // The overlay blue channel must likewise re-read on the 16-bit scale.
    assert!(
        (px[2] - 30000.0).abs() < 256.0,
        "re-chained blue should stay ~30000 (16-bit scale), got {px:?}"
    );
}
