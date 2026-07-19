//! Drift guards for the deduplicated CIEDE2000 kernel and correctness
//! guards for the `de00` / `de00_sharma` / `read_device_normalised`
//! documentation.
//!
//! `de00` (libvips `vips_col_dE00` parity) and `de00_sharma` (published
//! Sharma 2005) share one arithmetic kernel and differ **only** in the
//! hue-wrap rule (the mean-hue and delta-hue branches). Historically the
//! two were a copy-paste pair (libviprs#370): any correctness fix to a
//! shared line — the T polynomial, RC/RT, SL/SC/SH — had to be applied
//! twice, and a one-sided edit would silently diverge the arms on *every*
//! pair, not just hue-wrap ones.
//!
//! This suite pins that contract behaviourally:
//!
//! - on genuinely non-wrapping pairs the two arms must agree **bit-for-bit**
//!   (they run identical arithmetic), across a dense same-quadrant grid;
//! - on hue-wrap pairs they diverge, and both arms' absolute values are
//!   pinned to the published reference so a drift that moves *both* arms
//!   together (which the agreement test alone cannot see) is still caught.
//!
//! It is a companion to `colour_de00_sharma_reference.rs`, which pins the
//! Sharma arm against the full 34-pair published dataset; here we pin the
//! `de00` parity arm and the cross-arm relationship.
//!
//! The documentation guards (libviprs#371/#372/#373) read `src/colour.rs`
//! and assert the corrected wording is in place: they are not expressible
//! as a vips differential, so they assert on source text directly, in the
//! style of `counterpart_pinning.rs` / `fixture_audit.rs`.

use std::path::PathBuf;

use libviprs::{Interpretation, Raster};

/// Evaluate the public `Raster::de00` (libvips parity arm) on a Lab pair.
fn de00(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let a = Raster::constant(2, 2, &lab1, Interpretation::Lab);
    let b = Raster::constant(2, 2, &lab2, Interpretation::Lab);
    a.de00(&b).getpoint(0, 0)[0]
}

/// Evaluate the public `Raster::de00_sharma` (published Sharma arm) on a
/// Lab pair.
fn de00_sharma(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let a = Raster::constant(2, 2, &lab1, Interpretation::Lab);
    let b = Raster::constant(2, 2, &lab2, Interpretation::Lab);
    a.de00_sharma(&b).getpoint(0, 0)[0]
}

/// The two arms must be **bit-identical** on every non-wrapping pair,
/// because outside the hue-wrap branch they execute the same arithmetic.
///
/// A non-wrapping pair is one whose two colours share an `(a*, b*)`
/// quadrant, so `|Δh'|` stays well under 180 and neither arm ever enters
/// its wrap branch. Any one-sided drift in the shared kernel (T, RC/RT,
/// SL/SC/SH, G, …) breaks the exact equality here, on 16 384 pairs.
#[test]
fn arms_agree_bit_for_bit_on_nonwrapping_pairs() {
    let mags = [3.0, 12.0, 35.0, 60.0];
    let lightness = [15.0, 40.0, 55.0, 85.0];
    let quadrants = [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)];

    let mut checked = 0u64;
    for &l1 in &lightness {
        for &l2 in &lightness {
            for (sa, sb) in quadrants {
                for &a1 in &mags {
                    for &b1 in &mags {
                        for &a2 in &mags {
                            for &b2 in &mags {
                                let p1 = [l1, sa * a1, sb * b1];
                                let p2 = [l2, sa * a2, sb * b2];
                                let d = de00(p1, p2);
                                let s = de00_sharma(p1, p2);
                                assert_eq!(
                                    d.to_bits(),
                                    s.to_bits(),
                                    "arms diverge on non-wrap pair {p1:?}/{p2:?}: \
                                     de00={d} de00_sharma={s} — shared kernel drift"
                                );
                                checked += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(checked, 16_384, "grid coverage changed unexpectedly");
}

/// On hue-wrap pairs the arms diverge, and the divergence is confined to
/// the hue term: both arms' *absolute* values are pinned to the published
/// reference. This catches a drift that shifts both arms equally (leaving
/// the agreement test above green) and locks the hue-rule contract.
///
/// - Genuinely asymmetric wrap (`|Δh'| > 180`), the ΔC' != 0 pair from
///   libviprs#332: de00 (parity) = 27.2312, de00_sharma (published)
///   = 31.9030.
/// - The antipodal boundary (`|Δh'| == 180` exactly, a *symmetric*
///   h1'+h2'==360 pair on the yellow/blue b* axis): the arms split only
///   because de00 uses a `< 180` branch cutoff and Sharma a `<= 180` one,
///   so the mean-hue term differs. de00 = 4.7460, de00_sharma = 4.8045.
#[test]
fn arms_diverge_only_via_hue_term_on_wrap_pairs() {
    // Asymmetric hue-wrap, ΔC' != 0.
    let a1 = [50.0, 2.5, 0.0];
    let a2 = [56.0, -27.0, -3.0];
    let d_asym = de00(a1, a2);
    let s_asym = de00_sharma(a1, a2);
    assert!(
        (d_asym - 27.2312).abs() < 1e-3,
        "de00 parity arm for [50,2.5,0]/[56,-27,-3] should be 27.2312, got {d_asym}"
    );
    assert!(
        (s_asym - 31.9030).abs() < 1e-3,
        "de00_sharma arm for [50,2.5,0]/[56,-27,-3] should be 31.9030, got {s_asym}"
    );

    // Antipodal boundary: symmetric pair, |Δh'| == 180 exactly.
    let b1 = [50.0, 0.0, 2.49];
    let b2 = [50.0, 0.0, -2.49];
    let d_bnd = de00(b1, b2);
    let s_bnd = de00_sharma(b1, b2);
    assert!(
        (d_bnd - 4.7460).abs() < 1e-3,
        "de00 parity arm for [50,0,2.49]/[50,0,-2.49] should be 4.7460, got {d_bnd}"
    );
    assert!(
        (s_bnd - 4.8045).abs() < 1e-3,
        "de00_sharma arm for [50,0,2.49]/[50,0,-2.49] should be 4.8045, got {s_bnd}"
    );
    assert!(
        (s_bnd - d_bnd).abs() > 0.05,
        "the antipodal boundary pair must diverge between the arms: {s_bnd} vs {d_bnd}"
    );
}

/// Independent pin on the `de00` parity arm's shared kernel: the ported
/// libvips reference pair Lab [50,10,20]/[40,-20,10] is 30.238, and it is
/// a non-wrap pair so `de00_sharma` reproduces it identically. Together
/// with `colour_de00_sharma_reference.rs` (which pins the Sharma arm on 34
/// pairs) this makes a one-sided kernel drift visible from *either* arm.
#[test]
fn de00_parity_reference_pair_pinned() {
    let p1 = [50.0, 10.0, 20.0];
    let p2 = [40.0, -20.0, 10.0];
    let d = de00(p1, p2);
    assert!(
        (d - 30.238).abs() < 1e-2,
        "de00 parity arm for [50,10,20]/[40,-20,10] should be ~30.238, got {d}"
    );
    assert_eq!(
        d.to_bits(),
        de00_sharma(p1, p2).to_bits(),
        "non-wrap reference pair must be arm-independent"
    );
}

/// The dataset-wide `de00`-vs-Sharma deviation ceiling the `de00`
/// docstring quotes (~4.67 units, ~1.17×) must be numerically true: the
/// worst departure across the published 34-pair set is the asymmetric-wrap
/// pair [50,2.5,0]/[56,-27,-3].
#[test]
fn de00_parity_ceiling_matches_docstring_claim() {
    // The Sharma 2005 pairs whose published dE00 differs from de00 at all
    // (the hue-wrap subset); the rest agree to ~1e-9. The worst is the
    // asymmetric-wrap pair.
    let published: &[([f64; 3], [f64; 3], f64)] = &[
        ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        ([50.0, 2.5, 0.0], [61.0, -5.0, 29.0], 22.8977),
        ([50.0, 2.5, 0.0], [56.0, -27.0, -3.0], 31.9030),
        ([50.0, 2.5, 0.0], [58.0, 24.0, 15.0], 19.4535),
    ];
    let mut max_dev = 0.0f64;
    let mut max_ratio = 1.0f64;
    for (p1, p2, expected) in published {
        let d = de00(*p1, *p2);
        let dev = (d - expected).abs();
        if dev > max_dev {
            max_dev = dev;
            max_ratio = (expected / d).max(d / expected);
        }
    }
    assert!(
        (max_dev - 4.67).abs() < 0.05,
        "docstring claims a ~4.67-unit parity ceiling; measured {max_dev:.4}"
    );
    assert!(
        (max_ratio - 1.17).abs() < 0.01,
        "docstring claims a ~1.17× parity ceiling; measured {max_ratio:.4}"
    );
}

// ---------------------------------------------------------------------------
// Documentation guards (libviprs#371/#372/#373). Not expressible as a vips
// differential; asserted against the core source text.
// ---------------------------------------------------------------------------

/// Read `src/colour.rs` from the core crate and return it with the `///` /
/// `//` doc prefixes stripped and all runs of whitespace collapsed to a
/// single space, so substring checks are robust to line wrapping.
fn colour_source_normalised() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../libviprs/src/colour.rs");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let stripped: String = raw
        .lines()
        .map(|line| {
            let t = line.trim_start();
            t.strip_prefix("///")
                .or_else(|| t.strip_prefix("//"))
                .unwrap_or(t)
        })
        .collect::<Vec<_>>()
        .join(" ");
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// libviprs#371/#372: the `de00` "parity ceiling" docstring must not claim
/// the Sharma divergence appears *only* on asymmetric hue-wrap pairs (the
/// antipodal `|Δh'| == 180` boundary is symmetric yet also diverges), and
/// must not misdescribe the boundary example as straddling the 0/360 red
/// boundary — its hues are on the yellow/blue b* axis (90°/270°).
#[test]
fn de00_docstring_geometry_is_corrected() {
    let src = colour_source_normalised();
    // #372: must not claim the divergence is limited to asymmetric pairs.
    assert!(
        !src.contains("appears only on **asymmetric**"),
        "de00 docstring still claims divergence is limited to asymmetric pairs (#372)"
    );
    // #371/#372: must document the antipodal |Δh'| == 180 boundary and
    // place its example colours on the yellow/blue b* axis, not the red
    // boundary the asymmetric case straddles.
    assert!(
        src.contains("antipodal"),
        "de00 docstring must document the antipodal |Δh'| == 180 boundary case (#371/#372)"
    );
    assert!(
        src.contains("yellow/blue b* axis"),
        "de00 docstring must correct the boundary example's geometry to the b* axis (#372)"
    );
}

/// libviprs#373: `read_device_normalised`'s rationale must not overstate
/// the old `.round()` bug as collapsing the sample to `{0, 1/255}` — the
/// crate's stated convention is a `0..255` device float, so `.round()`
/// quantised to 256 integer levels (8-bit), not two.
#[test]
fn read_device_normalised_rationale_is_corrected() {
    let src = colour_source_normalised();
    assert!(
        !src.contains("{0, 1/255}"),
        "read_device_normalised rationale still overstates the old bug as {{0, 1/255}} (#373)"
    );
    assert!(
        src.contains("nearest integer"),
        "read_device_normalised rationale must describe the old .round() as quantising \
         the 0..255 device float to the nearest integer (256 levels) (#373)"
    );
}
