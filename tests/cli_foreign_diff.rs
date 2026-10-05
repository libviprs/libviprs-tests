//! CLI-DIFFERENTIAL suite: load and save for every codec libviprs ships
//! (libviprs/libviprs-cli#65, `OP_MAP.md` foreign section).
//!
//! Each codec gets the same three kinds of cell:
//!
//! * **save**: `viprs <codec>save` writes `canonical_input.png` (or a small
//!   window of it for the uncompressed containers), and the file is decoded
//!   and compared against the file `vips <codec>save` wrote from the same
//!   input. EXACT for the lossless codecs; BOUNDED-TOL, with the tolerance
//!   and its measured basis stated on the constant, for JPEG, GIF and the
//!   Ultra HDR base image, whose encoders are not vips's.
//! * **load**: `viprs <codec>load` reads a file vips wrote (or, for the
//!   formats vips cannot write, a core oracle-capture fixture) and its pixels
//!   are compared against vips's own decode of that file.
//! * **option and limit**: every option flag has a cell that goes red if the
//!   flag is ignored, and every loader has a `--max-pixels 1` cell that must
//!   be refused before anything is written.
//! * **header lies**: a PNG, a PPM and a ragged CSV that declare gigabytes in
//!   a few bytes, run under a wall-clock deadline and an address-space cap,
//!   which only a refusal from the header can pass.
//! * **usage**: flag values and an OUT of `-` that clap refuses (exit 2)
//!   before anything is read.
//!
//! The references are `tests/fixtures/cli/foreign/`, made by
//! `GEN_ONLY=foreign ./tools/gen_cli_expected.sh` with vips 8.18.4 on the
//! native x86_64 NAS (`tools/Dockerfile.vips-oracle`; `PROVENANCE.md`, "codec
//! load/save references").
//!
//! # The feature-gated codecs
//!
//! JPEG XL, JPEG 2000, AVIF and SVG are cargo features of both the core and
//! the CLI. Their pixel cells need this crate built with the same feature
//! (the decode side of the comparison runs through the library), and they
//! drive a `viprs` built with exactly that feature, via `viprs_bin_for`,
//! which is the same per-configuration binary `cli_features.rs` builds.
//! Without the feature the cells are `#[ignore]`d with a reason naming the
//! `--features` flag to run them, so `cargo test` lists them as ignored rather
//! than passing them. What the bare build can check it does check: every
//! gated command refuses, exit 1, naming its feature.

mod common;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use common::cli::{
    byte_compare, cli_available, cli_fixture, cli_source_available, decode_compare, run_bin,
    viprs_bin, viprs_bin_for,
};

/// EXACT oracle class: bit-exact decode comparison (`CLI_CONTRACT.md` §5).
const EXACT: f64 = 0.0;

// Every tolerance below is the largest per-sample difference I measured
// between the two files, both decoded by the same libviprs decoder, on the
// native x86_64 NAS against the vips 8.18.4 references. They are exactly the
// measured value, not rounded up, because every encoder and decoder involved
// is deterministic: a cell that moves at all has changed.

/// BOUNDED-TOL for `jpegsave`. The core's JPEG encoder is not libjpeg-turbo,
/// so quantisation and rounding differ. Measured: 4 at Q 75, 3 at Q 95, 6 at
/// Q 50 with 4:4:4.
const JPEG_SAVE_TOL: f64 = 6.0;

/// BOUNDED-TOL for `jpegload`: the core decodes with zune-jpeg, vips with
/// libjpeg-turbo, and the two upsample 4:2:0 chroma differently. Measured 2
/// (through the CLI, with the cell's tolerance set to 0 to make it print).
const JPEG_LOAD_TOL: f64 = 2.0;

/// BOUNDED-TOL for `jpegload --shrink 2`. vips shrinks in the DCT domain; the
/// core decodes at full size and box-shrinks, so edges between the flat
/// quadrants land differently. Measured 47, mean 0.74.
const JPEG_SHRINK_TOL: f64 = 47.0;

/// BOUNDED-TOL for `gifsave` at its defaults: different quantisers and
/// different error diffusion over pure gradients. Measured 106. It is wide
/// because dither moves single pixels a long way; the cell exists to catch a
/// wrong palette or geometry, and the option cells below hold the flags to
/// their structure.
const GIF_SAVE_TOL: f64 = 106.0;

/// BOUNDED-TOL for `gifsave --dither 0`: no error diffusion, so the two
/// quantisers are much closer. Measured 37.
const GIF_NODITHER_TOL: f64 = 37.0;

/// BOUNDED-TOL for `pngsave --palette`: 256-colour quantisation of the same
/// gradients, different quantisers. Measured 51.
const PNG_PALETTE_TOL: f64 = 51.0;

/// BOUNDED-TOL for the Ultra HDR base image. The core picks its own gain map
/// and tone mapping (`libviprs::uhdr::encode_uhdr` says why: libuhdr's choice
/// is a policy with no specification to port), so the SDR base differs from
/// libuhdr's. Measured 82.
const UHDR_SAVE_TOL: f64 = 82.0;

/// BOUNDED-TOL for `uhdrload`: the base half is a JPEG, decoded by two
/// different decoders. Measured 2.
const UHDR_LOAD_TOL: f64 = 2.0;

// A max-only tolerance as wide as the GIF and Ultra HDR ones would still pass
// a file that is wrong almost everywhere by a little, so each of those cells
// carries a mean bound beside its max, the way `tests/codec_e2e.rs` pairs
// them. Each is the measured mean absolute difference over every sample,
// rounded up at the third decimal, on the native x86_64 NAS against the vips
// 8.18.4 references.

/// Mean bound for `gifsave` at its defaults (max [`GIF_SAVE_TOL`]).
/// Measured 3.22837.
const GIF_SAVE_MEAN: f64 = 3.229;
/// Mean bound for `gifsave --dither 0` (max [`GIF_NODITHER_TOL`]).
/// Measured 2.54873.
const GIF_NODITHER_MEAN: f64 = 2.549;
/// Mean bound for `pngsave --palette` (max [`PNG_PALETTE_TOL`]).
/// Measured 2.73796.
const PNG_PALETTE_MEAN: f64 = 2.738;
/// Mean bound for the Ultra HDR base image (max [`UHDR_SAVE_TOL`]).
/// Measured 11.77604.
const UHDR_SAVE_MEAN: f64 = 11.777;
/// Mean bound for `jpegload --shrink 2` (max [`JPEG_SHRINK_TOL`]).
/// Measured 0.74286.
const JPEG_SHRINK_MEAN: f64 = 0.743;

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

/// Skip-guard: `true` (with a printed reason) when the CLI sibling is absent.
/// Under `$VIPRS_REQUIRE_CLI=1` an absent sibling panics inside
/// [`cli_available`] instead.
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

/// A `viprs` built with exactly one codec feature, or `None` (a printed skip)
/// when the CLI source is absent.
#[cfg(any(feature = "jxl", feature = "jp2k", feature = "avif", feature = "svg"))]
fn gated_bin(test: &str, feature: &str) -> Option<PathBuf> {
    if !cli_source_available() {
        eprintln!("SKIP {test}: libviprs-cli source not checked out.");
        return None;
    }
    Some(viprs_bin_for(false, &[feature]))
}

/// Where a cell writes `name`. Inside cargo's per-target scratch directory,
/// so a run reuses one directory instead of leaving a new temp dir behind
/// each time (a static `TempDir` is never dropped), and every cell's names
/// are its own, so cells running in parallel never share a file.
fn out_path(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_foreign_diff");
    std::fs::create_dir_all(&dir).expect("create the cell output dir");
    dir.join(name)
}

/// A directory of its own for a cell that asserts what else lands beside its
/// output, emptied first so a previous run's files cannot answer for this one.
fn fresh_dir(name: &str) -> PathBuf {
    let dir = out_path(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the cell dir");
    dir
}

fn fx(rel: &str) -> String {
    cli_fixture(rel).to_str().unwrap().to_string()
}

/// `tests/fixtures/canonical_input.png`, the 256x256 RGBA8 save input.
fn canonical() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/canonical_input.png")
        .to_str()
        .unwrap()
        .to_string()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn describe(args: &[&str], out: &Output) -> String {
    format!(
        "viprs {args:?} exited {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Run `bin` and assert it exited 0.
fn ok_with(bin: &Path, args: &[&str]) {
    let out = run_bin(bin, args);
    assert!(out.status.success(), "{}", describe(args, &out));
}

fn ok(args: &[&str]) {
    ok_with(&viprs_bin(), args);
}

/// Run `bin` and assert a clean refusal: exit 1 (an operational failure, not a
/// usage error and not a panic), no `out` file left behind, and every one of
/// `needles` in stderr.
fn refused_with(bin: &Path, args: &[&str], out: &Path, needles: &[&str]) {
    let _ = std::fs::remove_file(out);
    let run = run_bin(bin, args);
    assert_eq!(run.status.code(), Some(1), "{}", describe(args, &run));
    assert!(!out.exists(), "a refused run still wrote {}", out.display());
    let stderr = String::from_utf8_lossy(&run.stderr).to_lowercase();
    for needle in needles {
        assert!(
            stderr.contains(&needle.to_lowercase()),
            "stderr does not mention {needle:?}\n{}",
            describe(args, &run)
        );
    }
}

fn refused(args: &[&str], out: &Path, needles: &[&str]) {
    refused_with(&viprs_bin(), args, out, needles);
}

/// Run `bin` and assert a usage error: exit 2 from argument parsing, nothing
/// written to `out`, every one of `needles` in stderr.
fn usage_error_with(bin: &Path, args: &[&str], out: &Path, needles: &[&str]) {
    let _ = std::fs::remove_file(out);
    let run = run_bin(bin, args);
    assert_eq!(run.status.code(), Some(2), "{}", describe(args, &run));
    assert!(!out.exists(), "a usage error still wrote {}", out.display());
    let stderr = String::from_utf8_lossy(&run.stderr);
    for needle in needles {
        assert!(
            stderr.contains(needle),
            "stderr does not mention {needle:?}\n{}",
            describe(args, &run)
        );
    }
}

fn usage_error(args: &[&str], out: &Path, needles: &[&str]) {
    usage_error_with(&viprs_bin(), args, out, needles);
}

/// The ceiling for a cell whose input lies about its size: refusing it is a
/// header check and an arithmetic, so anything near this means the CLI went
/// and did the work.
const LIE_DEADLINE: Duration = Duration::from_secs(20);

/// The address-space cap those cells run `viprs` under where `ulimit -v`
/// exists, 1 GiB. Every lie below asks for several times this, so a CLI that
/// tries to allocate dies on the cap (and fails the exit-code assertion)
/// instead of taking the runner's memory with it.
const LIE_VM_KIB: u64 = 1024 * 1024;

/// As [`refused`], under [`LIE_DEADLINE`] and, on unix, [`LIE_VM_KIB`]. A
/// child still running at the deadline is killed and the cell fails.
fn refused_fast(args: &[&str], out: &Path, needles: &[&str]) {
    let _ = std::fs::remove_file(out);
    let bin = viprs_bin();
    let mut cmd = if cfg!(unix) {
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(format!(
                "ulimit -v {LIE_VM_KIB} 2>/dev/null; exec \"$0\" \"$@\""
            ))
            .arg(&bin);
        c
    } else {
        Command::new(&bin)
    };
    let started = Instant::now();
    let mut child = cmd
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn viprs");
    while child.try_wait().expect("poll viprs").is_none() {
        if started.elapsed() > LIE_DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "viprs {args:?} was still running after {LIE_DEADLINE:?}: a header \
                 that lies about its size has to be refused before the decode"
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let run = child.wait_with_output().expect("collect viprs");
    let took = started.elapsed();
    assert_eq!(run.status.code(), Some(1), "{}", describe(args, &run));
    assert!(!out.exists(), "a refused run still wrote {}", out.display());
    let stderr = String::from_utf8_lossy(&run.stderr).to_lowercase();
    for needle in needles {
        assert!(
            stderr.contains(&needle.to_lowercase()),
            "stderr does not mention {needle:?}\n{}",
            describe(args, &run)
        );
    }
    eprintln!("viprs {} refused in {took:?}", args[0]);
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// [`decode_compare`] at `max`, and then the mean absolute difference over
/// every sample held to `mean`. For the 8-bit integer outputs the
/// BOUNDED-TOL cells write.
fn decode_compare_mean(actual: &Path, expected: &Path, max: f64, mean: f64) {
    decode_compare(actual, expected, max);
    let a = libviprs::decode_file(actual).expect("decode the output");
    let e = libviprs::decode_file(expected).expect("decode the reference");
    assert_eq!(
        a.format().bytes_per_pixel(),
        a.format().channels(),
        "the mean bound is for 8-bit samples, {} is {:?}",
        actual.display(),
        a.format()
    );
    let (da, de) = (a.data(), e.data());
    assert_eq!(da.len(), de.len());
    let sum: u64 = da
        .iter()
        .zip(de)
        .map(|(x, y)| u64::from(x.abs_diff(*y)))
        .sum();
    let got = sum as f64 / da.len() as f64;
    eprintln!("mean |d| {got:.5} for {}", actual.display());
    assert!(
        got <= mean,
        "mean |d| {got:.5} > bound {mean} for {} against {}",
        actual.display(),
        expected.display()
    );
}

// ---------------------------------------------------------------------------
// Container parsers, so an option cell looks at what the flag changes rather
// than at a file size that could move for other reasons.
// ---------------------------------------------------------------------------

/// `(Hi, Vi)` sampling factors of each component in a JPEG's SOF frame header.
fn jpeg_sampling(bytes: &[u8]) -> Vec<(u8, u8)> {
    let mut i = 2;
    while i + 4 <= bytes.len() {
        assert_eq!(bytes[i], 0xFF, "lost JPEG marker sync at {i}");
        let marker = bytes[i + 1];
        let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let seg = &bytes[i + 4..i + 2 + len];
            let n = usize::from(seg[5]);
            return (0..n)
                .map(|c| {
                    let hv = seg[6 + 3 * c + 1];
                    (hv >> 4, hv & 0x0F)
                })
                .collect();
        }
        i += 2 + len;
    }
    panic!("no SOF marker in the JPEG");
}

/// Each PNG chunk as `(type, data)`.
fn png_chunks(bytes: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    let mut out = Vec::new();
    let mut i = 8;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let ty: [u8; 4] = bytes[i + 4..i + 8].try_into().unwrap();
        out.push((ty, bytes[i + 8..i + 8 + len].to_vec()));
        i += 12 + len;
    }
    out
}

fn png_chunk(bytes: &[u8], ty: &[u8; 4]) -> Option<Vec<u8>> {
    png_chunks(bytes)
        .into_iter()
        .find(|(t, _)| t == ty)
        .map(|(_, d)| d)
}

/// The value of a TIFF tag in the first IFD, for the short/long tags this
/// suite reads.
fn tiff_tag(bytes: &[u8], tag: u16) -> Option<u32> {
    let le = match &bytes[..4] {
        b"II*\0" => true,
        b"MM\0*" => false,
        other => panic!("not a classic TIFF: {other:?}"),
    };
    let u16_at = |o: usize| {
        let b = [bytes[o], bytes[o + 1]];
        if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    };
    let u32_at = |o: usize| {
        let b = [bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]];
        if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    };
    let ifd = u32_at(4) as usize;
    for k in 0..usize::from(u16_at(ifd)) {
        let e = ifd + 2 + 12 * k;
        if u16_at(e) == tag {
            return Some(match u16_at(e + 2) {
                3 => u32::from(u16_at(e + 8)),
                _ => u32_at(e + 8),
            });
        }
    }
    None
}

/// GIF: `(global colour table entries, first image descriptor is interlaced)`.
fn gif_layout(bytes: &[u8]) -> (usize, bool) {
    assert!(bytes.starts_with(b"GIF8"), "not a GIF");
    let packed = bytes[10];
    let gct = if packed & 0x80 != 0 {
        1usize << ((packed & 7) + 1)
    } else {
        0
    };
    let mut i = 13 + 3 * gct;
    loop {
        match bytes[i] {
            0x21 => {
                i += 2;
                while bytes[i] != 0 {
                    i += usize::from(bytes[i]) + 1;
                }
                i += 1;
            }
            0x2C => return (gct, bytes[i + 9] & 0x40 != 0),
            other => panic!("unexpected GIF block 0x{other:02x} at {i}"),
        }
    }
}

/// `(XTsiz, YTsiz)` from a JPEG 2000 codestream's SIZ marker, found wherever
/// it sits (inside a JP2 `jp2c` box or at the front of a bare codestream).
#[cfg(feature = "jp2k")]
fn jp2k_tile_size(bytes: &[u8]) -> (u32, u32) {
    let at = bytes
        .windows(4)
        .position(|w| w == [0xFF, 0x4F, 0xFF, 0x51])
        .expect("no SOC+SIZ in the JPEG 2000 file")
        + 4;
    let u32_at = |o: usize| u32::from_be_bytes(bytes[o..o + 4].try_into().unwrap());
    // Lsiz(2) Rsiz(2) Xsiz Ysiz XOsiz YOsiz XTsiz YTsiz
    (u32_at(at + 20), u32_at(at + 24))
}

// ---------------------------------------------------------------------------
// Save: JPEG
// ---------------------------------------------------------------------------

#[test]
fn jpegsave_default_matches_vips() {
    if skip_if_no_cli("jpegsave_default_matches_vips") {
        return;
    }
    let out = out_path("jpegsave_default.jpg");
    ok(&["jpegsave", &canonical(), s(&out)]);
    decode_compare(
        &out,
        &cli_fixture("foreign/jpegsave_default.jpg"),
        JPEG_SAVE_TOL,
    );
}

#[test]
fn jpegsave_q95_matches_vips_and_changes_the_file() {
    if skip_if_no_cli("jpegsave_q95_matches_vips_and_changes_the_file") {
        return;
    }
    let q75 = out_path("jpegsave_q75_for_q.jpg");
    let q95 = out_path("jpegsave_q95.jpg");
    ok(&["jpegsave", &canonical(), s(&q75)]);
    ok(&["jpegsave", &canonical(), s(&q95), "--Q", "95"]);
    decode_compare(
        &q95,
        &cli_fixture("foreign/jpegsave_q95.jpg"),
        JPEG_SAVE_TOL,
    );
    assert!(
        read(&q95).len() > read(&q75).len(),
        "--Q 95 must spend more bytes than the default Q 75"
    );
}

#[test]
fn jpegsave_subsample_mode_off_writes_444() {
    if skip_if_no_cli("jpegsave_subsample_mode_off_writes_444") {
        return;
    }
    let auto = out_path("jpegsave_q50_auto.jpg");
    let off = out_path("jpegsave_q50_444.jpg");
    ok(&["jpegsave", &canonical(), s(&auto), "--Q", "50"]);
    ok(&[
        "jpegsave",
        &canonical(),
        s(&off),
        "--Q",
        "50",
        "--subsample-mode",
        "off",
    ]);
    // Below Q 90, auto subsamples: luma 2x2, chroma 1x1.
    assert_eq!(
        jpeg_sampling(&read(&auto))[0],
        (2, 2),
        "auto at Q 50 is 4:2:0"
    );
    assert!(
        jpeg_sampling(&read(&off)).iter().all(|&hv| hv == (1, 1)),
        "--subsample-mode off must write 4:4:4, got {:?}",
        jpeg_sampling(&read(&off))
    );
    assert_eq!(
        jpeg_sampling(&read(&off)),
        jpeg_sampling(&read(&cli_fixture("foreign/jpegsave_q50_444.jpg"))),
        "the sampling factors must be the ones vips wrote"
    );
    decode_compare(
        &off,
        &cli_fixture("foreign/jpegsave_q50_444.jpg"),
        JPEG_SAVE_TOL,
    );
}

// ---------------------------------------------------------------------------
// Save: PNG
// ---------------------------------------------------------------------------

#[test]
fn pngsave_default_matches_vips() {
    if skip_if_no_cli("pngsave_default_matches_vips") {
        return;
    }
    let out = out_path("pngsave_default.png");
    ok(&["pngsave", &canonical(), s(&out)]);
    decode_compare(&out, &cli_fixture("foreign/pngsave_default.png"), EXACT);
    assert_eq!(
        png_chunk(&read(&out), b"IHDR").unwrap()[12],
        0,
        "not interlaced"
    );
}

#[test]
fn pngsave_compression_changes_the_deflate_level() {
    if skip_if_no_cli("pngsave_compression_changes_the_deflate_level") {
        return;
    }
    let stored = out_path("pngsave_c0.png");
    let max = out_path("pngsave_c9.png");
    ok(&["pngsave", &canonical(), s(&stored), "--compression", "0"]);
    ok(&["pngsave", &canonical(), s(&max), "--compression", "9"]);
    decode_compare(&stored, &cli_fixture("foreign/pngsave_default.png"), EXACT);
    decode_compare(&max, &cli_fixture("foreign/pngsave_default.png"), EXACT);
    // Level 0 stores: the IDAT is at least the raw 256x256x4 samples.
    let idat = |p: &Path| -> usize {
        png_chunks(&read(p))
            .iter()
            .filter(|(t, _)| t == b"IDAT")
            .map(|(_, d)| d.len())
            .sum()
    };
    assert!(
        idat(&stored) >= 256 * 256 * 4,
        "--compression 0 must store, got {}",
        idat(&stored)
    );
    assert!(
        idat(&max) < idat(&stored) / 4,
        "--compression 9 must deflate"
    );
}

#[test]
fn pngsave_interlace_writes_adam7() {
    if skip_if_no_cli("pngsave_interlace_writes_adam7") {
        return;
    }
    let out = out_path("pngsave_interlace.png");
    ok(&["pngsave", &canonical(), s(&out), "--interlace"]);
    assert_eq!(
        png_chunk(&read(&out), b"IHDR").unwrap()[12],
        1,
        "IHDR interlace method"
    );
    decode_compare(&out, &cli_fixture("foreign/pngsave_interlace.png"), EXACT);
}

#[test]
fn pngsave_palette_writes_an_indexed_png() {
    if skip_if_no_cli("pngsave_palette_writes_an_indexed_png") {
        return;
    }
    let out = out_path("pngsave_palette.png");
    ok(&["pngsave", &canonical(), s(&out), "--palette"]);
    let bytes = read(&out);
    assert_eq!(
        png_chunk(&bytes, b"IHDR").unwrap()[9],
        3,
        "IHDR colour type 3 (indexed)"
    );
    assert!(png_chunk(&bytes, b"PLTE").unwrap().len() <= 256 * 3);
    decode_compare_mean(
        &out,
        &cli_fixture("foreign/pngsave_palette.png"),
        PNG_PALETTE_TOL,
        PNG_PALETTE_MEAN,
    );
}

#[test]
fn pngsave_palette_bitdepth_caps_the_palette() {
    if skip_if_no_cli("pngsave_palette_bitdepth_caps_the_palette") {
        return;
    }
    let out = out_path("pngsave_palette_bd2.png");
    ok(&[
        "pngsave",
        &canonical(),
        s(&out),
        "--palette",
        "--bitdepth",
        "2",
    ]);
    let entries = png_chunk(&read(&out), b"PLTE").unwrap().len() / 3;
    let vips = png_chunk(
        &read(&cli_fixture("foreign/pngsave_palette_bd2.png")),
        b"PLTE",
    )
    .unwrap()
    .len()
        / 3;
    assert!(
        entries <= 4,
        "--bitdepth 2 allows 4 colours, the palette has {entries}"
    );
    assert!(
        vips <= 4,
        "the vips reference should hold 4 colours too, holds {vips}"
    );
}

// ---------------------------------------------------------------------------
// Save: TIFF
// ---------------------------------------------------------------------------

fn tiffsave_case(test: &str, compression: Option<&str>, tag: u32) {
    if skip_if_no_cli(test) {
        return;
    }
    let out = out_path(&format!("{test}.tif"));
    let input = canonical();
    let mut args = vec!["tiffsave", input.as_str(), s(&out)];
    if let Some(c) = compression {
        args.extend(["--compression", c]);
    }
    ok(&args);
    assert_eq!(tiff_tag(&read(&out), 259), Some(tag), "Compression (259)");
    decode_compare(&out, &cli_fixture("foreign/tiffsave_deflate.tif"), EXACT);
}

#[test]
fn tiffsave_default_is_uncompressed() {
    tiffsave_case("tiffsave_default_is_uncompressed", None, 1);
}

#[test]
fn tiffsave_compression_lzw() {
    tiffsave_case("tiffsave_compression_lzw", Some("lzw"), 5);
}

#[test]
fn tiffsave_compression_deflate() {
    tiffsave_case("tiffsave_compression_deflate", Some("deflate"), 8);
}

// ---------------------------------------------------------------------------
// Save: WebP, GIF
// ---------------------------------------------------------------------------

#[test]
fn webpsave_lossless_matches_vips() {
    if skip_if_no_cli("webpsave_lossless_matches_vips") {
        return;
    }
    let out = out_path("webpsave.webp");
    ok(&["webpsave", &canonical(), s(&out), "--lossless"]);
    // Against the input rather than vips's file: vips drops an opaque alpha
    // when it writes WebP (its reference decodes to three bands) and the core
    // keeps it, and the pixels are what lossless is a promise about.
    decode_compare(&out, Path::new(&canonical()), EXACT);
}

/// Leaving `--lossless` off is a usage mistake (vips's default is lossy, and
/// this build has no lossy encoder), so it is clap's required-argument error:
/// exit 2, with the flag in the usage line.
#[test]
fn webpsave_without_lossless_is_a_usage_error() {
    if skip_if_no_cli("webpsave_without_lossless_is_a_usage_error") {
        return;
    }
    let out = out_path("webpsave_lossy.webp");
    usage_error(
        &["webpsave", &canonical(), s(&out)],
        &out,
        &["--lossless", "Usage: viprs webpsave --lossless"],
    );
}

#[test]
fn gifsave_default_matches_vips() {
    if skip_if_no_cli("gifsave_default_matches_vips") {
        return;
    }
    let out = out_path("gifsave_default.gif");
    ok(&["gifsave", &canonical(), s(&out)]);
    decode_compare_mean(
        &out,
        &cli_fixture("foreign/gifsave_default.gif"),
        GIF_SAVE_TOL,
        GIF_SAVE_MEAN,
    );
    assert!(!gif_layout(&read(&out)).1, "not interlaced by default");
}

#[test]
fn gifsave_bitdepth_caps_the_colour_table() {
    if skip_if_no_cli("gifsave_bitdepth_caps_the_colour_table") {
        return;
    }
    let out = out_path("gifsave_bd2.gif");
    ok(&["gifsave", &canonical(), s(&out), "--bitdepth", "2"]);
    let (entries, _) = gif_layout(&read(&out));
    assert!(
        entries <= 4,
        "--bitdepth 2 allows 4 colours, the table has {entries}"
    );
    // No pixel compare: two 4-colour quantisations of these gradients differ
    // by up to 206 per sample (measured), which bounds nothing. vips's own
    // table is the structural oracle.
    let (vips, _) = gif_layout(&read(&cli_fixture("foreign/gifsave_bd2.gif")));
    assert!(vips <= 4, "the vips reference holds {vips} entries");
}

#[test]
fn gifsave_dither_zero_changes_the_pixels() {
    if skip_if_no_cli("gifsave_dither_zero_changes_the_pixels") {
        return;
    }
    let dithered = out_path("gifsave_dither1.gif");
    let flat = out_path("gifsave_dither0.gif");
    ok(&["gifsave", &canonical(), s(&dithered)]);
    ok(&["gifsave", &canonical(), s(&flat), "--dither", "0"]);
    assert_ne!(
        read(&flat),
        read(&dithered),
        "--dither 0 must change the output"
    );
    decode_compare_mean(
        &flat,
        &cli_fixture("foreign/gifsave_dither0.gif"),
        GIF_NODITHER_TOL,
        GIF_NODITHER_MEAN,
    );
}

#[test]
fn gifsave_interlace_sets_the_descriptor_flag() {
    if skip_if_no_cli("gifsave_interlace_sets_the_descriptor_flag") {
        return;
    }
    let out = out_path("gifsave_interlace.gif");
    ok(&["gifsave", &canonical(), s(&out), "--interlace"]);
    assert!(gif_layout(&read(&out)).1, "image descriptor interlace bit");
    decode_compare_mean(
        &out,
        &cli_fixture("foreign/gifsave_default.gif"),
        GIF_SAVE_TOL,
        GIF_SAVE_MEAN,
    );
}

// ---------------------------------------------------------------------------
// Save: FITS, Radiance, Ultra HDR, CSV, matrix, PPM
// ---------------------------------------------------------------------------

#[test]
fn fitssave_matches_vips() {
    if skip_if_no_cli("fitssave_matches_vips") {
        return;
    }
    let out = out_path("fitssave.fits");
    ok(&["fitssave", &fx("foreign/rgb32.png"), s(&out)]);
    // FITS is uncompressed and fixed-format, and the core writes vips's bytes.
    byte_compare(&out, &cli_fixture("foreign/fitssave.fits"));
}

#[test]
fn radsave_matches_vips() {
    if skip_if_no_cli("radsave_matches_vips") {
        return;
    }
    let out = out_path("radsave.hdr");
    ok(&["radsave", &fx("foreign/float32.v"), s(&out)]);
    decode_compare(&out, &cli_fixture("foreign/radsave.hdr"), EXACT);
}

#[test]
fn uhdrsave_base_is_within_tolerance_of_vips() {
    if skip_if_no_cli("uhdrsave_base_is_within_tolerance_of_vips") {
        return;
    }
    let out = out_path("uhdrsave.jpg");
    ok(&["uhdrsave", &fx("foreign/scrgb32.v"), s(&out)]);
    assert!(
        libviprs::uhdr::is_uhdr(&read(&out)),
        "not an Ultra HDR container"
    );
    decode_compare_mean(
        &out,
        &cli_fixture("foreign/uhdrsave.jpg"),
        UHDR_SAVE_TOL,
        UHDR_SAVE_MEAN,
    );
}

#[test]
fn uhdrsave_gainmap_scale_factor_sizes_the_gain_map() {
    if skip_if_no_cli("uhdrsave_gainmap_scale_factor_sizes_the_gain_map") {
        return;
    }
    let factor = |args: &[&str], name: &str| -> String {
        let out = out_path(name);
        let mut full = vec!["uhdrsave"];
        let input = fx("foreign/scrgb32.v");
        full.push(&input);
        full.push(s(&out));
        full.extend_from_slice(args);
        ok(&full);
        let r = libviprs::uhdr::decode_uhdr(&read(&out), libviprs::source::DecodeLimits::default())
            .expect("the output decodes as Ultra HDR");
        format!(
            "{:?}",
            r.get_field("gainmap-scale-factor")
                .expect("scale factor field")
        )
    };
    let default = factor(&[], "uhdrsave_gm_default.jpg");
    let one = factor(&["--gainmap-scale-factor", "1"], "uhdrsave_gm1.jpg");
    assert_ne!(
        default, one,
        "--gainmap-scale-factor 1 must change the gain map size"
    );
    assert!(
        one.contains('1'),
        "full-size gain map reports scale factor 1, got {one}"
    );
}

#[test]
fn uhdrsave_q_changes_the_file() {
    if skip_if_no_cli("uhdrsave_q_changes_the_file") {
        return;
    }
    let q75 = out_path("uhdrsave_q75.jpg");
    let q95 = out_path("uhdrsave_q95.jpg");
    ok(&["uhdrsave", &fx("foreign/scrgb32.v"), s(&q75)]);
    ok(&["uhdrsave", &fx("foreign/scrgb32.v"), s(&q95), "--Q", "95"]);
    assert!(
        read(&q95).len() > read(&q75).len(),
        "--Q 95 must spend more bytes"
    );
}

#[test]
fn csvsave_matches_vips() {
    if skip_if_no_cli("csvsave_matches_vips") {
        return;
    }
    let out = out_path("csvsave.csv");
    ok(&["csvsave", &fx("foreign/gray32.png"), s(&out)]);
    byte_compare(&out, &cli_fixture("foreign/csvsave.csv"));
}

#[test]
fn matrixsave_matches_vips() {
    if skip_if_no_cli("matrixsave_matches_vips") {
        return;
    }
    let out = out_path("matrixsave.mat");
    ok(&["matrixsave", &fx("foreign/gray32.png"), s(&out)]);
    byte_compare(&out, &cli_fixture("foreign/matrixsave.mat"));
}

#[test]
fn ppmsave_matches_vips_byte_for_byte() {
    if skip_if_no_cli("ppmsave_matches_vips_byte_for_byte") {
        return;
    }
    let out = out_path("ppmsave.ppm");
    ok(&["ppmsave", &fx("foreign/rgb32.png"), s(&out)]);
    byte_compare(&out, &cli_fixture("foreign/ppmsave.ppm"));
}

// ---------------------------------------------------------------------------
// Save by extension, through the op harness (`viprs copy IN OUT.<ext>`)
// ---------------------------------------------------------------------------

fn copy_to(test: &str, input: &str, ext: &str, reference: &str, tol: f64) {
    copy_to_mean(test, input, ext, reference, tol, f64::INFINITY);
}

fn copy_to_mean(test: &str, input: &str, ext: &str, reference: &str, tol: f64, mean: f64) {
    if skip_if_no_cli(test) {
        return;
    }
    let out = out_path(&format!("{test}.{ext}"));
    ok(&["copy", input, s(&out)]);
    if mean.is_finite() {
        decode_compare_mean(&out, &cli_fixture(reference), tol, mean);
    } else {
        decode_compare(&out, &cli_fixture(reference), tol);
    }
}

#[test]
fn copy_to_webp_routes_to_the_webp_encoder() {
    if skip_if_no_cli("copy_to_webp") {
        return;
    }
    let out = out_path("copy_to.webp");
    ok(&["copy", &canonical(), s(&out)]);
    decode_compare(&out, Path::new(&canonical()), EXACT);
}

#[test]
fn copy_to_gif_routes_to_the_gif_encoder() {
    copy_to_mean(
        "copy_to_gif",
        &canonical(),
        "gif",
        "foreign/gifsave_default.gif",
        GIF_SAVE_TOL,
        GIF_SAVE_MEAN,
    );
}

#[test]
fn copy_to_fits_routes_to_the_fits_encoder() {
    copy_to(
        "copy_to_fits",
        &fx("foreign/rgb32.png"),
        "fits",
        "foreign/fitssave.fits",
        EXACT,
    );
}

#[test]
fn copy_to_hdr_routes_to_the_radiance_encoder() {
    copy_to(
        "copy_to_hdr",
        &fx("foreign/float32.v"),
        "hdr",
        "foreign/radsave.hdr",
        EXACT,
    );
}

#[test]
fn copy_to_csv_and_mat_route_to_the_text_encoders() {
    if skip_if_no_cli("copy_to_csv_and_mat_route_to_the_text_encoders") {
        return;
    }
    let csv = out_path("copy_to.csv");
    let mat = out_path("copy_to.mat");
    ok(&["copy", &fx("foreign/gray32.png"), s(&csv)]);
    ok(&["copy", &fx("foreign/gray32.png"), s(&mat)]);
    byte_compare(&csv, &cli_fixture("foreign/csvsave.csv"));
    byte_compare(&mat, &cli_fixture("foreign/matrixsave.mat"));
}

#[test]
fn copy_to_jpg_stays_banned_in_the_op_harness() {
    if skip_if_no_cli("copy_to_jpg_stays_banned_in_the_op_harness") {
        return;
    }
    let out = out_path("copy_to.jpg");
    refused(&["copy", &canonical(), s(&out)], &out, &["banned"]);
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

fn load_case(test: &str, command: &str, input: &str, reference: &str, extra: &[&str], tol: f64) {
    if skip_if_no_cli(test) {
        return;
    }
    let ext = Path::new(reference).extension().unwrap().to_str().unwrap();
    let out = out_path(&format!("{test}.{ext}"));
    let mut args = vec![command, input, s(&out)];
    args.extend_from_slice(extra);
    ok(&args);
    decode_compare(&out, &cli_fixture(reference), tol);
}

#[test]
fn jpegload_matches_vips() {
    load_case(
        "jpegload",
        "jpegload",
        &fx("foreign/jpegsave_default.jpg"),
        "foreign/jpegload_expected.png",
        &[],
        JPEG_LOAD_TOL,
    );
}

#[test]
fn jpegload_shrink_halves_the_image() {
    if skip_if_no_cli("jpegload_shrink2") {
        return;
    }
    let out = out_path("jpegload_shrink2.png");
    ok(&[
        "jpegload",
        &fx("foreign/jpegsave_default.jpg"),
        s(&out),
        "--shrink",
        "2",
    ]);
    decode_compare_mean(
        &out,
        &cli_fixture("foreign/jpegload_shrink2_expected.png"),
        JPEG_SHRINK_TOL,
        JPEG_SHRINK_MEAN,
    );
}

#[test]
fn pngload_matches_vips() {
    load_case(
        "pngload",
        "pngload",
        &fx("foreign/pngsave_default.png"),
        "foreign/pngload_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn pngload_reads_stdin() {
    if skip_if_no_cli("pngload_reads_stdin") {
        return;
    }
    let out = out_path("pngload_stdin.png");
    let mut child = Command::new(viprs_bin())
        .args(["pngload", "-", s(&out)])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn viprs");
    // A viprs that exits before reading stdin closes the pipe; the exit
    // status below is the assertion that says why, not this write.
    let _ = child
        .stdin
        .take()
        .unwrap()
        .write_all(&read(&cli_fixture("foreign/pngsave_default.png")));
    let run = child.wait_with_output().unwrap();
    assert!(
        run.status.success(),
        "{}",
        describe(&["pngload", "-"], &run)
    );
    decode_compare(&out, &cli_fixture("foreign/pngload_expected.png"), EXACT);
}

#[test]
fn pngload_refuses_a_jpeg() {
    if skip_if_no_cli("pngload_refuses_a_jpeg") {
        return;
    }
    let out = out_path("pngload_of_jpeg.png");
    refused(
        &["pngload", &fx("foreign/jpegsave_default.jpg"), s(&out)],
        &out,
        &["not a PNG"],
    );
}

#[test]
fn tiffload_reads_the_first_page_by_default() {
    load_case(
        "tiffload_p0",
        "tiffload",
        &fx("foreign/pages.tif"),
        "foreign/tiffload_p0_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn tiffload_page_selects_a_page() {
    load_case(
        "tiffload_p2",
        "tiffload",
        &fx("foreign/pages.tif"),
        "foreign/tiffload_p2_expected.png",
        &["--page", "2"],
        EXACT,
    );
}

#[test]
fn tiffload_carries_the_page_count() {
    if skip_if_no_cli("tiffload_carries_the_page_count") {
        return;
    }
    let out = out_path("tiffload_pages.v");
    ok(&["tiffload", &fx("foreign/pages.tif"), s(&out), "--page", "1"]);
    let r = libviprs::decode_file(&out).expect("decode the .v");
    assert_eq!(
        r.get_n_pages(),
        3,
        "vipsheader -f n-pages says 3 for pages.tif"
    );
}

#[test]
fn tiffload_page_past_the_end_is_refused() {
    if skip_if_no_cli("tiffload_page_past_the_end_is_refused") {
        return;
    }
    let out = out_path("tiffload_p3.png");
    refused(
        &["tiffload", &fx("foreign/pages.tif"), s(&out), "--page", "3"],
        &out,
        &["page 3", "3"],
    );
}

#[test]
fn tiffload_max_pages_is_honoured() {
    if skip_if_no_cli("tiffload_max_pages_is_honoured") {
        return;
    }
    let out = out_path("tiffload_maxpages.png");
    refused(
        &[
            "tiffload",
            &fx("foreign/pages.tif"),
            s(&out),
            "--max-pages",
            "2",
        ],
        &out,
        // Not "pages": the input is pages.tif / pages.gif, so that needle
        // matched the file name in any error at all.
        &["more than 2 pages", "max_pages"],
    );
}

#[test]
fn gifload_page_selects_a_frame() {
    load_case(
        "gifload_p1",
        "gifload",
        &fx("foreign/pages.gif"),
        "foreign/gifload_p1_expected.png",
        &["--page", "1"],
        EXACT,
    );
}

#[test]
fn gifload_n_minus_one_loads_every_frame() {
    load_case(
        "gifload_all",
        "gifload",
        &fx("foreign/pages.gif"),
        "foreign/gifload_all_expected.png",
        &["--n", "-1"],
        EXACT,
    );
}

#[test]
fn gifload_max_pages_is_honoured() {
    if skip_if_no_cli("gifload_max_pages_is_honoured") {
        return;
    }
    let out = out_path("gifload_maxpages.png");
    refused(
        &[
            "gifload",
            &fx("foreign/pages.gif"),
            s(&out),
            "--n",
            "-1",
            "--max-pages",
            "2",
        ],
        &out,
        // Not "pages": the input is pages.tif / pages.gif, so that needle
        // matched the file name in any error at all.
        &["more than 2 pages", "max_pages"],
    );
}

#[test]
fn webpload_matches_vips() {
    load_case(
        "webpload",
        "webpload",
        &fx("foreign/webpsave_lossless.webp"),
        "foreign/webpload_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn webpload_page_selects_a_frame() {
    load_case(
        "webpload_p1",
        "webpload",
        &fx("foreign/pages.webp"),
        "foreign/webpload_p1_expected.png",
        &["--page", "1"],
        EXACT,
    );
}

#[test]
fn webpload_n_minus_one_loads_every_frame() {
    load_case(
        "webpload_all",
        "webpload",
        &fx("foreign/pages.webp"),
        "foreign/webpload_all_expected.png",
        &["--n", "-1"],
        EXACT,
    );
}

#[test]
fn fitsload_matches_vips() {
    load_case(
        "fitsload",
        "fitsload",
        &fx("foreign/fitssave.fits"),
        "foreign/fitsload_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn radload_matches_vips() {
    load_case(
        "radload",
        "radload",
        &fx("foreign/radsave.hdr"),
        "foreign/radload_expected.v",
        &[],
        EXACT,
    );
}

#[test]
fn csvload_matches_vips() {
    load_case(
        "csvload",
        "csvload",
        &fx("foreign/csvsave.csv"),
        "foreign/csvload_expected.v",
        &[],
        EXACT,
    );
}

#[test]
fn matrixload_matches_vips() {
    load_case(
        "matrixload",
        "matrixload",
        &fx("foreign/matrixsave.mat"),
        "foreign/matrixload_expected.v",
        &[],
        EXACT,
    );
}

#[test]
fn ppmload_matches_vips() {
    load_case(
        "ppmload",
        "ppmload",
        &fx("foreign/ppmsave.ppm"),
        "foreign/ppmload_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn openexrload_matches_vips() {
    load_case(
        "openexrload",
        "openexrload",
        &fx("foreign/openexr_rgba_half.exr"),
        "foreign/openexrload_expected.v",
        &[],
        EXACT,
    );
}

#[test]
fn niftiload_matches_nifti_clib() {
    load_case(
        "niftiload",
        "niftiload",
        &fx("foreign/nifti_uint8.nii"),
        "foreign/niftiload_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn analyzeload_matches_vips() {
    load_case(
        "analyzeload",
        "analyzeload",
        &fx("foreign/analyze_uchar.hdr"),
        "foreign/analyzeload_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn matload_matches_vips() {
    load_case(
        "matload",
        "matload",
        &fx("foreign/matlab_uint8.mat"),
        "foreign/matload_expected.png",
        &[],
        EXACT,
    );
}

#[test]
fn uhdrload_matches_vips() {
    load_case(
        "uhdrload",
        "uhdrload",
        &fx("foreign/uhdr_source.jpg"),
        "foreign/uhdrload_expected.png",
        &[],
        UHDR_LOAD_TOL,
    );
}

// ---------------------------------------------------------------------------
// Decode limits: one `--max-pixels 1` refusal per loader. `max_pixels` is the
// ceiling the core honours on every decode path (`DecodeLimits` docs), so it
// is the one flag every loader can be held to before it allocates a frame.
// ---------------------------------------------------------------------------

fn limit_case(bin: &Path, command: &str, input: &str) {
    let out = out_path(&format!("{command}_limited.png"));
    refused_with(
        bin,
        &[command, input, s(&out), "--max-pixels", "1"],
        &out,
        &["exceed"],
    );
}

macro_rules! limit_cells {
    ($($name:ident: $command:literal, $input:literal;)*) => {$(
        #[test]
        fn $name() {
            if skip_if_no_cli(stringify!($name)) {
                return;
            }
            limit_case(&viprs_bin(), $command, &fx($input));
        }
    )*};
}

limit_cells! {
    jpegload_honours_max_pixels: "jpegload", "foreign/jpegsave_default.jpg";
    pngload_honours_max_pixels: "pngload", "foreign/pngsave_default.png";
    tiffload_honours_max_pixels: "tiffload", "foreign/pages.tif";
    gifload_honours_max_pixels: "gifload", "foreign/pages.gif";
    webpload_honours_max_pixels: "webpload", "foreign/pages.webp";
    fitsload_honours_max_pixels: "fitsload", "foreign/fitssave.fits";
    radload_honours_max_pixels: "radload", "foreign/radsave.hdr";
    uhdrload_honours_max_pixels: "uhdrload", "foreign/uhdr_source.jpg";
    csvload_honours_max_pixels: "csvload", "foreign/csvsave.csv";
    matrixload_honours_max_pixels: "matrixload", "foreign/matrixsave.mat";
    ppmload_honours_max_pixels: "ppmload", "foreign/ppmsave.ppm";
    openexrload_honours_max_pixels: "openexrload", "foreign/openexr_rgba_half.exr";
    niftiload_honours_max_pixels: "niftiload", "foreign/nifti_uint8.nii";
    analyzeload_honours_max_pixels: "analyzeload", "foreign/analyze_uchar.hdr";
    matload_honours_max_pixels: "matload", "foreign/matlab_uint8.mat";
}

#[test]
fn a_saver_honours_the_decode_limits_on_its_input() {
    if skip_if_no_cli("a_saver_honours_the_decode_limits_on_its_input") {
        return;
    }
    let out = out_path("pngsave_limited.png");
    refused(
        &["pngsave", &canonical(), s(&out), "--max-pixels", "1"],
        &out,
        &["exceed"],
    );
}

// ---------------------------------------------------------------------------
// The gated codecs, bare build: each command refuses naming its feature.
// ---------------------------------------------------------------------------

#[test]
fn a_bare_viprs_refuses_every_gated_codec_naming_its_feature() {
    // Built bare on purpose rather than taken from `viprs_bin()`, which
    // `$VIPRS_BIN` can point at a build that has every feature.
    if !cli_source_available() {
        eprintln!("SKIP a_bare_viprs_refuses_every_gated_codec_naming_its_feature: no CLI source.");
        return;
    }
    let bare = viprs_bin_for(false, &[]);
    let canonical = canonical();
    let jxl = fx("features/canonical.jxl");
    let jp2 = fx("features/canonical.jp2");
    let avif = fx("features/canonical.avif");
    let svg = fx("features/canonical.svg");
    let cases: [(&str, &str, &str, &[&str]); 6] = [
        ("jxlsave", &canonical, "jxl", &["--lossless"]),
        ("jp2ksave", &canonical, "jp2k", &["--lossless"]),
        ("jxlload", &jxl, "jxl", &[]),
        ("jp2kload", &jp2, "jp2k", &[]),
        ("heifload", &avif, "avif", &[]),
        ("svgload", &svg, "svg", &[]),
    ];
    for (command, input, feature, extra) in cases {
        let out = out_path(&format!("bare_{command}.png"));
        let mut args = vec![command, input, s(&out)];
        args.extend_from_slice(extra);
        refused_with(
            &bare,
            &args,
            &out,
            &[&format!("`{feature}`"), &format!("--features {feature}")],
        );
    }
}

// ---------------------------------------------------------------------------
// The gated codecs, with their feature.
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(
    not(feature = "jxl"),
    ignore = "JPEG XL cell: run with `cargo test --features jxl --test cli_foreign_diff`"
)]
fn jxl_save_and_load_match_vips() {
    #[cfg(feature = "jxl")]
    {
        let Some(bin) = gated_bin("jxl_save_and_load_match_vips", "jxl") else {
            return;
        };
        let saved = out_path("jxlsave.jxl");
        ok_with(&bin, &["jxlsave", &canonical(), s(&saved), "--lossless"]);
        decode_compare(&saved, &cli_fixture("foreign/jxlsave_lossless.jxl"), EXACT);
        let refused_out = out_path("jxlsave_lossy.jxl");
        usage_error_with(
            &bin,
            &["jxlsave", &canonical(), s(&refused_out)],
            &refused_out,
            &["--lossless"],
        );
        let loaded = out_path("jxlload.png");
        ok_with(
            &bin,
            &["jxlload", &fx("features/canonical.jxl"), s(&loaded)],
        );
        decode_compare(&loaded, &cli_fixture("foreign/jxlload_expected.png"), EXACT);
        limit_case(&bin, "jxlload", &fx("features/canonical.jxl"));
        let routed = out_path("copy_to.jxl");
        ok_with(&bin, &["copy", &canonical(), s(&routed)]);
        decode_compare(&routed, &cli_fixture("foreign/jxlsave_lossless.jxl"), EXACT);
    }
}

#[test]
#[cfg_attr(
    not(feature = "jp2k"),
    ignore = "JPEG 2000 cell: run with `cargo test --features jp2k --test cli_foreign_diff`"
)]
fn jp2k_save_and_load_match_vips() {
    #[cfg(feature = "jp2k")]
    {
        let Some(bin) = gated_bin("jp2k_save_and_load_match_vips", "jp2k") else {
            return;
        };
        let saved = out_path("jp2ksave.jp2");
        ok_with(&bin, &["jp2ksave", &canonical(), s(&saved), "--lossless"]);
        // The core's JPEG 2000 encoder writes the codestream OpenJPEG does,
        // so this is a byte compare, not a decode compare.
        byte_compare(&saved, &cli_fixture("foreign/jp2ksave_lossless.jp2"));
        assert_eq!(
            jp2k_tile_size(&read(&saved)),
            jp2k_tile_size(&read(&cli_fixture("foreign/jp2ksave_lossless.jp2"))),
            "default tile size"
        );
        let refused_out = out_path("jp2ksave_lossy.jp2");
        usage_error_with(
            &bin,
            &["jp2ksave", &canonical(), s(&refused_out)],
            &refused_out,
            &["--lossless"],
        );
        let loaded = out_path("jp2kload.png");
        ok_with(
            &bin,
            &["jp2kload", &fx("features/canonical.jp2"), s(&loaded)],
        );
        decode_compare(
            &loaded,
            &cli_fixture("foreign/jp2kload_expected.png"),
            EXACT,
        );
        limit_case(&bin, "jp2kload", &fx("features/canonical.jp2"));
        let routed = out_path("copy_to.jp2");
        ok_with(&bin, &["copy", &canonical(), s(&routed)]);
        decode_compare(
            &routed,
            &cli_fixture("foreign/jp2ksave_lossless.jp2"),
            EXACT,
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "jp2k"),
    ignore = "JPEG 2000 cell: run with `cargo test --features jp2k --test cli_foreign_diff`"
)]
fn jp2ksave_tile_size_matches_vips() {
    #[cfg(feature = "jp2k")]
    {
        let Some(bin) = gated_bin("jp2ksave_tile_size_matches_vips", "jp2k") else {
            return;
        };
        let out = out_path("jp2ksave_tile64x128.jp2");
        ok_with(
            &bin,
            &[
                "jp2ksave",
                &canonical(),
                s(&out),
                "--lossless",
                "--tile-width",
                "64",
                "--tile-height",
                "128",
            ],
        );
        assert_eq!(jp2k_tile_size(&read(&out)), (64, 128), "SIZ XTsiz/YTsiz");
        assert_eq!(
            jp2k_tile_size(&read(&out)),
            jp2k_tile_size(&read(&cli_fixture("foreign/jp2ksave_tile64x128.jp2")))
        );
        byte_compare(&out, &cli_fixture("foreign/jp2ksave_tile64x128.jp2"));
    }
}

#[test]
#[cfg_attr(
    not(feature = "avif"),
    ignore = "AVIF cell: run with `cargo test --features avif --test cli_foreign_diff`"
)]
fn heifload_reads_avif_and_refuses_the_rest() {
    #[cfg(feature = "avif")]
    {
        let Some(bin) = gated_bin("heifload_reads_avif_and_refuses_the_rest", "avif") else {
            return;
        };
        let loaded = out_path("heifload.png");
        ok_with(
            &bin,
            &["heifload", &fx("features/canonical.avif"), s(&loaded)],
        );
        decode_compare(
            &loaded,
            &cli_fixture("foreign/heifload_expected.png"),
            EXACT,
        );
        limit_case(&bin, "heifload", &fx("features/canonical.avif"));
        let not_avif = out_path("heifload_of_png.png");
        refused_with(
            &bin,
            &["heifload", &canonical(), s(&not_avif)],
            &not_avif,
            &["not an AVIF"],
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "svg"),
    ignore = "SVG cell: run with `cargo test --features svg --test cli_foreign_diff`"
)]
fn svgload_scale_and_dpi_match_vips() {
    #[cfg(feature = "svg")]
    {
        let Some(bin) = gated_bin("svgload_scale_and_dpi_match_vips", "svg") else {
            return;
        };
        let scaled = out_path("svgload_scale2.png");
        ok_with(
            &bin,
            &[
                "svgload",
                &fx("features/canonical.svg"),
                s(&scaled),
                "--scale",
                "2",
            ],
        );
        decode_compare(
            &scaled,
            &cli_fixture("foreign/svgload_scale2_expected.png"),
            EXACT,
        );
        let dpi = out_path("svgload_dpi144.png");
        ok_with(
            &bin,
            &[
                "svgload",
                &fx("features/canonical.svg"),
                s(&dpi),
                "--dpi",
                "144",
            ],
        );
        decode_compare(
            &dpi,
            &cli_fixture("foreign/svgload_dpi144_expected.png"),
            EXACT,
        );
        limit_case(&bin, "svgload", &fx("features/canonical.svg"));
    }
}

#[test]
#[cfg_attr(
    not(feature = "svg"),
    ignore = "SVG cell: run with `cargo test --features svg --test cli_foreign_diff`"
)]
fn svgload_unlimited_lifts_the_input_ceiling() {
    #[cfg(feature = "svg")]
    {
        let Some(bin) = gated_bin("svgload_unlimited_lifts_the_input_ceiling", "svg") else {
            return;
        };
        // One byte over the core's 10 MB ceiling, padded with a comment so the
        // document is still a 2x2 red square.
        let big = out_path("big.svg");
        let head = b"<svg xmlns='http://www.w3.org/2000/svg' width='2' height='2'><rect width='2' height='2' fill='red'/><!--";
        let tail = b"--></svg>";
        let pad = libviprs::svg::MAX_INPUT_BYTES + 1 - head.len() - tail.len();
        let mut doc = head.to_vec();
        doc.resize(head.len() + pad, b' ');
        doc.extend_from_slice(tail);
        std::fs::write(&big, &doc).unwrap();
        let out = out_path("big_svg.png");
        // Not "svg": the input is big.svg, so that matched any error at all.
        refused_with(
            &bin,
            &["svgload", s(&big), s(&out)],
            &out,
            &["byte ceiling", "unlimited"],
        );
        ok_with(&bin, &["svgload", s(&big), s(&out), "--unlimited"]);
        let r = libviprs::decode_file(&out).unwrap();
        assert_eq!((r.width(), r.height()), (2, 2));
    }
}

// ---------------------------------------------------------------------------
// Inputs whose header lies about their size. The honest 256x256 files above,
// with a ceiling one below, cannot tell "refused before allocating" from
// "decoded, then refused": a decompression bomb passes both. These declare
// gigabytes in a few bytes, run under a wall-clock deadline and an
// address-space cap, and have to be refused from the header.
// ---------------------------------------------------------------------------

/// A PNG chunk with its CRC, so the decoder reads the header for real rather
/// than refusing a corrupt file before it gets to the size.
fn png_chunk_bytes(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let mut covered = ty.to_vec();
    covered.extend_from_slice(data);
    out.extend_from_slice(&crc32(&covered).to_be_bytes());
    out
}

#[test]
fn pngload_refuses_an_ihdr_claiming_65535_square_from_the_header() {
    if skip_if_no_cli("pngload_refuses_an_ihdr_claiming_65535_square_from_the_header") {
        return;
    }
    // 65535 x 65535 RGB8 is 12.9 GB of pixels; the body is one stored
    // deflate block of nothing.
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&65535u32.to_be_bytes());
    ihdr.extend_from_slice(&65535u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(png_chunk_bytes(b"IHDR", &ihdr));
    png.extend(png_chunk_bytes(
        b"IDAT",
        &[0x78, 0x01, 0x01, 0x00, 0x00, 0xFF, 0xFF, 0, 0, 0, 1],
    ));
    png.extend(png_chunk_bytes(b"IEND", &[]));
    let input = out_path("lies_65535.png");
    std::fs::write(&input, &png).unwrap();
    let out = out_path("lies_65535_out.png");
    refused_fast(
        &["pngload", s(&input), s(&out)],
        &out,
        &["65535x65535", "ceiling"],
    );
}

#[test]
fn ppmload_refuses_a_p6_claiming_100000_square_from_the_header() {
    if skip_if_no_cli("ppmload_refuses_a_p6_claiming_100000_square_from_the_header") {
        return;
    }
    let mut ppm = b"P6\n100000 100000\n255\n".to_vec();
    ppm.extend_from_slice(&[0u8; 64]);
    let input = out_path("lies_100000.ppm");
    std::fs::write(&input, &ppm).unwrap();
    let out = out_path("lies_100000_out.png");
    refused_fast(
        &["ppmload", s(&input), s(&out)],
        &out,
        &["100000x100000", "allocation ceiling"],
    );
}

/// The ragged CSV from the review, in the one shape that slips past every
/// geometry default: one 65535-field row, then 16383 one-field rows. The
/// core pads every row to the first one's width, so 160 KB of text becomes
/// 65535 x 16384 floats, about 4.3 GB a copy, while 65535 x 16384 stays
/// under the 2^30 `--max-pixels` default. (16384 one-field rows would make
/// 16385 rows, which `--max-pixels` already refuses.) The refusal has to come
/// from pricing the grid against `--max-alloc-bytes`.
#[test]
fn csvload_refuses_a_ragged_grid_that_pads_to_gigabytes() {
    if skip_if_no_cli("csvload_refuses_a_ragged_grid_that_pads_to_gigabytes") {
        return;
    }
    let mut csv = vec!["0"; 65535].join(",");
    csv.push_str(&"\n1".repeat(16383));
    csv.push('\n');
    let input = out_path("ragged_bomb.csv");
    std::fs::write(&input, csv).unwrap();
    let out = out_path("ragged_bomb_out.v");
    refused_fast(
        &["csvload", s(&input), s(&out)],
        &out,
        &["--max-alloc-bytes"],
    );
}

// ---------------------------------------------------------------------------
// Usage errors: refused while the arguments are parsed, exit 2.
// ---------------------------------------------------------------------------

/// Nothing in the family writes stdout, and loaders read `-` as stdin, so an
/// OUT of `-` is refused rather than creating a file called `-` in the
/// working directory.
#[test]
fn an_out_of_dash_is_a_usage_error_and_writes_nothing() {
    if skip_if_no_cli("an_out_of_dash_is_a_usage_error_and_writes_nothing") {
        return;
    }
    let cwd = fresh_dir("dash_out");
    for args in [
        ["jpegsave".to_string(), canonical(), "-".to_string()],
        ["tiffsave".to_string(), canonical(), "-".to_string()],
        [
            "pngload".to_string(),
            fx("foreign/pngsave_default.png"),
            "-".to_string(),
        ],
    ] {
        let run = Command::new(viprs_bin())
            .args(&args)
            .current_dir(&cwd)
            .output()
            .expect("spawn viprs");
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        assert_eq!(run.status.code(), Some(2), "{}", describe(&argv, &run));
        assert!(
            String::from_utf8_lossy(&run.stderr).contains("OUT cannot be -"),
            "{}",
            describe(&argv, &run)
        );
    }
    let left: Vec<_> = std::fs::read_dir(&cwd).unwrap().collect();
    assert!(
        left.is_empty(),
        "an OUT of - left {left:?} in the working directory"
    );
}

#[test]
fn svgload_refuses_a_non_finite_or_non_positive_dpi_and_scale() {
    if skip_if_no_cli("svgload_refuses_a_non_finite_or_non_positive_dpi_and_scale") {
        return;
    }
    // Parsing comes before the feature check, so this holds in a bare build.
    let out = out_path("svgload_bad_number.png");
    let svg = fx("features/canonical.svg");
    for arg in [
        "--dpi=NaN",
        "--dpi=inf",
        "--dpi=-72",
        "--dpi=0",
        "--scale=NaN",
        "--scale=0",
    ] {
        usage_error(
            &["svgload", &svg, s(&out), arg],
            &out,
            &["finite number above 0"],
        );
    }
}

#[test]
fn gifsave_dither_outside_0_to_1_is_a_usage_error() {
    if skip_if_no_cli("gifsave_dither_outside_0_to_1_is_a_usage_error") {
        return;
    }
    let out = out_path("gifsave_bad_dither.gif");
    for arg in ["--dither=1.5", "--dither=-0.5", "--dither=NaN"] {
        usage_error(&["gifsave", &canonical(), s(&out), arg], &out, &["0 to 1"]);
    }
}

/// `tiffsave` writes OUT through the core's `save_tiff` and nothing else: no
/// temp file beside it, which a planted symlink could redirect, and nothing
/// left behind.
#[test]
fn tiffsave_writes_only_its_output() {
    if skip_if_no_cli("tiffsave_writes_only_its_output") {
        return;
    }
    let dir = fresh_dir("tiffsave_alone");
    let out = dir.join("out.tif");
    ok(&[
        "tiffsave",
        &canonical(),
        s(&out),
        "--compression",
        "deflate",
    ]);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["out.tif"], "tiffsave left more than its output");
    decode_compare(&out, &cli_fixture("foreign/tiffsave_deflate.tif"), EXACT);
}
