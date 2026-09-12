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

// ---------------------------------------------------------------------------
// Reading the workflow as structure rather than as text
// ---------------------------------------------------------------------------

/// A workflow read as jobs and steps rather than as one long string.
///
/// Every guard in this repo used to ask whether some substring appeared
/// *anywhere* in `ci.yml`. That is a spelling check wearing a guard's clothes,
/// and seven separate one-line edits pass it while changing what CI does:
/// disabling a job with `if: false`, letting a step fail with
/// `continue-on-error: true`, commenting out an `env:` block so the string
/// survives in a comment, deleting `set -euo pipefail` from an install script,
/// appending `|| true` to a `sha256sum -c`, moving a `--test` flag to a job
/// that has no external binary, and setting the require-variable on the wrong
/// job. All seven are valid YAML that GitHub honours.
///
/// So the guards read structure. This is not a YAML parser and does not want to
/// be one: it understands the block style these workflows are written in and
/// [refuses](Workflow::parse) anything else rather than quietly reporting that a
/// job it could not parse has no steps, which is the failure that would put us
/// back where we started.
#[derive(Debug, Clone)]
pub struct Workflow {
    /// The raw lines of the `on:` block, so a guard can say what the workflow
    /// triggers on.
    ///
    /// Nothing read this at first, and `branches: ['no-such-branch']` then
    /// stopped `ci.yml` running on any push while every guard stayed green.
    /// A job that never runs reports no failures.
    on: Vec<String>,
    jobs: Vec<Job>,
}

/// One entry under `jobs:`.
#[derive(Debug, Clone)]
pub struct Job {
    /// The mapping key, e.g. `pmtiles-interop`.
    pub id: String,
    /// Scalar keys directly on the job: `name`, `runs-on`, `if`, and so on.
    pub keys: Vec<(String, String)>,
    /// The job's own `env:` mapping, which GitHub gives to every step in it.
    ///
    /// Read rather than skipped because a guard that only looks at the step
    /// reds when somebody hoists a variable to the job, which is a refactor
    /// that changes nothing about what CI does. A guard people route around is
    /// worse than one that is slightly looser.
    pub env: Vec<(String, String)>,
    /// The job's `steps:` in order.
    pub steps: Vec<Step>,
}

/// One entry under a job's `steps:`.
#[derive(Debug, Clone)]
pub struct Step {
    /// Scalar keys on the step: `name`, `uses`, `run`, `continue-on-error`, ...
    pub keys: Vec<(String, String)>,
    /// The step's own `env:` mapping. Only this step's, never the job's and
    /// never another step's, which is the whole point of reading structure.
    pub env: Vec<(String, String)>,
}

impl Job {
    /// The value of a scalar key on the job, if it has one.
    pub fn key(&self, name: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// What this step sees for an environment variable: its own setting if it
    /// has one, otherwise the job's.
    pub fn env_for<'a>(&'a self, step: &'a Step, name: &str) -> Option<&'a str> {
        step.env_value(name).or_else(|| {
            self.env
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        })
    }
}

impl Step {
    /// The value of a scalar key on the step, if it has one.
    pub fn key(&self, name: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// The step's `run:` script, or the empty string for a `uses:` step.
    pub fn run(&self) -> &str {
        self.key("run").unwrap_or("")
    }

    /// The step's script with comment-only lines removed and line
    /// continuations folded, which is closer to what the shell runs.
    ///
    /// Both halves are measured holes rather than tidiness. Asking whether a
    /// script `contains("set -euo pipefail")` is satisfied by
    /// `# set -euo pipefail`, and asking per line whether a `sha256sum -c`
    /// carries `|| true` misses the `|| true` that sits on the next line after
    /// a trailing backslash, which is one command as far as the shell is
    /// concerned.
    pub fn run_code(&self) -> String {
        let mut out = String::new();
        let mut pending = String::new();
        for line in self.run().lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let body = line.strip_suffix('\\').unwrap_or(line);
            pending.push_str(body);
            if line.ends_with('\\') {
                continue;
            }
            out.push_str(pending.trim_end());
            out.push('\n');
            pending.clear();
        }
        if !pending.is_empty() {
            out.push_str(pending.trim_end());
            out.push('\n');
        }
        out
    }

    /// The value this step sets for an environment variable, if it sets one.
    pub fn env_value(&self, name: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

impl Workflow {
    /// Every job, in file order.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// The `on:` block, one entry per line, indentation stripped.
    pub fn on(&self) -> &[String] {
        &self.on
    }

    /// The job with this id.
    ///
    /// Panics when it is absent, naming the jobs that do exist. A guard that
    /// silently found no job is a guard that passes.
    pub fn job(&self, id: &str) -> &Job {
        self.jobs.iter().find(|j| j.id == id).unwrap_or_else(|| {
            panic!(
                "no job `{id}` in the workflow. It has: {}. A renamed job \
                 leaves every guard about it asserting nothing, so this is a \
                 failure rather than a skip.",
                self.jobs
                    .iter()
                    .map(|j| j.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
    }

    /// Every step of every job, paired with the job it belongs to.
    pub fn steps(&self) -> impl Iterator<Item = (&Job, &Step)> {
        self.jobs
            .iter()
            .flat_map(|j| j.steps.iter().map(move |s| (j, s)))
    }

    /// Parse the block-style workflow in `text`.
    ///
    /// Handles exactly what these files use: a `jobs:` mapping, two-space
    /// indent steps, `run: |` and `run: >` block scalars, and `env:` mappings
    /// on jobs and steps. Anything else is a panic rather than a guess, because
    /// the alternative is a guard reporting that the construct it could not
    /// read is fine.
    pub fn parse(text: &str) -> Self {
        assert!(
            !text.contains('\t'),
            "the workflow contains a tab, which YAML does not allow as indentation \
             and this reader does not try to interpret"
        );
        assert!(
            !text.contains("\n---"),
            "the workflow has more than one document. This reader assumes one, \
             and would silently read only the first"
        );

        let lines: Vec<&str> = text.lines().collect();
        let mut jobs: Vec<Job> = Vec::new();

        let Some(jobs_at) = lines.iter().position(|l| l.trim_end() == "jobs:") else {
            panic!("the workflow has no `jobs:` key at column 0");
        };

        // `on:` is a bare key at column 0, and its block runs until the next
        // one. YAML 1.1 reads an unquoted `on` as the boolean true, which is
        // why some workflows spell it `"on":`, so both are accepted here.
        let on_at = lines
            .iter()
            .position(|l| matches!(l.trim_end(), "on:" | "\"on\":" | "'on':" | "true:"))
            .unwrap_or_else(|| panic!("the workflow has no `on:` key at column 0"));
        let mut on: Vec<String> = Vec::new();
        for line in &lines[on_at + 1..] {
            if skippable(line) {
                continue;
            }
            if indent_of(line) == 0 {
                break;
            }
            on.push(line.trim().to_string());
        }
        assert!(
            !on.is_empty(),
            "the workflow's `on:` block is empty, so nothing triggers it"
        );

        let mut i = jobs_at + 1;
        while i < lines.len() {
            let line = lines[i];
            if skippable(line) {
                i += 1;
                continue;
            }
            let indent = indent_of(line);
            // Column 0 ends the `jobs:` mapping.
            if indent == 0 {
                break;
            }
            assert_eq!(
                indent,
                2,
                "expected a job id at indent 2, got {indent} on line {}: {line:?}",
                i + 1
            );
            let id = mapping_key(line, i).0.to_string();
            let (job, next) = parse_job(&lines, i + 1, id);
            jobs.push(job);
            i = next;
        }

        assert!(!jobs.is_empty(), "the workflow declares no jobs");
        Self { on, jobs }
    }
}

/// Blank lines and whole-line comments carry nothing this reader needs.
///
/// A commented-out `env:` block is the Y3 mutation, and the reason the guards
/// have to read structure at all: as text the variable is still there.
fn skippable(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || t.starts_with('#')
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Split `key: value` at the first colon that ends the key.
///
/// Returns the key and the value with any trailing comment and surrounding
/// quotes removed. `line_no` is only for the panic.
///
/// The **key** is unquoted too, which is not decoration. YAML lets a key be
/// quoted and GitHub honours it, so `"if": false` on a job and
/// `'continue-on-error': true` on a step do exactly what the bare spellings do.
/// Measured: with the key left quoted, both are invisible to every guard here.
fn mapping_key(line: &str, line_no: usize) -> (&str, String) {
    let body = line.trim_start();
    let body = body.strip_prefix("- ").unwrap_or(body);
    let colon = body.find(':').unwrap_or_else(|| {
        panic!(
            "expected `key: value` on line {}, got {line:?}",
            line_no + 1
        )
    });
    let key = unquote(body[..colon].trim());
    let value = body[colon + 1..].trim();
    (key, scalar(value))
}

/// A scalar with its trailing comment and quotes taken off.
///
/// Only strips a `#` that follows whitespace, so a digest or a URL fragment
/// survives.
/// Strip one matching pair of surrounding quotes, if there is one.
fn unquote(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
        .unwrap_or(text)
}

fn scalar(value: &str) -> String {
    let mut end = value.len();
    let bytes = value.as_bytes();
    let mut quote: Option<u8> = None;
    for (idx, b) in bytes.iter().enumerate() {
        match quote {
            // Inside a quoted run nothing is a comment, which matters because
            // `run: echo "tag # 1"` would otherwise be truncated to `echo "tag`
            // and a `contains` asking whether the line is safe would be asking
            // about text that is not what runs.
            Some(q) if *b == q => quote = None,
            Some(_) => {}
            None if *b == b'"' || *b == b'\'' => quote = Some(*b),
            None if *b == b'#' && idx > 0 && bytes[idx - 1].is_ascii_whitespace() => {
                end = idx;
                break;
            }
            None => {}
        }
    }
    unquote(value[..end].trim()).to_string()
}

/// Read one job's body, starting at `from`. Returns it and the line after it.
fn parse_job(lines: &[&str], from: usize, id: String) -> (Job, usize) {
    let mut job = Job {
        id,
        keys: Vec::new(),
        env: Vec::new(),
        steps: Vec::new(),
    };
    let mut i = from;

    while i < lines.len() {
        let line = lines[i];
        if skippable(line) {
            i += 1;
            continue;
        }
        if indent_of(line) <= 2 {
            break;
        }
        assert_eq!(
            indent_of(line),
            4,
            "expected a job key at indent 4 in job `{}`, got {} on line {}: {line:?}",
            job.id,
            indent_of(line),
            i + 1
        );
        let (key, value) = mapping_key(line, i);
        if key == "steps" {
            let (steps, next) = parse_steps(lines, i + 1, &job.id);
            job.steps = steps;
            i = next;
            continue;
        }
        if key == "env" {
            let (env, next) = parse_mapping(lines, i + 1, 6, &job.id);
            job.env = env;
            i = next;
            continue;
        }
        if value.is_empty() {
            // Another nested mapping on the job (`strategy:`, `outputs:`).
            // Its own keys are not read here; a guard that needs one asks for
            // the step it lives on instead.
            let (_, next) = skip_block(lines, i + 1, 4);
            job.keys.push((key.to_string(), String::new()));
            i = next;
            continue;
        }
        job.keys.push((key.to_string(), value));
        i += 1;
    }

    (job, i)
}

/// Read a `steps:` sequence. Items are `- ` at indent 6, keys at indent 8.
fn parse_steps(lines: &[&str], from: usize, job_id: &str) -> (Vec<Step>, usize) {
    let mut steps: Vec<Step> = Vec::new();
    let mut i = from;

    while i < lines.len() {
        let line = lines[i];
        if skippable(line) {
            i += 1;
            continue;
        }
        let indent = indent_of(line);
        if indent <= 4 {
            break;
        }
        assert!(
            line.trim_start().starts_with("- "),
            "expected a step to start with `- ` in job `{job_id}`, got line {}: \
             {line:?}",
            i + 1
        );
        assert_eq!(
            indent,
            6,
            "expected a step at indent 6 in job `{job_id}`, got {indent} on line \
             {}: {line:?}. These workflows are hand-formatted and this reader \
             models only that shape; a formatter that puts the sequence at the \
             same indent as its `steps:` key is legal YAML and lands here.",
            i + 1
        );
        let (step, next) = parse_step(lines, i, job_id);
        steps.push(step);
        i = next;
    }

    (steps, i)
}

/// Read one step. `at` is its `- ` line, whose own `key: value` counts.
fn parse_step(lines: &[&str], at: usize, job_id: &str) -> (Step, usize) {
    let mut step = Step {
        keys: Vec::new(),
        env: Vec::new(),
    };
    let mut i = at;
    let mut first = true;

    while i < lines.len() {
        let line = lines[i];
        if skippable(line) {
            i += 1;
            continue;
        }
        let indent = indent_of(line);
        if !first {
            // A new `- ` at indent 6 is the next step; anything shallower ends
            // the sequence.
            if indent <= 6 {
                break;
            }
            assert_eq!(
                indent,
                8,
                "expected a step key at indent 8 in job `{job_id}`, got {indent} \
                 on line {}: {line:?}",
                i + 1
            );
        }
        first = false;

        let (key, value) = mapping_key(line, i);
        if key == "env" {
            let (env, next) = parse_mapping(lines, i + 1, 10, job_id);
            step.env = env;
            i = next;
            continue;
        }
        if value == "|" || value == ">" || value == "|-" || value == ">-" {
            let (block, next) = read_block_scalar(lines, i + 1);
            step.keys.push((key.to_string(), block));
            i = next;
            continue;
        }
        if value.is_empty() {
            // `with:` and friends. Not read as scalars, skipped as a block.
            let (_, next) = skip_block(lines, i + 1, 8);
            step.keys.push((key.to_string(), String::new()));
            i = next;
            continue;
        }
        assert!(
            !step.keys.iter().any(|(k, _)| k == key),
            "`{key}` appears twice on one step in job `{job_id}`, on line {}. \
             YAML takes the last and this reader would hand back the first.",
            i + 1
        );
        step.keys.push((key.to_string(), value));
        i += 1;
    }

    (step, i)
}

/// Read a mapping whose keys sit at `indent`, e.g. an `env:` block.
///
/// Used for a step's `env:` at indent 10 and a job's at indent 6.
fn parse_mapping(
    lines: &[&str],
    from: usize,
    indent: usize,
    job_id: &str,
) -> (Vec<(String, String)>, usize) {
    let mut mapping: Vec<(String, String)> = Vec::new();
    let mut i = from;
    while i < lines.len() {
        let line = lines[i];
        if skippable(line) {
            i += 1;
            continue;
        }
        if indent_of(line) < indent {
            break;
        }
        assert_eq!(
            indent_of(line),
            indent,
            "expected a mapping key at indent {indent} in job `{job_id}`, got {} \
             on line {}: {line:?}",
            indent_of(line),
            i + 1
        );
        let (key, value) = mapping_key(line, i);
        assert!(
            !mapping.iter().any(|(k, _)| k == key),
            "`{key}` appears twice in one mapping in job `{job_id}`, on line {}. \
             YAML takes the last and this reader would hand back the first, so \
             it refuses rather than disagree with the thing that runs.",
            i + 1
        );
        mapping.push((key.to_string(), value));
        i += 1;
    }
    (mapping, i)
}

/// A `|` or `>` block scalar: every following line indented past the key.
fn read_block_scalar(lines: &[&str], from: usize) -> (String, usize) {
    let mut body = String::new();
    let mut i = from;
    let mut base: Option<usize> = None;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            body.push('\n');
            i += 1;
            continue;
        }
        let indent = indent_of(line);
        let base = *base.get_or_insert(indent);
        if indent < base {
            break;
        }
        body.push_str(&line[base..]);
        body.push('\n');
        i += 1;
    }
    (body, i)
}

/// Walk past a nested block whose keys this reader does not need.
fn skip_block(lines: &[&str], from: usize, parent_indent: usize) -> ((), usize) {
    let mut i = from;
    while i < lines.len() {
        let line = lines[i];
        if skippable(line) {
            i += 1;
            continue;
        }
        if indent_of(line) <= parent_indent {
            break;
        }
        i += 1;
    }
    ((), i)
}

/// The workflow CI runs, read as structure.
pub fn workflow(name: &str) -> Workflow {
    Workflow::parse(&read_workflow(name))
}
