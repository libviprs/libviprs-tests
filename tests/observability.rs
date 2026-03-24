use libviprs::{
    CollectingObserver, EngineConfig, EngineEvent, Layout, MemorySink, PixelFormat, PyramidPlanner,
    Raster, generate_pyramid_observed,
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

#[test]
fn progress_events_match_tile_count() {
    let src = gradient_raster(512, 384);
    let planner = PyramidPlanner::new(512, 384, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();
    let obs = CollectingObserver::new();

    let result = generate_pyramid_observed(
        &src,
        &plan,
        &sink,
        &EngineConfig::default().with_concurrency(4),
        &obs,
    )
    .unwrap();

    let events = obs.events();

    // Count event types
    let level_starts = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::LevelStarted { .. }))
        .count();
    let level_completes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::LevelCompleted { .. }))
        .count();
    let tile_completes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::TileCompleted { .. }))
        .count();
    let finishes = events
        .iter()
        .filter(|e| matches!(e, EngineEvent::Finished { .. }))
        .count();

    assert_eq!(level_starts, plan.level_count());
    assert_eq!(level_completes, plan.level_count());
    assert_eq!(tile_completes as u64, plan.total_tile_count());
    assert_eq!(finishes, 1);
    assert_eq!(result.tiles_produced, plan.total_tile_count());
}

#[test]
fn level_started_before_tile_completed() {
    let src = gradient_raster(256, 256);
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();
    let obs = CollectingObserver::new();

    generate_pyramid_observed(&src, &plan, &sink, &EngineConfig::default(), &obs).unwrap();

    let events = obs.events();

    // For each level, LevelStarted must precede its TileCompleted events
    let mut current_level: Option<u32> = None;
    for event in &events {
        match event {
            EngineEvent::LevelStarted { level, .. } => {
                current_level = Some(*level);
            }
            EngineEvent::TileCompleted { coord } => {
                assert_eq!(
                    current_level,
                    Some(coord.level),
                    "TileCompleted for level {} before LevelStarted",
                    coord.level
                );
            }
            EngineEvent::LevelCompleted { level, .. } => {
                assert_eq!(current_level, Some(*level));
            }
            EngineEvent::Finished { .. } => {}
            _ => {} // Pipeline-level and streaming events not relevant here
        }
    }
}

#[test]
fn level_completed_tiles_match_actual() {
    let src = gradient_raster(300, 200);
    let planner = PyramidPlanner::new(300, 200, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();
    let obs = CollectingObserver::new();

    generate_pyramid_observed(
        &src,
        &plan,
        &sink,
        &EngineConfig::default().with_concurrency(2),
        &obs,
    )
    .unwrap();

    let events = obs.events();

    // For each LevelCompleted, verify tiles_produced matches expected
    for event in &events {
        if let EngineEvent::LevelCompleted {
            level,
            tiles_produced,
        } = event
        {
            let level_plan = &plan.levels[*level as usize];
            assert_eq!(
                *tiles_produced,
                level_plan.tile_count(),
                "Level {level}: expected {} tiles, got {tiles_produced}",
                level_plan.tile_count()
            );
        }
    }
}

#[test]
fn peak_memory_bounded_for_medium_image() {
    let src = gradient_raster(2048, 2048);
    let planner = PyramidPlanner::new(2048, 2048, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = generate_pyramid_observed(
        &src,
        &plan,
        &sink,
        &EngineConfig::default().with_concurrency(4),
        &libviprs::observe::NoopObserver,
    )
    .unwrap();

    let source_bytes = 2048u64 * 2048 * 3;
    // Peak should not exceed 2x source
    assert!(
        result.peak_memory_bytes <= source_bytes * 2,
        "Peak memory {} exceeds 2x source {} for 2048x2048",
        result.peak_memory_bytes,
        source_bytes
    );
}
