//! Guards that the pre-commit hook runs what each repo's CI runs
//! (libviprs/libviprs#715, and libviprs-tests#198 for the org-wide sweep).
//!
//! `tools/install-hooks.sh` states a contract at the top of the file: the
//! pre-commit hook is the LINT HALF of a repo's CI, which is the jobs listed
//! under `mirror_jobs` for that repo, minus the steps `exempt_steps` names
//! with a reason; the pre-push hook is the TEST HALF; and `deferred_jobs`
//! accounts for everything in neither. This file is what makes that a contract
//! rather than a paragraph.
//!
//! Four things are enforced, and each one is a way the arrangement has already
//! gone wrong at least once:
//!
//! 1. Every job in a repo's workflow is either mirrored or deferred with a
//!    reason. A new CI job that is in neither fails here. Without this, CI can
//!    grow a whole job and the local gate says nothing, which is how the core's
//!    list came to run two of ten clippy passes.
//! 2. The hook runs every command in the jobs it mirrors. A missing one is a
//!    commit that passes locally and fails remotely.
//! 3. The hook runs nothing those jobs do not. An extra is a compile spent on
//!    something nobody asked for, or a check CI quietly dropped.
//! 4. Every exemption still names a step CI actually runs. A stale exemption is
//!    a hole with a reassuring comment over it.
//!
//! Nothing here greps either file. The contract is read by running
//! `tools/install-hooks.sh --describe`, and what the hook runs is read by
//! running the generated hook with recording stand-ins in front of every tool
//! it shells out to. Two earlier guards on these hooks asserted by substring
//! and stayed green while the behaviour they named had been deleted, which is
//! the whole of libviprs/libviprs#695.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

mod common;
use common::cli::{cli_dir, require_cli};
use common::hooks::{Workspace, make_executable, repo_root};

// ---------------------------------------------------------------------------
// Recording stand-ins
// ---------------------------------------------------------------------------

/// A tool that records its arguments and succeeds, so the hook runs every step
/// instead of stopping at the first one.
///
/// It reports the directory it was called in, relative to the repo, because a
/// CI step can carry a `working-directory:` and the hook mirrors that with a
/// `cd`. Recording the command without the directory would let a hook that
/// runs the right command in the wrong place read as correct.
fn recorder(tool: &str) -> String {
    format!(
        r#"#!/bin/sh
here=$(pwd)
case "$here" in
  "$LOCKSTEP_ROOT") printf '{tool} %s\n' "$*" >> "$LOCKSTEP_RECORD" ;;
  *) printf 'cd %s && {tool} %s\n' "${{here#"$LOCKSTEP_ROOT"/}}" "$*" >> "$LOCKSTEP_RECORD" ;;
esac
exit 0
"#
    )
}

/// `git` is the one tool the hook uses for two different reasons: `git diff
/// --exit-code` is a CI step, and everything else is plumbing the hook needs
/// to work at all. Recording the first and handing the rest to the real git
/// keeps the comparison exact without breaking the hook underneath it.
fn git_recorder(real_git: &str) -> String {
    format!(
        r#"#!/bin/sh
if [ "${{1-}}" = "diff" ]; then
  here=$(pwd)
  case "$here" in
    "$LOCKSTEP_ROOT") printf 'git %s\n' "$*" >> "$LOCKSTEP_RECORD" ;;
    *) printf 'cd %s && git %s\n' "${{here#"$LOCKSTEP_ROOT"/}}" "$*" >> "$LOCKSTEP_RECORD" ;;
  esac
  exit 0
fi
exec {real_git} "$@"
"#
    )
}

/// The same, for a script the hook reaches by a path relative to the repo
/// rather than through `PATH`. It reports `$0`, which is the path the hook
/// typed, so `./tools/x.sh` and `tools/x.sh` stay distinguishable: those are
/// different commands and CI writes one of them.
const SCRIPT_RECORDER: &str = r#"#!/bin/sh
here=$(pwd)
case "$here" in
  "$LOCKSTEP_ROOT") printf '%s %s\n' "$0" "$*" >> "$LOCKSTEP_RECORD" ;;
  *) printf 'cd %s && %s %s\n' "${here#"$LOCKSTEP_ROOT"/}" "$0" "$*" >> "$LOCKSTEP_RECORD" ;;
esac
exit 0
"#;

/// Tools the generated hooks reach through `PATH`.
const RECORDED_TOOLS: &[&str] = &["cargo", "node", "shellcheck", "ruff", "pytest"];

/// Scripts the generated hooks reach by a path relative to the repo, so a
/// stand-in has to sit at that path rather than on `PATH`.
const RECORDED_SCRIPTS: &[&str] = &["tools/run_ported_cells.sh", "cli/tools/sync-cli-src.sh"];

/// Where the real `git` is, resolved once so the recording `git` can hand the
/// plumbing calls on to it.
fn real_git() -> String {
    let out = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("these guards drive the real hooks, which need git on PATH");
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert!(
        !path.is_empty(),
        "git is not on PATH, and these guards drive hooks that use it"
    );
    path
}

// ---------------------------------------------------------------------------
// The contract, read out of the installer by running it
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct RepoContract {
    workflow: String,
    /// The subdirectory CI checks this repo out into, when it does. Every path
    /// in that job's commands carries it and no local path does.
    checkout_path: Option<String>,
    mirrored: Vec<String>,
    /// job -> why it is in neither half.
    deferred: BTreeMap<String, String>,
    /// step -> why the hook skips it.
    exempt: BTreeMap<String, String>,
    pre_push: bool,
}

/// Every repo the installer knows about, read by running it with `--describe`.
fn contract() -> BTreeMap<String, RepoContract> {
    let script = repo_root().join("tools/install-hooks.sh");
    let out = Command::new("bash")
        .arg(&script)
        .arg("--describe")
        .output()
        .unwrap_or_else(|e| panic!("run {} --describe: {e}", script.display()));
    assert!(
        out.status.success(),
        "tools/install-hooks.sh --describe exited {}:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).into_owned();

    let mut repos: BTreeMap<String, RepoContract> = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 2 {
            continue;
        }
        let entry = repos.entry(f[1].to_string()).or_default();
        match (f[0], f.len()) {
            ("repo", _) => {}
            ("workflow", 3) => entry.workflow = f[2].to_string(),
            ("checkout-path", 3) => entry.checkout_path = Some(f[2].to_string()),
            ("mirror", 3) => entry.mirrored.push(f[2].to_string()),
            ("prepush", 3) => entry.pre_push = f[2] == "yes",
            ("defer", 4) => {
                entry.deferred.insert(f[2].to_string(), f[3].to_string());
            }
            ("exempt", 4) => {
                entry.exempt.insert(f[2].to_string(), f[3].to_string());
            }
            _ => panic!(
                "tools/install-hooks.sh --describe printed a line this guard cannot read, so the contract and the guard have gone out of step:\n  {line}"
            ),
        }
    }
    assert!(
        !repos.is_empty(),
        "tools/install-hooks.sh --describe printed nothing, so every comparison \
         below would be vacuous"
    );
    repos
}

// ---------------------------------------------------------------------------
// Enough of a workflow parser to read steps out of jobs
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct Step {
    name: Option<String>,
    run: Option<String>,
    working_directory: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct Job {
    id: String,
    steps: Vec<Step>,
    /// The `feature: [...]` list, when the job has one.
    features: Vec<String>,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn structural(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty() && !t.starts_with('#')
}

/// The jobs of a workflow, in file order.
///
/// This is not a YAML parser and does not try to be. It reads the shape every
/// workflow in this org is written in: `jobs:` at column zero, one job header
/// per indent level below it, a `steps:` list, and `key: value` or `key: |`
/// inside each step. Anything it cannot read it refuses rather than skips,
/// because a parser that silently returns nothing turns every comparison below
/// into a vacuous pass.
fn parse_jobs(workflow: &str, whose: &str) -> Vec<Job> {
    let lines: Vec<&str> = workflow.lines().collect();
    let jobs_at = lines
        .iter()
        .position(|l| l.trim_end() == "jobs:")
        .unwrap_or_else(|| {
            panic!("{whose} has no `jobs:` block, so this guard would compare nothing")
        });

    // The jobs block runs to the end of the file or to the next column-zero key.
    let end = lines[jobs_at + 1..]
        .iter()
        .position(|l| structural(l) && indent_of(l) == 0)
        .map(|i| jobs_at + 1 + i)
        .unwrap_or(lines.len());
    let body = &lines[jobs_at + 1..end];

    // Job headers all sit at the first indent seen inside the block.
    let job_indent = body
        .iter()
        .find(|l| structural(l))
        .map(|l| indent_of(l))
        .unwrap_or_else(|| panic!("{whose} declares `jobs:` and then nothing"));

    let is_header = |l: &str| {
        structural(l) && indent_of(l) == job_indent && {
            let t = l.trim();
            t.ends_with(':')
                && t[..t.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                && !t[..t.len() - 1].is_empty()
        }
    };

    let starts: Vec<usize> = (0..body.len()).filter(|&i| is_header(body[i])).collect();
    assert!(
        !starts.is_empty(),
        "{whose} declares `jobs:` but this guard found no job headers under it"
    );

    let mut jobs = Vec::new();
    for (n, &start) in starts.iter().enumerate() {
        let stop = starts.get(n + 1).copied().unwrap_or(body.len());
        let id = body[start].trim().trim_end_matches(':').to_string();
        jobs.push(parse_job(id, &body[start + 1..stop], whose));
    }
    jobs
}

fn parse_job(id: String, body: &[&str], whose: &str) -> Job {
    let features = body
        .iter()
        .find_map(|l| {
            let t = l.trim();
            let list = t.strip_prefix("feature: [")?.strip_suffix(']')?;
            Some(list.split(',').map(|f| f.trim().to_string()).collect())
        })
        .unwrap_or_default();

    let Some(steps_at) = body.iter().position(|l| l.trim() == "steps:") else {
        return Job {
            id,
            steps: Vec::new(),
            features,
        };
    };
    let rest = &body[steps_at + 1..];

    // Every list item sits at the first indent seen after `steps:`.
    let Some(dash_indent) = rest
        .iter()
        .find(|l| structural(l))
        .map(|l| indent_of(l))
        .filter(|_| rest.iter().any(|l| l.trim().starts_with("- ")))
    else {
        return Job {
            id,
            steps: Vec::new(),
            features,
        };
    };

    // Normalising the `- ` away puts every key of every step at one indent,
    // which is what makes the block-scalar rule below a single comparison.
    let key_indent = dash_indent + 2;
    let flattened: Vec<String> = rest
        .iter()
        .map(|l| {
            if structural(l) && indent_of(l) == dash_indent && l.trim_start().starts_with("- ") {
                format!("{}{}", " ".repeat(key_indent), &l.trim_start()[2..])
            } else {
                (*l).to_string()
            }
        })
        .collect();

    let item_starts: Vec<usize> = (0..rest.len())
        .filter(|&i| {
            structural(rest[i])
                && indent_of(rest[i]) == dash_indent
                && rest[i].trim().starts_with("- ")
        })
        .collect();

    let mut steps = Vec::new();
    for (n, &start) in item_starts.iter().enumerate() {
        let stop = item_starts.get(n + 1).copied().unwrap_or_else(|| {
            // The step runs until the steps list itself ends.
            rest.iter()
                .enumerate()
                .skip(start + 1)
                .find(|(_, l)| structural(l) && indent_of(l) < dash_indent)
                .map(|(i, _)| i)
                .unwrap_or(rest.len())
        });
        steps.push(parse_step(&flattened[start..stop], key_indent, whose, &id));
    }

    Job {
        id,
        steps,
        features,
    }
}

fn parse_step(lines: &[String], key_indent: usize, whose: &str, job: &str) -> Step {
    let mut step = Step::default();
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if !structural(line) || indent_of(line) != key_indent {
            i += 1;
            continue;
        }
        let t = line.trim();
        let Some((key, value)) = t.split_once(':') else {
            i += 1;
            continue;
        };
        let key = key.trim();
        let mut value = value.trim().to_string();

        // A block scalar: the value is on the following, more-indented lines.
        if value.is_empty() || value == "|" || value == "|-" || value == ">" || value == ">-" {
            let mut block = Vec::new();
            let mut j = i + 1;
            while j < lines.len() && (!structural(&lines[j]) || indent_of(&lines[j]) > key_indent) {
                if structural(&lines[j]) {
                    block.push(lines[j].trim().to_string());
                }
                j += 1;
            }
            i = j;
            value = block.join("\n");
        } else {
            i += 1;
        }

        match key {
            "name" => step.name = Some(strip_quotes(&value)),
            "run" => step.run = Some(value),
            "working-directory" => step.working_directory = Some(strip_quotes(&value)),
            _ => {}
        }
    }
    let _ = (whose, job);
    step
}

fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && (s.starts_with('\'') && s.ends_with('\'') || s.starts_with('"') && s.ends_with('"'))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Turning a mirrored job into the commands the hook has to run
// ---------------------------------------------------------------------------

/// Expand any glob in `cmd` the way the hook's own shell did, in the same
/// directory, so `shellcheck tools/*.sh` on the CI side and the file list the
/// hook actually passed compare equal.
///
/// Only word expansion runs here, never the command, and only for a command
/// that has a glob character and none of the shell metacharacters that would
/// make that unsafe.
fn expand_globs(cmd: &str, dir: &Path) -> String {
    if !cmd.contains(['*', '?']) {
        return cmd.to_string();
    }
    if cmd.contains(['&', '|', ';', '$', '`', '(', ')', '<', '>', '\'', '"']) {
        return cmd.to_string();
    }
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!("for w in {cmd}; do printf '%s\\n' \"$w\"; done"))
        .current_dir(dir)
        .output()
        .expect("expand a glob the way the hook's shell would");
    if !out.status.success() {
        return cmd.to_string();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The directories the mirrored jobs ask their steps to run in, expressed the
/// way they are locally: relative to the repo root, with CI's checkout
/// subdirectory taken off.
fn local_workdirs(c: &RepoContract, jobs: &[Job]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for job in jobs.iter().filter(|j| c.mirrored.contains(&j.id)) {
        for wd in job
            .steps
            .iter()
            .filter_map(|s| s.working_directory.as_ref())
        {
            let local = match &c.checkout_path {
                Some(p) if wd == p => String::new(),
                Some(p) => wd.strip_prefix(&format!("{p}/")).unwrap_or(wd).to_string(),
                None => wd.clone(),
            };
            if !local.is_empty() && local != "." {
                out.insert(local);
            }
        }
    }
    out
}

struct Mirrored {
    commands: BTreeSet<String>,
    /// Which exemptions actually matched a step.
    used_exemptions: BTreeSet<String>,
}

fn mirrored_commands(repo: &str, c: &RepoContract, jobs: &[Job], dir: &Path) -> Mirrored {
    let by_id: BTreeMap<&str, &Job> = jobs.iter().map(|j| (j.id.as_str(), j)).collect();
    let mut commands = BTreeSet::new();
    let mut used = BTreeSet::new();

    for want in &c.mirrored {
        let job = by_id.get(want.as_str()).unwrap_or_else(|| {
            panic!(
                "tools/install-hooks.sh says the pre-commit hook for {repo} mirrors \
                 the `{want}` job, and {} has no job by that name. Either the job \
                 was renamed and the mirror list did not follow, or the hook is \
                 standing in for something that no longer exists.",
                c.workflow
            )
        });

        for step in &job.steps {
            let Some(run) = step.run.as_deref() else {
                continue;
            };
            let first = run.lines().next().unwrap_or("").trim().to_string();
            let ids = [step.name.clone(), Some(first.clone())];
            if let Some(hit) = ids
                .iter()
                .flatten()
                .find(|id| c.exempt.contains_key(id.as_str()))
            {
                used.insert(hit.clone());
                continue;
            }

            assert!(
                !run.contains('\n'),
                "{repo}'s `{want}` job runs a multi-line shell block that the \
                 pre-commit hook does not, and a hook cannot faithfully stand in \
                 for an arbitrary block. Either exempt it in \
                 tools/install-hooks.sh with a reason, or give the hook a step \
                 that does the same job. The block starts:\n  {first}"
            );

            let mut cmd = run.trim().to_string();
            if let Some(prefix) = &c.checkout_path {
                let with_slash = format!("{prefix}/");
                if let Some(rest) = cmd.strip_prefix(&with_slash) {
                    cmd = rest.to_string();
                }
            }
            if let Some(wd) = &step.working_directory {
                let local = match &c.checkout_path {
                    Some(p) if wd == p => String::new(),
                    Some(p) => wd
                        .strip_prefix(&format!("{p}/"))
                        .unwrap_or(wd.as_str())
                        .to_string(),
                    None => wd.clone(),
                };
                if !local.is_empty() && local != "." {
                    cmd = format!("cd {local} && {cmd}");
                }
            }

            if cmd.contains("${{ matrix.feature }}") {
                assert!(
                    !job.features.is_empty(),
                    "{repo}'s `{want}` job uses a feature matrix but declares no \
                     `feature: [...]` list, so this guard cannot expand it: {cmd}"
                );
                for feature in &job.features {
                    commands.insert(expand_globs(
                        &cmd.replace("${{ matrix.feature }}", feature),
                        dir,
                    ));
                }
            } else {
                commands.insert(expand_globs(&cmd, dir));
            }
        }
    }

    Mirrored {
        commands,
        used_exemptions: used,
    }
}

// ---------------------------------------------------------------------------
// What the installed hook actually runs
// ---------------------------------------------------------------------------

/// Run the pre-commit hook `install-hooks.sh` wrote for `repo` with a recorder
/// in front of every tool it can reach, and report what it invoked.
fn hook_runs(ws: &Workspace, repo: &str, workdirs: &BTreeSet<String>) -> BTreeSet<String> {
    let (ok, recorded, printed) = run_recorded(ws, repo, workdirs);
    assert!(
        ok,
        "the pre-commit hook for {repo} failed with every command stubbed to \
         succeed, so it is broken independently of what it runs:\n{printed}"
    );
    recorded
}

/// The same, handing back the verdict and the output as well, for the cases
/// that are about a hook deciding not to run something.
fn run_recorded(
    ws: &Workspace,
    repo: &str,
    workdirs: &BTreeSet<String>,
) -> (bool, BTreeSet<String>, String) {
    let repo_dir = ws.repo(repo);
    let hook = repo_dir.join(".git/hooks/pre-commit");
    assert!(
        hook.is_file(),
        "no pre-commit hook was installed into {repo}, so this guard would \
         pass on an empty recording"
    );

    let bin = ws.root.join("recorders").join(repo);
    std::fs::create_dir_all(&bin).expect("create the recorder directory");
    for tool in RECORDED_TOOLS {
        let path = bin.join(tool);
        std::fs::write(&path, recorder(tool)).expect("write a recording tool");
        make_executable(&path);
    }
    let git = bin.join("git");
    std::fs::write(&git, git_recorder(&real_git())).expect("write the recording git");
    make_executable(&git);

    for script in RECORDED_SCRIPTS {
        let path = repo_dir.join(script);
        std::fs::create_dir_all(path.parent().expect("script directory"))
            .expect("create a script directory");
        std::fs::write(&path, SCRIPT_RECORDER).expect("write a recording script");
        make_executable(&path);
    }

    // A step that carries a `working-directory:` becomes a `cd` in the hook,
    // and a `cd` into a directory the stand-in does not have takes the hook
    // down before it reaches anything. These come from the workflow rather
    // than from reading the hook, so a hook that cds somewhere CI never asks
    // it to still fails here rather than being quietly accommodated.
    for dir in workdirs {
        std::fs::create_dir_all(repo_dir.join(dir)).expect("create a stand-in working directory");
    }

    let record = ws.root.join(format!("{repo}.record"));
    let _ = std::fs::remove_file(&record);
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let out = Command::new("bash")
        .arg(&hook)
        .current_dir(&repo_dir)
        .env("PATH", path)
        .env("LOCKSTEP_RECORD", &record)
        .env("LOCKSTEP_ROOT", &repo_dir)
        .output()
        .expect("run the generated pre-commit hook");

    let recorded = std::fs::read_to_string(&record)
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), recorded, printed)
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

/// Where a repo's checkout is, or `None` when it is not laid down here.
fn sibling(repo: &str) -> Option<PathBuf> {
    if repo == "libviprs-tests" {
        return Some(repo_root());
    }
    if repo == "libviprs-cli" {
        let dir = cli_dir();
        return dir
            .join(".github/workflows/ci.yml")
            .is_file()
            .then_some(dir);
    }
    let dir = repo_root().join("..").join(repo);
    dir.join(".github").is_dir().then_some(dir)
}

/// A missing sibling reads to `cargo test` as a pass, so the job that lays
/// them all down sets this and turns the skip into a panic. Same shape as
/// `VIPRS_REQUIRE_CLI` (`tests/common/cli.rs`).
fn require_siblings() -> bool {
    std::env::var("VIPRS_REQUIRE_SIBLINGS").is_ok_and(|v| v == "1")
}

fn workflow_of(dir: &Path, rel: &str) -> String {
    let path = dir.join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Resolve a repo to its checkout and parsed workflow, or report why it is
/// being skipped. Returns `None` only where skipping is allowed.
fn resolve(repo: &str, c: &RepoContract) -> Option<(PathBuf, Vec<Job>)> {
    let Some(dir) = sibling(repo) else {
        assert!(
            !(require_siblings() || (repo == "libviprs-cli" && require_cli())),
            "the siblings are required here but {repo} is not laid down beside \
             this checkout, so this guard would compare nothing and report a \
             false green"
        );
        return None;
    };
    let text = workflow_of(&dir, &c.workflow);
    let jobs = parse_jobs(&text, repo);
    Some((dir, jobs))
}

fn check_repo(repo: &str) {
    let contract = contract();
    let c = contract
        .get(repo)
        .unwrap_or_else(|| panic!("tools/install-hooks.sh --describe never mentions {repo}"));
    let Some((_dir, jobs)) = resolve(repo, c) else {
        return;
    };

    // 1. Every job is either mirrored or deferred with a reason.
    let unaccounted: Vec<&str> = jobs
        .iter()
        .map(|j| j.id.as_str())
        .filter(|id| !c.mirrored.iter().any(|m| m == id) && !c.deferred.contains_key(*id))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "{repo}'s {} has jobs the local hooks account for neither way:\n  {}\n\
         Every job is either in the lint half the pre-commit hook mirrors \
         (`mirror_jobs`) or deferred with a reason (`deferred_jobs`). A job in \
         neither is CI coverage with no local counterpart and nothing saying \
         why. Decide which it is in tools/install-hooks.sh.",
        c.workflow,
        unaccounted.join("\n  ")
    );

    // A deferral naming a job the workflow no longer has is dead text.
    let stale: Vec<&str> = c
        .deferred
        .keys()
        .map(|k| k.as_str())
        .filter(|k| !jobs.iter().any(|j| j.id == *k))
        .collect();
    assert!(
        stale.is_empty(),
        "tools/install-hooks.sh defers these jobs of {repo} and {} has no job by \
         those names:\n  {}\n\
         Either the job was renamed and the reason did not follow, or it is gone \
         and the entry should be too.",
        c.workflow,
        stale.join("\n  ")
    );

    if c.mirrored.is_empty() {
        return;
    }

    // 2, 3 and 4 need the hook actually run.
    let ws = Workspace::new();
    let hook = hook_runs(&ws, repo, &local_workdirs(c, &jobs));
    let m = mirrored_commands(repo, c, &jobs, &ws.repo(repo));

    assert!(
        !m.commands.is_empty(),
        "the jobs the hook mirrors for {repo} run no commands at all, so this \
         guard would pass on a hook that runs nothing"
    );

    let missing: Vec<&String> = m.commands.difference(&hook).collect();
    let extra: Vec<&String> = hook.difference(&m.commands).collect();

    assert!(
        missing.is_empty(),
        "{repo}'s CI runs these in the jobs the pre-commit hook mirrors, and the \
         hook does not:\n  {}\n\
         Each one is a commit that passes locally and fails remotely, which is \
         the whole reason the hook exists. Add them to the step list for {repo} \
         in tools/install-hooks.sh, or exempt them there with a reason.",
        missing
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
    assert!(
        extra.is_empty(),
        "the pre-commit hook for {repo} runs these and the jobs it mirrors do \
         not:\n  {}\n\
         Either CI lost a check the hook still remembers, or the hook is \
         spending a compile on something nothing asks for. Reconcile them in \
         tools/install-hooks.sh.",
        extra
            .iter()
            .map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    // 4. No exemption may name a step CI does not run.
    let dead: Vec<&str> = c
        .exempt
        .keys()
        .map(|k| k.as_str())
        .filter(|k| !m.used_exemptions.contains(*k))
        .collect();
    assert!(
        dead.is_empty(),
        "tools/install-hooks.sh exempts these steps of {repo} from the \
         pre-commit hook and no step in the jobs it mirrors matches \
         them:\n  {}\n\
         An exemption that matches nothing is a hole with a reassuring comment \
         over it: the step it was written for has been renamed or moved, and \
         whatever is there now is going unchecked.",
        dead.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// One test per repo, so a failure names the repo without reading the message
// ---------------------------------------------------------------------------

/// Every repo below has a test of its own, and this is what stops a new one
/// arriving without one.
///
/// A repo added to `REPOS` in the installer gets a pre-commit hook in
/// everybody's checkout immediately. Nothing else here would notice: the
/// per-repo tests name the repos they check, so an eighth repo would simply be
/// unmentioned, and unmentioned reads to `cargo test` as a pass. That is the
/// shape of hole this whole file exists to close, so it gets closed here too.
const CHECKED_REPOS: &[&str] = &[
    "libviprs",
    "libviprs-cli",
    "libviprs-tests",
    "libviprs-bench",
    "libviprs-org",
    "libviprs-dep",
    "pdfium-render",
];

#[test]
fn every_repo_the_installer_visits_has_a_test_here() {
    let visits: BTreeSet<String> = contract().keys().cloned().collect();
    let checked: BTreeSet<String> = CHECKED_REPOS.iter().map(|r| r.to_string()).collect();

    let unchecked: Vec<&String> = visits.difference(&checked).collect();
    assert!(
        unchecked.is_empty(),
        "tools/install-hooks.sh installs hooks into these and nothing here \
         checks them against their CI:\n  {}\n\
         Add a test calling check_repo for each, and add it to CHECKED_REPOS.",
        unchecked
            .iter()
            .map(|r| r.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );

    let gone: Vec<&String> = checked.difference(&visits).collect();
    assert!(
        gone.is_empty(),
        "these are checked here and the installer no longer visits them:\n  {}\n\
         A test that resolves nothing still passes, so drop them from \
         CHECKED_REPOS along with their tests.",
        gone.iter()
            .map(|r| r.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// This repo. Its own workflow is always here, so there is nothing to skip.
#[test]
fn the_hook_mirrors_this_repos_ci() {
    check_repo("libviprs-tests");
}

/// The core. `libviprs = { path = "../libviprs" }` in this crate's manifest
/// means it is there whenever this suite can run at all.
///
/// The core ships `tools/local-ci.py`, so its real pre-commit hook defers to
/// that rather than to the step list below, and this arm is checking the
/// fallback. The fallback still matters: it is what lands in any repo without
/// one, it is what the core gets back if that script goes away, and it is the
/// only written statement of what the core's lint half is. What `local-ci.py`
/// runs is that script's own business and it derives its list from ci.yml at
/// run time.
#[test]
fn the_hook_mirrors_the_cores_ci() {
    check_repo("libviprs");
}

/// The cli is not laid down by the default `test` job or by the Docker gate,
/// so this one skips where it is absent. Skipping reads to `cargo test` as a
/// pass, so it follows the same false-green guard as the differential cells:
/// `VIPRS_REQUIRE_CLI=1`, set on the job that does lay the cli down, turns the
/// skip into a panic (`tests/common/cli.rs`).
#[test]
fn the_hook_mirrors_the_clis_ci() {
    check_repo("libviprs-cli");
}

/// The benchmark crate, which had no local hook at all until #198. Its CI has
/// no `cargo fmt` line, so neither does its hook: a hook step CI does not have
/// is drift in the other direction and the comparison above refuses it.
#[test]
fn the_hook_mirrors_the_bench_ci() {
    check_repo("libviprs-bench");
}

/// The doc site, which also had no local hook. Not a Rust repo: its CI is four
/// regenerate-and-assert-no-drift gates in node, shell and a small cargo
/// extractor, and the hook mirrors all four.
#[test]
fn the_hook_mirrors_the_org_site_ci() {
    check_repo("libviprs-org");
}

/// The pdfium build inputs. This repo ships an installer of its own whose hook
/// skips shellcheck when shellcheck is not installed, which is a green commit
/// on a check CI will fail. Nothing held that script to this workflow; this
/// does.
#[test]
fn the_hook_mirrors_the_dep_ci() {
    check_repo("libviprs-dep");
}

/// pdfium-render is the one repo whose pre-commit hook is not a CI mirror, and
/// that is only defensible while upstream's workflow really does run no lint.
/// The moment it grows a `cargo fmt` or `cargo clippy` line there is something
/// to mirror, and this goes red saying so.
#[test]
fn the_pdfium_fork_ci_still_has_no_lint_to_mirror() {
    let contract = contract();
    let c = contract
        .get("pdfium-render")
        .expect("tools/install-hooks.sh --describe never mentions pdfium-render");
    assert!(
        c.mirrored.is_empty(),
        "pdfium-render now mirrors CI jobs, so this guard is the wrong shape \
         for it and check_repo should cover it instead"
    );
    let Some((_dir, jobs)) = resolve("pdfium-render", c) else {
        return;
    };

    let lints: Vec<String> = jobs
        .iter()
        .flat_map(|j| j.steps.iter().filter_map(|s| s.run.clone()))
        .flat_map(|run| {
            run.lines()
                .map(|l| l.trim().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|l| l.starts_with("cargo clippy") || l.starts_with("cargo fmt"))
        .collect();
    assert!(
        lints.is_empty(),
        "pdfium-render's CI now runs lints:\n  {}\n\
         The fork's pre-commit hook is fork policy rather than a CI mirror, and \
         the reason written down for that is that upstream's workflow lints \
         nothing. That is no longer true, so either mirror these or rewrite the \
         reason.",
        lints.join("\n  ")
    );

    // The same workflow still has to be readable, or the assertion above is
    // vacuous: a parse that found no steps would also find no lints.
    let steps: usize = jobs.iter().map(|j| j.steps.len()).sum();
    assert!(
        steps > 50,
        "this guard read only {steps} steps out of pdfium-render's workflow, \
         which is far fewer than the 205 compatibility cells it carries, so the \
         `no lints` verdict above is about a file this guard failed to parse"
    );
}

/// The doc site's sync check is the one conditional step in any of these
/// hooks, and it mirrors a conditional in the workflow: CI runs the
/// frozen-copy comparison only when the canonical `libviprs-cli` checkout
/// succeeded, and skips it green otherwise. Locally the equivalent question is
/// whether the sibling is there at all.
///
/// A condition has two arms and only one of them is exercised by the
/// comparison above, which runs in a stand-in workspace that always has the
/// sibling. So this drives the other arm. A condition that was accidentally
/// always-true would pass every test in this file without it, and one that was
/// always-false would silently drop the sync check in every real checkout.
#[test]
fn the_org_hook_skips_the_sync_check_only_when_the_cli_sibling_is_missing() {
    let contract = contract();
    let c = contract
        .get("libviprs-org")
        .expect("tools/install-hooks.sh --describe never mentions libviprs-org");
    let Some((_dir, jobs)) = resolve("libviprs-org", c) else {
        return;
    };
    let workdirs = local_workdirs(c, &jobs);
    const SYNC: &str = "cli/tools/sync-cli-src.sh --check";

    // With the sibling there, the check runs.
    let ws = Workspace::new();
    let (ok, recorded, printed) = run_recorded(&ws, "libviprs-org", &workdirs);
    assert!(
        ok,
        "the doc site's hook failed with the sibling present:\n{printed}"
    );
    assert!(
        recorded.iter().any(|l| l == SYNC),
        "the doc site's hook did not run the frozen-copy sync check even with \
         the libviprs-cli sibling laid down beside it, so the condition is \
         always false and that check never runs anywhere.\nIt ran:\n  {}",
        recorded.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );

    // With the sibling gone, it skips, says so, and still runs everything else.
    let ws = Workspace::new();
    std::fs::remove_dir_all(ws.repo("libviprs-cli")).expect("take the cli sibling away");
    let (ok, recorded, printed) = run_recorded(&ws, "libviprs-org", &workdirs);
    assert!(
        ok,
        "the doc site's hook refused the commit because the libviprs-cli \
         sibling is absent. CI skips that step green in the same situation, so \
         this makes the hook stricter than the thing it stands in for and \
         unusable for anyone who has not cloned the cli.\n{printed}"
    );
    assert!(
        !recorded.iter().any(|l| l == SYNC),
        "the doc site's hook ran the frozen-copy sync check with no \
         libviprs-cli sibling to compare against, so the condition is always \
         true and the check is comparing against whatever happens to be \
         there.\nIt ran:\n  {}",
        recorded.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );
    assert!(
        printed.contains("skipping") && printed.contains("libviprs-cli"),
        "the doc site's hook skipped the sync check without saying so. A check \
         that quietly does not run is worth less than no check, because the \
         green still reads as a green:\n{printed}"
    );
    assert!(
        recorded.len() >= 8,
        "with the sync check skipped the hook only ran {} steps, so it did not \
         carry on to the rest of the workflow.\nIt ran:\n  {}",
        recorded.len(),
        recorded.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );
}
