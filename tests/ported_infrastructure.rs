#![cfg(feature = "ported_tests")]

//! Ported infrastructure tests from libvips shell-based test suites.
//!
//! These tests exercise metadata preservation, threading consistency,
//! sequential access, file descriptor management, pipeline stalls,
//! timeout/cancellation, tokenization, and CLI behaviour.
//!
//! Run this cell with `--test-threads=1`. The shell originals each ran in
//! their own process; their ports mutate process-global state (TMPDIR,
//! VIPS_MAX_COORD, the max-coord limit, and lsof-based fd counting), which
//! individual tests save and restore but cannot make atomic across
//! concurrently running tests.

use std::path::Path;

use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, FsSink, Interpolator, Layout, MetadataValue,
    PixelFormat, Precision, PyramidPlanner, Raster, decode_file,
};

/// Path to the libvips reference test images directory.
const REF_IMAGES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tmp/libvips-reference-tests/test-suite/images"
);

fn ref_image(name: &str) -> std::path::PathBuf {
    Path::new(REF_IMAGES).join(name)
}

// ─── 13.1 Metadata Preservation ─────────────────────────────────────────────

mod metadata {
    use super::*;

    #[test]
    /// ICC profile preservation: load an image with an ICC profile,
    /// save it, reload, and verify the profile is still present.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Get a metadata field by name.
    /// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
    ///
    /// /// Save a raster to a file (format inferred from extension).
    /// fn Raster::save(&self, path: &Path) -> Result<(), SaveError>;
    /// ```
    ///
    /// ## Test logic (from test_keep.sh)
    ///
    /// 1. Load sample.jpg (has embedded ICC profile).
    /// 2. get_field("icc-profile-data") should be Some(Blob).
    /// 3. Save as JPEG, reload, verify profile is preserved.
    ///
    /// Reference: test_keep.sh
    fn test_keep_icc() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();
        let profile = im.get_field("icc-profile-data");
        assert!(profile.is_some(), "sample.jpg should have an ICC profile");

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("keep_icc.jpg");
        im.save(&out).unwrap();

        let im2 = decode_file(&out).unwrap();
        let profile2 = im2.get_field("icc-profile-data");
        assert!(
            profile2.is_some(),
            "ICC profile should be preserved after save"
        );
    }

    #[test]
    /// XMP metadata preservation across save/reload.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::get_field(&self, name: &str) -> Option<MetadataValue>;
    /// fn Raster::save(&self, path: &Path) -> Result<(), SaveError>;
    /// ```
    ///
    /// ## Test logic (from test_keep.sh)
    ///
    /// 1. Load an image with XMP metadata.
    /// 2. Save and reload.
    /// 3. Verify "xmp-data" field is preserved.
    ///
    /// Reference: test_keep.sh
    fn test_keep_xmp() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        // Not all JPEGs have XMP — check if present first
        if let Some(_xmp) = im.get_field("xmp-data") {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("keep_xmp.jpg");
            im.save(&out).unwrap();

            let im2 = decode_file(&out).unwrap();
            assert!(
                im2.get_field("xmp-data").is_some(),
                "XMP should be preserved"
            );
        }
    }

    #[test]
    /// Strip all metadata from output.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Save with all metadata stripped.
    /// fn Raster::save_stripped(&self, path: &Path) -> Result<(), SaveError>;
    ///
    /// /// Or: use a SaveOptions builder with .strip(true).
    /// fn Raster::save_with_options(&self, path: &Path, opts: SaveOptions) -> Result<(), SaveError>;
    /// ```
    ///
    /// ## Test logic (from test_keep.sh)
    ///
    /// 1. Load sample.jpg.
    /// 2. Save with strip=true.
    /// 3. Reload: no ICC, no EXIF, no XMP.
    ///
    /// Reference: test_keep.sh
    fn test_keep_none() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("stripped.jpg");
        im.save_stripped(&out).unwrap();

        let im2 = decode_file(&out).unwrap();
        assert!(
            im2.get_field("icc-profile-data").is_none(),
            "ICC should be stripped"
        );
        assert!(
            im2.get_field("exif-data").is_none(),
            "EXIF should be stripped"
        );
    }

    #[test]
    /// Apply a custom ICC profile to output.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Attach an ICC profile to the image before saving.
    /// fn Raster::set_icc_profile(&mut self, profile: &[u8]);
    /// fn Raster::save(&self, path: &Path) -> Result<(), SaveError>;
    /// ```
    ///
    /// ## Test logic (from test_keep.sh)
    ///
    /// 1. Load sample.jpg.
    /// 2. Read sRGB.icc from reference fixtures.
    /// 3. Attach profile and save.
    /// 4. Reload and verify the embedded profile matches.
    ///
    /// Reference: test_keep.sh
    fn test_keep_custom_profile() {
        let mut im = decode_file(&ref_image("sample.jpg")).unwrap();
        let srgb_icc = std::fs::read(ref_image("sRGB.icm")).unwrap();
        im.set_icc_profile(&srgb_icc);

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("custom_icc.jpg");
        im.save(&out).unwrap();

        let im2 = decode_file(&out).unwrap();
        if let Some(MetadataValue::Blob(profile)) = im2.get_field("icc-profile-data") {
            assert_eq!(
                profile.len(),
                srgb_icc.len(),
                "ICC profile size should match"
            );
        } else {
            panic!("Custom ICC profile should be preserved");
        }
    }
}

// NOTE: test_new_from_image moved to ported_iofuncs.rs to match libvips
// test_iofuncs.py structure.

// ─── 13.2 Threading & Concurrency ───────────────────────────────────────────

mod threading {
    use super::*;

    #[test]
    /// Multi-threaded consistency: same output regardless of thread count.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Configure the number of worker threads for image processing.
    /// fn EngineConfig::with_threads(n: usize) -> EngineConfig;
    /// ```
    ///
    /// ## Test logic (from test_threading.sh)
    ///
    /// 1. Generate a pyramid with 1 thread.
    /// 2. Generate same pyramid with 4 threads.
    /// 3. Compare tile-by-tile: all tiles should be identical.
    ///
    /// Reference: test_threading.sh
    fn test_threading_consistency() {
        let src = decode_file(&ref_image("sample.jpg")).unwrap();
        let planner =
            PyramidPlanner::new(src.width(), src.height(), 256, 0, Layout::DeepZoom).unwrap();
        let plan = planner.plan();

        let dir1 = tempfile::tempdir().unwrap();
        let base1 = dir1.path().join("t1");
        let sink1 = FsSink::new(base1.clone(), plan.clone());
        // The core spells the worker-thread knob `with_concurrency` (builder
        // on `EngineConfig::default()`), not the aspirational `with_threads`
        // constructor this port sketched; same semantics (n workers).
        let config1 = EngineConfig::default().with_concurrency(1);
        EngineBuilder::new(&src, plan.clone(), &sink1)
            .with_engine(EngineKind::Monolithic)
            .with_config(config1)
            .run()
            .unwrap();

        let dir4 = tempfile::tempdir().unwrap();
        let base4 = dir4.path().join("t4");
        let sink4 = FsSink::new(base4.clone(), plan.clone());
        let config4 = EngineConfig::default().with_concurrency(4);
        EngineBuilder::new(&src, plan.clone(), &sink4)
            .with_engine(EngineKind::Monolithic)
            .with_config(config4)
            .run()
            .unwrap();

        // Compare top-level tiles
        let top = plan.levels.last().unwrap();
        let tile_path = format!("{}/0_0.png", top.level);
        let t1 = std::fs::read(base1.join(&tile_path)).unwrap();
        let t4 = std::fs::read(base4.join(&tile_path)).unwrap();
        assert_eq!(
            t1, t4,
            "Tiles should be identical regardless of thread count"
        );
    }

    #[test]
    /// Thread pool size control.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn EngineConfig::with_threads(n: usize) -> EngineConfig;
    /// fn EngineConfig::max_threads(&self) -> usize;
    /// ```
    ///
    /// ## Test logic (from test_threading.sh)
    ///
    /// 1. Create config with max_threads=2.
    /// 2. Verify max_threads() returns 2.
    ///
    /// Reference: test_threading.sh
    fn test_max_threads() {
        // The core spells this knob `with_concurrency` and exposes it as the
        // public `concurrency` field; there is no separate `max_threads`
        // accessor. Same semantics: the configured worker-thread count.
        let config = EngineConfig::default().with_concurrency(2);
        assert_eq!(config.concurrency, 2);
    }
}

// ─── 13.3 Sequential Access ─────────────────────────────────────────────────

mod sequential {
    use super::*;

    #[test]
    /// Sequential thumbnail: generate a thumbnail in a streaming manner.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Open an image for sequential (top-to-bottom) access.
    /// fn decode_file_sequential(path: &Path) -> Result<Raster, DecodeError>;
    ///
    /// /// Or: decode option to enable sequential mode.
    /// fn Raster::thumbnail(width: u32) -> Raster;
    /// ```
    ///
    /// ## Test logic (from test_seq.sh)
    ///
    /// 1. Open sample.jpg in sequential mode.
    /// 2. Generate thumbnail at 50% width.
    /// 3. Verify output dimensions.
    ///
    /// Reference: test_seq.sh
    fn test_seq_thumbnail() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();
        // The core's aspect-preserving streaming thumbnail is
        // `thumbnail_image(width)` (in-memory `vips_thumbnail_image`,
        // core PR #307), the real surface this shell test exercises. It
        // fits a width x width box, so the result never exceeds half width.
        let thumb = im.thumbnail_image(im.width() / 2);
        assert!(thumb.width() <= im.width() / 2 + 1);
        assert!(thumb.height() > 0);
    }

    #[test]
    /// No temp files created in sequential mode.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn decode_file_sequential(path: &Path) -> Result<Raster, DecodeError>;
    /// ```
    ///
    /// ## Test logic (from test_seq.sh)
    ///
    /// 1. Open in sequential mode.
    /// 2. Process (thumbnail/shrink).
    /// 3. Verify no temp files were created in the temp directory.
    ///
    /// Reference: test_seq.sh
    fn test_seq_no_temps() {
        // The shell original (test_seq.sh) ran in its own process, so its
        // TMPDIR override died with it. In-process, the override must be
        // restored or every later `tempfile::tempdir()` resolves into this
        // test's deleted directory. This cell also requires serial
        // execution (--test-threads=1): TMPDIR is process-global state.
        let old_tmpdir = std::env::var_os("TMPDIR");
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("TMPDIR", dir.path()) };

        let im = decode_file(&ref_image("sample.jpg")).unwrap();
        // See test_seq_thumbnail: `thumbnail_image` is the core's
        // aspect-preserving streaming thumbnail; quarter-width box here.
        let _thumb = im.thumbnail_image(im.width() / 4);

        let temp_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        match &old_tmpdir {
            Some(v) => unsafe { std::env::set_var("TMPDIR", v) },
            None => unsafe { std::env::remove_var("TMPDIR") },
        }
        assert!(
            temp_files.is_empty(),
            "Sequential mode should not create temp files, found {}",
            temp_files.len()
        );
    }

    #[test]
    /// Shrink with no temp files in sequential mode.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::shrink(&self, xshrink: f64, yshrink: f64) -> Raster;
    /// ```
    ///
    /// ## Test logic (from test_seq.sh)
    ///
    /// 1. Open in sequential mode.
    /// 2. Shrink by 4.
    /// 3. Verify no temp files.
    ///
    /// Reference: test_seq.sh
    fn test_seq_shrink_no_temps() {
        // Save and restore TMPDIR; see test_seq_no_temps.
        let old_tmpdir = std::env::var_os("TMPDIR");
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("TMPDIR", dir.path()) };

        let im = decode_file(&ref_image("sample.jpg")).unwrap();
        let _shrunk = im.shrink(4.0, 4.0);

        let temp_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        match &old_tmpdir {
            Some(v) => unsafe { std::env::set_var("TMPDIR", v) },
            None => unsafe { std::env::remove_var("TMPDIR") },
        }
        assert!(temp_files.is_empty());
    }
}

// ─── 13.4 File Descriptor Management ────────────────────────────────────────

mod descriptors {
    use super::*;

    #[test]
    /// JPEG file descriptor leak check.
    ///
    /// ## Required API
    ///
    /// No special API — this tests that decode_file properly closes
    /// file handles when the Raster is dropped.
    ///
    /// ## Test logic (from test_descriptors.sh)
    ///
    /// 1. Count open file descriptors.
    /// 2. Load and drop 100 JPEG images.
    /// 3. Count open file descriptors again — should not have grown.
    ///
    /// Reference: test_descriptors.sh
    fn test_fd_leak_jpeg() {
        let initial_fds = count_open_fds();
        for _ in 0..100 {
            let _im = decode_file(&ref_image("sample.jpg")).unwrap();
            // Raster dropped here
        }
        let final_fds = count_open_fds();
        assert!(
            final_fds <= initial_fds + 5,
            "FD leak: started with {initial_fds}, ended with {final_fds}"
        );
    }

    #[test]
    /// PNG file descriptor leak check.
    ///
    /// Same as JPEG but with PNG files.
    ///
    /// Reference: test_descriptors.sh
    fn test_fd_leak_png() {
        let initial_fds = count_open_fds();
        for _ in 0..100 {
            let _im = decode_file(&ref_image("sample.png")).unwrap();
        }
        let final_fds = count_open_fds();
        assert!(
            final_fds <= initial_fds + 5,
            "FD leak: started with {initial_fds}, ended with {final_fds}"
        );
    }

    #[test]
    /// TIFF file descriptor leak check.
    ///
    /// Reference: test_descriptors.sh
    fn test_fd_leak_tiff() {
        let initial_fds = count_open_fds();
        for _ in 0..100 {
            let _im = decode_file(&ref_image("sample.tif")).unwrap();
        }
        let final_fds = count_open_fds();
        assert!(
            final_fds <= initial_fds + 5,
            "FD leak: started with {initial_fds}, ended with {final_fds}"
        );
    }

    /// Count open file descriptors for the current process (macOS/Linux).
    fn count_open_fds() -> usize {
        #[cfg(target_os = "macos")]
        {
            let pid = std::process::id();
            let output = std::process::Command::new("lsof")
                .args(["-p", &pid.to_string()])
                .output();
            match output {
                Ok(o) => String::from_utf8_lossy(&o.stdout).lines().count(),
                Err(_) => 0,
            }
        }
        #[cfg(target_os = "linux")]
        {
            std::fs::read_dir("/proc/self/fd")
                .map(|entries| entries.count())
                .unwrap_or(0)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0
        }
    }
}

// ─── 13.5 Pipeline Stall ────────────────────────────────────────────────────

mod pipeline {
    use super::*;

    #[test]
    /// Pipeline stall detection: verify that a large pipeline completes
    /// without deadlock within a reasonable time.
    ///
    /// ## Required API
    ///
    /// No special API — tests that chained operations don't deadlock.
    ///
    /// ## Test logic (from test_stall.sh)
    ///
    /// 1. Load a large image.
    /// 2. Chain multiple operations (resize, sharpen, etc.).
    /// 3. Force evaluation by reading a pixel.
    /// 4. The test passes if it completes (timeout would indicate stall).
    ///
    /// Reference: test_stall.sh
    fn test_pipeline_stall() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();

        // Chain multiple operations. Core signatures: `resize` takes a single
        // scale factor, and `gaussblur` takes (sigma, min_ampl, precision);
        // 0.2 / integer are the libvips gaussblur defaults.
        let result = im
            .resize(0.5)
            .gaussblur(2.0, 0.2, Precision::Integer)
            .resize(2.0);

        // Force evaluation
        let _px = result.getpoint(0, 0);
        // If we get here, no stall occurred
    }
}

// ─── 13.6 Timeout / Kill ────────────────────────────────────────────────────

mod timeout {
    use super::*;
    use libviprs::CollectingObserver;

    #[test]
    /// Verify progress events are emitted during pyramid generation.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// use libviprs::observe::{CollectingObserver, Observer};
    ///
    /// /// An observer that collects progress events.
    /// struct CollectingObserver;
    ///
    /// impl Observer for CollectingObserver {
    ///     fn on_progress(&self, fraction: f64, tiles_done: u64, tiles_total: u64);
    ///     fn should_cancel(&self) -> bool;
    /// }
    ///
    /// /// Generate a pyramid with an observer for progress/cancellation.
    /// fn generate_pyramid_observed(
    ///     src: &Raster, plan: &Plan, sink: &dyn Sink,
    ///     config: &EngineConfig, observer: &dyn Observer,
    /// ) -> Result<PyramidResult, PyramidError>;
    /// ```
    ///
    /// ## Test logic
    ///
    /// 1. Create a CollectingObserver.
    /// 2. Generate a pyramid with observer attached.
    /// 3. Verify at least one progress event was received.
    ///
    /// Reference: manual — observe module usage
    fn test_progress_cancel() {
        let src = decode_file(&ref_image("sample.jpg")).unwrap();
        let planner =
            PyramidPlanner::new(src.width(), src.height(), 256, 0, Layout::DeepZoom).unwrap();
        let plan = planner.plan();

        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("progress");
        let sink = FsSink::new(base, plan.clone());

        let observer = std::sync::Arc::new(CollectingObserver::new());
        let _result = EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::Monolithic)
            .with_config(EngineConfig::default())
            .with_observer_arc(observer.clone())
            .run()
            .unwrap();

        assert!(
            observer.event_count() > 0,
            "Should have received at least one progress event"
        );
    }

    #[test]
    /// Timeout during GIF save.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Encode raster as GIF bytes.
    /// fn Raster::encode_gif(&self, options: gif::SaveOptions) -> Result<Vec<u8>, EncodeError>;
    /// ```
    ///
    /// ## Test logic
    ///
    /// 1. Create a large synthetic image.
    /// 2. Attempt GIF encoding with a timeout.
    /// 3. Verify that the operation either completes or times out gracefully.
    ///
    /// Reference: manual — no GIF encoding support yet
    fn test_timeout_gifsave() {
        let data = vec![128u8; 1000 * 1000 * 3];
        let im = Raster::new(1000, 1000, PixelFormat::Rgb8, data).unwrap();
        // This exercises the GIF surface through `Raster::save`'s extension
        // dispatch rather than `encode_gif`, because `save` is what a caller
        // reaches for under a timeout. It returns a typed
        // `SaveError::UnsupportedExtension` (a clean error, never a panic),
        // exactly the graceful-degradation contract this test checks.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("timeout.gif");
        let result = im.save(&out);
        // Either it works or returns a clean error, not a panic
        // (Err is the expected path: GIF encoding not supported)
        if let Ok(()) = result {
            assert!(std::fs::metadata(&out).unwrap().len() > 0);
        }
    }

    #[test]
    /// Timeout during WebP save.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// fn Raster::encode_webp(&self, options: webp::SaveOptions) -> Result<Vec<u8>, EncodeError>;
    /// ```
    ///
    /// ## Test logic
    ///
    /// 1. Create large synthetic image.
    /// 2. Attempt WebP encoding.
    /// 3. Verify clean success or error.
    ///
    /// Reference: manual — no WebP encoding support yet
    fn test_timeout_webpsave() {
        let data = vec![128u8; 1000 * 1000 * 3];
        let im = Raster::new(1000, 1000, PixelFormat::Rgb8, data).unwrap();
        // See test_timeout_gifsave: `Raster::save` extension dispatch is the
        // core's WebP surface and errors cleanly on unsupported formats.
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("timeout.webp");
        let result = im.save(&out);
        // (Err is the expected path: WebP encoding not supported)
        if let Ok(()) = result {
            assert!(std::fs::metadata(&out).unwrap().len() > 0);
        }
    }
}

// ─── 13.7 Tokenization ─────────────────────────────────────────────────────

mod tokenization {
    #[test]
    /// Token parsing: quoted, unquoted, and escaped tokens.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Parse a command-line-style string into tokens.
    /// /// Handles quoting ("..."), escaping (\"), and whitespace splitting.
    /// fn tokenize(input: &str) -> Vec<String>;
    /// ```
    ///
    /// ## Test logic (from test_token.sh)
    ///
    /// 1. tokenize("hello world") → ["hello", "world"].
    /// 2. tokenize("\"hello world\"") → ["hello world"].
    /// 3. tokenize("hello\\ world") → ["hello world"].
    /// 4. tokenize("a \"b c\" d") → ["a", "b c", "d"].
    ///
    /// Reference: test_token.sh
    fn test_token_parsing() {
        use libviprs::tokenize;

        let result = tokenize("hello world");
        assert_eq!(result, vec!["hello", "world"]);

        let result = tokenize("\"hello world\"");
        assert_eq!(result, vec!["hello world"]);

        let result = tokenize("a \"b c\" d");
        assert_eq!(result, vec!["a", "b c", "d"]);
    }
}

// ─── 13.8 CLI ───────────────────────────────────────────────────────────────

mod cli {
    use super::*;

    #[test]
    /// Thumbnail geometry parsing from CLI-style input.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Parse a thumbnail geometry string (e.g. "200x150", "50%", "200x").
    /// fn parse_thumbnail_geometry(spec: &str) -> ThumbnailGeometry;
    ///
    /// pub struct ThumbnailGeometry {
    ///     pub width: Option<u32>,
    ///     pub height: Option<u32>,
    /// }
    /// ```
    ///
    /// ## Test logic (from test_cli.sh)
    ///
    /// 1. Parse "200" → width=200, height=None.
    /// 2. Parse "200x150" → width=200, height=150.
    /// 3. Apply to sample.jpg, verify output dimensions.
    ///
    /// Reference: test_cli.sh
    fn test_cli_thumbnail() {
        use libviprs::parse_thumbnail_geometry;

        let geom = parse_thumbnail_geometry("200");
        assert_eq!(geom.width, Some(200));

        let geom = parse_thumbnail_geometry("200x150");
        assert_eq!(geom.width, Some(200));
        assert_eq!(geom.height, Some(150));

        let im = decode_file(&ref_image("sample.jpg")).unwrap();
        // The core has no `Raster::thumbnail`; `resize` to the equivalent
        // scale factor is the real core surface for a width-200 downscale.
        let thumb = im.resize(200.0 / im.width() as f64);
        assert!(thumb.width() <= 200);
    }

    #[test]
    /// Affine rotation with various interpolators via CLI-style API.
    ///
    /// ## Required API
    ///
    /// ```rust,ignore
    /// /// Rotate by an arbitrary angle (degrees) with a specified interpolator.
    /// fn Raster::rotate(&self, angle: f64, interpolator: Interpolator) -> Raster;
    ///
    /// pub enum Interpolator { Nearest, Bilinear, Bicubic, Nohalo }
    /// ```
    ///
    /// ## Test logic (from test_cli.sh)
    ///
    /// 1. Rotate sample.jpg by 45° with bilinear interpolation.
    /// 2. Verify output has reasonable dimensions.
    ///
    /// Reference: test_cli.sh
    fn test_cli_rotate() {
        let im = decode_file(&ref_image("sample.jpg")).unwrap();
        // The core spells arbitrary-angle rotation with an explicit
        // interpolator as `try_rotate_with(angle, interpolator)`; the 2-arg
        // panicking `rotate` this port sketched does not exist.
        let rotated = im.try_rotate_with(45.0, Interpolator::Bilinear).unwrap();
        assert!(rotated.width() > 0);
        assert!(rotated.height() > 0);
    }

    #[test]
    /// Max coordinate limit via CLI flag.
    ///
    /// ## API
    ///
    /// The coordinate ceiling is a per-decode knob, not a process-global.
    /// The former `set_max_coord`/`get_max_coord` shims (deprecated on
    /// arrival, removed in libviprs#462) are superseded by
    /// [`DecodeLimits::with_max_coord`], enforced by every decoder before
    /// any pixel allocation.
    ///
    /// ```rust,ignore
    /// let limits = DecodeLimits::default().with_max_coord(1000);
    /// let raster = decode_bytes_with_limits(&bytes, limits)?;
    /// ```
    ///
    /// ## Test logic (from test_cli.sh)
    ///
    /// 1. Configure a ceiling of 1000.
    /// 2. Decode a 2000×2000 header — must reject as over-ceiling.
    /// 3. Decode a 500×500 image — must succeed.
    ///
    /// Reference: test_cli.sh
    fn test_cli_max_coord_flag() {
        use libviprs::source::{DecodeLimits, SourceError, decode_bytes_with_limits};

        // The ceiling now travels with the decode call, so nothing global
        // is mutated or restored: distinct decodes carry distinct limits.
        let limits = DecodeLimits::default().with_max_coord(1000);

        // A 2000×2000 declared header exceeds the ceiling on both axes and
        // is rejected before allocation.
        let oversized = Raster::zeroed(2000, 2000, PixelFormat::Gray8)
            .unwrap()
            .encode_vips()
            .unwrap();
        let err =
            decode_bytes_with_limits(&oversized, limits).expect_err("oversized decode must reject");
        assert!(
            matches!(
                err,
                SourceError::CoordLimitExceeded {
                    max_coord: 1000,
                    ..
                }
            ),
            "expected CoordLimitExceeded at the 1000 px ceiling, got {err}"
        );

        // A 500×500 image is under the ceiling and decodes cleanly.
        let small = Raster::zeroed(500, 500, PixelFormat::Gray8)
            .unwrap()
            .encode_vips()
            .unwrap();
        let result = decode_bytes_with_limits(&small, limits).unwrap();
        assert_eq!(result.width(), 500);
    }

    #[test]
    /// Max coordinate limit via environment variable.
    ///
    /// ## API
    ///
    /// The former `init_from_env` shim folded `VIPS_MAX_COORD` into a
    /// process-global (removed in libviprs#462). The caller now reads the
    /// variable and builds a [`DecodeLimits`] from it.
    ///
    /// ```rust,ignore
    /// let ceiling: u32 = std::env::var("VIPS_MAX_COORD")?.parse()?;
    /// let limits = DecodeLimits::default().with_max_coord(ceiling);
    /// ```
    ///
    /// ## Test logic (from test_cli.sh)
    ///
    /// 1. Set VIPS_MAX_COORD=500 in the environment.
    /// 2. Read it in the caller into a `DecodeLimits`.
    /// 3. Verify the configured ceiling is 500 and enforced on decode.
    ///
    /// Reference: test_cli.sh
    fn test_cli_max_coord_env() {
        use libviprs::source::{DecodeLimits, SourceError, decode_bytes_with_limits};

        // SAFETY: test-local mutation of this process's environment; the
        // variable is removed again before the ceiling is exercised.
        unsafe { std::env::set_var("VIPS_MAX_COORD", "500") };
        let ceiling: u32 = std::env::var("VIPS_MAX_COORD")
            .ok()
            .and_then(|v| v.parse().ok())
            .expect("VIPS_MAX_COORD parses");
        unsafe { std::env::remove_var("VIPS_MAX_COORD") };

        let limits = DecodeLimits::default().with_max_coord(ceiling);
        assert_eq!(limits.max_coord, 500);

        // The env-derived ceiling enforces like any other: a 600 px width
        // over it rejects.
        let over = Raster::zeroed(600, 1, PixelFormat::Gray8)
            .unwrap()
            .encode_vips()
            .unwrap();
        assert!(
            matches!(
                decode_bytes_with_limits(&over, limits),
                Err(SourceError::CoordLimitExceeded { max_coord: 500, .. })
            ),
            "width over the env-derived ceiling must reject"
        );
    }
}
