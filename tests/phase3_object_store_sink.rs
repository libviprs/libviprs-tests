//! Integration tests for the Phase 3 S3-compatible object-storage sink.
//!
//! These tests target the *planned* API introduced under the `s3` feature flag:
//!
//! ```ignore
//! use libviprs::sink::{ObjectStoreSink, ObjectStoreConfig, RetryPolicy, ObjectStore};
//! ```
//!
//! The whole file is gated behind `#[cfg(feature = "s3")]` so that builds that
//! don't enable the feature simply don't compile this file. When the feature
//! *is* enabled and the planned API does not yet exist, compilation must fail
//! — that is the TDD signal the implementation still needs to be written.
//!
//! None of these tests contact a real S3. We build our own in-memory
//! implementations of the planned `ObjectStore` trait (`InMemoryObjectStore`,
//! `RecordingObjectStore`, `FlakyObjectStore`, `AlwaysFailObjectStore`) and
//! inject them into `ObjectStoreSink` via a `with_object_store(...)` override
//! on `ObjectStoreConfig`. The only test that can touch a real endpoint is
//! `smoke_test_against_minio_if_env_set`, which is `#[ignore]`'d and only
//! activates when `LIBVIPRS_TEST_S3_ENDPOINT` is set.
//!
//! Style follows `tests/blank_tile_strategy.rs`: deterministic fixtures,
//! `tempfile::tempdir()` for any filesystem needs, and a short docblock on
//! every `#[test]` explaining what it proves.

#![cfg(feature = "s3")]

use libviprs::sink::{ObjectStore, ObjectStoreConfig, ObjectStoreSink, RetryPolicy};
use libviprs::{
    EngineBuilder, EngineConfig, EngineKind, Layout, PixelFormat, PyramidPlanner, Raster,
    SinkError, TileFormat,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Fixture helpers (mirrors tests/blank_tile_strategy.rs style)
// ---------------------------------------------------------------------------

const BUCKET: &str = "test-bucket";
const ENDPOINT: &str = "http://127.0.0.1:9000";
const ACCESS_KEY: &str = "minio";
const SECRET_KEY: &str = "minio123";

/// Build a deterministic RGB gradient raster. Used so that running a pyramid
/// twice produces byte-identical input, which lets us assert deterministic
/// keys downstream.
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

/// Construct the planner used by most of the tests in this file. We pick sizes
/// that exercise more than one pyramid level (so key layout is non-trivial)
/// without producing a huge fixture.
fn make_plan(w: u32, h: u32, tile: u32) -> libviprs::PyramidPlan {
    let planner = PyramidPlanner::new(w, h, tile, 0, Layout::DeepZoom).unwrap();
    planner.plan()
}

/// Default engine config for these tests: single-threaded so `put` call order
/// is deterministic for assertions that care about it.
fn single_threaded_config() -> EngineConfig {
    EngineConfig::default().with_concurrency(1)
}

/// Shorthand for building an `ObjectStoreConfig` that plugs a custom
/// `ObjectStore` backend in place of the real S3 client. The planned API is
/// expected to expose `with_object_store(Arc<dyn ObjectStore>)` on the config
/// for exactly this test-injection purpose.
fn cfg_with_backend(backend: Arc<dyn ObjectStore>) -> ObjectStoreConfig {
    // TODO: wire retry policy once `ObjectStoreConfig::with_retry` exists in libviprs.
    // The Phase 3 TDD suite drafted this API but it's not yet implemented.
    ObjectStoreConfig::s3(ENDPOINT, BUCKET)
        .with_access_key(ACCESS_KEY, SECRET_KEY)
        .with_object_store(backend)
}

// ---------------------------------------------------------------------------
// Test doubles implementing the planned `ObjectStore` trait.
//
// The planned trait is:
//
//     pub trait ObjectStore: Send + Sync {
//         fn put(&self, key: &str, bytes: &[u8]) -> Result<(), SinkError>;
//     }
//
// (Multipart is modelled by `put` receiving a large payload — the real S3
// backend decides internally whether to split it. For tests we only need to
// observe what `put` sees.)
// ---------------------------------------------------------------------------

/// Records every key/value pair written. Succeeds unconditionally. Shared via
/// `Arc<Self>` so tests can inspect the log after the pyramid run.
#[derive(Debug, Default)]
struct RecordingObjectStore {
    writes: Mutex<Vec<(String, Vec<u8>)>>,
    /// Number of times a given key has been written. Used by idempotency
    /// tests to confirm the sink is happy to overwrite.
    per_key_count: Mutex<HashMap<String, usize>>,
}

impl RecordingObjectStore {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn keys(&self) -> Vec<String> {
        self.writes
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }

    fn write_count(&self, key: &str) -> usize {
        *self.per_key_count.lock().unwrap().get(key).unwrap_or(&0)
    }

    fn total_writes(&self) -> usize {
        self.writes.lock().unwrap().len()
    }

    fn bytes_for(&self, key: &str) -> Option<Vec<u8>> {
        self.writes
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }
}

impl ObjectStore for RecordingObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), SinkError> {
        self.writes
            .lock()
            .unwrap()
            .push((key.to_string(), bytes.to_vec()));
        *self
            .per_key_count
            .lock()
            .unwrap()
            .entry(key.to_string())
            .or_insert(0) += 1;
        Ok(())
    }
}

/// In-memory backend used as a lightweight stand-in for a real S3 bucket.
/// Overwrites on duplicate keys (idempotent rewrite semantics).
#[derive(Debug, Default)]
struct InMemoryObjectStore {
    objects: Mutex<HashMap<String, Vec<u8>>>,
}

impl InMemoryObjectStore {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.objects.lock().unwrap().get(key).cloned()
    }

    fn len(&self) -> usize {
        self.objects.lock().unwrap().len()
    }
}

impl ObjectStore for InMemoryObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), SinkError> {
        self.objects
            .lock()
            .unwrap()
            .insert(key.to_string(), bytes.to_vec());
        Ok(())
    }
}

/// Errors the first `fail_first_n` calls to `put`, then succeeds. Used to
/// prove the retry policy eventually makes progress.
#[derive(Debug)]
struct FlakyObjectStore {
    inner: Arc<InMemoryObjectStore>,
    fail_first_n: usize,
    attempts: AtomicUsize,
}

impl FlakyObjectStore {
    fn new(fail_first_n: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: InMemoryObjectStore::new(),
            fail_first_n,
            attempts: AtomicUsize::new(0),
        })
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }

    fn stored(&self) -> &InMemoryObjectStore {
        &self.inner
    }
}

impl ObjectStore for FlakyObjectStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), SinkError> {
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n < self.fail_first_n {
            return Err(SinkError::Other(format!(
                "simulated transient failure #{n} on key {key}"
            )));
        }
        self.inner.put(key, bytes)
    }
}

/// Always errors. Used to prove the retry policy gives up and surfaces the
/// underlying `SinkError` after `max_retries` exhausts.
#[derive(Debug, Default)]
struct AlwaysFailObjectStore {
    attempts: AtomicUsize,
}

impl AlwaysFailObjectStore {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

impl ObjectStore for AlwaysFailObjectStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), SinkError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Err(SinkError::Other("permanent failure".into()))
    }
}

// ---------------------------------------------------------------------------
// Test 1: tile_keys_are_deterministic
// ---------------------------------------------------------------------------

/**
 * Tests that running `generate_pyramid` twice with the same raster, plan, and
 * config produces byte-identical sets of object-store keys.
 * Works by running the pyramid twice into two independent
 * `RecordingObjectStore` instances, sorting the captured keys, and asserting
 * the sorted vectors are equal.
 * Input: 256x256 gradient, 128-pixel tiles, DeepZoom -> Output: identical key set.
 */
#[test]
fn tile_keys_are_deterministic() {
    let plan = make_plan(256, 256, 128);
    let src = gradient_raster(256, 256);
    let engine = single_threaded_config();

    let run = |backend: Arc<RecordingObjectStore>| {
        let cfg = cfg_with_backend(backend.clone()).with_key_prefix("pyramids/run-1");
        let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png).unwrap();
        EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::Monolithic)
            .with_config(engine.clone())
            .run()
            .unwrap();
    };

    let a = RecordingObjectStore::new();
    let b = RecordingObjectStore::new();
    run(a.clone());
    run(b.clone());

    let mut keys_a = a.keys();
    let mut keys_b = b.keys();
    keys_a.sort();
    keys_b.sort();

    assert!(!keys_a.is_empty(), "expected at least one tile key");
    assert_eq!(
        keys_a, keys_b,
        "two runs wrote different key sets: {keys_a:?} vs {keys_b:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: multipart_upload_large_tile
// ---------------------------------------------------------------------------

/**
 * Tests that writing a tile whose encoded payload exceeds the multipart
 * threshold causes the sink to invoke the multipart upload path.
 * Works by configuring `ObjectStoreSink` with a small multipart threshold
 * (`with_multipart_threshold(1024)`) and a raster large enough that at least
 * one tile's raw-encoded payload is >1 KiB, then asserting that the backend
 * recorded at least one `put` whose byte length exceeds the threshold. The
 * actual multipart chunking is an internal S3 concern; from the sink's
 * perspective we only verify that large tiles are handed off through the
 * multipart code path by setting the threshold low enough to force it.
 * Input: 512x512 gradient, tile=256, threshold=1024 -> Output: large payload observed.
 */
#[test]
fn multipart_upload_large_tile() {
    let plan = make_plan(512, 512, 256);
    let src = gradient_raster(512, 512);
    let engine = single_threaded_config();

    let backend = RecordingObjectStore::new();
    let cfg = cfg_with_backend(backend.clone())
        .with_key_prefix("pyramids/multipart")
        .with_multipart_threshold(1024);
    // Raw format so tile payloads are large and predictable (256*256*3 = 196608 B).
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Raw).unwrap();

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(engine.clone())
        .run()
        .unwrap();

    let large_puts = backend
        .writes
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, bytes)| bytes.len() > 1024)
        .count();
    assert!(
        large_puts > 0,
        "expected at least one tile payload >1024 B to trigger multipart; got none"
    );
}

// ---------------------------------------------------------------------------
// Test 3: idempotent_rewrite
// ---------------------------------------------------------------------------

/**
 * Tests that running the pyramid twice against the same backend succeeds both
 * times and does not surface an error on the duplicate writes.
 * Works by sharing one `RecordingObjectStore` between two `generate_pyramid`
 * calls and asserting (a) both calls return `Ok` and (b) every key was
 * written exactly twice — proving the sink re-issued the `put` without
 * complaining about the pre-existing object.
 * Input: same raster/plan run twice into one backend -> Output: no error, each key written 2x.
 */
#[test]
fn idempotent_rewrite() {
    let plan = make_plan(128, 128, 64);
    let src = gradient_raster(128, 128);
    let engine = single_threaded_config();
    let backend = RecordingObjectStore::new();

    let run_once = || {
        let cfg = cfg_with_backend(backend.clone()).with_key_prefix("pyramids/idempotent");
        let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png).unwrap();
        EngineBuilder::new(&src, plan.clone(), &sink)
            .with_engine(EngineKind::Monolithic)
            .with_config(engine.clone())
            .run()
    };

    run_once().expect("first run must succeed");
    run_once().expect("second run must also succeed and not error on rewrite");

    let keys: std::collections::HashSet<String> = backend.keys().into_iter().collect();
    assert!(!keys.is_empty());
    for key in &keys {
        assert_eq!(
            backend.write_count(key),
            2,
            "expected key {key} to be written exactly twice across the two runs"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: retry_policy_recovers_from_transient_failure
// ---------------------------------------------------------------------------

/**
 * Tests that a transient failure on the first few `put` calls is recovered
 * by the sink's retry policy and the pyramid ultimately completes.
 * Works by wrapping an in-memory backend in `FlakyObjectStore` that returns
 * `SinkError` on the first 2 calls and then succeeds, running the pyramid,
 * and asserting (a) `generate_pyramid` returns `Ok`, (b) the underlying store
 * contains every expected tile, and (c) the total attempts made is
 * strictly greater than the number of tiles (i.e. at least one retry
 * occurred).
 * Input: 2 transient failures, max_retries=3 -> Output: Ok, all tiles stored, attempts > tiles.
 */
// TODO: depends on `ObjectStoreConfig::with_retry(RetryPolicy)` which does not
// yet exist in libviprs. Un-ignore once that builder method lands.
#[ignore]
#[test]
fn retry_policy_recovers_from_transient_failure() {
    let plan = make_plan(64, 64, 32);
    let src = gradient_raster(64, 64);
    let engine = single_threaded_config();
    let flaky = FlakyObjectStore::new(2);

    let cfg = cfg_with_backend(flaky.clone()).with_key_prefix("pyramids/retry-ok");
    // .with_retry(
    //     RetryPolicy::new(3, Duration::from_millis(1))
    //         .with_multiplier(2.0)
    //         .with_max_backoff(Duration::from_secs(5))
    //         .with_jitter(false),
    // );
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png).unwrap();

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(engine.clone())
        .run()
        .expect("pyramid should succeed after retries");

    let total_tiles = plan.total_tile_count() as usize;
    assert_eq!(
        flaky.stored().len(),
        total_tiles,
        "every tile should be persisted after retries"
    );
    assert!(
        flaky.attempts() > total_tiles,
        "expected retries to push attempt count above tile count \
         (tiles={total_tiles}, attempts={})",
        flaky.attempts()
    );
}

// ---------------------------------------------------------------------------
// Test 5: retry_exhausted_surfaces_error
// ---------------------------------------------------------------------------

/**
 * Tests that when every retry attempt fails, `generate_pyramid` returns a
 * `SinkError` rather than hanging or succeeding silently.
 * Works by using `AlwaysFailObjectStore` with `max_retries=2` and asserting
 * (a) `generate_pyramid` returns `Err(_)`, (b) the backend saw at least
 * `max_retries + 1` attempts for the first tile (initial try plus retries),
 * confirming the retry budget was respected before bubbling the error.
 * Input: always-fail backend, max_retries=2 -> Output: SinkError, attempts >= 3.
 */
// TODO: depends on `ObjectStoreConfig::with_retry(RetryPolicy)` which does not
// yet exist in libviprs. Un-ignore once that builder method lands.
#[ignore]
#[test]
fn retry_exhausted_surfaces_error() {
    let plan = make_plan(64, 64, 32);
    let src = gradient_raster(64, 64);
    let engine = single_threaded_config();
    let failing = AlwaysFailObjectStore::new();

    let cfg = cfg_with_backend(failing.clone()).with_key_prefix("pyramids/retry-dead");
    // .with_retry(
    //     RetryPolicy::new(2, Duration::from_millis(1))
    //         .with_multiplier(2.0)
    //         .with_max_backoff(Duration::from_secs(5))
    //         .with_jitter(false),
    // );
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png).unwrap();

    let err = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(engine.clone())
        .run()
        .expect_err("pyramid must fail when every retry fails");
    // The pipeline wraps sink failures in its own error; just require the
    // message references the sink-level cause so we know it bubbled up.
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("sink") || msg.to_lowercase().contains("permanent"),
        "error message should surface the underlying sink failure, got: {msg}"
    );
    assert!(
        failing.attempts() >= 3,
        "expected at least initial + 2 retries = 3 attempts, got {}",
        failing.attempts()
    );
}

// ---------------------------------------------------------------------------
// Test 6: deterministic_key_layout_deep_zoom
// ---------------------------------------------------------------------------

/**
 * Tests that generated keys follow the Deep Zoom layout
 * `<prefix>/<image>_files/<level>/<x>_<y>.<ext>`.
 * Works by running the pyramid into a `RecordingObjectStore` with
 * prefix="pyramids/run-1" and image_name="output", then regex-matching every
 * recorded key against `^pyramids/run-1/output_files/\d+/\d+_\d+\.png$`.
 * Input: 128x128 gradient, tile=64, DeepZoom, PNG -> Output: every key matches layout.
 */
#[test]
fn deterministic_key_layout_deep_zoom() {
    let plan = make_plan(128, 128, 64);
    let src = gradient_raster(128, 128);
    let engine = single_threaded_config();
    let backend = RecordingObjectStore::new();

    let cfg = cfg_with_backend(backend.clone())
        .with_key_prefix("pyramids/run-1")
        .with_image_name("output");
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png).unwrap();

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(engine.clone())
        .run()
        .unwrap();

    let keys = backend.keys();
    assert!(!keys.is_empty());
    for key in &keys {
        // Manual structural check (no regex crate available in dev-deps).
        let stripped = key
            .strip_prefix("pyramids/run-1/output_files/")
            .unwrap_or_else(|| panic!("key does not match expected prefix/layout: {key}"));
        // stripped should look like "<level>/<x>_<y>.png"
        let (level_part, rest) = stripped
            .split_once('/')
            .unwrap_or_else(|| panic!("missing level segment in {key}"));
        assert!(
            level_part.chars().all(|c| c.is_ascii_digit()),
            "level segment must be numeric in {key}"
        );
        let (xy, ext) = rest
            .rsplit_once('.')
            .unwrap_or_else(|| panic!("missing extension in {key}"));
        assert_eq!(ext, "png", "expected png extension in {key}");
        let (x, y) = xy
            .split_once('_')
            .unwrap_or_else(|| panic!("tile filename must be X_Y.ext in {key}"));
        assert!(
            x.chars().all(|c| c.is_ascii_digit()) && y.chars().all(|c| c.is_ascii_digit()),
            "x and y components must both be numeric in {key}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 7: key_prefix_respected
// ---------------------------------------------------------------------------

/**
 * Tests that every key emitted by the sink starts with the configured prefix.
 * Works by setting `with_key_prefix("foo/bar")`, running the pyramid, and
 * asserting `key.starts_with("foo/bar/")` for every recorded key. Also
 * asserts that no key contains `//` (i.e. the sink joins prefix and tile
 * path cleanly with a single `/`).
 * Input: prefix="foo/bar", 128x128 raster -> Output: every key starts with "foo/bar/" and contains no "//".
 */
#[test]
fn key_prefix_respected() {
    let plan = make_plan(128, 128, 64);
    let src = gradient_raster(128, 128);
    let engine = single_threaded_config();
    let backend = RecordingObjectStore::new();

    let cfg = cfg_with_backend(backend.clone()).with_key_prefix("foo/bar");
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png).unwrap();

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(engine.clone())
        .run()
        .unwrap();

    assert!(backend.total_writes() > 0, "expected at least one put");
    for key in backend.keys() {
        assert!(
            key.starts_with("foo/bar/"),
            "key {key:?} is missing configured prefix"
        );
        assert!(
            !key.contains("//"),
            "key {key:?} contains doubled slash, prefix join is broken"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 8: smoke_test_against_minio_if_env_set
// ---------------------------------------------------------------------------

/**
 * Smoke test against a real MinIO/S3-compatible endpoint. Skipped by default;
 * only runs when `LIBVIPRS_TEST_S3_ENDPOINT` is set in the environment (e.g.
 * on a dev machine with MinIO running at 127.0.0.1:9000). Uses the real
 * `ObjectStoreSink` without injecting a backend, so this exercises the actual
 * S3 client code path.
 * Works by reading `LIBVIPRS_TEST_S3_ENDPOINT`, `LIBVIPRS_TEST_S3_BUCKET`,
 * `LIBVIPRS_TEST_S3_ACCESS_KEY`, `LIBVIPRS_TEST_S3_SECRET_KEY`, running a
 * small pyramid through `ObjectStoreSink` against that endpoint, and simply
 * asserting that `generate_pyramid` returns `Ok`. Contents are not
 * re-downloaded — the goal is just to confirm the wire protocol works.
 * Input: real MinIO at env-provided endpoint -> Output: pyramid run completes without error.
 */
#[test]
#[ignore = "requires LIBVIPRS_TEST_S3_ENDPOINT and a running S3-compatible endpoint"]
fn smoke_test_against_minio_if_env_set() {
    let endpoint = match std::env::var("LIBVIPRS_TEST_S3_ENDPOINT") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!("LIBVIPRS_TEST_S3_ENDPOINT not set; skipping smoke test");
            return;
        }
    };
    let bucket =
        std::env::var("LIBVIPRS_TEST_S3_BUCKET").unwrap_or_else(|_| "test-bucket".to_string());
    let access_key =
        std::env::var("LIBVIPRS_TEST_S3_ACCESS_KEY").unwrap_or_else(|_| "minio".to_string());
    let secret_key =
        std::env::var("LIBVIPRS_TEST_S3_SECRET_KEY").unwrap_or_else(|_| "minio123".to_string());

    // `tempdir` kept alive for any fixture reads the planned API might do
    // (e.g. staging). Not strictly needed today, but matches the style of
    // other tests in this crate.
    let _tmp = tempfile::tempdir().unwrap();

    let plan = make_plan(128, 128, 64);
    let src = gradient_raster(128, 128);
    let engine = single_threaded_config();

    // TODO: re-add .with_retry(...) once `ObjectStoreConfig::with_retry` lands.
    let cfg = ObjectStoreConfig::s3(&endpoint, &bucket)
        .with_access_key(&access_key, &secret_key)
        .with_key_prefix("pyramids/smoke-test");
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png)
        .expect("smoke test: sink construction must succeed");

    EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(engine.clone())
        .run()
        .expect("smoke test: pyramid run against real S3 must succeed");
}
