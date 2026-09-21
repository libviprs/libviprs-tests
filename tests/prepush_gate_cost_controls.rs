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
//! Two of them lift `inert_path()` out of the hook and *execute* it: one runs
//! every tracked path in both repos through it and pins the set that comes
//! back inert, the other installs the whole hook in a throwaway workspace and
//! drives real pushes past it. An earlier pair of text guards here could not
//! fail for the reason they claimed: a reviewer flipped `inert_path`'s
//! fallthrough to `true`, disabling the gate for every push in every repo, and
//! both stayed green.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

mod common;
use common::hooks::{RAN_MARKER, STUB_RUN_TESTS, Workspace, pre_push_hook, read, repo_root};

// ---------------------------------------------------------------------------
// Running the hook's own `inert_path()`
// ---------------------------------------------------------------------------

/// The `inert_path()` shell function, lifted verbatim out of the hook and
/// wrapped in a driver that prints the inert ones from a list of paths on
/// stdin.
///
/// Lifting the real text is the point. A guard that re-implements the case
/// statement in Rust tests the re-implementation, and a guard that greps the
/// text tests the grep; both were tried here and both waved through six
/// poisoned entries and a flipped fallthrough.
fn inert_path_driver() -> String {
    let hook = pre_push_hook();
    let start = hook.find("inert_path() {").expect(
        "the pre-push hook must decide what it may skip in one place, a shell \
         function called inert_path()",
    );
    let end = hook[start..]
        .find("\n}\n")
        .map(|i| start + i + 2)
        .expect("unterminated inert_path()");
    let body = &hook[start..end];
    // `TREE` is the tree whose commits are going out, which is where the hook
    // looks for tests that pull a candidate in with `include_str!`. git gives
    // the hook one; the driver is handed one.
    format!(
        "REPO_NAME=\"$1\"\n\
         TREE=\"$2\"\n\
         {body}\n\
         while IFS= read -r candidate; do\n\
         \x20   [ -z \"$candidate\" ] && continue\n\
         \x20   if inert_path \"$candidate\"; then printf '%s\\n' \"$candidate\"; fi\n\
         done\n"
    )
}

/// Which of `paths` the hook would treat as inert for a push to `repo_name`
/// out of the checkout at `tree`.
fn inert_paths(repo_name: &str, tree: &Path, paths: &[String]) -> BTreeSet<String> {
    let dir = tempfile::tempdir().expect("temp dir for the inert_path driver");
    let script = dir.path().join("inert_path.sh");
    std::fs::write(&script, inert_path_driver()).expect("write the inert_path driver");

    // `bash`, not `sh`. The hook's shebang is `/usr/bin/env bash` and
    // `tests/common/hooks.rs` drives the whole thing with `bash`, so a driver
    // that reached for `sh` was certifying this function under an interpreter
    // it never runs on. On macOS that gap is bash 3.2.57 versus bash 5, and it
    // is not cosmetic: 3.2 cannot parse a `case` inside `"$( ... )"`, so the
    // scan below used to die there, come back empty and answer inert for
    // everything, and this guard could not see it.
    let mut child = Command::new("bash")
        .arg(&script)
        .arg(repo_name)
        .arg(tree)
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

/// The same for the core checkout, where `.gitignore` is inert because nothing
/// in either repo opens the core's copy of it.
///
/// `.github/` is not on this list as a directory and no longer answers on the
/// prefix. It used to, and three workflow files sat here as a result:
/// `ci.yml`, `merge-gate.yml` and `publish.yml` were pinned inert while the
/// core's own guards pulled all three in with `include_str!`
/// (`tests/ci_feature_coverage.rs`, `tests/local_gate_is_the_job_list.rs`,
/// `tests/doc_link_gate.rs`, `tests/miri_invocation_parity.rs`,
/// `tests/pmtiles_release_readiness.rs`). A push editing only a workflow
/// skipped the guard that reads it, which is #214's `docs/*` bug wearing a
/// different prefix, and libviprs/libviprs#1117's EPIC issue form is what
/// finally made this guard say so.
///
/// `docs/` is not on this list as a directory either, and for the same reason.
/// Whether a doc is inert is a question about the tree, not about the path:
/// `docs/streaming-pdf-rotation.md` is inert because nothing pulls it in, and
/// libviprs/libviprs#1010's `docs/pmtiles-benchmarks.md` is not, because
/// `tests/pmtiles_release_readiness.rs` reads it with `include_str!` and
/// asserts on its columns and on the test names it quotes. `inert_path()` goes
/// and looks, which is why only the first of those two is named here.
///
/// A standing obligation comes with that, and it is the cost of deciding on the
/// tree rather than on the path. This list is now measured against three
/// different core trees, and it has to be true of all three at once: the pinned
/// `COUNTERPART_REV` that this repo's CI clones, the PR head that libviprs' own
/// Integration Tests job checks out, and whatever sits at `../libviprs` locally
/// when the hook runs at pre-push time. They agree today. They stop agreeing
/// the moment the pin lags `main` across a commit that changes whether a
/// `.github/` or `docs/` path is read, because then one tree says inert and
/// another says read while this list can only say one of them. So a commit that
/// adds or drops an `include_str!` on one of those paths has to move the pin in
/// the same wave rather than after it.
const INERT_IN_LIBVIPRS: &[&str] = &[".gitignore", "LICENSE", "docs/streaming-pdf-rotation.md"];

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

        let got = inert_paths(repo_name, &root, &paths);
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
        let tree = if *repo_name == "libviprs" {
            core_root()
        } else {
            repo_root()
        };
        let inert = inert_paths(repo_name, &tree, &[(*path).to_string()]);
        assert!(
            inert.is_empty(),
            "the pre-push gate would skip a {repo_name} push that changes only \
             {path}, but {why}. A skip there lets a real failure through as a \
             saved five minutes (#683)."
        );
    }
}

/// A doc a test pulls in with `include_str!` is read, and one nothing pulls in
/// is inert. Both halves, against a tree built here rather than against
/// whatever the two checkouts happen to hold.
///
/// `docs/*` used to mean inert on the strength of the pattern alone, and the
/// day a test pulled a doc in, the gate started waving through the doc edits
/// that break it: libviprs/libviprs#1010 adds `docs/pmtiles-benchmarks.md`,
/// whose contents two cells in `tests/pmtiles_release_readiness.rs` assert on.
/// A push changing only that file skipped the suite and failed in CI instead.
///
/// Both halves are here because either one passes on its own for the wrong
/// reason. Delete the scan and the read doc comes back inert; make the scan
/// answer yes to everything and the unread one stops being inert. Only the
/// pair pins the rule.
#[test]
fn a_doc_a_test_includes_is_read_and_one_nothing_includes_is_inert() {
    let dir = tempfile::tempdir().expect("temp dir for a stand-in tree");
    let tree = dir.path();
    std::fs::create_dir_all(tree.join("tests/common")).expect("stand-in tests directory");
    std::fs::create_dir_all(tree.join("docs")).expect("stand-in docs directory");

    let read_by_a_test = "docs/read-by-a-test.md";
    let read_from_a_submodule = "docs/read-from-a-submodule.md";
    let read_by_nothing = "docs/read-by-nothing.md";
    for rel in [read_by_a_test, read_from_a_submodule, read_by_nothing] {
        std::fs::write(tree.join(rel), "stand-in doc\n").expect("write a stand-in doc");
    }
    std::fs::write(
        tree.join("tests/reads_a_doc.rs"),
        format!("const DOC: &str = include_str!(\"../{read_by_a_test}\");\n"),
    )
    .expect("write the stand-in test that reads a doc");
    // One directory deeper, so the answer cannot come from counting `../`.
    std::fs::write(
        tree.join("tests/common/mod.rs"),
        format!("pub const DOC: &str = include_str!(\"../../{read_from_a_submodule}\");\n"),
    )
    .expect("write the stand-in helper that reads a doc");

    let candidates: Vec<String> = [read_by_a_test, read_from_a_submodule, read_by_nothing]
        .iter()
        .map(|p| (*p).to_string())
        .collect();
    let inert = inert_paths("libviprs", tree, &candidates);

    for read in [read_by_a_test, read_from_a_submodule] {
        assert!(
            !inert.contains(read),
            "a test in this tree pulls {read} in with include_str!, so a push \
             that changes only that file changes what a test asserts against. \
             inert_path() called it inert, which is the gate declining to run \
             the test that would have caught it (#683, libviprs/libviprs#1010). \
             It called these inert: {inert:?}"
        );
    }
    assert!(
        inert.contains(read_by_nothing),
        "nothing in this tree reads {read_by_nothing}, so a push that changes \
         only that file cannot fail a test and the gate should skip it. \
         inert_path() wanted to run the whole Docker suite for it, which is \
         what taught everyone to reach for --no-verify (#683). It called these \
         inert: {inert:?}"
    );
}

// ---------------------------------------------------------------------------
// Driving the whole hook
// ---------------------------------------------------------------------------

/// The skip list decides nothing on its own; the hook does. So install the
/// hook, push at it, and read the answer off the run.
///
/// These are the rows the PR body used to report by hand, in the tree, where a
/// change to `inert_path` has to face them.
///
/// The `.github/` pair is load-bearing and easy to drop as redundant. One row
/// expects a workflow nothing reads to skip and the next expects a workflow a
/// test reads to run, and only the second one can tell this hook apart from a
/// revert to deciding `.github/` on the prefix, because a prefix rule agrees
/// with the first row.
#[test]
fn the_hook_skips_and_runs_the_way_the_list_says() {
    let ws = Workspace::new();

    let base = [
        ("README.md", "# stand-in\n"),
        (".gitignore", "target/\n*.dylib\n"),
        (".github/workflows/ci.yml", "name: ci\n"),
        ("LICENSE", "stand-in licence\n"),
        ("docs/note.md", "stand-in note\n"),
        ("docs/read-by-a-test.md", "stand-in doc\n"),
        (
            "tests/reads_a_doc.rs",
            "const DOC: &str = include_str!(\"../docs/read-by-a-test.md\");\n",
        ),
        // The same pairing for `.github/`, and it is the row that tells this
        // PR's hook apart from the one it replaces. Without a test in the tree
        // pulling a workflow in, every `.github/` row here expects
        // `should_run=false`, which a hook that answers "inert" on the prefix
        // alone also says, so reverting the fix left this whole test green.
        (
            ".github/workflows/read-by-a-test.yml",
            "name: read-by-a-test\n",
        ),
        (
            "tests/reads_a_workflow.rs",
            "const WORKFLOW: &str = \
             include_str!(\"../.github/workflows/read-by-a-test.yml\");\n",
        ),
        ("src/lib.rs", "// stand-in\n"),
        ("tools/run-tests.sh", STUB_RUN_TESTS),
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
            "#!/bin/sh\n# changed\necho \"STUB-RAN-THE-SUITE\"\n",
            true,
            "it is the suite",
        ),
        (
            "libviprs",
            ".github/workflows/ci.yml",
            "name: ci\njobs: {}\n",
            false,
            "nothing in THIS stand-in tree pulls it in. The real core checkout \
             is the other way round, and deliberately so: its own guards read \
             all three of its workflows with include_str!, so there the same \
             push runs the suite. Both answers come from the scan rather than \
             from the .github/ prefix, which is the whole point",
        ),
        (
            "libviprs",
            ".github/workflows/read-by-a-test.yml",
            "name: read-by-a-test\njobs: {}\n",
            true,
            "tests/reads_a_workflow.rs pulls it in with include_str!, which is \
             the shape of the real core checkout, where ci.yml, merge-gate.yml \
             and publish.yml are all read that way. This is the row that fails \
             if .github/ goes back to answering inert on the prefix: the row \
             above it says false and a prefix rule says false too, so the two \
             hooks are indistinguishable without this one",
        ),
        (
            "libviprs",
            "docs/note.md",
            "stand-in note, revised\n",
            false,
            "nothing in the tree pulls it in",
        ),
        (
            "libviprs",
            "docs/read-by-a-test.md",
            "stand-in doc, revised\n",
            true,
            "tests/reads_a_doc.rs pulls it in with include_str!, so editing it \
             changes what a test asserts against. This is the shape of \
             libviprs/libviprs#1010's docs/pmtiles-benchmarks.md, which two \
             cells in tests/pmtiles_release_readiness.rs read",
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
        let out = ws.push(repo).range(&before, &after).run();

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
/// This used to assert `hook.contains("${LIBVIPRS_PREPUSH_ALL")` as well. The
/// bare name appears three times in the hook and is expanded once, so that
/// half was there to catch "delete the line that reads it, keep the comment",
/// and it is worth exactly nothing next to running the push twice.
#[test]
fn the_skip_can_be_overridden() {
    let ws = Workspace::new();
    let before = ws.commit(
        "libviprs-tests",
        "base",
        &[
            ("LICENSE", "stand-in licence\n"),
            ("tools/run-tests.sh", STUB_RUN_TESTS),
        ],
    );
    let after = ws.commit(
        "libviprs-tests",
        "licence tweak",
        &[("LICENSE", "stand-in licence, revised\n")],
    );

    let skipped = ws.push("libviprs-tests").range(&before, &after).run();
    assert!(
        !skipped.contains(RAN_MARKER),
        "a LICENSE-only push should have been skipped, so the override below \
         proves nothing. The hook said:\n{skipped}"
    );

    let forced = ws
        .push("libviprs-tests")
        .range(&before, &after)
        .force_all()
        .run();
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
