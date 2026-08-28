//! Reads the git hooks `tools/install-hooks.sh` writes, so the guards on them
//! can assert against the text that actually lands in `.git/hooks/`.
//!
//! Two suites need this (`prepush_gate_tests_the_pushed_tree` for #684 and
//! `prepush_gate_cost_controls` for #683) and both used to carry their own
//! copy, including the heredoc marker string byte for byte. That marker is the
//! fragile part: change the quoting in `install-hooks.sh` and every copy has to
//! follow, and the one that does not follow fails with "no longer writes the
//! pre-push hook from a heredoc" rather than with anything about the change
//! that broke it. One copy, here.

use std::path::{Path, PathBuf};

/// This crate's root, which is also the repo root for the harness checkout.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Read a repo-relative file, naming the path if it is not there.
pub fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The marker `install-hooks.sh` opens the pre-push heredoc with. Kept as one
/// constant so the two guards cannot drift apart on it.
const PRE_PUSH_HEREDOC: &str = "cat > \"$pre_push\" << 'HOOK'\n";

/// The pre-push hook body exactly as `install-hooks.sh` writes it: the heredoc
/// between `<< 'HOOK'` and the closing `HOOK`. Reading the generated text
/// rather than the generator keeps the assertions pointed at what actually
/// lands in `.git/hooks/pre-push`.
pub fn generated_pre_push() -> String {
    let script = read("tools/install-hooks.sh");
    let start = script
        .find(PRE_PUSH_HEREDOC)
        .map(|i| i + PRE_PUSH_HEREDOC.len())
        .expect(
            "tools/install-hooks.sh no longer writes the pre-push hook from a \
             `cat > \"$pre_push\" << 'HOOK'` heredoc; update PRE_PUSH_HEREDOC in \
             tests/common/hooks.rs to follow it rather than deleting the guards \
             that read it",
        );
    let rest = &script[start..];
    let end = rest
        .find("\nHOOK\n")
        .expect("unterminated pre-push heredoc in tools/install-hooks.sh");
    rest[..end].to_string()
}
