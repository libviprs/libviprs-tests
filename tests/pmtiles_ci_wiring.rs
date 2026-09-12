//! Guards that the PMTiles interop job in CI is a job and not decoration
//! (libviprs-tests#202).
//!
//! `tests/pmtiles_interop.rs` runs an external binary when it can find one and
//! skips when it cannot, the same shape the CLI-differential cells use. A skip
//! reads to `cargo test` as a pass, so the skip is only safe while something
//! makes CI refuse to take it. That something is `VIPRS_REQUIRE_GO_PMTILES=1`
//! on the interop job, and this file is what stops the env line being deleted
//! or the job being renamed out from under it.
//!
//! # Why this is a separate binary from the interop cells
//!
//! Everything here reads repository files: no network, no sibling checkout, no
//! external binary, and, deliberately, **no `libviprs::pmtiles`**. So it
//! compiles and runs against any counterpart, including one that predates
//! PMTiles entirely. The interop cells themselves cannot say that, and a guard
//! that only compiles once the thing it guards already works is not much of a
//! guard.
//!
//! # The drift these catch
//!
//! The digest CI verifies the download with and the digest the committed
//! fixtures were produced under are the same number written in two places. A
//! test that reads it from one of them cannot notice them diverging, so these
//! read the number out of `tests/fixtures/pmtiles/vectors/*.json`, which the
//! oracle's dump program wrote, and require `ci.yml` to carry that one.

use std::path::{Path, PathBuf};

mod common;
use common::workflows::{Step, Workflow, read_workflow, workflow};

/// The job that lays the pinned reference binary down and runs the cells that
/// need it.
const INTEROP_JOB: &str = "pmtiles-interop";
/// The job that builds `viprs` and runs the CLI-driven cells against it.
const CLI_JOB: &str = "cli-differential";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The `produced_by` block of a committed vector file.
///
/// Panics rather than returning `None` on anything missing: every one of those
/// is "the guard below has nothing to compare against", and a guard with
/// nothing to compare against passes.
fn produced_by(vector_file: &str) -> serde_json::Value {
    let text = read(&format!("tests/fixtures/pmtiles/vectors/{vector_file}"));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        panic!("tests/fixtures/pmtiles/vectors/{vector_file} is not JSON: {e}")
    });
    value
        .get("produced_by")
        .cloned()
        .unwrap_or_else(|| panic!("{vector_file} has no produced_by block"))
}

fn produced_by_str(vector_file: &str, key: &str) -> String {
    let block = produced_by(vector_file);
    block
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{vector_file}'s produced_by has no string {key:?}"))
        .to_string()
}

fn is_lowercase_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The committed fixtures and the CI download have to name one release.
///
/// Two numbers in two files that must be the same number is drift waiting to
/// happen, and the failure it produces is the worst kind: a green interop job
/// that verified our archives with a different build of the tool than the one
/// the goldens came from.
fn assert_ci_pins_the_release_the_fixtures_came_from(ci: &Workflow) {
    let job = ci.job(INTEROP_JOB);
    let install = job
        .steps
        .iter()
        .find(|s| s.run().contains("sha256sum -c"))
        .unwrap_or_else(|| panic!("no install step in `{INTEROP_JOB}`"));

    let tag = produced_by_str("tiles.json", "release_tag");
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    assert!(
        !version.is_empty(),
        "the fixtures' release_tag is empty, so there is nothing to pin against"
    );
    let pinned = install.env_value("GO_PMTILES_VERSION").unwrap_or_else(|| {
        panic!(
            "the install step of `{INTEROP_JOB}` does not set GO_PMTILES_VERSION \
             in its own `env:`. Anywhere else in the file, a comment included, \
             is not a pin."
        )
    });
    assert_eq!(
        pinned, version,
        "ci.yml downloads go-pmtiles {pinned} and tests/fixtures/pmtiles/ was \
         produced by {version}. Verifying our archives with a different build \
         of the tool is the pleasant kind of wrong answer."
    );
    assert!(
        install.run().contains("v${GO_PMTILES_VERSION}") || install.run().contains(&version),
        "the install step pins GO_PMTILES_VERSION and its download URL does not \
         use it, so the two can drift apart silently. The script reads:\n{}",
        install.run()
    );

    let digest = produced_by_str("tiles.json", "linux_amd64_tarball_sha256");
    assert!(
        is_lowercase_hex(&digest, 64),
        "the fixtures record an amd64 tarball digest that is not 64 lowercase \
         hex digits ({digest:?}), so `sha256sum -c` could never accept it"
    );
    let ci_digest = install
        .env_value("GO_PMTILES_TARBALL_SHA256")
        .unwrap_or_else(|| panic!("the install step does not set GO_PMTILES_TARBALL_SHA256"));
    assert_eq!(
        ci_digest, digest,
        "ci.yml verifies the download against {ci_digest} and the fixtures were \
         produced from {digest}. Runners are x86_64, so amd64 is the one CI \
         downloads."
    );

    // The two vector files were dumped by the same run, so they have to agree
    // with each other as well as with ci.yml. If they ever disagree, the digest
    // above is evidence about one of them and nothing about the other.
    assert_eq!(
        produced_by_str("header.json", "linux_amd64_tarball_sha256"),
        digest,
        "tiles.json and header.json disagree about which go-pmtiles tarball \
         produced them"
    );
    assert_eq!(
        produced_by_str("header.json", "source_commit"),
        produced_by_str("tiles.json", "source_commit"),
        "tiles.json and header.json disagree about the go-pmtiles commit"
    );
}

/// The interop job has to be a job that runs.
///
/// Every guard below reads the workflow's block structure rather than searching
/// the file for a string, because the review that prompted this measured seven
/// separate one-line edits that GitHub honours and every substring guard
/// missed. `if: false` on the job, `continue-on-error: true` on the step, the
/// `env:` block turned into a comment so the variable name survives as text, a
/// `--test` flag moved to a job with no external binary, the require-variable
/// set on a different job, `set -euo pipefail` deleted from the install script,
/// and `|| true` appended to the digest checks. All seven left 14 of 14 guards
/// green. [`the_wiring_guards_red_on_the_edits_they_exist_to_catch`] applies
/// each of them to the real file and requires a failure.
fn assert_the_job_is_not_disabled(ci: &Workflow) {
    // A workflow that triggers on nothing is the cheapest way to turn every
    // guard in this repository into decoration, and `branches:` naming a branch
    // that does not exist is one word.
    let on = ci.on().join("\n");
    assert!(
        on.contains("push"),
        "the workflow no longer triggers on push, so nothing here runs on the \
         event this repository is gated by. It reads:\n{on}"
    );
    let mut branch_filters = 0usize;
    for line in ci.on() {
        if let Some(branches) = line.strip_prefix("branches:") {
            branch_filters += 1;
            assert!(
                branches.contains('*'),
                "the workflow restricts itself to {branches}, so a push to \
                 anything else runs no job at all and every guard in this file \
                 passes having checked a workflow that never ran. This repo \
                 runs on every branch on purpose."
            );
        }
    }
    assert!(
        branch_filters <= 1,
        "the `on:` block carries {branch_filters} `branches:` filters, and this \
         guard only reasons about one"
    );

    let job = ci.job(INTEROP_JOB);
    let job_tolerated = job.key("continue-on-error").unwrap_or("false");
    assert_eq!(
        job_tolerated, "false",
        "the `{INTEROP_JOB}` job sets continue-on-error: {job_tolerated} on \
         itself, so it can fail without failing the run, which is the same \
         green as never having run it"
    );
    assert!(
        job.key("if").is_none(),
        "the `{INTEROP_JOB}` job carries `if: {}`, so whether it runs at all now \
         depends on a condition. A job that does not run reports no failures, \
         and every guard about its contents keeps passing.",
        job.key("if").unwrap_or_default()
    );
    assert!(
        !job.steps.is_empty(),
        "the `{INTEROP_JOB}` job has no steps"
    );
    for (index, step) in job.steps.iter().enumerate() {
        assert!(
            step.key("if").is_none(),
            "step {index} of `{INTEROP_JOB}` is conditional on `{}`",
            step.key("if").unwrap_or_default()
        );
        let tolerated = step.key("continue-on-error").unwrap_or("false");
        assert_eq!(
            tolerated, "false",
            "step {index} of `{INTEROP_JOB}` sets continue-on-error: \
             {tolerated}, so it can fail without failing the job, which is the \
             same green as never having run it"
        );
    }
}

/// The download has to be verified, and the thing inside it too.
///
/// The pdfium block this is modelled on verifies the tarball and stops there. A
/// tarball digest says the bytes arrived intact; it does not say the binary
/// inside is the one that wrote the goldens. Both digests exist, measured, in
/// `tests/fixtures/pmtiles/PROVENANCE.md`, so both get checked.
///
/// Counting `sha256sum -c` occurrences was the old shape of this and it cannot
/// fail usefully: `|| true` after each one leaves the count unchanged, and
/// deleting `set -euo pipefail` means a failing check no longer stops the
/// script, because the pipeline's status is discarded and the next line runs
/// anyway. Both were measured green. So this reads the install step's script.
fn assert_the_download_is_verified(ci: &Workflow) {
    let job = ci.job(INTEROP_JOB);
    let install = job
        .steps
        .iter()
        .find(|s| s.run().contains("sha256sum -c"))
        .unwrap_or_else(|| {
            panic!(
                "no step of `{INTEROP_JOB}` runs `sha256sum -c`, so nothing \
                 verifies the download"
            )
        });

    for var in ["GO_PMTILES_TARBALL_SHA256", "GO_PMTILES_BINARY_SHA256"] {
        let value = install.env_value(var).unwrap_or_else(|| {
            panic!(
                "the install step does not set {var} in its own `env:`. The \
                 tarball digest says the bytes arrived intact; the binary \
                 digest is what says the thing inside is the build that wrote \
                 the goldens."
            )
        });
        assert!(
            is_lowercase_hex(value, 64),
            "{var} is {value:?}, which is not 64 lowercase hex digits, so \
             `sha256sum -c` could never accept it"
        );
    }

    // `run_code()` and not `run()`: comment-only lines are dropped and line
    // continuations are folded, which is what the shell sees. Both matter.
    // `# set -euo pipefail` satisfies a `contains` on the raw text, and a
    // `|| true` sitting after a trailing backslash is part of the command on
    // the line before it. Measured: with either hole open, every guard here
    // stays green while a tampered tarball installs.
    let script = install.run_code();
    let guarded = script
        .lines()
        .position(|l| l.trim_start().starts_with("set -e"))
        .unwrap_or_else(|| {
            panic!(
                "the install step's script does not `set -e`, so a failing \
                 `sha256sum -c` no longer stops it: the pipeline's status is \
                 discarded and the next line installs the binary anyway. The \
                 script reads:\n{script}"
            )
        });
    for (index, line) in script.lines().enumerate() {
        assert!(
            !line.trim_start().starts_with("set +e"),
            "line {} of the install step turns errexit back off, which undoes \
             the `set -e` on line {}: {line:?}",
            index + 1,
            guarded + 1
        );
    }

    let checks: Vec<&str> = script
        .lines()
        .filter(|l| l.contains("sha256sum -c"))
        .collect();
    assert!(
        checks.len() >= 2,
        "the install step makes {} digest checks and needs two, one for the \
         tarball and one for the binary inside it",
        checks.len()
    );
    for line in &checks {
        for swallow in ["|| true", "|| :", "; true", "|| echo"] {
            assert!(
                !line.contains(swallow),
                "a digest check cannot fail, it carries `{swallow}`: {line:?}"
            );
        }
    }
    assert!(
        install.key("continue-on-error").unwrap_or("false") == "false",
        "the install step tolerates its own failure, so an unverified binary \
         installs and the job carries on"
    );
}

/// The interop job must be unable to report green without running the binary.
///
/// This is the whole reason the job exists as its own cell rather than as a
/// line in the default `test` job: `cargo test` reads a skip as a pass, so the
/// skip in `tests/pmtiles_interop.rs` needs somewhere that refuses to take it.
///
/// The variable has to be set on the step that runs the cells. Anywhere else in
/// the file is not a guard: measured, moving it to the `test-pdfium` job leaves
/// a whole-file `contains` green while the interop cells skip every comparison.
fn assert_the_job_cannot_skip_to_a_false_green(ci: &Workflow) {
    let job = ci.job(INTEROP_JOB);
    let runner = job
        .steps
        .iter()
        .find(|s| runs_binary(s, "pmtiles_interop"))
        .unwrap_or_else(|| {
            panic!(
                "no step of `{INTEROP_JOB}` runs `cargo test --test \
                 pmtiles_interop`, so the env var below would guard a binary \
                 nothing invokes"
            )
        });
    let set = job.env_for(runner, "VIPRS_REQUIRE_GO_PMTILES");
    assert_eq!(
        set,
        Some("1"),
        "the step that runs the interop cells sees VIPRS_REQUIRE_GO_PMTILES as \
         {set:?}. It has to be 1, set either on that step or on the job, so a \
         runner that failed to lay go-pmtiles down hard-fails instead of \
         skipping every comparison and reporting a pass. The same line on a \
         different job says nothing about this one."
    );
}

/// Every `tests/pmtiles_*.rs` binary has to run in the interop job.
///
/// The same shape as `cli_differential_job_runs_every_diff_binary`, and for the
/// same reason: a future PMTiles cell that lands as a new file and is never
/// wired in runs nowhere and says nothing. The default `test` job's bare
/// `cargo test` does pick these up, so this is not about them running at all,
/// it is about them running in the one job that has the external binary. A
/// `--test` flag moved out of this job and into that one satisfies a whole-file
/// search while the cell runs where no oracle exists.
fn assert_the_job_runs_every_pmtiles_binary(ci: &Workflow) {
    let binaries = test_binaries("pmtiles_");
    assert!(
        !binaries.is_empty(),
        "expected at least one tests/pmtiles_*.rs binary; if they have all been \
         renamed this guard is pinning an empty set and will pass forever"
    );
    let job = ci.job(INTEROP_JOB);
    for name in &binaries {
        assert!(
            job.steps.iter().any(|s| runs_binary(s, name)),
            "the `{INTEROP_JOB}` job must run `cargo test --test {name}` (found \
             tests/{name}.rs and no step of that job names it). A `--test` line \
             elsewhere in the file runs it somewhere with no external binary."
        );
    }
}

/// Whether this step runs `cargo test --test <name>`, matched at a word
/// boundary.
///
/// `--test pmtiles_interop` is a prefix of `--test pmtiles_interop_extra`, so a
/// plain substring search calls the first one wired on a line that names only
/// the second.
fn runs_binary(step: &Step, name: &str) -> bool {
    let needle = format!("--test {name}");
    step.run().match_indices(&needle).any(|(i, _)| {
        step.run()[i + needle.len()..]
            .chars()
            .next()
            .is_none_or(char::is_whitespace)
    })
}

/// The `tests/<prefix>*.rs` binaries this repo has.
fn test_binaries(prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(repo_root().join("tests"))
        .expect("read tests/ dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with(prefix) && n.ends_with(".rs"))
        .map(|n| n.trim_end_matches(".rs").to_string())
        .collect();
    names.sort();
    names
}

/// One structural guard: a name for the failure message and the assertion.
type Guard = (&'static str, fn(&Workflow));

/// Every structural guard, so the mutation table cannot drift out of step with
/// the tests by one of them being added and not listed.
const WIRING_GUARDS: &[Guard] = &[
    ("the job is not disabled", assert_the_job_is_not_disabled),
    ("the download is verified", assert_the_download_is_verified),
    (
        "the job cannot skip to a false green",
        assert_the_job_cannot_skip_to_a_false_green,
    ),
    (
        "the job runs every pmtiles binary",
        assert_the_job_runs_every_pmtiles_binary,
    ),
    (
        "the cli cell runs where it cannot skip",
        assert_the_cli_cell_runs_where_it_cannot_skip,
    ),
    (
        "ci pins the release the fixtures came from",
        assert_ci_pins_the_release_the_fixtures_came_from,
    ),
];

#[test]
fn the_interop_job_is_wired_the_way_the_guards_require() {
    let ci = workflow("ci.yml");
    for (name, guard) in WIRING_GUARDS {
        eprintln!("checking: {name}");
        guard(&ci);
    }
}

/// Every edit that GitHub honours and a guard here has to refuse.
///
/// This is the part that was missing. A guard that reads a file and asserts
/// something about it is worth exactly what it costs to defeat, and these cost
/// one line each until they read structure. So each mutation is applied to the
/// real `ci.yml` and at least one guard has to refuse the result.
///
/// The numbering is the order they were found in, not a sequence: Y1 to Y8 came
/// from the review that prompted reading structure at all (Y6 was about the CLI
/// job's require-variable and lives in `tests/cli_counterpart_pinning.rs`,
/// which is why it is not here), and Y9 to Y14 came from the adversarial review
/// of the first version of this file, every one of them measured green against
/// the guards as they stood then.
///
/// They are applied to the real file rather than to a synthetic fixture on
/// purpose: a fixture drifts away from the workflow and then this passes while
/// describing a file that no longer exists. That costs something, and it is
/// worth naming. An anchor has to match **exactly once**, or the edit lands
/// somewhere else and the row blames the guards for a mutation they never saw.
/// Measured: changing the interop job's `set -euo pipefail` to `set -eu`, which
/// the guard accepts on purpose, left the Y4 anchor matching the `hook-mirror`
/// job's copy, so the edit applied there and the failure read as "every guard
/// passes on Y4".
#[test]
fn the_wiring_guards_red_on_the_edits_they_exist_to_catch() {
    let ci = read_workflow("ci.yml");

    let mutations: Vec<(&str, &str, String)> = vec![
        (
            "Y1: the job is disabled with `if: false`",
            "  pmtiles-interop:\n    name:",
            "  pmtiles-interop:\n    if: false\n    name:".to_string(),
        ),
        (
            "Y2: the step that runs the cells tolerates its own failure",
            "      - env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n",
            "      - continue-on-error: true\n        env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n".to_string(),
        ),
        (
            "Y3: the env block becomes a comment, so the name survives as text",
            "      - env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n        run: cargo test --test pmtiles_interop",
            "      # env:\n      #   VIPRS_REQUIRE_GO_PMTILES: 1\n      - run: cargo test --test pmtiles_interop".to_string(),
        ),
        (
            "Y4: the install script loses `set -euo pipefail`",
            "          set -euo pipefail\n          curl -fsSL -o go-pmtiles.tgz",
            "          curl -fsSL -o go-pmtiles.tgz".to_string(),
        ),
        (
            "Y5: the tarball digest check cannot fail",
            "go-pmtiles.tgz\" | sha256sum -c -",
            "go-pmtiles.tgz\" | sha256sum -c - || true".to_string(),
        ),
        (
            "Y7: a --test flag leaves the job that has the oracle",
            " --test pmtiles_bounded",
            String::new(),
        ),
        (
            "Y8: the require variable moves off the step that runs the cells",
            "          VIPRS_REQUIRE_GO_PMTILES: 1\n",
            "          VIPRS_REQUIRE_SIBLINGS: 1\n".to_string(),
        ),
        (
            "Y9: the job tolerates its own failure",
            "  pmtiles-interop:\n    name:",
            "  pmtiles-interop:\n    continue-on-error: true\n    name:".to_string(),
        ),
        (
            "Y10: `set -euo pipefail` is commented out, so it survives as text",
            "          set -euo pipefail\n          curl -fsSL -o go-pmtiles.tgz",
            "          # set -euo pipefail\n          curl -fsSL -o go-pmtiles.tgz".to_string(),
        ),
        (
            "Y11: `|| true` hides after a line continuation",
            "go-pmtiles/pmtiles\" | sha256sum -c -",
            "go-pmtiles/pmtiles\" | sha256sum -c - \\\n            || true".to_string(),
        ),
        (
            "Y12: the workflow stops triggering on any real branch",
            "branches: ['**']",
            "branches: ['no-such-branch']".to_string(),
        ),
        (
            "Y13: the job is disabled with a quoted key",
            "  pmtiles-interop:\n    name:",
            "  pmtiles-interop:\n    \"if\": false\n    name:".to_string(),
        ),
        (
            "Y14: the step tolerates its own failure, with a quoted key",
            "      - env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n",
            "      - 'continue-on-error': true\n        env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n".to_string(),
        ),
        (
            "Y16: the pinned release becomes a comment, so the version survives as text",
            "          GO_PMTILES_VERSION: 1.31.2\n",
            "          # GO_PMTILES_VERSION: 1.31.2\n          GO_PMTILES_VERSION: 1.30.0\n".to_string(),
        ),
        (
            "Y15: the CLI cell's --test flag becomes a comment in the same step",
            " --test cli_pmtiles",
            "\n        # TODO: re-enable --test cli_pmtiles once it is less flaky".to_string(),
        ),
    ];

    let stale: Vec<String> = mutations
        .iter()
        .map(|(name, anchor, _)| (name, ci.matches(*anchor).count()))
        .filter(|(_, hits)| *hits != 1)
        .map(|(name, hits)| format!("{name}: anchor matches {hits} times"))
        .collect();
    assert!(
        stale.is_empty(),
        "{stale:#?}\nAn anchor that matches more than once applies the edit \
         somewhere other than the place its row is about, and the row then \
         blames the guards for a mutation they never saw. An anchor that \
         matches nothing means ci.yml no longer has the shape that row was \
         written against. Every bad anchor is listed so they can be fixed in one \
         pass."
    );

    let mut results = Vec::new();
    for (name, anchor, replacement) in &mutations {
        let mutated = ci.replacen(*anchor, replacement.as_str(), 1);
        assert_ne!(&mutated, &ci, "{name}: the edit changed nothing");

        // Parsed once, outside the catch below. A mutant the reader refuses is
        // not a mutant a guard refused, and counting them together would let
        // every row in this table pass for the wrong reason without the table
        // looking any different. Measured: replacing two spaces with a tab is
        // "caught by" all four guards when the parse sits inside the closure.
        let parsed = Workflow::parse(&mutated);

        let caught: Vec<&str> = WIRING_GUARDS
            .iter()
            .filter(|(_, guard)| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| guard(&parsed))).is_err()
            })
            .map(|(label, _)| *label)
            .collect();
        results.push((*name, caught));
    }

    let blind: Vec<&str> = results
        .iter()
        .filter(|(_, caught)| caught.is_empty())
        .map(|(name, _)| *name)
        .collect();
    assert!(
        blind.is_empty(),
        "these edits to ci.yml are valid YAML that GitHub honours, and every \
         guard in this file passes on them: {blind:#?}"
    );
    for (name, caught) in &results {
        eprintln!("{name}\n    caught by: {caught:?}");
    }
}

/// The CLI-driven cell has to run in the one job that lays the CLI down, with
/// the env var that turns its skip into a panic reaching the same step.
///
/// `tests/cli_pmtiles.rs` is named `cli_pmtiles` rather than
/// `cli_pmtiles_diff`, so neither of the two wiring guards that already exist
/// sees it: `cli_differential_job_runs_every_diff_binary` globs
/// `tests/cli_*_diff.rs` and [`assert_the_job_runs_every_pmtiles_binary`] globs
/// `tests/pmtiles_*.rs`. The name is deliberate ("differential" in this repo
/// means differential against a vips oracle, and PMTiles has no vips oracle),
/// so the cost of the honest name is this guard.
///
/// It used to read `ci.yml` as text, find `--test cli_pmtiles` anywhere in the
/// file and slice back to the previous `- ` at six spaces. That is the idiom
/// the rest of this file exists to replace, and it is defeated the same way:
/// delete the flag from the run line and leave
/// `# TODO: re-enable --test cli_pmtiles` in the same step and it passes, with
/// the cell running nowhere.
fn assert_the_cli_cell_runs_where_it_cannot_skip(ci: &Workflow) {
    let cells = test_binaries("cli_pmtiles");
    assert!(
        !cells.is_empty(),
        "expected at least one tests/cli_pmtiles*.rs binary; with none, this \
         guard is pinning an empty set and passes forever"
    );

    let job = ci.job(CLI_JOB);
    for name in &cells {
        let runner = job
            .steps
            .iter()
            .find(|s| runs_binary(s, name))
            .unwrap_or_else(|| {
                panic!(
                    "no step of `{CLI_JOB}` runs `cargo test --test {name}` \
                     (found tests/{name}.rs and no step of that job names it). \
                     Nothing else wires this file in: it is neither a \
                     `cli_*_diff.rs` nor a `pmtiles_*.rs`."
                )
            });
        let set = job.env_for(runner, "VIPRS_REQUIRE_CLI");
        assert_eq!(
            set,
            Some("1"),
            "the step of `{CLI_JOB}` that runs `--test {name}` sees \
             VIPRS_REQUIRE_CLI as {set:?}. Without it an absent libviprs-cli \
             sibling makes every cell in that binary skip, and the job reports \
             green having compared nothing."
        );
    }
}

/// Every committed archive has to be described by the vector files and by the
/// provenance, and every file the vector files describe has to be committed.
///
/// The list-to-disk direction existed already. The disk-to-list direction is
/// the silent one: a `.pmtiles` committed under `tests/fixtures/pmtiles/` and
/// described by nothing is read by nothing, and nothing would notice if it were
/// wrong.
///
/// This compares the directory against the **vector files** rather than against
/// `GOLDENS` in `tests/common/pmtiles.rs`, and that is deliberate rather than
/// convenient. Importing that module would pull `libviprs::pmtiles` into this
/// binary, and the whole point of this file (see the module doc above) is that
/// it compiles and runs against any counterpart, including one that predates
/// PMTiles. A guard that only compiles once the thing it guards already works
/// is not much of a guard. Reading the JSON is also the stronger check: a
/// golden can be in `GOLDENS` and described by no vector file, and this catches
/// that too.
#[test]
fn every_committed_go_pmtiles_fixture_is_named_by_a_test_and_by_its_provenance() {
    let dir = repo_root().join("tests/fixtures/pmtiles");
    let provenance = std::fs::read_to_string(dir.join("PROVENANCE.md"))
        .expect("tests/fixtures/pmtiles/PROVENANCE.md");

    for rel in [
        "vectors/tiles.json",
        "vectors/header.json",
        "vectors/show.json",
        "vectors/sweep.json",
        "vectors/tileid.json",
    ] {
        assert!(
            dir.join(rel).is_file(),
            "tests/fixtures/pmtiles/{rel} is missing, and every assertion that \
             reads it would be a loop over nothing"
        );
    }

    let described = |vector_file: &str| -> Vec<String> {
        let raw = std::fs::read_to_string(dir.join("vectors").join(vector_file))
            .unwrap_or_else(|e| panic!("read vectors/{vector_file}: {e}"));
        let doc: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("vectors/{vector_file} does not parse: {e}"));
        doc.get("archives")
            .and_then(serde_json::Value::as_object)
            .unwrap_or_else(|| panic!("vectors/{vector_file} has no archives block"))
            .keys()
            .cloned()
            .collect()
    };
    let swept = described("sweep.json");
    let shown = described("show.json");
    let headers = described("header.json");

    let mut committed: Vec<String> = std::fs::read_dir(&dir)
        .expect("read the fixtures dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".pmtiles"))
        .collect();
    committed.sort();
    assert!(
        committed.len() >= 3,
        "only {} archives are committed, so this guard is pinning almost \
         nothing",
        committed.len()
    );

    for name in &committed {
        for (vector_file, names) in [
            ("sweep.json", &swept),
            ("show.json", &shown),
            ("header.json", &headers),
        ] {
            assert!(
                names.contains(name),
                "tests/fixtures/pmtiles/{name} is committed and \
                 vectors/{vector_file} does not describe it, so nothing in that \
                 file reads it and nothing would notice if it were wrong"
            );
        }
        assert!(
            provenance.contains(name),
            "PROVENANCE.md does not mention {name}. A golden with no provenance \
             is a file somebody generated, and this repo's whole oracle rule is \
             that it is not."
        );
    }

    for (vector_file, names) in [
        ("sweep.json", &swept),
        ("show.json", &shown),
        ("header.json", &headers),
    ] {
        for name in names {
            assert!(
                committed.contains(name),
                "vectors/{vector_file} describes {name} and no such archive is \
                 committed, so those rows are read by nothing"
            );
        }
    }
}
