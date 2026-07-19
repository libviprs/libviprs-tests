//! Public-API hygiene guards for libviprs#299.
//!
//! The expert panel flagged four "ergonomic burrs" in the public API:
//!
//! 1. `SlowSink` — a sink that *sleeps* in `write_tile`, explicitly a testing
//!    double — was unconditionally public (`libviprs::sink::SlowSink`).
//! 2. `MemorySink::tiles()` clones every collected tile's pixel bytes on each
//!    call, making progress polling O(pyramid bytes) with no cheap read path.
//! 3. `Raster` derives `Clone` while owning a multi-gigabyte pixel buffer, so a
//!    stray `.clone()` silently duplicates the whole image — undocumented.
//! 4. `getpoint` allocates a fresh `Vec<f64>` per pixel read — undocumented.
//!
//! The fix is behaviour-preserving: gate `SlowSink` behind the `test-util`
//! Cargo feature (not `cfg(test)` alone — this external suite needs it), add a
//! borrowing [`MemorySink::with_tiles`] accessor, and document the clone /
//! allocation costs. There is no pixel behaviour to express as a vips
//! differential, so — in the style of `feature_rename_docs_present.rs` — the
//! packaging/documentation guards assert on the core crate's source text and
//! `Cargo.toml`, and the one behavioural guard (`with_tiles`) exercises the new
//! accessor against the owned `tiles()` snapshot at runtime.

use std::path::PathBuf;

use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, MemorySink, PyramidPlanner,
    generate_test_raster,
};

// ---------------------------------------------------------------------------
// Source-scanning helpers (mirrors feature_rename_docs_present.rs)
// ---------------------------------------------------------------------------

/// Absolute path to a file in the core `libviprs` crate (the path-dep this
/// test crate compiles against).
fn core_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../libviprs")
        .join(rel)
}

/// Read a core-crate file, lowercase it, and collapse all runs of whitespace
/// to a single space so substring checks are robust to line wrapping.
fn normalised(rel: &str) -> String {
    let path = core_path(rel);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Build a small RGB pyramid into `sink` from a generated gradient raster.
/// Returns the number of tiles the plan expected, so callers can cross-check.
fn run_small_pyramid<S: libviprs::TileSink>(sink: &S) -> u64 {
    let raster = generate_test_raster(96, 96).expect("generate_test_raster");
    let plan = PyramidPlanner::new(96, 96, 32, 0, Layout::DeepZoom)
        .expect("planner")
        .plan();
    let result = EngineBuilder::new(&raster, plan.clone(), sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .expect("pyramid generation failed");
    assert_eq!(result.tiles_produced, plan.total_tile_count());
    plan.total_tile_count()
}

// ---------------------------------------------------------------------------
// #299 item 1 — SlowSink is gated behind `test-util`, not unconditionally pub
// ---------------------------------------------------------------------------

/// Behavioural/packaging anchor: the core crate must declare a `test-util`
/// feature, and `SlowSink` must be gated on it (plus `cfg(test)` for the core's
/// own unit tests) rather than shipping as an unconditional public type.
#[test]
fn slow_sink_gated_behind_test_util_feature() {
    // The feature must exist in Cargo.toml.
    let cargo = core_path("Cargo.toml");
    let raw = std::fs::read_to_string(&cargo)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", cargo.display()));
    assert!(
        raw.lines()
            .any(|l| l.trim_start().starts_with("test-util = [")),
        "core Cargo.toml has no `test-util` feature (libviprs#299)"
    );

    // `SlowSink` must be gated: the `pub struct SlowSink` declaration must be
    // immediately preceded by a `cfg(... feature = \"test-util\" ...)` gate, so
    // it no longer ships in a default build. `cfg(test)` alone is rejected by
    // the panel (the external suite needs the feature), so the gate names the
    // feature explicitly.
    let sink_src = std::fs::read_to_string(core_path("src/sink.rs")).expect("read sink.rs");
    let lines: Vec<&str> = sink_src.lines().collect();
    let decl = lines
        .iter()
        .position(|l| l.trim_start().starts_with("pub struct SlowSink"))
        .expect("src/sink.rs no longer declares `pub struct SlowSink`");
    // Look at the small window of attributes above the declaration.
    let window_start = decl.saturating_sub(6);
    let attrs = lines[window_start..decl].join("\n");
    assert!(
        attrs.contains("feature = \"test-util\""),
        "`pub struct SlowSink` is not gated behind `feature = \"test-util\"` \
         (attributes above it: {attrs:?}) (libviprs#299)"
    );
}

/// Runtime proof that `SlowSink` is reachable *through* the `test-util` feature
/// gate and still functions as a `TileSink`. Compiled only when the feature is
/// on (this crate turns it on by default; see Cargo.toml).
#[cfg(feature = "test-util")]
#[test]
fn slow_sink_reachable_and_functional_via_test_util() {
    use libviprs::sink::SlowSink;

    let sink = SlowSink::new(std::time::Duration::from_micros(1));
    let expected = run_small_pyramid(&sink);
    assert_eq!(
        sink.tile_count() as u64,
        expected,
        "SlowSink collected {} tiles, expected {expected}",
        sink.tile_count()
    );
}

// ---------------------------------------------------------------------------
// #299 item 2 — MemorySink gains a borrowing accessor
// ---------------------------------------------------------------------------

/// `MemorySink::with_tiles` borrows the collected tiles without cloning and
/// observes exactly the same tiles as the owned `tiles()` snapshot.
#[test]
fn memory_sink_with_tiles_matches_owned_snapshot() {
    let sink = MemorySink::new();
    let expected = run_small_pyramid(&sink);

    // Borrowing accessor: count and coordinate-set are derived without an
    // owned clone.
    let (borrowed_count, mut borrowed_coords) = sink.with_tiles(|tiles| {
        let coords: Vec<_> = tiles.iter().map(|t| t.coord).collect();
        (tiles.len(), coords)
    });
    assert_eq!(borrowed_count as u64, expected);

    // Owned snapshot: must describe the identical set of tiles.
    let owned = sink.tiles();
    let mut owned_coords: Vec<_> = owned.iter().map(|t| t.coord).collect();

    borrowed_coords.sort_by_key(|c| (c.level, c.row, c.col));
    owned_coords.sort_by_key(|c| (c.level, c.row, c.col));
    assert_eq!(
        borrowed_coords, owned_coords,
        "with_tiles() and tiles() disagree on the collected tile set"
    );

    // The borrowed view must see identical pixel bytes per coordinate (proving
    // it exposes the real collected data, not an empty/placeholder slice).
    for owned_tile in &owned {
        let matches = sink.with_tiles(|tiles| {
            tiles
                .iter()
                .find(|t| t.coord == owned_tile.coord)
                .map(|t| t.data == owned_tile.data)
                .unwrap_or(false)
        });
        assert!(
            matches,
            "with_tiles() data mismatch at {:?}",
            owned_tile.coord
        );
    }
}

// ---------------------------------------------------------------------------
// #299 items 3 & 4 — clone / allocation costs are documented
// ---------------------------------------------------------------------------

/// The `Raster` rustdoc must warn that `Clone` copies the whole pixel buffer.
#[test]
fn raster_clone_cost_documented() {
    let src = normalised("src/raster.rs");
    assert!(
        src.contains("# cloning") || src.contains("cloning is"),
        "src/raster.rs does not document the Raster::clone cost (libviprs#299)"
    );
    assert!(
        src.contains("copies the entire owned pixel buffer")
            || src.contains("copies the entire pixel buffer"),
        "Raster clone doc does not spell out that cloning copies the whole buffer (libviprs#299)"
    );
}

/// The `getpoint` rustdoc must warn that it allocates a `Vec<f64>` per call.
#[test]
fn getpoint_allocation_cost_documented() {
    let src = normalised("src/raster_ops.rs");
    assert!(
        src.contains("# performance"),
        "getpoint has no Performance rustdoc section documenting its cost (libviprs#299)"
    );
    assert!(
        src.contains("allocates a fresh `vec<f64>`"),
        "getpoint doc does not state it allocates a fresh Vec<f64> per call (libviprs#299)"
    );
}
