use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, MemorySink, PixelFormat, PyramidPlanner,
    Raster,
};
use std::process::Command;

/// Environment marker that flips this test binary into its child "worker"
/// branch. When present, the process was re-spawned by the parent invocation
/// with `TMPDIR` already pointing at a read-only directory, so the worker just
/// runs the engine and asserts. The name is unlikely to collide with anything
/// in the ambient environment.
const WORKER_ENV: &str = "LIBVIPRS_NO_TEMP_FILES_WORKER";

/// Build the deterministic 512x512 RGB8 source raster the engine processes.
fn build_source() -> Raster {
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
}

/// The real behavioral check, mirroring libvips' test_seq.sh: with `TMPDIR`
/// pointing at a read-only directory, the engine must still process the full
/// pyramid because it never spills to temp files.
///
/// This function reads `TMPDIR` from the ambient environment and never mutates
/// it. In the child worker the parent has already placed the read-only path
/// there via `Command::env` at spawn time (see the parent branch below), so the
/// process running this code performs no `env::set_var`/`remove_var` at all.
fn run_engine_under_readonly_tmpdir() {
    let src = build_source();
    let planner = PyramidPlanner::new(512, 512, 256, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default().with_concurrency(4))
        .run();

    assert!(
        result.is_ok(),
        "Engine must not require temp files: {:?}",
        result.err()
    );
    assert_eq!(sink.tile_count() as u64, plan.total_tile_count());
}

/// Verify the engine does not create temp files while processing.
///
/// ## Why this is a subprocess and not an in-process `env::set_var`
///
/// The check needs the engine to run with `TMPDIR` pointing at a read-only
/// directory. `std::env::set_var` / `remove_var` are `unsafe` because a
/// concurrent `getenv` in another thread races the `setenv` write; mutating the
/// environment of this multi-threaded test process was sound only by accident,
/// because this binary happened to contain exactly one `#[test]`. Adding a
/// second test to the file would have reintroduced that race (issue #60).
///
/// Instead of mutating the parent's environment, the parent branch spawns a
/// child (a re-exec of this same test binary) and hands it `TMPDIR` through
/// `Command::env`. `Command::env` records the value in the child's spawn
/// environment; it does not call `setenv()` in this process. The engine
/// therefore runs against the read-only `TMPDIR` in a process whose environment
/// was set before any thread started, so there is no `setenv` race and the
/// soundness of this test no longer depends on the file staying single-test.
#[test]
fn no_temp_files_during_processing() {
    // Worker branch: the parent re-spawned us with TMPDIR already pointing at
    // the read-only directory. Do the real work and return without recursing.
    if std::env::var_os(WORKER_ENV).is_some() {
        run_engine_under_readonly_tmpdir();
        return;
    }

    // Parent branch: create the read-only temp dir and drive the check inside a
    // child process. No environment mutation happens in this process.
    let temp_dir = tempfile::tempdir().unwrap();
    let readonly = std::os::unix::fs::PermissionsExt::from_mode(0o444);
    std::fs::set_permissions(temp_dir.path(), readonly).unwrap();

    let exe = std::env::current_exe().expect("locate current test binary");
    let output = Command::new(exe)
        .args([
            "--exact",
            "no_temp_files_during_processing",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(WORKER_ENV, "1")
        .env("TMPDIR", temp_dir.path())
        .output();

    // Restore permissions so the temp dir can be removed on drop, whatever the
    // child's outcome.
    let writable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(temp_dir.path(), writable).unwrap();

    let output = output.expect("spawn worker process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "worker process (engine under read-only TMPDIR) failed: {}\n--- worker stdout ---\n{stdout}\n--- worker stderr ---\n{stderr}",
        output.status,
    );
    // Guard against a false green: a `--exact` filter that matches nothing (for
    // example after a rename) also exits 0. Require that the worker actually ran
    // the one test so this check can never silently stop exercising the engine.
    assert!(
        stdout.contains("1 passed"),
        "worker did not run exactly one test; the engine check was skipped\n--- worker stdout ---\n{stdout}\n--- worker stderr ---\n{stderr}",
    );
}
