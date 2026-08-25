//! Locates the CI workflow definitions that this repo's pinning guards read.
//!
//! Several guard suites (`counterpart_pinning`, `cli_counterpart_pinning`,
//! `pdfium_ci_policy`, `pdfium_provenance`) pin the *contents* of the CI
//! workflows: they assert that CI still clones the counterparts at a pinned
//! rev, still runs every differential binary, still checksum-verifies PDFium,
//! and so on. Their whole value comes from reading the file CI actually runs.
//!
//! The Gitea Actions migration (#135) moved `ci.yml` and `nightly.yml` from
//! `.github/workflows/` to `.gitea/workflows/` and left the guards pointing at
//! the old path (#137); dropping Gitea (libviprs/libviprs#585) moved them back.
//! A guard that reads the wrong CI file has failed at its own purpose, and a
//! stale local checkout that still carries the removed file hides that: it goes
//! green locally and red in CI.
//!
//! So resolution lives here, once, and it refuses to guess. [`read_workflow`]
//! looks for a workflow by name in every directory this repo has kept
//! workflows in, and panics unless exactly one of them has it, naming all the
//! candidates either way, so the next migration diagnoses itself.

use std::path::{Path, PathBuf};

/// Every directory this repo has kept CI workflow definitions in, current
/// location first. Add to this list when the workflows move again; the guards
/// themselves need no change.
///
/// `.gitea/workflows` stays listed even though the directory is gone: a stale
/// checkout that still carries the removed copy then trips the "exists in 2
/// locations" panic below instead of silently pinning the wrong file.
pub const WORKFLOW_DIRS: &[&str] = &[".github/workflows", ".gitea/workflows"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Read the workflow named `name` (e.g. `"ci.yml"`), wherever it currently
/// lives.
///
/// Panics if no known location has it (the workflows moved and this list did
/// not follow), or if more than one does (the guards would pin whichever copy
/// they happened to read, which is exactly the silent failure this exists to
/// prevent).
pub fn read_workflow(name: &str) -> String {
    let root = repo_root();
    let found: Vec<PathBuf> = WORKFLOW_DIRS
        .iter()
        .map(|dir| root.join(dir).join(name))
        .filter(|path| path.is_file())
        .collect();

    match found.as_slice() {
        [path] => std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display())),
        [] => panic!(
            "no CI workflow named {name} in any known location ({}). If the \
             workflows moved again, add the new directory to WORKFLOW_DIRS in \
             tests/common/workflows.rs so the pinning guards follow them (#137).",
            candidate_list(name)
        ),
        several => panic!(
            "{name} exists in {} known locations at once ({}). The pinning \
             guards would silently pin whichever copy they happened to read, so \
             drop the stale one before the next migration hides behind it (#137).",
            several.len(),
            several
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn candidate_list(name: &str) -> String {
    WORKFLOW_DIRS
        .iter()
        .map(|dir| format!("{dir}/{name}"))
        .collect::<Vec<_>>()
        .join(", ")
}
