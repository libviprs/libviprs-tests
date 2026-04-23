//! Phase 0 TDD — `PackfileSink::builder(path)` fluent builder.
//!
//! Today: `PackfileSink::new(path, format, plan, tile_format) -> Result<…>`
//! — four positional args, easy to swap.
//! Target: `PackfileSink::builder(path).format(..).tile_format(..).build()`
//! with ordering-independent chaining. `format` defaults to `Tar` and
//! `tile_format` defaults to `Png` so the minimum-viable call is
//! `PackfileSink::builder(path).build()`.

#![cfg(all(feature = "builder_v1", feature = "packfile"))]
#![allow(unused_imports)]

use libviprs::sink::{PackfileFormat, PackfileSink, TileFormat, TileSink};
use libviprs::{EngineBuilder, Layout, PixelFormat, PyramidPlanner, Raster};

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

#[test]
fn builder_minimum_viable_call() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.tar");

    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = PackfileSink::builder(&out)
        .plan(plan.clone())
        .build()
        .unwrap();

    let src = gradient_raster(64, 64);
    EngineBuilder::new(&src, plan, sink).run().unwrap();

    assert!(out.exists(), "packfile archive was not created");
}

#[test]
fn builder_accepts_format_and_tile_format() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.zip");

    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();
    let sink = PackfileSink::builder(&out)
        .plan(plan.clone())
        .format(PackfileFormat::Zip)
        .tile_format(TileFormat::Jpeg { quality: 80 })
        .build()
        .unwrap();

    let src = gradient_raster(64, 64);
    EngineBuilder::new(&src, plan, sink).run().unwrap();
}

#[test]
fn builder_is_order_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    let a_out = dir.path().join("a.tar.gz");
    let b_out = dir.path().join("b.tar.gz");
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    let a = PackfileSink::builder(&a_out)
        .format(PackfileFormat::TarGz)
        .tile_format(TileFormat::Png)
        .plan(plan.clone())
        .build()
        .unwrap();
    let b = PackfileSink::builder(&b_out)
        .plan(plan.clone())
        .tile_format(TileFormat::Png)
        .format(PackfileFormat::TarGz)
        .build()
        .unwrap();

    let src = gradient_raster(64, 64);
    EngineBuilder::new(&src, plan.clone(), a).run().unwrap();
    EngineBuilder::new(&src, plan, b).run().unwrap();

    // Both archives exist and are non-empty — the chain order does not matter.
    assert!(std::fs::metadata(&a_out).unwrap().len() > 0);
    assert!(std::fs::metadata(&b_out).unwrap().len() > 0);
}

#[test]
fn build_without_plan_errors() {
    // A plan is required — the build step must refuse to proceed without one
    // rather than panicking at `write_tile` time.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("oops.tar");

    let res = PackfileSink::builder(&out).build();
    assert!(res.is_err(), "builder without plan must surface an error");
}

#[test]
fn deprecated_new_still_works() {
    // Migration alias kept alongside the builder during the transition.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("legacy.tar");
    let plan = PyramidPlanner::new(64, 64, 32, 0, Layout::DeepZoom)
        .unwrap()
        .plan();

    #[allow(deprecated)]
    let sink = PackfileSink::new(&out, PackfileFormat::Tar, plan.clone(), TileFormat::Png).unwrap();

    let src = gradient_raster(64, 64);
    EngineBuilder::new(&src, plan, sink).run().unwrap();
    assert!(out.exists());
}
