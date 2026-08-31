//! Guards that the LOCALCI pre-commit hook (`write_pre_commit` in
//! `tools/install-hooks.sh`) resolves `REPO_DIR` against the tree actually
//! being committed in, not against the shared `.git/hooks` directory a
//! repo's main checkout and every one of its linked worktrees have in
//! common (libviprs/libviprs#684).
//!
//! This is the same bug `install_pre_push` already dodges for the pre-push
//! hook, via `git rev-parse --show-toplevel`. The LOCALCI branch shipped
//! with `REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"` instead, which
//! resolves from the hook's own path rather than from the tree it is
//! running in, so it always pointed at the main checkout: a commit made
//! inside a linked worktree silently ran `local-ci.py` against the wrong
//! tree, with no error.
//!
//! Mirrors `tests/prepush_gate_tests_the_pushed_tree.rs`: install the hooks
//! with the real installer, put a recording stand-in in place of
//! `tools/local-ci.py` so nothing here asserts on the hook's text
//! (libviprs/libviprs#695), create a linked worktree with
//! `Workspace::worktree`, and run the installed hook the way git does — the
//! hook file always lives under the main checkout's shared `.git/hooks`,
//! while the process's working directory is whichever tree is actually
//! being committed in.

use std::path::Path;
use std::process::Command;

mod common;
use common::hooks::{Workspace, make_executable};

/// A stand-in for `tools/local-ci.py` that records its own resolved path
/// (`sys.argv[0]`, made absolute) and its arguments, so a push can be judged
/// by what the hook actually invoked rather than by reading the hook's text.
/// Carries a real shebang so it also runs correctly when invoked directly by
/// path under the pre-fix hook, isolating these tests to the REPO_DIR bug
/// rather than entangling them with the separate executable-bit fix.
const RECORDING_LOCALCI: &str = r#"#!/usr/bin/env python3
import os
import sys

with open(os.environ["LOCALCI_RECORD"], "a") as f:
    f.write("ARGV0=" + os.path.abspath(sys.argv[0]) + "\n")
    f.write("ARGS=" + " ".join(sys.argv[1:]) + "\n")
"#;

/// Run the pre-commit hook installed into `repo`'s main checkout, with the
/// process's working directory set to `cwd`. This is exactly the shape git
/// itself uses: the hook file always lives in the main checkout's shared
/// `.git/hooks`, and the working directory is whichever tree is actually
/// being committed in — the main checkout itself, or one of its linked
/// worktrees.
fn run_pre_commit(ws: &Workspace, repo: &str, cwd: &Path, record: &Path) -> String {
    let hook = ws.repo(repo).join(".git/hooks/pre-commit");
    let _ = std::fs::remove_file(record);

    let out = Command::new("bash")
        .arg(&hook)
        .current_dir(cwd)
        .env("LOCALCI_RECORD", record)
        .output()
        .expect("run the installed pre-commit hook");
    assert!(
        out.status.success(),
        "the pre-commit hook failed running from {}:\n{}{}",
        cwd.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    std::fs::read_to_string(record).unwrap_or_default()
}

/// The `ARGV0=` line out of what a hook run recorded, or a panic naming what
/// was printed instead — the hook running at all but `local-ci.py` never
/// executing is itself a finding worth seeing.
fn recorded_argv0(out: &str) -> String {
    out.lines()
        .find_map(|l| l.strip_prefix("ARGV0="))
        .unwrap_or_else(|| {
            panic!("local-ci.py never ran, so REPO_DIR resolved to nothing useful:\n{out}")
        })
        .to_string()
}

/// #684, for the pre-commit hook this time rather than the pre-push one:
/// `REPO_DIR` has to name the tree actually being committed in, whether
/// that is the main checkout or one of its linked worktrees.
#[test]
fn the_localci_hook_runs_local_ci_py_from_the_tree_being_committed_in() {
    let ws = Workspace::new();

    // Give "libviprs" a tools/local-ci.py so write_pre_commit takes the
    // LOCALCI branch instead of the hardcoded cargo list, then re-run the
    // installer so the new hook lands. Executable, so this is a pure test of
    // REPO_DIR resolution and not entangled with the executable-bit fix
    // covered separately in tests/install_hooks_localci_selection.rs.
    let local_ci = ws.repo("libviprs").join("tools/local-ci.py");
    ws.commit(
        "libviprs",
        "add a recording stand-in for local-ci.py",
        &[("tools/local-ci.py", RECORDING_LOCALCI)],
    );
    make_executable(&local_ci);
    ws.install_hooks();

    let hook_text = std::fs::read_to_string(ws.repo("libviprs").join(".git/hooks/pre-commit"))
        .expect("read the installed pre-commit hook");
    assert!(
        hook_text.contains("local-ci.py"),
        "libviprs now ships tools/local-ci.py, so its pre-commit hook must \
         defer to it instead of the hardcoded cargo list. Installed hook:\n{hook_text}"
    );

    let lane = ws.worktree("libviprs", "lane");
    let main = ws.repo("libviprs");

    let cases: &[(&Path, &str)] = &[
        (
            &main,
            "a commit in the main checkout must run local-ci.py from the main checkout",
        ),
        (
            &lane,
            "a commit in a linked worktree must run local-ci.py from that \
             worktree, not from the main checkout whose .git/hooks directory \
             the hook file actually lives in and is shared by every worktree \
             (libviprs/libviprs#684)",
        ),
    ];

    for (i, (cwd, why)) in cases.iter().enumerate() {
        let record = ws.root.join(format!("localci-{i}.record"));
        let out = run_pre_commit(&ws, "libviprs", cwd, &record);
        let argv0 = recorded_argv0(&out);

        let expected = cwd.join("tools/local-ci.py");
        assert_eq!(
            argv0,
            expected.display().to_string(),
            "the pre-commit hook ran local-ci.py from {argv0}, not from {}, \
             because {why}. Fix: derive REPO_DIR from \
             `git rev-parse --show-toplevel`, the way install_pre_push \
             already does for the pre-push hook, rather than from \
             `$(dirname \"$0\")/../..`, which always resolves to wherever the \
             hook file itself sits. The hook said:\n{out}",
            expected.display()
        );
    }
}

/// The same hook, run against a repo with more than one linked worktree, has
/// to tell them apart from each other too — not just from the main checkout.
/// A fix that special-cased "not the main checkout" without actually asking
/// git which tree is live would still be wrong here.
#[test]
fn the_localci_hook_tells_two_worktrees_of_the_same_repo_apart() {
    let ws = Workspace::new();

    let local_ci = ws.repo("libviprs").join("tools/local-ci.py");
    ws.commit(
        "libviprs",
        "add a recording stand-in for local-ci.py",
        &[("tools/local-ci.py", RECORDING_LOCALCI)],
    );
    make_executable(&local_ci);
    ws.install_hooks();

    let lane_a = ws.worktree("libviprs", "lane-a");
    let lane_b = ws.worktree("libviprs", "lane-b");

    for (i, lane) in [&lane_a, &lane_b].into_iter().enumerate() {
        let record = ws.root.join(format!("localci-lane-{i}.record"));
        let out = run_pre_commit(&ws, "libviprs", lane, &record);
        let argv0 = recorded_argv0(&out);

        let expected = lane.join("tools/local-ci.py");
        assert_eq!(
            argv0,
            expected.display().to_string(),
            "a commit in {} ran local-ci.py from {argv0} instead of from that \
             same worktree. The hook said:\n{out}",
            lane.display()
        );
    }
}
