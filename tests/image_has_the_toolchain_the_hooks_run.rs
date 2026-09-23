//! The image this suite runs in must carry every cargo subcommand the hooks it
//! installs actually invoke.
//!
//! # Why this exists
//!
//! `Dockerfile`'s builder stage is `FROM rust:latest`, and `rust:latest` used to
//! ship rustfmt and clippy in its default profile. Nothing here ever had to ask
//! for them, so nothing did, and the omission was never a decision. Then the
//! base image stopped: 1.98.1 installs exactly `cargo`, `rust-std` and `rustc`.
//!
//! What that looked like from the outside was `install_hooks_pdfium_scope`
//! failing 6 of 7 cells on **every** libviprs tree, with
//! `error: 'cargo-fmt' is not installed for the toolchain`, because the hook
//! that test drives runs `cargo fmt -- --check`. A gate that cannot pass for any
//! input is not a gate, and the failure names the toolchain rather than the
//! image, so it reads as a problem in whatever branch happened to be under test.
//!
//! libviprs' own `ci.yml` uses `dtolnay/rust-toolchain@stable`, which installs
//! both, so GitHub was green throughout and only the local mirror was broken.
//! That is the part worth guarding: the mirror and CI had quietly come to
//! disagree about what a toolchain contains, which is the one thing the mirror
//! exists not to do.
//!
//! # Why it derives the list instead of naming rustfmt and clippy
//!
//! Asserting those two would catch the drift that already happened and nothing
//! else. The hooks' command list is data in `tools/install-hooks.sh`, so this
//! reads it and probes whatever it finds. Add a `cargo deny` line there and this
//! starts requiring `cargo deny` without anyone editing this file, which is the
//! difference between a regression test and a note about the past.
//!
//! This is deliberately not `install-hooks.sh --describe`: that reports what the
//! hooks would mirror, and the question here is what they will actually try to
//! execute.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The tests tree, which is the directory holding `tools/install-hooks.sh`.
fn tests_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Every distinct cargo subcommand named in the hook installer's command lists.
///
/// The lists are shell arrays of quoted commands, so a line carrying one looks
/// like `    "cargo clippy --all-targets -- -D warnings"`. Taking the first two
/// words after the quote is enough and stays right if the flags change, which
/// they do far more often than the subcommands.
///
/// `cargo test`, `cargo build` and `cargo run` are excluded: they are part of
/// every toolchain by definition, so requiring them proves nothing and a false
/// red here would be worse than no test.
fn subcommands_the_hooks_run() -> BTreeSet<String> {
    let script = tests_root().join("tools/install-hooks.sh");
    let text = std::fs::read_to_string(&script)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", script.display()));

    let always_present = ["test", "build", "run", "check"];
    let mut found = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        // Only the quoted command entries, not prose about them in comments.
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.split_once("\"cargo ").map(|(_, r)| r) else {
            continue;
        };
        let Some(word) = rest.split_whitespace().next() else {
            continue;
        };
        // Cut at the first character a subcommand cannot contain. Trimming
        // quotes off the ends is not enough: `install-hooks.sh` also matches on
        // these commands in `case` patterns, so the first draft of this read
        // `"cargo fmt"*)` as the subcommand `fmt"*)` and demanded a cargo
        // subcommand of that name. The test caught it, which is the argument
        // for the vacuity guard above rather than against the parser.
        let sub: String = word
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if sub.is_empty() || always_present.contains(&sub.as_str()) {
            continue;
        }
        found.insert(sub);
    }
    found
}

/// The installer really does name at least one subcommand that needs a
/// component, so a green from the test below cannot come from an empty set.
///
/// Without this, deleting the command lists, renaming the script or changing the
/// quoting would make the real check vacuous and it would go green saying
/// nothing. An empty reading is a refusal here, not a pass.
#[test]
fn the_hook_installer_names_subcommands_to_check() {
    let subs = subcommands_the_hooks_run();
    assert!(
        !subs.is_empty(),
        "tools/install-hooks.sh named no cargo subcommand beyond the ones every \
         toolchain has. Either the hooks stopped running lints, which is a real \
         change somebody should be told about, or the command lists moved and \
         this file's parser is now reading nothing. Both make \
         `the_image_carries_every_cargo_subcommand_the_hooks_run` vacuous."
    );
    assert!(
        subs.contains("fmt") && subs.contains("clippy"),
        "expected the hooks to run at least `cargo fmt` and `cargo clippy`, \
         found {subs:?}. If the hooks genuinely dropped one, update this \
         expectation and say why in the commit."
    );
}

/// Every one of those subcommands must actually run in this image.
///
/// Probing with `--version` rather than checking `rustup component list`: the
/// question is whether the command the hook will type works, and a component
/// can be listed while the binary is missing from `PATH`.
#[test]
fn the_image_carries_every_cargo_subcommand_the_hooks_run() {
    let mut missing = Vec::new();
    for sub in subcommands_the_hooks_run() {
        let out = Command::new("cargo").arg(&sub).arg("--version").output();
        let ok = match &out {
            Ok(o) => o.status.success(),
            Err(_) => false,
        };
        if !ok {
            let detail = out
                .map(|o| {
                    String::from_utf8_lossy(&o.stderr)
                        .lines()
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .unwrap_or_else(|e| e.to_string());
            missing.push(format!("  cargo {sub}: {detail}"));
        }
    }
    assert!(
        missing.is_empty(),
        "this image cannot run every cargo subcommand the pre-commit hook \
         invokes:\n{}\n\n\
         The hooks will fail for every tree, not just the one under test, and \
         the error names the toolchain rather than the image, so it reads as a \
         problem in the branch. Fix it where the image is built, in \
         `Dockerfile`'s builder stage:\n\
         \n    RUN rustup component add rustfmt clippy\n\n\
         adding whatever else is listed above. GitHub CI will not show this: \
         libviprs' ci.yml uses `dtolnay/rust-toolchain@stable`, which installs \
         them, so only the local mirror breaks and it breaks silently.",
        missing.join("\n")
    );
}
