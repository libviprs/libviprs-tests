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
        // The ported-cells recorder has no arguments to echo, which leaves a
        // trailing space on its line.
        let calls = std::fs::read_to_string(self.root.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| format!("{}\n", l.trim_end()))
            .collect();
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
            "a target name that is a cargo flag",
            Some(encode("core|test|--release\n")),
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

// ===========================================================================
// Choosing the targets: tools/gate-scope.sh
// ===========================================================================
//
// `gate-scope.sh` is pure: the pushed repo, the changed paths, the two trees
// the gate is about to ship and the tip commit messages go in, and either
// `FULL|<why>` or a plan comes out. The trees are real git trees in stand-in
// repositories, because that is what `mars-run.sh` hands it.

use std::collections::BTreeSet;

use common::hooks::{Workspace, git};

const CORE_BASE: &[(&str, &str)] = &[
    ("Cargo.toml", "[package]\nname = \"libviprs\"\n"),
    (
        "src/lib.rs",
        "pub mod foo;\npub mod bar;\npub use foo::{Thing, make_thing};\n",
    ),
    ("src/foo.rs", "pub struct Thing;\n"),
    ("src/bar.rs", "// bar\n"),
    ("tests/miri_ignore_convention.rs", "// guard\n"),
    ("tests/unrelated.rs", "fn main() {}\n"),
];

const TESTS_BASE: &[(&str, &str)] = &[
    ("Cargo.toml", "[package]\nname = \"libviprs-tests\"\n"),
    ("tests/install_hooks_mirror_ci.rs", "// guard\n"),
    ("tests/unrelated.rs", "fn main() {}\n"),
];

/// A stand-in repository holding `files`, and the oid of its tree.
fn repo_with(dir: &Path, files: &[(&str, &str)]) -> String {
    std::fs::create_dir_all(dir).expect("create a stand-in repo");
    git(dir, &["init", "-q", "-b", "main"]);
    for (rel, contents) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, contents).expect("write a stand-in file");
    }
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "user.name=scope guard",
            "-c",
            "user.email=guard@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--no-verify",
            "--allow-empty",
            "-m",
            "stand-in",
        ],
    );
    git(dir, &["rev-parse", "HEAD^{tree}"]).trim().to_string()
}

/// What `gate-scope.sh` said.
struct Scope {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Scope {
    fn lines(&self) -> BTreeSet<String> {
        self.stdout.lines().map(str::to_string).collect()
    }

    /// The reason, when the answer is the full suite.
    fn full(&self) -> Option<String> {
        self.stdout
            .lines()
            .find_map(|l| l.strip_prefix("FULL|"))
            .map(str::to_string)
    }
}

fn set(lines: &[&str]) -> BTreeSet<String> {
    lines.iter().map(|s| s.to_string()).collect()
}

/// Run the selector for a push of `repo` (`libviprs` or `libviprs-tests`) that
/// changed `changed`, against stand-in trees holding the given files.
fn scope(
    repo: &str,
    changed: &[&str],
    core_files: &[(&str, &str)],
    tests_files: &[(&str, &str)],
    messages: &[&str],
    env: &[(&str, &str)],
) -> Scope {
    let dir = tempfile::tempdir().expect("scratch dir");
    let root = dir.path().canonicalize().expect("canonical scratch dir");
    let core = root.join("core");
    let tests = root.join("tests");
    let core_tree = repo_with(&core, core_files);
    let tests_tree = repo_with(&tests, tests_files);
    let changed_file = root.join("changed");
    std::fs::write(&changed_file, changed.join("\n") + "\n").expect("write the changed list");

    // Run from a directory that holds a file named like a trailer item, so a
    // trailer that is globbed instead of read as data would find a match.
    let cwd = root.join("cwd");
    std::fs::create_dir_all(&cwd).expect("create the working directory");
    std::fs::write(cwd.join("core:x"), "").expect("write a file a glob would match");
    let mut cmd = Command::new("bash");
    cmd.current_dir(&cwd)
        .arg(repo_root().join("tools/gate-scope.sh"))
        .args(["--repo", repo, "--changed"])
        .arg(&changed_file)
        .arg("--core-git")
        .arg(&core)
        .args(["--core-tree", &core_tree])
        .arg("--tests-git")
        .arg(&tests)
        .args(["--tests-tree", &tests_tree])
        .env_remove("LIBVIPRS_PREPUSH_ALL")
        .env_remove("LIBVIPRS_GATE_SCOPE_CAP");
    for (i, message) in messages.iter().enumerate() {
        let file = root.join(format!("message-{i}"));
        std::fs::write(&file, message).expect("write a tip message");
        cmd.arg("--message").arg(file);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run tools/gate-scope.sh");
    Scope {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn with(
    base: &[(&'static str, &'static str)],
    extra: &[(&'static str, &'static str)],
) -> Vec<(&'static str, &'static str)> {
    base.iter().chain(extra).copied().collect()
}

/// A test file that changed runs that target and nothing else of its own, plus
/// the guards that run on every scoped push. A shared helper under
/// `tests/common/` runs the targets that name it.
#[test]
fn a_changed_test_file_runs_only_that_target() {
    let core = with(
        CORE_BASE,
        &[
            ("tests/x.rs", "#[test] fn x() {}\n"),
            ("tests/y.rs", "#[test] fn y() {}\n"),
        ],
    );
    let s = scope("libviprs", &["tests/x.rs"], &core, TESTS_BASE, &[], &[]);
    assert!(s.ok, "{}", s.stderr);
    assert_eq!(
        s.lines(),
        set(&["core|test|miri_ignore_convention", "core|test|x", "lint"]),
        "stderr: {}",
        s.stderr
    );

    let core = with(
        CORE_BASE,
        &[
            ("tests/common/helper.rs", "pub fn help() {}\n"),
            (
                "tests/uses_helper.rs",
                "mod common;\nuse common::helper::help;\n",
            ),
            (
                "tests/also_uses_helper.rs",
                "fn f() { common::helper::help(); }\n",
            ),
        ],
    );
    let s = scope(
        "libviprs",
        &["tests/common/helper.rs"],
        &core,
        TESTS_BASE,
        &[],
        &[],
    );
    assert!(s.ok, "{}", s.stderr);
    assert_eq!(
        s.lines(),
        set(&[
            "core|test|miri_ignore_convention",
            "core|test|uses_helper",
            "core|test|also_uses_helper",
            "lint"
        ]),
        "stderr: {}",
        s.stderr
    );
}

/// A core module change runs the module's unit tests and doctests and every
/// test that names it, in the core and in the harness, each read from its own
/// tree.
#[test]
fn a_src_module_change_selects_its_unit_doc_and_naming_tests_in_both_repos() {
    let core = with(
        CORE_BASE,
        &[
            ("tests/names_foo.rs", "use libviprs::foo::Thing;\n"),
            ("tests/names_foo_braced.rs", "use libviprs::{bar, foo};\n"),
            ("tests/names_foo_inline.rs", "fn f() { foo::go(); }\n"),
        ],
    );
    let tests = with(
        TESTS_BASE,
        &[
            (
                "tests/harness_uses_foo.rs",
                "fn f() { libviprs::foo::make(); }\n",
            ),
            (
                "tests/harness_uses_bar.rs",
                "fn f() { libviprs::bar::make(); }\n",
            ),
        ],
    );
    let s = scope("libviprs", &["src/foo.rs"], &core, &tests, &[], &[]);
    assert!(s.ok, "{}", s.stderr);
    let lines = s.lines();
    for want in [
        "core|lib|foo::",
        "core|doc|src/foo",
        "core|test|names_foo",
        "core|test|names_foo_braced",
        "core|test|names_foo_inline",
        "tests|test|harness_uses_foo",
        "core|test|miri_ignore_convention",
    ] {
        assert!(lines.contains(want), "no `{want}` in:\n{}", s.stdout);
    }
    for unwanted in [
        "tests|test|harness_uses_bar",
        "core|test|unrelated",
        "tests|test|unrelated",
    ] {
        assert!(
            !lines.contains(unwanted),
            "`{unwanted}` selected:\n{}",
            s.stdout
        );
    }
    assert!(s.full().is_none(), "{}", s.stdout);
}

/// A test that reaches a module only through what the crate root re-exports
/// from it still names the module.
#[test]
fn a_root_reexport_counts_as_naming_the_module() {
    let core = with(
        CORE_BASE,
        &[
            ("tests/by_type.rs", "use libviprs::Thing;\n"),
            (
                "tests/by_fn.rs",
                "fn f() { let _ = libviprs::make_thing(); }\n",
            ),
            (
                "tests/multi_line_use.rs",
                "use libviprs::{\n    Raster,\n    Thing,\n};\n",
            ),
            (
                "tests/mentions_thing_but_not_the_crate.rs",
                "struct Thing;\n",
            ),
        ],
    );
    let s = scope("libviprs", &["src/foo.rs"], &core, TESTS_BASE, &[], &[]);
    assert!(s.ok, "{}", s.stderr);
    let lines = s.lines();
    for want in [
        "core|test|by_type",
        "core|test|by_fn",
        "core|test|multi_line_use",
    ] {
        assert!(lines.contains(want), "no `{want}` in:\n{}", s.stdout);
    }
    assert!(
        !lines.contains("core|test|mentions_thing_but_not_the_crate"),
        "a file that never names the crate was selected:\n{}",
        s.stdout
    );

    // A glob re-export names every item of the module, so the idents cannot be
    // listed. That is a full run, not a guess.
    let core = with(
        CORE_BASE,
        &[("src/lib.rs", "pub mod foo;\npub use foo::*;\n")],
    );
    let s = scope("libviprs", &["src/foo.rs"], &core, TESTS_BASE, &[], &[]);
    assert!(
        s.full().is_some_and(|why| why.contains("src/foo.rs")),
        "a glob re-export did not fall back to the full suite:\n{}",
        s.stdout
    );
}

/// What can change what every test sees runs everything, and says which path
/// did it.
#[test]
fn shared_infrastructure_runs_the_full_suite() {
    for repo in ["libviprs", "libviprs-tests"] {
        for path in [
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            "fuzz/Cargo.toml",
            "Dockerfile",
            "tools/Dockerfile.hooks",
            ".dockerignore",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "tests/common/mod.rs",
            "src/lib.rs",
            "COUNTERPART_REV",
            "CLI_COUNTERPART_REV",
            "tools/run-tests.sh",
            "tools/gate-scope.sh",
            "tools/scoped-suite.sh",
        ] {
            // One mapped path alongside, so the infrastructure path has to win.
            let s = scope(
                repo,
                &["tests/unrelated.rs", path],
                CORE_BASE,
                TESTS_BASE,
                &[],
                &[],
            );
            let why = s.full().unwrap_or_else(|| {
                panic!(
                    "{repo}: a change to {path} was scoped:\n{}{}",
                    s.stdout, s.stderr
                )
            });
            assert!(
                why.contains(path),
                "{repo}: the reason does not name {path}: {why}"
            );
            assert_eq!(
                s.stdout.lines().count(),
                1,
                "{repo}: {path}: FULL came with a plan:\n{}",
                s.stdout
            );
        }
    }
}

/// A path no rule maps is not "nothing to run". It is the full suite.
#[test]
fn a_path_no_rule_maps_runs_the_full_suite_not_nothing() {
    for path in [
        "tools/mystery.txt",
        "src/bin/tool.rs",
        // A test file that was deleted maps to no target that exists.
        "tests/gone.rs",
        // A fixture nothing names.
        "tests/fixtures/orphan.png",
    ] {
        let s = scope("libviprs", &[path], CORE_BASE, TESTS_BASE, &[], &[]);
        let why = s
            .full()
            .unwrap_or_else(|| panic!("{path} was scoped:\n{}{}", s.stdout, s.stderr));
        assert!(why.contains(path), "the reason does not name {path}: {why}");
    }

    // One unmapped path among mapped ones still decides it.
    let core = with(CORE_BASE, &[("tests/x.rs", "// x\n")]);
    let s = scope(
        "libviprs",
        &["tests/x.rs", "tools/mystery.txt"],
        &core,
        TESTS_BASE,
        &[],
        &[],
    );
    assert!(
        s.full().is_some(),
        "a mapped path hid an unmapped one:\n{}",
        s.stdout
    );
}

/// A selection too big to be "the PR's own tests" is the full suite, and the
/// cap is a knob.
#[test]
fn a_selection_over_the_cap_runs_the_full_suite() {
    let core = with(
        CORE_BASE,
        &[
            ("tests/n1.rs", "use libviprs::foo::A;\n"),
            ("tests/n2.rs", "use libviprs::foo::B;\n"),
            ("tests/n3.rs", "use libviprs::foo::C;\n"),
            ("tests/n4.rs", "use libviprs::foo::D;\n"),
        ],
    );
    let small = scope(
        "libviprs",
        &["src/foo.rs"],
        &core,
        TESTS_BASE,
        &[],
        &[("LIBVIPRS_GATE_SCOPE_CAP", "5")],
    );
    let why = small
        .full()
        .unwrap_or_else(|| panic!("a selection over the cap was scoped:\n{}", small.stdout));
    assert!(
        why.contains("cap"),
        "the reason does not mention the cap: {why}"
    );

    let roomy = scope(
        "libviprs",
        &["src/foo.rs"],
        &core,
        TESTS_BASE,
        &[],
        &[("LIBVIPRS_GATE_SCOPE_CAP", "50")],
    );
    assert!(roomy.full().is_none(), "{}", roomy.stdout);
    let default = scope("libviprs", &["src/foo.rs"], &core, TESTS_BASE, &[], &[]);
    assert!(
        default.full().is_none(),
        "the default cap is too small for 7 targets:\n{}",
        default.stdout
    );
}

/// Every scoped run carries the guards that are cheap and cannot be scoped
/// away, and the lint half.
#[test]
fn every_scoped_run_carries_the_always_on_guards_and_the_lint_half() {
    let core = with(CORE_BASE, &[("tests/x.rs", "// x\n")]);
    let s = scope("libviprs", &["tests/x.rs"], &core, TESTS_BASE, &[], &[]);
    let lines = s.lines();
    for want in ["core|test|miri_ignore_convention", "lint"] {
        assert!(
            lines.contains(want),
            "no `{want}` for a core push:\n{}",
            s.stdout
        );
    }

    // Core Rust under src/ or tests/ is what the Miri convention reads.
    let core = with(
        CORE_BASE,
        &[("tests/names_foo.rs", "use libviprs::foo::Thing;\n")],
    );
    let s = scope("libviprs", &["src/foo.rs"], &core, TESTS_BASE, &[], &[]);
    assert!(
        s.lines().contains("core|test|miri_ignore_convention"),
        "{}",
        s.stdout
    );

    // The harness mirrors the core's CI list, so it is checked on every push.
    let tests = with(TESTS_BASE, &[("tests/z.rs", "// z\n")]);
    let s = scope(
        "libviprs-tests",
        &["tests/z.rs"],
        CORE_BASE,
        &tests,
        &[],
        &[],
    );
    assert_eq!(
        s.lines(),
        set(&["tests|test|install_hooks_mirror_ci", "tests|test|z", "lint"]),
        "stderr: {}",
        s.stderr
    );

    // The hook and the tooling it is built from run the four hook guards.
    let s = scope(
        "libviprs-tests",
        &["tools/hooks/pre-push"],
        CORE_BASE,
        &with(
            TESTS_BASE,
            &[
                ("tests/hooks_run_on_mars.rs", ""),
                ("tests/prepush_gate_cost_controls.rs", ""),
                ("tests/prepush_gate_tests_the_pushed_tree.rs", ""),
            ],
        ),
        &[],
        &[],
    );
    let lines = s.lines();
    for guard in [
        "hooks_run_on_mars",
        "prepush_gate_cost_controls",
        "prepush_gate_tests_the_pushed_tree",
        "install_hooks_mirror_ci",
    ] {
        assert!(
            lines.contains(&format!("tests|test|{guard}")),
            "no {guard} for a hook change:\n{}",
            s.stdout
        );
    }
}

/// A `Tests:` trailer on the tip can add targets and never take one away.
#[test]
fn a_tests_trailer_adds_targets_and_never_removes_any() {
    let core = with(
        CORE_BASE,
        &[("tests/x.rs", "// x\n"), ("tests/y.rs", "// y\n")],
    );
    let tests = with(TESTS_BASE, &[("tests/bar.rs", "// bar\n")]);
    let plain = scope("libviprs", &["tests/x.rs"], &core, &tests, &[], &[]);
    let with_trailer = scope(
        "libviprs",
        &["tests/x.rs"],
        &core,
        &tests,
        &[
            "Change x\n\nWhy it needs more\n\nTests: core:y, tests:bar, core:lib:foo::, core:doc:src/foo",
        ],
        &[],
    );
    assert!(with_trailer.ok, "{}", with_trailer.stderr);
    let lines = with_trailer.lines();
    for want in [
        "core|test|y",
        "tests|test|bar",
        "core|lib|foo::",
        "core|doc|src/foo",
    ] {
        assert!(
            lines.contains(want),
            "the trailer did not add `{want}`:\n{}",
            with_trailer.stdout
        );
    }
    assert!(
        plain.lines().is_subset(&lines),
        "the trailer took something away.\nwithout:\n{}\nwith:\n{}",
        plain.stdout,
        with_trailer.stdout
    );

    // A line that merely starts with the word, away from the trailers, is prose.
    let prose = scope(
        "libviprs",
        &["tests/x.rs"],
        &core,
        &tests,
        &["Change x\n\nTests: core:y is what I would add, but this is prose.\n\nCloses #1"],
        &[],
    );
    assert!(
        !prose.lines().contains("core|test|y"),
        "prose was read as a trailer:\n{}",
        prose.stdout
    );
}

/// A name in `Tests:` that is not in the tree being gated refuses the push, and
/// says which name.
#[test]
fn a_tests_trailer_naming_a_missing_target_refuses_the_push() {
    let core = with(CORE_BASE, &[("tests/x.rs", "// x\n")]);
    for bad in [
        "core:nope",
        "tests:also_nope",
        "core:lib:nomodule::",
        "core:doc:src/nomodule",
        "elsewhere:x",
        "justaword",
    ] {
        let s = scope(
            "libviprs",
            &["tests/x.rs"],
            &core,
            TESTS_BASE,
            &[&format!("Change\n\nTests: core:x, {bad}")],
            &[],
        );
        assert!(!s.ok, "`{bad}` was accepted:\n{}{}", s.stdout, s.stderr);
        assert!(
            s.stderr.contains(bad),
            "the refusal does not name `{bad}`:\n{}",
            s.stderr
        );
        assert_eq!(
            s.stdout, "",
            "a refused push still printed a plan:\n{}",
            s.stdout
        );
    }
}

/// A target whose Cargo.toml entry asks for features gets them, from the tree
/// the target is in.
#[test]
fn a_feature_gated_target_gets_its_required_features() {
    let core_toml = "[package]\nname = \"libviprs\"\n\n[[test]]\nname = \"gated\"\nrequired-features = [\"pvec-raster\", \"svg\"]\n\n[[test]]\nname = \"plain\"\n";
    let tests_toml = "[package]\nname = \"libviprs-tests\"\n\n[[test]]\nname = \"t_gated\"\nrequired-features = [\"pvec-raster\"]\n";
    let core = with(
        CORE_BASE,
        &[
            ("Cargo.toml", core_toml),
            ("tests/gated.rs", "// g\n"),
            ("tests/plain.rs", "// p\n"),
        ],
    );
    let tests = with(
        TESTS_BASE,
        &[("Cargo.toml", tests_toml), ("tests/t_gated.rs", "// g\n")],
    );
    let s = scope(
        "libviprs",
        &["tests/gated.rs", "tests/plain.rs"],
        &core,
        &tests,
        &["Change\n\nTests: tests:t_gated"],
        &[],
    );
    assert!(s.ok, "{}", s.stderr);
    let lines = s.lines();
    assert!(
        lines.contains("features|core|pvec-raster,svg"),
        "no core features:\n{}",
        s.stdout
    );
    assert!(
        lines.contains("features|tests|pvec-raster"),
        "no harness features:\n{}",
        s.stdout
    );

    let s = scope("libviprs", &["tests/plain.rs"], &core, &tests, &[], &[]);
    assert!(
        !s.stdout.lines().any(|l| l.starts_with("features|")),
        "features for a target that asks for none:\n{}",
        s.stdout
    );
}

/// A data file a test reads selects the test that names it.
#[test]
fn a_fixture_read_by_one_test_selects_that_test() {
    let core = with(
        CORE_BASE,
        &[
            ("tests/fixtures/a.png", "png"),
            ("tests/fixtures/sub/b.png", "png"),
            (
                "tests/reads_a.rs",
                "fn f() { let _ = std::fs::read(\"tests/fixtures/a.png\"); }\n",
            ),
            (
                "tests/reads_dir.rs",
                "fn f() { let _ = std::fs::read_dir(\"tests/fixtures/sub\"); }\n",
            ),
            ("tests/reads_nothing.rs", "fn f() {}\n"),
            ("docs/table.md", "| a |\n"),
            (
                "tests/reads_doc.rs",
                "const D: &str = include_str!(\"../docs/table.md\");\n",
            ),
        ],
    );
    let s = scope(
        "libviprs",
        &["tests/fixtures/a.png"],
        &core,
        TESTS_BASE,
        &[],
        &[],
    );
    assert_eq!(
        s.lines(),
        set(&["core|test|reads_a", "lint"]),
        "stderr: {}",
        s.stderr
    );

    let s = scope(
        "libviprs",
        &["tests/fixtures/sub/b.png"],
        &core,
        TESTS_BASE,
        &[],
        &[],
    );
    assert_eq!(
        s.lines(),
        set(&["core|test|reads_dir", "lint"]),
        "stderr: {}",
        s.stderr
    );

    let s = scope("libviprs", &["docs/table.md"], &core, TESTS_BASE, &[], &[]);
    assert_eq!(
        s.lines(),
        set(&["core|test|reads_doc", "lint"]),
        "stderr: {}",
        s.stderr
    );
}

// ===========================================================================
// The hook and mars-run.sh: where the trees come from, and what is never skipped
// ===========================================================================

/// A stand-in `ssh`. Same contract as the one in `hooks_run_on_mars`: it
/// answers the preflight probes, keeps the tar and the command of every run on
/// MARS, and records the gate command.
const FAKE_SSH: &str = r#"#!/bin/sh
for last; do :; done
echo "$last" >> "$FAKE_DIR/ssh.log"
case "$last" in
  *"docker info"*) exit 0 ;;
  *"docker image inspect"*) exit 0 ;;
  *"docker ps"*) exit 0 ;;
  *"docker run --rm -i --name viprs-hook-"*)
    n=$(ls "$FAKE_DIR" | grep -c '^shipped-')
    cat > "$FAKE_DIR/shipped-$n.tar"
    echo "$last" > "$FAKE_DIR/run-$n.cmd"
    exit 0 ;;
  *"tar -xi -f - -C"*)
    n=$(ls "$FAKE_DIR" | grep -c '^shipped-')
    cat > "$FAKE_DIR/shipped-$n.tar"
    exit 0 ;;
  *"run-tests.sh"*)
    echo "$last" > "$FAKE_DIR/gate.cmd"
    exit 0 ;;
  *) exit 0 ;;
esac
"#;

struct FakeMars {
    ws: Workspace,
    dir: PathBuf,
}

impl FakeMars {
    fn new() -> FakeMars {
        let ws = Workspace::new();
        let dir = ws.root.join("fake-mars");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("create the stand-in bin dir");
        write_exec(&bin.join("ssh"), FAKE_SSH);
        FakeMars { ws, dir }
    }

    /// A push of `repo` from its main checkout through the installed shim and
    /// the real `mars-run.sh`.
    fn push(&self, repo: &str, before: &str, after: &str, env: &[(&str, &str)]) -> (bool, String) {
        let root = self.ws.repo(repo);
        let mut cmd = Command::new("bash");
        cmd.arg(root.join(".git/hooks/pre-push"))
            .arg("origin")
            .arg("git@example.invalid:libviprs/x.git")
            .current_dir(&root)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.dir.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("FAKE_DIR", &self.dir)
            .env_remove("LIBVIPRS_ON_MARS")
            .env_remove("LIBVIPRS_HOOKS_SKIP_MARS")
            .env_remove("LIBVIPRS_HOOKS_MARS_RUN")
            .env_remove("LIBVIPRS_PREPUSH_ALL")
            .env_remove("LIBVIPRS_GATE_SCOPE_CAP")
            .env_remove("GIT_SSH_COMMAND")
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("run the installed pre-push hook");
        writeln!(
            child.stdin.as_mut().expect("hook stdin"),
            "refs/heads/main {after} refs/heads/main {before}"
        )
        .expect("feed the ref update");
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

    /// The plan the gate command carried, decoded, or `None` when it carried
    /// none, which is the full suite.
    fn scope_of_the_gate(&self) -> Option<String> {
        let gate = self.read("gate.cmd");
        assert!(
            gate.contains("tools/run-tests.sh"),
            "the suite never ran:\n{gate}"
        );
        let (_, after) = gate.split_once("--scope")?;
        let b64: String = after
            .chars()
            .skip_while(|c| !c.is_ascii_alphanumeric())
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
            .collect();
        let mut child = Command::new("base64")
            .arg("-d")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn base64");
        child
            .stdin
            .take()
            .expect("base64 stdin")
            .write_all(b64.as_bytes())
            .expect("feed base64");
        let out = child.wait_with_output().expect("run base64");
        Some(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// The trees the plan is chosen from are the trees that run: a core push reads
/// the harness at origin/main, not the sibling checkout, and a harness push
/// reads the core its COUNTERPART_REV pins, not the sibling checkout.
#[test]
fn a_core_push_greps_the_harness_at_origin_main_and_a_harness_push_greps_the_pinned_core() {
    let m = FakeMars::new();
    let tests = m.ws.repo("libviprs-tests");

    // The harness at origin/main has a test naming the module. The sibling
    // checkout is ahead of it with another one, which must not be picked up.
    m.ws.commit(
        "libviprs-tests",
        "harness at origin/main",
        &[(
            "tests/at_origin_main.rs",
            "fn f() { libviprs::foo::go(); }\n",
        )],
    );
    git(&tests, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    m.ws.commit(
        "libviprs-tests",
        "somebody's working branch",
        &[(
            "tests/only_in_the_sibling.rs",
            "fn f() { libviprs::foo::go(); }\n",
        )],
    );
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[("src/foo.rs", "// a\n"), ("src/lib.rs", "pub mod foo;\n")],
    );
    let after =
        m.ws.commit("libviprs", "change", &[("src/foo.rs", "// b\n")]);
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    let plan = m
        .scope_of_the_gate()
        .unwrap_or_else(|| panic!("a module change ran the full suite:\n{out}"));
    assert!(
        plan.contains("tests|test|at_origin_main"),
        "the harness at origin/main was not read:\n{plan}"
    );
    assert!(
        !plan.contains("only_in_the_sibling"),
        "the sibling checkout was read instead of origin/main:\n{plan}"
    );

    // A harness push: the pinned core has a test that names the data file, the
    // core checkout's HEAD has another that must not be picked up.
    let pinned = m.ws.commit(
        "libviprs",
        "the pinned core",
        &[(
            "tests/pinned_reads_notes.rs",
            "const N: &str = \"docs/notes.txt\";\n",
        )],
    );
    m.ws.commit(
        "libviprs",
        "the core's HEAD",
        &[(
            "tests/head_reads_notes.rs",
            "const N: &str = \"docs/notes.txt\";\n",
        )],
    );
    let before = m.ws.commit(
        "libviprs-tests",
        "harness base",
        &[
            ("COUNTERPART_REV", &format!("# pin\n{pinned}\n")),
            ("docs/notes.txt", "a\n"),
            (
                "tests/reads_notes.rs",
                "const N: &str = \"docs/notes.txt\";\n",
            ),
        ],
    );
    let after =
        m.ws.commit("libviprs-tests", "change", &[("docs/notes.txt", "b\n")]);
    let _ = std::fs::remove_file(m.dir.join("gate.cmd"));
    let (ok, out) = m.push("libviprs-tests", &before, &after, &[]);
    assert!(ok, "{out}");
    let plan = m
        .scope_of_the_gate()
        .unwrap_or_else(|| panic!("a notes change ran the full suite:\n{out}"));
    assert!(plan.contains("tests|test|reads_notes"), "{plan}");
    assert!(
        plan.contains("core|test|pinned_reads_notes"),
        "the pinned core was not read:\n{plan}"
    );
    assert!(
        !plan.contains("head_reads_notes"),
        "the core checkout was read instead of the core COUNTERPART_REV pins:\n{plan}"
    );
}

/// The existing override keeps its meaning, and it only ever means more.
#[test]
fn prepush_all_still_runs_the_full_suite() {
    let m = FakeMars::new();
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[
            ("src/foo.rs", "// a\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            ("tests/names_foo.rs", "use libviprs::foo::go;\n"),
        ],
    );
    let after =
        m.ws.commit("libviprs", "change", &[("src/foo.rs", "// b\n")]);

    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    assert!(
        m.scope_of_the_gate().is_some(),
        "the module change was not scoped to begin with:\n{out}"
    );

    let _ = std::fs::remove_file(m.dir.join("gate.cmd"));
    let (ok, out) = m.push(
        "libviprs",
        &before,
        &after,
        &[("LIBVIPRS_PREPUSH_ALL", "1")],
    );
    assert!(ok, "{out}");
    assert!(
        m.scope_of_the_gate().is_none(),
        "LIBVIPRS_PREPUSH_ALL=1 still scoped the run:\n{out}"
    );

    // The selector itself answers the same way if it is asked.
    let s = scope(
        "libviprs",
        &["src/foo.rs"],
        CORE_BASE,
        TESTS_BASE,
        &[],
        &[("LIBVIPRS_PREPUSH_ALL", "1")],
    );
    assert!(s.full().is_some(), "{}", s.stdout);
}

/// A push whose base cannot be found is not one the selector can judge, so it
/// is the full suite.
#[test]
fn an_unknown_merge_base_runs_the_full_suite() {
    let m = FakeMars::new();
    let after = m.ws.commit(
        "libviprs",
        "a new branch with nothing to compare against",
        &[
            ("src/foo.rs", "// b\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            ("tests/names_foo.rs", "use libviprs::foo::go;\n"),
        ],
    );
    let zeros = "0".repeat(40);
    let (ok, out) = m.push("libviprs", &zeros, &after, &[]);
    assert!(ok, "{out}");
    assert!(
        m.scope_of_the_gate().is_none(),
        "an unknown merge base was scoped:\n{out}"
    );
}

/// The lint half runs after a scoped suite, on each tip, through the same
/// pre-commit hook the commit ran, so the mirror of the core's CI list stays
/// one list. The guards ride in the plan.
#[test]
fn the_lint_half_runs_after_a_scoped_suite_through_the_pre_commit_hook() {
    let m = FakeMars::new();
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[
            ("src/foo.rs", "// a\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            ("tests/names_foo.rs", "use libviprs::foo::go;\n"),
            ("tests/miri_ignore_convention.rs", "// guard\n"),
        ],
    );
    let after =
        m.ws.commit("libviprs", "change", &[("src/foo.rs", "// b\n")]);
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    let plan = m
        .scope_of_the_gate()
        .unwrap_or_else(|| panic!("not scoped:\n{out}"));
    assert!(plan.contains("core|test|miri_ignore_convention"), "{plan}");
    assert!(
        m.read("ssh.log").contains("viprs-hook-libviprs"),
        "the lint half did not run on MARS:\n{out}"
    );

    // A full run has no lint line to honour, and the suite's own verdict is
    // all there is.
    let before =
        m.ws.commit("libviprs", "base 2", &[("Cargo.toml", "# a\n")]);
    let after =
        m.ws.commit("libviprs", "change 2", &[("Cargo.toml", "# b\n")]);
    let _ = std::fs::remove_file(m.dir.join("ssh.log"));
    let _ = std::fs::remove_file(m.dir.join("gate.cmd"));
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    assert!(
        m.scope_of_the_gate().is_none(),
        "a Cargo.toml change was scoped:\n{out}"
    );
    assert!(
        !m.read("ssh.log").contains("viprs-hook-libviprs"),
        "a full run linted as well:\n{out}"
    );
}

/// A `Tests:` name that is not in the tree refuses the push through the hook
/// too, and nothing runs on MARS for it.
#[test]
fn a_push_naming_a_missing_target_in_tests_is_refused() {
    let m = FakeMars::new();
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[("src/foo.rs", "// a\n"), ("src/lib.rs", "pub mod foo;\n")],
    );
    let after = m.ws.commit(
        "libviprs",
        "change\n\nTests: core:nope",
        &[("src/foo.rs", "// b\n")],
    );
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(!ok, "the push went through:\n{out}");
    assert!(
        out.contains("core:nope"),
        "the refusal does not name the target:\n{out}"
    );
    assert_eq!(
        m.read("gate.cmd"),
        "",
        "the suite ran for a refused push:\n{out}"
    );
}

/// A source path that no test names is not "the module's own filters": the
/// filters always exist, so they cannot be what decides a module has a test.
/// A deleted source file maps to nothing that exists either.
#[test]
fn a_source_path_no_test_names_runs_the_full_suite() {
    let core = with(
        CORE_BASE,
        &[
            ("src/quiet.rs", "// nobody names me\n"),
            ("src/m/other.rs", "// kept\n"),
        ],
    );
    for path in ["src/quiet.rs", "src/m/x.rs"] {
        let s = scope("libviprs", &[path], &core, TESTS_BASE, &[], &[]);
        let why = s
            .full()
            .unwrap_or_else(|| panic!("{path} was scoped:\n{}{}", s.stdout, s.stderr));
        assert!(why.contains(path), "the reason does not name {path}: {why}");
    }
}

/// A `Tests:` trailer is data. A glob in it must be refused, never expanded
/// against the directory the hook happens to run in.
#[test]
fn a_tests_trailer_glob_is_refused() {
    let core = with(CORE_BASE, &[("tests/x.rs", "// x\n")]);
    for bad in ["core:*", "*", "tests:x?"] {
        let s = scope(
            "libviprs",
            &["tests/x.rs"],
            &core,
            TESTS_BASE,
            &[&format!("Change\n\nTests: {bad}")],
            &[],
        );
        assert!(!s.ok, "`{bad}` was accepted:\n{}{}", s.stdout, s.stderr);
        assert_eq!(s.stdout, "", "a refused push printed a plan:\n{}", s.stdout);
    }
}

/// A scope list left in the environment by something else is not this push's.
/// With no base to compare against there is no list, and the full suite runs.
#[test]
fn an_inherited_scope_list_does_not_narrow_an_unknown_base_push() {
    let m = FakeMars::new();
    let after = m.ws.commit(
        "libviprs",
        "a new branch with nothing to compare against",
        &[
            ("src/foo.rs", "// b\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            // The stale list names this file, so it has to exist for the list
            // to narrow anything: only the unset stands between the two.
            ("tests/unrelated.rs", "fn main() {}\n"),
        ],
    );
    let stale = m.ws.root.join("stale-changed");
    std::fs::write(&stale, "tests/unrelated.rs\n").expect("write a stale list");
    let (ok, out) = m.push(
        "libviprs",
        &"0".repeat(40),
        &after,
        &[
            ("LIBVIPRS_GATE_CHANGED_FILE", stale.to_str().expect("utf-8")),
            ("LIBVIPRS_GATE_REPO", "libviprs"),
            ("LIBVIPRS_GATE_TIPS", &after),
        ],
    );
    assert!(ok, "{out}");
    assert!(
        m.scope_of_the_gate().is_none(),
        "an inherited list narrowed the push:\n{out}"
    );
}

/// A rename is a delete and an add. Listing only the new name would scope a
/// push to a file and never see the one that went away.
#[test]
fn a_renamed_test_file_counts_its_old_name_too() {
    let m = FakeMars::new();
    let body = "#[test] fn t() { assert!(true); }\n// enough text to be a rename\n";
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[
            ("src/lib.rs", "pub mod foo;\n"),
            ("src/foo.rs", "// a\n"),
            ("tests/old.rs", body),
        ],
    );
    git(
        &m.ws.repo("libviprs"),
        &["mv", "tests/old.rs", "tests/new.rs"],
    );
    let after = m.ws.commit("libviprs", "rename", &[]);
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    assert!(
        m.scope_of_the_gate().is_none(),
        "the old name of a renamed test never reached the selector:\n{out}"
    );
}

/// If the selector itself falls over, the answer is the whole suite.
#[test]
fn a_selector_that_crashes_runs_the_full_suite() {
    let m = FakeMars::new();
    write_exec(
        &m.ws.repo("libviprs-tests").join("tools/gate-scope.sh"),
        "#!/bin/sh\necho boom >&2\nexit 1\n",
    );
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[("src/foo.rs", "// a\n"), ("src/lib.rs", "pub mod foo;\n")],
    );
    let after =
        m.ws.commit("libviprs", "change", &[("src/foo.rs", "// b\n")]);
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "a crashed selector refused the push:\n{out}");
    assert!(
        m.scope_of_the_gate().is_none(),
        "a crashed selector still scoped the run:\n{out}"
    );
    assert!(
        out.contains("whole suite"),
        "the fallback was silent:\n{out}"
    );
}

// ===========================================================================
// The PR's own changes (libviprs-tests#375)
// ===========================================================================

/// The tree under test: a base commit with a module and a test naming it,
/// and `origin/main` moved on by a manifest change that is not on the branch.
/// Returns the stand-in and the branch tip the remote already has.
fn branch_with_main_ahead(m: &FakeMars) -> String {
    let repo = m.ws.repo("libviprs");
    m.ws.commit(
        "libviprs",
        "base",
        &[
            ("src/foo.rs", "// a\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            ("tests/names_foo.rs", "use libviprs::foo::go;\n"),
        ],
    );
    git(&repo, &["checkout", "-q", "-b", "side"]);
    m.ws.commit(
        "libviprs",
        "main moves on",
        &[
            ("Cargo.toml", "[package]\nname = \"libviprs\"\n"),
            (".github/workflows/ci.yml", "# new\n"),
        ],
    );
    git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(&repo, &["checkout", "-q", "main"]);
    m.ws.commit("libviprs", "work on the PR", &[("src/foo.rs", "// b\n")])
}

fn merge_main(m: &FakeMars) -> String {
    let repo = m.ws.repo("libviprs");
    git(
        &repo,
        &[
            "-c",
            "user.name=scope guard",
            "-c",
            "user.email=guard@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "merge",
            "-q",
            "--no-edit",
            "origin/main",
        ],
    );
    git(&repo, &["rev-parse", "HEAD"]).trim().to_string()
}

/// Main merged into the branch is not the PR's change. A manifest change that
/// is on main only must not make the push the full suite.
#[test]
fn a_main_merge_does_not_make_mains_changes_the_prs() {
    let m = FakeMars::new();
    let remote_tip = branch_with_main_ahead(&m);
    merge_main(&m);
    let tip =
        m.ws.commit("libviprs", "more work", &[("src/foo.rs", "// c\n")]);

    let (ok, out) = m.push("libviprs", &remote_tip, &tip, &[]);
    assert!(ok, "{out}");
    let plan = m
        .scope_of_the_gate()
        .unwrap_or_else(|| panic!("main's changes made the PR a full run:\n{out}"));
    assert!(plan.contains("core|lib|foo::"), "{plan}");
    assert!(plan.contains("core|test|names_foo"), "{plan}");
}

/// A merge-forward and nothing else adds nothing of the PR's own, so there is
/// nothing new to run.
#[test]
fn a_pure_main_merge_forward_contributes_nothing() {
    let m = FakeMars::new();
    let remote_tip = branch_with_main_ahead(&m);
    let tip = merge_main(&m);

    let (ok, out) = m.push("libviprs", &remote_tip, &tip, &[]);
    assert!(ok, "{out}");
    assert_eq!(
        m.read("gate.cmd"),
        "",
        "a merge-forward ran the suite:\n{out}"
    );
    assert!(
        out.contains("nothing in this push reaches the test suite"),
        "{out}"
    );
}

/// An issue form is read by the tests that name it and checked by the CI
/// parity guards, and it is scoped, not the full suite. A test that walks all of
/// `.github` for what it contains is a reader of every file there, so it comes
/// along for a workflow or a form (libviprs's pvec_1079_provider_default does,
/// for libredwg and dwg2svg names).
#[test]
fn an_issue_form_change_maps_to_its_readers_and_the_cheap_guards() {
    let core = with(
        CORE_BASE,
        &[
            (".github/ISSUE_TEMPLATE/epic.yml", "name: Epic\n"),
            (".github/workflows/native-dwg.yml", "name: dwg\n"),
            (
                "tests/epic_form.rs",
                "const F: &str = include_str!(\"../.github/ISSUE_TEMPLATE/epic.yml\");\n",
            ),
            (
                "tests/pvec_1079_provider_default.rs",
                "fn walk() { let root = std::path::Path::new(\".\").join(\".github\"); let _ = root; }\n",
            ),
            ("tests/local_gate_is_the_job_list.rs", "// guard\n"),
            ("tests/local_gate_native_dwg_1295.rs", "// guard\n"),
            ("tests/miri_invocation_parity.rs", "// guard\n"),
        ],
    );
    for path in [
        ".github/ISSUE_TEMPLATE/epic.yml",
        ".github/workflows/native-dwg.yml",
    ] {
        let s = scope("libviprs", &[path], &core, TESTS_BASE, &[], &[]);
        assert!(s.full().is_none(), "{path}: {}", s.stdout);
        let lines = s.lines();
        for want in [
            "core|test|pvec_1079_provider_default",
            "core|test|local_gate_is_the_job_list",
            "core|test|local_gate_native_dwg_1295",
            "core|test|miri_invocation_parity",
            "lint",
        ] {
            assert!(
                lines.contains(want),
                "{path}: no `{want}` in:\n{}",
                s.stdout
            );
        }
    }
    let s = scope(
        "libviprs",
        &[".github/ISSUE_TEMPLATE/epic.yml"],
        &core,
        TESTS_BASE,
        &[],
        &[],
    );
    assert!(s.lines().contains("core|test|epic_form"), "{}", s.stdout);
}

/// A tracked dotfile at the root that no test names (`.commitmsg-631` is one
/// that main has carried since #660) is not code and not a reason for the
/// whole suite. It maps to the cheap guards, present or deleted.
#[test]
fn a_root_dotfile_no_test_names_maps_to_the_cheap_guards_only() {
    let core = with(
        CORE_BASE,
        &[
            (".commitmsg-631", "an old message\n"),
            ("tests/local_gate_is_the_job_list.rs", "// guard\n"),
            ("tests/local_gate_native_dwg_1295.rs", "// guard\n"),
            ("tests/miri_invocation_parity.rs", "// guard\n"),
        ],
    );
    let guards = set(&[
        "core|test|local_gate_is_the_job_list",
        "core|test|local_gate_native_dwg_1295",
        "core|test|miri_invocation_parity",
        "lint",
    ]);
    let s = scope("libviprs", &[".commitmsg-631"], &core, TESTS_BASE, &[], &[]);
    assert_eq!(s.lines(), guards, "present: {}{}", s.stdout, s.stderr);
    let s = scope("libviprs", &[".commitmsg-632"], &core, TESTS_BASE, &[], &[]);
    assert_eq!(s.lines(), guards, "deleted: {}{}", s.stdout, s.stderr);

    // A dotfile a test does name still runs that test.
    let core = with(
        &core,
        &[(
            "tests/reads_msg.rs",
            "fn f() { let _ = std::fs::read(\".commitmsg-631\"); }\n",
        )],
    );
    let s = scope("libviprs", &[".commitmsg-631"], &core, TESTS_BASE, &[], &[]);
    assert!(s.lines().contains("core|test|reads_msg"), "{}", s.stdout);
}

/// A branch that was never pushed is measured against the default branch as
/// the remote has it now, not as this clone last saw it. With a stale
/// origin/main the commits main gained since count as the branch's own.
#[test]
fn a_new_branch_is_measured_against_the_fetched_default_branch() {
    let m = FakeMars::new();
    let repo = m.ws.repo("libviprs");
    let base = m.ws.commit(
        "libviprs",
        "base",
        &[
            ("src/foo.rs", "// a\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            ("tests/names_foo.rs", "use libviprs::foo::go;\n"),
        ],
    );
    let bare = m.ws.root.join("origin.git");
    git(
        &m.ws.root,
        &[
            "clone",
            "-q",
            "--bare",
            repo.to_str().expect("utf-8"),
            bare.to_str().expect("utf-8"),
        ],
    );
    git(
        &repo,
        &["remote", "add", "origin", bare.to_str().expect("utf-8")],
    );
    git(&repo, &["fetch", "-q", "origin"]);

    // Main moves on at the remote only: this clone's origin/main stays stale.
    git(&repo, &["checkout", "-q", "-b", "side"]);
    m.ws.commit("libviprs", "main moves on", &[("Cargo.toml", "# main\n")]);
    // Fetched into the bare repository, so no hook runs and this clone's
    // remote-tracking ref does not move.
    git(
        &bare,
        &["fetch", "-q", repo.to_str().expect("utf-8"), "+side:main"],
    );

    git(&repo, &["checkout", "-q", "-b", "feat", &base]);
    m.ws.commit("libviprs", "the PR's work", &[("src/foo.rs", "// b\n")]);
    git(
        &repo,
        &[
            "-c",
            "user.name=scope guard",
            "-c",
            "user.email=guard@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "merge",
            "-q",
            "--no-edit",
            "side",
        ],
    );
    let tip = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();

    let (ok, out) = m.push("libviprs", &"0".repeat(40), &tip, &[]);
    assert!(ok, "{out}");
    assert!(
        m.scope_of_the_gate().is_some(),
        "main's newer commits counted as the new branch's own:\n{out}"
    );
}

/// Only the old `.commitmsg-*` scratch files get the cheap guards. Other root
/// dotfiles change what is tested or how (`.gitattributes` export-ignore changes
/// the tree `git archive` ships to MARS), so they stay the full suite.
#[test]
fn other_root_dotfiles_still_run_the_full_suite() {
    for path in [
        ".gitattributes",
        ".gitmodules",
        ".lfsconfig",
        ".clippy.toml",
        ".env",
        ".env.local",
        ".fill-631.sh",
    ] {
        for repo in ["libviprs", "libviprs-tests"] {
            let s = scope(repo, &[path], CORE_BASE, TESTS_BASE, &[], &[]);
            assert!(
                s.full().is_some_and(|why| why.contains(path)),
                "{repo}: {path} was scoped:\n{}{}",
                s.stdout,
                s.stderr
            );
        }
    }
}

/// A merge commit's own resolution is the PR's change even though the commit
/// has main's changes beside it.
#[test]
fn a_change_made_in_the_merge_commit_itself_is_the_prs() {
    let m = FakeMars::new();
    let repo = m.ws.repo("libviprs");
    let remote_tip = branch_with_main_ahead(&m);
    git(
        &repo,
        &[
            "-c",
            "user.name=scope guard",
            "-c",
            "user.email=guard@example.invalid",
            "merge",
            "-q",
            "--no-commit",
            "--no-ff",
            "origin/main",
        ],
    );
    std::fs::write(repo.join("tests/evil.rs"), "// resolved by hand\n").expect("write");
    git(&repo, &["add", "tests/evil.rs"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=scope guard",
            "-c",
            "user.email=guard@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            // The stand-in's installed pre-commit hook would go to the real MARS.
            "--no-verify",
            "--no-edit",
        ],
    );
    let tip = git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
    let (ok, out) = m.push("libviprs", &remote_tip, &tip, &[]);
    assert!(ok, "{out}");
    let plan = m
        .scope_of_the_gate()
        .unwrap_or_else(|| panic!("the merge's own change was lost or main's counted:\n{out}"));
    assert!(plan.contains("core|test|evil"), "{plan}");
}

/// With no default branch to subtract, the pushed range stands, so main's
/// manifest change is visible in it and the push is the full suite, while an
/// ordinary change is still scoped.
#[test]
fn without_a_default_branch_the_range_stands() {
    let m = FakeMars::new();
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[
            ("src/foo.rs", "// a\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            ("tests/names_foo.rs", "use libviprs::foo::go;\n"),
        ],
    );
    let ordinary = m.ws.commit("libviprs", "work", &[("src/foo.rs", "// b\n")]);
    let (ok, out) = m.push("libviprs", &before, &ordinary, &[]);
    assert!(ok, "{out}");
    assert!(m.scope_of_the_gate().is_some(), "{out}");

    let _ = std::fs::remove_file(m.dir.join("gate.cmd"));
    let manifest =
        m.ws.commit("libviprs", "manifest", &[("Cargo.toml", "# x\n")]);
    let (ok, out) = m.push("libviprs", &ordinary, &manifest, &[]);
    assert!(ok, "{out}");
    assert!(m.scope_of_the_gate().is_none(), "{out}");
}

/// A fetch that fails must not fail the push: the ref stays as stale as it
/// was and the push is measured against that.
#[test]
fn a_failing_fetch_of_the_default_branch_does_not_refuse_the_push() {
    let m = FakeMars::new();
    let repo = m.ws.repo("libviprs");
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[
            ("src/foo.rs", "// a\n"),
            ("src/lib.rs", "pub mod foo;\n"),
            ("tests/names_foo.rs", "use libviprs::foo::go;\n"),
        ],
    );
    git(
        &repo,
        &["remote", "add", "origin", "/nonexistent/origin.git"],
    );
    git(&repo, &["update-ref", "refs/remotes/origin/main", &before]);
    let after = m.ws.commit("libviprs", "work", &[("src/foo.rs", "// b\n")]);
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "a failed fetch refused the push:\n{out}");
    assert!(m.scope_of_the_gate().is_some(), "{out}");
}

/// The fetch is bounded once ssh has connected and it composes with the
/// ssh command the user has configured instead of replacing it.
#[test]
fn the_fetch_keeps_a_configured_ssh_command_and_bounds_a_silent_host() {
    let m = FakeMars::new();
    let repo = m.ws.repo("libviprs");
    let before = m.ws.commit(
        "libviprs",
        "base",
        &[("src/foo.rs", "// a\n"), ("src/lib.rs", "pub mod foo;\n")],
    );
    let recorder = m.dir.join("user-ssh");
    write_exec(
        &recorder,
        &format!(
            "#!/bin/sh\necho \"$*\" >> \"{}/user-ssh.log\"\nexit 1\n",
            m.dir.display()
        ),
    );
    git(
        &repo,
        &["remote", "add", "origin", "ssh://example.invalid/x.git"],
    );
    git(
        &repo,
        &[
            "config",
            "core.sshCommand",
            recorder.to_str().expect("utf-8"),
        ],
    );
    git(&repo, &["update-ref", "refs/remotes/origin/main", &before]);
    let after = m.ws.commit("libviprs", "work", &[("src/foo.rs", "// b\n")]);
    let (ok, out) = m.push("libviprs", &before, &after, &[]);
    assert!(ok, "{out}");
    let log = m.read("user-ssh.log");
    assert!(
        log.contains("ServerAliveInterval=5") && log.contains("ServerAliveCountMax=2"),
        "the configured ssh command never ran bounded; it logged: {log:?}"
    );
}
