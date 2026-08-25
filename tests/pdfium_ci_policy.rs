//! Guards the pdfium test-execution policy from issue #59.
//!
//! Two invariants used to be entangled with the unpropagated pdfium-render
//! fork and with shared-runner timing noise:
//!
//! 1. The pdfium suites were pinned to `--test-threads=1` because upstream
//!    `ThreadSafePdfiumBindings` bypassed the pdfium global mutex on every
//!    call but `FPDF_InitLibrary`, so parallel cargo-test workers raced FPDF
//!    state. The `libviprs/integration` fork now locks per call (a direct dep
//!    of libviprs, mirrored here via `[patch.crates-io]`), so the suite is
//!    safe multi-threaded and must exercise that cross-test concurrency.
//! 2. A wall-clock perf-ratio smoke (streaming <= 12x cached) gated every PR
//!    on shared-runner timing, a flake vector. It now carries `#[ignore]` and
//!    runs in the nightly workflow instead.
//!
//! These tests fail loudly if either guarantee regresses. They read only
//! repository files, so they run under the default `cargo test` with no
//! network, no sibling checkout, and no feature flags.

use std::path::{Path, PathBuf};

mod common;
use common::workflows::read_workflow;

/// The serial-execution flag whose removal issue #59 pins. Assembled from
/// fragments so this guard file does not itself match a raw substring scan
/// for the flag in the workflow / Dockerfile.
fn serial_flag() -> String {
    format!("--test-threads{}1", "=")
}

/// Lines that actually invoke `cargo test` (the command surface), ignoring
/// prose comments that may legitimately name the dropped flag while
/// explaining why it is gone.
fn cargo_test_command_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            // Skip comment lines (YAML and Dockerfile both use `#`) so prose
            // that names the dropped flag does not count as a command.
            !trimmed.starts_with('#') && trimmed.contains("cargo test")
        })
        .collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The flag must be absent from every actual `cargo test` invocation in the
/// file (comments that explain why it is gone are allowed to name it).
fn assert_no_serial_flag_in_commands(file: &str, contents: &str) {
    let commands = cargo_test_command_lines(contents);
    let pdfium_cmds = commands
        .iter()
        .filter(|c| c.contains("--features pdfium"))
        .count();
    assert!(
        pdfium_cmds >= 1,
        "{file} must still invoke the pdfium suite via `cargo test --features pdfium`"
    );
    for cmd in &commands {
        assert!(
            !cmd.contains(&serial_flag()),
            "{file} must not pin a `cargo test` invocation to {}; the fork makes multi-threaded runs safe (issue #59). Offending line: {cmd:?}",
            serial_flag()
        );
    }
}

#[test]
fn ci_runs_the_pdfium_suite_multi_threaded() {
    let ci = read_workflow("ci.yml");
    assert_no_serial_flag_in_commands("ci.yml", &ci);
}

#[test]
fn dockerfile_runs_the_pdfium_suites_multi_threaded() {
    let dockerfile = read("Dockerfile");
    assert_no_serial_flag_in_commands("Dockerfile", &dockerfile);
}

#[test]
fn perf_ratio_smoke_is_ignored_out_of_normal_ci() {
    let src = read("tests/pdfium_streaming_perf_smoke.rs");
    // Count real attribute lines (trimmed line starts with the attribute),
    // not prose in the module doc that mentions `#[ignore]` / `#[test]`.
    let attr_lines = |attr: &str| {
        src.lines()
            .filter(|l| l.trim_start().starts_with(attr))
            .count()
    };
    let test_attrs = attr_lines("#[test]");
    let ignore_attrs = attr_lines("#[ignore");
    // Both wall-clock assertions must be behind `#[ignore]` so a normal
    // `cargo test` (and the per-PR CI job) never blocks on runner timing.
    assert_eq!(
        test_attrs, 2,
        "expected the two perf-smoke tests; found {test_attrs} (update this guard if the file changed)"
    );
    assert_eq!(
        ignore_attrs, test_attrs,
        "every perf-smoke #[test] must carry #[ignore]; found {ignore_attrs} ignore attrs for {test_attrs} tests (issue #59)"
    );
}

#[test]
fn nightly_workflow_runs_the_ignored_perf_smoke_on_a_schedule() {
    let nightly = read_workflow("nightly.yml");
    assert!(
        nightly.contains("schedule:") && nightly.contains("cron:"),
        "nightly.yml must run on a cron schedule"
    );
    assert!(
        nightly.contains("cargo test --features pdfium -- --ignored"),
        "nightly.yml must run the ignored (perf-ratio smoke) tests"
    );
}
