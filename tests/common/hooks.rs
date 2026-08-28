//! The pre-push hook, and a stand-in workspace for driving it.
//!
//! The hook is a real tracked file, `tools/hooks/pre-push`
//! (libviprs/libviprs#695). It used to be a heredoc inside
//! `tools/install-hooks.sh`, which meant the only thing a guard could do with
//! it was grep it, and two guards that did exactly that passed while the
//! behaviour they named had been deleted. So nothing here asserts on the
//! hook's text. [`Workspace`] builds a throwaway workspace of stand-in repos,
//! runs `tools/install-hooks.sh` in it for real, and drives the installed hook
//! with a synthetic ref-update on stdin, the way git does.
//!
//! Two suites use it: `prepush_gate_tests_the_pushed_tree` for #684 and
//! `prepush_gate_cost_controls` for #683.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// This crate's root, which is also the repo root for the harness checkout.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Read a repo-relative file, naming the path if it is not there.
pub fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The hook, relative to the repo root.
pub const PRE_PUSH_HOOK: &str = "tools/hooks/pre-push";

/// The pre-push hook body, read from the file that *is* the hook rather than
/// from a generator that writes one.
pub fn pre_push_hook() -> String {
    let path = repo_root().join(PRE_PUSH_HOOK);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\nThe pre-push hook is a tracked file and \
             tools/install-hooks.sh installs a shim that runs it, rather than \
             generating a copy of it (libviprs/libviprs#695).",
            path.display()
        )
    })
}

/// What the stub `run-tests.sh` prints when the hook decides to run the suite.
pub const RAN_MARKER: &str = "STUB-RAN-THE-SUITE";

/// A stand-in for `run-tests.sh` that reports the environment the hook handed
/// it, so both halves of a decision (did it run, and what did it run against)
/// are readable off one push without Docker.
pub const STUB_RUN_TESTS: &str = r#"#!/bin/sh
echo "STUB-RAN-THE-SUITE"
echo "STUB-SCRIPT=$0"
echo "STUB-LIBVIPRS_DIR=${LIBVIPRS_DIR-unset}"
echo "STUB-LIBVIPRS_TESTS_DIR=${LIBVIPRS_TESTS_DIR-unset}"
echo "STUB-GIT_DIR=${GIT_DIR-unset}"
echo "STUB-GIT_WORK_TREE=${GIT_WORK_TREE-unset}"
echo "STUB-GIT_INDEX_FILE=${GIT_INDEX_FILE-unset}"
echo "STUB-GIT_PREFIX=${GIT_PREFIX-unset}"
echo "STUB-GIT_QUARANTINE_PATH=${GIT_QUARANTINE_PATH-unset}"
"#;

/// One `STUB-<name>=` line out of what a push printed.
pub fn reported(out: &str, name: &str) -> String {
    let prefix = format!("STUB-{name}=");
    out.lines()
        .find_map(|line| line.trim().strip_prefix(&prefix))
        .unwrap_or_else(|| {
            panic!("the stub suite never reported {name}, so it did not run:\n{out}")
        })
        .to_string()
}

/// A throwaway workspace holding the two repos the hook expects as siblings,
/// with the hooks installed by the real `tools/install-hooks.sh` and a stub in
/// place of `run-tests.sh`.
///
/// The evidence that #683 and #684 work was rows of skip/run decisions
/// produced by hand. For changes whose failure mode is the gate silently
/// ceasing to run, or silently gating the wrong tree, by hand and uncommitted
/// is the wrong place for it.
pub struct Workspace {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn new() -> Workspace {
        let dir = tempfile::tempdir().expect("temp dir for a stand-in workspace");
        let root = dir.path().canonicalize().expect("canonical temp path");

        // libviprs-cli gets no pre-push hook (libviprs/libviprs#691) but it
        // does get a pre-commit one, and #715's guard runs that, so it is here
        // too. Nothing pushes from it.
        for repo in ["libviprs", "libviprs-cli", "libviprs-tests"] {
            let repo_root = root.join(repo);
            std::fs::create_dir_all(&repo_root).expect("create the stand-in repo");
            git(&repo_root, &["init", "-q", "-b", "main"]);
        }

        // The harness carries both the installer and the hook it installs, so
        // both have to be here before the installer runs and committed before
        // a linked worktree can see them.
        let tests_root = root.join("libviprs-tests");
        std::fs::create_dir_all(tests_root.join("tools/hooks"))
            .expect("create the stand-in tools directory");
        for rel in ["tools/install-hooks.sh", PRE_PUSH_HOOK] {
            let to = tests_root.join(rel);
            let from = repo_root().join(rel);
            std::fs::copy(&from, &to).unwrap_or_else(|e| {
                panic!("copy {} into the stand-in workspace: {e}", from.display())
            });
            make_executable(&to);
        }
        let stub = tests_root.join("tools/run-tests.sh");
        std::fs::write(&stub, STUB_RUN_TESTS).expect("write the stub run-tests.sh");
        make_executable(&stub);

        let ws = Workspace { _dir: dir, root };
        ws.commit("libviprs-tests", "harness tooling", &[]);
        ws.commit(
            "libviprs",
            "stand-in core crate",
            &[("src/lib.rs", "// stand-in\n")],
        );
        ws.install_hooks();
        ws
    }

    /// Run `tools/install-hooks.sh` from inside the stand-in harness, the way
    /// a developer does, and hand back everything it printed.
    pub fn install_hooks(&self) -> String {
        let out = Command::new("bash")
            .arg(self.repo("libviprs-tests").join("tools/install-hooks.sh"))
            .current_dir(self.repo("libviprs-tests"))
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

    pub fn repo(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The checked-in hook inside the stand-in harness, which is what the
    /// installed shim points at.
    pub fn checked_in_hook(&self) -> PathBuf {
        self.repo("libviprs-tests").join(PRE_PUSH_HOOK)
    }

    /// A linked worktree of `repo`, on a new branch, somewhere that is not a
    /// sibling of the checkouts. Epic #520 runs every lane in one of these,
    /// and it is the case the shared hooks directory gets wrong.
    pub fn worktree(&self, repo: &str, branch: &str) -> PathBuf {
        let path = self.root.join("lanes").join(branch);
        std::fs::create_dir_all(path.parent().expect("lanes dir")).expect("create the lanes dir");
        git(
            &self.repo(repo),
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                branch,
                path.to_str().expect("utf-8 worktree path"),
            ],
        );
        path.canonicalize().expect("canonical worktree path")
    }

    /// Commit `files` in `tree` and return the commit's oid. `tree` can be a
    /// main checkout or a linked worktree.
    pub fn commit_in(&self, tree: &Path, message: &str, files: &[(&str, &str)]) -> String {
        for (rel, contents) in files {
            let path = tree.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create a directory for a stand-in file");
            }
            std::fs::write(&path, contents).expect("write a stand-in file");
        }
        git(tree, &["add", "-A"]);
        git(
            tree,
            &[
                "-c",
                "user.name=prepush guard",
                "-c",
                "user.email=guard@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                // These guards are about the pre-push hook. The pre-commit
                // hook the installer also writes runs `cargo fmt` in the repo
                // it is installed in, and a stand-in repo is not a crate.
                "--no-verify",
                "--allow-empty",
                "-m",
                message,
            ],
        );
        git(tree, &["rev-parse", "HEAD"]).trim().to_string()
    }

    /// The same, on a repo's main checkout.
    pub fn commit(&self, repo: &str, message: &str, files: &[(&str, &str)]) -> String {
        self.commit_in(&self.repo(repo), message, files)
    }

    /// Start describing a push. Nothing happens until [`Push::run`].
    pub fn push<'a>(&'a self, repo: &'a str) -> Push<'a> {
        Push {
            ws: self,
            repo,
            from: None,
            before: String::new(),
            after: String::new(),
            force_all: false,
            git_env: false,
        }
    }
}

/// One push at the installed hook.
pub struct Push<'a> {
    ws: &'a Workspace,
    repo: &'a str,
    from: Option<PathBuf>,
    before: String,
    after: String,
    force_all: bool,
    git_env: bool,
}

impl<'a> Push<'a> {
    /// Push from a linked worktree rather than from the main checkout. git
    /// still runs the hook out of the main checkout's shared hooks directory,
    /// which is the whole of #684.
    pub fn from(mut self, tree: &Path) -> Push<'a> {
        self.from = Some(tree.to_path_buf());
        self
    }

    pub fn range(mut self, before: &str, after: &str) -> Push<'a> {
        self.before = before.to_string();
        self.after = after.to_string();
        self
    }

    pub fn force_all(mut self) -> Push<'a> {
        self.force_all = true;
        self
    }

    /// Hand the hook the git environment git itself exports into it. It beats
    /// `git -C` and the working directory for every git command anything
    /// downstream runs, so a hook that does not drop it makes the suite answer
    /// for the pushing repository whatever tree it was aimed at.
    pub fn with_git_env(mut self) -> Push<'a> {
        self.git_env = true;
        self
    }

    /// Run the hook and require it to allow the push.
    pub fn run(self) -> String {
        let (ok, text) = self.try_run();
        assert!(ok, "the pre-push hook rejected the push:\n{text}");
        text
    }

    /// Run the hook and hand back whether it allowed the push, and everything
    /// it printed.
    pub fn try_run(self) -> (bool, String) {
        let main = self.ws.repo(self.repo);
        let tree = self.from.clone().unwrap_or_else(|| main.clone());

        let mut cmd = Command::new("bash");
        cmd.arg(main.join(".git/hooks/pre-push"))
            .arg("origin")
            .arg("git@example.invalid:libviprs/x.git")
            .current_dir(&tree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for leaked in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_PREFIX",
            "GIT_QUARANTINE_PATH",
            "LIBVIPRS_PREPUSH_ALL",
        ] {
            cmd.env_remove(leaked);
        }
        if self.force_all {
            cmd.env("LIBVIPRS_PREPUSH_ALL", "1");
        }
        if self.git_env {
            let dir = git(&tree, &["rev-parse", "--absolute-git-dir"])
                .trim()
                .to_string();
            cmd.env("GIT_DIR", &dir)
                .env("GIT_INDEX_FILE", format!("{dir}/index"))
                .env("GIT_PREFIX", "")
                .env("GIT_QUARANTINE_PATH", format!("{dir}/incoming"));
        }

        let mut child = cmd.spawn().expect("run the installed pre-push hook");
        {
            let stdin = child.stdin.as_mut().expect("hook stdin");
            writeln!(
                stdin,
                "refs/heads/main {} refs/heads/main {}",
                self.after, self.before
            )
            .expect("feed the ref update to the hook");
        }
        let out = child
            .wait_with_output()
            .expect("collect the hook's decision");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.success(), text)
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
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
                "these guards drive the real hook, which needs git on PATH: \
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make the stand-in script executable");
    }
    #[cfg(not(unix))]
    let _ = path;
}
