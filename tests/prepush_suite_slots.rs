//! The pre-push hook for the acadsharp-rs-tests suite and its two engines
//! (acadsharp-rs-tests#26).
//!
//! The suite's gate is its own `tools/run-tests.sh`, and it takes the other two
//! trees by flag. So a push from the suite runs the pushed tree's script, and a
//! push from `acadsharp-rs` or `acadrust` runs the suite's script with the
//! pushed tree in that repo's slot, the way a core push runs this harness.
//! Measured the same way as the other hook guards: real installer, real hook, a
//! ref-update on stdin, and the stub gate reports what it was handed.

mod common;
use common::hooks::{Workspace, git, reported};

fn run_from(ws: &Workspace, repo: &str, tree: &std::path::Path) -> String {
    let before = git(tree, &["rev-parse", "HEAD"]).trim().to_string();
    let after = ws.commit_in(tree, "a change", &[("src/lib.rs", "// changed\n")]);
    ws.push(repo).from(tree).range(&before, &after).run()
}

fn path_of(out: &str, name: &str) -> String {
    reported(out, name)
}

#[test]
fn a_suite_push_runs_the_pushed_trees_own_gate_with_both_engines_named() {
    let ws = Workspace::new();
    let lane = ws.worktree("acadsharp-rs-tests", "suite-lane");
    ws.commit_in(
        &lane,
        "gate in lane",
        &[("tools/run-tests.sh", common::hooks::STUB_RUN_TESTS)],
    );
    common::hooks::make_executable(&lane.join("tools/run-tests.sh"));
    let out = run_from(&ws, "acadsharp-rs-tests", &lane);
    assert!(
        path_of(&out, "SCRIPT").ends_with("lanes/suite-lane/tools/run-tests.sh"),
        "a suite push must run the script it is pushing:\n{out}"
    );
    let args = reported(&out, "ARGS");
    for (flag, repo) in [
        ("--acadsharp-rs", "acadsharp-rs"),
        ("--acadrust", "acadrust"),
    ] {
        let want = format!("{flag} {}", ws.repo(repo).display());
        assert!(args.contains(&want), "missing `{want}` in `{args}`:\n{out}");
    }
}

#[test]
fn an_engine_push_runs_the_suites_gate_with_the_pushed_tree_in_its_slot() {
    let ws = Workspace::new();
    for (repo, flag, other_flag, other) in [
        ("acadsharp-rs", "--acadsharp-rs", "--acadrust", "acadrust"),
        ("acadrust", "--acadrust", "--acadsharp-rs", "acadsharp-rs"),
    ] {
        let lane = ws.worktree(repo, &format!("{repo}-lane"));
        let out = run_from(&ws, repo, &lane);
        let suite = ws.repo("acadsharp-rs-tests").join("tools/run-tests.sh");
        assert_eq!(
            path_of(&out, "SCRIPT"),
            suite.display().to_string(),
            "{repo}:\n{out}"
        );
        let args = reported(&out, "ARGS");
        let pushed = format!("{flag} {}", lane.display());
        assert!(
            args.contains(&pushed),
            "{repo}: want `{pushed}` in `{args}`:\n{out}"
        );
        let sibling = format!("{other_flag} {}", ws.repo(other).display());
        assert!(
            args.contains(&sibling),
            "{repo}: want `{sibling}` in `{args}`:\n{out}"
        );
    }
}

#[test]
fn an_engine_push_from_the_main_checkout_names_that_checkout() {
    let ws = Workspace::new();
    let out = run_from(&ws, "acadsharp-rs", &ws.repo("acadsharp-rs"));
    let args = reported(&out, "ARGS");
    let want = format!("--acadsharp-rs {}", ws.repo("acadsharp-rs").display());
    assert!(args.contains(&want), "want `{want}` in `{args}`:\n{out}");
}

#[test]
fn the_core_and_the_harness_still_get_no_arguments() {
    let ws = Workspace::new();
    for repo in ["libviprs", "libviprs-tests"] {
        let out = run_from(&ws, repo, &ws.repo(repo));
        assert_eq!(
            reported(&out, "ARGS"),
            "",
            "{repo} must run the harness bare:\n{out}"
        );
    }
}

#[test]
fn a_failing_suite_gate_refuses_the_push() {
    let ws = Workspace::new();
    let suite = ws.repo("acadsharp-rs-tests").join("tools/run-tests.sh");
    std::fs::write(&suite, "#!/bin/sh\necho moved-row-in-the-log\nexit 1\n").unwrap();
    let tree = ws.repo("acadrust");
    let before = git(&tree, &["rev-parse", "HEAD"]).trim().to_string();
    let after = ws.commit_in(&tree, "a change", &[("src/lib.rs", "// x\n")]);
    let (ok, text) = ws.push("acadrust").range(&before, &after).try_run();
    assert!(!ok, "a failing gate must abort the push:\n{text}");
    assert!(
        text.contains("moved-row-in-the-log"),
        "the gate's output must reach the pusher:\n{text}"
    );
}

#[test]
fn a_suite_push_hands_the_gate_the_remote_commit_as_the_verdicts_base() {
    let ws = Workspace::new();
    let tree = ws.repo("acadsharp-rs-tests");
    let before = git(&tree, &["rev-parse", "HEAD"]).trim().to_string();
    let after = ws.commit_in(&tree, "a change", &[("src/lib.rs", "// y\n")]);
    let out = ws
        .push("acadsharp-rs-tests")
        .range(&before, &after)
        .run();
    let args = reported(&out, "ARGS");
    assert!(
        args.contains(&format!("--verdicts-base {before}")),
        "the remote commit must reach the gate as --verdicts-base: `{args}`\n{out}"
    );
}

#[test]
fn a_new_branch_gets_no_verdicts_base_and_engines_never_do() {
    let ws = Workspace::new();
    let tree = ws.repo("acadsharp-rs-tests");
    let after = ws.commit_in(&tree, "a change", &[("src/lib.rs", "// z\n")]);
    let zero = "0".repeat(40);
    let out = ws.push("acadsharp-rs-tests").range(&zero, &after).run();
    assert!(
        !reported(&out, "ARGS").contains("--verdicts-base"),
        "a new branch has no remote commit:\n{out}"
    );

    let engine = ws.repo("acadrust");
    let before = git(&engine, &["rev-parse", "HEAD"]).trim().to_string();
    let after = ws.commit_in(&engine, "a change", &[("src/lib.rs", "// w\n")]);
    let out = ws.push("acadrust").range(&before, &after).run();
    assert!(
        !reported(&out, "ARGS").contains("--verdicts-base"),
        "only a suite push carries a verdicts base:\n{out}"
    );
}
