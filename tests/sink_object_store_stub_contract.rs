//! Integration contract for the `ObjectStoreSink` LIST stub.
//!
//! `ObjectStoreSink::list_objects()` is an as-yet-unbuilt object-store
//! transport: the injectable [`ObjectStore`] trait exposes only `put`, so the
//! sink structurally cannot enumerate what it uploaded. The historical stub
//! returned `Ok(Vec::new())` unconditionally — a silent-wrong-answer footgun
//! (issues #379/#384): a caller diffing server-side keys against a filesystem
//! reference could not tell "empty because nothing was uploaded" apart from
//! "empty because the stub ignores server state", so the diff either passed
//! falsely or failed spuriously.
//!
//! This test pins the *honest* contract: after uploading real tiles through the
//! sink (a recording backend captures every `put`), `list_objects()` must fail
//! loud with [`SinkError::Unsupported`] rather than masquerade the populated
//! backend as an empty list. It also confirms the sink genuinely uploads, so
//! the empty-listing behaviour cannot be mistaken for the sink doing nothing.
//!
//! This is a stub-*contract* assertion, not a pixel-level golden: there is no
//! image output to diff against a vips reference, so no vips fixture is used.
//! Gated behind the `object-store-sink` feature (with `s3` kept as a deprecated
//! alias) since the module it exercises is feature-gated in the core crate.

#![cfg(feature = "object-store-sink")]

use std::sync::{Arc, Mutex};

use libviprs::sink::{ObjectStore, ObjectStoreConfig, ObjectStoreSink};
use libviprs::{
    Layout, PixelFormat, PyramidPlanner, Raster, SinkError, Tile, TileCoord, TileFormat, TileSink,
};

/// A backend that records every `(key, bytes)` handed to `put`, so the test can
/// prove the sink actually uploaded and then confront that with what
/// `list_objects` reports.
#[derive(Default)]
struct RecordingStore {
    puts: Mutex<Vec<(String, Vec<u8>)>>,
}

impl RecordingStore {
    fn keys(&self) -> Vec<String> {
        self.puts
            .lock()
            .unwrap()
            .iter()
            .map(|(k, _)| k.clone())
            .collect()
    }
}

impl ObjectStore for RecordingStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<(), SinkError> {
        self.puts
            .lock()
            .unwrap()
            .push((key.to_string(), bytes.to_vec()));
        Ok(())
    }
}

/// Upload several tiles through the sink, then assert `list_objects` fails loud
/// with `SinkError::Unsupported` — even though the backend really did record
/// those uploads. Locks in the "even when objects exist" half of the contract
/// that the old vacuous `list_objects() == []` test never exercised.
#[test]
fn list_objects_is_unsupported_even_after_writes() {
    let store = Arc::new(RecordingStore::default());
    let cfg = ObjectStoreConfig::s3("http://localhost:9000", "bucket")
        .with_key_prefix("run-1")
        .with_image_name("contract")
        .with_object_store(store.clone());

    let plan = PyramidPlanner::new(512, 512, 256, 0, Layout::DeepZoom)
        .expect("planner params are valid")
        .plan();
    let sink = ObjectStoreSink::new(cfg, plan.clone(), TileFormat::Png)
        .expect("sink constructs once a backend is injected");

    // Drive every tile of the pyramid through the sink so the backend ends up
    // holding real uploaded objects (more than one, across levels).
    let mut uploaded = 0usize;
    for level in &plan.levels {
        for row in 0..level.rows {
            for col in 0..level.cols {
                let tile = Tile {
                    coord: TileCoord::new(level.level, col, row),
                    raster: Raster::zeroed(256, 256, PixelFormat::Rgb8).unwrap(),
                    blank: false,
                };
                sink.write_tile(&tile).expect("tile upload succeeds");
                uploaded += 1;
            }
        }
    }
    assert!(uploaded > 1, "test must drive more than one tile");

    // The backend really captured every upload...
    let recorded = store.keys();
    assert_eq!(
        recorded.len(),
        uploaded,
        "backend must record one put per uploaded tile, got {recorded:?}"
    );

    // ...yet the sink cannot enumerate them: the stub fails loud instead of
    // returning a misleading empty list that would make a server-vs-reference
    // diff pass falsely (empty) or fail spuriously.
    let err = sink
        .list_objects()
        .expect_err("list_objects must not report success while unimplemented");
    assert!(
        matches!(err, SinkError::Unsupported(_)),
        "expected SinkError::Unsupported, got {err:?}"
    );

    let msg = err.to_string();
    assert!(
        msg.contains("list_objects") && msg.contains("LIST"),
        "error must name the operation and the missing transport: {msg}"
    );
}
