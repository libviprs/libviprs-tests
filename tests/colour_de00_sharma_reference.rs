//! Known-answer golden test for [`Raster::de00_sharma`] against the
//! **published Sharma 2005 CIEDE2000 reference dataset**.
//!
//! This is a *reference* golden, not a vips-differential test:
//! `de00_sharma` deliberately computes the textbook CIEDE2000 value
//! (Sharma, Wu & Dalal 2005), which differs from libvips' `vips_col_dE00`
//! parity path (exposed as `de00`) on asymmetric hue-wrap pairs. The
//! expected values below are the canonical published dE00 numbers from
//! Table 1 of:
//!
//!   G. Sharma, W. Wu, E. N. Dalal, "The CIEDE2000 Color-Difference
//!   Formula: Implementation Notes, Supplementary Test Data, and
//!   Mathematical Observations", Color Research & Application, 30(1),
//!   pp. 21-30, 2005.
//!
//! The dataset is embedded as constants (it is a fixed textbook reference,
//! not a fixture that can drift). Every pair must reproduce within 1e-3.
//!
//! Regression guard for libviprs#332: the original `de00_sharma` copied
//! `de00`'s delta-hue wrap arm verbatim (`360 - (h1'-h2')`), which has the
//! wrong sign relative to the `c1' - c2'` chroma-difference convention, so
//! the RT * dC' * dh' cross term is negated. That is invisible on
//! symmetric-wrap or dC'=0 pairs, but on asymmetric hue-wrap pairs with
//! dC' != 0 (e.g. pair 19, Lab [50,2.5,0]/[56,-27,-3]) it returns 21.12
//! instead of the published 31.9030.

use libviprs::{Interpretation, Raster};

/// Evaluate the public `Raster::de00_sharma` on a single Lab pair.
fn sharma(lab1: [f64; 3], lab2: [f64; 3]) -> f64 {
    let a = Raster::constant(2, 2, &lab1, Interpretation::Lab);
    let b = Raster::constant(2, 2, &lab2, Interpretation::Lab);
    a.de00_sharma(&b).getpoint(0, 0)[0]
}

/// Canonical Sharma/Wu/Dalal 2005 CIEDE2000 test data (Table 1): 34 pairs
/// of `(Lab1, Lab2, published dE00)`.
const SHARMA_2005: &[([f64; 3], [f64; 3], f64)] = &[
    (
        [50.0000, 2.6772, -79.7751],
        [50.0000, 0.0000, -82.7485],
        2.0425,
    ),
    (
        [50.0000, 3.1571, -77.2803],
        [50.0000, 0.0000, -82.7485],
        2.8615,
    ),
    (
        [50.0000, 2.8361, -74.0200],
        [50.0000, 0.0000, -82.7485],
        3.4412,
    ),
    (
        [50.0000, -1.3802, -84.2814],
        [50.0000, 0.0000, -82.7485],
        1.0000,
    ),
    (
        [50.0000, -1.1848, -84.8006],
        [50.0000, 0.0000, -82.7485],
        1.0000,
    ),
    (
        [50.0000, -0.9009, -85.5211],
        [50.0000, 0.0000, -82.7485],
        1.0000,
    ),
    (
        [50.0000, 0.0000, 0.0000],
        [50.0000, -1.0000, 2.0000],
        2.3669,
    ),
    (
        [50.0000, -1.0000, 2.0000],
        [50.0000, 0.0000, 0.0000],
        2.3669,
    ),
    (
        [50.0000, 2.4900, -0.0010],
        [50.0000, -2.4900, 0.0009],
        7.1792,
    ),
    (
        [50.0000, 2.4900, -0.0010],
        [50.0000, -2.4900, 0.0010],
        7.1792,
    ),
    (
        [50.0000, 2.4900, -0.0010],
        [50.0000, -2.4900, 0.0011],
        7.2195,
    ),
    (
        [50.0000, 2.4900, -0.0010],
        [50.0000, -2.4900, 0.0012],
        7.2195,
    ),
    (
        [50.0000, -0.0010, 2.4900],
        [50.0000, 0.0009, -2.4900],
        4.8045,
    ),
    (
        [50.0000, -0.0010, 2.4900],
        [50.0000, 0.0010, -2.4900],
        4.8045,
    ),
    (
        [50.0000, -0.0010, 2.4900],
        [50.0000, 0.0011, -2.4900],
        4.7461,
    ),
    (
        [50.0000, 2.5000, 0.0000],
        [50.0000, 0.0000, -2.5000],
        4.3065,
    ),
    (
        [50.0000, 2.5000, 0.0000],
        [73.0000, 25.0000, -18.0000],
        27.1492,
    ),
    (
        [50.0000, 2.5000, 0.0000],
        [61.0000, -5.0000, 29.0000],
        22.8977,
    ),
    (
        [50.0000, 2.5000, 0.0000],
        [56.0000, -27.0000, -3.0000],
        31.9030,
    ),
    (
        [50.0000, 2.5000, 0.0000],
        [58.0000, 24.0000, 15.0000],
        19.4535,
    ),
    ([50.0000, 2.5000, 0.0000], [50.0000, 3.1736, 0.5854], 1.0000),
    ([50.0000, 2.5000, 0.0000], [50.0000, 3.2972, 0.0000], 1.0000),
    ([50.0000, 2.5000, 0.0000], [50.0000, 1.8634, 0.5757], 1.0000),
    ([50.0000, 2.5000, 0.0000], [50.0000, 3.2592, 0.3350], 1.0000),
    (
        [60.2574, -34.0099, 36.2677],
        [60.4626, -34.1751, 39.4387],
        1.2644,
    ),
    (
        [63.0109, -31.0961, -5.8663],
        [62.8187, -29.7946, -4.0864],
        1.2630,
    ),
    (
        [61.2901, 3.7196, -5.3901],
        [61.4292, 2.2480, -4.9620],
        1.8731,
    ),
    (
        [35.0831, -44.1164, 3.7933],
        [35.0232, -40.0716, 1.5901],
        1.8645,
    ),
    (
        [22.7233, 20.0904, -46.6940],
        [23.0331, 14.9730, -42.5619],
        2.0373,
    ),
    (
        [36.4612, 47.8580, 18.3852],
        [36.2715, 50.5065, 21.2231],
        1.4146,
    ),
    (
        [90.8027, -2.0831, 1.4410],
        [91.1528, -1.6435, 0.0447],
        1.4441,
    ),
    (
        [90.9257, -0.5406, -0.9208],
        [88.6381, -0.8985, -0.7239],
        1.5381,
    ),
    (
        [6.7747, -0.2908, -2.4247],
        [5.8714, -0.0985, -2.2286],
        0.6377,
    ),
    (
        [2.0776, 0.0795, -1.1350],
        [0.9033, -0.0636, -0.5514],
        0.9082,
    ),
];

/// The full published Sharma 2005 dataset must reproduce within 1e-3.
///
/// Against the buggy core (pre-#332-followup) this is RED: pair 19
/// (index 18), Lab [50,2.5,0]/[56,-27,-3], returns 21.12 instead of
/// 31.9030 — an error of ~10.78.
#[test]
fn de00_sharma_matches_published_sharma_2005_dataset() {
    const TOL: f64 = 1e-3;
    let mut failures = Vec::new();
    let mut max_err = 0.0f64;

    for (i, (lab1, lab2, expected)) in SHARMA_2005.iter().enumerate() {
        let got = sharma(*lab1, *lab2);
        let err = (got - expected).abs();
        if err > max_err {
            max_err = err;
        }
        if err > TOL {
            failures.push(format!(
                "pair {} {:?}/{:?}: expected {:.4}, got {:.4} (err {:.4})",
                i + 1,
                lab1,
                lab2,
                expected,
                got,
                err
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "de00_sharma deviates from published Sharma 2005 on {} of {} pairs \
         (max err {:.4}):\n{}",
        failures.len(),
        SHARMA_2005.len(),
        max_err,
        failures.join("\n")
    );
}

/// The specific asymmetric hue-wrap pair from the #332 defect report, in
/// isolation: Lab [50,2.5,0]/[56,-27,-3] must equal the published 31.9030,
/// not the sign-flipped 21.12 the buggy wrap arm produced.
#[test]
fn de00_sharma_asymmetric_wrap_pair19() {
    let got = sharma([50.0, 2.5, 0.0], [56.0, -27.0, -3.0]);
    assert!(
        (got - 31.9030).abs() < 1e-3,
        "published Sharma dE00 for [50,2.5,0]/[56,-27,-3] is 31.9030, got {got:.4}"
    );
}

/// A high-RT asymmetric-wrap pair outside the standard 34-pair table,
/// where the negated cross term is most damaging: the buggy arm returns
/// ~21.84 versus the true ~53.53. Independent published reference value.
#[test]
fn de00_sharma_high_rt_asymmetric_wrap() {
    let got = sharma([78.35, -59.81, -9.59], [69.99, 8.14, 0.04]);
    assert!(
        (got - 53.53).abs() < 1e-2,
        "textbook Sharma dE00 for [78.35,-59.81,-9.59]/[69.99,8.14,0.04] is ~53.53, got {got:.4}"
    );
}
