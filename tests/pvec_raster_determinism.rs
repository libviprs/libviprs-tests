//! The same fixture at 1, 2 and 8 workers and in both emission orders is a byte-identical `.pvec`
//! (libviprs-tests#305, the end-to-end half of libviprs/libviprs#1331's claim).
//!
//! The sink's own tests (in the core) use a synthetic raster. This runs the committed scan, with
//! its odd size and its blank quarter, through the same sink, and compares the whole file, not
//! the tiles: the placement, the index and the Hilbert layout have to agree too. Needs
//! `--features pvec-raster`.

#![cfg(feature = "pvec-raster")]

#[path = "common/pvec_raster.rs"]
mod fx;

use libviprs::raster_pvec::container::write_rasters;
use libviprs::raster_pvec::pyramid::{
    RasterPlacement, RasterPyramidOptions, prepare_raster, produce_raster_pyramid,
};
use libviprs::sink_pvec::PvecSink;
use libviprs::{EmissionOrder, EngineBuilder, EngineKind, SinkError, Tile, TileSink};

fn options() -> RasterPyramidOptions {
    let mut o = RasterPyramidOptions::new(RasterPlacement::Explicit([
        1.0, 0.0, 0.0, -1.0, 0.0, 1031.0,
    ]));
    o.skip_blanks = true;
    o
}

/// Hands the tiles to the sink and asks the engine for the order it was constructed with.
struct Ordered<'a>(&'a PvecSink, EmissionOrder);

impl TileSink for Ordered<'_> {
    fn write_tile(&self, tile: &Tile) -> Result<(), SinkError> {
        self.0.write_tile(tile)
    }
    fn emission_order(&self) -> EmissionOrder {
        self.1
    }
}

fn through_sink(engine: EngineKind, workers: usize, order: EmissionOrder) -> Vec<u8> {
    let raster = fx::load("scan_gray_1537x1031.png");
    let job = prepare_raster(&raster, &fx::plan(1537, 1031, 256), &options()).unwrap();
    let sink = PvecSink::new(fx::drawing_writer(), &job.pyramid, &options()).unwrap();
    EngineBuilder::new(
        job.source.as_ref(),
        job.pyramid.plan.clone(),
        Ordered(&sink, order),
    )
    .with_engine(engine)
    .with_concurrency(workers)
    .with_skip_blanks(true)
    .run()
    .unwrap();
    sink.into_writer().finish().unwrap()
}

#[test]
fn one_two_and_eight_workers_and_both_emission_orders_write_the_same_bytes() {
    let reference = through_sink(EngineKind::Monolithic, 1, EmissionOrder::Cascade);
    for (workers, order) in [
        (2, EmissionOrder::Cascade),
        (8, EmissionOrder::Cascade),
        (1, EmissionOrder::TileId),
        (2, EmissionOrder::TileId),
        (8, EmissionOrder::TileId),
    ] {
        assert!(
            through_sink(EngineKind::Monolithic, workers, order) == reference,
            "x{workers} {order:?} changed the bytes"
        );
    }
}

#[test]
fn the_streaming_and_map_reduce_engines_are_each_stable_across_worker_counts() {
    // The engines round the overviews differently from each other, which is theirs to settle; what
    // the sink promises is that for one engine the worker count does not change a byte.
    for engine in [EngineKind::Streaming, EngineKind::MapReduce] {
        let one = through_sink(engine, 1, EmissionOrder::Cascade);
        assert!(
            through_sink(engine, 8, EmissionOrder::Cascade) == one,
            "{engine:?}: 8 workers differ from 1"
        );
    }
}

#[test]
fn the_sink_writes_what_the_collecting_path_writes() {
    let raster = fx::load("scan_gray_1537x1031.png");
    let pyramid = produce_raster_pyramid(&raster, &fx::plan(1537, 1031, 256), &options()).unwrap();
    let mut w = fx::drawing_writer();
    write_rasters(&mut w, std::slice::from_ref(&pyramid)).unwrap();
    assert!(
        w.finish().unwrap() == through_sink(EngineKind::Monolithic, 8, EmissionOrder::Cascade),
        "the streaming sink and produce_raster_pyramid + write_rasters disagree"
    );
}

#[test]
fn the_control_a_different_fixture_is_a_different_file() {
    // Otherwise "the same bytes" could be a file that ignores the raster.
    let a = through_sink(EngineKind::Monolithic, 2, EmissionOrder::Cascade);
    assert_ne!(
        a,
        fx::drawing_pvec(),
        "the raster-free twin is not the raster file"
    );
    let other = fx::load("scan_1bit_1537x1031.png");
    let job = prepare_raster(&other, &fx::plan(1537, 1031, 256), &options()).unwrap();
    let sink = PvecSink::new(fx::drawing_writer(), &job.pyramid, &options()).unwrap();
    EngineBuilder::new(job.source.as_ref(), job.pyramid.plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_skip_blanks(true)
        .run()
        .unwrap();
    assert_ne!(sink.into_writer().finish().unwrap(), a);
}
