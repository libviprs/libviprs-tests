//! `tools/run-tests.sh` has to leave the Docker host the way it found it, and
//! it has to queue behind other runs instead of piling on (issue #248).
//!
//! # The failure this exists to stop
//!
//! The script builds `libviprs-tests:local-<tree tag>` on every run. It used to
//! never remove it, so every rebuild moved the tag and left the previous image
//! behind untagged, and the run container only went away on the path where
//! `docker run` returned normally. With ten lanes gating at once on one laptop,
//! Docker Desktop's 196.6 GB disk went to 100% and every pre-push in this repo
//! died with `No space left on device`. Nothing capped how many runs built at
//! once, either, so all ten were holding an image and a target dir at the same
//! time.
//!
//! # How these run without a daemon
//!
//! Every test here drives the real script, but with a stand-in `docker` first
//! on PATH. The stand-in keeps images and containers as files in a temp dir and
//! behaves like the real CLI where it matters: `build` moves an existing tag and
//! leaves the old image behind untagged, `run` leaves its container behind
//! (there is no `--rm`), and `rmi` refuses an image a container still uses. So
//! "what is left on the daemon afterwards" is a directory listing, and the tests
//! run the same in the CI image, which has no daemon, as on a laptop.
//!
//! The trees under test are stand-ins too: a core tree with a `Cargo.toml` and
//! a harness tree with a `Dockerfile` and a reference-suite fetch that does
//! nothing, which is everything the script reads before it hands off to Docker.

use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The stand-in `docker`. State lives under `$FAKE_DOCKER_STATE`:
///
/// * `images/<tag>` one file per tagged image, holding its id
/// * `images/untagged-<id>` an image whose tag moved on to a newer build
/// * `containers/<name>` one file per container, holding its image tag
/// * `log` every invocation, one line each
///
/// `build` also records the `LIBVIPRS_DOCKER_SLOT` it was started under in
/// `slot-at-build`.
///
/// Knobs: `FAKE_DOCKER_BUILD_FAIL=1` makes `build` fail, `FAKE_DOCKER_RUN_EXIT`
/// is what `run` exits with, and while `$FAKE_DOCKER_STATE/hold` exists `run`
/// writes `$FAKE_DOCKER_STATE/running` and waits.
const FAKE_DOCKER: &str = r#"#!/usr/bin/env bash
set -u
S="${FAKE_DOCKER_STATE:?}"
mkdir -p "$S/images" "$S/containers"
echo "$*" >> "$S/log"
key() { printf '%s' "$1" | tr '/:' '__'; }
new_id() { n=$(cat "$S/next_id" 2>/dev/null || echo 1); echo $((n + 1)) > "$S/next_id"; echo "img$n"; }
cmd="${1:-}"; shift || true
case "$cmd" in
  info)
    case "$*" in
      *MemTotal*) echo 17179869184 ;;
      *NCPU*) echo 4 ;;
    esac
    exit 0 ;;
  ps)
    ls "$S/containers"
    exit 0 ;;
  build)
    tag=""
    while [ $# -gt 0 ]; do
      case "$1" in -t) tag="$2"; shift ;; esac
      shift
    done
    if [ "${FAKE_DOCKER_BUILD_FAIL:-}" = 1 ]; then
      echo "ERROR: failed to solve: process did not complete successfully" >&2
      exit 1
    fi
    k=$(key "$tag")
    if [ -f "$S/images/$k" ]; then
      old=$(cat "$S/images/$k")
      mv "$S/images/$k" "$S/images/untagged-$old"
    fi
    new_id > "$S/images/$k"
    printf '%s\n' "${LIBVIPRS_DOCKER_SLOT:-}" > "$S/slot-at-build"
    exit 0 ;;
  run)
    name=""; image=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --name) name="$2"; shift ;;
        --name=*) name="${1#--name=}" ;;
        -e|-v|--platform) shift ;;
        -*) ;;
        *) image="$1" ;;
      esac
      shift
    done
    if [ ! -f "$S/images/$(key "$image")" ]; then
      echo "Unable to find image '$image' locally" >&2
      exit 125
    fi
    echo "$image" > "$S/containers/$name"
    if [ -e "$S/hold" ]; then
      : > "$S/running"
      i=0
      while [ -e "$S/hold" ] && [ $i -lt 1200 ]; do sleep 0.05; i=$((i + 1)); done
    fi
    exit "${FAKE_DOCKER_RUN_EXIT:-0}" ;;
  inspect)
    for a in "$@"; do last="$a"; done
    [ -f "$S/containers/${last:-}" ] || exit 1
    echo false
    exit 0 ;;
  rm)
    rc=0
    for a in "$@"; do
      case "$a" in -*) continue ;; esac
      if [ -f "$S/containers/$a" ]; then rm -f "$S/containers/$a"
      else echo "Error response from daemon: No such container: $a" >&2; rc=1; fi
    done
    exit $rc ;;
  rmi|image)
    if [ "$cmd" = image ]; then
      sub="${1:-}"; shift || true
      case "$sub" in
        inspect)
          for a in "$@"; do case "$a" in -*) ;; *) t="$a" ;; esac; done
          [ -f "$S/images/$(key "${t:-}")" ] || exit 1
          cat "$S/images/$(key "$t")"
          exit 0 ;;
        rm) ;;
        *) echo "fake docker: unsupported: image $sub" >&2; exit 2 ;;
      esac
    fi
    rc=0
    for a in "$@"; do
      case "$a" in
        -f|--force) echo "fake docker: rmi -f is not something run-tests.sh should need" >&2; exit 2 ;;
        -*) continue ;;
      esac
      k=$(key "$a")
      if [ ! -f "$S/images/$k" ]; then
        echo "Error response from daemon: No such image: $a" >&2; rc=1; continue
      fi
      if grep -qxF "$a" "$S"/containers/* 2>/dev/null; then
        echo "Error response from daemon: conflict: unable to remove $a (in use by a container)" >&2; rc=1; continue
      fi
      rm -f "$S/images/$k"
    done
    exit $rc ;;
  *)
    echo "fake docker: unsupported command: $cmd $*" >&2
    exit 2 ;;
esac
"#;

fn tests_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path).expect("stat").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod +x");
}

/// One sandbox: a stand-in daemon, stand-in trees, and a private context and
/// slot directory, so nothing here touches the developer's real ones.
struct Sandbox {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        let dir = tempfile::tempdir().expect("temp dir for the sandbox");
        let root = dir.path().canonicalize().expect("canonical temp path");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).expect("create bin");
        fs::write(bin.join("docker"), FAKE_DOCKER).expect("write the stand-in docker");
        make_executable(&bin.join("docker"));
        fs::create_dir_all(root.join("slots")).expect("create the slot dir");
        Sandbox { _dir: dir, root }
    }

    /// A stand-in pair of trees named `name`. Different names give different
    /// tree paths, and so a different image tag and container name.
    fn trees(&self, name: &str) -> (PathBuf, PathBuf) {
        let core = self.root.join(name).join("libviprs");
        let harness = self.root.join(name).join("libviprs-tests");
        fs::create_dir_all(core.join("src")).expect("create the core tree");
        fs::write(
            core.join("Cargo.toml"),
            "[package]\nname = \"fake\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        fs::create_dir_all(harness.join("tools")).expect("create the harness tree");
        fs::write(harness.join("Dockerfile"), "FROM scratch\n").expect("write Dockerfile");
        let fetch = harness.join("tools/fetch_reference_suite.sh");
        fs::write(&fetch, "#!/bin/sh\nexit 0\n").expect("write the fetch stand-in");
        make_executable(&fetch);
        (core, harness)
    }

    fn state(&self, name: &str) -> PathBuf {
        self.root.join(name).join("docker-state")
    }

    fn slots(&self) -> PathBuf {
        self.root.join("slots")
    }

    /// The script, set up to run over the trees called `name` against that
    /// pair's own stand-in daemon state.
    fn command(&self, name: &str) -> Command {
        let (core, harness) = self.trees(name);
        let state = self.state(name);
        fs::create_dir_all(&state).expect("create the daemon state dir");
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![self.root.join("bin")];
        dirs.extend(std::env::split_paths(&path));

        let mut cmd = Command::new("bash");
        cmd.arg(tests_root().join("tools/run-tests.sh"))
            .arg("amd64")
            .arg("--libviprs")
            .arg(&core)
            .arg("--libviprs-tests")
            .arg(&harness)
            .env(
                "PATH",
                std::env::join_paths(dirs).expect("a PATH with the stand-in"),
            )
            .env("FAKE_DOCKER_STATE", &state)
            .env(
                "RUN_TESTS_CONTEXT_DIR",
                self.root.join(name).join("context"),
            )
            .env("RUN_TESTS_SLOT_DIR", self.slots())
            .env("RUN_TESTS_SLOT_POLL", "0.1")
            .env_remove("RUN_TESTS_KEEP_IMAGE")
            .env_remove("RUN_TESTS_MAX_PARALLEL")
            .env_remove("RUN_TESTS_IMAGE_NAME")
            .env_remove("RUN_TESTS_CONTAINER_NAME")
            .env_remove("LIBVIPRS_DOCKER_SLOT")
            .env_remove("LIBVIPRS_DOCKER_SLOT_DIR")
            .env_remove("LIBVIPRS_DOCKER_MAX_PARALLEL")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// Images the stand-in daemon still holds for the pair `name`, tagged and
    /// untagged.
    fn images(&self, name: &str) -> Vec<String> {
        list(&self.state(name).join("images"))
    }

    fn containers(&self, name: &str) -> Vec<String> {
        list(&self.state(name).join("containers"))
    }

    fn log(&self, name: &str) -> String {
        fs::read_to_string(self.state(name).join("log")).unwrap_or_default()
    }
}

fn list(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

fn text(out: &Output) -> String {
    format!(
        "status: {}\n--- stdout\n{}\n--- stderr\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Waits for a child with a deadline, so a regression that makes the script
/// hang fails the test instead of hanging the suite.
fn wait_with_deadline(mut child: Child, secs: u64, what: &str) -> Output {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        if child.try_wait().expect("poll the child").is_some() {
            return child
                .wait_with_output()
                .expect("collect the child's output");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let out = child
                .wait_with_output()
                .expect("collect the child's output");
            panic!("{what} did not finish within {secs}s\n{}", text(&out));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run(cmd: &mut Command, what: &str) -> Output {
    wait_with_deadline(cmd.spawn().expect("spawn run-tests.sh"), 60, what)
}

fn wait_for(path: &Path, secs: u64, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Removes a `hold` file when dropped, so a failing assertion cannot leave a
/// stand-in `docker run` waiting for the full minute.
struct Release(PathBuf);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn nothing_left_behind(sb: &Sandbox, name: &str, out: &Output, situation: &str) {
    let images = sb.images(name);
    let containers = sb.containers(name);
    assert!(
        images.is_empty() && containers.is_empty(),
        "{situation}, run-tests.sh left images {images:?} and containers {containers:?} \
         on the daemon.\n\nEvery one of those is gigabytes on a laptop, and ten lanes \
         gating at once filled a 196.6 GB Docker disk this way (#248). The run has to \
         remove its own container and image on every way out, success, failure and \
         interrupt alike.\n\ndocker calls:\n{}\n{}",
        sb.log(name),
        text(out)
    );
}

/// The happy path: tests pass, and nothing of the run stays on the daemon.
#[test]
fn a_passing_run_leaves_no_image_and_no_container() {
    let sb = Sandbox::new();
    let out = run(&mut sb.command("a"), "a passing run");
    assert!(out.status.success(), "the run should pass\n{}", text(&out));
    nothing_left_behind(&sb, "a", &out, "after a passing run");
}

/// A failing test run takes the other exit arm, which used to remove the
/// container and keep the image.
#[test]
fn a_failing_run_leaves_no_image_and_no_container() {
    let sb = Sandbox::new();
    let out = run(
        sb.command("a").env("FAKE_DOCKER_RUN_EXIT", "101"),
        "a failing run",
    );
    assert_eq!(
        out.status.code(),
        Some(101),
        "the run should report the test failure's exit code\n{}",
        text(&out)
    );
    nothing_left_behind(&sb, "a", &out, "after a failing run");
}

/// A build that fails partway. `set -e` takes the script out at the build line,
/// so anything cleaned up only further down never happens. The daemon already
/// holds the image an earlier run left under this tag, which is exactly the
/// state every checkout was in when the disk filled.
#[test]
fn a_failed_build_leaves_nothing_behind() {
    let sb = Sandbox::new();
    let first = run(
        sb.command("a").env("RUN_TESTS_KEEP_IMAGE", "1"),
        "a run that keeps its image",
    );
    assert!(first.status.success(), "{}", text(&first));
    assert_eq!(
        sb.images("a").len(),
        1,
        "setup: the first run should leave one image\n{}",
        sb.log("a")
    );

    let out = run(
        sb.command("a").env("FAKE_DOCKER_BUILD_FAIL", "1"),
        "a run whose build fails",
    );
    assert!(
        !out.status.success(),
        "a failed build has to fail the run\n{}",
        text(&out)
    );
    nothing_left_behind(&sb, "a", &out, "after a failed build");
}

/// A rebuild over a tag that already exists. Docker moves the tag and keeps
/// the old image untagged, which is invisible in `docker images` output unless
/// you ask for dangling ones, and was most of the pile.
#[test]
fn a_rebuild_does_not_orphan_the_previous_image() {
    let sb = Sandbox::new();
    for i in 0..2 {
        let out = run(
            sb.command("a").env("RUN_TESTS_KEEP_IMAGE", "1"),
            "a run that keeps its image",
        );
        assert!(out.status.success(), "run {i}\n{}", text(&out));
    }
    let untagged: Vec<String> = sb
        .images("a")
        .into_iter()
        .filter(|i| i.starts_with("untagged-"))
        .collect();
    assert!(
        untagged.is_empty(),
        "two runs over the same trees left untagged images {untagged:?}.\n\n\
         The second build moved the tag and orphaned the first image. Remove what \
         the tag points at before building, so a rebuild replaces the image rather \
         than adding one (#248). BuildKit's cache outlives the image, so the build \
         stays warm.\n\ndocker calls:\n{}",
        sb.log("a")
    );
    assert_eq!(
        sb.images("a").len(),
        1,
        "RUN_TESTS_KEEP_IMAGE=1 should still leave the newest image\n{}",
        sb.log("a")
    );
}

/// The opt-out: somebody debugging the image wants it kept. The container
/// still goes, since it is useless once the run is over.
#[test]
fn keep_image_keeps_the_image_and_still_removes_the_container() {
    let sb = Sandbox::new();
    let out = run(
        sb.command("a").env("RUN_TESTS_KEEP_IMAGE", "1"),
        "a run that keeps its image",
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        sb.images("a").len(),
        1,
        "RUN_TESTS_KEEP_IMAGE=1 has to leave the image in place\n{}\n{}",
        sb.log("a"),
        text(&out)
    );
    assert!(
        sb.containers("a").is_empty(),
        "the container should go even when the image is kept: {:?}\n{}",
        sb.containers("a"),
        sb.log("a")
    );
}

/// Ctrl-C mid-run. The terminal signals the whole process group, the docker
/// CLI goes down with it, and the script used to die right behind it with its
/// container and image still on the daemon.
#[test]
fn an_interrupted_run_leaves_nothing_behind() {
    let sb = Sandbox::new();
    let state = sb.state("a");
    fs::create_dir_all(&state).expect("create the daemon state dir");
    fs::write(state.join("hold"), "").expect("write hold");
    let _release = Release(state.join("hold"));

    let mut cmd = sb.command("a");
    cmd.process_group(0);
    let child = cmd.spawn().expect("spawn run-tests.sh");
    let pgid = child.id();
    wait_for(&state.join("running"), 30, "the run to reach `docker run`");

    // The shell's own `kill`, not procps': `/usr/bin/kill -TERM -<pgid>` can
    // take the negative pid for an option and signal nothing.
    let killed = Command::new("sh")
        .arg("-c")
        .arg("kill -s TERM -- \"-$1\"")
        .arg("sh")
        .arg(pgid.to_string())
        .status()
        .expect("run kill");
    assert!(killed.success(), "could not signal the run's process group");
    let out = wait_with_deadline(child, 60, "an interrupted run");
    assert!(
        !out.status.success(),
        "an interrupted run must not report success\n{}",
        text(&out)
    );
    nothing_left_behind(&sb, "a", &out, "after an interrupted run");
}

/// With one slot, a second run waits for the first to finish before it builds
/// anything. Every run holds a couple of GB of image and more of build output
/// while it is going, so the number running at once is what decides whether a
/// laptop's Docker disk survives a busy afternoon (#248).
#[test]
fn a_second_run_waits_for_a_free_slot() {
    let sb = Sandbox::new();
    let a_state = sb.state("a");
    fs::create_dir_all(&a_state).expect("create the daemon state dir");
    fs::write(a_state.join("hold"), "").expect("write hold");
    let release = Release(a_state.join("hold"));

    let first = sb
        .command("a")
        .env("RUN_TESTS_MAX_PARALLEL", "1")
        .spawn()
        .expect("spawn the first run");
    wait_for(
        &a_state.join("running"),
        30,
        "the first run to reach `docker run`",
    );

    let second = sb
        .command("b")
        .env("RUN_TESTS_MAX_PARALLEL", "1")
        .spawn()
        .expect("spawn the second run");

    // Long enough for an uncapped second run to stage, build and finish
    // against the stand-in daemon several times over.
    std::thread::sleep(Duration::from_secs(3));
    let built_early = sb.log("b").lines().any(|l| l.starts_with("build"));

    drop(release);
    let first_out = wait_with_deadline(first, 60, "the first run");
    let second_out = wait_with_deadline(second, 60, "the second run");

    assert!(
        !built_early,
        "with RUN_TESTS_MAX_PARALLEL=1 the second run built its image while the \
         first was still running.\n\nNothing caps how many runs share the daemon, \
         so ten lanes pushing at once all build and hold an image together, which \
         is what filled the disk (#248).\n\nsecond run's docker calls:\n{}\n{}",
        sb.log("b"),
        text(&second_out)
    );
    assert!(first_out.status.success(), "{}", text(&first_out));
    assert!(
        second_out.status.success(),
        "the second run should go ahead once the slot is free\n{}",
        text(&second_out)
    );
    assert!(
        list(&sb.slots()).is_empty(),
        "both runs are over, so no slot should still be held: {:?}",
        list(&sb.slots())
    );
}

/// A run killed hard (SIGKILL, a crashed laptop) never gets to release its
/// slot. The next run has to notice the holder is gone and take the slot over,
/// or a single crash would wedge every later push behind it.
#[test]
fn a_slot_held_by_a_dead_process_is_taken_over() {
    let sb = Sandbox::new();
    let mut gone = Command::new("true")
        .spawn()
        .expect("spawn a short-lived process");
    let dead_pid = gone.id();
    gone.wait().expect("reap it");

    let slot = sb.slots().join("slot1");
    fs::create_dir_all(&slot).expect("create a stale slot");
    fs::write(slot.join("pid"), format!("{dead_pid}\n")).expect("write the stale pid");

    let out = run(
        sb.command("a").env("RUN_TESTS_MAX_PARALLEL", "1"),
        "a run behind a stale slot",
    );
    assert!(out.status.success(), "{}", text(&out));
    let still_stale = fs::read_to_string(slot.join("pid"))
        .map(|p| p.trim() == dead_pid.to_string())
        .unwrap_or(false);
    assert!(
        !still_stale && list(&sb.slots()).is_empty(),
        "after the run the slot directory holds {:?} (slot1 stale: {still_stale}).\n\n\
         The run should have taken over the slot whose owner (pid {dead_pid}) is gone \
         and released it when it finished, so the next run finds it free.\n{}",
        list(&sb.slots()),
        text(&out)
    );
}

/// `--plan` resolves names and prints them, and must never touch the daemon or
/// take a slot: it is what a hook, or a person, runs to check what a gate is
/// about to do, and it runs in the CI image where there is no daemon.
#[test]
fn plan_touches_neither_docker_nor_the_slots() {
    let sb = Sandbox::new();
    let out = run(sb.command("a").arg("--plan"), "a --plan run");
    assert!(out.status.success(), "{}", text(&out));
    assert!(
        !sb.log("a").lines().any(|l| !l.starts_with("info")),
        "--plan made docker calls beyond `docker info`:\n{}",
        sb.log("a")
    );
    assert!(list(&sb.slots()).is_empty(), "--plan took a slot");
}

/// A caller that already holds a slot (the workspace's push wrapper, or a hook
/// that took one before it called this script) names it in
/// `LIBVIPRS_DOCKER_SLOT`. The run is already counted, and queueing for a
/// second slot would deadlock the moment the cap is one: the caller waits for
/// this run, and this run waits for the caller's slot.
#[test]
fn a_run_under_a_caller_holding_a_slot_does_not_queue_for_another() {
    let sb = Sandbox::new();
    let slot = sb.slots().join("slot1");
    fs::create_dir_all(&slot).expect("create the caller's slot");
    fs::write(slot.join("pid"), format!("{}\n", std::process::id()))
        .expect("write the caller's pid");

    let out = run(
        sb.command("a")
            .env("RUN_TESTS_MAX_PARALLEL", "1")
            .env("LIBVIPRS_DOCKER_SLOT", &slot),
        "a run under a caller that holds the only slot",
    );
    assert!(
        out.status.success(),
        "with the only slot held by its caller (LIBVIPRS_DOCKER_SLOT={}), the \
         run should go ahead rather than wait for a second one\n{}",
        slot.display(),
        text(&out)
    );
    assert_eq!(
        fs::read_to_string(slot.join("pid"))
            .unwrap_or_default()
            .trim(),
        std::process::id().to_string(),
        "the run released a slot that belongs to its caller"
    );
}

/// The pool is one per machine and shared with core's local-ci.py, so a run
/// takes its slot from `LIBVIPRS_DOCKER_SLOT_DIR` when nothing more specific is
/// set, and tells what it starts which slot it holds, so a hook further down
/// does not take a second one.
#[test]
fn the_slot_comes_from_the_shared_pool_and_is_handed_down() {
    let sb = Sandbox::new();
    let shared = sb.root.join("shared-pool");
    let out = run(
        sb.command("a")
            .env_remove("RUN_TESTS_SLOT_DIR")
            .env("LIBVIPRS_DOCKER_SLOT_DIR", &shared),
        "a run against the shared pool",
    );
    assert!(out.status.success(), "{}", text(&out));
    let handed_down = fs::read_to_string(sb.state("a").join("slot-at-build"))
        .unwrap_or_default()
        .trim()
        .to_string();
    assert!(
        handed_down.starts_with(&shared.display().to_string()),
        "`docker build` ran with LIBVIPRS_DOCKER_SLOT={handed_down:?}, expected a \
         slot under the shared pool {}.\n\nThe slots have to come from the one \
         pool every Docker-starting hook uses, and the holder has to say so to \
         what it starts, or each script caps only itself.\n{}",
        shared.display(),
        text(&out)
    );
    assert!(
        list(&shared).is_empty(),
        "the run is over, so its slot should be released: {:?}",
        list(&shared)
    );
}
