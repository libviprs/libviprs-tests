use libviprs::{
    generate_pyramid, EngineConfig, Layout, MemorySink, PixelFormat, PyramidPlanner, Raster,
};

fn gradient_raster(w: u32, h: u32) -> Raster {
    let bpp = PixelFormat::Rgb8.bytes_per_pixel();
    let mut data = vec![0u8; w as usize * h as usize * bpp];
    for y in 0..h {
        for x in 0..w {
            let off = (y as usize * w as usize + x as usize) * bpp;
            data[off] = (x % 256) as u8;
            data[off + 1] = (y % 256) as u8;
            data[off + 2] = ((x * 7 + y * 13) % 256) as u8;
        }
    }
    Raster::new(w, h, PixelFormat::Rgb8, data).unwrap()
}

/// Mirrors libvips' test_threading.sh: verify output is identical at concurrency 1..N.
#[test]
fn deterministic_across_concurrency_levels() {
    let src = gradient_raster(512, 384);
    let planner = PyramidPlanner::new(512, 384, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();

    // Reference: single-threaded
    let ref_sink = MemorySink::new();
    generate_pyramid(
        &src,
        &plan,
        &ref_sink,
        &EngineConfig::default(),
    )
    .unwrap();
    let mut ref_tiles = ref_sink.tiles();
    ref_tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

    for concurrency in [1, 2, 4, 8, 16, 32] {
        let sink = MemorySink::new();
        let config = EngineConfig::default().with_concurrency(concurrency);
        generate_pyramid(&src, &plan, &sink, &config).unwrap();

        let mut tiles = sink.tiles();
        tiles.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

        assert_eq!(
            tiles.len(),
            ref_tiles.len(),
            "Tile count mismatch at concurrency={concurrency}"
        );

        for (ref_t, t) in ref_tiles.iter().zip(tiles.iter()) {
            assert_eq!(ref_t.coord, t.coord);
            assert_eq!(
                ref_t.data, t.data,
                "Tile data diverged at {:?} with concurrency={concurrency}",
                t.coord
            );
        }
    }
}

/// Verify different tile sizes produce correct tile counts.
#[test]
fn deterministic_across_tile_sizes() {
    let src = gradient_raster(300, 200);

    for tile_size in [64, 128, 256] {
        let planner = PyramidPlanner::new(300, 200, tile_size, 0, Layout::DeepZoom).unwrap();
        let plan = planner.plan();

        // Single-threaded vs 4-thread
        let sink_st = MemorySink::new();
        generate_pyramid(&src, &plan, &sink_st, &EngineConfig::default()).unwrap();

        let sink_mt = MemorySink::new();
        generate_pyramid(
            &src,
            &plan,
            &sink_mt,
            &EngineConfig::default().with_concurrency(4),
        )
        .unwrap();

        let mut tiles_st = sink_st.tiles();
        let mut tiles_mt = sink_mt.tiles();
        tiles_st.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));
        tiles_mt.sort_by_key(|t| (t.coord.level, t.coord.row, t.coord.col));

        assert_eq!(tiles_st.len(), tiles_mt.len(), "tile_size={tile_size}");
        for (a, b) in tiles_st.iter().zip(tiles_mt.iter()) {
            assert_eq!(a.coord, b.coord);
            assert_eq!(a.data, b.data, "Diverged at {:?} tile_size={tile_size}", a.coord);
        }
    }
}
