//! Two gate runs over different trees must not share the Docker build context
//! or the image tag.
//!
//! # The failure this exists to stop
//!
//! `tools/run-tests.sh` stages both trees under test into a build context and
//! then builds an image from it. Both of those used to be one fixed location:
//! `tmp/build-context` beside the script, and the tag `libviprs-tests:local`.
//! One name, every caller, and the staging is `rsync -a --delete`.
//!
//! Serially that is a feature, and the script says so: a worktree reuses the
//! warm copy instead of re-staging from scratch. Concurrently it is unsound.
//! Whichever run rsyncs last owns the staged source, and the others build and
//! test **that** tree. Measured during a four-lane campaign: the shared context
//! held one lane's `EXPECTED_FS_TOUCHING_TESTS` while two other lanes were
//! mid-gate against it, and a test file belonging to a third lane was not in
//! the staged tree at all.
//!
//! What makes it worth a test rather than a note is that the result does not
//! look wrong. The run prints the caller's own path and sha:
//!
//! ```text
//!   libviprs:       /Users/…/.lanes/a1130 (44c5cbe5)
//!   image:          libviprs-tests:local
//! ```
//!
//! so a contaminated run is attributed to the right commit, and a green from it
//! is indistinguishable from a real one. That is the pass nobody rechecks.
//!
//! The script already had the shape of the answer and applied it to one name
//! out of three: `CONTAINER_NAME` carries a per-tree suffix, with a comment
//! explaining that a fixed name makes two runs collide. The context directory
//! and the image tag needed the same treatment, and the image tag had no
//! environment override at all, so a private context could not save a caller
//! on its own: the image embeds the context, so sharing the tag re-shares the
//! source.
//!
//! # Why the default rather than a flag
//!
//! `RUN_TESTS_CONTEXT_DIR` already existed and was documented, and four lanes
//! still collided, because nobody registers taking a default as a choice. A
//! default that is wrong under concurrency will keep being taken. So these
//! assert the *derived* value, not that an override works.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tests_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// A directory that looks enough like a libviprs checkout for `--dry-run`.
///
/// The script checks the directory exists and carries a `Cargo.toml`, and
/// `--dry-run` returns before it needs anything else, including a Docker
/// daemon: the availability wait is guarded on the flag and both `docker info`
/// reads fall back to `0`. So this runs the same in the CI image and on a
/// laptop, which a test about concurrency had better do.
fn fake_tree(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("viprs-gate-isolation-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fake tree");
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"fake\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    dir
}

/// The `key: value` lines the plan prints, lowercased keys, trimmed values.
fn plan_for(libviprs: &Path) -> Vec<(String, String)> {
    let script = tests_root().join("tools/run-tests.sh");
    let out = Command::new("sh")
        .arg(&script)
        .arg("--dry-run")
        .arg("--libviprs")
        .arg(libviprs)
        .arg("--libviprs-tests")
        .arg(tests_root())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", script.display()));

    assert!(
        out.status.success(),
        "`run-tests.sh --dry-run` failed ({}):\n{}\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let k = k.trim().to_ascii_lowercase();
            if k.is_empty() || k.contains(' ') {
                return None;
            }
            Some((k, v.trim().to_string()))
        })
        .collect()
}

fn field(plan: &[(String, String)], key: &str) -> String {
    plan.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| {
            panic!(
                "`run-tests.sh --dry-run` printed no `{key}:` line, so this test \
                 cannot see what the run would use. Keys it did print: {:?}. \
                 Both the image tag and the build context have to appear in the \
                 plan, because the plan is what somebody reads to check the run \
                 is about the tree they think it is.",
                plan.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>()
            )
        })
}

/// Different trees, different image tag and different build context.
#[test]
fn two_tree_pairs_do_not_share_an_image_tag_or_a_build_context() {
    let a = fake_tree("a");
    let b = fake_tree("b");
    let plan_a = plan_for(&a);
    let plan_b = plan_for(&b);

    for key in ["image", "context"] {
        let va = field(&plan_a, key);
        let vb = field(&plan_b, key);
        assert_ne!(
            va, vb,
            "two different libviprs trees resolve to the same `{key}`: {va}\n\n\
             The context is staged with `rsync -a --delete` and the image is \
             built from it, so sharing either means one run tests another run's \
             source while reporting its own path and sha. Derive this per tree \
             pair the way CONTAINER_NAME already is."
        );
    }

    let _ = fs::remove_dir_all(&a);
    let _ = fs::remove_dir_all(&b);
}

/// The same trees, the same names, every time.
///
/// Without this, a random or time-based suffix would satisfy the test above and
/// break every warm-cache run, which is the reason the shared context existed.
#[test]
fn the_same_tree_pair_resolves_to_the_same_names() {
    let a = fake_tree("stable");
    let first = plan_for(&a);
    let second = plan_for(&a);

    for key in ["image", "context", "container"] {
        let f = field(&first, key);
        let s = field(&second, key);
        assert_eq!(
            f, s,
            "`{key}` changed between two runs over the same trees: {f} then {s}. \
             It has to be derived from the trees, not from the clock or a random \
             value, or every run stages and builds from cold."
        );
    }

    let _ = fs::remove_dir_all(&a);
}
