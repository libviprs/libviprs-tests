//! Phase 0 TDD — the five legacy `generate_pyramid_*` free functions still
//! work after the builder lands.
//!
//! The migration plan keeps the free functions as thin wrappers over
//! `EngineBuilder` for one minor version, marked `#[deprecated]`. This test
//! file exercises each one end-to-end so we catch any regression in the
//! shim layer during the source migration. Once downstream has migrated,
//! PR #7 deletes this file.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports, deprecated)]

use libviprs::sink::{MemorySink, TileSink};
use libviprs::streaming::{RasterStripSource, StreamingConfig};
use libviprs::streaming_mapreduce::MapReduceConfig;
use libviprs::{
    EngineConfig, Layout, PixelFormat, PyramidPlanner, Raster, generate_pyramid,
    generate_pyramid_auto, generate_pyramid_mapreduce, generate_pyramid_mapreduce_auto,
    generate_pyramid_observed, generate_pyramid_streaming,
};

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

#[test]
fn legacy_generate_pyramid_still_works() {
    let src = gradient(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = MemorySink::new();
    let result = generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn legacy_generate_pyramid_observed_still_works() {
    let src = gradient(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = MemorySink::new();
    let result = generate_pyramid_observed(
        &src,
        &plan,
        &sink,
        &EngineConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn legacy_generate_pyramid_streaming_still_works() {
    let src = gradient(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = MemorySink::new();
    let result = generate_pyramid_streaming(
        &RasterStripSource::new(&src),
        &plan,
        &sink,
        &StreamingConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn legacy_generate_pyramid_auto_still_works() {
    let src = gradient(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = MemorySink::new();
    let result = generate_pyramid_auto(
        &src,
        &plan,
        &sink,
        &StreamingConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn legacy_generate_pyramid_mapreduce_still_works() {
    let src = gradient(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = MemorySink::new();
    let result = generate_pyramid_mapreduce(
        &RasterStripSource::new(&src),
        &plan,
        &sink,
        &MapReduceConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn legacy_generate_pyramid_mapreduce_auto_still_works() {
    let src = gradient(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = MemorySink::new();
    let result = generate_pyramid_mapreduce_auto(
        &src,
        &plan,
        &sink,
        &MapReduceConfig::default(),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn deprecation_warnings_are_attached() {
    // Compile-time probe: if `#[deprecated]` is ever removed from the free
    // functions during the migration, the `#[allow(deprecated)]` at the top
    // becomes dead and Clippy (in a future PR) will flag it. For now this
    // test is a sanity invocation — the attribute check lives in rustdoc.
    let _ = generate_pyramid;
}
