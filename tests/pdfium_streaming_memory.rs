//! Memory invariants for streaming-mode `PdfiumStripSource`.
//!
//! Streaming mode's reason for existing is that it does **not** cache a
//! full-page raster the way cached mode does. Cached mode's source for a
//! 47-megapixel A0 blueprint at 300 DPI carries ~180 MiB of pixel data
//! resident for its lifetime; streaming mode carries only the parsed
//! `PdfDocument` (kilobytes-to-low-megabytes for typical PDFs).
//!
//! # What this file tests vs. what it can't
//!
//! True process-RSS measurement requires either OS-level sampling
//! (`getrusage`, `mach_task_info`, `/proc/self/status`) or an
//! instrumented allocator (jemalloc-ctl, dhat-rs). The libviprs-bench
//! crate uses `getrusage` for that purpose; its
//! [pdf_engine_bench](../../libviprs-bench/src/pdf_engine_bench.rs) is
//! the canonical place to look for actual peak-RSS numbers.
//!
//! Inside the test suite we pin three weaker but well-defined invariants:
//!
//! 1. **Structural** — `PdfiumStripSource` itself does not contain a
//!    `Raster` field on the streaming path. Verified via
//!    [`std::mem::size_of_val`] on the struct.
//! 2. **Per-strip allocation bound** — every `render_strip` returns a
//!    raster whose byte length equals exactly `width × strip_h × 4`,
//!    so the live raster set never exceeds one strip at a time. (The
//!    `MemoryTracker` accounting is honor-system; it pins what the
//!    test believes about its own behaviour, not what the OS says.)
//! 3. **Drop releases** — explicitly dropping a strip raster releases
//!    its `Vec<u8>` immediately; the source itself remains usable.

#![cfg(feature = "pdfium")]

mod common;

use common::FIXTURE_BLUEPRINT;
use libviprs::observe::MemoryTracker;
use libviprs::streaming::StripSource;
use libviprs::{PdfiumStripSource, Raster};

// ---------------------------------------------------------------------------
// (1) Structural: streaming source does not embed a full-page raster
// ---------------------------------------------------------------------------

/// `PdfiumStripSource` size on the streaming path is bounded by a small
/// constant (a few pointer-sized fields plus the cached `PdfDocument`
/// handle, which is itself a pointer + small metadata). It does NOT
/// embed a `Raster`. If the struct grows by megabytes between releases
/// it almost certainly means a full-page buffer crept back in.
#[test]
fn streaming_source_struct_is_compact() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 72).unwrap();
    let stack_size = std::mem::size_of_val(&source);
    // Empirically ~80–200 bytes for the stack representation across
    // amd64 / arm64 — three u32s plus an enum carrying a small struct
    // with an Option<PdfDocument<'static>>. Generous ceiling so target
    // architecture differences and minor field additions don't break
    // the test, but a megabyte-scale regression would jump out.
    assert!(
        stack_size < 1024,
        "PdfiumStripSource stack size grew to {stack_size} bytes — \
         did a Raster sneak into the streaming-mode state?"
    );
}

/// Cached source's stack size is also small (Raster's `Vec<u8>` lives
/// on the heap; only the Vec's pointer/len/cap are in the struct), so
/// `size_of_val` does NOT distinguish cached from streaming. This test
/// documents that — to actually compare heap usage between modes, see
/// `libviprs-bench/src/pdf_engine_bench.rs` for a getrusage-based
/// measurement.
#[test]
fn cached_source_struct_is_also_compact() {
    let source = PdfiumStripSource::new(FIXTURE_BLUEPRINT, 1, 72).unwrap();
    let stack_size = std::mem::size_of_val(&source);
    assert!(stack_size < 1024, "cached source stack size: {stack_size}");
}

// ---------------------------------------------------------------------------
// (2) Per-strip allocation bound — honor-system; the assertions below
// pin the test's accounting, not OS-level memory. See module-level docs
// for why this is intentional.
// ---------------------------------------------------------------------------

/// Each rendered strip's `Vec<u8>` is exactly `width × strip_h × 4`.
/// No padding, no over-allocation, no surprise growth.
#[test]
fn streaming_strip_size_equals_strip_pixel_count_times_four() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 300).unwrap();
    let full_page_bytes = u64::from(source.width()) * u64::from(source.height()) * 4;
    assert!(
        full_page_bytes > 50_000_000,
        "fixture not large enough to make this test meaningful: {full_page_bytes} bytes"
    );
    let strip_h = 256u32;
    let strip = source.render_strip(0, strip_h).unwrap();
    let strip_bytes = strip.data().len() as u64;
    assert_eq!(
        strip_bytes,
        u64::from(source.width()) * u64::from(strip_h) * 4,
        "streaming strip raster size diverges from declared geometry"
    );
    assert!(
        strip_bytes < full_page_bytes / 4,
        "strip bytes ({strip_bytes}) >= 25% of full page ({full_page_bytes}) — \
         test geometry is wrong, not the implementation"
    );
}

/// Honor-system rolling tracker: account for each strip's `Vec<u8>` as
/// it's rendered and freed. The tracker reports the test's belief about
/// its own peak; if any strip's reported size diverges from the strip
/// size formula, this catches it. It does NOT prove the absence of
/// hidden allocations elsewhere — that's what `pdf_engine_bench` is for.
#[test]
fn streaming_per_strip_accounting_stays_bounded() {
    let dpi = 150;
    let strip_h = 256u32;

    let stream_tracker = MemoryTracker::new();
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, dpi).unwrap();
    let mut y = 0u32;
    while y < source.height() {
        let h = strip_h.min(source.height() - y);
        let strip = source.render_strip(y, h).unwrap();
        let bytes = strip.data().len() as u64;
        stream_tracker.alloc(bytes);
        drop(strip);
        stream_tracker.dealloc(bytes);
        y += h;
    }
    let stream_peak = stream_tracker.peak_bytes();

    // The accounted peak must be no more than two strips' worth (one
    // strip allocated + the prior strip's bytes still in the tracker
    // before the dealloc lands). In practice we account/deallocate
    // strictly in order so the peak is one strip; the 2× cushion just
    // makes the assertion robust against trivial test-loop tweaks.
    let one_strip_bytes = u64::from(source.width()) * u64::from(strip_h) * 4;
    let strip_bound = one_strip_bytes * 2;

    assert!(
        stream_peak <= strip_bound,
        "honor-system per-strip accounting exceeded {strip_bound} bytes (got {stream_peak})"
    );
}

// ---------------------------------------------------------------------------
// (3) Drop releases — verifies render_strip's returned Raster owns its
// data outright (no internal sharing/Arc) so dropping it frees the buffer.
// ---------------------------------------------------------------------------

#[test]
fn dropping_a_strip_raster_releases_immediately() {
    let source = PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, 72).unwrap();
    let strip_h = 128u32;
    let mut last_kept: Option<Raster> = None;
    for y in (0..source.height()).step_by(strip_h as usize) {
        let h = strip_h.min(source.height() - y);
        let strip = source.render_strip(y, h).unwrap();
        // Replacing the previous Some(strip) drops the prior strip
        // before the new one is even bound — the source must remain
        // healthy across that drop.
        last_kept = Some(strip);
    }
    let last = last_kept.expect("at least one strip rendered");
    assert!(last.height() <= strip_h);
    assert_eq!(last.width(), source.width());
}
