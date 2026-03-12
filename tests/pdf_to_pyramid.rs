//! End-to-end integration test: PDF → extract image → geo-reference → tile pyramid.
//!
//! This is the core workflow from issue #142: take a blueprint PDF, extract
//! the raster image, attach a geo-transform, and generate a tile pyramid.

use std::path::Path;

use libviprs::{
    EngineConfig, FsSink, GeoCoord, GeoTransform, Layout, MemorySink, PixelFormat, PyramidPlanner,
    TileFormat, extract_page_image, generate_pyramid,
};

const FIXTURE_PDF: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/blueprint.pdf");

/// Full pipeline: PDF → raster → geo-reference → pyramid (in-memory sink).
#[test]
fn pdf_to_georeferenced_pyramid_memory() {
    // Step 1: Extract raster from PDF
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let w = raster.width();
    let h = raster.height();
    assert!(w > 0 && h > 0, "Extracted raster has zero dimensions");

    // Step 2: Attach geo-reference (simulated construction site coordinates)
    // Origin at top-left, 0.01 degrees per pixel, Y inverted
    let geo = GeoTransform::from_origin_and_scale(
        GeoCoord::new(-122.4194, 37.7749), // San Francisco
        0.0001,                            // ~11m per pixel at this latitude
        -0.0001,
    );

    // Verify geo bounds cover the image
    let bounds = geo.image_bounds(w, h);
    assert!(bounds.width() > 0.0);
    assert!(bounds.height() > 0.0);
    assert!(
        bounds
            .contains(geo.pixel_to_geo(libviprs::PixelCoord::new(w as f64 / 2.0, h as f64 / 2.0,)))
    );

    // Step 3: Generate tile pyramid
    let tile_size = 256;
    let planner = PyramidPlanner::new(w, h, tile_size, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink = MemorySink::new();
    let config = EngineConfig::default().with_concurrency(4);

    let result =
        generate_pyramid(&raster, &plan, &sink, &config).expect("pyramid generation failed");

    // Verify all tiles produced
    assert_eq!(result.tiles_produced, plan.total_tile_count());
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());

    // Step 4: Verify geo-coordinates for interior tile centers are within bounds.
    // Edge tiles may extend past the image pixel extent (partial tiles), so we
    // only check tiles whose pixel-space center is within the image.
    let top_level = plan.levels.last().unwrap();
    for row in 0..top_level.rows {
        for col in 0..top_level.cols {
            let px_center_x = (col as f64 + 0.5) * tile_size as f64;
            let px_center_y = (row as f64 + 0.5) * tile_size as f64;
            if px_center_x >= w as f64 || px_center_y >= h as f64 {
                continue; // Edge tile extends past image
            }
            let center = geo.tile_center(col, row, tile_size);
            assert!(
                bounds.contains(center),
                "Tile ({col}, {row}) center ({}, {}) outside image bounds",
                center.x,
                center.y,
            );
        }
    }
}

/// Full pipeline to filesystem with PNG encoding.
#[test]
fn pdf_to_pyramid_filesystem_png() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let w = raster.width();
    let h = raster.height();

    let planner = PyramidPlanner::new(w, h, 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("blueprint_tiles");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default().with_concurrency(4);

    let result =
        generate_pyramid(&raster, &plan, &sink, &config).expect("pyramid generation failed");

    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // Verify DZI manifest
    let dzi_path = dir.path().join("blueprint_tiles.dzi");
    assert!(dzi_path.exists(), "DZI manifest not found");
    let manifest = std::fs::read_to_string(&dzi_path).unwrap();
    assert!(manifest.contains(&format!("Width=\"{w}\"")));
    assert!(manifest.contains(&format!("Height=\"{h}\"")));

    // Verify top-level tile is valid PNG
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0_0.png", top.level));
    assert!(tile_path.exists(), "Top-level tile not found");
    let bytes = std::fs::read(&tile_path).unwrap();
    assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
}

/// Verify determinism: same PDF produces identical pyramids across runs.
#[test]
fn pdf_pyramid_deterministic() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    let planner = PyramidPlanner::new(raster.width(), raster.height(), 256, 0, Layout::DeepZoom)
        .expect("failed to create pyramid planner");
    let plan = planner.plan();

    let sink1 = MemorySink::new();
    let sink2 = MemorySink::new();
    let config = EngineConfig::default().with_concurrency(4);

    generate_pyramid(&raster, &plan, &sink1, &config).unwrap();
    generate_pyramid(&raster, &plan, &sink2, &config).unwrap();

    let mut tiles1 = sink1.tiles();
    let mut tiles2 = sink2.tiles();
    tiles1.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
    tiles2.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    assert_eq!(tiles1.len(), tiles2.len());
    for (t1, t2) in tiles1.iter().zip(tiles2.iter()) {
        assert_eq!(t1.coord, t2.coord);
        assert_eq!(t1.data, t2.data, "Tile {:?} differs between runs", t1.coord);
    }
}

/// Verify extracted raster pixel format is usable for pyramiding.
#[test]
fn pdf_raster_format_compatible() {
    let raster = extract_page_image(Path::new(FIXTURE_PDF), 1)
        .expect("failed to extract image from blueprint PDF");

    // Should be RGB8 or RGBA8 — formats the engine handles
    let fmt = raster.format();
    assert!(
        fmt == PixelFormat::Rgb8 || fmt == PixelFormat::Rgba8 || fmt == PixelFormat::Gray8,
        "Unexpected pixel format from PDF extraction: {:?}",
        fmt
    );
}
