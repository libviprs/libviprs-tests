//! Regression coverage for issue #297 — `EngineBuilder::with_config` must
//! compose with earlier fine-grained setters instead of clobbering them.
//!
//! The panel review (issue #270 → #297) confirmed that `with_config`
//! historically overwrote **every** field it carries *unconditionally*, so a
//! coarse `.with_config(cfg)` call silently undid an earlier fine-grained
//! setter. The two headline traps are exactly the settings whose absence
//! surfaces in production as a hung job or a bloated output:
//!
//!  * `.with_cancel(token).with_config(cfg)` — the config's `cancel` is `None`,
//!    so the cooperative-cancellation token is dropped and the run can no
//!    longer be stopped (an unkillable job).
//!  * `.with_skip_blanks(true).with_config(cfg)` — the config's default
//!    `skip_blanks` is `false`, so blank tiles are emitted after all (a
//!    bloated output).
//!
//! These tests assert *observable engine behaviour* (a cancelled run actually
//! cancels; skipped blanks actually vanish from the sink), not builder-field
//! equality, so they survive the internal fill-if-unset refactor.
//!
//! In each clobber test two independent options are set — one via the coarse
//! `with_config` setter, one via a fine-grained setter, in the clobbering
//! order — and BOTH must survive on the built engine.

use libviprs::sink::MemorySink;
use libviprs::{
    CancelToken, EngineBuilder, EngineConfig, EngineError, Layout, PixelFormat, PyramidPlanner,
    Raster,
};

/// A `w`×`h` raster of a single uniform colour. Every extracted tile is
/// therefore blank (exact uniformity), which is what `skip_blanks` acts on.
fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for px in data.chunks_exact_mut(bpp) {
        px.copy_from_slice(&rgb);
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// All-white source. The default `background_rgb` is white too, so even the
/// sub-tile-sized upper pyramid levels — which are padded up to the full tile
/// with the background colour — stay uniformly white and thus blank. That
/// makes `skip_blanks` drop *every* tile, giving a clean empty-sink signal.
const WHITE: [u8; 3] = [255, 255, 255];

fn plan_for(w: u32, h: u32, tile: u32) -> libviprs::planner::PyramidPlan {
    PyramidPlanner::new(w, h, tile, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

/// Trap #1 (hung / unkillable job): a `CancelToken` attached *before*
/// `with_config` must survive the coarse call.
///
/// Two independent options are set in the clobbering order:
///  * option A (fine setter): a *pre-cancelled* cancel token.
///  * option B (coarse setter): a non-default `concurrency` carried on the
///    config — the config legitimately still applies its own fields.
///
/// With the token retained, the monolithic engine polls it at the very first
/// level boundary and returns `EngineError::Cancelled`. Under the old
/// clobbering `with_config`, `self.cancel = config.cancel` (`None`) wiped the
/// token, so the run instead completed successfully — the RED failure.
#[test]
fn cancel_token_set_before_with_config_survives() {
    let src = solid(64, 64, [128, 128, 128]);
    let plan = plan_for(64, 64, 32);

    let token = CancelToken::new();
    token.cancel();

    // config carries option B (concurrency) but no cancel token of its own.
    let cfg = EngineConfig::default().with_concurrency(2);

    let result = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_cancel(token) // option A — fine-grained setter, set first
        .with_config(cfg) // coarse setter carrying option B; its cancel is None
        .run();

    assert!(
        matches!(result, Err(EngineError::Cancelled)),
        "cancel token set before with_config must survive (option A); with_config \
         silently cleared it, so the run completed instead of cancelling: {result:?}"
    );
}

/// Trap #2 (bloated output): `.with_skip_blanks(true)` attached *before*
/// `with_config` must survive the coarse call.
///
/// Two independent options are set in the clobbering order:
///  * option A (fine setter): `skip_blanks = true`.
///  * option B (coarse setter): a non-default `concurrency` on the config
///    (its default `skip_blanks` is `false`).
///
/// The source is a single uniform colour, so every tile at every level is
/// blank. With option A retained, all blank tiles are dropped and the sink is
/// empty; the run also completing proves option B (the config's concurrency)
/// was applied — BOTH survive. Under the old clobbering `with_config`,
/// `self.skip_blanks = Some(config.skip_blanks)` reset it to `false`, so every
/// blank tile was emitted — the RED failure.
#[test]
fn skip_blanks_set_before_with_config_survives() {
    let src = solid(64, 64, WHITE);
    let plan = plan_for(64, 64, 32);

    // config carries option B (concurrency); its skip_blanks is the default false.
    let cfg = EngineConfig::default().with_concurrency(2);

    let (_result, sink) = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_skip_blanks(true) // option A — fine-grained setter, set first
        .with_config(cfg) // coarse setter carrying option B
        .run_collect()
        .expect("run must succeed with the config's concurrency applied (option B)");

    assert!(
        sink.tiles().is_empty(),
        "skip_blanks(true) set before with_config must survive (option A); with_config's \
         default skip_blanks=false clobbered it and {} blank tiles were emitted",
        sink.tiles().len()
    );
}

/// Guard: a fine-grained setter applied *after* `with_config` still wins.
/// This is the documented, intended precedence and the fill-if-unset fix must
/// preserve it. Passes both before and after the fix.
#[test]
fn setter_after_with_config_still_wins() {
    let src = solid(64, 64, WHITE);
    let plan = plan_for(64, 64, 32);

    let cfg = EngineConfig::default(); // skip_blanks = false

    let (_result, sink) = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_config(cfg)
        .with_skip_blanks(true) // applied after with_config -> wins
        .run_collect()
        .unwrap();

    assert!(
        sink.tiles().is_empty(),
        "a setter applied after with_config must win; {} tiles were emitted",
        sink.tiles().len()
    );
}

/// Guard: on a clean builder (no prior fine setters) `with_config` still
/// applies its own fields. Fill-if-unset fills them because every field is
/// still unset. Passes both before and after the fix.
#[test]
fn with_config_applies_its_own_fields_on_clean_chain() {
    let src = solid(64, 64, WHITE);
    let plan = plan_for(64, 64, 32);

    let cfg = EngineConfig::default().skip_blanks(true);

    let (_result, sink) = EngineBuilder::new(&src, plan, MemorySink::new())
        .with_config(cfg)
        .run_collect()
        .unwrap();

    assert!(
        sink.tiles().is_empty(),
        "with_config's own skip_blanks(true) must apply on a clean chain; {} tiles were emitted",
        sink.tiles().len()
    );
}
