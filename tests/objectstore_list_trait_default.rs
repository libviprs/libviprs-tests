//! Integration contract for the defaulted `ObjectStore::list` trait method
//! and `ObjectStoreSink::list_objects`'s delegation to it (issue #465).
//!
//! Historically `ObjectStoreSink::list_objects()` hardcoded
//! `SinkError::Unsupported` inside the sink. The extensible shape is a
//! *defaulted* trait method `ObjectStore::list(&self, prefix)` that refuses
//! loud by default, with `list_objects()` delegating to the injected backend:
//!
//!   * a listing-capable backend overrides `ObjectStore::list` and returns
//!     real keys — the sink surfaces them, no sink edit required, and
//!   * a write-only backend (one that only implements `put`) inherits the
//!     trait's defaulted loud failure, so a stub can never masquerade a
//!     populated backend as an empty `Ok(vec![])`.
//!
//! This is a trait-delegation / API-shape assertion, not a pixel-level golden:
//! there is no image output to diff against a vips reference, so no vips
//! fixture is used. Gated behind the `object-store-sink` feature (with `s3`
//! kept as a deprecated alias) since the module it exercises is feature-gated
//! in the core crate.

#![cfg(feature = "object-store-sink")]

use std::sync::{Arc, Mutex};

use libviprs::sink::{ObjectStore, ObjectStoreConfig, ObjectStoreSink};
use libviprs::{
    Layout, PixelFormat, PyramidPlanner, Raster, SinkError, Tile, TileCoord, TileFormat, TileSink,
};

/// A listing-capable backend: records every `(key, bytes)` handed to `put`, and
/// *overrides* [`ObjectStore::list`] to return the keys it holds (filtered by
/// the requested prefix). Also captures the last prefix it was asked to list,
/// so the test can prove the sink forwards its configured `key_prefix`.
#[derive(Default)]
struct ListingStore {
    puts: Mutex<Vec<(String, Vec<u8>)>>,
    last_prefix: Mutex<Option<String>>,
}

impl ListingStore {
    /// The keys the backend actually recorded via `put`, sorted.
    fn recorded_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .puts
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        keys.sort();
        keys
    }
}

impl ObjectStore for ListingStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), SinkError> {
        self.puts
            .lock()
            .unwrap()
            .push((key.to_string(), bytes.to_vec()));
        Ok(())
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>, SinkError> {
        *self.last_prefix.lock().unwrap() = Some(prefix.to_string());
        Ok(self
            .puts
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .filter(|k| k.starts_with(prefix))
            .collect())
    }
}

/// A write-only backend: implements only `put`, so it *inherits* the trait's
/// defaulted `list` (a loud refusal). Models a real transport that can upload
/// but cannot enumerate.
#[derive(Default)]
struct WriteOnlyStore {
    puts: Mutex<usize>,
}

impl ObjectStore for WriteOnlyStore {
    fn put(&self, _key: &str, _bytes: &[u8]) -> Result<(), SinkError> {
        *self.puts.lock().unwrap() += 1;
        Ok(())
    }
    // Deliberately does NOT override `list` — inherits the trait default.
}

fn small_deepzoom_plan() -> libviprs::PyramidPlan {
    PyramidPlanner::new(256, 256, 256, 0, Layout::DeepZoom)
        .expect("planner params are valid")
        .plan()
}

/// A backend that overrides `ObjectStore::list` makes `list_objects` return the
/// keys the backend actually holds — proving the sink *delegates* enumeration
/// to the backend instead of hardcoding `SinkError::Unsupported`.
#[test]
fn list_objects_delegates_to_overriding_backend() {
    let store = Arc::new(ListingStore::default());
    // Configure a prefix wrapped in slashes: the write path normalises
    // `key_prefix` with `trim_matches('/')` before building each object key, so
    // `list_objects` must forward the *same* trimmed prefix — otherwise a
    // listing would query a key space no written key carries.
    let cfg = ObjectStoreConfig::s3("http://localhost:9000", "bucket")
        .with_key_prefix("/run-1/")
        .with_image_name("deleg")
        .with_object_store(store.clone());
    let plan = small_deepzoom_plan();
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png)
        .expect("sink constructs once a backend is injected");

    // Upload every tile of the pyramid so the backend holds real objects to
    // enumerate (a DeepZoom plan spans several levels, each a single tile).
    for level in &plan.levels {
        for row in 0..level.rows {
            for col in 0..level.cols {
                let tile = Tile {
                    coord: TileCoord::new(level.level, col, row),
                    raster: Raster::zeroed(256, 256, PixelFormat::Rgb8).unwrap(),
                    blank: false,
                };
                sink.write_tile(&tile).expect("tile upload succeeds");
            }
        }
    }

    let recorded = store.recorded_keys();
    assert!(
        recorded.len() > 1,
        "test must drive more than one tile through the sink, got {recorded:?}"
    );

    let mut listed = sink
        .list_objects()
        .expect("list_objects must delegate to the overriding backend and succeed");
    listed.sort();
    assert_eq!(
        listed, recorded,
        "list_objects must return exactly the keys the backend recorded"
    );

    // The sink forwarded its configured key_prefix to the backend's `list`,
    // *trimmed* of leading/trailing slashes to match the write path's
    // normalisation — the backend must see "run-1", not the raw "/run-1/".
    assert_eq!(
        store.last_prefix.lock().unwrap().as_deref(),
        Some("run-1"),
        "list_objects must delegate with the write-path-trimmed key_prefix"
    );
}

/// A write-only backend inherits the trait's defaulted `list`, so
/// `list_objects` still fails loud with `SinkError::Unsupported` — even after
/// real uploads. This is the behaviour the sink used to hardcode; it now lives
/// on the trait default, so the refusal is preserved for backends that cannot
/// list.
#[test]
fn list_objects_inherits_loud_default_for_write_only_backend() {
    let store = Arc::new(WriteOnlyStore::default());
    let cfg = ObjectStoreConfig::s3("http://localhost:9000", "bucket")
        .with_key_prefix("run-2")
        .with_image_name("wo")
        .with_object_store(store.clone());
    let plan = small_deepzoom_plan();
    let sink = ObjectStoreSink::new(cfg, plan, TileFormat::Png)
        .expect("sink constructs once a backend is injected");

    let tile = Tile {
        coord: TileCoord::new(0, 0, 0),
        raster: Raster::zeroed(256, 256, PixelFormat::Rgb8).unwrap(),
        blank: false,
    };
    sink.write_tile(&tile).expect("tile upload succeeds");
    assert_eq!(
        *store.puts.lock().unwrap(),
        1,
        "backend must have recorded the upload"
    );

    let err = sink
        .list_objects()
        .expect_err("write-only backend must inherit the loud-fail default");
    assert!(
        matches!(err, SinkError::Unsupported(_)),
        "expected SinkError::Unsupported, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("list_objects") && msg.contains("LIST"),
        "default list error must name the operation and the missing transport: {msg}"
    );
}

/// Calling the *defaulted* trait method directly on a write-only backend
/// refuses loud — the default lives on the trait, not on the sink.
#[test]
fn default_trait_list_method_refuses_loud() {
    let store = WriteOnlyStore::default();
    let err =
        ObjectStore::list(&store, "any-prefix").expect_err("defaulted ObjectStore::list refuses");
    assert!(
        matches!(err, SinkError::Unsupported(_)),
        "expected SinkError::Unsupported, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("list_objects") && msg.contains("LIST"),
        "default list error must name the operation and the missing transport: {msg}"
    );
}
