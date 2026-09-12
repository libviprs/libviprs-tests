//! Cargo.toml metadata / provenance guards (issues #375, #376, #377, #378, #391).
//!
//! These pin the *documentation accuracy* of the core `libviprs/Cargo.toml`
//! dependency-provenance block. They are not vips-differential: every finding
//! is about a manifest comment or feature-alias claim that Cargo itself cannot
//! express or enforce, so there is no image operation to diff against a libvips
//! reference (vips_diff_applicable = false).
//!
//! The single behavioural fact Cargo *can* enforce — that the pdfium-render
//! fork is pinned to an immutable commit `rev` rather than a bare, force-
//! pushable `branch` — is asserted structurally against the manifest so a
//! future edit cannot silently regress the pin back to a mutable branch.
//!
//! The core manifest is read from `../libviprs/Cargo.toml`, the same path this
//! crate's `libviprs = { path = "../libviprs" }` dependency resolves from, so
//! the test always inspects the exact core crate it compiles against. A second
//! group guards this crate's own `Cargo.toml`: its `[patch.crates-io]` override
//! mirrors the pdfium fork but pins the MUTABLE `libviprs/integration` branch
//! rather than the core crate's immutable `rev`, so the comment there must not
//! let "mirror"/"lockstep" imply matching immutability. Both are checked-in
//! repository files needing no feature or native install, so this runs under
//! the default `cargo test`.

use std::path::PathBuf;

/// Path to the core crate's manifest, resolved the same way as the
/// `path = "../libviprs"` dependency edge in this crate's own `Cargo.toml`.
fn core_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../libviprs/Cargo.toml")
}

fn read_core_manifest() -> String {
    let p = core_manifest_path();
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("failed to read {}: {e}", p.display()))
}

/// Path to this (tests) crate's own manifest, which carries the
/// `[patch.crates-io]` override that mirrors the core crate's pdfium fork.
fn tests_manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

fn read_tests_manifest() -> String {
    let p = tests_manifest_path();
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("failed to read {}: {e}", p.display()))
}

/// Extract the single-line `pdfium-render = { ... }` workspace-dependency
/// declaration (the actual key/value line, not the surrounding comments).
fn pdfium_dep_line(manifest: &str) -> String {
    let mut in_dep = false;
    let mut collected = String::new();
    for line in manifest.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if !in_dep {
            if trimmed.starts_with("pdfium-render") && trimmed.contains('=') {
                in_dep = true;
                collected.push_str(trimmed);
            }
        } else {
            collected.push(' ');
            collected.push_str(trimmed);
        }
        // The declaration closes on the line carrying the final `}`.
        if in_dep && trimmed.contains('}') {
            break;
        }
    }
    assert!(
        !collected.is_empty(),
        "no `pdfium-render = {{ ... }}` dependency line found in core Cargo.toml"
    );
    collected
}

// ---------------------------------------------------------------------------
// #375, then #981 — there is no pin any more, and that is the point.
//
// These guards used to require the core crate's `pdfium-render` to be a git
// dependency at an immutable `rev`, never a mutable `branch`, and to require
// this repo's `[patch.crates-io]` to admit in prose that it *was* a branch pin
// and route the follow-up. Both were the right guards for a world where the
// fork was load-bearing.
//
// libviprs#981 retired the fork: upstream reinstated the per-call locking in
// 0.9.4 and what the fork still carried over it is nothing either crate calls.
// A git source cannot survive `cargo publish`, so pinning one made the crate
// everyone builds a different piece of software from the crate everyone
// installs. Worse for this repo specifically: our patch tracked a *branch*, so
// the suite that gates libviprs was exercising a third variant again, one
// where `src/bindings/thread_safe.rs` is absent entirely.
//
// So the invariant flipped, and these assert the new one in both manifests.
// Inverted rather than deleted, because a silent resolution change on this
// dependency is what #149 was.
// ---------------------------------------------------------------------------

/// Neither manifest declares `pdfium-render` from git.
#[test]
fn pdfium_comes_from_the_registry_in_both_manifests() {
    for (what, manifest) in [
        ("the core crate", read_core_manifest()),
        ("this crate", read_tests_manifest()),
    ] {
        let dep = pdfium_dep_line(&manifest);
        for forbidden in ["git =", "rev =", "branch ="] {
            assert!(
                !dep.contains(forbidden),
                "{what} declares pdfium-render with `{forbidden}`, so a git \
                 consumer and a crates.io consumer get different code under one \
                 name (libviprs#981). Found: {dep}"
            );
        }
    }
}

/// Neither manifest carries a `[patch.crates-io]` override for it either.
///
/// A patch is the same divergence by another route, and ours was worse than the
/// core's `rev` because it tracked a branch head.
#[test]
fn no_patch_table_redirects_pdfium_render() {
    for (what, manifest) in [
        ("the core crate", read_core_manifest()),
        ("this crate", read_tests_manifest()),
    ] {
        // Line-based, and deliberately so. Splitting the file on the literal
        // `[patch.crates-io]` also matches the string where a *comment*
        // mentions it, and the core's manifest opens with a paragraph doing
        // exactly that. A guard that reads a comment as configuration is the
        // failure this file exists to prevent, so the table is recognised only
        // as a line that is nothing but the header.
        let mut in_patch_table = false;
        let mut patched = false;
        for line in manifest.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_patch_table = trimmed == "[patch.crates-io]";
                continue;
            }
            if in_patch_table && !trimmed.starts_with('#') && trimmed.starts_with("pdfium-render") {
                patched = true;
                break;
            }
        }
        assert!(
            !patched,
            "{what} still redirects pdfium-render through [patch.crates-io]"
        );
    }
}

/// Both manifests agree on the floor and on the libpdfium ABI.
///
/// Cargo unions features across the graph, so this repo taking defaults would
/// turn `pdfium_latest` back on and undo the explicit ABI pin the core makes.
/// The two have to be chosen together, and a mismatch here is the drift that
/// would hide it.
#[test]
fn both_manifests_name_the_same_floor_and_abi() {
    let core = pdfium_dep_line(&read_core_manifest());
    let tests = pdfium_dep_line(&read_tests_manifest());
    for (what, dep) in [("the core crate", &core), ("this crate", &tests)] {
        assert!(
            dep.contains("0.9.4"),
            "{what} does not declare a 0.9.4 floor: {dep}"
        );
        assert!(
            dep.contains("default-features = false"),
            "{what} takes pdfium-render's default features, which turns \
             `pdfium_latest` back on: {dep}"
        );
        assert!(
            dep.contains("pdfium_7881"),
            "{what} does not name a libpdfium ABI: {dep}"
        );
        assert!(
            dep.contains("thread_safe"),
            "{what} does not request `thread_safe`: {dep}"
        );
    }
}

fn s3_feature_comment(manifest: &str) -> String {
    let mut block: Vec<&str> = Vec::new();
    for line in manifest.lines() {
        let trimmed = line.trim_start();
        if let Some(body) = trimmed.strip_prefix('#') {
            block.push(body.trim());
        } else if trimmed.starts_with("s3") && trimmed.contains('=') {
            // The accumulated run of comment lines directly above the `s3`
            // feature line is its documentation.
            return block.join(" ");
        } else if !trimmed.is_empty() {
            // Any non-comment, non-`s3` line breaks the contiguous comment run.
            block.clear();
        }
    }
    String::new()
}

#[test]
fn s3_alias_comment_states_no_build_time_signal() {
    let manifest = read_core_manifest();
    let comment = s3_feature_comment(&manifest).to_ascii_lowercase();
    assert!(
        !comment.is_empty(),
        "the `s3` feature must carry an explanatory comment"
    );
    assert!(
        comment.contains("object-store-sink"),
        "the `s3` deprecation comment must point migrators at `object-store-sink`"
    );
    assert!(
        comment.contains("no build-time warning")
            || comment.contains("emits no")
            || comment.contains("cargo has no mechanism"),
        "the `s3` comment must make explicit that Cargo emits no deprecation \
         warning for the feature alias, so consumers get no automated signal (#391)"
    );
}
