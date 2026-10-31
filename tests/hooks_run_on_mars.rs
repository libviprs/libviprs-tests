//! The git hooks run on MARS and never on the machine that is committing
//! (libviprs-tests#311).
//!
//! The hooks used to run cargo, clippy and docker on the Mac. That broke the
//! rule that this work happens in MARS containers, made a hook's verdict
//! depend on the Mac's state (the sibling `../libviprs` it happened to have
//! checked out, a case-insensitive bind mount carrying untracked files), and
//! left no honest way to push a test-first commit that is red on purpose, so
//! `--no-verify` became normal practice.
//!
//! Nothing here reaches a real MARS. `ssh` is a stand-in on `PATH` that records
//! what it was asked to run and keeps whatever was piped to it, and `cargo`,
//! `docker` and friends are stand-ins that record being run on THIS machine,
//! which is the thing that must not happen. The real `tools/mars-run.sh` and
//! the real installed hooks are driven end to end, so what is asserted is
//! behaviour: what was shipped, where it would have run, what came back.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod common;
use common::hooks::{Workspace, git, make_executable};

/// Every tool the old hooks ran on the host. Running any of these here is the
/// failure, so each one records itself into `host.log`.
const HOST_TOOLS: &[&str] = &[
    "cargo",
    "rustc",
    "rustfmt",
    "cargo-clippy",
    "clippy-driver",
    "docker",
    "shellcheck",
    "python3-local-ci",
];

/// A stand-in `ssh`. Modes (`FAKE_SSH_MODE`): `down` fails the way an
/// unreachable host does (255). Otherwise it answers the preflight and image
/// probes, keeps the tar piped into a run, and exits `FAKE_SSH_RUN_RC` for the
/// run itself.
const FAKE_SSH: &str = r#"#!/bin/sh
# the remote command is the last argument
for last; do :; done
echo "$last" >> "$FAKE_DIR/ssh.log"
[ "${FAKE_SSH_MODE:-}" = down ] && exit 255
case "$last" in
  *"docker info"*) exit 0 ;;
  *"docker image inspect"*) exit 0 ;;
  *"docker ps -q"*) exit 0 ;;
  *"docker run --rm -i --name viprs-hook-"*)
    n=$(ls "$FAKE_DIR" | grep -c '^shipped-')
    cat > "$FAKE_DIR/shipped-$n.tar"
    echo "$last" > "$FAKE_DIR/run-$n.cmd"
    exit "${FAKE_SSH_RUN_RC:-0}" ;;
  *"tar -xi -f - -C"*)
    n=$(ls "$FAKE_DIR" | grep -c '^shipped-')
    cat > "$FAKE_DIR/shipped-$n.tar"
    exit 0 ;;
  *"run-tests.sh"*)
    echo "$last" > "$FAKE_DIR/gate.cmd"
    exit "${FAKE_SSH_RUN_RC:-0}" ;;
  *) exit 0 ;;
esac
"#;

struct Mars {
    ws: Workspace,
    dir: PathBuf,
    bin: PathBuf,
}

impl Mars {
    fn new() -> Mars {
        let ws = Workspace::new();
        let dir = ws.root.join("fake-mars");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("create the stand-in bin dir");
        let ssh = bin.join("ssh");
        std::fs::write(&ssh, FAKE_SSH).expect("write the stand-in ssh");
        make_executable(&ssh);
        for tool in HOST_TOOLS {
            let path = bin.join(tool);
            std::fs::write(
                &path,
                "#!/bin/sh\necho \"$0 $*\" >> \"$FAKE_DIR/host.log\"\nexit 0\n",
            )
            .expect("write a host-tool recorder");
            make_executable(&path);
        }
        Mars { ws, dir, bin }
    }

    fn path(&self) -> String {
        format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    /// Run `.git/hooks/<hook>` of `repo` the way git does, from the top of the
    /// repo, with the stand-ins in front and any extra environment.
    fn commit_hook(&self, repo: &str, env: &[(&str, &str)]) -> (bool, String) {
        let root = self.ws.repo(repo);
        let mut cmd = Command::new("bash");
        cmd.arg(root.join(".git/hooks/pre-commit"))
            .current_dir(&root)
            .env("PATH", self.path())
            .env("FAKE_DIR", &self.dir)
            .env_remove("LIBVIPRS_ON_MARS")
            .env_remove("LIBVIPRS_HOOKS_SKIP_MARS")
            .env_remove("LIBVIPRS_HOOKS_MARS_RUN")
            .env_remove("LIBVIPRS_HOOK_TREEISH")
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run the installed pre-commit hook");
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    /// A push of `repo` from its main checkout, through the real pre-push shim
    /// and the real `mars-run.sh`, with `ssh` standing in for the connection.
    fn push(&self, repo: &str, before: &str, after: &str, env: &[(&str, &str)]) -> (bool, String) {
        let root = self.ws.repo(repo);
        let mut cmd = Command::new("bash");
        cmd.arg(root.join(".git/hooks/pre-push"))
            .arg("origin")
            .arg("git@example.invalid:libviprs/x.git")
            .current_dir(&root)
            .env("PATH", self.path())
            .env("FAKE_DIR", &self.dir)
            .env_remove("LIBVIPRS_ON_MARS")
            .env_remove("LIBVIPRS_HOOKS_SKIP_MARS")
            .env_remove("LIBVIPRS_HOOKS_MARS_RUN")
            .env_remove("LIBVIPRS_PREPUSH_ALL")
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("run the installed pre-push hook");
        {
            use std::io::Write;
            let stdin = child.stdin.as_mut().expect("hook stdin");
            writeln!(stdin, "refs/heads/main {after} refs/heads/main {before}")
                .expect("feed the ref update");
        }
        let out = child.wait_with_output().expect("collect the push verdict");
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.join(name)).unwrap_or_default()
    }

    /// The tar the run was handed, as `tar -t` lists it.
    fn shipped_listing(&self, n: usize) -> String {
        let out = Command::new("tar")
            .arg("-tf")
            .arg(self.dir.join(format!("shipped-{n}.tar")))
            .output()
            .expect("list the shipped tar");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// One file out of the shipped tar. A tar of two concatenated archives
    /// (what the runner sends) still reads here, because `-i` skips the
    /// end-of-archive blocks in the middle.
    fn shipped_file(&self, n: usize, member: &str) -> String {
        let out = Command::new("tar")
            .arg("-xiOf")
            .arg(self.dir.join(format!("shipped-{n}.tar")))
            .arg(member)
            .output()
            .expect("read a member of the shipped tar");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

fn stage(repo: &Path, rel: &str, contents: &str) {
    let path = repo.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, contents).expect("write");
    git(repo, &["add", rel]);
}

// ---------------------------------------------------------------------------
// Pre-commit
// ---------------------------------------------------------------------------

/// The reason for all of this: a commit runs no cargo, clippy, docker or
/// shellcheck here. It ships the tree to MARS and the run happens there, in a
/// container, with the target dir where binaries can exec.
#[test]
fn a_commit_runs_its_checks_on_mars_and_nothing_on_this_machine() {
    let m = Mars::new();
    stage(&m.ws.repo("libviprs"), "src/lib.rs", "// staged\n");

    let (ok, out) = m.commit_hook("libviprs", &[]);
    assert!(
        ok,
        "the hook failed although the stand-in MARS passed:\n{out}"
    );

    assert_eq!(
        m.read("host.log"),
        "",
        "a tool ran on this machine. The hook must hand everything to MARS:\n{out}"
    );
    let run = m.read("run-0.cmd");
    assert!(!run.is_empty(), "nothing was run on MARS:\n{out}");
    for needle in [
        "docker run --rm",
        "--platform linux/amd64",
        "CARGO_TARGET_DIR=/tmp/target",
        "viprs-hook-target-libviprs",
        "viprs-hooks:",
    ] {
        assert!(run.contains(needle), "the MARS run lacks `{needle}`: {run}");
    }
}

/// What is checked is what is staged. A working-tree edit that is not staged,
/// and a file that is not tracked, must not be what the verdict is about: that
/// is how a case-insensitive bind mount passed a tree CI rejected.
#[test]
fn what_ships_is_the_staged_tree_not_the_working_tree() {
    let m = Mars::new();
    let repo = m.ws.repo("libviprs");
    stage(&repo, "src/lib.rs", "// staged version\n");
    std::fs::write(repo.join("src/lib.rs"), "// edited after staging\n").unwrap();
    std::fs::write(repo.join("untracked.txt"), "never added\n").unwrap();

    let (ok, out) = m.commit_hook("libviprs", &[]);
    assert!(ok, "{out}");

    assert_eq!(
        m.shipped_file(0, "libviprs/src/lib.rs"),
        "// staged version\n",
        "the working-tree edit was shipped instead of what is staged"
    );
    assert!(
        !m.shipped_listing(0).contains("untracked.txt"),
        "an untracked file was shipped, so the verdict depends on files git will not commit"
    );
}

/// The hook file that runs on MARS is the one that was installed, byte for
/// byte, so the commands are the same on both sides.
#[test]
fn the_text_that_runs_on_mars_is_the_installed_hook() {
    let m = Mars::new();
    let (ok, out) = m.commit_hook("libviprs", &[]);
    assert!(ok, "{out}");

    let run = m.read("run-0.cmd");
    let b64 = run
        .split("MARS_HOOK_B64=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .expect("the run carries the hook text");
    let decoded = Command::new("sh")
        .arg("-c")
        .arg(format!("printf %s '{b64}' | base64 -d"))
        .output()
        .expect("decode the hook text");
    let installed =
        std::fs::read(m.ws.repo("libviprs").join(".git/hooks/pre-commit")).expect("read the hook");
    assert_eq!(
        decoded.stdout, installed,
        "MARS ran a different text from the hook that was installed"
    );
}

/// An unreachable MARS stops the commit and says why and how to override. It
/// does not pass, and it does not fall back to running cargo here.
#[test]
fn an_unreachable_mars_stops_the_commit_and_names_the_override() {
    let m = Mars::new();
    let (ok, out) = m.commit_hook("libviprs-tests", &[("FAKE_SSH_MODE", "down")]);
    assert!(!ok, "the hook passed with MARS unreachable:\n{out}");
    assert!(
        out.contains("cannot reach MARS"),
        "it did not say MARS was the problem:\n{out}"
    );
    assert!(
        out.contains("LIBVIPRS_HOOKS_SKIP_MARS=1"),
        "it did not name the override:\n{out}"
    );
    assert_eq!(
        m.read("host.log"),
        "",
        "it fell back to running a tool here:\n{out}"
    );
}

/// The override is explicit and loud, and it still runs nothing here.
#[test]
fn the_override_skips_loudly_and_runs_nothing() {
    let m = Mars::new();
    let (ok, out) = m.commit_hook(
        "libviprs-bench",
        &[("FAKE_SSH_MODE", "down"), ("LIBVIPRS_HOOKS_SKIP_MARS", "1")],
    );
    assert!(ok, "the override did not let the commit through:\n{out}");
    assert!(out.contains("SKIPPED"), "the skip was quiet:\n{out}");
    assert_eq!(
        m.read("host.log"),
        "",
        "the override ran something here:\n{out}"
    );
    assert_eq!(
        m.read("ssh.log"),
        "",
        "the override still tried to reach MARS:\n{out}"
    );
}

/// What the run on MARS decides is what the hook decides.
#[test]
fn a_failure_on_mars_fails_the_commit() {
    let m = Mars::new();
    let (ok, out) = m.commit_hook("libviprs", &[("FAKE_SSH_RUN_RC", "1")]);
    assert!(!ok, "a red run on MARS let the commit through:\n{out}");
}

/// The harness builds against the core as a path dependency. It has to be the
/// core CI would use (COUNTERPART_REV), not whatever the sibling checkout has.
#[test]
fn the_harness_is_checked_against_the_pinned_core_not_the_sibling_checkout() {
    let m = Mars::new();
    let core = m.ws.repo("libviprs");
    let pinned =
        m.ws.commit("libviprs", "pinned core", &[("src/lib.rs", "// pinned\n")]);
    // The sibling moves on after the pin, and has an uncommitted edit too.
    m.ws.commit("libviprs", "later core", &[("src/lib.rs", "// later\n")]);
    std::fs::write(core.join("src/lib.rs"), "// dirty\n").unwrap();

    let tests = m.ws.repo("libviprs-tests");
    stage(&tests, "COUNTERPART_REV", &format!("# pin\n{pinned}\n"));

    let (ok, out) = m.commit_hook("libviprs-tests", &[]);
    assert!(ok, "{out}");
    assert_eq!(
        m.shipped_file(0, "libviprs/src/lib.rs"),
        "// pinned\n",
        "the core on MARS is not the one COUNTERPART_REV names"
    );
}

/// The installer says which repos run on MARS, and it is these three.
#[test]
fn the_installer_names_the_repos_that_run_on_mars() {
    let out = Command::new("bash")
        .arg(common::hooks::repo_root().join("tools/install-hooks.sh"))
        .arg("--describe")
        .output()
        .expect("run the installer");
    let text = String::from_utf8_lossy(&out.stdout);
    for repo in ["libviprs", "libviprs-tests", "libviprs-bench"] {
        assert!(
            text.lines().any(|l| l == format!("mars\t{repo}\tyes")),
            "{repo} is not marked as running on MARS:\n{text}"
        );
    }
    assert!(
        text.lines().any(|l| l == "mars\tlibviprs-cli\tno"),
        "libviprs-cli is another change's and must not be switched here:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// Pre-push and test-first commits
// ---------------------------------------------------------------------------

/// A push runs the suite on MARS: the trees go over, run-tests.sh runs in a
/// container with the docker socket (it builds an image), and the scratch
/// directory is removed afterwards. Nothing runs here.
#[test]
fn a_push_runs_the_suite_on_mars_and_cleans_up() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit("libviprs", "change", &[("src/a.rs", "// b\n")]);

    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(
        ok,
        "the push was refused although the stand-in MARS passed:\n{out}"
    );
    assert_eq!(
        m.read("host.log"),
        "",
        "something ran on this machine:\n{out}"
    );

    let gate = m.read("gate.cmd");
    for needle in [
        "docker.sock",
        "--platform linux/amd64",
        "tools/run-tests.sh",
        "LIBVIPRS_DIR=",
    ] {
        assert!(
            gate.contains(needle),
            "the suite run lacks `{needle}`: {gate}"
        );
    }
    assert_eq!(
        m.shipped_file(0, "libviprs/src/a.rs"),
        "// b\n",
        "the pushed tree is not what went to MARS"
    );
    assert!(
        m.read("ssh.log").contains("rm -rf /w/hook-gate-"),
        "the scratch directory on MARS was not removed:\n{}",
        m.read("ssh.log")
    );
}

#[test]
fn a_push_with_mars_unreachable_is_refused_and_says_how_to_override() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit("libviprs", "change", &[("src/a.rs", "// b\n")]);

    let (ok, out) = m.push("libviprs", &before, &after, &[("FAKE_SSH_MODE", "down")]);
    assert!(!ok, "the push went through with MARS unreachable:\n{out}");
    assert!(
        out.contains("LIBVIPRS_HOOKS_SKIP_MARS=1"),
        "no override named:\n{out}"
    );

    let (ok, out) = m.push(
        "libviprs",
        &before,
        &after,
        &[("FAKE_SSH_MODE", "down"), ("LIBVIPRS_HOOKS_SKIP_MARS", "1")],
    );
    assert!(ok, "the override did not let the push through:\n{out}");
    assert!(out.contains("SKIPPED"), "the skip was quiet:\n{out}");
}

/// The test-first commit. Failing tests written before the fix are the only
/// proof the fix addressed the cause, so they have to be pushable. A tip with a
/// `Red-tests:` trailer gets the compile-and-lint half on MARS and not the
/// suite.
#[test]
fn a_red_tests_tip_gets_the_lint_half_and_not_the_suite() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit(
        "libviprs",
        "Red tests for the thing\n\nRed-tests: #311",
        &[("tests/red.rs", "#[test] fn red() { panic!() }\n")],
    );

    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "a red-tests tip was refused:\n{out}");
    assert_eq!(
        m.read("gate.cmd"),
        "",
        "the suite ran for a red-tests tip:\n{out}"
    );
    assert!(
        m.read("run-0.cmd").contains("viprs-hook-libviprs"),
        "the compile-and-lint half did not run on MARS:\n{out}"
    );
    assert_eq!(
        m.shipped_file(0, "libviprs/tests/red.rs"),
        "#[test] fn red() { panic!() }\n",
        "the commit being pushed is not what the lint half checked"
    );
}

/// Only the tip counts, so the fix on top of the red commit brings the full
/// gate back with no one having to remember to.
#[test]
fn the_fix_on_top_of_a_red_commit_gets_the_full_suite() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    m.ws.commit(
        "libviprs",
        "Red tests\n\nRed-tests: #311",
        &[("tests/red.rs", "// red\n")],
    );
    let after =
        m.ws.commit("libviprs", "The fix", &[("src/a.rs", "// fixed\n")]);

    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    assert!(
        m.read("gate.cmd").contains("tools/run-tests.sh"),
        "the suite did not run for a tip with no trailer:\n{out}"
    );
}

/// An empty trailer is not a reason, and a lookalike line in the middle of the
/// body is not a trailer.
#[test]
fn a_trailer_with_nothing_after_it_does_not_skip_the_suite() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit(
        "libviprs",
        "Change\n\nRed-tests:",
        &[("src/a.rs", "// b\n")],
    );
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    assert!(
        m.read("gate.cmd").contains("tools/run-tests.sh"),
        "an empty Red-tests: skipped the suite:\n{out}"
    );
}

/// A sentence in the body that happens to start with `Red-tests:` is prose, not
/// a trailer. Only the last paragraph of the message counts, which is where
/// trailers go. The first version of the hook matched any line and took the
/// commit that introduced it for a red one.
#[test]
fn a_red_tests_line_in_the_middle_of_the_body_is_prose_not_a_trailer() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit(
        "libviprs",
        "Change\n\nRed-tests: is the trailer a test-first commit carries, and\nthis paragraph only talks about it.\n\nCloses #1",
        &[("src/a.rs", "// b\n")],
    );
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    assert!(
        m.read("gate.cmd").contains("tools/run-tests.sh"),
        "prose that starts with Red-tests: skipped the suite:\n{out}"
    );
}
