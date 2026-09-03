//! Guards the honesty of the LOCALCI pre-commit hook that
//! `write_pre_commit` in `tools/install-hooks.sh` generates.
//!
//! The hook runs `tools/local-ci.py --fast --worktree`. The `--worktree` half
//! is a deliberate speed choice: it bind-mounts the tree instead of
//! provisioning it from git, which is about 12s per commit against about 107s
//! measured, and nobody tolerates 107s on every commit.
//!
//! The cost is that a Docker Desktop bind mount off an APFS host is
//! case-insensitive and carries untracked files, so the hook can pass on a
//! tree CI rejects. That is not hypothetical: libviprs#977 was a fixture whose
//! committed name differed from the referenced one only in case, it resolved
//! fine through the bind mount, and `main` was red on ubuntu-latest for two
//! days while the local mirror said PASS.
//!
//! So the hook has to say out loud that it is the fast check and not the gate.
//! This guard pins that coupling in BOTH directions, because each half rots on
//! its own: drop `--worktree` and the warning becomes a lie, keep
//! `--worktree` and drop the warning and the hook oversells itself. It reads
//! the generator rather than an installed hook, so it needs no harness and
//! cannot be fooled by a stale install.

use std::path::{Path, PathBuf};

fn installer_source() -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/install-hooks.sh");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// The LOCALCI heredoc, which is the hook body for repos shipping
/// `tools/local-ci.py`. Everything this guard asserts lives inside it, so
/// slicing it out keeps an unrelated mention elsewhere in the installer from
/// satisfying or breaking the check.
fn localci_hook_body(src: &str) -> &str {
    let start = src
        .find("<< 'LOCALCI'")
        .expect("install-hooks.sh no longer has a LOCALCI heredoc; this guard needs updating");
    let rest = &src[start..];
    let end = rest
        .find("\nLOCALCI\n")
        .expect("the LOCALCI heredoc is not terminated, so the installer is broken");
    &rest[..end]
}

/// The line that actually RUNS local-ci.py, as opposed to the comments and
/// `echo`s around it that also name its flags.
///
/// This distinction is the whole point. My first version of this guard read
/// the entire heredoc, so removing `--worktree` from the command still left it
/// in the explanatory comment and the guard passed a mutation it was written
/// to catch. `pdfium_ci_policy.rs` learned the same lesson about prose that
/// names a flag it is explaining.
fn invocation(body: &str) -> &str {
    body.lines()
        .map(str::trim_start)
        .find(|l| !l.starts_with('#') && !l.starts_with("echo") && l.contains("local-ci.py"))
        .expect("the LOCALCI hook no longer invokes local-ci.py at all")
}

#[test]
fn a_bind_mounted_pre_commit_hook_admits_it_is_not_the_gate() {
    let src = installer_source();
    let body = localci_hook_body(&src);

    let runs_bind_mounted = invocation(body).contains("--worktree");
    // "not the gate" in some casing, plus the command that IS the gate. Both,
    // because a warning that does not say what to run instead is not actionable.
    let lower = body.to_lowercase();
    let warns = lower.contains("not the gate");
    let names_the_gate = body.contains("make ci");

    if runs_bind_mounted {
        assert!(
            warns && names_the_gate,
            "the LOCALCI pre-commit hook runs local-ci.py with --worktree, which bind-mounts \
             the tree (case-insensitive here, and it carries untracked files), so it can pass \
             on a tree CI rejects. It must say it is not the gate and name `make ci` as the one \
             that is. Found: says-not-the-gate={warns}, names-make-ci={names_the_gate}"
        );
    } else {
        assert!(
            !warns,
            "the LOCALCI pre-commit hook no longer passes --worktree, so it provisions from git \
             and IS case-exact, but it still tells the reader it is not the gate. That warning \
             is now false and should go, along with this branch of the guard."
        );
    }
}

#[test]
fn the_hook_still_runs_the_fast_half() {
    let src = installer_source();
    let body = localci_hook_body(&src);
    assert!(
        invocation(body).contains("--fast"),
        "the LOCALCI pre-commit hook dropped --fast, so every commit now runs the full job list \
         including the test and integration jobs. That is the pre-push half, not the pre-commit \
         half (see the commit/push split this installer declares)."
    );
}
