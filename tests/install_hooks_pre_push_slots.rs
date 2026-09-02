//! Guards on which repos `tools/install-hooks.sh` gives a pre-push gate to
//! (libviprs/libviprs#691).
//!
//! The installer wrote the same pre-push hook into `libviprs`, `libviprs-cli`
//! and `libviprs-tests`, and that hook runs `tools/run-tests.sh`, whose image
//! holds the core crate and this harness and nothing else. A cli developer
//! therefore paid a full image build and suite run on every push and got back
//! a verdict that could not fail on the commits going out. That is the trade
//! that teaches people to reach for `--no-verify`, which is what
//! libviprs/libviprs#683 was opened on.
//!
//! CI still catches a bad cli change: the dedicated `cli-differential` job
//! lays the cli down at `CLI_COUNTERPART_REV` and runs with
//! `VIPRS_REQUIRE_CLI=1`, so a silent skip there is a hard panic
//! (`tests/common/cli.rs`). This is a local-gate honesty problem, not a hole
//! in CI coverage.
//!
//! Every guard here *runs* `tools/install-hooks.sh` against a throwaway
//! workspace of stand-in repos and reads the answer off the files it leaves
//! behind. None of them greps the installer: the whole reason
//! libviprs/libviprs#695 exists is that two guards on this hook asserted by
//! substring and stayed green while the behaviour they named was gone.

use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::hooks::{STANDIN_REPOS, repo_root};

/// The comment line every hook this installer writes carries. The installer
/// uses it to tell its own output apart from a hand-written hook, so the
/// guards have to agree with it byte for byte.
const INSTALLER_MARKER: &str = "Installed by libviprs-tests/tools/install-hooks.sh";

/// A throwaway workspace with every sibling repo the installer expects and a
/// copy of the installer itself, so it can be run for real without touching a
/// developer's checkouts. It removes hooks it recognises as its own, so
/// running it against the live workspace from a test would be destructive.
///
/// The repo list is shared with `tests/common/hooks.rs` rather than written
/// out again here. It used to be a local copy of four names, and when the
/// installer grew three more repos the copy did not: the stand-in workspace
/// simply never created them, the installer skipped them as "not a git repo",
/// and the table below asserted nothing about the three repos that had just
/// started getting hooks.
struct Workspace {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Workspace {
    fn new() -> Workspace {
        let dir = tempfile::tempdir().expect("temp dir for a stand-in workspace");
        let root = dir.path().canonicalize().expect("canonical temp path");

        for repo in STANDIN_REPOS {
            let repo_root = root.join(repo);
            std::fs::create_dir_all(&repo_root).expect("create a stand-in repo");
            git(&repo_root, &["init", "-q"]);
        }

        // The installer resolves the workspace from its own location, so it
        // has to be run from inside the stand-in harness rather than from the
        // real one.
        let tools = root.join("libviprs-tests/tools");
        std::fs::create_dir_all(&tools).expect("create the stand-in tools directory");
        std::fs::create_dir_all(tools.join("hooks")).expect("create the stand-in hooks directory");
        // The installer points the shim it writes at tools/hooks/pre-push and
        // refuses to install without it, so the stand-in harness carries the
        // real one (libviprs/libviprs#695).
        for script in [
            "install-hooks.sh",
            "run-tests.sh",
            "run_ported_cells.sh",
            "hooks/pre-push",
        ] {
            let to = tools.join(script);
            std::fs::copy(repo_root().join("tools").join(script), &to)
                .unwrap_or_else(|e| panic!("copy tools/{script} into the stand-in workspace: {e}"));
            make_executable(&to);
        }

        Workspace { _dir: dir, root }
    }

    /// Run the installer the way a developer does, and hand back everything it
    /// printed.
    fn install(&self) -> String {
        let out = Command::new("bash")
            .arg(self.root.join("libviprs-tests/tools/install-hooks.sh"))
            .current_dir(self.root.join("libviprs-tests"))
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("run tools/install-hooks.sh");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success(),
            "tools/install-hooks.sh exited {}:\n{text}",
            out.status
        );
        text
    }

    fn hook(&self, repo: &str, name: &str) -> PathBuf {
        self.root.join(repo).join(".git/hooks").join(name)
    }

    fn write_hook(&self, repo: &str, name: &str, body: &str) {
        let path = self.hook(repo, name);
        std::fs::create_dir_all(path.parent().expect("hooks dir")).expect("create hooks dir");
        std::fs::write(&path, body).expect("write a stand-in hook");
        make_executable(&path);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "these guards drive the real installer, which needs git on PATH: \
                 `git {}` in {}: {e}",
                args.join(" "),
                dir.display()
            )
        });
    assert!(
        out.status.success(),
        "git {} failed in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make a stand-in script executable");
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// #691: the pre-push gate only goes where the suite it runs can see the
/// repo. `run-tests.sh` builds an image holding the core crate and this
/// harness, so those two get it and the other two do not.
///
/// The pre-commit hook is a different story and stays everywhere: it runs the
/// repo's own `cargo fmt` and `cargo clippy` in the repo it is installed in,
/// so for the cli it gates the cli.
#[test]
fn a_pre_push_hook_only_lands_where_the_suite_has_a_slot() {
    let ws = Workspace::new();
    let printed = ws.install();

    let expected: &[(&str, bool, &str)] = &[
        (
            "libviprs",
            true,
            "run-tests.sh builds the core crate, so the gate can fail on a core change",
        ),
        (
            "libviprs-tests",
            true,
            "run-tests.sh builds this harness, so the gate can fail on a harness change",
        ),
        (
            "libviprs-cli",
            false,
            "the suite has no slot for the cli, so the gate would report a verdict \
             independent of the commits going out (#691)",
        ),
        (
            "pdfium-render",
            false,
            "the fork has no run-tests.sh integration at all",
        ),
        (
            "libviprs-bench",
            false,
            "the image holds no bench crate, so the gate could not fail on a bench change",
        ),
        (
            "libviprs-org",
            false,
            "the image holds no doc site, and the site's own checks are all in its pre-commit hook",
        ),
        (
            "libviprs-dep",
            false,
            "the image holds none of the pdfium build inputs, and that repo's checks are python and shell",
        ),
    ];

    // A repo the installer visits and this table does not name gets no
    // assertion at all, and no assertion reads to `cargo test` as a pass.
    let named: std::collections::BTreeSet<&str> = expected.iter().map(|(r, _, _)| *r).collect();
    let visited: std::collections::BTreeSet<&str> = STANDIN_REPOS.iter().copied().collect();
    assert_eq!(
        named, visited,
        "this table and the repos the installer visits have gone out of step, \
         so some repo is getting hooks with nothing here saying which. The \
         installer said:\n{printed}"
    );

    for (repo, wants_pre_push, why) in expected {
        assert!(
            ws.hook(repo, "pre-commit").is_file(),
            "{repo} got no pre-commit hook, and that one is per-repo and cheap. \
             The installer said:\n{printed}"
        );
        assert_eq!(
            ws.hook(repo, "pre-push").is_file(),
            *wants_pre_push,
            "{repo} must {} a pre-push hook, because {why}. The installer said:\n{printed}",
            if *wants_pre_push { "get" } else { "not get" }
        );
    }
}

/// The installer has to clear a pre-push hook an older copy of itself put in
/// a repo that no longer gets one. Every clone that ran the old installer is
/// still carrying that hook, and it does not go away on its own: nothing but
/// this script ever writes to `.git/hooks`, so "stop writing it" leaves the
/// misleading gate running everywhere it already landed.
#[test]
fn a_previously_installed_pre_push_is_cleared_from_a_repo_with_no_slot() {
    let ws = Workspace::new();
    ws.write_hook(
        "libviprs-cli",
        "pre-push",
        &format!("#!/usr/bin/env bash\n# {INSTALLER_MARKER}\nexit 0\n"),
    );

    let printed = ws.install();

    assert!(
        !ws.hook("libviprs-cli", "pre-push").is_file(),
        "the installer left an older copy of its own pre-push hook in \
         libviprs-cli. A cli push still gates on a suite that cannot see the \
         cli, which is #691 surviving the fix. The installer said:\n{printed}"
    );
}

/// Clearing is scoped to hooks this installer wrote. Somebody else's pre-push
/// hook is not ours to remove, and removing it silently would be a worse bug
/// than the one being fixed.
#[test]
fn a_hand_written_pre_push_hook_is_left_alone() {
    let ws = Workspace::new();
    let body = "#!/bin/sh\n# mine, not the installer's\nexit 0\n";
    ws.write_hook("libviprs-cli", "pre-push", body);

    let printed = ws.install();

    let path = ws.hook("libviprs-cli", "pre-push");
    assert!(
        path.is_file(),
        "the installer removed a pre-push hook it did not write. It said:\n{printed}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read the hand-written hook"),
        body,
        "the installer rewrote a pre-push hook it did not write. It said:\n{printed}"
    );
}

/// The other half of the coupling, so the two facts cannot drift apart: the
/// cli gets no pre-push *because* the suite has no cli slot. Ask the suite
/// directly rather than asserting it about a script.
///
/// If someone gives `run-tests.sh` a cli tree (option 1 on #691: a `CLI_DIR`,
/// a `COPY libviprs-cli/` in the Dockerfile, `VIPRS_CLI_DIR` and
/// `VIPRS_REQUIRE_CLI=1` so the 18 differential binaries run against the
/// pushed tree) this goes red, and the fix is to give the cli its pre-push
/// arm back rather than to delete this.
#[test]
fn the_suite_still_has_no_slot_for_the_cli() {
    let script = repo_root().join("tools/run-tests.sh");

    let rejected = Command::new("bash")
        .arg(&script)
        .args(["--plan", "--libviprs-cli", "/nonexistent"])
        .output()
        .expect("run tools/run-tests.sh --plan --libviprs-cli");
    assert!(
        !rejected.status.success(),
        "tools/run-tests.sh now accepts a cli tree, so the suite has a cli slot \
         and libviprs-cli should get the pre-push gate back (#691). It said:\n{}",
        String::from_utf8_lossy(&rejected.stdout)
    );

    let plan = Command::new("bash")
        .arg(&script)
        .arg("--plan")
        .output()
        .expect("run tools/run-tests.sh --plan");
    assert!(
        plan.status.success(),
        "`run-tests.sh --plan` failed: {}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan = String::from_utf8_lossy(&plan.stdout);
    assert!(
        !plan.contains("libviprs-cli:"),
        "tools/run-tests.sh plans a cli tree, so the suite has a cli slot and \
         libviprs-cli should get the pre-push gate back (#691). It planned:\n{plan}"
    );
}
