//! Documentation guards for the `s3` -> `object-store-sink` cargo-feature
//! rename (libviprs#385/#386/#387/#388/#389/#390).
//!
//! The feature that gates the `sink_object_store` module (`ObjectStoreSink`)
//! was renamed from `s3` to `object-store-sink`, with `s3` retained as a
//! deprecated alias (`s3 = ["object-store-sink"]` in `Cargo.toml`). The code
//! already gates on the new name; these guards pin the *documentation* to the
//! same story so the README, the crate-root "Feature flags" rustdoc, and the
//! CHANGELOG stop steering users to the deprecated alias.
//!
//! This is a pure documentation/packaging change: there is no pixel behaviour
//! to express as a vips differential, so — in the style of
//! `colour_de00_dedup_reference.rs`, `counterpart_pinning.rs`, and
//! `fixture_audit.rs` — the guards assert on the core crate's source text
//! directly. The one behavioural anchor is the `Cargo.toml` `[features]`
//! table: both feature names must still gate the same module, which
//! `both_feature_names_gate_the_object_store_sink` pins.

use std::path::PathBuf;

/// Absolute path to a file in the core `libviprs` crate (the path-dep this
/// test crate compiles against).
fn core_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../libviprs")
        .join(rel)
}

/// Read a core-crate file, lowercase it, and collapse all runs of whitespace
/// to a single space so substring checks are robust to line wrapping.
fn normalised(rel: &str) -> String {
    let path = core_path(rel);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Extract only the crate-root inner-doc (`//!`) lines from `src/lib.rs`,
/// with the `//!` prefix stripped, joined and whitespace-collapsed. This
/// isolates the "Feature flags" rustdoc prose from the `#[cfg(...)]` code
/// gates (which already name `object-store-sink`), so the assertions below
/// speak to the documentation a reader sees, not the compiled gate.
fn lib_rustdoc_normalised() -> String {
    let path = core_path("src/lib.rs");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let doc: String = raw
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("//!"))
        .collect::<Vec<_>>()
        .join(" ");
    doc.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Slice the `## [Unreleased]` section out of `CHANGELOG.md` (up to the next
/// released-version heading), whitespace-collapsed and lowercased, so the
/// rename entry is asserted in the *unreleased* section specifically.
fn changelog_unreleased_normalised() -> String {
    let path = core_path("CHANGELOG.md");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut in_unreleased = false;
    let mut section = String::new();
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("## [Unreleased]") {
            in_unreleased = true;
            continue;
        }
        if in_unreleased && t.starts_with("## [") {
            break; // reached the first released version heading
        }
        if in_unreleased {
            section.push_str(line);
            section.push(' ');
        }
    }
    assert!(
        !section.trim().is_empty(),
        "CHANGELOG.md has no non-empty [Unreleased] section"
    );
    section
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// #385/#389: the README must document `object-store-sink` as the canonical
/// feature name and present `s3` only as a deprecated alias — not as the sole
/// / canonical feature name it historically advertised.
#[test]
fn readme_documents_object_store_sink_as_canonical() {
    let readme = normalised("README.md");
    assert!(
        readme.contains("object-store-sink"),
        "README does not document the canonical `object-store-sink` feature (#385/#389)"
    );
    assert!(
        readme.contains("deprecated alias"),
        "README does not mark `s3` as a deprecated alias of `object-store-sink` (#385/#389)"
    );
    // The module table must gate `sink_object_store` on the new name.
    assert!(
        readme.contains("gated by `object-store-sink`"),
        "README module table still gates `sink_object_store` on the old feature name (#389)"
    );
}

/// #386/#388: the crate-root "Feature flags" rustdoc must advertise
/// `object-store-sink` as the gating feature and mention `s3` only as a
/// deprecated alias, instead of steering users to the deprecated name.
#[test]
fn lib_rs_feature_flags_rustdoc_advertises_object_store_sink() {
    let doc = lib_rustdoc_normalised();
    assert!(
        doc.contains("## feature flags"),
        "src/lib.rs crate-root rustdoc no longer has a Feature flags section"
    );
    assert!(
        doc.contains("`object-store-sink`"),
        "Feature flags rustdoc does not advertise the canonical `object-store-sink` gate (#386/#388)"
    );
    assert!(
        doc.contains("deprecated alias"),
        "Feature flags rustdoc does not mark `s3` as a deprecated alias (#386/#388)"
    );
}

/// #387/#390: the CHANGELOG must record the `s3` -> `object-store-sink` rename
/// and the `s3` deprecation in the [Unreleased] section, as the crate declares
/// adherence to Keep a Changelog and SemVer.
#[test]
fn changelog_records_s3_rename_and_deprecation() {
    let unreleased = changelog_unreleased_normalised();
    assert!(
        unreleased.contains("object-store-sink"),
        "CHANGELOG [Unreleased] does not mention the new `object-store-sink` feature (#387/#390)"
    );
    assert!(
        unreleased.contains("`s3`") || unreleased.contains(" s3 "),
        "CHANGELOG [Unreleased] does not mention the old `s3` feature name (#387/#390)"
    );
    assert!(
        unreleased.contains("renamed"),
        "CHANGELOG [Unreleased] does not record the feature *rename* (#387/#390)"
    );
    assert!(
        unreleased.contains("deprecated"),
        "CHANGELOG [Unreleased] does not record the `s3` deprecation (#387/#390)"
    );
}

/// Behavioural anchor: both feature names must still gate the same
/// `sink_object_store` module. `s3` is kept as a pure alias
/// (`s3 = ["object-store-sink"]`), so a consumer pinned to either name keeps
/// building. This is the one guard that is not merely prose — it pins the
/// packaging contract in `Cargo.toml`.
#[test]
fn both_feature_names_gate_the_object_store_sink() {
    let path = core_path("Cargo.toml");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    // Canonical feature exists.
    assert!(
        raw.lines()
            .any(|l| l.trim_start().starts_with("object-store-sink = [")),
        "Cargo.toml has no `object-store-sink` feature"
    );
    // `s3` survives only as an alias that pulls in `object-store-sink`.
    let s3_line = raw
        .lines()
        .find(|l| l.trim_start().starts_with("s3 = ["))
        .expect("Cargo.toml has no `s3` alias feature");
    assert!(
        s3_line.contains("object-store-sink"),
        "`s3` feature must alias `object-store-sink` (s3 = [\"object-store-sink\"]), got: {s3_line}"
    );

    // Sanity: the gated module source is present in the core crate, so the
    // feature actually has something to gate.
    assert!(
        core_path("src/sink_object_store.rs").exists(),
        "core crate is missing src/sink_object_store.rs"
    );
}
