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
#[test]
fn ci_pins_the_go_pmtiles_release_the_fixtures_were_produced_by() {
    let ci = read_workflow("ci.yml");

    let tag = produced_by_str("tiles.json", "release_tag");
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    assert!(
        !version.is_empty(),
        "the fixtures' release_tag is empty, so there is nothing to pin against"
    );
    assert!(
        ci.contains(&format!("GO_PMTILES_VERSION: {version}")),
        "ci.yml must download go-pmtiles {version}, the release \
         tests/fixtures/pmtiles/ was produced by, and it does not name it"
    );

    let digest = produced_by_str("tiles.json", "linux_amd64_tarball_sha256");
    assert!(
        is_lowercase_hex(&digest, 64),
        "the fixtures record an amd64 tarball digest that is not 64 lowercase \
         hex digits ({digest:?}), so `sha256sum -c` could never accept it"
    );
    assert!(
        ci.contains(&digest),
        "ci.yml must pin the amd64 tarball digest {digest} the fixtures name. \
         Runners are x86_64, so amd64 is the one CI downloads."
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
    let job = ci.job(INTEROP_JOB);
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

    let script = install.run();
    assert!(
        script.contains("set -euo pipefail") || script.contains("set -eu"),
        "the install step's script does not `set -e`, so a failing \
         `sha256sum -c` no longer stops it: the pipeline's status is discarded \
         and the next line installs the binary anyway. The script reads:\n{script}"
    );
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
        assert!(
            !line.contains("|| true") && !line.contains("|| :"),
            "a digest check cannot fail: {line:?}"
        );
    }
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
    let set = runner.env_value("VIPRS_REQUIRE_GO_PMTILES");
    assert_eq!(
        set,
        Some("1"),
        "the step that runs the interop cells sets VIPRS_REQUIRE_GO_PMTILES to \
         {set:?}. It has to be 1, on this step, so a runner that failed to lay \
         go-pmtiles down hard-fails instead of skipping every comparison and \
         reporting a pass."
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
];

#[test]
fn the_interop_job_is_wired_the_way_the_guards_require() {
    let ci = workflow("ci.yml");
    for (name, guard) in WIRING_GUARDS {
        eprintln!("checking: {name}");
        guard(&ci);
    }
}

/// Each of the seven edits the review measured has to redden something here.
///
/// This is the part that was missing. A guard that reads a file and asserts
/// something about it is worth exactly what it costs to defeat, and these cost
/// one line each until they read structure. So each mutation is applied to the
/// real `ci.yml`, the result is checked to still be a workflow, and at least
/// one guard has to refuse it.
///
/// They are applied to the real file rather than to a synthetic fixture on
/// purpose: a fixture drifts away from the workflow and then this passes while
/// describing a file that no longer exists.
#[test]
fn the_wiring_guards_red_on_the_edits_they_exist_to_catch() {
    let ci = read_workflow("ci.yml");

    let mutations: Vec<(&str, String)> = vec![
        (
            "Y1: the job is disabled with `if: false`",
            ci.replace(
                "  pmtiles-interop:\n    name:",
                "  pmtiles-interop:\n    if: false\n    name:",
            ),
        ),
        (
            "Y2: the step that runs the cells tolerates its own failure",
            ci.replace(
                "      - env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n",
                "      - continue-on-error: true\n        env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n",
            ),
        ),
        (
            "Y3: the env block becomes a comment, so the name survives as text",
            ci.replace(
                "      - env:\n          VIPRS_REQUIRE_GO_PMTILES: 1\n        run: cargo test --test pmtiles_interop",
                "      # env:\n      #   VIPRS_REQUIRE_GO_PMTILES: 1\n      - run: cargo test --test pmtiles_interop",
            ),
        ),
        (
            "Y4: the install script loses `set -euo pipefail`",
            ci.replace("          set -euo pipefail\n", ""),
        ),
        (
            "Y5: the digest checks cannot fail",
            ci.replace("| sha256sum -c -", "| sha256sum -c - || true"),
        ),
        (
            "Y7: a --test flag moves to a job with no external binary",
            ci.replace(" --test pmtiles_bounded", ""),
        ),
        (
            "Y8: the require variable moves off the step that runs the cells",
            ci.replace("          VIPRS_REQUIRE_GO_PMTILES: 1\n", "          VIPRS_REQUIRE_SIBLINGS: 1\n"),
        ),
    ];

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut results = Vec::new();
    for (name, mutated) in &mutations {
        if mutated == &ci {
            std::panic::set_hook(previous);
            panic!(
                "{name}: the edit changed nothing, so ci.yml no longer has the \
                 shape this mutation was written against and the row below \
                 proves nothing"
            );
        }
        let caught: Vec<&str> = WIRING_GUARDS
            .iter()
            .filter(|(_, guard)| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    guard(&Workflow::parse(mutated));
                }))
                .is_err()
            })
            .map(|(label, _)| *label)
            .collect();
        results.push((*name, caught));
    }
    std::panic::set_hook(previous);

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

/// The committed fixtures have to be the ones the tests name.
///
/// Not a digest check, which the loaders do at run time, but an existence check
/// that fails loudly rather than letting a missing archive turn into a skipped
/// sweep somewhere downstream. `tests/fixtures/pmtiles/PROVENANCE.md` is on the
/// list on purpose: a golden with no provenance is a file somebody generated,
/// and this repo's whole oracle rule is that it is not.
#[test]
fn the_committed_go_pmtiles_fixtures_are_all_present() {
    for rel in [
        "raster-z0z2.pmtiles",
        "dupes-z0z3.pmtiles",
        "leaves-z0z7.pmtiles",
        "distinct-z0z7.pmtiles",
        "header-mvt-z2z4.pmtiles",
        "vectors/tiles.json",
        "vectors/header.json",
        "vectors/show.json",
        "vectors/sweep.json",
        "PROVENANCE.md",
    ] {
        let path = repo_root().join("tests/fixtures/pmtiles").join(rel);
        assert!(
            path.is_file(),
            "tests/fixtures/pmtiles/{rel} is missing, and every interop \
             assertion that reads it would be a loop over nothing"
        );
    }
}

/// The CLI-driven cell has to run in the one job that lays the CLI down, with
/// the env var that turns its skip into a panic set on the same step.
///
/// `tests/cli_pmtiles.rs` is named `cli_pmtiles` rather than
/// `cli_pmtiles_diff`, so neither of the two wiring guards that already exist
/// sees it: `cli_differential_job_runs_every_diff_binary` globs
/// `tests/cli_*_diff.rs` and `the_interop_job_runs_every_pmtiles_binary` globs
/// `tests/pmtiles_*.rs`. The name is deliberate ("differential" in this repo
/// means differential against a vips oracle, and PMTiles has no vips oracle),
/// so the cost of the honest name is this guard.
///
/// The env check is the half that matters. `--test cli_pmtiles` in a job with
/// no `VIPRS_REQUIRE_CLI` is a cell that skips every assertion and reports a
/// pass, which is the same green a dev machine gives and for the same reason:
/// it did not run. So this looks for the var **inside the step that runs the
/// cell**, not anywhere in the file, because the file already contains that
/// string on a different job and a whole-file `contains` would pass while the
/// cell sat somewhere it can never run.
#[test]
fn the_cli_pmtiles_cell_runs_where_it_cannot_skip() {
    let mut cells: Vec<String> = std::fs::read_dir(repo_root().join("tests"))
        .expect("read tests/ dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("cli_pmtiles") && n.ends_with(".rs"))
        .map(|n| n.trim_end_matches(".rs").to_string())
        .collect();
    cells.sort();
    assert!(
        !cells.is_empty(),
        "expected at least one tests/cli_pmtiles*.rs binary; with none, this \
         guard is pinning an empty set and passes forever"
    );

    let ci = read_workflow("ci.yml");
    for name in &cells {
        // The match has to end at a word boundary. `--test cli_pmtiles` is a
        // prefix of `--test cli_pmtiles_extra`, so a plain substring search
        // would call `cli_pmtiles` wired on a line that only names a different
        // binary, which is the false pass this guard exists to prevent.
        let needle = format!("--test {name}");
        let at = ci
            .match_indices(&needle)
            .find(|(i, _)| {
                ci[i + needle.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| c.is_whitespace())
            })
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                panic!(
                    "ci.yml must run `cargo test --test {name}` (found \
                     tests/{name}.rs but no line naming exactly that binary). \
                     Nothing else wires this file in: it is neither a \
                     `cli_*_diff.rs` nor a `pmtiles_*.rs`."
                )
            });

        // The step this line belongs to, back to the previous `- ` at the
        // six-space step indent. Anything before that belongs to another step
        // and says nothing about this one.
        let head = &ci[..at];
        let step_start = head.rfind("\n      - ").map(|i| i + 1).unwrap_or(0);
        let step = &ci[step_start..at + needle.len()];
        assert!(
            step.contains("VIPRS_REQUIRE_CLI: 1") || step.contains("VIPRS_REQUIRE_CLI: \"1\""),
            "`--test {name}` runs in a step that does not set \
             VIPRS_REQUIRE_CLI=1, so an absent libviprs-cli sibling would make \
             every cell in it skip and the job would report green having \
             compared nothing. The step reads:\n{step}"
        );
    }
}
