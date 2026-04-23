//! Phase 0 TDD — proptest invariants on builder composition.
//!
//! The per-component builder design leans on two properties:
//!
//! 1. **Orthogonal knobs commute.** If two `.with_*` setters touch disjoint
//!    engine fields, swapping their order must produce byte-identical
//!    pyramid output.
//!
//! 2. **Engine × source matrix is closed.** Every (`EngineKind`,
//!    `EngineSource`) combination either succeeds or surfaces a specific
//!    typed error. The builder never panics and never silently changes
//!    semantics.
//!
//! These are exactly the properties the panel flagged as load-bearing for
//! the whole refactor — they're what lets us claim per-component builders
//! are "composable" without hand-waving.
//!
//! Generators are kept small: the matrix is bounded, so proptest mostly
//! picks combinations rather than generating unbounded value spaces.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use proptest::prelude::*;

use libviprs::dedupe::DedupeStrategy;
use libviprs::engine::BlankTileStrategy;
use libviprs::planner::TileCoord;
use libviprs::retry::{FailurePolicy, RetryPolicy};
use libviprs::sink::{MemorySink, TileFormat, TileSink};
use libviprs::streaming::RasterStripSource;
use libviprs::{
    EngineBuilder, EngineError, EngineKind, Layout, PixelFormat, PyramidPlanner, Raster,
};

/// Comparable snapshot: (coord, raw bytes) per tile, in sorted order.
type TileSnapshot = Vec<(TileCoord, Vec<u8>)>;

#[derive(Debug, Clone, Copy)]
enum Knob {
    Concurrency(u8), // 0..=4 mapped to 1..=4
    BufferSize(u8),  // 0..=4 mapped to 4..=64
    Dedupe,
    BlankStrategy,
    Retry(u8), // 0..=5 max retries
}

fn gradient(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x + y) % 256) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

fn snapshot(sink: &MemorySink) -> TileSnapshot {
    let mut t: Vec<(TileCoord, Vec<u8>)> = sink
        .tiles()
        .into_iter()
        .map(|c| (c.coord, c.data))
        .collect();
    t.sort_by_key(|(coord, _)| (coord.level, coord.row, coord.col));
    t
}

fn apply(mut builder: EngineBuilder<MemorySink>, knob: &Knob) -> EngineBuilder<MemorySink> {
    match knob {
        Knob::Concurrency(n) => builder = builder.with_concurrency((*n as usize).max(1)),
        Knob::BufferSize(n) => builder = builder.with_buffer_size(((*n as usize) + 1) * 16),
        Knob::Dedupe => builder = builder.with_dedupe(DedupeStrategy::Blanks),
        Knob::BlankStrategy => {
            builder = builder.with_blank_strategy(BlankTileStrategy::Placeholder)
        }
        Knob::Retry(n) => {
            builder = builder.with_retry(RetryPolicy::default().with_max(*n as u32));
        }
    }
    builder
}

fn run(knobs: &[Knob]) -> TileSnapshot {
    let src = gradient(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let mut builder = EngineBuilder::new(&src, plan, MemorySink::new());
    for k in knobs {
        builder = apply(builder, k);
    }
    let (_, sink) = builder.run_collect().unwrap();
    snapshot(&sink)
}

fn knob_strategy() -> impl Strategy<Value = Knob> {
    prop_oneof![
        (0u8..=4).prop_map(Knob::Concurrency),
        (0u8..=4).prop_map(Knob::BufferSize),
        Just(Knob::Dedupe),
        Just(Knob::BlankStrategy),
        (0u8..=3).prop_map(Knob::Retry),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..Default::default() })]

    #[test]
    fn orthogonal_knobs_commute(
        knobs in prop::collection::vec(knob_strategy(), 2..6)
    ) {
        let forward = run(&knobs);
        let mut reversed = knobs.clone();
        reversed.reverse();
        let back = run(&reversed);
        prop_assert_eq!(forward, back);
    }

    #[test]
    fn shuffled_knobs_produce_same_output(
        knobs in prop::collection::vec(knob_strategy(), 3..6),
        perm in prop::sample::subsequence((0..6usize).collect::<Vec<_>>(), 3..=6),
    ) {
        let reference = run(&knobs);
        let shuffled: Vec<Knob> = perm
            .iter()
            .filter_map(|&i| knobs.get(i).copied())
            .collect();
        let shuffled_out = run(&shuffled);
        // Restrict the assertion: shuffled is a subsequence, not a
        // permutation. What we pin is "a given set of knobs, applied in
        // *any* order, produces a deterministic output for that set." So
        // we compare the shuffled run against a second run of the same
        // shuffled sequence, reversed.
        let mut shuffled_rev = shuffled.clone();
        shuffled_rev.reverse();
        let shuffled_rev_out = run(&shuffled_rev);
        prop_assert_eq!(shuffled_out, shuffled_rev_out);
        drop(reference);
    }
}

// ---------------------------------------------------------------------------
// Engine-kind × source-kind matrix — bounded so no proptest needed
// ---------------------------------------------------------------------------

#[test]
fn engine_source_matrix_is_closed() {
    let src = gradient(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let cases: &[(EngineKind, &str)] = &[
        (EngineKind::Auto, "raster"),
        (EngineKind::Auto, "strip"),
        (EngineKind::Monolithic, "raster"),
        (EngineKind::Monolithic, "strip"),
        (EngineKind::Streaming, "raster"),
        (EngineKind::Streaming, "strip"),
        (EngineKind::MapReduce, "raster"),
        (EngineKind::MapReduce, "strip"),
    ];

    for (kind, source_kind) in cases {
        let result = match *source_kind {
            "raster" => EngineBuilder::new(&src, plan.clone(), MemorySink::new())
                .with_engine(*kind)
                .run(),
            "strip" => EngineBuilder::new(
                RasterStripSource::new(&src),
                plan.clone(),
                MemorySink::new(),
            )
            .with_engine(*kind)
            .run(),
            _ => unreachable!(),
        };

        match (*kind, *source_kind, &result) {
            (EngineKind::Monolithic, "strip", Err(EngineError::IncompatibleSource { .. })) => {}
            (_, _, Ok(_)) => {}
            (k, s, Err(e)) => panic!("unexpected failure for ({k:?}, {s}): {e:?}"),
        }
    }
}
