//! Provenance / supply-chain guard for the PDFium binary (issue #56).
//!
//! Issue #56 removed the 6 MiB provenance-less `libpdfium.dylib` that had been
//! committed to the repo and moved Docker + CI onto a pinned, checksum-verified
//! binary published by `libviprs-dep`. These tests pin that outcome so a future
//! change cannot silently regress it: re-commit a native PDFium binary, drop the
//! SHA-256 verification, float the release channel, or let the Docker and CI
//! pins drift out of lockstep.
//!
//! The tests read the checked-in `Dockerfile`, `.github/workflows/ci.yml`,
//! and `.gitignore` from the crate root and assert structural properties. They
//! do not require the `pdfium` feature or a native PDFium install, so they run
//! on every `cargo test` invocation.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let p = root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("failed to read {}: {e}", p.display()))
}

/// A native shared/static library that must never be committed to the repo.
fn is_native_binary(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("libpdfium")
        || lower.ends_with(".dylib")
        || lower.ends_with(".so")
        || lower.ends_with(".dll")
        || lower.ends_with(".a")
        || lower.ends_with(".lib")
}

/// Extract the value that follows `key`, skipping an `=` or `:` separator plus
/// surrounding whitespace and quotes, then reading up to the next delimiter.
/// A real `=`/`:` assignment separator is required, so shell references such as
/// `${PDFIUM_SHA256}` are ignored. Returns every occurrence in document order
/// (the Dockerfile lists two per-arch SHA-256 digests).
fn values_after(haystack: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = haystack.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = haystack[search_from..].find(key) {
        let match_end = search_from + rel + key.len();
        let mut i = match_end;
        let mut saw_separator = false;
        while i < bytes.len()
            && (bytes[i] == b'='
                || bytes[i] == b':'
                || bytes[i] == b' '
                || bytes[i] == b'\t'
                || bytes[i] == b'"'
                || bytes[i] == b'\'')
        {
            if bytes[i] == b'=' || bytes[i] == b':' {
                saw_separator = true;
            }
            i += 1;
        }
        if !saw_separator {
            search_from = match_end;
            continue;
        }
        let start = i;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'"'
                || c == b'\''
                || c == b' '
                || c == b'\t'
                || c == b'\n'
                || c == b'\r'
                || c == b';'
            {
                break;
            }
            i += 1;
        }
        if i > start {
            out.push(haystack[start..i].to_string());
        }
        search_from = i.max(match_end);
    }
    out
}

fn is_pinned_release(v: &str) -> bool {
    // e.g. "pdfium-7881": a fixed prefix followed by a numeric build id.
    match v.strip_prefix("pdfium-") {
        Some(rest) => !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()),
        None => false,
    }
}

fn is_sha256_hex(v: &str) -> bool {
    v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Drop full-line comments so a floating-channel name mentioned in an
/// explanatory comment (documenting what the fetch deliberately avoids) does
/// not trip the negative provenance assertions. Both Dockerfile and YAML use
/// `#` to introduce a comment.
fn strip_comment_lines(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Acceptance criterion 1: the committed dylib is removed from the repo.
/// No native PDFium/library binary may be tracked by git.
#[test]
fn no_native_binary_is_committed() {
    let out = Command::new("git")
        .arg("-C")
        .arg(root())
        .args(["ls-files", "-z"])
        .output();

    match out {
        Ok(o) if o.status.success() => {
            let listing = String::from_utf8_lossy(&o.stdout);
            let offenders: Vec<&str> = listing
                .split('\0')
                .filter(|p| !p.is_empty())
                .filter(|p| {
                    let base = Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    is_native_binary(&base)
                })
                .collect();
            assert!(
                offenders.is_empty(),
                "native library binaries must not be committed (issue #56); found: {offenders:?}"
            );
        }
        _ => {
            // Not a git checkout (e.g. Docker build context copies source without
            // .git). Fall back to a working-tree scan so the guard still holds.
            let mut offenders = Vec::new();
            scan_tree(&root(), &mut offenders);
            assert!(
                offenders.is_empty(),
                "native library binaries must not live in the repo (issue #56); found: {offenders:?}"
            );
        }
    }
}

fn scan_tree(dir: &Path, offenders: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name == "target" || name == ".git" {
                continue;
            }
            scan_tree(&path, offenders);
        } else if is_native_binary(&name) {
            offenders.push(path.display().to_string());
        }
    }
}

/// A re-added dylib/so/dll must stay untracked by default: `.gitignore` keeps
/// the native-binary patterns so a stray build artifact is never committed.
#[test]
fn gitignore_excludes_native_binaries() {
    let ignore = read(".gitignore");
    for pat in ["*.dylib", "*.so", "*.dll"] {
        assert!(
            ignore.lines().any(|l| l.trim() == pat),
            ".gitignore must exclude {pat} so native binaries are never committed (issue #56)"
        );
    }
}

/// Acceptance criterion 2 (Docker): the Dockerfile fetches a pinned,
/// checksum-verified binary sourced from libviprs-dep, not a floating channel.
#[test]
fn dockerfile_fetches_pinned_checksummed_binary() {
    let df = read("Dockerfile");

    assert!(
        df.contains("libviprs/libviprs-dep"),
        "Dockerfile must source PDFium from libviprs-dep (issue #56)"
    );
    assert!(
        df.contains("sha256sum -c"),
        "Dockerfile must verify the PDFium download's SHA-256 (issue #56)"
    );

    let df_code = strip_comment_lines(&df);
    assert!(
        !df_code.contains("releases/latest") && !df_code.to_ascii_lowercase().contains("bblanchon"),
        "Dockerfile must not fetch PDFium from a floating/unpinned channel (issue #56)"
    );

    let releases = values_after(&df, "PDFIUM_RELEASE");
    assert!(
        releases.iter().any(|r| is_pinned_release(r)),
        "Dockerfile PDFIUM_RELEASE must be a pinned build id like pdfium-7881, got {releases:?}"
    );

    let shas: Vec<String> = values_after(&df, "PDFIUM_SHA256")
        .into_iter()
        .filter(|s| is_sha256_hex(s))
        .collect();
    assert!(
        shas.len() >= 2,
        "Dockerfile must declare per-arch 64-char hex PDFIUM_SHA256 digests (issue #56), got {shas:?}"
    );
}

/// Acceptance criterion 2 (CI): the CI workflow fetches a pinned,
/// checksum-verified binary sourced from libviprs-dep, not a floating channel.
#[test]
fn ci_fetches_pinned_checksummed_binary() {
    let ci = read(".github/workflows/ci.yml");

    assert!(
        ci.contains("libviprs/libviprs-dep"),
        "ci.yml must source PDFium from libviprs-dep (issue #56)"
    );
    assert!(
        ci.contains("sha256sum -c"),
        "ci.yml must verify the PDFium download's SHA-256 (issue #56)"
    );

    let ci_code = strip_comment_lines(&ci);
    assert!(
        !ci_code.contains("releases/latest") && !ci_code.to_ascii_lowercase().contains("bblanchon"),
        "ci.yml must not fetch PDFium from a floating/unpinned channel (issue #56)"
    );

    let releases = values_after(&ci, "PDFIUM_RELEASE");
    assert!(
        releases.iter().any(|r| is_pinned_release(r)),
        "ci.yml PDFIUM_RELEASE must be a pinned build id like pdfium-7881, got {releases:?}"
    );

    let shas = values_after(&ci, "PDFIUM_SHA256");
    assert!(
        shas.iter().any(|s| is_sha256_hex(s)),
        "ci.yml must declare a 64-char hex PDFIUM_SHA256 digest, got {shas:?}"
    );
}

/// The Docker and CI pins must agree: same release id and same linux-x64 digest.
/// Both files carry a comment demanding they be kept in lockstep; this enforces
/// it so a bump in one place cannot ship without the other (issue #56).
#[test]
fn docker_and_ci_pins_are_in_lockstep() {
    let df = read("Dockerfile");
    let ci = read(".github/workflows/ci.yml");

    let df_release = values_after(&df, "PDFIUM_RELEASE")
        .into_iter()
        .find(|r| is_pinned_release(r))
        .expect("Dockerfile PDFIUM_RELEASE");
    let ci_release = values_after(&ci, "PDFIUM_RELEASE")
        .into_iter()
        .find(|r| is_pinned_release(r))
        .expect("ci.yml PDFIUM_RELEASE");
    assert_eq!(
        df_release, ci_release,
        "Dockerfile and ci.yml PDFIUM_RELEASE must match (issue #56)"
    );

    // The Dockerfile lists the linux-x64 (amd64) digest first; CI runs on
    // ubuntu-latest / linux-x64 and declares exactly that digest.
    let df_x64 = values_after(&df, "PDFIUM_SHA256")
        .into_iter()
        .find(|s| is_sha256_hex(s))
        .expect("Dockerfile linux-x64 PDFIUM_SHA256");
    let ci_x64 = values_after(&ci, "PDFIUM_SHA256")
        .into_iter()
        .find(|s| is_sha256_hex(s))
        .expect("ci.yml linux-x64 PDFIUM_SHA256");
    assert_eq!(
        df_x64, ci_x64,
        "Dockerfile and ci.yml must pin the same linux-x64 SHA-256 (issue #56)"
    );
}
