//! CLI-DIFFERENTIAL suite — io save-path cleanup (libviprs-cli #37 / #38,
//! `CLI_CONTRACT.md` §2/§7).
//!
//! This suite exercises the shared save harness (`libviprs-cli/src/ops/io.rs`),
//! not a single op family: it pins the two save-path issues resolved in the
//! `iocleanup` lane.
//!
//! # #38 — the PNM (`.ppm` / `.pgm`) sink
//!
//! `.ppm`/`.pgm` are the one differential sink that is **byte-exact**: PNM is an
//! uncompressed, canonical container (a fixed `Pn\n<w> <h>\n<maxval>\n` header
//! then raw big-endian samples — no filters, no palette, no metadata), so two
//! conformant encoders that agree on the pixels emit byte-identical files.
//! These cases therefore [`byte_compare`] `viprs …→.ppm/.pgm` against the vips
//! 8.18.4 oracle — a strictly stronger assertion than the [`decode_compare`]
//! the PNG/TIFF sinks must use. The committed references are the vips output
//! with its non-pixel `#vips2ppm - <timestamp>` comment stripped by
//! `tools/gen_cli_expected.sh` (see `PROVENANCE.md`); the pixel payload and the
//! `w`/`h`/`maxval` tokens are vips's own bytes. Coverage: P6 (3-band) and P5
//! (1-band), each at 8-bit (maxval 255) and 16-bit (maxval 65535, big-endian),
//! plus the float-reject error path.
//!
//! # #37 — 16-bit depth on the integer PNG sink
//!
//! vips's `pngsave` picks its bit depth from the raster **interpretation**
//! (`grey16`/`rgb16` → 16-bit; `b-w`/`multiband`/`srgb` → 8-bit), and `io::save`
//! mirrors that. The honest limitation (documented on `io.rs`
//! `cast_float_to_integer_round_even`, verified against the vips oracle) is that
//! libviprs' EXACT-AFTER-CAST ops drop `grey16`→`multiband` on their float
//! result, so a *float EAC* op → `.png` cannot preserve 16-bit end-to-end
//! without core changes (16-bit EAC is pinned losslessly via `.v` in
//! `cli_core_diff.rs` `add_gray_expected.v`). What CAN be pinned here — and is —
//! is the 16-bit PNG *save path* through a format-preserving op (`copy`, which
//! keeps `Rgb16`): `viprs copy rgb16.v out.png` must stay 16-bit and decode-match
//! vips, exactly as `vips copy rgb16.v out.png` does.
//!
//! If the `libviprs-cli` sibling is not checked out every test SKIPS with a
//! clear message rather than failing; the dedicated `cli-differential` CI job
//! (`$VIPRS_REQUIRE_CLI=1`) lays the CLI down and actually exercises these.

mod common;

use std::path::PathBuf;
use std::sync::OnceLock;

use common::cli::{
    byte_compare, cli_available, cli_fixture, decode_compare, run_viprs, run_viprs_ok,
};

use tempfile::TempDir;

/// EXACT oracle class: bit-exact decode comparison (`CLI_CONTRACT.md` §5).
const EXACT: f64 = 0.0;

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
/// Under `$VIPRS_REQUIRE_CLI=1` (the `cli-differential` CI job) an absent sibling
/// instead PANICS inside [`cli_available`], so a would-be silent skip hard-fails.
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

/// Convenience: the absolute string path of a committed fixture.
fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

// Committed inputs, read identically by `viprs` (here) and by vips (the
// generator): rgb.png / gray.png are 8×8 8-bit; rgb16.v / gray16.v are the same
// scaled to the full 0..65535 range (samples > 255, so a silent 8-bit downcast
// is caught on VALUE as well as on format-class).
const RGB: &str = "iocleanup/rgb.png";
const GRAY: &str = "iocleanup/gray.png";
const RGB16: &str = "iocleanup/rgb16.v";
const GRAY16: &str = "iocleanup/gray16.v";

// ---------------------------------------------------------------------------
// #38 — PNM byte-exact differentials.
// ---------------------------------------------------------------------------

/// P6 (3-band) 8-bit: `flip horizontal` on the RGB input → `.ppm`, byte-exact.
#[test]
fn flip_ppm_matches_vips_bytes() {
    if skip_if_no_cli("flip_ppm") {
        return;
    }
    let out = out_path("flip.ppm");
    run_viprs_ok(&["flip", &fx(RGB), out.to_str().unwrap(), "horizontal"]);
    byte_compare(&out, &cli_fixture("iocleanup/flip_ppm_expected.ppm"));
}

/// P5 (1-band) 8-bit: `flip horizontal` on the gray input → `.pgm`, byte-exact.
/// The magic is chosen by band count, so a 1-band raster is PGM/P5.
#[test]
fn flip_pgm_matches_vips_bytes() {
    if skip_if_no_cli("flip_pgm") {
        return;
    }
    let out = out_path("flip.pgm");
    run_viprs_ok(&["flip", &fx(GRAY), out.to_str().unwrap(), "horizontal"]);
    byte_compare(&out, &cli_fixture("iocleanup/flip_pgm_expected.pgm"));
}

/// P6 (3-band) 16-bit: `copy` of the rgb16 input → `.ppm`, maxval 65535,
/// big-endian samples, byte-exact. `copy` keeps `Rgb16`, so the 16-bit payload
/// is preserved (a downcast would change every byte).
#[test]
fn copy_ppm16_matches_vips_bytes() {
    if skip_if_no_cli("copy_ppm16") {
        return;
    }
    let out = out_path("copy16.ppm");
    run_viprs_ok(&["copy", &fx(RGB16), out.to_str().unwrap()]);
    byte_compare(&out, &cli_fixture("iocleanup/copy_ppm16_expected.ppm"));
}

/// P5 (1-band) 16-bit: `copy` of the gray16 input → `.pgm`, maxval 65535,
/// big-endian, byte-exact.
#[test]
fn copy_pgm16_matches_vips_bytes() {
    if skip_if_no_cli("copy_pgm16") {
        return;
    }
    let out = out_path("copy16.pgm");
    run_viprs_ok(&["copy", &fx(GRAY16), out.to_str().unwrap()]);
    byte_compare(&out, &cli_fixture("iocleanup/copy_pgm16_expected.pgm"));
}

/// The float-reject error path (`CLI_CONTRACT.md` §8): a float-producing op to a
/// `.ppm` sink exits nonzero with a `viprs`-side message. No vips oracle — this
/// pins the CLI's own contract (PNM is integer-only; float goes to `.png`/`.v`).
#[test]
fn float_to_ppm_is_rejected() {
    if skip_if_no_cli("float_to_ppm") {
        return;
    }
    let out = out_path("float.ppm");
    // `linear a=1.5` yields a float raster; PNM must refuse it.
    let res = run_viprs(&["linear", &fx(RGB), out.to_str().unwrap(), "1.5", "0"]);
    assert!(
        !res.status.success(),
        "viprs linear …→.ppm on a float result must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&res.stderr);
    assert!(
        stderr.contains("integer-only"),
        "expected a PNM integer-only reject message, got stderr: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// #37 — 16-bit PNG save path (decode-compare; see the module header for why a
// float-EAC 16-bit .png differential is impractical and documented instead).
// ---------------------------------------------------------------------------

/// `copy rgb16.v out.png` must save a 16-bit PNG whose decode matches vips's
/// 16-bit PNG exactly. `decode_compare` asserts equal dimensions, band count,
/// and — crucially — format class (`bytes_per_channel`), so an 8-bit downcast
/// fails on format-class alone (and on value, the samples span > 255).
#[test]
fn copy_png16_stays_16bit_matches_vips() {
    if skip_if_no_cli("copy_png16") {
        return;
    }
    let out = out_path("copy16.png");
    run_viprs_ok(&["copy", &fx(RGB16), out.to_str().unwrap()]);
    decode_compare(
        &out,
        &cli_fixture("iocleanup/copy_png16_expected.png"),
        EXACT,
    );
}
