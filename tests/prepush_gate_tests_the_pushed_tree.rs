//! Guards that the local pre-push gate tests the tree being pushed
//! (libviprs/libviprs#684).
//!
//! `tools/run-tests.sh` used to derive the trees under test from its own
//! location, so `$WORKSPACE_ROOT/libviprs` was the answer no matter which
//! working tree the push came from. Epic #520 runs every lane in a git
//! worktree, and a repo shares one hooks directory between its main checkout
//! and all of its linked worktrees, so the gate compiled `main` on every lane
//! push and could not fail on the author's own changes. A green pre-push read
//! as "my branch passes" and meant nothing of the kind.
//!
//! Two properties have to hold together, and neither is visible from the
//! other side, which is why both are pinned here:
//!
//!   * the hook has to *find* the pushed tree, and it cannot do that from
//!     `$0` (shared hooks directory) or by walking up from `.git` (in a
//!     worktree that is a file holding a `gitdir:` pointer, not a directory).
//!     Only `git rev-parse` knows.
//!   * `run-tests.sh` has to *accept* it. It did not have a parameter for it
//!     at all, so passing one from the hook was not enough on its own.
//!
//! These are text and behaviour guards on the harness rather than on library
//! output, in the same shape as `counterpart_pinning` and
//! `feature_rename_docs_present`: there is no libvips analogue for "the local
//! gate is honest", and the failure mode is silent by construction.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The pre-push hook body exactly as `install-hooks.sh` writes it: the
/// heredoc between `<< 'HOOK'` and the closing `HOOK`. Reading the generated
/// text rather than the generator keeps the assertions below pointed at what
/// actually lands in `.git/hooks/pre-push`.
fn generated_pre_push() -> String {
    let script = read("tools/install-hooks.sh");
    let start = script
        .find("cat > \"$pre_push\" << 'HOOK'\n")
        .map(|i| i + "cat > \"$pre_push\" << 'HOOK'\n".len())
        .expect(
            "tools/install-hooks.sh no longer writes the pre-push hook from a \
             `cat > \"$pre_push\" << 'HOOK'` heredoc; update this guard to \
             follow it rather than deleting it",
        );
    let rest = &script[start..];
    let end = rest
        .find("\nHOOK\n")
        .expect("unterminated pre-push heredoc in tools/install-hooks.sh");
    rest[..end].to_string()
}

/// #684: the hook must ask git which tree is being pushed. `$0` points into
/// the hooks directory, which a main checkout shares with every worktree
/// hanging off it, so anything resolved from it names the main checkout.
#[test]
fn pre_push_hook_resolves_the_pushed_tree_from_git() {
    let hook = generated_pre_push();

    assert!(
        hook.contains("git rev-parse --show-toplevel"),
        "the pre-push hook must take the tree under test from \
         `git rev-parse --show-toplevel`, which is the working tree whose \
         commits are going out (#684)"
    );
    assert!(
        hook.contains("git rev-parse --git-common-dir"),
        "the pre-push hook must find the repo's main checkout through \
         `git rev-parse --git-common-dir`; a linked worktree's `.git` is a \
         file holding a `gitdir:` pointer, so walking up from it does not \
         give a repository root (#684)"
    );
    assert!(
        !hook.contains(r#"$(dirname "$0")/../.."#),
        "the pre-push hook still derives a repository from its own path. \
         Every worktree of a repo runs the same hook file out of the main \
         checkout's hooks directory, so that always names the main checkout \
         and never the branch being pushed (#684)"
    );
}

/// #684: finding the tree is only half of it. The hook has to hand it over,
/// and `run-tests.sh` has to have somewhere to put it.
#[test]
fn pre_push_hook_hands_the_tree_to_run_tests() {
    let hook = generated_pre_push();

    for slot in ["LIBVIPRS_DIR", "LIBVIPRS_TESTS_DIR"] {
        assert!(
            hook.contains(&format!("export {slot}=")),
            "the pre-push hook must export {slot} so run-tests.sh builds the \
             pushed tree; without it the script falls back to the sibling \
             checkout, which is the whole of #684"
        );
    }

    let script = read("tools/run-tests.sh");
    for slot in ["LIBVIPRS_DIR", "LIBVIPRS_TESTS_DIR"] {
        assert!(
            script.contains(slot),
            "tools/run-tests.sh must read {slot}, or the hook exporting it \
             has no effect (#684)"
        );
    }
}

/// The behavioural half: hand `run-tests.sh` a tree and it must say, before
/// it builds anything, that it is going to build that tree. `--plan` exists
/// so this is answerable without Docker, and so a human can ask the same
/// question from a worktree in under a second.
#[test]
fn run_tests_plan_reports_the_tree_it_was_handed() {
    let core = tempfile::tempdir().expect("temp dir for a stand-in core crate");
    let core_path = core.path().canonicalize().expect("canonical temp path");
    std::fs::write(
        core_path.join("Cargo.toml"),
        "[package]\nname = \"libviprs\"\nversion = \"0.0.0\"\n",
    )
    .expect("write stand-in Cargo.toml");

    let script = repo_root().join("tools/run-tests.sh");
    let root = repo_root();

    let by_flag = Command::new("bash")
        .arg(&script)
        .arg("--plan")
        .arg("--libviprs")
        .arg(&core_path)
        .arg("--libviprs-tests")
        .arg(&root)
        .output()
        .expect("run tools/run-tests.sh --plan");
    assert!(
        by_flag.status.success(),
        "`run-tests.sh --plan` failed: {}",
        String::from_utf8_lossy(&by_flag.stderr)
    );
    let by_flag = String::from_utf8_lossy(&by_flag.stdout).to_string();

    let by_env = Command::new("bash")
        .arg(&script)
        .arg("--plan")
        .env("LIBVIPRS_DIR", &core_path)
        .env("LIBVIPRS_TESTS_DIR", &root)
        .output()
        .expect("run tools/run-tests.sh --plan with the env form");
    assert!(
        by_env.status.success(),
        "`run-tests.sh --plan` with LIBVIPRS_DIR set failed: {}",
        String::from_utf8_lossy(&by_env.stderr)
    );
    let by_env = String::from_utf8_lossy(&by_env.stdout).to_string();

    let wanted = core_path.display().to_string();
    for (form, plan) in [("--libviprs", &by_flag), ("LIBVIPRS_DIR", &by_env)] {
        assert!(
            plan.contains(&wanted),
            "run-tests.sh ignored the core tree passed by {form}. It planned:\n{plan}"
        );
    }
}

/// The default has to keep working, because the sibling layout is what CI,
/// the README and every by-hand invocation use. #684 is about the default
/// being the *only* answer, not about the default being wrong.
#[test]
fn run_tests_still_defaults_to_the_sibling_layout() {
    let script = repo_root().join("tools/run-tests.sh");
    let out = Command::new("bash")
        .arg(&script)
        .arg("--plan")
        .env_remove("LIBVIPRS_DIR")
        .env_remove("LIBVIPRS_TESTS_DIR")
        .output()
        .expect("run tools/run-tests.sh --plan with no overrides");
    assert!(
        out.status.success(),
        "`run-tests.sh --plan` failed with no overrides: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let plan = String::from_utf8_lossy(&out.stdout);

    // `libviprs = { path = "../libviprs" }` in this crate's manifest means the
    // sibling is present whenever this test can run at all.
    let sibling = repo_root()
        .join("../libviprs")
        .canonicalize()
        .expect("the core crate sits at ../libviprs; this suite could not have compiled otherwise");
    assert!(
        plan.contains(&sibling.display().to_string()),
        "with nothing passed, run-tests.sh must still plan the sibling core \
         checkout at {}. It planned:\n{plan}",
        sibling.display()
    );
}
