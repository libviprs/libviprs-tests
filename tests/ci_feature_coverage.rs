//! Every cargo feature this suite declares has to say which CI cells run it,
//! and CI has to actually carry those cells.
//!
//! # The hole this closes
//!
//! The core grew this guard first (`tests/ci_feature_coverage.rs`, from
//! libviprs#816 and libviprs#824) because a feature that no job names is code
//! no job compiles: `#[cfg(feature = "...")]` bodies vanish from every build
//! that leaves the feature off, so both the lint and the assertions behind
//! them go quiet rather than red. Over there the first
//! `cargo clippy --features packfile` ever run came back red on a lint nobody
//! had ever compiled.
//!
//! This repo had no equivalent. Its `feature-cells` matrix in `ci.yml` is a
//! bare literal list, and nothing tied that list to the `[features]` table in
//! `Cargo.toml`, so a feature could be added to the manifest, gate a whole
//! test file, and be lint-checked and executed by nothing at all. That is not
//! hypothetical here either: `packfile` gated 14 tests in
//! `tests/phase3_packfile.rs` and `tests/builder_sink_packfile.rs` that no job
//! had ever executed, and the first run of them found one red (#191).
//!
//! For the record, the matrix was NOT stale against the manifest when this
//! landed. The point is that nothing was watching, and the way this repo finds
//! out is supposed to be a red test rather than someone reading two files side
//! by side.
//!
//! # Why a table rather than a rule
//!
//! No honest rule derives the right cells from a feature name. `pdfium` cannot
//! sit in the clippy matrix the way the others do, because its cells need a
//! native PDFium install and it gets a whole job of its own. `test-util` is on
//! by default, so every invocation already compiles it. `s3` is a deprecated
//! alias with no code behind it. `ported_tests` and `jxl` are run by
//! `tools/run_ported_cells.sh` rather than by a `- run: cargo test` line. So
//! each feature carries a row with its reason, and the question the guard
//! really asks is the one a rule cannot: does `[features]` contain exactly the
//! names this table covers?
//!
//! # What this guard does not do
//!
//! It says nothing about the local pre-commit mirror. That comparison already
//! exists and is sharper than anything this file could add:
//! `tests/install_hooks_mirror_ci.rs` runs the generated hook with a recording
//! `cargo` and diffs the invocations against the workflow, for this repo, the
//! core and the cli (libviprs/libviprs#715).
//!
//! `ci.yml` is resolved through `common::workflows::read_workflow` rather than
//! `include_str!`, because this repo has moved its workflows between
//! directories twice and that helper is where the guards refuse to guess which
//! copy they are reading (#137).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod common;
use common::workflows::read_workflow;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Which cells a feature needs, and why.
struct Coverage {
    /// The `feature-cells` matrix carries a row for it, so
    /// `cargo clippy --all-targets --features F -- -D warnings` compiles and
    /// lints every target under the feature.
    matrix: bool,
    /// The job whose `cargo test` runs with the feature enabled, if any.
    test: Option<&'static str>,
    /// `tools/run_ported_cells.sh` runs a `cargo test` with the feature
    /// enabled, and the `ported-tests` job runs that script.
    ported: bool,
    /// Why the three above are what they are. Read on failure.
    why: &'static str,
}

const fn cov(
    matrix: bool,
    test: Option<&'static str>,
    ported: bool,
    why: &'static str,
) -> Coverage {
    Coverage {
        matrix,
        test,
        ported,
        why,
    }
}

/// Every feature this crate declares, and the CI cells it needs.
const EXPECTED: &[(&str, Coverage)] = &[
    (
        "pdfium",
        cov(
            false,
            Some("test-pdfium"),
            false,
            "it has a job to itself rather than a matrix row: the cells need a \
             checksum-verified libpdfium installed first, and that job's \
             `cargo test --features pdfium` compiles and runs every gated \
             target, which is more than a clippy cell would do",
        ),
    ),
    (
        "ported_tests",
        cov(
            false,
            Some("test"),
            true,
            "the ported cells are run and linted through \
             tools/run_ported_cells.sh, which is the single source of truth \
             for the green subset, because the deferred codec cells do not \
             compile yet and `--all-targets` cannot carry the feature \
             (issue #77). The `test` job's phase3_tracing line enables it too",
        ),
    ),
    (
        "test-util",
        cov(
            false,
            None,
            false,
            "on by default, because this crate IS the external stress suite \
             the core's test-util feature exists to serve (libviprs#299). \
             Every cargo invocation here already compiles it, so a cell of \
             its own would run the same suite twice",
        ),
    ),
    (
        "object-store-sink",
        cov(
            true,
            Some("test"),
            false,
            "gates tests/phase3_object_store_sink.rs, \
             tests/sink_object_store_stub_contract.rs and the S3 leg of \
             tests/phase3_validation_stress.rs, none of which the default run \
             compiles (#382)",
        ),
    ),
    (
        "s3",
        cov(
            false,
            None,
            false,
            "a deprecated alias for object-store-sink with no code of its \
             own, so a cell would compile the same thing twice under a second \
             name. What keeps the alias honest is the manifest line, asserted \
             by `the_s3_alias_still_resolves_to_object_store_sink` below, and \
             the rename documentation guards in \
             tests/feature_rename_docs_present.rs",
        ),
    ),
    (
        "packfile",
        cov(
            true,
            Some("test"),
            true,
            "gates all 9 tests in tests/phase3_packfile.rs and all 5 in \
             tests/builder_sink_packfile.rs, plus the test_dz_zip ZipSink cell \
             in ported_foreign. It had the matrix row and no run cell for its \
             whole life, so 14 tests never executed anywhere and one of them \
             was red (#191)",
        ),
    ),
    (
        "tracing",
        cov(
            true,
            Some("test"),
            true,
            "gates the per-tile span tests in tests/phase3_tracing.rs, which \
             the default run compiles out (#83)",
        ),
    ),
    (
        "jxl",
        cov(
            true,
            None,
            true,
            "decides which contract test_jxlsave in ported_foreign pins, the \
             real JPEG XL round trip or the typed refusal, so the run cell is \
             the one run_ported_cells.sh makes under \
             `--features 'ported_tests jxl'` (libviprs#500)",
        ),
    ),
];

/// The feature names declared in `Cargo.toml`'s `[features]` table, in
/// declaration order, with `default` dropped.
fn declared_features() -> Vec<String> {
    let manifest = read("Cargo.toml");
    let mut out = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            inside = trimmed == "[features]";
            continue;
        }
        if !inside || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, _)) = trimmed.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name != "default" && !name.is_empty() {
            out.push(name.to_string());
        }
    }
    out
}

/// The features listed in the `feature-cells` matrix.
fn matrix_features(ci: &str) -> BTreeSet<String> {
    ci.lines()
        .filter_map(|line| {
            let line = line.trim();
            let list = line.strip_prefix("feature: [")?.strip_suffix(']')?;
            Some(list.split(',').map(|f| f.trim().to_string()))
        })
        .flatten()
        .collect()
}

/// The `- run:` command lines inside one two-space-indented job block.
///
/// Scoped to the job rather than searched over the whole file, because "the
/// feature is named somewhere in CI" is the question that passes while a
/// feature is only linted and never run.
fn run_lines_of_job(ci: &str, job: &str) -> Vec<String> {
    let header = format!("  {job}:");
    let mut out = Vec::new();
    let mut inside = false;
    for line in ci.lines() {
        if line == header {
            inside = true;
            continue;
        }
        if inside
            && line.starts_with("  ")
            && !line.starts_with("   ")
            && line.trim_end().ends_with(':')
        {
            break;
        }
        if inside && let Some(cmd) = line.trim().strip_prefix("- run: ") {
            out.push(cmd.trim().to_string());
        }
    }
    out
}

/// Split a command line into arguments, honouring the double quotes that
/// `--features "ported_tests tracing"` needs.
fn tokens(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut started = false;
    for ch in cmd.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// What a `cargo test` line enables, and whether it runs what it compiles.
struct TestCell {
    features: BTreeSet<String>,
    runs_tests: bool,
}

/// Read a command line as a `cargo test` cell, or `None` if it is not one.
///
/// `runs_tests` is false for the two ways a cell can name a feature and assert
/// nothing: `--no-run` compiles and runs zero tests, and a `--` separator
/// hands the harness a filter that can select none. The core hit exactly that
/// (libviprs#949), where adding `--no-run` to a cell left its whole coverage
/// guard green over a job that ran nothing.
fn test_cell(cmd: &str) -> Option<TestCell> {
    let toks = tokens(cmd);
    if toks.first().map(String::as_str) != Some("cargo")
        || toks.get(1).map(String::as_str) != Some("test")
    {
        return None;
    }
    let mut features = BTreeSet::new();
    let mut runs_tests = true;
    let mut rest = toks.iter().skip(2);
    while let Some(tok) = rest.next() {
        if tok == "--features" {
            if let Some(list) = rest.next() {
                features.extend(list.split_whitespace().map(str::to_string));
            }
        } else if let Some(list) = tok.strip_prefix("--features=") {
            features.extend(list.split_whitespace().map(str::to_string));
        } else if tok == "--no-run" || tok == "--" {
            runs_tests = false;
        }
    }
    Some(TestCell {
        features,
        runs_tests,
    })
}

/// Whether `job` runs a `cargo test` that enables `feature` and actually runs
/// the tests it compiles.
fn job_tests_feature(ci: &str, job: &str, feature: &str) -> bool {
    run_lines_of_job(ci, job)
        .iter()
        .filter_map(|line| test_cell(line))
        .any(|cell| cell.runs_tests && cell.features.contains(feature))
}

/// Whether `tools/run_ported_cells.sh` runs a `cargo test` under `feature`.
fn ported_script_tests_feature(feature: &str) -> bool {
    read("tools/run_ported_cells.sh")
        .lines()
        .map(str::trim)
        .filter_map(test_cell)
        .any(|cell| cell.features.contains(feature))
}

/// The table covers exactly the declared feature set.
///
/// This is the assertion that makes the rest of the file survive a new
/// feature. Without it, a feature added to `Cargo.toml` and to no cell would
/// simply not be looked at, and every other test here would stay green while
/// the thing they exist to prevent happened again.
#[test]
fn every_declared_feature_has_a_row_saying_which_cells_run_it() {
    let declared = declared_features();
    assert!(
        declared.len() >= 6,
        "the [features] parser found only {declared:?}, which cannot be right; \
         it has to fail loudly rather than pass vacuously"
    );
    assert!(
        declared.iter().any(|f| f == "packfile") && declared.iter().any(|f| f == "pdfium"),
        "positive control: the parser must find the features that are \
         definitely there, got {declared:?}"
    );

    for name in &declared {
        assert!(
            EXPECTED.iter().any(|(n, _)| n == name),
            "Cargo.toml declares the feature `{name}` and this table says \
             nothing about it. Add a row to EXPECTED naming which cells must \
             run it, and why. If it gates any `#[cfg(feature = \"{name}\")]` \
             code at all, the answer is at least a `feature-cells` matrix row: \
             without one, nothing in CI even type-checks those bodies."
        );
    }
    for (name, _) in EXPECTED {
        assert!(
            declared.iter().any(|f| f == name),
            "this table has a row for `{name}`, which Cargo.toml no longer \
             declares. Drop the row and the CI cells with it."
        );
    }
}

/// The `feature-cells` matrix is exactly the features the table says it lints.
///
/// Asserted as set equality in both directions. A feature in the manifest that
/// the matrix skips is the original hole, and a matrix row for a feature the
/// table does not claim means CI is spending a build on a name nobody owns,
/// which is how a stale row survives a feature rename.
#[test]
fn the_feature_cells_matrix_is_exactly_the_features_the_table_lints() {
    let ci = read_workflow("ci.yml");
    let listed = matrix_features(&ci);
    assert!(
        !listed.is_empty(),
        "found no `feature: [...]` list in ci.yml, so this guard would pass \
         over a workflow that lints no feature at all"
    );

    let claimed: BTreeSet<String> = EXPECTED
        .iter()
        .filter(|(_, c)| c.matrix)
        .map(|(name, _)| name.to_string())
        .collect();

    assert_eq!(
        listed,
        claimed,
        "the feature-cells matrix in ci.yml and this table disagree. Missing \
         from the matrix: {:?}. In the matrix with no row claiming it: {:?}. \
         Move both in the same change.",
        claimed.difference(&listed).collect::<Vec<_>>(),
        listed.difference(&claimed).collect::<Vec<_>>(),
    );

    // The matrix is worth nothing if the job stopped linting with it, so pin
    // the command that consumes it too.
    let lints = run_lines_of_job(&ci, "feature-cells");
    assert!(
        lints.iter().any(|cmd| {
            cmd.starts_with("cargo clippy --all-targets --features ${{ matrix.feature }}")
                && cmd.contains("-D warnings")
        }),
        "the feature-cells job no longer runs `cargo clippy --all-targets \
         --features ${{{{ matrix.feature }}}} -- -D warnings`, so the matrix \
         above expands into nothing: {lints:?}"
    );
}

/// Every run cell the table claims is really in `ci.yml`, in the right job, or
/// in the ported-cells script that `ci.yml` runs.
#[test]
fn ci_runs_every_cell_the_table_claims() {
    let ci = read_workflow("ci.yml");

    // The ported rows lean on this one line, so it is checked once here
    // rather than assumed eight times below.
    let ported_job = run_lines_of_job(&ci, "ported-tests");
    assert!(
        ported_job
            .iter()
            .any(|cmd| cmd.starts_with("./tools/run_ported_cells.sh") && !cmd.contains("--clippy")),
        "the ported-tests job no longer runs tools/run_ported_cells.sh for \
         real (a --clippy pass alone lints without running), so every `ported` \
         row in the table below claims coverage that does not exist: \
         {ported_job:?}"
    );

    let mut missing: Vec<String> = Vec::new();
    for (name, c) in EXPECTED {
        if let Some(job) = c.test
            && !job_tests_feature(&ci, job, name)
        {
            missing.push(format!(
                "the `{job}` job does not run a `cargo test` that enables \
                 `{name}` and runs what it compiles ({why})",
                why = c.why
            ));
        }
        if c.ported && !ported_script_tests_feature(name) {
            missing.push(format!(
                "tools/run_ported_cells.sh does not run a `cargo test` under \
                 `{name}` ({why})",
                why = c.why
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "CI is missing cells:\n  {}",
        missing.join("\n  ")
    );
}

/// The scanners find what is there, miss what is not, and stay inside the job
/// they were asked about.
///
/// The positive control matters more than usual here, because
/// `ci_runs_every_cell_the_table_claims` reports success by finding nothing: a
/// `run_lines_of_job` that returned an empty list for a mistyped job name
/// would make it pass while checking nothing at all.
#[test]
fn the_workflow_scanner_would_notice_a_missing_cell() {
    let ci = read_workflow("ci.yml");
    for job in ["test", "test-pdfium", "feature-cells", "ported-tests"] {
        assert!(
            !run_lines_of_job(&ci, job).is_empty(),
            "job `{job}` should carry `- run:` steps, found none"
        );
    }
    assert!(
        run_lines_of_job(&ci, "test").len() >= 4,
        "the `test` job should carry the default run plus the feature-gated \
         ones, found {:?}",
        run_lines_of_job(&ci, "test")
    );

    // A feature that is run.
    assert!(job_tests_feature(&ci, "test", "packfile"));
    // A feature that is not.
    assert!(!job_tests_feature(&ci, "test", "definitely-not-a-feature"));
    // A feature that is run, in a different job. This is the half that "named
    // somewhere in CI" gets wrong: pdfium has a job of its own and no cell in
    // the `test` job.
    assert!(job_tests_feature(&ci, "test-pdfium", "pdfium"));
    assert!(!job_tests_feature(&ci, "test", "pdfium"));
    // And the ported script really is read, rather than answering yes to
    // everything.
    assert!(ported_script_tests_feature("jxl"));
    assert!(!ported_script_tests_feature("definitely-not-a-feature"));
    // A job name nobody has is empty rather than quietly inheriting another
    // job's lines.
    assert!(run_lines_of_job(&ci, "no-such-job").is_empty());
}

/// A `- run:` line only satisfies a row when it runs what the row says.
#[test]
fn a_cell_that_names_a_feature_and_runs_nothing_does_not_count() {
    let cases: [(&str, &str, bool); 8] = [
        ("cargo test --features packfile", "packfile", true),
        (
            "cargo test --features packfile --test phase3_packfile --test builder_sink_packfile",
            "packfile",
            true,
        ),
        // The two ways a cell can be present and run nothing.
        ("cargo test --features packfile --no-run", "packfile", false),
        (
            "cargo test --features packfile -- some::filter",
            "packfile",
            false,
        ),
        // A feature name is not a prefix of a longer one, in either direction.
        ("cargo test --features packfiles", "packfile", false),
        ("cargo test --features pack", "packfile", false),
        // The quoted multi-feature spelling this repo uses.
        (
            "cargo test --features \"ported_tests tracing\" --test phase3_tracing",
            "tracing",
            true,
        ),
        // Not a cargo test line at all.
        (
            "cargo clippy --all-targets --features packfile -- -D warnings",
            "packfile",
            false,
        ),
    ];

    let mut wrong = Vec::new();
    for (line, feature, want) in cases {
        let got = test_cell(line).is_some_and(|c| c.runs_tests && c.features.contains(feature));
        if got != want {
            wrong.push(format!("{line:?} for `{feature}`: got {got}, want {want}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} cell-reading rows are wrong:\n  {}",
        wrong.len(),
        cases.len(),
        wrong.join("\n  ")
    );
}

/// `s3` is the one row claiming a feature needs no cell because it has no code,
/// so the manifest line that makes that true is asserted rather than trusted.
#[test]
fn the_s3_alias_still_resolves_to_object_store_sink() {
    let manifest = read("Cargo.toml");
    let line = manifest
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("s3 ="))
        .expect("Cargo.toml must still declare the deprecated `s3` alias");
    assert!(
        line.contains("object-store-sink"),
        "`s3` has stopped being a pure alias for object-store-sink ({line:?}), \
         so it now gates something of its own and needs cells and a row of its \
         own in EXPECTED"
    );
}
