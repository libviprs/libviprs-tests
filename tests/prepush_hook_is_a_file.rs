//! Guards on how the pre-push hook is delivered (libviprs/libviprs#695).
//!
//! It used to be a heredoc inside `tools/install-hooks.sh`, and three problems
//! followed from that shape rather than from anything the hook does. A linter
//! saw a string literal, so ~120 lines of shell got no static checking. A
//! guard could only assert on it by substring, and two that did stayed green
//! while the behaviour they named had been removed. And it reached a repo only
//! when somebody re-ran the installer, with no version stamp in the generated
//! file, so a stale install was undetectable: at any moment an unknown subset
//! of checkouts was running an unknown vintage.
//!
//! That third one has bite. libviprs/libviprs#684 existed because a hook
//! silently gated the wrong tree and read green, and the delivery mechanism
//! guaranteed the next such fix would ship the same way.
//!
//! So the hook is a tracked file and the installer writes a shim that runs it.
//! These guards are about that arrangement holding: they install into a
//! throwaway workspace with the real installer, change the checked-in file
//! without reinstalling, and check the change reaches the push.

use std::path::Path;

mod common;
use common::hooks::{PRE_PUSH_HOOK, Workspace, git, make_executable, repo_root};

/// Write the real hook to `dest` with one extra line saying which copy of it
/// ran. Always built from this repo's pristine copy, so marking a file never
/// inherits a marker from whatever it is overwriting.
fn marked(dest: &Path, marker: &str) {
    let body = std::fs::read_to_string(repo_root().join(PRE_PUSH_HOOK))
        .expect("read this repo's checked-in hook");
    let (shebang, rest) = body.split_once('\n').expect("the hook has a shebang");
    std::fs::write(dest, format!("{shebang}\necho \"{marker}\"\n{rest}"))
        .expect("write a marked hook");
    make_executable(dest);
}

/// Commit something the skip list cannot call inert, and hand back the range.
fn a_change(ws: &Workspace, tree: &Path) -> (String, String) {
    let before = git(tree, &["rev-parse", "HEAD"]).trim().to_string();
    let after = ws.commit_in(
        tree,
        "a change the suite can see",
        &[("src/lib.rs", "// c\n")],
    );
    (before, after)
}

/// The point of the whole change. Edit the checked-in hook, do not reinstall
/// anything, and the next push runs the edited hook.
///
/// While it was a heredoc this could not hold: a fix reached a repo only when
/// somebody re-ran `install-hooks.sh` there, and nothing anywhere could tell
/// you which vintage a given clone was on.
#[test]
fn a_push_runs_the_checked_in_hook_without_a_reinstall() {
    let ws = Workspace::new();
    marked(&ws.checked_in_hook(), "HOOK-EDITED-AFTER-INSTALL");

    let tree = ws.repo("libviprs");
    let (before, after) = a_change(&ws, &tree);
    let out = ws
        .push("libviprs")
        .range(&before, &after)
        .with_git_env()
        .run();

    assert!(
        out.contains("HOOK-EDITED-AFTER-INSTALL"),
        "a push ran a copy of the hook rather than the checked-in file, so a \
         fix to the hook reaches a repo only when somebody re-runs \
         install-hooks.sh there and no clone can say which vintage it is on \
         (#695). The hook said:\n{out}"
    );
}

/// The same property under the case that produced #684: a harness push runs
/// the hook it is pushing, out of the worktree, not the copy in the main
/// checkout. Otherwise a change to the hook is gated by the hook it replaces.
#[test]
fn a_harness_push_runs_the_hook_it_is_pushing() {
    let ws = Workspace::new();
    marked(&ws.checked_in_hook(), "HOOK-FROM-THE-MAIN-CHECKOUT");
    ws.commit("libviprs-tests", "mark the main checkout's hook", &[]);

    let lane = ws.worktree("libviprs-tests", "harness-lane");
    marked(&lane.join(PRE_PUSH_HOOK), "HOOK-FROM-THE-LANE");
    let before = git(&lane, &["rev-parse", "HEAD"]).trim().to_string();
    let after = ws.commit_in(&lane, "change the hook", &[]);

    let out = ws
        .push("libviprs-tests")
        .from(&lane)
        .range(&before, &after)
        .with_git_env()
        .run();

    assert!(
        out.contains("HOOK-FROM-THE-LANE"),
        "a harness push ran a hook from outside the tree being pushed, so a \
         change to the hook is gated by the hook it replaces (#684, #695). \
         The hook said:\n{out}"
    );
    assert!(
        !out.contains("HOOK-FROM-THE-MAIN-CHECKOUT"),
        "the push ran the main checkout's hook as well as the lane's. \
         The hook said:\n{out}"
    );
}

/// Installing by reference has one failure mode a copy does not: the thing
/// pointed at can go away. It must say so rather than let the push through,
/// because a gate that disappears quietly is the failure #684 and #683 were
/// both about.
#[test]
fn the_installed_shim_refuses_the_push_when_the_hook_is_gone() {
    let ws = Workspace::new();
    let hooks_dir = ws.repo("libviprs-tests").join("tools/hooks");
    std::fs::rename(&hooks_dir, hooks_dir.with_extension("moved"))
        .expect("move the checked-in hook out of the way");

    let tree = ws.repo("libviprs");
    let (before, after) = a_change(&ws, &tree);
    let (allowed, out) = ws
        .push("libviprs")
        .range(&before, &after)
        .with_git_env()
        .try_run();

    assert!(
        !allowed,
        "the push went through with no hook to run. A gate installed by \
         reference has to fail loudly when the reference breaks, or moving a \
         checkout turns the gate off in silence (#695). It said:\n{out}"
    );
    assert!(
        out.contains(&hooks_dir.join("pre-push").display().to_string()),
        "the refusal did not name the path it could not find, so nobody can \
         act on it. It said:\n{out}"
    );
    assert!(
        out.contains("install-hooks.sh"),
        "the refusal did not say how to fix it. It said:\n{out}"
    );
}

/// A copied-and-chmodded hook was executable because the installer made it so.
/// A hook reached by reference is executable because git tracked the bit, and
/// git will happily track it without one. A non-executable hook is not a
/// broken hook, it is no hook: git skips it and says nothing.
#[test]
fn the_hook_is_tracked_executable() {
    let entry = git(&repo_root(), &["ls-files", "-s", "--", PRE_PUSH_HOOK]);
    let entry = entry.trim();
    assert!(
        !entry.is_empty(),
        "git does not track {PRE_PUSH_HOOK}, so nothing ships it to a clone \
         and every install points at a file that is not there (#695)"
    );
    assert!(
        entry.starts_with("100755 "),
        "{PRE_PUSH_HOOK} is tracked without the executable bit ({entry}). git \
         skips a hook it cannot execute and prints nothing, so the gate would \
         be off in every fresh clone with no sign of it"
    );
}
