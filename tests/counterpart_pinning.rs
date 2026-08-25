//! Guards the cross-repo pinning invariants from issue #58.
//!
//! CI used to stitch the two repos together by branch name with a silent
//! fallback to `main`, so a PR could go green against the wrong revision of its
//! counterpart, and the libvips reference fixtures had no pinned fetch step at
//! all. These tests fail loudly if either guarantee regresses. They read only
//! repository files, so they run under the default `cargo test` with no
//! network, no sibling checkout, and no feature flags.

use std::path::{Path, PathBuf};

mod common;
use common::workflows::read_workflow;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// A full git object name: exactly 40 lowercase hex digits.
fn is_full_sha(token: &str) -> bool {
    token.len() == 40 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True if any whitespace/punctuation-delimited token in `text` is a full SHA.
fn has_full_sha(text: &str) -> bool {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .any(is_full_sha)
}

#[test]
fn counterpart_rev_pins_a_full_sha() {
    let raw = read("COUNTERPART_REV");
    let payload = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .expect("COUNTERPART_REV must name a revision");
    assert!(
        is_full_sha(payload),
        "COUNTERPART_REV must pin a 40-char commit SHA, not a branch or tag (found {payload:?})"
    );
}

#[test]
fn ci_checks_out_pinned_counterpart_with_no_branch_fallback() {
    // The reusable action drives the checkout from the pinned rev.
    let action = read(".github/actions/clone-counterpart/action.yml");
    assert!(
        action.contains("COUNTERPART_REV"),
        "clone-counterpart action must read the pinned rev from COUNTERPART_REV"
    );
    assert!(
        action.contains("FETCH_HEAD"),
        "clone-counterpart action must check out the exact fetched commit"
    );

    // `read_workflow` resolves ci.yml wherever it currently lives; the
    // composite actions it calls have always stayed under `.github/actions/`,
    // so the `uses:` path below is unchanged by either CI move.
    let ci = read_workflow("ci.yml");
    assert!(
        ci.contains("./.github/actions/clone-counterpart"),
        "every job needing the sibling must clone it via the pinned action"
    );
    // The old branch-name clone and its `|| git clone ... main` fallback are gone.
    assert!(
        !ci.contains("--branch"),
        "ci.yml must not clone the counterpart by branch name"
    );
    assert!(
        !ci.contains("|| git clone"),
        "ci.yml must not fall back to a default-branch clone"
    );
}

#[test]
fn reference_suite_fetch_is_scripted_and_pinned() {
    let rel = "tools/fetch_reference_suite.sh";
    let script = read(rel);
    assert!(
        has_full_sha(&script),
        "{rel} must pin an explicit upstream commit SHA"
    );
    assert!(
        script.contains("libvips-reference-tests/test-suite"),
        "{rel} must populate tmp/libvips-reference-tests/test-suite"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(repo_root().join(rel))
            .expect("stat fetch script")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "{rel} must be executable");
    }
}

#[test]
fn ci_wires_the_pinned_fetch_step() {
    let ci = read_workflow("ci.yml");
    assert!(
        ci.contains("tools/fetch_reference_suite.sh"),
        "ci.yml must fetch reference fixtures via the pinned script as a CI step"
    );
}
