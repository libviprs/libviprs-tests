//! `FsSink::new(dir, plan)` with fluent builder methods.
//!
//! `FsSink::new` takes two arguments; the tile encoding format defaults to
//! PNG and is overridden via `.with_format(fmt)`. Every other knob (dedupe,
//! checksums, resume, manifest, blank strategy) is reachable through
//! `with_*` methods that can be composed in any order.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use libviprs::checksum::{ChecksumAlgo, ChecksumMode};
use libviprs::dedupe::DedupeStrategy;
use libviprs::manifest::ManifestBuilder;
use libviprs::sink::{FsSink, TileFormat, TileSink};
use libviprs::{EngineBuilder, EngineConfig, Layout, PyramidPlanner};

mod common;
use common::fixtures::canonical_raster_scaled;

fn count_files(dir: &std::path::Path, ext: &str) -> usize {
    let mut n = 0;
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                n += count_files(&p, ext);
            } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn two_arg_new_defaults_to_png() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(dir.path().join("out"), plan.clone());
    EngineBuilder::new(&src, plan.clone(), sink).run().unwrap();

    let produced = count_files(&dir.path().join("out"), "png");
    assert_eq!(produced as u64, plan.total_tile_count());
}

#[test]
fn with_format_overrides_default() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(dir.path().join("out"), plan.clone())
        .with_format(TileFormat::Jpeg { quality: 85 });
    EngineBuilder::new(&src, plan.clone(), sink).run().unwrap();

    let jpegs = count_files(&dir.path().join("out"), "jpeg");
    let jpgs = count_files(&dir.path().join("out"), "jpg");
    assert_eq!((jpegs + jpgs) as u64, plan.total_tile_count());
    assert_eq!(count_files(&dir.path().join("out"), "png"), 0);
}

#[test]
fn raw_format_via_builder() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink = FsSink::new(dir.path().join("out"), plan.clone()).with_format(TileFormat::Raw);
    EngineBuilder::new(&src, plan.clone(), sink).run().unwrap();

    assert_eq!(
        count_files(&dir.path().join("out"), "raw") as u64,
        plan.total_tile_count()
    );
}

#[test]
fn compose_format_and_dedupe() {
    let dir = tempfile::tempdir().unwrap();
    let _src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let _sink = FsSink::new(dir.path().join("out"), plan)
        .with_format(TileFormat::Png)
        .with_dedupe(DedupeStrategy::Blanks);
}

#[test]
fn compose_format_checksums_manifest_resume() {
    let dir = tempfile::tempdir().unwrap();
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let _sink = FsSink::new(dir.path().join("out"), plan)
        .with_format(TileFormat::Png)
        .with_checksums(ChecksumMode::EmitOnly, ChecksumAlgo::Blake3)
        .with_manifest(ManifestBuilder::new().with_checksums(ChecksumAlgo::Blake3))
        .with_resume(true);
}

#[test]
fn builder_methods_are_order_insensitive_for_orthogonal_knobs() {
    let dir = tempfile::tempdir().unwrap();
    let src = canonical_raster_scaled(128, 128);
    let plan = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    // Two orderings of the same builder chain must produce identical output.
    let a_dir = dir.path().join("a");
    let b_dir = dir.path().join("b");

    let a = FsSink::new(&a_dir, plan.clone())
        .with_format(TileFormat::Png)
        .with_dedupe(DedupeStrategy::Blanks)
        .with_resume(true);
    let b = FsSink::new(&b_dir, plan.clone())
        .with_resume(true)
        .with_dedupe(DedupeStrategy::Blanks)
        .with_format(TileFormat::Png);

    EngineBuilder::new(&src, plan.clone(), a).run().unwrap();
    EngineBuilder::new(&src, plan, b).run().unwrap();

    assert_eq!(
        count_files(&a_dir, "png"),
        count_files(&b_dir, "png"),
        "tile counts differ between orderings"
    );
}
