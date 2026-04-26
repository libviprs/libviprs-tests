//! Concurrent `render_strip` correctness for streaming-mode
//! `PdfiumStripSource`.
//!
//! `PdfiumStripSource: Send + Sync`. The MapReduce engine (see
//! `streaming_mapreduce.rs:350-365`) spawns workers via
//! `std::thread::scope` and calls `render_strip` from each. This test
//! pins three properties:
//!
//! 1. **Correctness.** N concurrent `render_strip` calls each return a
//!    raster equivalent to what the same `(y, h)` pair would produce
//!    sequentially. No lost / corrupted strips.
//! 2. **No deadlock or panic.** The pdfium-render `sync` feature wraps
//!    every FPDF call in a process-wide mutex; multiple threads will
//!    queue on it. They must not deadlock or trigger any unwind path.
//! 3. **Self-consistency under concurrency.** Bytes returned by a
//!    concurrent strip render are byte-identical to bytes from the
//!    sequential render of the same strip. This is the single
//!    strongest correctness invariant we have for streaming mode.
//!
//! # What this test does *not* prove
//!
//! It does not prove **concurrency-induced speedup**. With the pdfium
//! `sync` feature on, all FPDF calls are serialised globally. N threads
//! over a streaming `PdfiumStripSource` produce sequential wall time.
//! That is a known limitation documented separately; this test verifies
//! that even though we don't get parallelism, we also don't get
//! corruption.

#![cfg(feature = "pdfium")]

mod common;

use std::sync::Arc;
use std::thread;

use common::FIXTURE_BLUEPRINT;
use libviprs::PdfiumStripSource;
use libviprs::streaming::StripSource;

const DPI: u32 = 72;
const THREADS: u32 = 4;

#[test]
fn concurrent_render_strip_does_not_deadlock_or_corrupt() {
    let source = Arc::new(
        PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, DPI).expect("streaming source"),
    );
    let h_total = source.height();
    let strip_h = (h_total / THREADS).max(1);

    // Sequential reference: render each strip on the main thread, in
    // order. Concurrent threads must produce byte-identical bytes for
    // the same (y, h) pair.
    let reference: Vec<Vec<u8>> = (0..THREADS)
        .map(|i| {
            let y = i * strip_h;
            let h = strip_h.min(h_total - y);
            source.render_strip(y, h).unwrap().data().to_vec()
        })
        .collect();

    // Concurrent run.
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(THREADS as usize);
        for i in 0..THREADS {
            let s = Arc::clone(&source);
            let reference_strip = reference[i as usize].clone();
            let strip_h = strip_h;
            handles.push(scope.spawn(move || {
                let y = i * strip_h;
                let h = strip_h.min(h_total - y);
                let strip = s.render_strip(y, h).expect("concurrent render_strip");
                assert_eq!(
                    strip.data(),
                    reference_strip.as_slice(),
                    "thread {i}: concurrent strip diverges from sequential reference at y={y}, h={h}"
                );
            }));
        }
        for h in handles {
            h.join().expect("worker panicked");
        }
    });
}

#[test]
fn concurrent_render_strip_arc_share_does_not_panic() {
    // Tighter property: two threads call render_strip on the SAME (y, h)
    // simultaneously. With the `sync` feature this is serialised; the
    // test pins that the serialisation is correct, not corrupt.
    let source = Arc::new(
        PdfiumStripSource::new_streaming(FIXTURE_BLUEPRINT, 1, DPI).expect("streaming source"),
    );
    let h_total = source.height();
    let strip_h = 64u32.min(h_total);
    let reference = source.render_strip(0, strip_h).unwrap().data().to_vec();

    thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let s = Arc::clone(&source);
                let reference = reference.clone();
                scope.spawn(move || {
                    let strip = s.render_strip(0, strip_h).unwrap();
                    assert_eq!(
                        strip.data(),
                        reference.as_slice(),
                        "concurrent same-strip render produced divergent bytes"
                    );
                })
            })
            .collect();
        for h in handles {
            h.join().expect("worker panicked");
        }
    });
}
