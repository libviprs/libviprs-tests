//! Fixture generator for blank_tile_strategy tests.
//!
//! Run with: `cargo test --test gen_blank_tile_fixtures -- --ignored generate_fixtures`
//!
//! This generates input PNG images and expected output tile pyramids under
//! `tests/fixtures/blank_tile_strategy/`. The generated files should be committed
//! to the repository so that the `blank_tile_strategy` tests can compare against
//! pre-existing expected output rather than generating everything at runtime.

use libviprs::{
    BlankTileStrategy, EngineConfig, FsSink, Layout, PixelFormat, PyramidPlanner, Raster,
    TileFormat, TileSink, generate_pyramid,
};
use std::path::Path;

const FIXTURE_BASE: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blank_tile_strategy");

// -- Image dimensions and tile size shared with the test file --
const IMG_SIZE: u32 = 128;
const TILE_SIZE: u32 = 64;

fn solid_white_raster() -> Raster {
    let data = vec![255u8; IMG_SIZE as usize * IMG_SIZE as usize * 3];
    Raster::new(IMG_SIZE, IMG_SIZE, PixelFormat::Rgb8, data).unwrap()
}

fn gradient_raster() -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; IMG_SIZE as usize * IMG_SIZE as usize * bpp];
    for y in 0..IMG_SIZE {
        for x in 0..IMG_SIZE {
            let off = (y as usize * IMG_SIZE as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x + y) % 256) as u8;
        }
    }
    Raster::new(IMG_SIZE, IMG_SIZE, PixelFormat::Rgb8, data).unwrap()
}

fn half_white_raster() -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![255u8; IMG_SIZE as usize * IMG_SIZE as usize * bpp];
    for y in 0..IMG_SIZE {
        for x in 0..(IMG_SIZE / 2) {
            let off = (y as usize * IMG_SIZE as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x + y) % 256) as u8;
        }
    }
    Raster::new(IMG_SIZE, IMG_SIZE, PixelFormat::Rgb8, data).unwrap()
}

fn save_input_png(raster: &Raster, name: &str) {
    let dir = Path::new(FIXTURE_BASE).join("inputs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    image::save_buffer(
        &path,
        raster.data(),
        raster.width(),
        raster.height(),
        image::ColorType::Rgb8,
    )
    .unwrap();
    eprintln!("  wrote input: {}", path.display());
}

fn generate_pyramid_fixture(
    raster: &Raster,
    strategy: BlankTileStrategy,
    output_name: &str,
) {
    let planner = PyramidPlanner::new(IMG_SIZE, IMG_SIZE, TILE_SIZE, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = Path::new(FIXTURE_BASE).join("expected").join(output_name);
    // Clean previous output
    if base.exists() {
        std::fs::remove_dir_all(&base).unwrap();
    }

    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);
    let config = EngineConfig::default().with_blank_tile_strategy(strategy);

    let result = generate_pyramid(raster, &plan, &sink, &config).unwrap();
    sink.finish().unwrap();

    eprintln!(
        "  wrote expected: {} ({} tiles, {} skipped)",
        base.display(),
        result.tiles_produced,
        result.tiles_skipped,
    );
}

#[test]
#[ignore]
fn generate_fixtures() {
    eprintln!("\n=== Generating blank_tile_strategy fixtures ===\n");

    let white = solid_white_raster();
    let gradient = gradient_raster();
    let half_white = half_white_raster();

    // Save input PNGs
    save_input_png(&white, "solid_white_128x128");
    save_input_png(&gradient, "gradient_128x128");
    save_input_png(&half_white, "half_white_128x128");

    // Generate expected output pyramids
    generate_pyramid_fixture(&white, BlankTileStrategy::Emit, "emit_solid_white");
    generate_pyramid_fixture(&white, BlankTileStrategy::Placeholder, "placeholder_solid_white");
    generate_pyramid_fixture(&gradient, BlankTileStrategy::Emit, "emit_gradient");
    generate_pyramid_fixture(&gradient, BlankTileStrategy::Placeholder, "placeholder_gradient");
    generate_pyramid_fixture(&half_white, BlankTileStrategy::Placeholder, "placeholder_half_white");

    eprintln!("\n=== Done. Commit tests/fixtures/blank_tile_strategy/ ===\n");
}
