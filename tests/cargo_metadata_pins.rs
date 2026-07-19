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

/// Extract the `[patch.crates-io]` pdfium-render override line from this
/// crate's manifest — the git-sourced declaration, not the crates-io
/// `[dependencies]` entry of the same name.
fn tests_pdfium_patch_line(manifest: &str) -> String {
    for line in manifest.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with("pdfium-render") && trimmed.contains("git") {
            return trimmed.to_string();
        }
    }
    String::new()
}

/// Collapse every `#`-comment line in the manifest into one whitespace-
/// normalised string. Comment bodies wrap across many physical lines with a
/// `# ` prefix; folding them lets a single substring check match a phrase that
/// spans line breaks without being brittle about exact wrapping.
fn folded_comments(manifest: &str) -> String {
    let mut words: Vec<&str> = Vec::new();
    for line in manifest.lines() {
        let trimmed = line.trim_start();
        if let Some(body) = trimmed.strip_prefix('#') {
            words.extend(body.split_whitespace());
        }
    }
    words.join(" ")
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

/// A 40-char lowercase hex commit SHA, e.g. the value of a git `rev` pin.
fn is_full_commit_sha(v: &str) -> bool {
    v.len() == 40 && v.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Pull the `rev = "..."` value out of the pdfium dependency line.
fn pdfium_rev(dep_line: &str) -> Option<String> {
    let after = dep_line.split("rev").nth(1)?;
    // Skip `=`, whitespace and the opening quote, then read to the close quote.
    let after = after.trim_start().strip_prefix('=')?.trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_string())
}

// ---------------------------------------------------------------------------
// #375 — the pin must be an immutable rev, and the comment must state the
// reachability precondition rather than claim unconditional immutability.
// ---------------------------------------------------------------------------

/// The fork dependency is pinned to a full immutable commit `rev`, never a
/// bare `branch`. A `branch` pin re-resolves on every fetch and lets a
/// force-push silently swap the dependency, so a regression to `branch = ...`
/// must fail this guard.
#[test]
fn pdfium_dep_is_pinned_to_immutable_rev() {
    let manifest = read_core_manifest();
    let dep = pdfium_dep_line(&manifest);

    assert!(
        !dep.contains("branch"),
        "pdfium-render must not be pinned to a mutable `branch`; found: {dep}"
    );
    let rev = pdfium_rev(&dep)
        .unwrap_or_else(|| panic!("pdfium-render dependency has no `rev = \"...\"` pin: {dep}"));
    assert!(
        is_full_commit_sha(&rev),
        "pdfium-render `rev` must be a full 40-char commit SHA (immutable pin), got {rev:?}"
    );
}

/// The pin comment must not oversell durability: it has to state the
/// reachability precondition (GitHub only serves a bare SHA that stays
/// reachable from an advertised ref), so a cold-cache fetch failure after the
/// branch is force-pushed past the commit is documented, not a surprise.
#[test]
fn pdfium_pin_comment_states_reachability_precondition() {
    let comments = folded_comments(&read_core_manifest()).to_ascii_lowercase();
    assert!(
        comments.contains("reachable"),
        "the pdfium pin comment must state the reachability precondition \
         (the immutable-rev pin only holds while the commit stays reachable \
         from an advertised ref) instead of claiming unconditional immutability"
    );
}

// ---------------------------------------------------------------------------
// #376 / #378 — the lead / top-of-block provenance comment must not claim the
// dependency is "pinned to its libviprs/integration branch", which the rev pin
// makes false. The branch may only appear as named provenance.
// ---------------------------------------------------------------------------

#[test]
fn pdfium_provenance_comment_does_not_claim_branch_pin() {
    let comments = folded_comments(&read_core_manifest());
    assert!(
        !comments.contains("pinned to its `libviprs/integration` branch"),
        "the provenance comment must not claim the dependency is `pinned to its \
         libviprs/integration branch`; the actual pin is an immutable rev and \
         the branch is only provenance (#376/#378)"
    );
    // The branch must still be documented as provenance so the pin's origin is
    // traceable when advancing the fork.
    assert!(
        comments.contains("provenance") && comments.contains("libviprs/integration"),
        "the comment must still name `libviprs/integration` as the pin's provenance"
    );
}

// ---------------------------------------------------------------------------
// #377 — the descoped published-crate soundness leg must not point at the
// already-closed issue #149 as its live tracker; it must name the descoping
// and route renewed work to an open tracker.
// ---------------------------------------------------------------------------

#[test]
fn pdfium_soundness_divergence_names_descope_and_open_tracker() {
    let comments = folded_comments(&read_core_manifest());
    assert!(
        comments.contains("descoped"),
        "the comment must state that PR #333 descoped the published-crate \
         soundness leg (#377), not silently defer it to a closed tracker"
    );
    assert!(
        comments.contains("#344"),
        "renewed published-crate soundness work must be routed to the open \
         follow-up epic #344, since #149 is closed (#377)"
    );
}

// ---------------------------------------------------------------------------
// #391 — the `s3` feature is deprecated only by comment; Cargo emits no
// warning. The comment must make that absence of a build-time signal explicit
// and unambiguously point migrators at `object-store-sink`.
// ---------------------------------------------------------------------------

/// Locate the comment block immediately preceding the `s3 = [...]` feature.
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

// ---------------------------------------------------------------------------
// tests-crate `[patch.crates-io]` honesty — this crate's own override pins the
// MUTABLE `libviprs/integration` branch, not the core crate's immutable `rev`.
// The comment must state that plainly (so "mirror"/"lockstep" is not read as a
// claim of matching immutability) and route the immutable-rev follow-up to an
// open tracker, mirroring the honesty guards applied to the core manifest.
// ---------------------------------------------------------------------------

/// The override really is a `branch` pin today — guard that actual state so the
/// prose below is checked against reality, not an aspirational description.
#[test]
fn tests_pdfium_patch_is_a_branch_pin() {
    let manifest = read_tests_manifest();
    let patch = tests_pdfium_patch_line(&manifest);
    assert!(
        !patch.is_empty(),
        "no git-sourced `pdfium-render` `[patch.crates-io]` override found in tests Cargo.toml"
    );
    assert!(
        patch.contains("branch"),
        "the tests `[patch.crates-io]` override is expected to be a `branch` pin; \
         if it was upgraded to an immutable `rev`, update the honesty comment and \
         this guard together. Found: {patch}"
    );
}

/// The patch comment must not let "mirror"/"lockstep" imply the tests override
/// shares the core crate's immutable pin: it has to state the override is a
/// MUTABLE branch pin and route the immutable-rev upgrade to the open #344
/// follow-up.
#[test]
fn tests_pdfium_patch_comment_admits_mutable_branch_and_routes_follow_up() {
    let comments = folded_comments(&read_tests_manifest()).to_ascii_lowercase();
    assert!(
        comments.contains("mutable"),
        "the tests `[patch.crates-io]` comment must state that it pins a MUTABLE \
         branch (not the core crate's immutable rev)"
    );
    assert!(
        comments.contains("immutable") && comments.contains("rev"),
        "the tests patch comment must contrast the mutable branch pin against the \
         core crate's immutable `rev`, so the divergence is explicit"
    );
    assert!(
        comments.contains("#344"),
        "the immutable-rev upgrade for the tests patch must be routed to the open \
         fork-pinning follow-up epic #344"
    );
}
