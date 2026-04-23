//! Phase 0 TDD — match arms feed the builder cleanly.
//!
//! A user-facing promise in the design: modifiers whose values are owned
//! typed values (`RetryPolicy`, `ResumePolicy`, `FailurePolicy`,
//! `BlankTileStrategy`, `EngineKind`, `DedupeStrategy`) must be composable
//! through `match` arms without type gymnastics. That rules out typestate
//! on `EngineBuilder` itself for those knobs — each `.with_*` must keep
//! the builder type unchanged.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use libviprs::dedupe::DedupeStrategy;
use libviprs::engine::BlankTileStrategy;
use libviprs::manifest::ChecksumAlgo;
use libviprs::retry::{FailurePolicy, RetryPolicy};
use libviprs::sink::{FsSink, MemorySink, TileFormat};
use libviprs::{
    EngineBuilder, EngineKind, Layout, PixelFormat, PyramidPlanner, Raster, ResumePolicy,
};

fn gradient_raster(w: u32, h: u32) -> Raster {
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

enum Mode {
    Fast,
    Safe,
    Audit,
}

#[test]
fn retry_through_match() {
    let mode = Mode::Safe;
    let retry = match mode {
        Mode::Fast => RetryPolicy::fail_fast(),
        Mode::Safe => RetryPolicy::default().with_max(5),
        Mode::Audit => RetryPolicy::default().with_max(10).with_jitter(false),
    };

    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    EngineBuilder::new(&src, plan, MemorySink::new())
        .with_retry(retry)
        .run()
        .unwrap();
}

#[test]
fn resume_through_match() {
    let mode = Mode::Audit;
    let resume = match mode {
        Mode::Fast => ResumePolicy::overwrite(),
        Mode::Safe => ResumePolicy::resume().with_checkpoint_every(100),
        Mode::Audit => ResumePolicy::verify(),
    };

    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    EngineBuilder::new(&src, plan, MemorySink::new())
        .with_resume(resume)
        .run()
        .unwrap();
}

#[test]
fn failure_policy_through_match() {
    let mode = Mode::Safe;
    let policy = match mode {
        Mode::Fast => FailurePolicy::FailFast,
        Mode::Safe => FailurePolicy::RetryThenFail(RetryPolicy::default().with_max(3)),
        Mode::Audit => FailurePolicy::RetryThenSkip(RetryPolicy::default().with_max(10)),
    };

    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    EngineBuilder::new(&src, plan, MemorySink::new())
        .with_failure_policy(policy)
        .run()
        .unwrap();
}

#[test]
fn engine_kind_through_match() {
    let parallel = true;
    let kind = match parallel {
        true => EngineKind::MapReduce,
        false => EngineKind::Streaming,
    };

    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    EngineBuilder::new(&src, plan, MemorySink::new())
        .with_engine(kind)
        .run()
        .unwrap();
}

#[test]
fn dedupe_through_match() {
    let strat = match Some(ChecksumAlgo::Blake3) {
        Some(algo) => DedupeStrategy::All { algo },
        None => DedupeStrategy::Blanks,
    };

    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    EngineBuilder::new(&src, plan, MemorySink::new())
        .with_dedupe(strat)
        .run()
        .unwrap();
}

#[test]
fn blank_strategy_through_match() {
    let tol = Some(2u8);
    let bts = match tol {
        Some(0) | None => BlankTileStrategy::Placeholder,
        Some(t) => BlankTileStrategy::PlaceholderWithTolerance {
            max_channel_delta: t,
        },
    };

    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    EngineBuilder::new(&src, plan, MemorySink::new())
        .with_blank_strategy(bts)
        .run()
        .unwrap();
}

#[test]
fn boxed_sink_through_match() {
    // Different sink types unified through `Box<dyn TileSink>` at the call
    // site — the pattern external users will reach for most often.
    let pick = "fs";
    let dir = tempfile::tempdir().unwrap();
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let sink: Box<dyn libviprs::sink::TileSink> = match pick {
        "mem" => Box::new(MemorySink::new()),
        "fs" => {
            Box::new(FsSink::new(dir.path().join("out"), plan.clone()).with_format(TileFormat::Png))
        }
        _ => unreachable!(),
    };

    let src = gradient_raster(64, 64);
    EngineBuilder::new(&src, plan, sink).run().unwrap();
}
