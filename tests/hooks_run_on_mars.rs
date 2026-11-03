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
  *"docker ps -q --filter name=hook-gate-"*)
    i=0; while [ "$i" -lt "${FAKE_GATES_RUNNING:-0}" ]; do echo "gate$i"; i=$((i+1)); done; exit 0 ;;
  *"docker ps --filter name=hook-gate-"*)
    i=0; while [ "$i" -lt "${FAKE_GATES_RUNNING:-0}" ]; do echo "hook-gate-$i"; i=$((i+1)); done; exit 0 ;;
  *"docker volume ls -q --filter name=viprs-hook-"*"viprs.hook.path"*)
    printf '%s' "${FAKE_VOLUMES:-}"; exit 0 ;;
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
        // run-tests.sh builds with BuildKit, and the docker client mounted in
        // from the host finds buildx only in its plugin directory.
        "cli-plugins",
        // rsync is what lets run-tests.sh stage the refs, and so the tags.
        "viprs-hooks:",
        // The 4g default OOMs streaming_pdfium_matrix on MARS.
        "RUN_TESTS_MEMORY=8g",
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

/// The trees go over as `git archive` output, which has no `.git`, and
/// run-tests.sh stages the index and the refs from one into its build
/// context: the guards that ask git what is tracked fail without a repository
/// (case_only_path_collisions did, on the first real run). So the run on MARS
/// has to turn each tree into a repository, and keep the tags the real one has.
#[test]
fn the_shipped_trees_become_repositories_with_their_tags() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    git(&m.ws.repo("libviprs"), &["tag", "v0.1.0", &before]);
    let after = m.ws.commit("libviprs", "change", &[("src/a.rs", "// b\n")]);

    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");

    let ssh = m.read("ssh.log");
    assert!(
        ssh.contains("git init") && ssh.contains("git add -A"),
        "the shipped trees were left without a .git:\n{ssh}"
    );
    assert!(
        ssh.contains("v0.1.0"),
        "the core's tags did not go over, so the changelog guard sees none:\n{ssh}"
    );
}

/// A core push is gated against the harness at origin/main, not against
/// whatever branch the sibling libviprs-tests checkout is on. The sibling is
/// someone's working branch with its own, older Dockerfile, and on the first
/// real core push it failed to build the image for a reason that has nothing
/// to do with the core.
#[test]
fn a_core_push_gates_against_the_harness_at_origin_main() {
    let m = Mars::new();
    let tests = m.ws.repo("libviprs-tests");
    let on_main =
        m.ws.commit("libviprs-tests", "on main", &[("MARKER", "main\n")]);
    git(
        &tests,
        &["update-ref", "refs/remotes/origin/main", &on_main],
    );
    // The sibling checkout has moved on to a branch of its own.
    m.ws.commit(
        "libviprs-tests",
        "somebody's branch",
        &[("MARKER", "branch\n")],
    );

    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit("libviprs", "change", &[("src/a.rs", "// b\n")]);
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");

    assert_eq!(
        m.shipped_file(0, "libviprs-tests/MARKER"),
        "main\n",
        "the harness that went to MARS is the sibling checkout, not origin/main"
    );
}

/// A harness push is gated against the core its COUNTERPART_REV pins, which is
/// what hosted CI and the merge gate pair it with, and not against whatever the
/// sibling core checkout has. The sibling's main moves on without the harness
/// (and was red in its own suite when this was found), so gating against it
/// blames the harness for the core.
#[test]
fn a_harness_push_gates_against_the_core_it_pins() {
    let m = Mars::new();
    let pinned =
        m.ws.commit("libviprs", "pinned core", &[("src/lib.rs", "// pinned\n")]);
    m.ws.commit("libviprs", "later core", &[("src/lib.rs", "// later\n")]);

    let before = m.ws.commit(
        "libviprs-tests",
        "base",
        &[("COUNTERPART_REV", "# pin\n0000000\n")],
    );
    let after = m.ws.commit(
        "libviprs-tests",
        "bump the pin",
        &[("COUNTERPART_REV", &format!("# pin\n{pinned}\n"))],
    );
    let (ok, out) = m.push("libviprs-tests", &before, &after, &[]);
    assert!(ok, "{out}");

    assert_eq!(
        m.shipped_file(0, "libviprs/src/lib.rs"),
        "// pinned\n",
        "the core that went to MARS is the sibling checkout, not COUNTERPART_REV"
    );
}

/// MARS is shared, and a full suite run is a docker build plus a test run. With
/// every agent's pushes going through the same hook, the runs have to queue
/// rather than all start at once: the first full runs on a busy MARS were
/// OOM-killed and starved each other for tens of minutes.
#[test]
fn a_push_waits_for_a_free_suite_slot_instead_of_piling_on() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit("libviprs", "change", &[("src/a.rs", "// b\n")]);

    let (ok, out) = m.push(
        "libviprs",
        &before,
        &after,
        &[("FAKE_GATES_RUNNING", "2"), ("LIBVIPRS_HOOKS_WAIT", "0")],
    );
    assert!(!ok, "a push started a third suite run on MARS:\n{out}");
    assert!(
        out.contains("giving up"),
        "it did not say it gave up waiting:\n{out}"
    );
    assert_eq!(
        m.read("gate.cmd"),
        "",
        "the suite ran although no slot was free:\n{out}"
    );

    let (ok, out) = m.push(
        "libviprs",
        &before,
        &after,
        &[("FAKE_GATES_RUNNING", "1"), ("LIBVIPRS_HOOKS_WAIT", "0")],
    );
    assert!(ok, "a free slot was not used:\n{out}");
}

// ---------------------------------------------------------------------------
// Concurrent runs (libviprs-tests#317)
// ---------------------------------------------------------------------------

/// The container name of a recorded `docker run ... --name NAME`.
fn container_name(run: &str) -> String {
    let after = run.split("--name ").nth(1).unwrap_or_default();
    after
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Every hook of a repo used to run in a container called
/// `viprs-hook-<repo>`, so a second commit from another worktree, or a second
/// agent, collided with the first. Each run gets its own name: the repo, the
/// worktree and the pid.
#[test]
fn two_hook_runs_of_one_repo_get_different_container_names() {
    let m = Mars::new();
    stage(&m.ws.repo("libviprs"), "src/lib.rs", "// staged\n");

    let (ok, out) = m.commit_hook("libviprs", &[]);
    assert!(ok, "{out}");
    let (ok, out) = m.commit_hook("libviprs", &[]);
    assert!(ok, "{out}");

    let first = container_name(&m.read("run-0.cmd"));
    let second = container_name(&m.read("run-1.cmd"));
    assert!(
        first.starts_with("viprs-hook-libviprs-"),
        "the name does not carry the repo and a suffix: {first:?}"
    );
    assert_ne!(first, second, "two runs share the container name {first}");
}

/// A run from a linked worktree names that worktree, so two lanes of one repo
/// cannot take the same name even if their pids ever matched.
#[test]
fn a_hook_run_from_a_worktree_names_the_worktree() {
    let m = Mars::new();
    let tree = m.ws.worktree("libviprs", "lane-317");
    stage(&tree, "src/lib.rs", "// staged\n");

    let root = m.ws.repo("libviprs");
    let out = Command::new("bash")
        .arg(root.join(".git/hooks/pre-commit"))
        .current_dir(&tree)
        .env("PATH", m.path())
        .env("FAKE_DIR", &m.dir)
        .env_remove("LIBVIPRS_ON_MARS")
        .env_remove("LIBVIPRS_HOOKS_SKIP_MARS")
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("run the hook from the worktree");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let name = container_name(&m.read("run-0.cmd"));
    assert!(
        name.contains("lane-317"),
        "the container name does not carry the worktree: {name:?}"
    );
}

/// The volume mounted at `/tmp/target` in a recorded `docker run`.
fn target_volume(run: &str) -> String {
    let after = run
        .split("-v ")
        .find(|p| p.contains(":/tmp/target"))
        .unwrap_or_default();
    after.split(':').next().unwrap_or_default().to_string()
}

/// libviprs-tests#349: the build cache was one volume per repo while sources
/// were per worktree, so a worktree with older sources than another's build
/// was treated as already built. The target volume is per worktree too.
#[test]
fn issue_349_two_worktrees_get_different_target_volumes() {
    let m = Mars::new();
    let root = m.ws.repo("libviprs");
    let mut vols = Vec::new();
    for (i, name) in ["lane-349a", "lane-349b"].iter().enumerate() {
        let tree = m.ws.worktree("libviprs", name);
        stage(&tree, "src/lib.rs", "// staged\n");
        let out = Command::new("bash")
            .arg(root.join(".git/hooks/pre-commit"))
            .current_dir(&tree)
            .env("PATH", m.path())
            .env("FAKE_DIR", &m.dir)
            .env_remove("LIBVIPRS_ON_MARS")
            .env_remove("LIBVIPRS_HOOKS_SKIP_MARS")
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .expect("run the hook from the worktree");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        vols.push(target_volume(&m.read(&format!("run-{i}.cmd"))));
    }
    assert!(
        vols[0].starts_with("viprs-hook-target-libviprs"),
        "{vols:?}"
    );
    assert_ne!(
        vols[0], vols[1],
        "two worktrees share the target volume {}",
        vols[0]
    );
    assert!(
        vols[0].contains("lane-349a"),
        "the target volume does not name its worktree: {vols:?}"
    );
}

/// libviprs-tests#360: `clean --stale [DAYS]` drops the per-worktree volumes (src, sib, target) that nothing
/// holds and whose worktree is gone or whose age is at least DAYS, and nothing else. The stand-in ssh answers
/// the listing from `FAKE_VOLUMES`, one `name|labelled path|age in days|containers holding it` per line.
#[test]
fn issue_360_clean_stale_drops_only_the_stale_unheld_volumes() {
    let m = Mars::new();
    let here = m.dir.display().to_string();
    let listing = format!(
        "viprs-hook-target-r-gone-1|/no/such/worktree-360|1|0\n\
         viprs-hook-src-r-old-2|{here}|40|0\n\
         viprs-hook-sib-r-fresh-3|{here}|2|0\n\
         viprs-hook-target-r-held-4|/no/such/worktree-360|90|1\n\
         viprs-hook-cargo|{here}|400|0\n"
    );
    let out = Command::new("bash")
        .arg(m.ws.repo("libviprs-tests").join("tools/mars-run.sh"))
        .args(["clean", "--stale", "14"])
        .env("PATH", m.path())
        .env("FAKE_DIR", &m.dir)
        .env("FAKE_VOLUMES", listing)
        .output()
        .expect("run clean --stale");
    let log = m.read("ssh.log");
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for gone in ["viprs-hook-target-r-gone-1", "viprs-hook-src-r-old-2"] {
        assert!(
            log.contains(&format!("docker volume rm {gone}")),
            "{gone} was not removed:\n{log}"
        );
    }
    for kept in [
        "viprs-hook-sib-r-fresh-3",
        "viprs-hook-target-r-held-4",
        "viprs-hook-cargo",
    ] {
        assert!(
            !log.contains(&format!("docker volume rm {kept}")),
            "{kept} was removed:\n{log}"
        );
    }
}

/// A hook run labels the volumes it creates with its worktree path, which is what `--stale` reads.
#[test]
fn issue_360_a_hook_run_labels_its_volumes_with_the_worktree() {
    let m = Mars::new();
    stage(&m.ws.repo("libviprs"), "src/lib.rs", "// staged\n");
    let (ok, out) = m.commit_hook("libviprs", &[]);
    assert!(ok, "{out}");
    let log = m.read("ssh.log");
    assert!(
        log.contains("docker volume create --label viprs.hook.path="),
        "no labelled volume create in the ssh log:\n{log}"
    );
}

/// A run must not wait for, or be killed by, another run's container.
#[test]
fn a_hook_run_does_not_wait_for_another_runs_container() {
    let m = Mars::new();
    stage(&m.ws.repo("libviprs"), "src/lib.rs", "// staged\n");
    let (ok, out) = m.commit_hook("libviprs", &[("LIBVIPRS_HOOKS_WAIT", "0")]);
    assert!(ok, "{out}");
    let log = m.read("ssh.log");
    assert!(
        !log.contains("--filter name=^viprs-hook-libviprs$"),
        "the run still waits on the shared container name:\n{log}"
    );
}

/// `set -u` and an empty argument list: `mars-run.sh gate` with nothing after
/// it is the full-suite push, and it died with "args: unbound variable".
#[test]
fn the_gate_with_no_arguments_does_not_trip_set_u() {
    let m = Mars::new();
    let core = m.ws.repo("libviprs");
    let tests = m.ws.repo("libviprs-tests");
    m.ws.commit("libviprs", "c", &[("src/a.rs", "// a\n")]);
    let script =
        m.ws.checked_in_hook()
            .parent()
            .expect("hooks dir")
            .join("../mars-run.sh");

    for shell in ["bash", "/bin/bash"] {
        let out = Command::new(shell)
            .arg(&script)
            .arg("gate")
            .env("PATH", m.path())
            .env("FAKE_DIR", &m.dir)
            .env("LIBVIPRS_DIR", &core)
            .env("LIBVIPRS_TESTS_DIR", &tests)
            .env_remove("LIBVIPRS_HOOKS_SKIP_MARS")
            .output()
            .expect("run mars-run.sh gate");
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(!text.contains("unbound variable"), "{shell}: {text}");
        assert!(out.status.success(), "{shell}: {text}");
    }
}

/// Waiting for a suite slot says who holds the slots, so a stuck queue can be
/// read instead of guessed at.
#[test]
fn waiting_for_a_slot_names_the_runs_that_hold_them() {
    let m = Mars::new();
    let before = m.ws.commit("libviprs", "base", &[("src/a.rs", "// a\n")]);
    let after = m.ws.commit("libviprs", "change", &[("src/a.rs", "// b\n")]);
    let (ok, out) = m.push(
        "libviprs",
        &before,
        &after,
        &[("FAKE_GATES_RUNNING", "2"), ("LIBVIPRS_HOOKS_WAIT", "0")],
    );
    assert!(!ok, "{out}");
    assert!(
        out.contains("hook-gate-0") && out.contains("hook-gate-1"),
        "the message does not name the holders:\n{out}"
    );
}
