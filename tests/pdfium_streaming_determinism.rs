//! Determinism across **independent** `PdfiumStripSource` constructions.
//!
//! The existing `streaming_render_strip_is_deterministic` test in
//! `pdfium_streaming_render.rs` calls `render_strip` twice on the
//! **same** source instance. That pins call-level determinism but
//! misses a category of bug: any global / process-wide state that
//! could differ between source constructions (lazy pdfium init order,
//! thread-local state, document handle caching that diverges across
//! instances) would only show up when comparing across source
//! lifetimes.
//!
//! This test constructs two independent `PdfiumStripSource` instances
//! with identical arguments, renders the same strip from each, and
//! asserts byte-equality. A divergence here would point at hidden
//! shared state that the source's contract should not have.
//!
//! Both cached and streaming modes are exercised — the property must
//! hold for either constructor.

#![cfg(feature = "pdfium")]

mod common;

use common::{FIXTURE_BLUEPRINT, FIXTURE_PORTRAIT};
use libviprs::PdfiumStripSource;
use libviprs::streaming::StripSource;

const DPI: u32 = 72;

#[test]
fn cached_mode_two_sources_same_args_produce_byte_identical_strips() {
    let a = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let b = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let strip_a = a.render_strip(0, 128).unwrap();
    let strip_b = b.render_strip(0, 128).unwrap();
    assert_eq!(
        strip_a.data(),
        strip_b.data(),
        "cached: two independently-constructed sources diverge on identical render_strip(0, 128)"
    );
}

#[test]
fn streaming_mode_two_sources_same_args_produce_byte_identical_strips() {
    let a = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let b = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let strip_a = a.render_strip(0, 128).unwrap();
    let strip_b = b.render_strip(0, 128).unwrap();
    assert_eq!(
        strip_a.data(),
        strip_b.data(),
        "streaming: two independently-constructed sources diverge on identical render_strip(0, 128)"
    );
}

#[test]
fn streaming_mode_byte_identical_across_drop_recreate() {
    // Construct, render, drop, construct again, render. If document
    // caching or pdfium init has hidden per-instance state that mutates
    // global pdfium internals, this is where it would surface.
    let strip_a = {
        let s = PdfiumStripSource::new_streaming(FIXTURE_PORTRAIT, 1, DPI).unwrap();
        s.render_strip(0, 64).unwrap().data().to_vec()
    };
    let strip_b = {
        let s = PdfiumStripSource::new_streaming(FIXTURE_PORTRAIT, 1, DPI).unwrap();
        s.render_strip(0, 64).unwrap().data().to_vec()
    };
    assert_eq!(
        strip_a, strip_b,
        "streaming: drop-and-recreate produced divergent bytes for identical render_strip(0, 64)"
    );
}

#[test]
fn streaming_mode_interleaved_renders_remain_deterministic() {
    // Two sources alive simultaneously, alternating render calls. The
    // (currently-not-cached, but soon-cached) document handle inside
    // each source must not be perturbed by the other source's calls.
    let a = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, DPI).unwrap();
    let b = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, DPI).unwrap();

    let a_first = a.render_strip(0, 128).unwrap().data().to_vec();
    let _b_first = b.render_strip(0, 128).unwrap();
    let a_second = a.render_strip(0, 128).unwrap().data().to_vec();
    let _b_second = b.render_strip(64, 128).unwrap();
    let a_third = a.render_strip(0, 128).unwrap().data().to_vec();

    assert_eq!(
        a_first, a_second,
        "interleaved: source A diverged after one B call"
    );
    assert_eq!(
        a_second, a_third,
        "interleaved: source A diverged after two B calls"
    );
}
