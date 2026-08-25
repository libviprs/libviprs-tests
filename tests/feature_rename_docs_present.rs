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

/// Split `CHANGELOG.md` into its `## [..]` entries, pairing each heading with
/// its body whitespace-collapsed and lowercased.
///
/// Deliberately version-agnostic. A changelog entry does not stay where it was
/// written: Keep a Changelog has a release promote `[Unreleased]` into a fresh
/// version section, so any assertion aimed at `[Unreleased]` goes stale the
/// moment a release ships and then fails by construction. That is exactly what
/// 0.4.0 did to the rename entry (libviprs-tests#141). Returning every entry
/// lets a caller ask "does *some* entry record this?", which survives releases,
/// while still keeping the question scoped to a single entry so unrelated
/// sections cannot satisfy a multi-part assertion between them.
fn changelog_sections_normalised() -> Vec<(String, String)> {
    let path = core_path("CHANGELOG.md");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut sections: Vec<(String, String)> = Vec::new();
    for line in raw.lines() {
        let t = line.trim_start();
        if t.starts_with("## [") {
            sections.push((t.trim_end().to_string(), String::new()));
            continue;
        }
        // Anything before the first `## [..]` heading is the file preamble.
        if let Some((_, body)) = sections.last_mut() {
            body.push_str(line);
            body.push(' ');
        }
    }
    assert!(
        !sections.is_empty(),
        "{} has no `## [..]` changelog entries",
        path.display()
    );
    sections
        .into_iter()
        .map(|(heading, body)| {
            let body = body
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            (heading, body)
        })
        .collect()
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
/// and the `s3` deprecation, as the crate declares adherence to Keep a
/// Changelog and SemVer.
///
/// This asks *which* entry carries the rename rather than naming one. It
/// originally read `[Unreleased]`, because that is where the entry sat while
/// the rename was pending, and 0.4.0 promoted it into `## [0.4.0]` on release,
/// which is precisely what Keep a Changelog prescribes and which broke the test
/// (libviprs-tests#141). Re-pinning it to `[0.4.0]` would only relocate the
/// staleness, so the guard now scans every entry instead. It still requires one
/// single entry to tell the whole story, so four unrelated sections cannot
/// satisfy the four checks between them.
///
/// What remains is a weak guard, and deliberately so: released history does not
/// change, so this can only fail if someone rewrites the 0.4.0 entry or drops
/// the rename before it ships in a future one.
#[test]
fn changelog_records_s3_rename_and_deprecation() {
    let sections = changelog_sections_normalised();
    let records_the_rename = |body: &str| {
        body.contains("object-store-sink")
            && (body.contains("`s3`") || body.contains(" s3 "))
            && body.contains("renamed")
            && body.contains("deprecated")
    };
    assert!(
        sections.iter().any(|(_, body)| records_the_rename(body)),
        "no CHANGELOG entry records the `s3` -> `object-store-sink` rename \
         together with the `s3` deprecation (#387/#390); entries scanned: {:?}",
        sections.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>()
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
