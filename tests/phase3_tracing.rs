//! Phase 3 hardening — failing integration tests (TDD).
//!
//! These tests describe the *planned* public API for two hardening
//! deliverables that do not yet exist in `libviprs`:
//!
//!   (1) Additional counters on `EngineResult`:
//!         - `bytes_read`
//!         - `bytes_written`
//!         - `retry_count`
//!         - `queue_pressure_peak`
//!         - `duration`
//!         - `stage_durations: StageDurations { planning, decode,
//!                                              resize, encode, sink }`
//!
//!   (2) A `tracing` feature that makes `generate_pyramid` emit a
//!       structured span tree:
//!
//!         libviprs::pipeline   (root)
//!         └─ libviprs::level    { level_index }
//!            └─ libviprs::tile  { x, y, level }
//!               ├─ libviprs::encode
//!               └─ libviprs::sink_write
//!
//! **Expected state: these tests do NOT compile today.** That is
//! intentional — they are the spec for Phase 3 and drive the core
//! implementation. Once the fields / spans exist, every test here
//! must pass without modification.
//!
//! No core-crate changes are made by this file.

use libviprs::{EngineBuilder, EngineConfig, EngineKind, Layout, MemorySink, PyramidPlanner};

mod common;
use common::fixtures::canonical_raster_scaled;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

// ===========================================================================
// Part 1 — EngineResult counter extensions
// ===========================================================================

#[test]
fn result_has_bytes_written_after_run() {
    let src = canonical_raster_scaled(512, 384);
    let planner = PyramidPlanner::new(512, 384, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap();

    assert!(
        result.bytes_written > 0,
        "expected bytes_written > 0 after a successful run, got {}",
        result.bytes_written
    );
}

#[test]
fn result_has_bytes_read_after_run() {
    let src = canonical_raster_scaled(512, 384);
    let planner = PyramidPlanner::new(512, 384, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap();

    assert!(
        result.bytes_read > 0,
        "expected bytes_read > 0 after a successful run, got {}",
        result.bytes_read
    );
}

#[test]
fn result_duration_is_positive() {
    let src = canonical_raster_scaled(256, 256);
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap();

    assert!(
        result.duration > std::time::Duration::from_nanos(0),
        "expected wall-clock duration > 0ns, got {:?}",
        result.duration
    );
}

#[test]
fn stage_durations_sum_roughly_equals_total() {
    let src = canonical_raster_scaled(512, 512);
    let planner = PyramidPlanner::new(512, 512, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap();

    let stages = &result.stage_durations;
    let stage_sum = stages.planning + stages.decode + stages.resize + stages.encode + stages.sink;

    // Stage timings measure disjoint work phases. Due to concurrency
    // and measurement overhead they may *not* be equal to wall-clock,
    // but the sum of single-threaded stage accounting should never
    // exceed wall-clock by more than a small slack (allow 50 ms of
    // measurement jitter).
    let slack = std::time::Duration::from_millis(50);
    assert!(
        stage_sum <= result.duration + slack,
        "sum of stage durations {stage_sum:?} exceeds total {:?} + slack {slack:?}",
        result.duration
    );
}

#[test]
fn retry_count_zero_on_happy_path() {
    let src = canonical_raster_scaled(256, 256);
    let planner = PyramidPlanner::new(256, 256, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap();

    assert_eq!(
        result.retry_count, 0,
        "happy-path run should require no retries, got {}",
        result.retry_count
    );
}

#[test]
fn queue_pressure_peak_bounded_by_concurrency() {
    let src = canonical_raster_scaled(1024, 1024);
    let planner = PyramidPlanner::new(1024, 1024, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let concurrency: u32 = 4;
    let config = EngineConfig::default().with_concurrency(concurrency as usize);
    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(config)
        .run()
        .unwrap();

    // The peak in-flight tile count should never exceed the configured
    // concurrency by more than a small fudge factor (e.g. +1 for the
    // tile currently being handed off to the sink).
    let fudge: u32 = 2;
    assert!(
        result.queue_pressure_peak <= concurrency + fudge,
        "queue_pressure_peak {} exceeds concurrency {} + fudge {}",
        result.queue_pressure_peak,
        concurrency,
        fudge,
    );
    assert!(
        result.queue_pressure_peak > 0,
        "queue_pressure_peak should be > 0 after a real run",
    );
}

#[test]
fn memory_sink_bytes_written_matches_payload_size() {
    let src = canonical_raster_scaled(512, 384);
    let planner = PyramidPlanner::new(512, 384, 128, 0, Layout::DeepZoom).unwrap();
    let plan = planner.plan();
    let sink = MemorySink::new();

    let result = EngineBuilder::new(&src, plan.clone(), &sink)
        .with_engine(EngineKind::Monolithic)
        .with_config(EngineConfig::default())
        .run()
        .unwrap();

    // Planned API: `CollectedTile` exposes the final encoded tile via
    // a `raster` accessor whose `data()` returns the raw bytes that
    // were written to the sink. Summing those lengths must equal the
    // new `bytes_written` counter exactly — no double-counting, no
    // framing overhead, no rounding.
    let expected: u64 = sink
        .tiles()
        .iter()
        .map(|tile| tile.raster.data().len() as u64)
        .sum();

    assert_eq!(
        result.bytes_written, expected,
        "bytes_written {} does not equal Σ tile.raster.data().len() {}",
        result.bytes_written, expected,
    );
}

// ===========================================================================
// Part 2 — tracing-span integration (feature-gated)
// ===========================================================================
//
// These tests require the `tracing` feature to be enabled:
//
//     cargo test -p libviprs-tests --features tracing --test phase3_tracing
//
// They use a custom `tracing_subscriber::Layer` that records span
// lifecycle events into an `Arc<Mutex<Vec<_>>>` so we can make
// precise assertions about name, count, and recorded fields without
// parsing formatted text.

#[cfg(feature = "tracing")]
mod tracing_tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id};
    use tracing::subscriber::with_default;
    use tracing::{Event, Subscriber};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::registry::{LookupSpan, Registry};

    /// A single captured span record: the span's target::name and a
    /// snapshot of the fields recorded at creation time.
    #[derive(Debug, Clone)]
    struct CapturedSpan {
        /// `target::name`, e.g. `"libviprs::pipeline"` or
        /// `"libviprs::tile"`.
        qualified_name: String,
        /// Fields captured from the `Attributes` at `on_new_span`.
        /// Values are stored as their `Debug` formatting, which is
        /// sufficient for assertion purposes.
        fields: HashMap<String, String>,
    }

    /// Thread-safe collector of span records.
    #[derive(Default, Clone)]
    struct SpanCollector {
        spans: Arc<Mutex<Vec<CapturedSpan>>>,
    }

    impl SpanCollector {
        fn new() -> Self {
            Self::default()
        }

        fn snapshot(&self) -> Vec<CapturedSpan> {
            self.spans.lock().unwrap().clone()
        }
    }

    struct FieldCollector<'a>(&'a mut HashMap<String, String>);

    impl<'a> Visit for FieldCollector<'a> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0
                .insert(field.name().to_string(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.0.insert(field.name().to_string(), value.to_string());
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.0.insert(field.name().to_string(), value.to_string());
        }
    }

    impl<S> Layer<S> for SpanCollector
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
            let meta = attrs.metadata();
            let qualified_name = format!("{}::{}", meta.target(), meta.name());
            let mut fields = HashMap::new();
            attrs.record(&mut FieldCollector(&mut fields));
            self.spans.lock().unwrap().push(CapturedSpan {
                qualified_name,
                fields,
            });
        }

        fn on_event(&self, _event: &Event<'_>, _ctx: Context<'_, S>) {
            // not used
        }
    }

    /// Runs `generate_pyramid` with a fresh `SpanCollector` installed
    /// as the default subscriber and returns both the captured spans
    /// and the `EngineResult` for follow-up assertions.
    fn run_capturing_spans(
        w: u32,
        h: u32,
        tile_size: u32,
    ) -> (
        Vec<CapturedSpan>,
        libviprs::PyramidPlan,
        libviprs::EngineResult,
    ) {
        let src = canonical_raster_scaled(w, h);
        let planner = PyramidPlanner::new(w, h, tile_size, 0, Layout::DeepZoom).unwrap();
        let plan = planner.plan();
        let sink = MemorySink::new();

        let collector = SpanCollector::new();
        let subscriber = Registry::default().with(collector.clone());

        let plan_for_run = plan.clone();
        let result = with_default(subscriber, || {
            EngineBuilder::new(&src, plan_for_run, &sink)
                .with_engine(EngineKind::Monolithic)
                .with_config(EngineConfig::default())
                .run()
                .unwrap()
        });

        (collector.snapshot(), plan, result)
    }

    /// Count captured spans whose `qualified_name` matches exactly.
    fn count_named(spans: &[CapturedSpan], name: &str) -> usize {
        spans.iter().filter(|s| s.qualified_name == name).count()
    }

    #[test]
    fn emits_pipeline_span() {
        let (spans, _plan, _result) = run_capturing_spans(256, 256, 128);

        let pipeline_spans = count_named(&spans, "libviprs::pipeline");
        assert_eq!(
            pipeline_spans, 1,
            "expected exactly one libviprs::pipeline span, got {pipeline_spans}",
        );
    }

    #[test]
    fn emits_span_per_level() {
        let (spans, plan, _result) = run_capturing_spans(512, 384, 128);

        let level_spans = count_named(&spans, "libviprs::level");
        assert_eq!(
            level_spans,
            plan.level_count(),
            "expected {} libviprs::level spans to match plan, got {}",
            plan.level_count(),
            level_spans,
        );
    }

    #[test]
    fn emits_span_per_tile() {
        let (spans, _plan, result) = run_capturing_spans(512, 384, 128);

        let tile_spans = count_named(&spans, "libviprs::tile") as u64;
        let expected = result.tiles_produced + result.tiles_skipped;
        assert_eq!(
            tile_spans, expected,
            "expected {} libviprs::tile spans (tiles_produced + tiles_skipped), got {}",
            expected, tile_spans,
        );
    }

    #[test]
    fn level_span_carries_level_index_field() {
        let (spans, plan, _result) = run_capturing_spans(512, 384, 128);

        let level_spans: Vec<&CapturedSpan> = spans
            .iter()
            .filter(|s| s.qualified_name == "libviprs::level")
            .collect();
        assert_eq!(
            level_spans.len(),
            plan.level_count(),
            "sanity: expected one level span per plan level",
        );

        // Every level span must carry a `level_index` field, and the
        // set of recorded indices must equal 0..level_count().
        let mut recorded_indices: Vec<u32> = level_spans
            .iter()
            .map(|s| {
                s.fields
                    .get("level_index")
                    .unwrap_or_else(|| {
                        panic!(
                            "libviprs::level span missing `level_index` field; \
                             recorded fields: {:?}",
                            s.fields
                        )
                    })
                    .parse::<u32>()
                    .expect("level_index must parse as u32")
            })
            .collect();
        recorded_indices.sort_unstable();

        let expected: Vec<u32> = (0..plan.level_count() as u32).collect();
        assert_eq!(
            recorded_indices,
            expected,
            "recorded level_index values do not cover 0..{}",
            plan.level_count(),
        );
    }

    #[test]
    fn tile_span_carries_coords() {
        let (spans, _plan, _result) = run_capturing_spans(256, 256, 128);

        let tile_spans: Vec<&CapturedSpan> = spans
            .iter()
            .filter(|s| s.qualified_name == "libviprs::tile")
            .collect();
        assert!(
            !tile_spans.is_empty(),
            "expected at least one libviprs::tile span",
        );

        for span in tile_spans {
            for required in &["x", "y", "level"] {
                assert!(
                    span.fields.contains_key(*required),
                    "libviprs::tile span missing `{required}` field; recorded fields: {:?}",
                    span.fields,
                );
            }
        }
    }
}
