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

fn docker() -> Option<String> {
    let probe = Command::new("docker").arg("version").output().ok()?;
    probe.status.success().then(|| "docker".to_string())
}
