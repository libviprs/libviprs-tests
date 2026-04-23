//! Phase 0 TDD — `ResumePolicy` + `RetryPolicy` fluent builders.
//!
//! `ResumePolicy` wraps the raw `ResumeMode` enum + checkpoint knobs into a
//! single typed builder. Three mutually-exclusive factories anchor the mode
//! (`overwrite`, `resume`, `verify`); the `with_*` methods add the
//! checkpoint frequency and root.
//!
//! `RetryPolicy` already has `with_max_retries` / `with_initial_backoff` /
//! `with_multiplier` / `with_max_backoff` / `with_jitter`. This test locks
//! the short-form alias `with_max` as a synonym for `with_max_retries`
//! (requested in the design doc) and pins the default constants.

#![cfg(feature = "builder_v1")]
#![allow(unused_imports)]

use std::path::PathBuf;
use std::time::Duration;

use libviprs::resume::ResumeMode;
use libviprs::retry::{FailurePolicy, RetryPolicy};
use libviprs::sink::{FsSink, MemorySink, TileFormat};
use libviprs::{EngineBuilder, Layout, PixelFormat, PyramidPlanner, Raster, ResumePolicy};

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

// ---------------------------------------------------------------------------
// ResumePolicy factories
// ---------------------------------------------------------------------------

#[test]
fn resume_policy_overwrite_factory() {
    let p = ResumePolicy::overwrite();
    assert_eq!(p.mode(), ResumeMode::Overwrite);
}

#[test]
fn resume_policy_resume_factory() {
    let p = ResumePolicy::resume();
    assert_eq!(p.mode(), ResumeMode::Resume);
}

#[test]
fn resume_policy_verify_factory() {
    let p = ResumePolicy::verify();
    assert_eq!(p.mode(), ResumeMode::Verify);
}

#[test]
fn resume_policy_checkpoint_every_is_threaded() {
    let p = ResumePolicy::resume().with_checkpoint_every(100);
    assert_eq!(p.checkpoint_every(), 100);
    assert_eq!(p.mode(), ResumeMode::Resume);
}

#[test]
fn resume_policy_checkpoint_root_is_threaded() {
    let root = PathBuf::from("/tmp/libviprs-ckpt");
    let p = ResumePolicy::resume().with_checkpoint_root(&root);
    assert_eq!(p.checkpoint_root(), Some(root.as_path()));
}

#[test]
fn resume_policy_default_mode_is_overwrite() {
    // `ResumePolicy::default()` — fresh run, no resume state consulted.
    let p = ResumePolicy::default();
    assert_eq!(p.mode(), ResumeMode::Overwrite);
    assert_eq!(p.checkpoint_every(), 0);
    assert!(p.checkpoint_root().is_none());
}

#[test]
fn builder_accepts_resume_policy() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = FsSink::new(dir.path().join("out"), plan.clone())
        .with_format(TileFormat::Png)
        .with_resume(true);

    EngineBuilder::new(&src, plan, sink)
        .with_resume(ResumePolicy::overwrite())
        .run()
        .unwrap();
}

// ---------------------------------------------------------------------------
// RetryPolicy short-form alias
// ---------------------------------------------------------------------------

#[test]
fn retry_policy_with_max_is_alias_for_with_max_retries() {
    // The design doc uses `.with_max(n)` in examples. Provide it as a thin
    // synonym for the existing `.with_max_retries(n)` so both names work.
    let a = RetryPolicy::default().with_max(5);
    let b = RetryPolicy::default().with_max_retries(5);
    assert_eq!(a, b);
}

#[test]
fn retry_policy_default_constants() {
    let p = RetryPolicy::default();
    assert_eq!(p.max_retries, 3);
    assert_eq!(p.initial_backoff, Duration::from_millis(50));
    assert_eq!(p.max_backoff, Duration::from_secs(5));
    assert!(p.jitter);
}

#[test]
fn retry_policy_fail_fast_convenience() {
    // `RetryPolicy::fail_fast()` — shorthand for max_retries = 0.
    let p = RetryPolicy::fail_fast();
    assert_eq!(p.max_retries, 0);
}

#[test]
fn builder_with_retry_lowers_to_retry_then_fail() {
    // `.with_retry(policy)` is a shorthand for
    // `.with_failure_policy(FailurePolicy::RetryThenFail(policy))`.
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let (_, _) = EngineBuilder::new(&src, plan.clone(), MemorySink::new())
        .with_retry(RetryPolicy::default().with_max(3))
        .run_collect()
        .unwrap();
}

#[test]
fn builder_with_failure_policy_accepts_every_variant() {
    let src = gradient_raster(64, 64);
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    for policy in [
        FailurePolicy::FailFast,
        FailurePolicy::RetryThenFail(RetryPolicy::default()),
        FailurePolicy::RetryThenSkip(RetryPolicy::default()),
    ] {
        EngineBuilder::new(&src, plan.clone(), MemorySink::new())
            .with_failure_policy(policy)
            .run()
            .unwrap();
    }
}
