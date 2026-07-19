//! CLI-DIFFERENTIAL suite — draw family (the Wave-2 draw lane,
//! CLI_CONTRACT.md §3.6/§5/§7, OP_MAP.md draw section).
//!
//! **Every draw op is GOLDEN-ONLY.** `vips draw_*` are in-place mutators whose
//! CLI DISCARDS the mutated image (the op returns nothing to save), so there is
//! **NO vips CLI oracle** to cross-check against. Each reference under
//! `tests/fixtures/cli/draw/` is generated ONCE by `viprs` itself (deterministic)
//! by `tools/gen_cli_expected.sh` and committed; every test below is a
//! **regression pin** — it decode-compares `viprs` output against the committed
//! `viprs`-minted golden at tolerance 0, and it does NOT claim vips parity (there
//! is none for these ops).
//!
//! To keep each golden HONEST (a broken / no-op draw must fail the pin, not
//! silently match), every case ALSO asserts the output differs from its INPUT
//! (`drew_something`), and the outline/fill and bounded/blob pairs assert they
//! differ from each other — the discriminating inputs (a 2-D gradient, three flat
//! flood stripes, a high-frequency smudge input) are chosen so a vacuous op is
//! caught.
//!
//! If the `libviprs-cli` sibling is not checked out (the default CI `test` job
//! and the Docker gate clone only the core counterpart), every test SKIPS with a
//! clear message rather than failing — the dedicated `cli-differential` CI job
//! lays the CLI down and actually exercises these (CLI_CONTRACT.md §7).

mod common;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use common::cli::{
    cli_available, cli_fixture, decode_compare, max_abs_diff, run_viprs, run_viprs_ok,
};

use tempfile::TempDir;

/// GOLDEN-ONLY pin tolerance: bit-exact decode comparison against the committed
/// `viprs`-generated reference (CLI_CONTRACT.md §5). This is a regression pin,
/// NOT a cross-implementation check — there is no vips oracle for `draw_*`.
const GOLDEN: f64 = 0.0;

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
///
/// When `$VIPRS_REQUIRE_CLI=1` (the dedicated `cli-differential` CI job) an
/// absent sibling instead PANICS inside [`cli_available`], so this never returns
/// `true` there — a would-be silent skip becomes a hard failure.
fn skip_if_no_cli(test: &str) -> bool {
    if cli_available() {
        return false;
    }
    eprintln!(
        "SKIP {test}: libviprs-cli sibling not checked out \
         (set $VIPRS_CLI_DIR / $VIPRS_BIN, or run in the cli-differential job)."
    );
    true
}

/// Absolute path to a fresh output file inside a process-lifetime temp dir.
fn out_path(name: &str) -> PathBuf {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::tempdir().expect("create temp dir"));
    dir.path().join(name)
}

// Common committed inputs (consumed only as `viprs` inputs; never as an oracle).
const RGB: &str = "draw/rgb.png";
const FLOOD: &str = "draw/flood.png";
const MASK: &str = "draw/mask.png";
const SUB: &str = "draw/sub.png";
const SMUDGE: &str = "draw/smudge.png";

/// Absolute string path to a committed fixture (for passing to `viprs`).
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().expect("utf-8 path").to_string()
}

/// Decode a raster via the libviprs decoder (the same decoder the harness uses).
fn decode(path: &Path) -> libviprs::Raster {
    libviprs::decode_file(path)
        .unwrap_or_else(|e| panic!("failed to decode {} via libviprs: {e}", path.display()))
}

/// Assert the draw actually changed pixels: `out` must differ from the INPUT it
/// was drawn onto (max-abs-diff > 0). This de-rigs the GOLDEN-ONLY pin — a no-op
/// or broken draw would leave `out == input` and fail here even if the committed
/// golden were regenerated from the same broken op. Draw ops preserve the input's
/// dimensions / format, so the two are directly comparable.
fn drew_something(out: &Path, input_rel: &str) {
    let a = decode(out);
    let b = decode(&cli_fixture(input_rel));
    let diff = max_abs_diff(&a, &b);
    assert!(
        diff > 0.0,
        "draw output {} is identical to its input {input_rel} (max-abs-diff 0): the op did \
         nothing — a vacuous golden, not a real draw",
        out.display(),
    );
}

/// Max-abs-diff between two produced outputs (both drawn onto the same input, so
/// same dims/format). Used to assert two variants (outline/fill, bounded/blob)
/// are genuinely distinct.
fn outputs_differ(a: &Path, b: &Path) -> f64 {
    max_abs_diff(&decode(a), &decode(b))
}

// ---------------------------------------------------------------------------
// draw_circle — outline + filled disc.
// ---------------------------------------------------------------------------

#[test]
fn draw_circle_outline_golden_pin() {
    if skip_if_no_cli("draw_circle_outline") {
        return;
    }
    let out = out_path("draw_circle.png");
    run_viprs_ok(&[
        "draw_circle",
        &fx(RGB),
        out.to_str().unwrap(),
        "16",
        "16",
        "8",
        "--ink",
        "255 0 0",
    ]);
    // GOLDEN-ONLY: compared against a committed viprs-generated reference, not
    // vips — there is no cross-oracle for an in-place draw op.
    decode_compare(&out, &cli_fixture("draw/draw_circle_golden.png"), GOLDEN);
    drew_something(&out, RGB);
}

#[test]
fn draw_circle_fill_golden_pin() {
    if skip_if_no_cli("draw_circle_fill") {
        return;
    }
    let out = out_path("draw_circle_fill.png");
    run_viprs_ok(&[
        "draw_circle",
        &fx(RGB),
        out.to_str().unwrap(),
        "16",
        "16",
        "8",
        "--ink",
        "255 0 0",
        "--fill",
    ]);
    decode_compare(
        &out,
        &cli_fixture("draw/draw_circle_fill_golden.png"),
        GOLDEN,
    );
    drew_something(&out, RGB);

    // The --fill flag must actually change the result: a filled disc paints the
    // interior spans the outline leaves untouched, so the two outputs differ.
    let outline = out_path("draw_circle_for_cmp.png");
    run_viprs_ok(&[
        "draw_circle",
        &fx(RGB),
        outline.to_str().unwrap(),
        "16",
        "16",
        "8",
        "--ink",
        "255 0 0",
    ]);
    assert!(
        outputs_differ(&out, &outline) > 0.0,
        "draw_circle --fill must differ from the outline (the --fill flag is not a no-op)"
    );
}

// ---------------------------------------------------------------------------
// draw_rect — outline + filled.
// ---------------------------------------------------------------------------

#[test]
fn draw_rect_outline_golden_pin() {
    if skip_if_no_cli("draw_rect_outline") {
        return;
    }
    let out = out_path("draw_rect.png");
    run_viprs_ok(&[
        "draw_rect",
        &fx(RGB),
        out.to_str().unwrap(),
        "4",
        "4",
        "20",
        "16",
        "--ink",
        "0 255 0",
    ]);
    decode_compare(&out, &cli_fixture("draw/draw_rect_golden.png"), GOLDEN);
    drew_something(&out, RGB);
}

#[test]
fn draw_rect_fill_golden_pin() {
    if skip_if_no_cli("draw_rect_fill") {
        return;
    }
    let out = out_path("draw_rect_fill.png");
    run_viprs_ok(&[
        "draw_rect",
        &fx(RGB),
        out.to_str().unwrap(),
        "4",
        "4",
        "20",
        "16",
        "--ink",
        "0 255 0",
        "--fill",
    ]);
    decode_compare(&out, &cli_fixture("draw/draw_rect_fill_golden.png"), GOLDEN);
    drew_something(&out, RGB);

    let outline = out_path("draw_rect_for_cmp.png");
    run_viprs_ok(&[
        "draw_rect",
        &fx(RGB),
        outline.to_str().unwrap(),
        "4",
        "4",
        "20",
        "16",
        "--ink",
        "0 255 0",
    ]);
    assert!(
        outputs_differ(&out, &outline) > 0.0,
        "draw_rect --fill must differ from the outline"
    );
}

// ---------------------------------------------------------------------------
// draw_line.
// ---------------------------------------------------------------------------

#[test]
fn draw_line_golden_pin() {
    if skip_if_no_cli("draw_line") {
        return;
    }
    let out = out_path("draw_line.png");
    run_viprs_ok(&[
        "draw_line",
        &fx(RGB),
        out.to_str().unwrap(),
        "0",
        "0",
        "31",
        "31",
        "--ink",
        "0 0 255",
    ]);
    decode_compare(&out, &cli_fixture("draw/draw_line_golden.png"), GOLDEN);
    drew_something(&out, RGB);
}

// ---------------------------------------------------------------------------
// draw_flood — bounded + blob (--equal). Both region-limited on the 0/100/200
// stripe input, and distinct from each other.
// ---------------------------------------------------------------------------

#[test]
fn draw_flood_bounded_golden_pin() {
    if skip_if_no_cli("draw_flood_bounded") {
        return;
    }
    let out = out_path("draw_flood.png");
    // Bounded flood from the left (0) stripe with ink 100: fills the 0 stripe up
    // to the pre-existing 100 wall and STOPS (the 200 stripe is untouched).
    run_viprs_ok(&[
        "draw_flood",
        &fx(FLOOD),
        out.to_str().unwrap(),
        "0",
        "0",
        "--ink",
        "100",
    ]);
    decode_compare(&out, &cli_fixture("draw/draw_flood_golden.png"), GOLDEN);
    drew_something(&out, FLOOD);
}

#[test]
fn draw_flood_blob_golden_pin() {
    if skip_if_no_cli("draw_flood_blob") {
        return;
    }
    let out = out_path("draw_flood_blob.png");
    // Blob flood (--equal) from the same seed: recolours only the seed's own
    // equal-valued (0) stripe → 50, leaving 100 and 200 untouched.
    run_viprs_ok(&[
        "draw_flood",
        &fx(FLOOD),
        out.to_str().unwrap(),
        "0",
        "0",
        "--ink",
        "50",
        "--equal",
    ]);
    decode_compare(
        &out,
        &cli_fixture("draw/draw_flood_blob_golden.png"),
        GOLDEN,
    );
    drew_something(&out, FLOOD);

    // The --equal blob variant must differ from the bounded flood (different fill
    // semantics AND different ink), so the flag path is not a no-op.
    let bounded = out_path("draw_flood_for_cmp.png");
    run_viprs_ok(&[
        "draw_flood",
        &fx(FLOOD),
        bounded.to_str().unwrap(),
        "0",
        "0",
        "--ink",
        "100",
    ]);
    assert!(
        outputs_differ(&out, &bounded) > 0.0,
        "draw_flood --equal (blob) must differ from the bounded flood"
    );
}

// ---------------------------------------------------------------------------
// draw_mask — ink blended through a single-band 8-bit opacity stencil.
// ---------------------------------------------------------------------------

#[test]
fn draw_mask_golden_pin() {
    if skip_if_no_cli("draw_mask") {
        return;
    }
    let out = out_path("draw_mask.png");
    run_viprs_ok(&[
        "draw_mask",
        &fx(RGB),
        out.to_str().unwrap(),
        &fx(MASK),
        "8",
        "8",
        "--ink",
        "255 255 0",
    ]);
    decode_compare(&out, &cli_fixture("draw/draw_mask_golden.png"), GOLDEN);
    drew_something(&out, RGB);
}

// ---------------------------------------------------------------------------
// draw_smudge — box-blur of a rect over a high-frequency input (no ink).
// ---------------------------------------------------------------------------

#[test]
fn draw_smudge_golden_pin() {
    if skip_if_no_cli("draw_smudge") {
        return;
    }
    let out = out_path("draw_smudge.png");
    run_viprs_ok(&[
        "draw_smudge",
        &fx(SMUDGE),
        out.to_str().unwrap(),
        "4",
        "4",
        "8",
        "8",
    ]);
    decode_compare(&out, &cli_fixture("draw/draw_smudge_golden.png"), GOLDEN);
    // The high-frequency (0/255) input guarantees the 3x3 box-blur changes many
    // pixels — a smooth-gradient input would make this vacuous.
    drew_something(&out, SMUDGE);
}

// ---------------------------------------------------------------------------
// draw_image — paste a sub-image (no ink).
// ---------------------------------------------------------------------------

#[test]
fn draw_image_golden_pin() {
    if skip_if_no_cli("draw_image") {
        return;
    }
    let out = out_path("draw_image.png");
    run_viprs_ok(&[
        "draw_image",
        &fx(RGB),
        out.to_str().unwrap(),
        &fx(SUB),
        "10",
        "10",
    ]);
    decode_compare(&out, &cli_fixture("draw/draw_image_golden.png"), GOLDEN);
    drew_something(&out, RGB);
}

// ---------------------------------------------------------------------------
// Error paths (CLI_CONTRACT.md §8): a bad user input is a typed nonzero exit,
// NEVER a panic/abort. These need no reference output.
// ---------------------------------------------------------------------------

/// Run `viprs` expecting a NONZERO exit and a stderr substring, asserting it did
/// not abort (exit 101 = a Rust panic) — the bands B2 no-panic-on-user-input rule.
fn expect_error(args: &[&str], needle: &str) {
    let out = run_viprs(args);
    assert!(
        !out.status.success(),
        "viprs {args:?} unexpectedly succeeded (expected a typed error)"
    );
    // A Rust panic aborts with code 101; a typed error is exit 1/2. Guard the
    // no-panic contract explicitly.
    assert_ne!(
        out.status.code(),
        Some(101),
        "viprs {args:?} ABORTED (exit 101 = panic); a bad input must be a typed error, not a panic"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(needle),
        "viprs {args:?} stderr did not contain {needle:?}:\n{stderr}"
    );
}

#[test]
fn draw_image_format_mismatch_is_typed_error_not_panic() {
    if skip_if_no_cli("draw_image_format_mismatch") {
        return;
    }
    let out = out_path("draw_image_err.png");
    // SUB = the 1-band mask.png, IN = the 3-band rgb.png: the core paste is a
    // no-op on a format mismatch, which the CLI surfaces as a typed exit-1 error
    // rather than silently writing an unchanged image.
    expect_error(
        &[
            "draw_image",
            &fx(RGB),
            out.to_str().unwrap(),
            &fx(MASK),
            "0",
            "0",
        ],
        "identical format",
    );
}

#[test]
fn draw_ink_wrong_band_count_is_typed_error_not_panic() {
    if skip_if_no_cli("draw_ink_band_count") {
        return;
    }
    let out = out_path("draw_ink_err.png");
    // Two ink values for a 3-band image: a typed exit-1 error, not a core
    // check_ink panic and not a silently-cycled ink.
    expect_error(
        &[
            "draw_circle",
            &fx(RGB),
            out.to_str().unwrap(),
            "5",
            "5",
            "3",
            "--ink",
            "255 0",
        ],
        "band",
    );
}

#[test]
fn draw_flood_out_of_bounds_seed_is_typed_error_not_panic() {
    if skip_if_no_cli("draw_flood_oob") {
        return;
    }
    let out = out_path("draw_flood_err.png");
    // A seed past the right edge (flood.png is 32 wide): the core returns
    // SeedOutOfBounds, surfaced as a typed exit-1 error.
    expect_error(
        &[
            "draw_flood",
            &fx(FLOOD),
            out.to_str().unwrap(),
            "999",
            "0",
            "--ink",
            "50",
        ],
        "out of bounds",
    );
}
