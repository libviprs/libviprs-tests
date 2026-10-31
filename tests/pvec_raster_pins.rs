//! The pins the raster suites stand on (libviprs-tests#305).
//!
//! The sink takes a `PvecWriter`, so the `pvec` facade this crate builds the writer with has to be
//! the revision the core's `pvec-raster` feature pins: another revision is another crate, and the
//! types would not meet. This compares the two manifests instead of leaving the mismatch to a
//! compile error that names no revision.

use std::path::Path;

fn rev_of(manifest: &str, krate: &str) -> String {
    let line = manifest
        .lines()
        .find(|l| l.trim_start().starts_with(krate) && l.contains("rev ="))
        .unwrap_or_else(|| panic!("no `{krate}` with a rev"));
    let tail = line.split("rev = \"").nth(1).expect("a rev");
    tail.split('"').next().unwrap().to_string()
}

#[test]
fn the_facade_and_geometry_revisions_are_the_ones_the_core_pins() {
    let ours =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let core = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../libviprs/Cargo.toml"),
    )
    .expect("the core checkout beside this one");
    assert_eq!(rev_of(&ours, "pvec-facade"), rev_of(&core, "pvec-facade"));
    assert_eq!(
        rev_of(&ours, "pvec-geometry"),
        rev_of(&core, "pvec-geometry")
    );
}

#[test]
fn both_counterpart_pins_are_full_shas() {
    for name in ["COUNTERPART_REV", "CLI_COUNTERPART_REV"] {
        let text =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(name)).unwrap();
        let sha = text
            .lines()
            .find(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .unwrap()
            .trim();
        assert_eq!(sha.len(), 40, "{name}: {sha}");
        assert!(sha.bytes().all(|b| b.is_ascii_hexdigit()), "{name}: {sha}");
    }
}
