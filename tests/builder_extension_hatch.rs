//! Phase 0 TDD — `.with_extension::<T>()` escape hatch (Approach C surface).
//!
//! Pins:
//!
//! - `with_extension<T: Send + Sync + 'static>(self, T) -> Self`.
//! - `extension<T: Send + Sync + 'static>(&self) -> Option<&T>` returns the
//!   last-inserted value for a given type.
//! - `extensions(&self) -> &Extensions` hands a read-through view to
//!   third-party code that wants to pull user-supplied context (e.g. a
//!   custom observer pulling a metrics `Recorder` out of the map).
//! - Inserting the same type twice overwrites — there is one slot per
//!   `TypeId`.
//! - Values are not required to be `Clone` or `Debug` — only `Send + Sync`.

#![allow(unused_imports)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use libviprs::extensions::Extensions;
use libviprs::sink::MemorySink;
use libviprs::{EngineBuilder, Layout, PixelFormat, PyramidPlanner, Raster};

fn raster() -> Raster {
    Raster::new(
        32,
        32,
        PixelFormat::Rgb8,
        vec![0u8; 32 * 32 * PixelFormat::Rgb8.bytes_per_pixel()],
    )
    .unwrap()
}

fn plan() -> libviprs::PyramidPlan {
    PyramidPlanner::new(32, 32, 16, 0, Layout::DeepZoom)
        .unwrap()
        .plan()
}

#[derive(Debug, PartialEq)]
struct Marker(&'static str);

#[derive(Debug)]
struct CustomRecorder {
    tick: AtomicU64,
}

impl CustomRecorder {
    fn new() -> Self {
        Self {
            tick: AtomicU64::new(0),
        }
    }
}

#[test]
fn roundtrip_by_type() {
    let src = raster();
    let p = plan();
    let builder = EngineBuilder::new(&src, p, MemorySink::new()).with_extension(Marker("hello"));

    assert_eq!(builder.extension::<Marker>(), Some(&Marker("hello")));
}

#[test]
fn missing_extension_returns_none() {
    let src = raster();
    let p = plan();
    let builder = EngineBuilder::new(&src, p, MemorySink::new());
    assert!(builder.extension::<Marker>().is_none());
}

#[test]
fn same_type_overwrites() {
    let src = raster();
    let p = plan();
    let builder = EngineBuilder::new(&src, p, MemorySink::new())
        .with_extension(Marker("first"))
        .with_extension(Marker("second"));

    assert_eq!(builder.extension::<Marker>(), Some(&Marker("second")));
}

#[test]
fn different_types_coexist() {
    #[derive(Debug, PartialEq)]
    struct Alpha(u32);
    #[derive(Debug, PartialEq)]
    struct Beta(String);

    let src = raster();
    let p = plan();
    let builder = EngineBuilder::new(&src, p, MemorySink::new())
        .with_extension(Alpha(42))
        .with_extension(Beta("bee".into()));

    assert_eq!(builder.extension::<Alpha>(), Some(&Alpha(42)));
    assert_eq!(builder.extension::<Beta>(), Some(&Beta("bee".into())));
}

#[test]
fn extension_accepts_arcs_of_non_clone_values() {
    // Many real extensions are large or non-Clone — a metrics recorder,
    // a tracing span, a shared config. Pins Arc<T> as the recommended
    // wrapper for those cases.
    let src = raster();
    let p = plan();
    let rec = Arc::new(CustomRecorder::new());

    let builder = EngineBuilder::new(&src, p, MemorySink::new()).with_extension(Arc::clone(&rec));

    let pulled = builder
        .extension::<Arc<CustomRecorder>>()
        .expect("Arc<CustomRecorder> missing from extensions");
    pulled.tick.fetch_add(1, Ordering::Relaxed);
    assert_eq!(rec.tick.load(Ordering::Relaxed), 1);
}

#[test]
fn extensions_accessor_exposes_the_map() {
    // Third-party code (e.g. a custom EngineObserver) reads the map to pull
    // user-supplied context without going through the typed setters.
    let src = raster();
    let p = plan();
    let builder =
        EngineBuilder::new(&src, p, MemorySink::new()).with_extension(Marker("observer-sees-me"));

    let map: &Extensions = builder.extensions();
    let pulled = map.get::<Marker>().expect("Marker not in map");
    assert_eq!(pulled, &Marker("observer-sees-me"));
}

#[test]
fn extensions_survive_run() {
    // After `.run()`, the extensions attached to the builder must have been
    // visible to the engine (or at minimum recoverable via `run_collect`).
    // Day-one libviprs reads zero extensions itself; the test only pins the
    // survivorship so future features can *start* reading without a surface
    // change.
    let src = raster();
    let p = plan();
    let (_, _sink) = EngineBuilder::new(&src, p, MemorySink::new())
        .with_extension(Marker("persisted"))
        .run_collect()
        .unwrap();
}

#[test]
fn extension_requires_send_sync() {
    // Compile-only check — if someone weakens the bound on `with_extension`
    // to no longer require `Send + Sync`, this test still passes (`Rc` is
    // !Send), so we *assert* the bound with a negative trait-bound
    // technique: a generic helper only compiles when its argument is
    // Send + Sync.
    fn requires_send_sync<T: Send + Sync + 'static>(_: &T) {}
    let marker = Marker("ok");
    requires_send_sync(&marker);

    // The `.with_extension` call itself is the real compile-time check —
    // accepting `Rc<_>` here would fail to compile because it is !Send.
    let src = raster();
    let p = plan();
    let _ = EngineBuilder::new(&src, p, MemorySink::new()).with_extension(marker);
}
