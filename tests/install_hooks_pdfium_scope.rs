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

use std::path::{Path, PathBuf};
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

/// A cargo target directory of this workspace's own, outside every stand-in
/// repo so `git add -A` can never stage it.
///
/// # Why this is not just inherited
///
/// Every fixture here builds a crate called `pdfium-render-standin v0.0.0`, and
/// each test builds its own at its own temp path. Cargo does not keep those
/// apart when they share a `CARGO_TARGET_DIR`: the second one to be built is
/// reported **Fresh**, is not compiled, and **replays the first one's
/// diagnostics**. Measured 2026-09-11 on cargo 1.98.1 / clippy 0.1.98, two
/// crates at `/w/a` and `/w/b` over one target dir:
///
/// ```text
/// --- a (first, &Vec<u8> on line 3 and line 7) ---
///     Checking pdfium-render-standin v0.0.0 (/w/a)
///  --> src/lib.rs:3:26
///  --> src/lib.rs:7:23
/// --- b (second, &Vec<u8> on line 3, &[u8] on line 7) ---
///  --> src/lib.rs:3:26
///  --> src/lib.rs:7:23
/// ```
///
/// No `Checking` line for `b`, and a `&Vec` warning pinned on a line of `b`
/// that reads `pub fn added_clean(v: &[u8])`. Run the two concurrently instead
/// and the loss goes the other way: both come back with the line-3 warning and
/// neither reports line 7 at all.
///
/// That is a lint verdict about a file the compiler never looked at, and it is
/// enough to flip this file's answers in either direction. A phantom warning on
/// a line the change added turns the two "the hook stays quiet" cases red; a
/// missing one turns the two "the hook refuses" cases green, which is worse.
/// Both happened, on the same tree, an hour apart, and the difference was which
/// neighbour got there first.
///
/// `tools/local-ci.py:315` exports `CARGO_TARGET_DIR` for every job, so the
/// container mirror hits this every time and a bare `cargo test` on a dev
/// machine usually does not. A guard whose answer depends on an ambient
/// variable is not a guard, so the harness pins it rather than reading it.
fn standin_target_dir(ws: &Workspace) -> PathBuf {
    ws.root.join(".standin-cargo-target")
}

/// Run the installed hook in the stand-in fork and hand back whether it
/// allowed the commit, plus everything it printed.
fn run_hook(ws: &Workspace) -> (bool, String) {
    run_hook_full(ws, None, None)
}

/// The same, with a `RUSTFLAGS` of your choosing.
fn run_hook_with(ws: &Workspace, rustflags: Option<&str>) -> (bool, String) {
    run_hook_full(ws, rustflags, None)
}

/// The same again, plus a `CARGO_TARGET_DIR` the caller wants the hook to
/// inherit. It is set and then overridden on purpose: the override is the fix,
/// and a caller that passes one in is measuring that the override beats it.
fn run_hook_full(
    ws: &Workspace,
    rustflags: Option<&str>,
    ambient_target_dir: Option<&Path>,
) -> (bool, String) {
    let repo = ws.repo("pdfium-render");
    let hook = repo.join(".git/hooks/pre-commit");
    assert!(
        hook.is_file(),
        "no pre-commit hook was installed into the stand-in fork, so every \
         assertion here would be about a hook that does not exist"
    );
    let mut cmd = Command::new("bash");
    cmd.arg(&hook)
        .current_dir(&repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    match rustflags {
        Some(f) => cmd.env("RUSTFLAGS", f),
        None => cmd.env_remove("RUSTFLAGS"),
    };
    if let Some(dir) = ambient_target_dir {
        cmd.env("CARGO_TARGET_DIR", dir);
    }
    cmd.env("CARGO_TARGET_DIR", standin_target_dir(ws));
    let out = cmd.output().expect("run the fork's pre-commit hook");
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

/// The 1-based line of `INHERITED` carrying `needle`, so the expectations below
/// move with the fixture instead of being a number somebody typed once.
fn line_of(body: &str, needle: &str) -> usize {
    body.lines()
        .position(|line| line.contains(needle))
        .map(|i| i + 1)
        .unwrap_or_else(|| panic!("the fixture no longer contains {needle:?}:\n{body}"))
}

/// Every primary span clippy reported against `src/lib.rs`, as line numbers.
///
/// Read out of `--message-format=json` rather than out of the rendered text,
/// because `CARGO_TERM_COLOR=always` is set both in this repo's CI and in the
/// container image, and the escape sequence lands between `-->` and the path.
fn clippy_lines(ws: &Workspace) -> Vec<u64> {
    let repo = ws.repo("pdfium-render");
    let out = Command::new("cargo")
        .args(["clippy", "--all-targets", "--message-format=json"])
        .current_dir(&repo)
        .env("CARGO_TARGET_DIR", standin_target_dir(ws))
        .env_remove("RUSTFLAGS")
        .output()
        .expect("these guards drive a hook that runs cargo clippy");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = Vec::new();
    for record in stdout.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(record) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let level = message.get("level").and_then(|l| l.as_str());
        if level != Some("warning") && level != Some("error") {
            continue;
        }
        let Some(spans) = message.get("spans").and_then(|s| s.as_array()) else {
            continue;
        };
        for span in spans {
            if span.get("is_primary").and_then(|p| p.as_bool()) != Some(true) {
                continue;
            }
            if span.get("file_name").and_then(|f| f.as_str()) != Some("src/lib.rs") {
                continue;
            }
            if let Some(line) = span.get("line_start").and_then(|l| l.as_u64()) {
                lines.push(line);
            }
        }
    }
    lines
}

/// The positive control for everything else here, and the negative control for
/// the one thing a positive control cannot see.
///
/// Three of the four cases below assert the hook stays quiet, and "quiet" looks
/// identical whether clippy found nothing worth reporting or clippy never ran
/// at all. So prove the fixture really is dirty first.
///
/// It also has to prove the lint is on the line the fixture put it on and
/// nowhere else. `pdfium-render-standin v0.0.0` is the package name every
/// fixture in this file uses, and cargo replays a same-named neighbour's
/// diagnostics when they share a target directory (see [`standin_target_dir`]).
/// A replayed one lands on a line this fixture does not even have, which is how
/// a `&Vec` warning ends up pinned to a line reading `&[u8]`. Asserting a line
/// range catches that here, by name, instead of leaving it to surface as two
/// unrelated cases going red in whichever direction the race fell.
#[test]
fn the_fixture_really_does_carry_a_lint() {
    let ws = fixture();
    let lines = clippy_lines(&ws);
    assert!(
        !lines.is_empty(),
        "the stand-in fork was supposed to carry a pre-existing clippy lint and \
         clippy reported no primary span in src/lib.rs at all, so every `the \
         hook stayed quiet` assertion in this file would pass on a fixture with \
         nothing to be quiet about"
    );

    let debt = line_of(INHERITED, "&Vec<u8>") as u64;
    assert!(
        lines.contains(&debt),
        "clippy reported spans at {lines:?} but nothing on line {debt}, which is \
         the `&Vec<u8>` the whole fixture is built around"
    );

    let last = INHERITED.lines().count() as u64;
    for line in &lines {
        assert!(
            *line <= last,
            "clippy reported a lint at src/lib.rs:{line} in a fixture that is \
             {last} lines long, so it is not about this crate. That is a \
             neighbour's cached diagnostic being replayed through a shared \
             CARGO_TARGET_DIR (spans seen: {lines:?})"
        );
    }
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

/// `-D warnings` in the environment must not turn inherited debt into a
/// refusal.
///
/// The scoping rests on clippy calling an inherited lint a warning, so the
/// filter can drop it. With `-D warnings` set every one of them arrives as an
/// error, clippy exits non-zero, and the hook refuses a commit over exactly
/// the debt it exists to ignore. That is not a hypothetical: this repo's own
/// ci.yml sets `RUSTFLAGS: -Dwarnings` at the workflow level, and the first
/// run of this file on CI went red on it while every local run was green.
///
/// So the hook takes `-D warnings` back off for its own clippy call, and this
/// pins that rather than leaving the next person to find it the same way.
#[test]
fn an_ambient_deny_warnings_does_not_turn_inherited_debt_into_a_refusal() {
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

    let (ok, printed) = run_hook_with(&ws, Some("-Dwarnings"));
    assert!(
        ok,
        "with RUSTFLAGS=-Dwarnings the fork's hook refused a change that \
         introduces no lint of its own. The inherited lint came back as an \
         error rather than a warning and the scope filter never got to drop \
         it.\n{printed}"
    );

    // And a lint this change does write still has to be refused with the flag
    // set, or the fix is just "ignore clippy when RUSTFLAGS is present".
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
    let (ok, printed) = run_hook_with(&ws, Some("-Dwarnings"));
    assert!(
        !ok,
        "with RUSTFLAGS=-Dwarnings the fork's hook allowed a change whose own \
         new line trips a lint, so taking the flag off has been widened into \
         letting everything through.\n{printed}"
    );
}

/// A target directory shared with another fork must not decide this fork's
/// verdict.
///
/// The two cases above rest on clippy's line numbers meaning what they say.
/// Cargo breaks that when two crates of the same name and version share a
/// `CARGO_TARGET_DIR`: the second is Fresh, never compiled, and replays the
/// first one's diagnostics, so a line reading `&[u8]` arrives carrying a `&Vec`
/// warning. `tools/local-ci.py:315` exports `CARGO_TARGET_DIR` for every job, so
/// the container mirror is exactly the environment where this bites, and a bare
/// `cargo test` on a laptop is exactly the environment where it does not. That
/// difference is why it survived: the local run was green and the mirror was
/// red, and both were reporting the same code.
///
/// So drive two forks through one shared directory, dirty first so there is
/// something to replay, and require the clean one to commit anyway.
#[test]
fn a_target_dir_shared_with_another_fork_does_not_decide_this_one() {
    let shared = tempfile::tempdir().expect("a target dir for the two forks to share");

    // The first fork writes a lint on the line the second fork leaves clean, so
    // a replayed diagnostic is unmistakable rather than a coincidence.
    let dirty = fixture();
    write(
        &dirty.repo("pdfium-render"),
        "src/lib.rs",
        &format!(
            "{INHERITED}
pub fn added_dirty(v: &Vec<u8>) -> usize {{
    v.len()
}}
"
        ),
    );
    git(&dirty.repo("pdfium-render"), &["add", "-A"]);
    let (dirty_ok, dirty_printed) = run_hook_full(&dirty, None, Some(shared.path()));
    assert!(
        !dirty_ok,
        "the first fork writes `&Vec<u8>` on a line it is adding and the hook \
         let it through, so the second half of this test would be proving \
         nothing: there would be no lint to replay.\n{dirty_printed}"
    );

    // Same package name, same version, same shared directory, and a line in the
    // same place that is clean.
    let clean = fixture();
    write(
        &clean.repo("pdfium-render"),
        "src/lib.rs",
        &format!(
            "{INHERITED}
pub fn added_clean(v: &[u8]) -> usize {{
    v.len()
}}
"
        ),
    );
    git(&clean.repo("pdfium-render"), &["add", "-A"]);
    let (clean_ok, clean_printed) = run_hook_full(&clean, None, Some(shared.path()));
    assert!(
        clean_ok,
        "a second fork was refused over a lint on a line that reads `&[u8]`. \
         That lint belongs to the fork built before it in the same \
         CARGO_TARGET_DIR, replayed because cargo called this crate Fresh and \
         never compiled it. The harness has to pin the target directory rather \
         than inherit one.\n{clean_printed}"
    );
}
