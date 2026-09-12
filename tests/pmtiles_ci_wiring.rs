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
use common::workflows::read_workflow;

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

/// The download has to be verified, and the thing inside it too.
///
/// The pdfium block this is modelled on verifies the tarball and stops there. A
/// tarball digest says the bytes arrived intact; it does not say the binary
/// inside is the one that wrote the goldens. Both digests exist, measured, in
/// `tests/fixtures/pmtiles/PROVENANCE.md`, so both get checked.
#[test]
fn ci_verifies_the_go_pmtiles_download_before_running_it() {
    let ci = read_workflow("ci.yml");
    assert!(
        ci.contains("GO_PMTILES_TARBALL_SHA256"),
        "the interop job must pin the tarball digest"
    );
    assert!(
        ci.contains("GO_PMTILES_BINARY_SHA256"),
        "the interop job must pin the digest of the binary inside the tarball, \
         not only of the tarball"
    );
    let checks = ci.matches("sha256sum -c").count();
    assert!(
        checks >= 3,
        "ci.yml has {checks} `sha256sum -c` calls; pdfium accounts for one and \
         go-pmtiles needs two (tarball and binary), so something has stopped \
         being verified"
    );
}

/// The interop job must be unable to report green without running the binary.
///
/// This is the whole reason the job exists as its own cell rather than as a
/// line in the default `test` job: `cargo test` reads a skip as a pass, so the
/// skip in `tests/pmtiles_interop.rs` needs somewhere that refuses to take it.
#[test]
fn the_interop_job_cannot_skip_to_a_false_green() {
    let ci = read_workflow("ci.yml");
    assert!(
        ci.contains("VIPRS_REQUIRE_GO_PMTILES: 1")
            || ci.contains("VIPRS_REQUIRE_GO_PMTILES: \"1\""),
        "the interop job must set VIPRS_REQUIRE_GO_PMTILES=1 so a runner that \
         failed to lay go-pmtiles down hard-fails instead of skipping every \
         comparison and reporting a pass"
    );
    assert!(
        ci.contains("--test pmtiles_interop"),
        "the interop job must actually run tests/pmtiles_interop.rs; the env var \
         above guards a binary that is not being invoked otherwise"
    );
}

/// Every `tests/pmtiles_*.rs` binary has to be named in the interop job.
///
/// The same shape as `cli_differential_job_runs_every_diff_binary`, and for the
/// same reason: a future PMTiles cell that lands as a new file and is never
/// wired in runs nowhere and says nothing. The default `test` job's bare
/// `cargo test` does pick these up, so this is not about them running at all,
/// it is about them running in the one job that has the external binary.
#[test]
fn the_interop_job_runs_every_pmtiles_binary() {
    let ci = read_workflow("ci.yml");
    let mut binaries: Vec<String> = std::fs::read_dir(repo_root().join("tests"))
        .expect("read tests/ dir")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("pmtiles_") && n.ends_with(".rs"))
        .map(|n| n.trim_end_matches(".rs").to_string())
        .collect();
    binaries.sort();
    assert!(
        !binaries.is_empty(),
        "expected at least one tests/pmtiles_*.rs binary; if they have all been \
         renamed this guard is pinning an empty set and will pass forever"
    );
    for name in &binaries {
        assert!(
            ci.contains(&format!("--test {name}")),
            "the pmtiles-interop job in ci.yml must run `cargo test --test \
             {name}` (found tests/{name}.rs but no matching --test line)"
        );
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
        "vectors/tiles.json",
        "vectors/header.json",
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
