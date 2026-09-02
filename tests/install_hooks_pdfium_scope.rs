//! Guards the one hook in the org that is not a CI mirror: the pdfium-render
//! fork's pre-commit hook (libviprs-tests#198).
//!
//! The fork runs a lint policy of its own because upstream's workflow runs no
//! lint at all and the tree carries hundreds of pre-existing warnings. The
//! policy is one sentence: block on lints the commit in hand introduces, stay
//! quiet about everything it inherits. That is a hard thing to keep true,
//! because both halves fail silently in opposite directions, and both have
//! already happened here.
//!
//! It used to scope against the whole fork delta versus `upstream/master`,
//! which meant every line the fork had ever written was in scope forever. A
//! newer clippy grew `deref on an immutable reference`, 54 of them landed on
//! fork lines written months earlier, and the hook then refused every commit
//! on an unmodified `origin/master` checkout with nothing staged. Measured
//! 2026-09-02 before the fix: 54 warnings, exit 1, clean tree.
//!
//! So this drives the real installed hook against a throwaway crate that has
//! an inherited lint of its own, and pins all four answers. Nothing here reads
//! the hook's text: the two guards that did that on the pre-push hook stayed
//! green while the behaviour they named had been deleted
//! (libviprs/libviprs#695).

use std::path::Path;
use std::process::Command;

mod common;
use common::hooks::{Workspace, git};

/// The stand-in crate, committed first so it is what the fork "inherited".
///
/// `inherited_debt` takes a `&Vec<u8>`, which is `clippy::ptr_arg` and warns by
/// default. It stands in for the 470-odd warnings the real fork carries. Every
/// assertion below about the hook staying quiet is only worth something while
/// this lint is really there, so `the_fixture_really_does_carry_a_lint` checks
/// that directly rather than taking it on trust.
const INHERITED: &str = r#"//! A stand-in for the fork, carrying one pre-existing lint.

pub fn inherited_debt(v: &Vec<u8>) -> usize {
    v.len()
}
"#;

const MANIFEST: &str = r#"[package]
name = "pdfium-render-standin"
version = "0.0.0"
edition = "2021"
publish = false

[dependencies]
"#;

/// A `pdfium-render` stand-in with the fork's hook installed and one commit of
/// inherited lint debt behind it.
fn fixture() -> Workspace {
    let ws = Workspace::new();
    let repo = ws.repo("pdfium-render");
    std::fs::create_dir_all(repo.join("src")).expect("create the stand-in crate");
    ws.commit(
        "pdfium-render",
        "the fork as inherited",
        &[("Cargo.toml", MANIFEST), ("src/lib.rs", INHERITED)],
    );
    ws
}

/// Run the installed hook in the stand-in fork and hand back whether it
/// allowed the commit, plus everything it printed.
fn run_hook(ws: &Workspace) -> (bool, String) {
    let repo = ws.repo("pdfium-render");
    let hook = repo.join(".git/hooks/pre-commit");
    assert!(
        hook.is_file(),
        "no pre-commit hook was installed into the stand-in fork, so every \
         assertion here would be about a hook that does not exist"
    );
    let out = Command::new("bash")
        .arg(&hook)
        .current_dir(&repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("run the fork's pre-commit hook");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

fn write(repo: &Path, rel: &str, body: &str) {
    std::fs::write(repo.join(rel), body).expect("write into the stand-in fork");
}

/// The positive control for everything else here.
///
/// Three of the four cases below assert the hook stays quiet, and "quiet"
/// looks identical whether clippy found nothing worth reporting or clippy
/// never ran at all. So prove the fixture really is dirty first: a plain
/// `cargo clippy` on it has to report the inherited lint. Without this, a
/// clippy that silently failed to build would make this whole file pass.
#[test]
fn the_fixture_really_does_carry_a_lint() {
    let ws = fixture();
    let repo = ws.repo("pdfium-render");
    let out = Command::new("cargo")
        .args(["clippy", "--all-targets"])
        .current_dir(&repo)
        .output()
        .expect("these guards drive a hook that runs cargo clippy");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("ptr_arg") || text.contains("writing `&Vec`"),
        "the stand-in fork was supposed to carry a pre-existing clippy lint and \
         clippy did not report one, so every `the hook stayed quiet` assertion \
         in this file would pass on a fixture with nothing to be quiet \
         about:\n{text}"
    );
}

/// The failure that was actually in the field: a clean checkout, nothing
/// staged, and the hook refusing the commit over lints nobody had just
/// written.
#[test]
fn a_clean_checkout_with_nothing_staged_commits() {
    let ws = fixture();
    let (ok, printed) = run_hook(&ws);
    assert!(
        ok,
        "the fork's pre-commit hook refused a commit on a clean checkout with \
         nothing staged. That is the failure the scoping fix was for: a gate \
         that is red before you have typed anything is a gate people \
         delete.\n{printed}"
    );
}

/// The intent, kept: inherited lints stay out of the way.
#[test]
fn a_change_that_introduces_no_lint_commits_over_inherited_debt() {
    let ws = fixture();
    let repo = ws.repo("pdfium-render");
    write(
        &repo,
        "src/lib.rs",
        &format!(
            "{INHERITED}
pub fn added_clean(v: &[u8]) -> usize {{
    v.len()
}}
"
        ),
    );
    git(&repo, &["add", "-A"]);

    let (ok, printed) = run_hook(&ws);
    assert!(
        ok,
        "the fork's pre-commit hook refused a change that introduces no lint of \
         its own. The tree still carries the inherited one, and blocking on \
         that is what makes the hook unusable during an upstream merge.\n{printed}"
    );
}

/// The intent, kept the other way: a lint this change writes does block.
///
/// Without this the fix would be indistinguishable from deleting the hook.
#[test]
fn a_change_that_introduces_a_lint_is_refused() {
    let ws = fixture();
    let repo = ws.repo("pdfium-render");
    write(
        &repo,
        "src/lib.rs",
        &format!(
            "{INHERITED}
pub fn added_dirty(v: &Vec<u8>) -> usize {{
    v.len()
}}
"
        ),
    );
    git(&repo, &["add", "-A"]);

    let (ok, printed) = run_hook(&ws);
    assert!(
        !ok,
        "the fork's pre-commit hook allowed a change whose own new line trips a \
         clippy lint, so the scoping has been widened into letting everything \
         through.\n{printed}"
    );
    assert!(
        printed.contains("added_dirty") || printed.contains("&Vec"),
        "the hook refused the commit without naming the lint it refused it \
         for, which leaves whoever hit it with nothing to fix:\n{printed}"
    );
}

/// A commit that only deletes lines has a real diff and no added lines to
/// scope to, so the scope is empty while the tree can perfectly well be
/// broken. Deleting something another file depends on is the everyday way to
/// do it, and an empty scope must not read as a pass.
#[test]
fn a_deletion_that_breaks_the_build_is_refused() {
    let ws = fixture();
    let repo = ws.repo("pdfium-render");
    write(
        &repo,
        "src/lib.rs",
        "//! A stand-in for the fork, carrying one pre-existing lint.

pub fn inherited_debt(v: &Vec<u8>) -> usize {
    v.len()
}

pub fn caller() -> usize {
    helper()
}

pub fn helper() -> usize {
    0
}
",
    );
    ws.commit("pdfium-render", "add a caller and its helper", &[]);

    // Delete only `helper`, which leaves `caller` calling something that is no
    // longer there. Nothing on a line this change adds, because it adds none.
    write(
        &repo,
        "src/lib.rs",
        "//! A stand-in for the fork, carrying one pre-existing lint.

pub fn inherited_debt(v: &Vec<u8>) -> usize {
    v.len()
}

pub fn caller() -> usize {
    helper()
}
",
    );
    git(&repo, &["add", "-A"]);

    let out = git(&repo, &["diff", "--cached", "--numstat"]);
    assert!(
        out.split_whitespace().next() == Some("0"),
        "this case is only about a deletion-only change and this one adds \
         lines, so it is testing something else: {out}"
    );

    let (ok, printed) = run_hook(&ws);
    assert!(
        !ok,
        "the fork's pre-commit hook allowed a commit that stops the crate \
         building. The scope was empty because the change adds no lines, and an \
         empty scope was being read as nothing to check.\n{printed}"
    );
}
