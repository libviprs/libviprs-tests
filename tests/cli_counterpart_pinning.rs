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
use common::workflows::{Workflow, read_workflow, workflow};

/// The job that builds `viprs` and runs the differential cells against it.
const CLI_JOB: &str = "cli-differential";

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
///
/// # Why this reads the step and not the file
///
/// It used to ask whether `VIPRS_REQUIRE_CLI: 1` appeared anywhere in
/// `ci.yml`, and `ci.yml` contains that line on the `hook-mirror` job as well.
/// Measured: delete it from the `cli-differential` step and this guard stays
/// green, carried by the other job's copy, while all 17 differential binaries
/// run somewhere they can skip every assertion and report a pass. A guard that
/// another job can satisfy is not a guard for this one.
#[test]
fn cli_differential_job_requires_the_cli_to_run() {
    let ci = workflow("ci.yml");
    let job = ci.job(CLI_JOB);
    let runner = job
        .steps
        .iter()
        .find(|s| s.run().contains("cargo test"))
        .unwrap_or_else(|| panic!("no step of `{CLI_JOB}` runs `cargo test`"));
    let set = runner.env_value("VIPRS_REQUIRE_CLI");
    assert_eq!(
        set,
        Some("1"),
        "the step of `{CLI_JOB}` that runs the differential cells sets \
         VIPRS_REQUIRE_CLI to {set:?}. It has to be 1, on this step, so a \
         failure to lay the CLI down hard-fails instead of skipping to a false \
         green (F1). The same line elsewhere in the file says nothing about \
         this job."
    );
}

/// The same edit the guard above now catches, applied to the real workflow, so
/// the claim above is a measurement rather than a description.
#[test]
fn the_require_variable_on_another_job_does_not_satisfy_this_one() {
    let text = read_workflow("ci.yml");
    let job_at = text
        .find(&format!("  {CLI_JOB}:"))
        .unwrap_or_else(|| panic!("no `{CLI_JOB}` job in ci.yml"));
    // The next job header, which is a line at exactly two spaces of indent. A
    // plain search for "\n  " finds this job's own keys, which are indented
    // further, and slices the job down to its first line.
    let next_job = text[job_at + 1..]
        .match_indices("\n  ")
        .find(|(i, _)| {
            text[job_at + 1 + i + 3..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace())
        })
        .map(|(i, _)| job_at + 1 + i)
        .unwrap_or(text.len());
    let job_text = &text[job_at..next_job];
    let stripped = job_text.replace("          VIPRS_REQUIRE_CLI: 1\n", "");
    assert_ne!(
        stripped, job_text,
        "the `{CLI_JOB}` job no longer carries `VIPRS_REQUIRE_CLI: 1` at the \
         indent this test edits, so the row below proves nothing"
    );
    let mutated = format!("{}{}{}", &text[..job_at], stripped, &text[next_job..]);
    assert!(
        mutated.contains("VIPRS_REQUIRE_CLI: 1"),
        "another job's copy has to survive the edit, or this test is measuring \
         a whole-file search rather than a job-scoped one"
    );

    let ci = Workflow::parse(&mutated);
    let runner = ci
        .job(CLI_JOB)
        .steps
        .iter()
        .find(|s| s.run().contains("cargo test"))
        .expect("the job still runs cargo test");
    assert_ne!(
        runner.env_value("VIPRS_REQUIRE_CLI"),
        Some("1"),
        "with the variable deleted from this job's step, a job-scoped reading \
         still found it, which means the reading is not job-scoped"
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
