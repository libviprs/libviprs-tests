//! `viprs` says what it was built with, and a codec it was built without is a
//! refusal naming the feature, never a panic and never "unsupported format"
//! (libviprs/libviprs-cli#64).
//!
//! # Why this drives several builds
//!
//! Every other CLI cell runs the one `--no-default-features` binary
//! `tests/common/cli.rs` builds. That binary can only ever show one side of a
//! feature: it proves a refusal and can never prove a decode. So this file
//! builds the CLI once per configuration it checks, with
//! [`viprs_bin_for`](common::cli::viprs_bin_for), into a target dir of its own:
//!
//! * `bare` (`--no-default-features`), which has none of the codec features
//!   and must refuse all four fixtures, each naming its own feature;
//! * `default` and `full`, whose `viprs features` output is pinned exactly;
//! * one build per codec with that feature and nothing else, which must
//!   decode its own fixture and must list exactly that one feature.
//!
//! The single-feature builds are the half that `full` alone would not give:
//! a decoder that only worked when some other feature happened to be on
//! passes under `full` and fails here.
//!
//! # Evidence, not exit codes
//!
//! A decode is checked on its pixels. `viprs copy <fixture> out.png` has to
//! write a PNG equal at tolerance 0 to `tests/fixtures/canonical_input.png`,
//! which the AVIF, JPEG XL and JPEG 2000 fixtures were losslessly encoded
//! from (`tests/fixtures/cli/PROVENANCE.md`, "feature-gated codec fixtures").
//! The SVG fixture is two flat rectangles, so its render is checked at a
//! pixel inside each. A refusal is checked on its exit status (exactly 1),
//! on the feature it names, and on the output file it must not have written.
//!
//! # The skip
//!
//! Every cell returns early when the CLI sibling is absent, and
//! `VIPRS_REQUIRE_CLI=1` on the `cli-differential` job turns that skip into a
//! panic, as for every other CLI cell.

mod common;

use std::path::{Path, PathBuf};
use std::process::Output;

use common::cli::{cli_fixture, cli_source_available, run_bin, viprs_bin_for};

/// The four codec features, each with its fixture and what it decodes to.
struct Codec {
    feature: &'static str,
    fixture: &'static str,
    /// `(width, height)` the decode must have.
    dims: (u32, u32),
    /// Encoded losslessly from `canonical_input.png`, so the decode must
    /// equal it exactly. False for the hand-written SVG.
    lossless_of_canonical: bool,
}

const CODECS: &[Codec] = &[
    Codec {
        feature: "avif",
        fixture: "features/canonical.avif",
        dims: (256, 256),
        lossless_of_canonical: true,
    },
    Codec {
        feature: "svg",
        fixture: "features/canonical.svg",
        dims: (64, 48),
        lossless_of_canonical: false,
    },
    Codec {
        feature: "jxl",
        fixture: "features/canonical.jxl",
        dims: (256, 256),
        lossless_of_canonical: true,
    },
    Codec {
        feature: "jp2k",
        fixture: "features/canonical.jp2",
        dims: (256, 256),
        lossless_of_canonical: true,
    },
];

/// What `--features full` must turn on, in the order `viprs features`
/// prints (alphabetical). Every capability-bearing feature except
/// `pdfium-static`, which conflicts with the runtime-binding pdfium path.
const FULL: &[&str] = &[
    "avif",
    "jp2k",
    "jxl",
    "object-store-sink",
    "packfile",
    "pdfium",
    "s3",
    "svg",
    "tracing",
];

fn codec(feature: &str) -> &'static Codec {
    CODECS
        .iter()
        .find(|c| c.feature == feature)
        .unwrap_or_else(|| panic!("no codec row for `{feature}`"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn describe(what: &str, out: &Output) -> String {
    format!(
        "{what} exited {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status.code(),
        text(&out.stdout),
        text(&out.stderr)
    )
}

/// `viprs features` and `viprs features --json` both list exactly `expected`,
/// in that order, and both exit 0.
fn assert_lists_exactly(bin: &Path, config: &str, expected: &[&str]) {
    let out = run_bin(bin, &["features"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        describe(&format!("viprs-{config} features"), &out)
    );
    let listed: Vec<String> = text(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(
        listed, expected,
        "viprs-{config} features must list exactly the features it was built with, \
         one per line, in order"
    );

    let out = run_bin(bin, &["features", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        describe(&format!("viprs-{config} features --json"), &out)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "viprs-{config} features --json is not JSON ({e}):\n{}",
            text(&out.stdout)
        )
    });
    let listed: Vec<&str> = json["features"]
        .as_array()
        .unwrap_or_else(|| panic!("--json has no `features` array: {json}"))
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| panic!("non-string feature in {json}"))
        })
        .collect();
    assert_eq!(
        listed, expected,
        "viprs-{config} features --json must agree with the plain listing"
    );
}

/// `out` is the refusal a build without `feature` owes for that codec's
/// fixture: exit 1, the feature named as the thing to rebuild with, no panic,
/// no generic "unsupported format", and no other codec feature blamed.
fn assert_refusal(out: &Output, what: &str, feature: &str) {
    let stderr = text(&out.stderr);
    let described = describe(what, out);
    assert_eq!(out.status.code(), Some(1), "{described}");
    assert!(
        !stderr.contains("panicked"),
        "a missing feature must be a refusal, not a panic: {described}"
    );
    assert!(
        !stderr.to_ascii_lowercase().contains("unsupported format"),
        "a missing feature must not read as the generic unsupported format: {described}"
    );
    assert!(
        stderr.contains(&format!("`{feature}`")),
        "the refusal must name the `{feature}` feature: {described}"
    );
    assert!(
        stderr.contains(&format!("--features {feature}")),
        "the refusal must say what to rebuild with (`--features {feature}`): {described}"
    );
    for other in CODECS.iter().filter(|c| c.feature != feature) {
        assert!(
            !stderr.contains(&format!("`{}`", other.feature)),
            "the refusal for `{feature}` blames `{}` as well: {described}",
            other.feature
        );
    }
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("create a scratch dir")
}

fn canonical_input() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canonical_input.png")
}

/// A build without the codec's feature refuses its fixture through both ways
/// in: the built-in `info` and the op harness (`copy`), which load through
/// different code.
fn assert_bare_build_refuses(feature: &str) {
    if !cli_source_available() {
        return;
    }
    let c = codec(feature);
    let bin = viprs_bin_for(false, &[]);
    let fixture = cli_fixture(c.fixture);
    let fixture = fixture.to_str().unwrap();

    let out = run_bin(&bin, &["info", fixture]);
    assert_refusal(&out, &format!("viprs-bare info {}", c.fixture), feature);

    let dir = scratch();
    let png = dir.path().join("out.png");
    let out = run_bin(&bin, &["copy", fixture, png.to_str().unwrap()]);
    assert_refusal(&out, &format!("viprs-bare copy {}", c.fixture), feature);
    assert!(
        !png.exists(),
        "a refused load must not leave an output behind at {}",
        png.display()
    );
}

/// A build with only the codec's feature lists exactly that feature, and
/// decodes the fixture to the right pixels through `info` and `copy`.
fn assert_single_feature_build_decodes(feature: &str) {
    if !cli_source_available() {
        return;
    }
    let c = codec(feature);
    let bin = viprs_bin_for(false, &[feature]);
    assert_lists_exactly(&bin, &format!("bare+{feature}"), &[feature]);

    let fixture = cli_fixture(c.fixture);
    let fixture = fixture.to_str().unwrap();

    let out = run_bin(&bin, &["info", fixture]);
    let described = describe(&format!("viprs-bare+{feature} info {}", c.fixture), &out);
    assert_eq!(out.status.code(), Some(0), "{described}");
    let (w, h) = c.dims;
    assert!(
        text(&out.stdout).contains(&format!("Dimensions: {w}x{h}")),
        "info must report the fixture's {w}x{h}: {described}"
    );

    let dir = scratch();
    let png = dir.path().join("out.png");
    let out = run_bin(&bin, &["copy", fixture, png.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        describe(&format!("viprs-bare+{feature} copy {}", c.fixture), &out)
    );
    if c.lossless_of_canonical {
        common::cli::decode_compare(&png, &canonical_input(), 0.0);
    } else {
        assert_svg_render(&png);
    }
}

/// The SVG fixture is a 64x48 red field with a blue rectangle over
/// x 16..48, y 8..24, every edge on a pixel boundary, so there is no
/// antialiasing to tolerate at the two pixels read here.
fn assert_svg_render(png: &Path) {
    let r = libviprs::decode_file(png)
        .unwrap_or_else(|e| panic!("cannot decode {}: {e}", png.display()));
    assert_eq!((r.width(), r.height()), (64, 48));
    let channels = r.format().channels();
    assert!(
        channels >= 3 && r.format().bytes_per_channel() == 1,
        "expected an 8-bit RGB(A) render, got {:?}",
        r.format()
    );
    let at = |x: usize, y: usize| {
        let off = y * r.stride() + x * channels;
        [r.data()[off], r.data()[off + 1], r.data()[off + 2]]
    };
    assert_eq!(
        at(4, 4),
        [255, 0, 0],
        "the field outside the rectangle is red"
    );
    assert_eq!(at(32, 16), [0, 0, 255], "the rectangle is blue");
}

// ---------------------------------------------------------------------------
// `viprs features`
// ---------------------------------------------------------------------------

#[test]
fn features_lists_nothing_for_a_bare_build() {
    if !cli_source_available() {
        return;
    }
    let bin = viprs_bin_for(false, &[]);
    assert_lists_exactly(&bin, "bare", &[]);
}

#[test]
fn features_lists_exactly_the_default_build() {
    if !cli_source_available() {
        return;
    }
    let bin = viprs_bin_for(true, &[]);
    assert_lists_exactly(&bin, "default", &["pdfium"]);
}

#[test]
fn features_lists_exactly_the_full_build() {
    if !cli_source_available() {
        return;
    }
    let bin = viprs_bin_for(true, &["full"]);
    assert_lists_exactly(&bin, "default+full", FULL);
}

// ---------------------------------------------------------------------------
// Refused without the feature
// ---------------------------------------------------------------------------

#[test]
fn a_build_without_avif_refuses_the_avif_fixture() {
    assert_bare_build_refuses("avif");
}

#[test]
fn a_build_without_svg_refuses_the_svg_fixture() {
    assert_bare_build_refuses("svg");
}

#[test]
fn a_build_without_jxl_refuses_the_jxl_fixture() {
    assert_bare_build_refuses("jxl");
}

#[test]
fn a_build_without_jp2k_refuses_the_jp2k_fixture() {
    assert_bare_build_refuses("jp2k");
}

// ---------------------------------------------------------------------------
// Decoded with it
// ---------------------------------------------------------------------------

#[test]
fn a_build_with_only_avif_decodes_the_avif_fixture() {
    assert_single_feature_build_decodes("avif");
}

#[test]
fn a_build_with_only_svg_decodes_the_svg_fixture() {
    assert_single_feature_build_decodes("svg");
}

#[test]
fn a_build_with_only_jxl_decodes_the_jxl_fixture() {
    assert_single_feature_build_decodes("jxl");
}

#[test]
fn a_build_with_only_jp2k_decodes_the_jp2k_fixture() {
    assert_single_feature_build_decodes("jp2k");
}

// ---------------------------------------------------------------------------
// The fixtures are what this file says they are
// ---------------------------------------------------------------------------

/// Runs with no CLI at all, so a fixture that went missing or changed shape
/// is caught in the default `cargo test` too, not only where the CLI builds.
#[test]
fn every_codec_fixture_is_present_and_is_its_format() {
    let magic: &[(&str, &[u8], usize)] = &[
        // ISO BMFF `ftyp` box at offset 4, AVIF brand at 8.
        ("avif", b"ftypavif", 4),
        ("jxl", &[0xff, 0x0a], 0),
        // JP2 signature box.
        ("jp2k", &[0, 0, 0, 0x0c, b'j', b'P', b' ', b' '], 0),
        ("svg", b"<svg", 0),
    ];
    for c in CODECS {
        let path = cli_fixture(c.fixture);
        let bytes =
            std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let (_, sig, at) = magic
            .iter()
            .find(|(f, _, _)| *f == c.feature)
            .unwrap_or_else(|| panic!("no signature row for `{}`", c.feature));
        assert!(
            bytes.len() >= at + sig.len() && &bytes[*at..at + sig.len()] == *sig,
            "{} does not start with the {} signature",
            c.fixture,
            c.feature
        );
        assert!(
            bytes.len() < 16 * 1024,
            "{} is {} bytes; these fixtures are meant to stay tiny",
            c.fixture,
            bytes.len()
        );
    }
}
