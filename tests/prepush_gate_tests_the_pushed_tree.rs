//! Guards that the local pre-push gate tests the tree being pushed
//! (libviprs/libviprs#684).
//!
//! `tools/run-tests.sh` used to derive the trees under test from its own
//! location, so `$WORKSPACE_ROOT/libviprs` was the answer no matter which
//! working tree the push came from. Epic #520 runs every lane in a git
//! worktree, and a repo shares one hooks directory between its main checkout
//! and all of its linked worktrees, so the gate compiled `main` on every lane
//! push and could not fail on the author's own changes. A green pre-push read
//! as "my branch passes" and meant nothing of the kind.
//!
//! Two properties have to hold together, and neither is visible from the
//! other side, which is why both are driven here:
//!
//!   * the hook has to *find* the pushed tree, and it cannot do that from
//!     `$0` (shared hooks directory) or by walking up from `.git` (in a
//!     worktree that is a file holding a `gitdir:` pointer, not a directory).
//!   * `run-tests.sh` has to *accept* it. It did not have a parameter for it
//!     at all, so passing one from the hook was not enough on its own.
//!
//! Everything about the hook here is measured by running it. The guards that
//! stood here before asserted `hook.contains("git rev-parse --show-toplevel")`
//! and `hook.contains("unset GIT_DIR")`, and both of those stay green when the
//! line they name is deleted and the comment above it is left behind, which is
//! libviprs/libviprs#695. So instead: install the hooks with the real
//! installer, push at them from a linked worktree with a ref-update on stdin,
//! and read the trees off what the suite was handed.

use std::process::Command;

mod common;
use common::hooks::{Workspace, repo_root, reported};

/// #684, end to end and per push shape. `run-tests.sh` falls back to the
/// siblings of wherever the script itself sits, and for a worktree push that
/// script is inside the worktree, whose neighbours are other lanes rather than
/// the workspace, so both slots have to be named on every push and not just
/// the one being pushed.
#[test]
fn the_gate_is_handed_the_tree_being_pushed_and_its_sibling() {
    let ws = Workspace::new();

    let core_lane = ws.worktree("libviprs", "core-lane");
    let harness_lane = ws.worktree("libviprs-tests", "harness-lane");

    let cases: &[(&str, Option<&std::path::Path>, &str, &str)] = &[
        (
            "libviprs",
            None,
            "libviprs",
            "a core push from the main checkout gates on the main checkout",
        ),
        (
            "libviprs",
            Some(&core_lane),
            "lanes/core-lane",
            "a core push from a lane worktree must gate on the lane, not on \
             the main checkout whose hooks directory ran the hook",
        ),
        (
            "libviprs-tests",
            None,
            "libviprs-tests",
            "a harness push from the main checkout gates on the main checkout",
        ),
        (
            "libviprs-tests",
            Some(&harness_lane),
            "lanes/harness-lane",
            "a harness push from a lane worktree must gate on the lane",
        ),
    ];

    for (repo, from, expected_rel, why) in cases {
        let tree = from
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| ws.repo(repo));

        let before = common::hooks::git(&tree, &["rev-parse", "HEAD"])
            .trim()
            .to_string();
        // Something no skip list can call inert, so the gate reaches the suite.
        let after = ws.commit_in(
            &tree,
            "a change the suite can see",
            &[("src/lib.rs", "// changed\n")],
        );

        let out = ws
            .push(repo)
            .from(&tree)
            .range(&before, &after)
            .with_git_env()
            .run();

        let expected = ws.root.join(expected_rel);
        let (pushed_slot, sibling_slot, sibling_rel) = match *repo {
            "libviprs" => ("LIBVIPRS_DIR", "LIBVIPRS_TESTS_DIR", "libviprs-tests"),
            _ => ("LIBVIPRS_TESTS_DIR", "LIBVIPRS_DIR", "libviprs"),
        };

        assert_eq!(
            reported(&out, pushed_slot),
            expected.display().to_string(),
            "{pushed_slot} named the wrong tree, because {why}. The hook said:\n{out}"
        );
        assert_eq!(
            reported(&out, sibling_slot),
            ws.root.join(sibling_rel).display().to_string(),
            "{sibling_slot} was not pinned to the workspace sibling on a {repo} \
             push, so run-tests.sh is left to infer it from wherever the script \
             it picked happens to sit (#684). The hook said:\n{out}"
        );
    }
}

/// A push that changes the harness has to gate through the harness it is
/// changing, not through the copy in the main checkout. Finding the tree is
/// only half of #684; running the pushed tree's script is the other half.
#[test]
fn a_harness_push_runs_the_suite_script_it_is_pushing() {
    let ws = Workspace::new();
    let lane = ws.worktree("libviprs-tests", "harness-lane");

    let before = common::hooks::git(&lane, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let after = ws.commit_in(
        &lane,
        "a harness change",
        &[("tests/something.rs", "// changed\n")],
    );

    let out = ws
        .push("libviprs-tests")
        .from(&lane)
        .range(&before, &after)
        .with_git_env()
        .run();

    assert_eq!(
        reported(&out, "SCRIPT"),
        lane.join("tools/run-tests.sh").display().to_string(),
        "the gate ran a run-tests.sh from outside the tree being pushed, so a \
         change to the suite is gated by the suite it is replacing (#684). \
         The hook said:\n{out}"
    );
}

/// git hands every hook a `GIT_DIR`, and in a worktree it points at that
/// worktree's admin directory. It beats both `git -C` and the working
/// directory, so a `git` call made downstream answers for the repository doing
/// the pushing whatever directory it was aimed at. The first run of the fixed
/// hook reported the pushing branch's HEAD as the revision of an unrelated
/// tree, which is #684 wearing a different hat: a banner that names the wrong
/// commit is no better than a gate that builds the wrong one.
#[test]
fn the_gate_drops_the_git_environment_it_inherited() {
    let ws = Workspace::new();
    let lane = ws.worktree("libviprs-tests", "harness-lane");

    let before = common::hooks::git(&lane, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    let after = ws.commit_in(&lane, "a harness change", &[("tests/x.rs", "// c\n")]);

    let out = ws
        .push("libviprs-tests")
        .from(&lane)
        .range(&before, &after)
        .with_git_env()
        .run();

    for leaked in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_PREFIX",
        "GIT_QUARANTINE_PATH",
    ] {
        assert_eq!(
            reported(&out, leaked),
            "unset",
            "the gate handed {leaked} through to the suite. git exports it into \
             hooks and it wins over `git -C`, so everything the suite asks git \
             answers for the pushing repository rather than for the tree it was \
             handed (#684). The hook said:\n{out}"
        );
    }
}

/// The behavioural half on the `run-tests.sh` side: hand it a tree and it must
/// say, before it builds anything, that it is going to build that tree.
/// `--plan` exists so this is answerable without Docker, and so a human can
/// ask the same question from a worktree in under a second.
#[test]
fn run_tests_plan_reports_the_tree_it_was_handed() {
    let core = tempfile::tempdir().expect("temp dir for a stand-in core crate");
    let core_path = core.path().canonicalize().expect("canonical temp path");
    std::fs::write(
        core_path.join("Cargo.toml"),
        "[package]\nname = \"libviprs\"\nversion = \"0.0.0\"\n",
    )
    .expect("write stand-in Cargo.toml");

    let script = repo_root().join("tools/run-tests.sh");
    let root = repo_root();

    let by_flag = Command::new("bash")
        .arg(&script)
        .arg("--plan")
        .arg("--libviprs")
        .arg(&core_path)
        .arg("--libviprs-tests")
        .arg(&root)
        .output()
        .expect("run tools/run-tests.sh --plan");
    assert!(
        by_flag.status.success(),
        "`run-tests.sh --plan` failed: {}",
        String::from_utf8_lossy(&by_flag.stderr)
    );
    let by_flag = String::from_utf8_lossy(&by_flag.stdout).to_string();

    let by_env = Command::new("bash")
        .arg(&script)
        .arg("--plan")
        .env("LIBVIPRS_DIR", &core_path)
        .env("LIBVIPRS_TESTS_DIR", &root)
        .output()
        .expect("run tools/run-tests.sh --plan with the env form");
    assert!(
        by_env.status.success(),
        "`run-tests.sh --plan` with LIBVIPRS_DIR set failed: {}",
        String::from_utf8_lossy(&by_env.stderr)
    );
    let by_env = String::from_utf8_lossy(&by_env.stdout).to_string();

    let wanted = core_path.display().to_string();
    for (form, plan) in [("--libviprs", &by_flag), ("LIBVIPRS_DIR", &by_env)] {
        assert!(
            plan.contains(&wanted),
            "run-tests.sh ignored the core tree passed by {form}. It planned:\n{plan}"
        );
    }
}

/// The default has to keep working, because the sibling layout is what CI,
/// the README and every by-hand invocation use. #684 is about the default
/// being the *only* answer, not about the default being wrong.
#[test]
fn run_tests_still_defaults_to_the_sibling_layout() {
    let script = repo_root().join("tools/run-tests.sh");
    let out = Command::new("bash")
        .arg(&script)
        .arg("--plan")
        .env_remove("LIBVIPRS_DIR")
        .env_remove("LIBVIPRS_TESTS_DIR")
        .output()
        .expect("run tools/run-tests.sh --plan with no overrides");
    assert!(
        out.status.success(),
        "`run-tests.sh --plan` failed with no overrides: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan = String::from_utf8_lossy(&out.stdout);

    // `libviprs = { path = "../libviprs" }` in this crate's manifest means the
    // sibling is present whenever this test can run at all. Both sides are
    // physical paths: `canonicalize` here, `pwd -P` in the script. A logical
    // `pwd` over there would print the symlink a workspace was reached through
    // and this would go red on a layout that works perfectly well.
    let sibling = repo_root()
        .join("../libviprs")
        .canonicalize()
        .expect("the core crate sits at ../libviprs; this suite could not have compiled otherwise");
    assert!(
        plan.contains(&sibling.display().to_string()),
        "with nothing passed, run-tests.sh must still plan the sibling core \
         checkout at {}. It planned:\n{plan}",
        sibling.display()
    );
}

/// The same leak as `the_gate_drops_the_git_environment_it_inherited`, from
/// the other side: even with a `GIT_DIR` set, `run-tests.sh` must describe the
/// tree it was handed rather than the repository that exported it.
#[test]
fn run_tests_does_not_report_the_pushing_repo_as_another_tree() {
    let core = tempfile::tempdir().expect("temp dir for a stand-in core crate");
    let core_path = core.path().canonicalize().expect("canonical temp path");
    std::fs::write(
        core_path.join("Cargo.toml"),
        "[package]\nname = \"libviprs\"\nversion = \"0.0.0\"\n",
    )
    .expect("write stand-in Cargo.toml");

    let mut cmd = Command::new("bash");
    cmd.arg(repo_root().join("tools/run-tests.sh"))
        .arg("--plan")
        .arg("--libviprs")
        .arg(&core_path)
        .arg("--libviprs-tests")
        .arg(repo_root());

    // Reproduce the hook's environment where there is one to reproduce. Inside
    // the container the build context carries no `.git`, so there is no git
    // dir to inherit and the assertion below simply holds for the plainer
    // reason; on a developer checkout it is the real thing.
    if let Ok(out) = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["rev-parse", "--absolute-git-dir"])
        .output()
    {
        if out.status.success() {
            let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !dir.is_empty() {
                cmd.env("GIT_DIR", dir);
            }
        }
    }

    let out = cmd.output().expect("run tools/run-tests.sh --plan");
    assert!(
        out.status.success(),
        "`run-tests.sh --plan` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan = String::from_utf8_lossy(&out.stdout);
    let line = plan
        .lines()
        .find(|l| l.contains(&core_path.display().to_string()))
        .unwrap_or_else(|| panic!("no plan line names the stand-in core tree:\n{plan}"));
    assert!(
        line.contains("not a git checkout"),
        "run-tests.sh described a directory that is not a git checkout as though \
         it were one, which means it answered from the inherited GIT_DIR rather \
         than from the tree it was handed. Line was: {line}"
    );
}
