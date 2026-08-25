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

mod common;
use common::workflows::read_workflow;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// A full git object name: exactly 40 **lowercase** hex digits.
///
/// git emits object names in lowercase; requiring lowercase here keeps this
/// side's acceptance identical to the clone action's (which reads the first
/// non-comment line and `git rev-parse HEAD == pin` asserts it byte-for-byte, so
/// an uppercased pin would compare unequal there).
fn is_full_sha(token: &str) -> bool {
    token.len() == 40
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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

    // The action asserts the checked-out HEAD equals the pin (F10), so a fetch
    // that resolved to the wrong commit fails loudly rather than testing stale
    // sources.
    assert!(
        action.contains("rev-parse HEAD"),
        "clone-cli-counterpart action must assert `git rev-parse HEAD` equals the pin"
    );

    // `read_workflow` resolves ci.yml wherever it currently lives; the
    // composite actions it calls have always stayed under `.github/actions/`,
    // so the `uses:` path below is unchanged by either CI move.
    let ci = read_workflow("ci.yml");
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

/// The `cli-differential` job must hard-fail rather than skip to a false green
/// when it cannot actually run `viprs` (F1): it sets `VIPRS_REQUIRE_CLI: 1`,
/// which makes the harness panic instead of skipping when the CLI is absent.
#[test]
fn cli_differential_job_requires_the_cli_to_run() {
    let ci = read_workflow("ci.yml");
    assert!(
        ci.contains("VIPRS_REQUIRE_CLI: 1") || ci.contains("VIPRS_REQUIRE_CLI: \"1\""),
        "the cli-differential job must set VIPRS_REQUIRE_CLI=1 so a failure to lay \
         the CLI down hard-fails instead of skipping to a false green (F1)"
    );
}

/// Every `tests/cli_*_diff.rs` differential binary must be wired into the
/// `cli-differential` job's `cargo test` invocation (F2): a future family that
/// adds a `cli_<family>_diff.rs` file but forgets its `--test` line would never
/// run in CI and its cell would be silently uncovered.
///
/// We chose the **guard-test** option over `cargo test --tests`: this repo's
/// default test binaries include heavy suites (`ported_*`, `phase3_*`) that
/// `--tests` would pull into the CLI-differential job, duplicating the main
/// `test` job and lengthening it needlessly. The guard keeps the job scoped to
/// the differential cells while making a forgotten wiring a hard CI failure.
#[test]
fn cli_differential_job_runs_every_diff_binary() {
    let ci = read_workflow("ci.yml");
    let tests_dir = repo_root().join("tests");
    let mut diff_bins: Vec<String> = std::fs::read_dir(&tests_dir)
        .expect("read tests/ dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("cli_") && n.ends_with("_diff.rs"))
        .map(|n| n.trim_end_matches(".rs").to_string())
        .collect();
    diff_bins.sort();
    assert!(
        !diff_bins.is_empty(),
        "expected at least one tests/cli_*_diff.rs differential binary"
    );
    for bin in &diff_bins {
        assert!(
            ci.contains(&format!("--test {bin}")),
            "cli-differential job in ci.yml must run `cargo test --test {bin}` \
             (found the diff binary tests/{bin}.rs but no matching --test line)"
        );
    }
}
