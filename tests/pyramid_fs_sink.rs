use libviprs::{
    EngineConfig, FsSink, Layout, PixelFormat, PyramidPlanner, Raster, TileFormat, generate_pyramid,
};
use std::path::Path;

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

fn count_files(dir: &Path, ext: &str) -> usize {
    let mut count = 0;
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                count += count_files(&path, ext);
            } else if path.extension().and_then(|e| e.to_str()) == Some(ext) {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn full_pyramid_to_disk_deep_zoom_raw() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(512, 384);
    let planner = PyramidPlanner::new(512, 384, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("output_files");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);
    let config = EngineConfig::default().with_concurrency(4);

    let result = generate_pyramid(&src, &plan, &sink, &config).unwrap();

    // Tile count matches
    let actual_files = count_files(&base, "raw");
    assert_eq!(
        actual_files as u64,
        plan.total_tile_count(),
        "File count mismatch: {actual_files} files vs {} planned tiles",
        plan.total_tile_count()
    );
    assert_eq!(result.tiles_produced, plan.total_tile_count());

    // DZI manifest exists
    let dzi = dir.path().join("output_files.dzi");
    assert!(dzi.exists(), "DZI manifest not found");
    let manifest = std::fs::read_to_string(&dzi).unwrap();
    assert!(manifest.contains("Width=\"512\""));
    assert!(manifest.contains("Height=\"384\""));
}

#[test]
fn full_pyramid_to_disk_deep_zoom_png() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(128, 128);
    let planner = PyramidPlanner::new(128, 128, 64, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("tiles");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Png);
    let config = EngineConfig::default().with_concurrency(2);

    generate_pyramid(&src, &plan, &sink, &config).unwrap();

    let actual_files = count_files(&base, "png");
    assert_eq!(actual_files as u64, plan.total_tile_count());

    // Verify a tile is valid PNG
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0_0.png", top.level));
    let bytes = std::fs::read(&tile_path).unwrap();
    assert_eq!(&bytes[..4], &[0x89, b'P', b'N', b'G']);
}

#[test]
fn full_pyramid_to_disk_xyz_layout() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(256, 256);
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::Xyz).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("xyz_tiles");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Raw);

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let actual_files = count_files(&base, "raw");
    assert_eq!(actual_files as u64, plan.total_tile_count());

    // XYZ path: {z}/{x}/{y}.raw
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0/0.raw", top.level));
    assert!(tile_path.exists(), "XYZ tile not at expected path");

    // No DZI manifest for XYZ
    assert!(!dir.path().join("xyz_tiles.dzi").exists());
}

#[test]
fn full_pyramid_to_disk_jpeg() {
    let dir = tempfile::tempdir().unwrap();
    let src = gradient_raster(64, 64);
    let planner = PyramidPlanner::new(64, 64, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    let base = dir.path().join("jpeg_out");
    let sink = FsSink::new(base.clone(), plan.clone(), TileFormat::Jpeg { quality: 80 });

    generate_pyramid(&src, &plan, &sink, &EngineConfig::default()).unwrap();

    let actual_files = count_files(&base, "jpeg");
    assert_eq!(actual_files as u64, plan.total_tile_count());

    // Verify JPEG magic
    let top = plan.levels.last().unwrap();
    let tile_path = base.join(format!("{}/0_0.jpeg", top.level));
    let bytes = std::fs::read(&tile_path).unwrap();
    assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
}

#[test]
fn deterministic_fs_output() {
    let dir1 = tempfile::tempdir().unwrap();
    let dir2 = tempfile::tempdir().unwrap();
    let src = gradient_raster(256, 256);
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    // Run 1
    let sink1 = FsSink::new(dir1.path().join("out"), plan.clone(), TileFormat::Raw);
    generate_pyramid(&src, &plan, &sink1, &EngineConfig::default()).unwrap();

    // Run 2
    let sink2 = FsSink::new(dir2.path().join("out"), plan.clone(), TileFormat::Raw);
    generate_pyramid(&src, &plan, &sink2, &EngineConfig::default()).unwrap();

    // Compare every tile file
    for coord in plan.tile_coords() {
        let rel = plan.tile_path(coord, "raw").unwrap();
        let bytes1 = std::fs::read(dir1.path().join("out").join(&rel)).unwrap();
        let bytes2 = std::fs::read(dir2.path().join("out").join(&rel)).unwrap();
        assert_eq!(bytes1, bytes2, "Tile {rel} differs between runs");
    }

    // Compare DZI manifests
    let m1 = std::fs::read_to_string(dir1.path().join("out.dzi")).unwrap();
    let m2 = std::fs::read_to_string(dir2.path().join("out.dzi")).unwrap();
    assert_eq!(m1, m2, "DZI manifests differ");
}
