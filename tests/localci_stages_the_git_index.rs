//! The local gate stages enough of a git repository for `git ls-files` to work
//! (libviprs-tests#205).
//!
//! # What was broken
//!
//! `tools/run-tests.sh` stages the two trees under test into a scratch build
//! context and excludes `.git` from both, which it has to: the counterpart's is
//! hundreds of megabytes and every byte would go into the Docker build context.
//!
//! But some guards ask git rather than walking the tree, and they are right to:
//! on a case-insensitive filesystem only one side of a case-only collision is
//! physically there, so the index is the only place both names exist. Measured
//! by running the counterpart's whole suite in a context with no repository in
//! it, **five** of its binaries do this and every one of them asserts on git's
//! exit status rather than skipping:
//!
//! ```text
//! case_only_path_collisions    git ls-files
//! fixture_paths_are_committed  git ls-files
//! oracle_capture_pins          git ls-files
//! changelog_release_claims     git tag
//! local_ci_invocation          git rev-parse --git-common-dir
//! ```
//!
//! So with no repository in the context the first one fails:
//!
//! ```text
//! git ls-files failed, so this guard has nothing to check:
//! fatal: not a git repository (or any of the parent directories): .git
//! ```
//!
//! The run then stops in the `Running libviprs unit tests` phase and never
//! reaches this repo's own integration suite at all. That is not a regression,
//! it is how the gate has always been: it has never been able to finish the
//! counterpart's unit tests.
//!
//! # Why this is a test and not a comment
//!
//! The fix is spread across two files that do not mention each other. If
//! `stage_git_index` stops writing the index, or
//! `Dockerfile.dockerignore` stops re-including it, the context goes back to
//! having no repository in it and the symptom is a counterpart test failing for
//! a reason that has nothing to do with the counterpart. Both halves are
//! checked here, and the second is checked by building a context rather than by
//! reading the ignore file, because an ignore pattern is not something you can
//! reason about by eye: `**/.git/**` with a `!` re-inclusion behaves one way
//! and `**/.git/` behaves another, and only one of them lets anything through.

use std::path::Path;
use std::process::Command;

mod common;
use common::hooks::repo_root;

fn script() -> String {
    std::fs::read_to_string(repo_root().join("tools/run-tests.sh")).expect("tools/run-tests.sh")
}

/// `stage_tree` has to leave a usable index behind.
#[test]
fn staging_a_tree_writes_the_pieces_git_ls_files_needs() {
    let text = script();
    assert!(
        text.contains("stage_git_index"),
        "tools/run-tests.sh no longer stages a git index, so every guard in \
         either tree that asks git what is tracked fails with `not a git \
         repository` and takes the run down before it reaches this repo"
    );
    assert!(
        text.contains("stage_git_index \"$src\" \"$dst\""),
        "stage_git_index exists and stage_tree does not call it"
    );
    for piece in ["/index", "HEAD", "repositoryformatversion", "packed-refs"] {
        assert!(
            text.contains(piece),
            "stage_git_index does not write {piece:?}; git wants the index, a \
             HEAD and a config before it will call a path a repository"
        );
    }
}

/// And the ignore set has to let them through.
///
/// This builds a real context rather than reading the patterns, because the
/// question "does this pattern let `.git/index` through" is not answerable by
/// reading it: `**/.git/` and `**/.git/**` behave differently under a `!`
/// re-inclusion and only one of them lets anything through.
///
/// # About the skip
///
/// It needs a Docker daemon, and there is none inside the mirror's own
/// container, so this skips there and finishes in no time at all. That is the
/// false-green shape the rest of this repo spends so much effort on, so it gets
/// the same treatment: `VIPRS_REQUIRE_DOCKER_PROBE=1` turns the skip into a
/// failure. `run-tests.sh` itself runs on the host and shells out to docker, so
/// the host is where this can actually run, and where it is worth requiring.
#[test]
fn the_ignore_set_lets_the_index_into_the_build_context() {
    let required = std::env::var("VIPRS_REQUIRE_DOCKER_PROBE").is_ok_and(|v| v == "1");
    let Some(docker) = docker() else {
        assert!(
            !required,
            "VIPRS_REQUIRE_DOCKER_PROBE=1 and no docker daemon answered, so \
             this cell would skip the only check that can tell whether the \
             ignore set lets the git index through"
        );
        eprintln!(
            "skipping: no docker daemon reachable. There is none inside the \
             mirror's container; run this on the host, or set \
             VIPRS_REQUIRE_DOCKER_PROBE=1 to make the skip a failure."
        );
        return;
    };

    let dir = tempfile::tempdir().expect("a scratch directory");
    let ctx = dir.path().join("ctx");
    let tree = ctx.join("libviprs");
    std::fs::create_dir_all(tree.join(".git/objects")).unwrap();
    std::fs::create_dir_all(tree.join(".git/refs/heads")).unwrap();
    std::fs::create_dir_all(tree.join("src")).unwrap();
    for (rel, body) in [
        (".git/index", "an index"),
        (".git/HEAD", "ref: refs/heads/staged\n"),
        (".git/config", "[core]\n"),
        (".git/packed-refs", "# pack-refs with: peeled\n"),
        (
            ".git/refs/tags/v0.1.0",
            "0000000000000000000000000000000000000000\n",
        ),
        (".git/objects/.keep", ""),
        // The commit and its trees ride along as a pack so `HEAD` resolves to
        // an object (libviprs-tests#226).
        (".git/objects/pack/pack-1.pack", "a pack"),
        (".git/objects/pack/pack-1.idx", "an index of it"),
        (".git/shallow", "0000000000000000000000000000000000000000\n"),
        // The half that must NOT come through: history is what makes `.git`
        // hundreds of megabytes, and none of it is needed.
        (".git/objects/ab/cdef", "an object"),
        (".git/logs/HEAD", "a reflog"),
        ("src/a.rs", "fn main() {}"),
    ] {
        let at = tree.join(rel);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, body).unwrap();
    }

    let dockerfile = dir.path().join("Dockerfile");
    std::fs::write(
        &dockerfile,
        "FROM alpine:3\nCOPY . /ctx\nRUN find /ctx -type f | sort\n",
    )
    .unwrap();
    std::fs::copy(
        repo_root().join("Dockerfile.dockerignore"),
        dir.path().join("Dockerfile.dockerignore"),
    )
    .expect("this repo's ignore set is the authoritative one for the mirror");

    let out = Command::new(&docker)
        .args(["build", "--no-cache", "-f"])
        .arg(&dockerfile)
        .arg(&ctx)
        .env_remove("DOCKER_DEFAULT_PLATFORM")
        .output()
        .expect("run docker build");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "the probe build failed:\n{log}");

    for wanted in [
        "/ctx/libviprs/.git/index",
        "/ctx/libviprs/.git/HEAD",
        "/ctx/libviprs/.git/config",
        "/ctx/libviprs/.git/packed-refs",
        "/ctx/libviprs/.git/refs/tags/v0.1.0",
        "/ctx/libviprs/.git/objects/pack/pack-1.pack",
        "/ctx/libviprs/.git/objects/pack/pack-1.idx",
        "/ctx/libviprs/.git/shallow",
        "/ctx/libviprs/src/a.rs",
    ] {
        assert!(
            log.contains(wanted),
            "{wanted} did not reach the build context, so the counterpart's \
             tree has no git index in it and `git ls-files` fails there:\n{log}"
        );
    }
    for unwanted in [
        "/ctx/libviprs/.git/objects/ab/cdef",
        "/ctx/libviprs/.git/logs/HEAD",
    ] {
        assert!(
            !log.contains(unwanted),
            "{unwanted} reached the build context. The point of excluding \
             `.git` is that history is hundreds of megabytes; only the index \
             and its two companions should come through.\n{log}"
        );
    }
}

/// `tools/run-tests.sh` is a script with top-level side effects, so it cannot
/// be sourced. This lifts `stage_git_index` out of it by its own braces, along
/// with the `git_at` it may call, and runs just that, under the same
/// `set -euo pipefail` the script runs under, so the test exercises the shipped
/// text rather than a copy of it.
fn run_stage_git_index(src: &Path, dst: &Path) {
    run_stage_git_index_with(src, dst, &[]);
}

/// The text of the shell function `name` in tools/run-tests.sh, braces and all.
fn shell_function(text: &str, name: &str) -> String {
    let start = text
        .find(&format!("\n{name}() {{"))
        .unwrap_or_else(|| panic!("{name} is defined in tools/run-tests.sh"))
        + 1;
    let body = &text[start..];
    let end = body
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} ends in a `}}`"))
        + 3;
    body[..end].to_string()
}

/// `run_stage_git_index` with extra environment, for the cases where what the
/// caller exports is the point.
fn run_stage_git_index_with(src: &Path, dst: &Path, env: &[(&str, &Path)]) {
    let text = script();
    let program = format!(
        "set -euo pipefail\n{}\n{}\nstage_git_index \"$1\" \"$2\"\n",
        shell_function(&text, "git_at"),
        shell_function(&text, "stage_git_index"),
    );
    let mut cmd = git_env(Command::new("bash"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd
        .args(["-c", &program, "bash"])
        .arg(src)
        .arg(dst)
        .output()
        .expect("run bash");
    assert!(
        out.status.success(),
        "stage_git_index failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A pre-push hook runs with `GIT_DIR` and friends set, and the user's own git
/// config has no business in a fixture either.
fn git_env(mut cmd: Command) -> Command {
    for var in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com");
    cmd
}

/// Run git in `dir` and give back (exit code, stdout).
fn git(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = git_env(Command::new("git"))
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("run git");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

/// A staged repo has to look coherent to git (libviprs-tests#226).
///
/// `stage_git_index` pointed HEAD at `refs/heads/staged` and never created it,
/// so `git rev-parse HEAD` exited 128 while `git status` exited 0: two answers
/// to "is this a checkout" from the same directory. The core's benchmark
/// envelope asserts that the two agree, `cargo test` stops at the first red
/// binary, and the local gate never got past it.
///
/// The pair has to agree, and it has to agree on the honest answer: the commit
/// that was actually staged, with `status` working against it rather than
/// failing on an object it was never given.
#[test]
fn a_staged_repo_answers_rev_parse_and_status_the_same_way() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let src = dir.path().join("src");
    let dst = dir.path().join("dst");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), "one\n").unwrap();
    std::fs::write(src.join("sub/b.txt"), "two\n").unwrap();
    assert_eq!(git(&src, &["init", "--quiet"]).0, 0);
    assert_eq!(git(&src, &["add", "."]).0, 0);
    assert_eq!(git(&src, &["commit", "--quiet", "-m", "fixture"]).0, 0);
    let (_, real_head) = git(&src, &["rev-parse", "HEAD"]);
    assert_eq!(real_head.len(), 40, "the fixture needs a real commit");

    // What the gate actually stages: the tree without its `.git`.
    std::fs::create_dir_all(dst.join("sub")).unwrap();
    std::fs::copy(src.join("a.txt"), dst.join("a.txt")).unwrap();
    std::fs::copy(src.join("sub/b.txt"), dst.join("sub/b.txt")).unwrap();
    run_stage_git_index(&src, &dst);

    let (head_rc, head) = git(&dst, &["rev-parse", "HEAD"]);
    let (status_rc, status) = git(&dst, &["status", "--porcelain"]);
    assert_eq!(
        head_rc == 0,
        status_rc == 0,
        "rev-parse HEAD exited {head_rc} and status exited {status_rc}: the \
         staged repo is a checkout to one and not to the other"
    );
    assert_eq!(head_rc, 0, "HEAD points at a ref that was never created");
    assert_eq!(
        head, real_head,
        "the staged HEAD is not the commit that was staged"
    );
    assert_eq!(
        status, "",
        "an unchanged tree staged from a clean commit reads as dirty"
    );

    // And it is a working repo rather than a quiet one: an edit shows up.
    std::fs::write(dst.join("a.txt"), "changed\n").unwrap();
    let (rc, status) = git(&dst, &["status", "--porcelain"]);
    assert_eq!(rc, 0);
    assert!(
        status.contains("a.txt"),
        "an edit reads as clean: {status:?}"
    );
}

/// A git repo at `dir` holding `files`, committed once per entry of
/// `commits` (each a list of files to write before that commit).
fn repo_with_commits(dir: &Path, commits: &[&[(&str, &str)]]) {
    std::fs::create_dir_all(dir).unwrap();
    assert_eq!(git(dir, &["init", "--quiet"]).0, 0);
    for (i, files) in commits.iter().enumerate() {
        for (rel, body) in *files {
            let at = dir.join(rel);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, body).unwrap();
        }
        assert_eq!(git(dir, &["add", "-A"]).0, 0);
        let msg = format!("fixture {i}");
        assert_eq!(git(dir, &["commit", "--quiet", "-m", &msg]).0, 0);
    }
}

/// Copy the files of `src` into `dst`, leaving `.git` behind, the way
/// `stage_tree` does.
fn copy_worktree(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        if from.is_dir() {
            std::fs::create_dir_all(&to).unwrap();
            copy_worktree(&from, &to);
        } else {
            std::fs::create_dir_all(dst).unwrap();
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// The staged repo is shallow and says so: with history behind HEAD in the
/// source, `git log` in the staged repo stops at the one commit it was given
/// instead of failing on a parent it never got. Without `shallow`, this is the
/// cell that goes red.
#[test]
fn a_staged_repo_is_one_commit_deep_and_git_log_knows_it() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let src = dir.path().join("src");
    let dst = dir.path().join("dst");
    repo_with_commits(&src, &[&[("a.txt", "one\n")], &[("a.txt", "two\n")]]);
    copy_worktree(&src, &dst);
    run_stage_git_index(&src, &dst);

    let (rc, log) = git(&dst, &["log", "--oneline"]);
    assert_eq!(
        rc, 0,
        "git log failed in the staged repo, so it went looking for a parent \
         commit the staged repo was never given"
    );
    assert_eq!(
        log.lines().count(),
        1,
        "the staged repo should show exactly the one staged commit: {log:?}"
    );
}

/// A pre-push hook exports `GIT_DIR` for the repo that's pushing, and it beats
/// `-C`. Every git call in `stage_git_index` has to go through `git_at` or the
/// staged HEAD and pack come from the pushing repo while the index comes from
/// `$src`, which is exactly the incoherence #226 is about.
#[test]
fn an_exported_git_dir_does_not_change_what_gets_staged() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let src = dir.path().join("src");
    let other = dir.path().join("other");
    let dst = dir.path().join("dst");
    repo_with_commits(&src, &[&[("a.txt", "the source\n")]]);
    repo_with_commits(&other, &[&[("b.txt", "somebody else\n")]]);
    let (_, src_head) = git(&src, &["rev-parse", "HEAD"]);
    let (_, other_head) = git(&other, &["rev-parse", "HEAD"]);
    assert_ne!(src_head, other_head);

    copy_worktree(&src, &dst);
    run_stage_git_index_with(&src, &dst, &[("GIT_DIR", &other.join(".git"))]);

    let (rc, head) = git(&dst, &["rev-parse", "HEAD"]);
    assert_eq!(rc, 0, "HEAD does not resolve in the staged repo");
    assert_eq!(
        head, src_head,
        "with GIT_DIR exported the staged HEAD is the other repo's commit \
         ({other_head}), not $src's"
    );
    let (rc, status) = git(&dst, &["status", "--porcelain"]);
    assert_eq!((rc, status.as_str()), (0, ""), "status disagrees with HEAD");
}

/// A staged rename plus an edit makes `git status` try inexact rename
/// detection, which reads HEAD's blobs, and the staged repo has none by
/// design. With renames off in the staged config `status` doesn't need them.
#[test]
fn status_works_with_a_staged_rename_and_edit() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let src = dir.path().join("src");
    let dst = dir.path().join("dst");
    let body = "line one\nline two\nline three\nline four\nline five\n";
    repo_with_commits(&src, &[&[("a.txt", body)]]);
    assert_eq!(git(&src, &["mv", "a.txt", "renamed.txt"]).0, 0);
    std::fs::write(src.join("renamed.txt"), format!("{body}line six\n")).unwrap();
    assert_eq!(git(&src, &["add", "renamed.txt"]).0, 0);

    copy_worktree(&src, &dst);
    run_stage_git_index(&src, &dst);

    let out = git_env(Command::new("git"))
        .arg("-C")
        .arg(&dst)
        .args(["status", "--porcelain"])
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git status failed in the staged repo with a staged rename plus edit:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let status = String::from_utf8_lossy(&out.stdout);
    assert!(
        status.contains("renamed.txt") && status.contains("a.txt"),
        "status should show the delete and the add: {status:?}"
    );
}

/// The build context is cached and reused, so staging again has to replace
/// the staged repo rather than add to it. A second stage from the same source
/// leaves one pack, and a source that went back to having no commit leaves no
/// stale `staged` ref behind for HEAD to resolve to.
#[test]
fn restaging_replaces_the_staged_repo_instead_of_adding_to_it() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let src = dir.path().join("src");
    let dst = dir.path().join("dst");
    repo_with_commits(&src, &[&[("a.txt", "one\n")]]);
    copy_worktree(&src, &dst);
    run_stage_git_index(&src, &dst);
    repo_with_commits(&src, &[&[("a.txt", "two\n")]]);
    copy_worktree(&src, &dst);
    run_stage_git_index(&src, &dst);

    let packs: Vec<_> = std::fs::read_dir(dst.join(".git/objects/pack"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".pack"))
        .map(|e| e.file_name())
        .collect();
    assert_eq!(packs.len(), 1, "packs pile up across stagings: {packs:?}");

    // Same destination, a source with an index and no commit.
    let unborn = dir.path().join("unborn");
    std::fs::create_dir_all(&unborn).unwrap();
    std::fs::write(unborn.join("a.txt"), "three\n").unwrap();
    assert_eq!(git(&unborn, &["init", "--quiet"]).0, 0);
    assert_eq!(git(&unborn, &["add", "a.txt"]).0, 0);
    copy_worktree(&unborn, &dst);
    run_stage_git_index(&unborn, &dst);

    let (rc, head) = git(&dst, &["rev-parse", "--verify", "--quiet", "HEAD"]);
    assert_ne!(
        rc, 0,
        "the source has no commit, yet the staged HEAD resolves to {head}, left \
         over from the previous staging"
    );
}

fn docker() -> Option<String> {
    let probe = Command::new("docker").arg("version").output().ok()?;
    probe.status.success().then(|| "docker".to_string())
}
