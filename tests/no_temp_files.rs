use libviprs::{
    EngineConfig, Layout, MemorySink, PixelFormat, PyramidPlanner, Raster, generate_pyramid,
};

/// Mirrors libvips' test_seq.sh: verify the engine doesn't create temp files.
/// Makes the temp dir read-only and confirms processing still succeeds.
#[test]
fn no_temp_files_during_processing() {
    let src = {
        let bpp = PixelFormat::Rgb8.bytes_per_pixel();
        let w = 512u32;
        let h = 512;
        let mut data = vec![0u8; w as usize * h as usize * bpp];
        for y in 0..h {
            for x in 0..w {
                let off = (y as usize * w as usize + x as usize) * bpp;
                data[off] = (x % 256) as u8;
                data[off + 1] = (y % 256) as u8;
                data[off + 2] = 128;
            }
        }
        Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
    };

    let planner = PyramidPlanner::new(512, 512, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    // Set TMPDIR to a read-only directory
    let temp_dir = tempfile::tempdir().unwrap();
    let readonly = std::os::unix::fs::PermissionsExt::from_mode(0o444);
    std::fs::set_permissions(temp_dir.path(), readonly).unwrap();

    // Save and override TMPDIR
    let old_tmpdir = std::env::var("TMPDIR").ok();
    unsafe {
        std::env::set_var("TMPDIR", temp_dir.path());
    }

    let result = generate_pyramid(
        &src,
        &plan,
        &sink,
        &EngineConfig::default().with_concurrency(4),
    );

    // Restore TMPDIR
    match old_tmpdir {
        Some(val) => unsafe { std::env::set_var("TMPDIR", val) },
        None => unsafe { std::env::remove_var("TMPDIR") },
    }

    // Restore permissions so temp dir can be cleaned up
    let writable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(temp_dir.path(), writable).unwrap();

    assert!(
        result.is_ok(),
        "Engine must not require temp files: {:?}",
        result.err()
    );
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());
}
