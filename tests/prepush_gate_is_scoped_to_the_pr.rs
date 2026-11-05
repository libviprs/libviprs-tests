//! The pre-push gate runs only the tests that belong to the pushed PR
//! (libviprs-tests#370).
//!
//! The gate used to run the whole Docker suite on every push that was not
//! inert. The owner decision of 2026-11-04 is that a PR runs its own tests, so
//! the gate is scoped the same way, and it must never get there by running
//! nothing. This file holds the guards for that, and the stand-ins they drive
//! are real scripts run for real: no assertion here is made on a script's text.
//!
//! The container half lives in `tools/scoped-suite.sh`, which turns a plan
//! into `cargo test` invocations. `tools/run-tests.sh --scope` hands it the
//! plan. The selecting half is `tools/gate-scope.sh`, in the following change.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod common;
use common::hooks::repo_root;

/// The plan, the way `run-tests.sh --scope` carries it: base64 on one line.
fn encode(plan: &str) -> String {
    let mut child = Command::new("base64")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn base64");
    child
        .stdin
        .take()
        .expect("base64 stdin")
        .write_all(plan.as_bytes())
        .expect("feed base64");
    let out = child.wait_with_output().expect("run base64");
    String::from_utf8_lossy(&out.stdout)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// A container-shaped scratch tree: a `libviprs` and a `libviprs-tests`
/// directory, a recording `cargo`, and a recording ported-cells script. Every
/// command the scoped suite runs lands in `calls.log`, one line each, led by
/// the directory it ran in.
struct Container {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Container {
    fn new() -> Container {
        let dir = tempfile::tempdir().expect("scratch dir");
        let root = dir.path().canonicalize().expect("canonical scratch dir");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("bin dir");
        for sub in ["libviprs", "libviprs-tests/tools"] {
            std::fs::create_dir_all(root.join(sub)).expect("tree dir");
        }
        let recorder = |name: &str| {
            format!(
                "#!/bin/sh\necho \"$(basename \"$PWD\") {name} $*\" >> \"{}/calls.log\"\n",
                root.display()
            )
        };
        write_exec(&bin.join("cargo"), &recorder("cargo"));
        write_exec(
            &root.join("libviprs-tests/tools/run_ported_cells.sh"),
            &recorder("ported"),
        );
        Container { _dir: dir, root }
    }

    /// Run the scoped suite on `plan`. Returns the exit status, everything it
    /// printed, and the commands it ran.
    fn run(&self, scope: Option<&str>) -> (bool, String, String) {
        let mut cmd = Command::new("sh");
        cmd.arg(repo_root().join("tools/scoped-suite.sh"))
            .current_dir(&self.root)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("LIBVIPRS_SCOPE_CORE_DIR", self.root.join("libviprs"))
            .env("LIBVIPRS_SCOPE_TESTS_DIR", self.root.join("libviprs-tests"))
            .env_remove("LIBVIPRS_GATE_SCOPE");
        if let Some(scope) = scope {
            cmd.env("LIBVIPRS_GATE_SCOPE", scope);
        }
        let out = cmd.output().expect("run tools/scoped-suite.sh");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let calls = std::fs::read_to_string(self.root.join("calls.log")).unwrap_or_default();
        (out.status.success(), text, calls)
    }
}

fn write_exec(path: &Path, body: &str) {
    std::fs::write(path, body).expect("write a stand-in script");
    common::hooks::make_executable(path);
}

/// The container runs the plan and nothing else: each core target once, the
/// unit and doc filters, the harness targets, the ported cells, and the
/// required features of the tree they belong to. The build is split from the
/// run with a `sync` between, the way the full chain's is (libviprs#683).
#[test]
fn the_scoped_container_runs_exactly_the_plan() {
    let c = Container::new();
    let plan = "\
lint
core|test|x
core|test|y
core|lib|cancel::
core|doc|src/cancel
tests|test|z
features|core|a
features|tests|pvec-raster
ported
";
    let (ok, out, calls) = c.run(Some(&encode(plan)));
    assert!(ok, "the scoped suite failed on a good plan:\n{out}");

    let expected = "\
libviprs cargo test --features pdfium,a --no-run --lib --test x --test y
libviprs cargo test --features pdfium,a --test x --test y
libviprs cargo test --features pdfium,a --lib -- cancel::
libviprs cargo test --features pdfium,a --doc -- src/cancel
libviprs cargo clean
libviprs-tests cargo test --features pdfium,pvec-raster --no-run --test z
libviprs-tests cargo test --features pdfium,pvec-raster --test z
libviprs-tests ported
";
    assert_eq!(
        calls, expected,
        "the container ran something other than the plan:\n{out}"
    );
}

/// An empty or unreadable plan is a refusal. Scoping never silently runs
/// nothing, and the container is the last place that could.
#[test]
fn the_scoped_container_refuses_a_plan_that_runs_nothing() {
    let c = Container::new();
    for (what, scope) in [
        ("no plan at all", None),
        ("an empty plan", Some(encode(""))),
        ("a plan of only a lint line", Some(encode("lint\n"))),
        (
            "a plan that is not base64",
            Some("not base64 !!".to_string()),
        ),
        (
            "a line it does not know",
            Some(encode("core|test|x\nsurprise|y\n")),
        ),
        (
            "a target name with a glob in it",
            Some(encode("core|test|x*\n")),
        ),
    ] {
        let (ok, out, calls) = c.run(scope.as_deref());
        assert!(!ok, "{what} passed:\n{out}");
        assert_eq!(calls, "", "{what} still ran something:\n{out}");
        assert!(
            out.contains("scoped-suite:"),
            "{what} was refused without saying who refused it:\n{out}"
        );
    }
}

/// `run-tests.sh --scope` takes the plan, shows it in `--plan`, and refuses a
/// value it could not hand on. Callers that do not pass it see no change.
#[test]
fn run_tests_takes_a_scope_and_shows_it_in_the_plan() {
    let script = repo_root().join("tools/run-tests.sh");
    let plan = |args: &[&str]| {
        Command::new("bash")
            .arg(&script)
            .arg("--plan")
            .args(args)
            .output()
            .expect("run tools/run-tests.sh --plan")
    };

    let scoped = plan(&["--scope", &encode("core|test|x\ntests|test|z\n")]);
    let text = String::from_utf8_lossy(&scoped.stdout);
    assert!(
        scoped.status.success(),
        "--scope with a good plan was refused:\n{text}{}",
        String::from_utf8_lossy(&scoped.stderr)
    );
    for needle in ["scope:", "core|test|x", "tests|test|z"] {
        assert!(text.contains(needle), "--plan lacks `{needle}`:\n{text}");
    }

    let unscoped = plan(&[]);
    assert!(
        !String::from_utf8_lossy(&unscoped.stdout).contains("scope:"),
        "a run without --scope mentions a scope"
    );

    for bad in ["", "not base64 !!"] {
        let refused = plan(&["--scope", bad]);
        assert!(
            !refused.status.success(),
            "--scope {bad:?} was accepted:\n{}",
            String::from_utf8_lossy(&refused.stdout)
        );
    }
    let missing = plan(&["--scope"]);
    assert!(
        !missing.status.success(),
        "--scope with no value was accepted"
    );
}
