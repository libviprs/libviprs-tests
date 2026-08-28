//! Guards on what the local pre-push gate costs, and on what it is allowed to
//! skip to save that cost (libviprs/libviprs#683).
//!
//! The gate ran the whole Docker suite on every push, including pushes that
//! touched no Rust at all, and it fell over under its own memory ceiling in a
//! way that read as a test failure. Between the two, `--no-verify` became the
//! normal way to push, and a hook nobody runs protects nothing. Both halves
//! are cheap to get wrong again in the direction that hurts: a skip list that
//! grows past what is provably inert, and a budget that goes back to being a
//! number nobody checked against the machine.
//!
//! The failure mode of the whole change is the gate quietly ceasing to run, so
//! these guards are built to fail when that happens rather than to describe it.
//! Two of them lift `inert_path()` out of the generated hook and *execute* it:
//! one runs every tracked path in both repos through it and pins the set that
//! comes back inert, the other installs the whole hook in a throwaway workspace
//! and drives real pushes past it. An earlier pair of text guards here could
//! not fail for the reason they claimed: a reviewer flipped `inert_path`'s
//! fallthrough to `true`, disabling the gate for every push in every repo, and
//! both stayed green.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod common;
use common::hooks::{generated_pre_push, read, repo_root};

// ---------------------------------------------------------------------------
// Running the hook's own `inert_path()`
// ---------------------------------------------------------------------------

/// The `inert_path()` shell function, lifted verbatim out of the generated
/// hook and wrapped in a driver that prints the inert ones from a list of
/// paths on stdin.
///
/// Lifting the real text is the point. A guard that re-implements the case
/// statement in Rust tests the re-implementation, and a guard that greps the
/// text tests the grep; both were tried here and both waved through six
/// poisoned entries and a flipped fallthrough.
fn inert_path_driver() -> String {
    let hook = generated_pre_push();
    let start = hook.find("inert_path() {").expect(
        "the pre-push hook must decide what it may skip in one place, a shell \
         function called inert_path()",
    );
    let end = hook[start..]
        .find("\n}\n")
        .map(|i| start + i + 2)
        .expect("unterminated inert_path()");
    let body = &hook[start..end];
    format!(
        "REPO_NAME=\"$1\"\n\
         {body}\n\
         while IFS= read -r candidate; do\n\
         \x20   [ -z \"$candidate\" ] && continue\n\
         \x20   if inert_path \"$candidate\"; then printf '%s\\n' \"$candidate\"; fi\n\
         done\n"
    )
}

/// Which of `paths` the hook would treat as inert for a push to `repo_name`.
fn inert_paths(repo_name: &str, paths: &[String]) -> BTreeSet<String> {
    let dir = tempfile::tempdir().expect("temp dir for the inert_path driver");
    let script = dir.path().join("inert_path.sh");
    std::fs::write(&script, inert_path_driver()).expect("write the inert_path driver");

    let mut child = Command::new("sh")
        .arg(&script)
        .arg(repo_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run the inert_path driver");
    {
        let stdin = child.stdin.as_mut().expect("driver stdin");
        for p in paths {
            writeln!(stdin, "{p}").expect("feed a path to the driver");
        }
    }
    let out = child
        .wait_with_output()
        .expect("collect the driver's answer");
    assert!(
        out.status.success(),
        "the inert_path driver failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// Every path a repo tracks, or, where there is no repository to ask (the
/// Docker build context copies both trees without their `.git`), every file in
/// the tree. Same shape as the working-tree fallback in `pdfium_provenance`.
fn candidate_paths(root: &Path) -> Vec<String> {
    if let Ok(out) = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
    {
        if out.status.success() {
            return String::from_utf8_lossy(&out.stdout)
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
    let mut found = Vec::new();
    walk(root, root, &mut found);
    found
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // The same three the build context drops, so the two ways of listing
        // a tree agree on what is in it.
        if matches!(name.as_str(), ".git" | "target" | "tmp") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, found);
        } else if let Ok(rel) = path.strip_prefix(root) {
            found.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// The sibling core checkout. `libviprs = { path = "../libviprs" }` in this
/// crate's manifest means it is present whenever this suite can run at all.
fn core_root() -> PathBuf {
    repo_root().join("../libviprs")
}

/// Every path this repo tracks that the pre-push gate is allowed to skip.
///
/// `.gitignore` is deliberately absent: `tests/pdfium_provenance.rs` reads it
/// for the `*.dylib` / `*.so` / `*.dll` patterns that keep a provenance-less
/// PDFium binary out of the repo (issue #56), so a `.gitignore`-only push is
/// the one push that must not skip the guard on it.
const INERT_IN_LIBVIPRS_TESTS: &[&str] = &["LICENSE"];

/// The same for the core checkout, where `.github/` and `.gitignore` are inert
/// because nothing in either repo reads the core's copies of them.
const INERT_IN_LIBVIPRS: &[&str] = &[
    ".github/workflows/ci.yml",
    ".github/workflows/merge-gate.yml",
    ".github/workflows/publish.yml",
    ".gitignore",
    "LICENSE",
    "docs/streaming-pdf-rotation.md",
];

/// The skip list may only hold paths nothing reads, and the only way to know
/// that is to run every path there is through it.
///
/// Set equality rather than a spot check, so widening the list takes two
/// deliberate edits instead of one. Add `tools/*` and this fails naming
/// `tools/run-tests.sh`, which *is* the suite; add `src/*` and it fails naming
/// the core library's source; flip the fallthrough to `true` and it fails
/// naming everything.
#[test]
fn the_skip_list_exempts_exactly_the_pinned_paths() {
    for (repo_name, root, pinned) in [
        ("libviprs-tests", repo_root(), INERT_IN_LIBVIPRS_TESTS),
        ("libviprs", core_root(), INERT_IN_LIBVIPRS),
    ] {
        let paths = candidate_paths(&root);
        assert!(
            paths.len() > 50,
            "only {} paths found under {}, so this guard would pass on an empty \
             tree rather than on a correct skip list",
            paths.len(),
            root.display()
        );

        let got = inert_paths(repo_name, &paths);
        let want: BTreeSet<String> = pinned.iter().map(|s| s.to_string()).collect();

        let newly_exempt: Vec<&String> = got.difference(&want).collect();
        let no_longer_there: Vec<&String> = want.difference(&got).collect();

        assert!(
            newly_exempt.is_empty(),
            "the pre-push gate would now skip a push that changes only these \
             {} path(s) in {repo_name}, and they are not on the pinned list:\n  \
             {}\nEvery one of them is a push that no longer runs a test. If \
             that is genuinely what you want, add them to the pinned list in \
             tests/prepush_gate_cost_controls.rs and say in the commit why \
             nothing reads them (#683).",
            newly_exempt.len(),
            newly_exempt
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
        assert!(
            no_longer_there.is_empty(),
            "these paths are pinned as inert for {repo_name} but the tree no \
             longer has them, or inert_path() no longer says so:\n  {}\nDrop \
             them from the pinned list in tests/prepush_gate_cost_controls.rs.",
            no_longer_there
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
}

/// The same function, asked directly about the paths whose exemption would
/// hurt most. Cheap, independent of what either tree happens to contain, and
/// it names the consequence rather than the pattern.
#[test]
fn the_skip_list_never_exempts_what_the_suite_reads() {
    let load_bearing: &[(&str, &str, &str)] = &[
        (
            "libviprs-tests",
            ".gitignore",
            "tests/pdfium_provenance.rs reads it for the native-binary patterns \
             that keep a provenance-less PDFium out of the repo (issue #56)",
        ),
        (
            "libviprs-tests",
            ".github/workflows/ci.yml",
            "the pinning guards read this repo's workflows directly \
             (tests/common/workflows.rs)",
        ),
        ("libviprs-tests", "tools/run-tests.sh", "it is the suite"),
        (
            "libviprs-tests",
            "tools/install-hooks.sh",
            "it generates the hook these guards read",
        ),
        (
            "libviprs-tests",
            "tests/pdfium_provenance.rs",
            "it is a test",
        ),
        ("libviprs-tests", "Cargo.lock", "it decides what gets built"),
        (
            "libviprs-tests",
            "Dockerfile",
            "it decides how it gets built",
        ),
        (
            "libviprs-tests",
            "README.md",
            "tests/feature_rename_docs_present.rs reads it",
        ),
        ("libviprs", "src/lib.rs", "it is the library under test"),
        (
            "libviprs",
            "README.md",
            "the documentation guards read the core checkout's copy too",
        ),
        ("libviprs", "CHANGELOG.md", "same"),
        ("libviprs", "Cargo.toml", "it decides what gets built"),
    ];

    for (repo_name, path, why) in load_bearing {
        let inert = inert_paths(repo_name, &[(*path).to_string()]);
        assert!(
            inert.is_empty(),
            "the pre-push gate would skip a {repo_name} push that changes only \
             {path}, but {why}. A skip there lets a real failure through as a \
             saved five minutes (#683)."
        );
    }
}

// ---------------------------------------------------------------------------
// Driving the whole hook
// ---------------------------------------------------------------------------

/// What the stub `run-tests.sh` prints when the hook decides to run the suite.
const RAN_MARKER: &str = "STUB-RAN-THE-SUITE";

/// A throwaway workspace holding the two repos the hook expects as siblings,
/// with the generated pre-push hook installed in both and a stub in place of
/// `run-tests.sh`, so a decision can be observed without Docker.
///
/// The evidence that this change works was six rows of skip/run decisions
/// produced by hand. For a change whose failure mode is the gate silently
/// ceasing to run, by hand and uncommitted is the wrong place for it.
struct Workspace {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Workspace {
    fn new() -> Workspace {
        let dir = tempfile::tempdir().expect("temp dir for a stand-in workspace");
        let root = dir.path().canonicalize().expect("canonical temp path");
        let hook_body = generated_pre_push();

        for repo in ["libviprs", "libviprs-tests"] {
            let repo_root = root.join(repo);
            std::fs::create_dir_all(repo_root.join("tools")).expect("create the stand-in repo");
            git(&repo_root, &["init", "-q"]);

            let hook = repo_root.join(".git/hooks/pre-push");
            std::fs::create_dir_all(hook.parent().expect("hooks dir")).expect("create hooks dir");
            std::fs::write(&hook, &hook_body).expect("install the generated pre-push hook");
            make_executable(&hook);
        }

        // The hook points a libviprs push at the sibling harness's script and a
        // libviprs-tests push at the pushed tree's own copy; in this workspace
        // those are the same file, and it must exist or the hook bails out
        // before it ever reaches the skip decision.
        let stub = root.join("libviprs-tests/tools/run-tests.sh");
        std::fs::write(&stub, format!("#!/bin/sh\necho \"{RAN_MARKER}\"\n"))
            .expect("write the stub run-tests.sh");
        make_executable(&stub);

        Workspace { _dir: dir, root }
    }

    fn repo(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Commit `files` in `repo` and return the commit's oid.
    fn commit(&self, repo: &str, message: &str, files: &[(&str, &str)]) -> String {
        let root = self.repo(repo);
        for (rel, contents) in files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create a directory for a stand-in file");
            }
            std::fs::write(&path, contents).expect("write a stand-in file");
        }
        git(&root, &["add", "-A"]);
        git(
            &root,
            &[
                "-c",
                "user.name=prepush guard",
                "-c",
                "user.email=guard@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                message,
            ],
        );
        git(&root, &["rev-parse", "HEAD"]).trim().to_string()
    }

    /// Run the installed hook the way git does: from the top of the working
    /// tree, with the ref updates on stdin.
    fn push(&self, repo: &str, before: &str, after: &str, force_all: bool) -> String {
        let root = self.repo(repo);
        let mut cmd = Command::new("bash");
        cmd.arg(root.join(".git/hooks/pre-push"))
            .arg("origin")
            .arg("git@example.invalid:libviprs/x.git")
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for leaked in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_PREFIX",
            "LIBVIPRS_PREPUSH_ALL",
        ] {
            cmd.env_remove(leaked);
        }
        if force_all {
            cmd.env("LIBVIPRS_PREPUSH_ALL", "1");
        }

        let mut child = cmd.spawn().expect("run the generated pre-push hook");
        {
            let stdin = child.stdin.as_mut().expect("hook stdin");
            writeln!(stdin, "refs/heads/main {after} refs/heads/main {before}")
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
        assert!(
            out.status.success(),
            "the pre-push hook exited {} on a {repo} push:\n{text}",
            out.status
        );
        text
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
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

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("make the stand-in script executable");
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The skip list decides nothing on its own; the hook does. So install the
/// hook, push at it, and read the answer off the run.
///
/// These are the six rows the PR body reported by hand, in the tree, where a
/// change to `inert_path` has to face them.
#[test]
fn the_hook_skips_and_runs_the_way_the_list_says() {
    let ws = Workspace::new();

    let base = [
        ("README.md", "# stand-in\n"),
        (".gitignore", "target/\n*.dylib\n"),
        (".github/workflows/ci.yml", "name: ci\n"),
        ("LICENSE", "stand-in licence\n"),
        ("docs/note.md", "stand-in note\n"),
        ("src/lib.rs", "// stand-in\n"),
        (
            "tools/run-tests.sh",
            "#!/bin/sh\necho \"STUB-RAN-THE-SUITE\"\n",
        ),
    ];

    let cases: &[(&str, &str, &str, bool, &str)] = &[
        (
            "libviprs-tests",
            ".gitignore",
            "target/\n",
            true,
            "tests/pdfium_provenance.rs reads this repo's .gitignore for the \
             native-binary patterns (issue #56), and a push that drops them is \
             exactly the push that must not skip",
        ),
        (
            "libviprs-tests",
            ".github/workflows/ci.yml",
            "name: ci\njobs: {}\n",
            true,
            "the pinning guards read this repo's workflows",
        ),
        (
            "libviprs-tests",
            "LICENSE",
            "stand-in licence, revised\n",
            false,
            "nothing reads it",
        ),
        (
            "libviprs-tests",
            "tools/run-tests.sh",
            "#!/bin/sh\necho \"STUB-RAN-THE-SUITE\"\n# changed\n",
            true,
            "it is the suite",
        ),
        (
            "libviprs",
            ".github/workflows/ci.yml",
            "name: ci\njobs: {}\n",
            false,
            "no test reads the core checkout's workflows",
        ),
        (
            "libviprs",
            "src/lib.rs",
            "// stand-in, changed\n",
            true,
            "it is the library under test",
        ),
        (
            "libviprs",
            "README.md",
            "# stand-in, revised\n",
            true,
            "the documentation guards read the core checkout's README",
        ),
    ];

    for (repo, path, contents, should_run, why) in cases {
        let before = ws.commit(repo, "base", &base);
        let after = ws.commit(repo, "change", &[(path, contents)]);
        let out = ws.push(repo, &before, &after, false);

        let ran = out.contains(RAN_MARKER);
        let skipped = out.contains("skipping it");
        assert!(
            ran != skipped,
            "the hook neither clearly ran nor clearly skipped a {repo} push \
             touching {path}:\n{out}"
        );
        assert_eq!(
            ran,
            *should_run,
            "a {repo} push touching only {path} must {} the suite, because {why}. \
             The hook said:\n{out}",
            if *should_run { "run" } else { "skip" }
        );
    }
}

/// Whatever the list says, there has to be a way to say "run it anyway", or
/// the next person who does not trust the filter goes back to `--no-verify`
/// and takes the whole gate with them.
///
/// Asserted twice over, because the bare name `LIBVIPRS_PREPUSH_ALL` appears
/// three times in the hook and is expanded once: delete the one line that
/// reads it, keep the comment, and a `contains("LIBVIPRS_PREPUSH_ALL")` guard
/// stays green with the escape hatch gone.
#[test]
fn the_skip_can_be_overridden() {
    let hook = generated_pre_push();
    assert!(
        hook.contains("${LIBVIPRS_PREPUSH_ALL"),
        "the pre-push hook mentions LIBVIPRS_PREPUSH_ALL but never expands it, \
         so the escape hatch is documentation only. Without a real one the only \
         way past a wrong skip decision is --no-verify, which is what #683 is \
         about"
    );

    // And the same thing again through the hook itself, on a push it would
    // otherwise skip.
    let ws = Workspace::new();
    let before = ws.commit(
        "libviprs-tests",
        "base",
        &[
            ("LICENSE", "stand-in licence\n"),
            (
                "tools/run-tests.sh",
                "#!/bin/sh\necho \"STUB-RAN-THE-SUITE\"\n",
            ),
        ],
    );
    let after = ws.commit(
        "libviprs-tests",
        "licence tweak",
        &[("LICENSE", "stand-in licence, revised\n")],
    );

    let skipped = ws.push("libviprs-tests", &before, &after, false);
    assert!(
        !skipped.contains(RAN_MARKER),
        "a LICENSE-only push should have been skipped, so the override below \
         proves nothing. The hook said:\n{skipped}"
    );

    let forced = ws.push("libviprs-tests", &before, &after, true);
    assert!(
        forced.contains(RAN_MARKER),
        "LIBVIPRS_PREPUSH_ALL=1 must run the suite on a push the list would \
         have skipped. The hook said:\n{forced}"
    );
}

/// An out-of-memory kill is not a test failure and must never be reported as
/// one. Three lanes read a SIGKILL as a broken test because nothing said
/// otherwise.
#[test]
fn run_tests_names_an_out_of_memory_kill() {
    let script = read("tools/run-tests.sh");
    assert!(
        script.contains("OOMKilled"),
        "tools/run-tests.sh must ask the daemon whether the container was \
         killed for memory before it reports a failure, or a cgroup kill \
         arrives looking like whichever test was running at the time (#683)"
    );
    assert!(
        script.contains("MemTotal"),
        "tools/run-tests.sh must compare its memory ceiling against what the \
         Docker VM actually has. A ceiling above the VM total never binds, so \
         the kill lands outside the container and comes back unexplained (#683)"
    );
}

/// The budget is reported before anything is built, and it is a real number
/// that responds to the knobs. `--plan` makes that answerable without Docker.
#[test]
fn run_tests_reports_the_budget_it_will_use() {
    let out = Command::new("bash")
        .arg(repo_root().join("tools/run-tests.sh"))
        .arg("--plan")
        .env("RUN_TESTS_BUILD_JOBS", "1")
        .output()
        .expect("run tools/run-tests.sh --plan");
    assert!(
        out.status.success(),
        "`run-tests.sh --plan` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan = String::from_utf8_lossy(&out.stdout);
    let budget = plan
        .lines()
        .find(|l| l.trim_start().starts_with("budget:"))
        .unwrap_or_else(|| panic!("the plan does not report a budget:\n{plan}"));
    assert!(
        budget.contains("--memory=") && budget.contains("1 cargo build job"),
        "the plan must name the memory ceiling and honour RUN_TESTS_BUILD_JOBS. \
         It said: {budget}"
    );
}

/// A fractional ceiling used to be an arithmetic syntax error, and under
/// `set -e` that took the whole run out rather than rejecting the value.
#[test]
fn run_tests_accepts_a_fractional_memory_ceiling() {
    let out = Command::new("bash")
        .arg(repo_root().join("tools/run-tests.sh"))
        .arg("--plan")
        .env("RUN_TESTS_MEMORY", "1.5g")
        .output()
        .expect("run tools/run-tests.sh --plan");
    assert!(
        out.status.success(),
        "`RUN_TESTS_MEMORY=1.5g run-tests.sh --plan` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan = String::from_utf8_lossy(&out.stdout);
    assert!(
        plan.contains("--memory=1536m"),
        "RUN_TESTS_MEMORY=1.5g must resolve to 1536m. The plan said:\n{plan}"
    );

    let junk = Command::new("bash")
        .arg(repo_root().join("tools/run-tests.sh"))
        .arg("--plan")
        .env("RUN_TESTS_MEMORY", "lots")
        .output()
        .expect("run tools/run-tests.sh --plan");
    assert!(
        !junk.status.success(),
        "RUN_TESTS_MEMORY=lots must be rejected by name, not carried into a \
         --memory flag. It said:\n{}",
        String::from_utf8_lossy(&junk.stdout)
    );
}
