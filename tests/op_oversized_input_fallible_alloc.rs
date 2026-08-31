//! Follow-ups to libviprs/libviprs#460 (from the PR #459 panel review of the
//! fallible op-scratch work, #433 / #434 / #435).
//!
//! PR #459 routed the two known infallible op-scratch buffers — the
//! `try_stdif` integral images and the `try_hough_circle` vote accumulator —
//! through the fallible `try_scratch` path so an over-capacity size returns
//! `RasterError::AllocationFailed` instead of calling `handle_alloc_error`
//! and aborting (SIGABRT) the whole process. #460 closes the remaining gaps:
//!
//! 1. **Sibling infallible scratch.** `project()` builds two input-scaled `f64`
//!    accumulators (`col_sums` / `row_sums`) with an infallible `vec![..]`. A
//!    wide, short raster makes `col_sums` up to ~8x the input, so a legal large
//!    input could reach `handle_alloc_error` and abort. `project` has no error
//!    channel (it returns `(Raster, Raster)`), so — like its output allocation,
//!    already guarded by `op_output_or_panic` — the scratch is now routed
//!    through a fallible path that *panics* on an unsatisfiable size rather than
//!    aborting. A panic unwinds and is catchable; an abort is not.
//!
//! 2. **`try_stdif` not unit-exercised.** #459 fixed `try_stdif`'s abort path
//!    but could not test it: its scratch is only ~16x its input, so overflowing
//!    it under the 8 GiB `DEFAULT_MAX_ALLOC_BYTES` construction budget needs a
//!    ~8 TiB input that cannot be built. #460 asked for a lowered-cap test hook.
//!
//! The abort-safety guarantee for (1) and (2) is a pure behavioural contract
//! with no libvips analogue, and it is reached only through a per-thread scratch
//! cap that must stay private to the crate (shipping it in production builds
//! would permanently widen the public surface — the #460 panel follow-up). Both
//! abort-safety tests therefore live *in-crate*, next to that `cfg(test)`-only
//! hook, in `libviprs::arithmetic`'s `mod tests`
//! (`project_oversize_scratch_panics_not_aborts` and
//! `stdif_oversize_scratch_returns_typed_error_not_abort`), co-located with the
//! `raster.rs` `alloc_op_output_is_fallible_not_aborting` precedent.
//!
//! What remains here is the part that needs no private hook: the
//! vips-differential golden for `project` itself, proving the scratch rework
//! left the numeric result untouched. libvips `project` on a `uchar` image sums
//! each column and each row; the crate produces the same sums in a 16-bit
//! container (saturating at `65535`, which the fixture stays well under). The
//! reference was produced offline with vips-8.18.4:
//!   `vips rawload proj_raw.bin project_input.png 5 4 1`
//!   `vips project project_input.png cols.v rows.v`
//!   `vips cast cols.v cols_u.v ushort`
//!   `vips copy cols_u.v project_columns_expected.png --interpretation grey16`
//!   (and likewise for the rows output)
//! and committed under
//! `tests/fixtures/op_oversized_input_fallible_alloc_expected/`.

use core::num::NonZeroU16;

use libviprs::{PixelFormat, Raster};

/// Directory holding the committed `project` input fixture and its offline
/// libvips reference outputs.
fn fixture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/op_oversized_input_fallible_alloc_expected")
}

/// Load a committed 8-bit grayscale PNG fixture as a `Gray8` [`Raster`].
fn load_gray8(name: &str) -> Raster {
    let img = image::open(fixture_dir().join(name))
        .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
        .to_luma8();
    let (w, h) = img.dimensions();
    Raster::new(w, h, PixelFormat::Gray8, img.into_raw()).expect("fixture raster is well-formed")
}

/// Read a native-endian `u32` sample stream out of a raster buffer.
///
/// `u32` and not `u16` since libviprs#532: `project` counts pixels, and
/// libvips emits every counting op as `VIPS_FORMAT_UINT`, so a 300x300 image
/// already overflows a 16-bit counter. The reference PNGs are 16-bit, which
/// is still the right comparison for a fixture whose sums fit: they are
/// widened rather than re-captured.
fn samples_u32(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|c| u32::from_ne_bytes(c.try_into().expect("chunks_exact(4) yields 4 bytes")))
        .collect()
}

/// vips-differential reference for `project`: the column and row sums must
/// match libvips `project` sample-for-sample. Guards that routing the
/// accumulators through the fallible scratch path left the arithmetic result
/// untouched. Runs at the default (unbounded) scratch ceiling.
#[test]
fn project_matches_libvips_project_reference() {
    let input = load_gray8("project_input.png");
    let (columns, rows) = input.project();

    let one = NonZeroU16::new(1).expect("one band");
    assert_eq!(
        columns.format(),
        PixelFormat::Uint32(one),
        "project emits the uint carrier, the way libvips emits every counting op"
    );
    assert_eq!(
        rows.format(),
        PixelFormat::Uint32(one),
        "and the row sums take the same carrier as the column sums"
    );
    assert_eq!(
        (columns.width(), columns.height()),
        (input.width(), 1),
        "columns is a width×1 image"
    );
    assert_eq!(
        (rows.width(), rows.height()),
        (1, input.height()),
        "rows is a 1×height image"
    );

    let expected_cols = image::open(fixture_dir().join("project_columns_expected.png"))
        .expect("read project_columns_expected.png")
        .to_luma16();
    let expected_rows = image::open(fixture_dir().join("project_rows_expected.png"))
        .expect("read project_rows_expected.png")
        .to_luma16();

    let widen = |v: Vec<u16>| -> Vec<u32> { v.into_iter().map(u32::from).collect() };
    assert_eq!(
        samples_u32(columns.data()),
        widen(expected_cols.into_raw()),
        "project column sums must equal the libvips reference sample-for-sample"
    );
    assert_eq!(
        samples_u32(rows.data()),
        widen(expected_rows.into_raw()),
        "project row sums must equal the libvips reference sample-for-sample"
    );
}
