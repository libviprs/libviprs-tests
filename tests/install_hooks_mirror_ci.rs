//! Guards that the pre-commit hook runs what each repo's CI lint job runs
//! (libviprs/libviprs#715).
//!
//! `tools/install-hooks.sh` says its pre-commit hook "Mirrors each repo's
//! `.github/workflows/ci.yml` Check & Lint job *exactly*, so a clean local
//! commit means a clean remote Check & Lint", and then asks a human to keep
//! the per-repo cargo step lists in lockstep by hand. That did not happen. The
//! core's list ran two of the five clippy passes CI runs, missing
//! `object-store-sink`, `svg` and `jxl`, and this repo's missed `jxl` while
//! spending a compile on `s3`, a deprecated alias for a cell the matrix
//! already covers under its real name.
//!
//! Every one of those per-feature passes exists because the default pass
//! compiles none of that code (#382, #500, #502, and libviprs-tests#55). So a
//! commit that breaks `src/svg.rs` passed the local hook and failed CI, which
//! is the exact failure the hook was written to prevent.
//!
//! The guard runs the generated hook with a recording stand-in for `cargo`
//! first on `PATH` and compares the invocations it actually makes against the
//! lint lines in that repo's `ci.yml`. Grepping either file would be worth
//! nothing, which is the standing lesson of libviprs/libviprs#695.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

mod common;
use common::cli::{cli_dir, require_cli};
use common::hooks::{Workspace, make_executable, repo_root};

/// A `cargo` that records its arguments and succeeds, so the hook runs every
/// step instead of stopping at the first one.
const RECORDING_CARGO: &str = r#"#!/bin/sh
printf 'cargo %s\n' "$*" >> "$LOCKSTEP_RECORD"
exit 0
"#;

/// The same for the one non-cargo lint step, which the hook invokes by
/// relative path rather than through `PATH`.
const RECORDING_PORTED_CELLS: &str = r#"#!/bin/sh
printf './tools/run_ported_cells.sh %s\n' "$*" >> "$LOCKSTEP_RECORD"
exit 0
"#;

/// Every lint command `ci.yml` runs, with the feature matrix expanded.
///
/// `cargo check` and `cargo build` lines are deliberately out. The comment
/// above the step lists already says clippy does `cargo check`'s work, and the
/// core's `cargo build --features s3` is there to prove a deprecated alias
/// still resolves, which the manifest settles and a whole extra build on every
/// commit does not earn.
fn ci_lint_commands(workflow: &str) -> BTreeSet<String> {
    let features: Vec<String> = workflow
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let list = line.strip_prefix("feature: [")?.strip_suffix(']')?;
            Some(
                list.split(',')
                    .map(|f| f.trim().to_string())
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect();

    let mut found = BTreeSet::new();
    for line in workflow.lines() {
        let line = line.trim();
        let Some(cmd) = line
            .strip_prefix("- run: ")
            .or_else(|| line.strip_prefix("run: "))
        else {
            continue;
        };
        let cmd = cmd.trim();
        let is_lint = cmd.starts_with("cargo clippy")
            || cmd.starts_with("cargo fmt")
            || (cmd.starts_with("./tools/run_ported_cells.sh") && cmd.contains("--clippy"));
        if !is_lint {
            continue;
        }
        if cmd.contains("${{ matrix.feature }}") {
            assert!(
                !features.is_empty(),
                "a workflow line uses ${{{{ matrix.feature }}}} but no job \
                 declares a `feature: [...]` list, so this guard cannot expand \
                 it: {cmd}"
            );
            for feature in &features {
                found.insert(cmd.replace("${{ matrix.feature }}", feature));
            }
        } else {
            found.insert(cmd.to_string());
        }
    }
    found
}

/// What the pre-commit hook `install-hooks.sh` writes for `repo` actually
/// runs, observed by putting a recorder in front of it.
fn hook_runs(ws: &Workspace, repo: &str) -> BTreeSet<String> {
    let repo_dir = ws.repo(repo);
    let hook = repo_dir.join(".git/hooks/pre-commit");
    assert!(
        hook.is_file(),
        "no pre-commit hook was installed into {repo}, so this guard would \
         pass on an empty recording"
    );

    let bin = ws.root.join("recorders").join(repo);
    std::fs::create_dir_all(&bin).expect("create the recorder directory");
    let cargo = bin.join("cargo");
    std::fs::write(&cargo, RECORDING_CARGO).expect("write the recording cargo");
    make_executable(&cargo);

    // The hook reaches this one by relative path, so it has to sit in the
    // stand-in repo rather than on PATH.
    let ported = repo_dir.join("tools/run_ported_cells.sh");
    std::fs::create_dir_all(ported.parent().expect("tools dir")).expect("create tools dir");
    std::fs::write(&ported, RECORDING_PORTED_CELLS).expect("write the recording ported cells");
    make_executable(&ported);

    let record = ws.root.join(format!("{repo}.record"));
    let _ = std::fs::remove_file(&record);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = Command::new("bash")
        .arg(&hook)
        .current_dir(&repo_dir)
        .env("PATH", path)
        .env("LOCKSTEP_RECORD", &record)
        .output()
        .expect("run the generated pre-commit hook");
    assert!(
        out.status.success(),
        "the pre-commit hook failed with every command stubbed to succeed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    std::fs::read_to_string(&record)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The workflow for a repo, read out of that repo's checkout.
fn workflow_of(root: &Path) -> String {
    let path = root.join(".github/workflows/ci.yml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn compare(repo: &str, ci: BTreeSet<String>, hook: BTreeSet<String>) {
    assert!(
        !ci.is_empty(),
        "found no lint commands in {repo}'s ci.yml, so this guard would pass \
         on a workflow that lints nothing"
    );

    let missing: Vec<&String> = ci.difference(&hook).collect();
    let extra: Vec<&String> = hook.difference(&ci).collect();

    assert!(
        missing.is_empty(),
        "{repo}'s CI lints these and the pre-commit hook does not:\n  {}\n\
         Each one is a commit that passes locally and fails remotely, which is \
         the whole reason the hook exists. Add them to the step list for \
         {repo} in tools/install-hooks.sh (#715).",
        missing
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        extra.is_empty(),
        "the pre-commit hook for {repo} runs these and CI does not:\n  {}\n\
         Either CI lost a check the hook still remembers, or the hook is \
         spending a compile on something nothing asks for. Reconcile them in \
         tools/install-hooks.sh (#715).",
        extra
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// This repo. Its own workflow is always here, so there is nothing to skip.
#[test]
fn the_hook_lints_this_repo_the_way_its_ci_does() {
    let ws = Workspace::new();
    compare(
        "libviprs-tests",
        ci_lint_commands(&workflow_of(&repo_root())),
        hook_runs(&ws, "libviprs-tests"),
    );
}

/// The core checkout. `libviprs = { path = "../libviprs" }` in this crate's
/// manifest means it is there whenever this suite can run at all, and the
/// Docker build context stages it with its `.github/` intact.
#[test]
fn the_hook_lints_the_core_the_way_its_ci_does() {
    let ws = Workspace::new();
    compare(
        "libviprs",
        ci_lint_commands(&workflow_of(&repo_root().join("../libviprs"))),
        hook_runs(&ws, "libviprs"),
    );
}

/// The cli is not laid down by the default `test` job or by the Docker gate,
/// so this one has to skip where it is absent. Skipping reads to `cargo test`
/// as a pass, so it follows the same false-green guard as the differential
/// cells: `VIPRS_REQUIRE_CLI=1`, set on the job that does lay the cli down,
/// turns the skip into a panic (`tests/common/cli.rs`).
#[test]
fn the_hook_lints_the_cli_the_way_its_ci_does() {
    let dir = cli_dir();
    let workflow = dir.join(".github/workflows/ci.yml");
    if !workflow.is_file() {
        assert!(
            !require_cli(),
            "VIPRS_REQUIRE_CLI=1 but there is no libviprs-cli workflow at {}, \
             so this guard would compare nothing and report a false green",
            workflow.display()
        );
        return;
    }

    let ws = Workspace::new();
    compare(
        "libviprs-cli",
        ci_lint_commands(&workflow_of(&dir)),
        hook_runs(&ws, "libviprs-cli"),
    );
}
