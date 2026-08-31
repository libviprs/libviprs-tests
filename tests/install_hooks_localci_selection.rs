//! Guards that `write_pre_commit` (`tools/install-hooks.sh`) actually picks
//! between its two pre-commit templates on whether the target repo ships
//! `tools/local-ci.py`: present, it gets a hook that defers to it; absent, it
//! gets the hardcoded per-repo cargo list.
//!
//! `tests/install_hooks_mirror_ci.rs` exercises the hardcoded list's
//! *contents* in detail, but its synthetic `Workspace` never lays down a
//! `tools/local-ci.py` anywhere, so neither the detection logic nor the
//! LOCALCI branch itself was exercised by anything before this — and
//! `libviprs` ships `tools/local-ci.py` today, so that branch is live for
//! every real contributor.
//!
//! Both branches are driven by actually running the installed hook, with a
//! recording stand-in in place of whatever each branch shells out to, rather
//! than by reading the hook's text (libviprs/libviprs#695).

use std::process::Command;

mod common;
use common::hooks::{Workspace, make_executable};

/// A stand-in for `tools/local-ci.py` that just proves it ran. Carries a
/// real shebang so that invoking it directly by path (rather than through
/// `python3 <path>`) also works, which matters for the two tests here that
/// are not about the executable-bit fix and would otherwise fail for an
/// unrelated reason under the pre-fix hook.
const RECORDING_LOCALCI: &str = r#"#!/usr/bin/env python3
import os
with open(os.environ["LOCALCI_RECORD"], "a") as f:
    f.write("LOCALCI-RAN\n")
"#;

/// A recording `cargo`, the same stand-in `tests/install_hooks_mirror_ci.rs`
/// uses to let the hardcoded fallback branch run to completion without a
/// real cargo project.
const RECORDING_CARGO: &str = r#"#!/bin/sh
printf 'cargo %s\n' "$*" >> "$CARGO_RECORD"
exit 0
"#;

/// Run `repo`'s installed pre-commit hook with a recording `cargo` in front
/// of it and a stand-in `tools/run_ported_cells.sh` beside it, and report
/// what each side of the branch recorded: whatever `cargo` saw, and whatever
/// `local-ci.py` saw.
fn run_hook(ws: &Workspace, repo: &str) -> (String, String) {
    let repo_dir = ws.repo(repo);
    let hook = repo_dir.join(".git/hooks/pre-commit");
    assert!(
        hook.is_file(),
        "no pre-commit hook was installed into {repo}, so this guard would \
         pass on two empty recordings"
    );

    let bin = ws.root.join("cargo-recorder").join(repo);
    std::fs::create_dir_all(&bin).expect("create the recorder directory");
    let cargo = bin.join("cargo");
    std::fs::write(&cargo, RECORDING_CARGO).expect("write the recording cargo");
    make_executable(&cargo);

    // The fallback hook reaches this one by relative path rather than
    // through PATH (tests/install_hooks_mirror_ci.rs does the same).
    let ported = repo_dir.join("tools/run_ported_cells.sh");
    std::fs::create_dir_all(ported.parent().expect("tools dir")).expect("create tools dir");
    std::fs::write(&ported, "#!/bin/sh\nexit 0\n").expect("write a stand-in run_ported_cells.sh");
    make_executable(&ported);

    let cargo_record = ws.root.join(format!("{repo}.cargo.record"));
    let localci_record = ws.root.join(format!("{repo}.localci.record"));
    let _ = std::fs::remove_file(&cargo_record);
    let _ = std::fs::remove_file(&localci_record);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = Command::new("bash")
        .arg(&hook)
        .current_dir(&repo_dir)
        .env("PATH", path)
        .env("CARGO_RECORD", &cargo_record)
        .env("LOCALCI_RECORD", &localci_record)
        .output()
        .expect("run the generated pre-commit hook");
    assert!(
        out.status.success(),
        "the pre-commit hook for {repo} failed with every command stubbed to \
         succeed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    (
        std::fs::read_to_string(&cargo_record).unwrap_or_default(),
        std::fs::read_to_string(&localci_record).unwrap_or_default(),
    )
}

/// A repo with no `tools/local-ci.py` gets the hardcoded per-repo cargo list,
/// and nothing else.
#[test]
fn a_repo_without_local_ci_py_gets_the_hardcoded_fallback() {
    let ws = Workspace::new();

    let (cargo_ran, localci_ran) = run_hook(&ws, "libviprs-tests");

    assert!(
        !cargo_ran.is_empty(),
        "libviprs-tests has no tools/local-ci.py, so its pre-commit hook must \
         run the hardcoded cargo step list; nothing was recorded"
    );
    assert!(
        localci_ran.is_empty(),
        "libviprs-tests has no tools/local-ci.py, so its pre-commit hook must \
         not have run one:\n{localci_ran}"
    );
}

/// A repo that ships `tools/local-ci.py` gets a hook that defers to it
/// instead of the hardcoded list.
#[test]
fn a_repo_with_local_ci_py_defers_to_it_instead() {
    let ws = Workspace::new();

    let local_ci = ws.repo("libviprs").join("tools/local-ci.py");
    ws.commit(
        "libviprs",
        "add a recording stand-in for local-ci.py",
        &[("tools/local-ci.py", RECORDING_LOCALCI)],
    );
    make_executable(&local_ci);
    ws.install_hooks();

    let (cargo_ran, localci_ran) = run_hook(&ws, "libviprs");

    assert!(
        !localci_ran.is_empty(),
        "libviprs now ships tools/local-ci.py, so its pre-commit hook must \
         defer to it; it never ran"
    );
    assert!(
        cargo_ran.is_empty(),
        "libviprs ships tools/local-ci.py, so its pre-commit hook must not \
         also run the hardcoded cargo list:\n{cargo_ran}"
    );
}

/// Low-severity fix, same review round: the hook used to invoke
/// `local-ci.py` directly by path, relying on its shebang and the executable
/// bit. A present-but-not-executable script failed with a confusing
/// "Permission denied" while the hook's own error text claimed "CI will fail
/// the same way," which was false for what was actually a local permissions
/// problem. Invoking it through `python3` instead means the executable bit
/// no longer matters at all — assert that directly with a stand-in that is
/// deliberately left non-executable.
#[test]
fn the_localci_hook_runs_it_even_when_it_is_not_executable() {
    let ws = Workspace::new();

    ws.commit(
        "libviprs",
        "add a non-executable recording stand-in for local-ci.py",
        &[("tools/local-ci.py", RECORDING_LOCALCI)],
    );
    // Deliberately not made executable: the fix invokes it via `python3
    // "$REPO_DIR/tools/local-ci.py"`, so this must still run.
    ws.install_hooks();

    let (_, localci_ran) = run_hook(&ws, "libviprs");

    assert!(
        !localci_ran.is_empty(),
        "a non-executable tools/local-ci.py did not run. The hook must \
         invoke it in a way that does not depend on the executable bit or \
         its shebang line (e.g. `python3 \"$REPO_DIR/tools/local-ci.py\"`)."
    );
}
