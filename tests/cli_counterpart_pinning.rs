//! Guards the SECOND cross-repo pin (CLI_COUNTERPART_REV → libviprs-cli),
//! the sibling of `counterpart_pinning.rs` (which guards the core pin).
//!
//! The CLI-differential harness (`tests/common/cli.rs`) builds and runs the
//! `viprs` binary; in CI it must build the CLI at a KNOWN revision, cloned with
//! the same no-branch-guessing / no-silent-fallback discipline the core pin
//! uses (issue #58). These tests read only repository files, so they run under
//! the default `cargo test` with no network, no sibling checkout, and no CLI
//! build.

use std::path::{Path, PathBuf};

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

#[test]
fn cli_counterpart_rev_pins_a_full_sha() {
    let raw = read("CLI_COUNTERPART_REV");
    let payload = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .expect("CLI_COUNTERPART_REV must name a revision");
    assert!(
        is_full_sha(payload),
        "CLI_COUNTERPART_REV must pin a 40-char commit SHA, not a branch or tag (found {payload:?})"
    );
}

#[test]
fn ci_clones_pinned_cli_counterpart_with_no_branch_fallback() {
    let action = read(".github/actions/clone-cli-counterpart/action.yml");
    assert!(
        action.contains("CLI_COUNTERPART_REV"),
        "clone-cli-counterpart action must read the pinned rev from CLI_COUNTERPART_REV"
    );
    assert!(
        action.contains("FETCH_HEAD"),
        "clone-cli-counterpart action must check out the exact fetched commit"
    );

    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("./.github/actions/clone-cli-counterpart"),
        "the CLI-differential job must clone the CLI via the pinned action"
    );
    // No branch-name clone and no default-branch fallback (issue #58).
    assert!(
        !ci.contains("--branch"),
        "ci.yml must not clone a counterpart by branch name"
    );
    assert!(
        !ci.contains("|| git clone"),
        "ci.yml must not fall back to a default-branch clone"
    );
}

#[test]
fn cli_differential_job_runs_the_morphology_cell() {
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("cargo test --test cli_morphology_diff"),
        "ci.yml must run the morphology differential cell in the cli-differential job"
    );
}
