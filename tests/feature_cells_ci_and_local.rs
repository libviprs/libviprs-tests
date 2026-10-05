//! Every feature-gated test runs in a CI cell that has its feature, and the
//! local gate replays the same cells CI runs (libviprs-tests#234).
//!
//! # The hole this closes
//!
//! A test behind `#[cfg(feature = "x")]` compiles to nothing in every build
//! that leaves `x` off, and one behind `#[cfg_attr(not(feature = "x"), ignore)]`
//! reports `ignored`. Neither is a failure, so a feature-gated cell that no CI
//! step builds with its feature reads as green forever while asserting nothing.
//! The CLI-driven ones have a second way to do it: without the CLI sibling they
//! return early, and only `VIPRS_REQUIRE_CLI=1` turns that into a panic.
//!
//! The guards that already exist look at this from the side of a feature or a
//! job. `tests/ci_feature_coverage.rs` asks whether each cargo feature has
//! *some* cell, and `tests/pmtiles_ci_wiring.rs` asks whether each CLI suite
//! is named on a step with `VIPRS_REQUIRE_CLI=1`. Neither asks the question per
//! test, and that is where the gap was: `tests/objectstore_list_trait_default.rs`
//! is `#![cfg(feature = "object-store-sink")]`, the feature has a run cell, and
//! that run cell names three other `--test` targets. Its three tests had never
//! run anywhere.
//!
//! So [`every_feature_gated_test_runs_in_a_ci_cell`] reads each `tests/*.rs`,
//! works out the condition under which each `#[test]` compiles and is not
//! ignored, and the feature conditions inside its body, and requires a CI cell
//! that satisfies each of them: a `cargo test` that selects the target, enables
//! the features (through this crate's `[features]` implications), lets the test
//! through any `--` filter, and for a CLI-driven suite sees
//! `VIPRS_REQUIRE_CLI=1`.
//!
//! # The local gate
//!
//! A local green only means what a CI green means if it ran the same cells.
//! `tools/replay-ci.sh` is the local gate's copy of `ci.yml`: every job it does
//! not opt out of, step for step, with the same environment. It is a copy on
//! purpose, so the two can be read side by side, and
//! [`the_local_gate_replays_every_ci_cell`] is what stops the copy drifting:
//! the jobs, the order of the steps, each step's environment and each step's
//! code have to match exactly, and a job that is not replayed has to say why in
//! [`JOBS_NOT_REPLAYED`].
//!
//! # What the scanner reads
//!
//! It relies on rustfmt layout rather than parsing Rust: attributes and items
//! start their own lines, and a function ends at the `}` that sits at its own
//! indentation. Anything it cannot follow (a feature `cfg` on a `mod`, a `cfg`
//! predicate it does not know) is a panic naming the file, never a guess,
//! because a guess here is a test the guard silently stops looking at.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod common;
use common::manifest;
use common::workflows::{Workflow, read_workflow};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// cfg predicates
// ---------------------------------------------------------------------------

/// A `cfg` predicate, as far as these files use them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cfg {
    Feature(String),
    /// A predicate that does not depend on cargo features, already evaluated
    /// for the Linux x86_64 test build CI runs.
    Fixed(bool),
    All(Vec<Cfg>),
    Any(Vec<Cfg>),
    Not(Box<Cfg>),
}

impl Cfg {
    fn eval(&self, features: &BTreeSet<String>) -> bool {
        match self {
            Cfg::Feature(f) => features.contains(f),
            Cfg::Fixed(b) => *b,
            Cfg::All(v) => v.iter().all(|c| c.eval(features)),
            Cfg::Any(v) => v.iter().any(|c| c.eval(features)),
            Cfg::Not(c) => !c.eval(features),
        }
    }

    fn mentions_a_feature(&self) -> bool {
        match self {
            Cfg::Feature(_) => true,
            Cfg::Fixed(_) => false,
            Cfg::All(v) | Cfg::Any(v) => v.iter().any(Cfg::mentions_a_feature),
            Cfg::Not(c) => c.mentions_a_feature(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Eq,
    Open,
    Close,
    Comma,
}

fn lex(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut chars = src.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '(' => {
                chars.next();
                out.push(Tok::Open);
            }
            ')' => {
                chars.next();
                out.push(Tok::Close);
            }
            ',' => {
                chars.next();
                out.push(Tok::Comma);
            }
            '=' => {
                chars.next();
                out.push(Tok::Eq);
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                for ch in chars.by_ref() {
                    if ch == '"' {
                        break;
                    }
                    s.push(ch);
                }
                out.push(Tok::Str(s));
            }
            _ => {
                let mut s = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch.is_alphanumeric() || ch == '_' {
                        s.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
                assert!(
                    !s.is_empty(),
                    "cannot lex the cfg predicate {src:?} at {c:?}"
                );
                out.push(Tok::Ident(s));
            }
        }
    }
    out
}

/// Parse one predicate from the front of `toks`, returning it and the rest.
fn parse_pred<'a>(toks: &'a [Tok], whole: &str) -> (Cfg, &'a [Tok]) {
    let Some(Tok::Ident(name)) = toks.first() else {
        panic!("expected a cfg predicate in {whole:?}, got {toks:?}");
    };
    let rest = &toks[1..];
    match (name.as_str(), rest.first()) {
        ("all" | "any" | "not", Some(Tok::Open)) => {
            let mut items = Vec::new();
            let mut rest = &rest[1..];
            loop {
                if let Some(Tok::Close) = rest.first() {
                    rest = &rest[1..];
                    break;
                }
                let (item, after) = parse_pred(rest, whole);
                items.push(item);
                rest = after;
                if let Some(Tok::Comma) = rest.first() {
                    rest = &rest[1..];
                }
            }
            let cfg = match name.as_str() {
                "all" => Cfg::All(items),
                "any" => Cfg::Any(items),
                _ => {
                    assert_eq!(items.len(), 1, "not() takes one predicate in {whole:?}");
                    Cfg::Not(Box::new(items.remove(0)))
                }
            };
            (cfg, rest)
        }
        (key, Some(Tok::Eq)) => {
            let Some(Tok::Str(value)) = rest.get(1) else {
                panic!("expected a string after `{key} =` in {whole:?}");
            };
            let cfg = match key {
                "feature" => Cfg::Feature(value.clone()),
                "target_os" => Cfg::Fixed(value == "linux"),
                "target_family" => Cfg::Fixed(value == "unix"),
                "target_arch" => Cfg::Fixed(value == "x86_64"),
                "target_pointer_width" => Cfg::Fixed(value == "64"),
                other => panic!(
                    "the cfg key `{other}` in {whole:?} is one this guard does not know how \
                     to evaluate. Teach `parse_pred` what it means on CI's Linux x86_64 \
                     test build rather than letting the test drop out of the scan"
                ),
            };
            (cfg, &rest[2..])
        }
        (ident, _) => {
            let cfg = match ident {
                "unix" | "test" | "debug_assertions" => Cfg::Fixed(true),
                "windows" | "miri" | "loom" => Cfg::Fixed(false),
                other => panic!(
                    "the cfg predicate `{other}` in {whole:?} is one this guard does not \
                     know how to evaluate"
                ),
            };
            (cfg, rest)
        }
    }
}

fn parse_cfg(src: &str) -> Cfg {
    let toks = lex(src);
    let (cfg, rest) = parse_pred(&toks, src);
    assert!(
        rest.is_empty(),
        "trailing tokens after the predicate in {src:?}: {rest:?}"
    );
    cfg
}

/// The text between the outer parentheses that open right after `prefix` in
/// `text`, e.g. the predicate of `#[cfg(...)]`.
fn balanced_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let start = text.find(prefix)? + prefix.len();
    let mut depth = 1usize;
    let mut in_str = false;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split the arguments of `cfg_attr(pred, attr, ...)` at its top-level commas.
fn split_top_level(args: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut in_str, mut from) = (0usize, false, 0usize);
    for (i, ch) in args.char_indices() {
        match ch {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                out.push(args[from..i].trim());
                from = i + 1;
            }
            _ => {}
        }
    }
    if !args[from..].trim().is_empty() {
        out.push(args[from..].trim());
    }
    out
}

// ---------------------------------------------------------------------------
// Tests and their conditions
// ---------------------------------------------------------------------------

/// One `#[test]` and what it takes to run it.
#[derive(Debug)]
struct TestFn {
    suite: String,
    name: String,
    /// Every one must hold for the test to be compiled at all: the file's
    /// `#![cfg]` and the item's own `#[cfg]`s.
    compile: Vec<Cfg>,
    /// If any holds, the test is compiled and reported `ignored`.
    ignore_when: Vec<Cfg>,
    /// An unconditional `#[ignore]`: parked on purpose, with a reason, and run
    /// by nothing that runs the default harness.
    always_ignored: bool,
    /// Feature conditions inside the body (`#[cfg(...)]` blocks and
    /// `cfg!(...)`). Each one has to be true in some cell that runs the test,
    /// or that half of the body runs nowhere.
    body: Vec<Cfg>,
}

impl TestFn {
    fn is_feature_gated(&self) -> bool {
        self.compile
            .iter()
            .chain(&self.ignore_when)
            .chain(&self.body)
            .any(Cfg::mentions_a_feature)
    }
}

fn attribute_cfgs(
    attr: &str,
    compile: &mut Vec<Cfg>,
    ignore_when: &mut Vec<Cfg>,
    always: &mut bool,
) {
    let a = attr.trim();
    if a.starts_with("#[cfg(") {
        compile.push(parse_cfg(
            balanced_after(a, "#[cfg(").expect("unbalanced #[cfg("),
        ));
    } else if a.starts_with("#[cfg_attr(") {
        let args = balanced_after(a, "#[cfg_attr(").expect("unbalanced #[cfg_attr(");
        let parts = split_top_level(args);
        let pred = parse_cfg(parts[0]);
        if parts[1..]
            .iter()
            .any(|p| *p == "ignore" || p.starts_with("ignore =") || p.starts_with("ignore="))
        {
            ignore_when.push(pred);
        }
    } else if a == "#[ignore]" || a.starts_with("#[ignore =") || a.starts_with("#[ignore(") {
        *always = true;
    }
}

/// For every byte of `src`, whether it is code rather than the inside of a
/// string literal or a comment.
///
/// The scanner has to know, because source text quotes Rust: a guard's
/// failure message names `#[cfg(feature = \"{name}\")]`, and a test of the
/// scanner itself holds a whole fake test file in a raw string, `#[test]`s,
/// col-0 closing braces and all. Read as code, both would be wrong in ways
/// that pass.
fn code_mask(src: &str) -> Vec<bool> {
    let b = src.as_bytes();
    let mut mask = vec![true; b.len()];
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    mask[i] = false;
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut depth = 0usize;
                while i < b.len() {
                    if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        mask[i] = false;
                        mask[i + 1] = false;
                        i += 2;
                    } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        mask[i] = false;
                        mask[i + 1] = false;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        mask[i] = false;
                        i += 1;
                    }
                }
            }
            b'r' if {
                // `r"`, `r#"`, `br#"` and so on, but not the `r` that ends
                // an identifier such as `for` or `chr`.
                let starts =
                    i == 0 || !ident(b[i - 1]) || (b[i - 1] == b'b' && (i < 2 || !ident(b[i - 2])));
                let hashes = b[i + 1..].iter().take_while(|c| **c == b'#').count();
                starts && b.get(i + 1 + hashes) == Some(&b'"')
            } =>
            {
                let hashes = b[i + 1..].iter().take_while(|c| **c == b'#').count();
                let close: Vec<u8> = std::iter::once(b'"')
                    .chain(std::iter::repeat_n(b'#', hashes))
                    .collect();
                let body = i + 2 + hashes;
                let end = b[body..]
                    .windows(close.len())
                    .position(|w| w == close.as_slice())
                    .map(|p| body + p)
                    .unwrap_or(b.len());
                for m in &mut mask[body..end] {
                    *m = false;
                }
                i = (end + close.len()).min(b.len());
            }
            b'"' => {
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    mask[i] = false;
                    if b[i] == b'\\' && i + 1 < b.len() {
                        mask[i + 1] = false;
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'\'' => {
                // A char literal, or a lifetime / label, which is code.
                if b.get(i + 1) == Some(&b'\\') {
                    let end = b[i + 2..]
                        .iter()
                        .position(|c| *c == b'\'')
                        .map(|p| i + 2 + p)
                        .unwrap_or(b.len());
                    for m in &mut mask[i + 1..end] {
                        *m = false;
                    }
                    i = end + 1;
                } else if let Some(ch) = src[i + 1..].chars().next()
                    && src.as_bytes().get(i + 1 + ch.len_utf8()) == Some(&b'\'')
                {
                    for m in &mut mask[i + 1..i + 1 + ch.len_utf8()] {
                        *m = false;
                    }
                    i += 2 + ch.len_utf8();
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    mask
}

/// Every `cfg` predicate written as code inside `body`, as `#[cfg(...)]` or
/// `cfg!(...)`. `mask` is [`code_mask`] of `body`.
fn body_cfgs(body: &str, mask: &[bool]) -> Vec<Cfg> {
    let mut out = Vec::new();
    for prefix in ["#[cfg(", "cfg!("] {
        let mut at = 0;
        while let Some(found) = body[at..].find(prefix) {
            let from = at + found;
            if mask[from]
                && let Some(pred) = balanced_after(&body[from..], prefix)
            {
                out.push(parse_cfg(pred));
            }
            at = from + prefix.len();
        }
    }
    out
}

fn fn_name(line: &str) -> Option<String> {
    let mut t = line.trim_start();
    for q in ["pub(crate) ", "pub ", "async ", "unsafe "] {
        t = t.strip_prefix(q).unwrap_or(t);
    }
    let rest = t.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    Some(rest[..end].to_string())
}

/// Every `#[test]` in one source file.
fn scan_suite(suite: &str, src: &str) -> Vec<TestFn> {
    let mask = code_mask(src);
    let lines: Vec<&str> = src.split('\n').collect();
    // Byte offset of each line's start.
    let mut starts = Vec::with_capacity(lines.len());
    let mut off = 0;
    for l in &lines {
        starts.push(off);
        off += l.len() + 1;
    }
    // Whether line `n` starts with code, as opposed to being blank, a comment
    // or the inside of a string literal.
    let is_code = |n: usize| {
        let l = lines[n];
        let indent = l.len() - l.trim_start().len();
        !l.trim().is_empty() && mask[starts[n] + indent]
    };
    let mut file_cfg = Vec::new();
    let mut out = Vec::new();
    let mut attrs: Vec<String> = Vec::new();
    // Open inline modules: the line that closes each, and its cfgs.
    let mut modules: Vec<(String, Vec<Cfg>)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        if !is_code(i) {
            i += 1;
            continue;
        }
        if t.starts_with("#![cfg(") {
            file_cfg.push(parse_cfg(
                balanced_after(t, "#![cfg(").expect("unbalanced #![cfg("),
            ));
            i += 1;
            continue;
        }
        if t.starts_with("#[") {
            // An attribute can run over several lines (rustfmt splits a long
            // cfg_attr), so collect until its brackets balance.
            let mut text = String::new();
            let mut depth: i32 = 0;
            loop {
                let l = lines[i];
                text.push_str(l.trim());
                text.push(' ');
                let mut in_str = false;
                for ch in l.chars() {
                    match ch {
                        '"' => in_str = !in_str,
                        '[' if !in_str => depth += 1,
                        ']' if !in_str => depth -= 1,
                        _ => {}
                    }
                }
                i += 1;
                if depth <= 0 || i >= lines.len() {
                    break;
                }
            }
            attrs.push(text.trim().to_string());
            continue;
        }
        if let Some(name) = fn_name(line) {
            let is_test = attrs.iter().any(|a| a == "#[test]");
            if is_test {
                let indent = line.len() - t.len();
                let close = format!("{}}}", " ".repeat(indent));
                // `fn x() {}` closes on its own line.
                let (mut opens, mut closes) = (0, 0);
                for (k, ch) in line.bytes().enumerate() {
                    if mask[starts[i] + k] {
                        opens += usize::from(ch == b'{');
                        closes += usize::from(ch == b'}');
                    }
                }
                let one_line = opens > 0 && opens == closes;
                let end = (i..lines.len())
                    .find(|&n| (one_line && n == i) || (lines[n] == close && is_code(n)))
                    .unwrap_or_else(|| {
                        panic!(
                            "tests/{suite}.rs: cannot find the closing brace of `fn {name}` at \
                             its own indentation. This guard relies on rustfmt layout."
                        )
                    });
                let mut compile = file_cfg.clone();
                compile.extend(modules.iter().flat_map(|(_, c)| c.iter().cloned()));
                let mut ignore_when = Vec::new();
                let mut always_ignored = false;
                for a in &attrs {
                    attribute_cfgs(a, &mut compile, &mut ignore_when, &mut always_ignored);
                }
                out.push(TestFn {
                    suite: suite.to_string(),
                    name,
                    compile,
                    ignore_when,
                    always_ignored,
                    body: {
                        let (from, to) = (starts[i], starts[end] + lines[end].len());
                        body_cfgs(&src[from..to], &mask[from..to])
                    },
                });
            }
            attrs.clear();
            i += 1;
            continue;
        }
        // An inline module: its `cfg`s hold for everything inside it, until
        // the `}` at its own indentation.
        if (t.starts_with("mod ") || t.starts_with("pub mod ")) && t.ends_with('{') {
            let mut cfgs = Vec::new();
            let (mut ignore_when, mut always) = (Vec::new(), false);
            for a in &attrs {
                attribute_cfgs(a, &mut cfgs, &mut ignore_when, &mut always);
            }
            assert!(
                ignore_when.is_empty() && !always,
                "tests/{suite}.rs puts an ignore attribute on a module ({attrs:?} {t:?}), \
                 which this scanner does not follow"
            );
            modules.push((format!("{}}}", " ".repeat(line.len() - t.len())), cfgs));
            attrs.clear();
            i += 1;
            continue;
        }
        if modules.last().is_some_and(|(close, _)| line == close) {
            modules.pop();
        }
        attrs.clear();
        i += 1;
    }
    out
}

/// The `tests/*.rs` binaries, by name.
fn suites() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(repo_root().join("tests"))
        .expect("read tests/")
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.ends_with(".rs"))
        .map(|n| n.trim_end_matches(".rs").to_string())
        .collect();
    names.sort();
    names
}

/// Whether a suite drives the CLI and returns early without it, which is the
/// definition `tests/pmtiles_ci_wiring.rs` uses too.
fn is_cli_suite(src: &str) -> bool {
    src.contains("cli_available(") || src.contains("cli_source_available(")
}

// ---------------------------------------------------------------------------
// Cells
// ---------------------------------------------------------------------------

/// One `cargo test` invocation CI makes, and what it runs.
#[derive(Debug, Clone)]
struct Cell {
    /// Where it came from, for the failure message.
    origin: String,
    features: BTreeSet<String>,
    /// `--test` targets; empty means every integration target.
    targets: Vec<String>,
    /// Positional filters after `--`.
    filters: Vec<String>,
    skips: Vec<String>,
    exact: bool,
    ignored_only: bool,
    include_ignored: bool,
    runs_tests: bool,
    require_cli: bool,
}

/// Split a command line into arguments, honouring double quotes.
fn tokens(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut started) = (false, false);
    for ch in cmd.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            c => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

/// This crate's `[features]` table, for following `s3 -> object-store-sink`.
fn feature_table() -> Vec<manifest::Feature> {
    manifest::features(&read("Cargo.toml"))
}

/// Close `start` over the local implications in `table`.
fn resolve(start: BTreeSet<String>, table: &[manifest::Feature]) -> BTreeSet<String> {
    let mut set = start;
    loop {
        let mut grew = false;
        for f in table {
            if set.contains(&f.name) {
                for e in &f.enables {
                    if !e.contains('/') && !e.starts_with("dep:") && set.insert(e.clone()) {
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            return set;
        }
    }
}

fn parse_cell(
    origin: String,
    cmd: &str,
    require_cli: bool,
    table: &[manifest::Feature],
) -> Option<Cell> {
    let toks = tokens(cmd);
    if toks.first().map(String::as_str) != Some("cargo")
        || toks.get(1).map(String::as_str) != Some("test")
    {
        return None;
    }
    let mut features = BTreeSet::new();
    let mut default = true;
    let mut cell = Cell {
        origin,
        features: BTreeSet::new(),
        targets: Vec::new(),
        filters: Vec::new(),
        skips: Vec::new(),
        exact: false,
        ignored_only: false,
        include_ignored: false,
        runs_tests: true,
        require_cli,
    };
    let mut it = toks.iter().skip(2);
    let mut harness = false;
    while let Some(tok) = it.next() {
        if harness {
            match tok.as_str() {
                "--exact" => cell.exact = true,
                "--ignored" => cell.ignored_only = true,
                "--include-ignored" => cell.include_ignored = true,
                "--skip" => cell.skips.extend(it.next().cloned()),
                "--test-threads" | "--format" | "--color" => {
                    it.next();
                }
                t if t.starts_with("--") => {}
                t => cell.filters.push(t.to_string()),
            }
            continue;
        }
        match tok.as_str() {
            "--features" | "-F" => {
                if let Some(list) = it.next() {
                    features.extend(
                        list.split([' ', ','])
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    );
                }
            }
            "--no-default-features" => default = false,
            "--all-features" => features.extend(table.iter().map(|f| f.name.clone())),
            "--test" => cell.targets.extend(it.next().cloned()),
            "--no-run" => cell.runs_tests = false,
            "--" => harness = true,
            t => {
                if let Some(list) = t.strip_prefix("--features=") {
                    features.extend(
                        list.split([' ', ','])
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                    );
                } else if let Some(name) = t.strip_prefix("--test=") {
                    cell.targets.push(name.to_string());
                }
            }
        }
    }
    if default {
        features.insert("default".to_string());
    }
    cell.features = resolve(features, table);
    Some(cell)
}

/// The bash array `NAME=( ... )` in `script`, one element per line.
fn bash_array(script: &str, name: &str) -> Vec<String> {
    let open = format!("{name}=(");
    let mut out = Vec::new();
    let mut inside = false;
    for line in script.lines() {
        let t = line.trim();
        if !inside {
            if t == open {
                inside = true;
            }
            continue;
        }
        if t == ")" {
            return out;
        }
        if !t.is_empty() && !t.starts_with('#') {
            out.push(t.to_string());
        }
    }
    panic!("tools/run_ported_cells.sh has no `{name}=(` array, which this guard expands");
}

fn bash_scalar(script: &str, name: &str) -> String {
    let prefix = format!("{name}=");
    script
        .lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix(&prefix))
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| panic!("tools/run_ported_cells.sh has no `{name}=` line"))
}

/// Every `cargo test` CI runs: each step of each job of `ci.yml`, plus the
/// lines of `tools/run_ported_cells.sh`, which the `ported-tests` job runs.
fn ci_cells(ci: &Workflow) -> Vec<Cell> {
    let table = feature_table();
    let mut cells = Vec::new();
    let mut ported_job_runs_script = false;
    for (job, step) in ci.steps() {
        if job.key("if").is_some_and(|v| v.trim() == "false")
            || job.key("continue-on-error") == Some("true")
            || step.key("continue-on-error") == Some("true")
            || step.key("if").is_some_and(|v| v.trim() == "false")
        {
            continue;
        }
        let require_cli = job.env_for(step, "VIPRS_REQUIRE_CLI") == Some("1");
        for (n, line) in step.run_code().lines().enumerate() {
            let line = line.trim();
            if line.starts_with("./tools/run_ported_cells.sh") && !line.contains("--clippy") {
                ported_job_runs_script = true;
            }
            if let Some(cell) = parse_cell(
                format!("ci.yml job `{}`, line {}: {line}", job.id, n + 1),
                line,
                require_cli,
                &table,
            ) {
                cells.push(cell);
            }
        }
    }

    if ported_job_runs_script {
        let script = read("tools/run_ported_cells.sh");
        let mut parallel = vec!["--test fixture_audit".to_string()];
        parallel.extend(
            bash_array(&script, "PARALLEL_CELLS")
                .iter()
                .map(|c| format!("--test {c}")),
        );
        let serial = bash_scalar(&script, "SERIAL_CELL");
        for (n, line) in script.lines().enumerate() {
            let line = line.trim();
            if !line.starts_with("cargo test") {
                continue;
            }
            let expanded = line
                .replace("\"${PARALLEL_TARGETS[@]}\"", &parallel.join(" "))
                .replace("\"$SERIAL_CELL\"", &serial);
            if let Some(cell) = parse_cell(
                format!("tools/run_ported_cells.sh line {}: {expanded}", n + 1),
                &expanded,
                false,
                &table,
            ) {
                cells.push(cell);
            }
        }
    }
    cells
}

/// Whether `cell` runs `test` with every condition in `extra` true.
fn runs(cell: &Cell, test: &TestFn, cli: bool, extra: Option<&Cfg>) -> bool {
    let f = &cell.features;
    let ignored = test.always_ignored || test.ignore_when.iter().any(|c| c.eval(f));
    let selected_by_ignore = if cell.ignored_only {
        ignored
    } else {
        cell.include_ignored || !ignored
    };
    let name_ok = (cell.filters.is_empty()
        || cell.filters.iter().any(|flt| {
            if cell.exact {
                test.name == *flt
            } else {
                test.name.contains(flt.as_str())
            }
        }))
        && !cell.skips.iter().any(|s| test.name.contains(s.as_str()));
    cell.runs_tests
        && (cell.targets.is_empty() || cell.targets.contains(&test.suite))
        && test.compile.iter().all(|c| c.eval(f))
        && selected_by_ignore
        && name_ok
        && (!cli || cell.require_cli)
        && extra.is_none_or(|c| c.eval(f))
}

/// Feature-gated tests that no CI cell runs on purpose, with the reason.
///
/// Empty, and kept honest: a row for a test that does not exist, or that a
/// cell does run after all, is a failure, so it cannot turn into a list of
/// excuses nobody re-reads.
const GATED_TESTS_NOT_IN_CI: &[(&str, &str, &str)] = &[];

/// Every feature-gated test has a CI cell that runs it, and every
/// feature-conditional half of its body runs in some cell too.
#[test]
fn every_feature_gated_test_runs_in_a_ci_cell() {
    let ci = Workflow::parse(&read_workflow("ci.yml"));
    let cells = ci_cells(&ci);
    assert!(
        cells.len() >= 10,
        "found only {} cargo test cells in ci.yml and tools/run_ported_cells.sh, \
         which cannot be right: {cells:#?}",
        cells.len()
    );

    let mut gated = 0usize;
    let mut problems = Vec::new();
    for suite in suites() {
        let src = read(&format!("tests/{suite}.rs"));
        let cli = is_cli_suite(&src);
        for test in scan_suite(&suite, &src) {
            if test.always_ignored || !test.is_feature_gated() {
                continue;
            }
            gated += 1;
            let excused = GATED_TESTS_NOT_IN_CI
                .iter()
                .any(|(s, t, _)| *s == test.suite && *t == test.name);
            let mut missing = Vec::new();
            if !cells.iter().any(|c| runs(c, &test, cli, None)) {
                missing.push("no cell runs it at all".to_string());
            }
            for b in test.body.iter().filter(|b| b.mentions_a_feature()) {
                if !cells.iter().any(|c| runs(c, &test, cli, Some(b))) {
                    missing.push(format!("no cell that runs it has `{b:?}` true"));
                }
            }
            match (missing.is_empty(), excused) {
                (true, true) => problems.push(format!(
                    "{}::{} is in GATED_TESTS_NOT_IN_CI and a CI cell runs it. Drop the row.",
                    test.suite, test.name
                )),
                (false, false) => problems.push(format!(
                    "{}::{} (compiled when {:?}, ignored when {:?}{}): {}. Cells naming the \
                     suite: {:?}",
                    test.suite,
                    test.name,
                    test.compile,
                    test.ignore_when,
                    if cli {
                        ", CLI-driven so the cell needs VIPRS_REQUIRE_CLI=1"
                    } else {
                        ""
                    },
                    missing.join("; "),
                    cells
                        .iter()
                        .filter(|c| c.targets.contains(&test.suite))
                        .map(|c| c.origin.as_str())
                        .collect::<Vec<_>>()
                )),
                _ => {}
            }
        }
    }
    for (suite, name, why) in GATED_TESTS_NOT_IN_CI {
        let exists = repo_root().join(format!("tests/{suite}.rs")).is_file()
            && scan_suite(suite, &read(&format!("tests/{suite}.rs")))
                .iter()
                .any(|t| t.name == *name);
        if !exists {
            problems.push(format!(
                "GATED_TESTS_NOT_IN_CI names {suite}::{name} ({why}), which does not exist"
            ));
        }
        if why.trim().is_empty() {
            problems.push(format!(
                "GATED_TESTS_NOT_IN_CI names {suite}::{name} with no reason"
            ));
        }
    }

    assert!(
        gated >= 20,
        "the scanner found only {gated} feature-gated tests, which cannot be right; it has \
         to fail loudly rather than pass over a tree it stopped reading"
    );
    assert!(
        problems.is_empty(),
        "these feature-gated tests run in no CI cell that can run them, so in CI they compile \
         out, report `ignored` or skip, and read as a pass:\n  {}\n\nAdd the suite to a \
         `cargo test` step that enables the feature (and sees VIPRS_REQUIRE_CLI=1 if it \
         drives the CLI), or add a row to GATED_TESTS_NOT_IN_CI saying why it runs nowhere.",
        problems.join("\n  ")
    );
}

/// The scanner reads the shapes this repo uses, and the cell matcher refuses
/// the near misses. Without these the guard above can pass by having stopped
/// seeing things.
#[test]
fn the_scanner_and_the_matcher_see_what_is_there() {
    let src = r#"
#![cfg(feature = "packfile")]

#[test]
#[cfg_attr(
    not(feature = "jxl"),
    ignore = "needs jxl"
)]
fn gated_by_ignore() {
    #[cfg(feature = "jxl")]
    {
        let _ = 1;
    }
    if cfg!(feature = "svg") {}
}

/// docs
#[cfg(all(unix, feature = "tracing"))]
#[test]
fn gated_by_cfg() {}

#[test]
#[ignore = "parked"]
fn parked() {}

fn helper() {}

mod inner {
    #[test]
    fn nested() {}
}

#[cfg(feature = "tracing")]
mod gated {
    #[test]
    fn inside() {}
}
"#;
    let tests = scan_suite("demo", src);
    let names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "gated_by_ignore",
            "gated_by_cfg",
            "parked",
            "nested",
            "inside"
        ]
    );
    let t = &tests[0];
    assert_eq!(t.compile, [Cfg::Feature("packfile".into())]);
    assert_eq!(
        t.ignore_when,
        [Cfg::Not(Box::new(Cfg::Feature("jxl".into())))]
    );
    assert_eq!(
        t.body,
        [Cfg::Feature("jxl".into()), Cfg::Feature("svg".into())]
    );
    assert!(tests[2].always_ignored);
    assert_eq!(tests[3].compile, [Cfg::Feature("packfile".into())]);
    assert_eq!(
        tests[4].compile,
        [
            Cfg::Feature("packfile".into()),
            Cfg::Feature("tracing".into())
        ]
    );
    assert_eq!(
        tests[1].compile[1],
        Cfg::All(vec![Cfg::Fixed(true), Cfg::Feature("tracing".into())])
    );

    // Text that quotes Rust is not Rust: a cfg in a message, and a whole fake
    // test in a raw string, col-0 brace included.
    let quoting = "#[test]\nfn real() {\n    let _ = \"#[cfg(feature = \\\"x\\\")]\";\n    let _ = '\"';\n    let _ = r#\"\n#[test]\n#[cfg(feature = \"y\")]\nfn fake() {}\n}\n\"#;\n}\n";
    let quoted = scan_suite("quoting", quoting);
    assert_eq!(quoted.len(), 1, "{quoted:?}");
    assert_eq!(quoted[0].name, "real");
    assert!(
        quoted[0].body.is_empty() && quoted[0].compile.is_empty(),
        "{quoted:?}"
    );

    let table = feature_table();
    let cell = |cmd: &str, cli: bool| parse_cell("demo".into(), cmd, cli, &table).unwrap();
    assert_eq!(cell("cargo test --features jxl", false).origin, "demo");
    let gated = &tests[0];
    // Runs: the target, both features, the body feature only with svg too.
    assert!(runs(
        &cell("cargo test --features \"packfile jxl\" --test demo", false),
        gated,
        false,
        None
    ));
    assert!(!runs(
        &cell("cargo test --features \"packfile jxl\" --test demo", false),
        gated,
        false,
        Some(&Cfg::Feature("svg".into()))
    ));
    // Ignored without jxl, compiled out without packfile, another target, no run.
    assert!(!runs(
        &cell("cargo test --features packfile --test demo", false),
        gated,
        false,
        None
    ));
    assert!(!runs(
        &cell("cargo test --features jxl --test demo", false),
        gated,
        false,
        None
    ));
    assert!(!runs(
        &cell("cargo test --features \"packfile jxl\" --test other", false),
        gated,
        false,
        None
    ));
    assert!(!runs(
        &cell(
            "cargo test --features \"packfile jxl\" --test demo --no-run",
            false
        ),
        gated,
        false,
        None
    ));
    // A filter has to let it through, a skip must not stop it.
    assert!(runs(
        &cell(
            "cargo test --features \"packfile jxl\" --test demo -- by_ignore",
            false
        ),
        gated,
        false,
        None
    ));
    assert!(!runs(
        &cell(
            "cargo test --features \"packfile jxl\" --test demo -- unrelated",
            false
        ),
        gated,
        false,
        None
    ));
    assert!(!runs(
        &cell(
            "cargo test --features \"packfile jxl\" --test demo -- --skip gated",
            false
        ),
        gated,
        false,
        None
    ));
    // A CLI suite needs the require variable on the cell.
    assert!(!runs(
        &cell("cargo test --features \"packfile jxl\" --test demo", false),
        gated,
        true,
        None
    ));
    assert!(runs(
        &cell("cargo test --features \"packfile jxl\" --test demo", true),
        gated,
        true,
        None
    ));
    // `--ignored` runs only what is ignored there.
    assert!(!runs(
        &cell(
            "cargo test --features \"packfile jxl\" --test demo -- --ignored",
            false
        ),
        gated,
        false,
        None
    ));
    // The crate's own implications: s3 turns on object-store-sink.
    assert!(
        cell("cargo test --features s3", false)
            .features
            .contains("object-store-sink")
    );
    assert!(
        !cell("cargo test --no-default-features", false)
            .features
            .contains("test-util")
    );
    assert!(cell("cargo test", false).features.contains("test-util"));
}

// ---------------------------------------------------------------------------
// The local gate replays the same cells
// ---------------------------------------------------------------------------

/// `ci.yml` jobs the local gate does not replay, and why. Every other job is
/// replayed by `tools/replay-ci.sh`, step for step.
const JOBS_NOT_REPLAYED: &[(&str, &str)] = &[
    (
        "feature-cells",
        "a matrix of `cargo clippy --features <f>` lint cells that runs no test, so \
         leaving it out cannot let a suite skip as a pass; its matrix is held to \
         Cargo.toml by tests/ci_feature_coverage.rs, and the gate's clippy stages \
         lint the same feature names",
    ),
    (
        "hook-mirror",
        "it reads four unpinned sibling repos (libviprs-bench, libviprs-org, \
         libviprs-dep, pdfium-render) at whatever their default branch is today, so \
         its verdict is about those repos rather than the tree under test, and the \
         local gate only lays down the three trees this suite pins",
    ),
];

/// The `uses:` actions a replayed job may contain. Each one is something the
/// local gate already provides (the checkout and the two pinned siblings, a
/// toolchain, nothing for a cache). A new action is new behaviour the replay
/// does not have, so it has to be looked at rather than skipped.
const USES_THE_GATE_PROVIDES: &[&str] = &[
    "actions/checkout@",
    "./.github/actions/clone-counterpart",
    "./.github/actions/clone-cli-counterpart",
    "dtolnay/rust-toolchain@",
    "Swatinem/rust-cache@",
];

/// One step, reduced to what decides what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayStep {
    env: Vec<(String, String)>,
    code: Vec<String>,
}

/// The code lines of a script body the way `Step::run_code` reads a `run:`:
/// comment-only lines dropped, continuations folded, each line trimmed.
fn code_lines(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = String::new();
    for line in body.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let cont = line.ends_with('\\');
        pending.push_str(line.strip_suffix('\\').unwrap_or(line));
        if cont {
            continue;
        }
        let done = pending.trim().to_string();
        if !done.is_empty() {
            out.push(done);
        }
        pending.clear();
    }
    if !pending.trim().is_empty() {
        out.push(pending.trim().to_string());
    }
    out
}

/// The jobs `tools/replay-ci.sh` replays, in order, each with its steps.
///
/// The script's shape is `job <id>` to start a job, and
/// `step [NAME=VALUE ...] <<'STEP'` ... `STEP` for each step.
fn replayed_jobs(script: &str) -> Vec<(String, Vec<ReplayStep>)> {
    let mut jobs: Vec<(String, Vec<ReplayStep>)> = Vec::new();
    let lines: Vec<&str> = script.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(id) = t.strip_prefix("job ") {
            jobs.push((id.trim().to_string(), Vec::new()));
        } else if let Some(rest) = t.strip_prefix("step") {
            if let Some(args) = rest.strip_suffix("<<'STEP'") {
                let mut env: Vec<(String, String)> = tokens(args)
                    .iter()
                    .map(|kv| {
                        let (k, v) = kv.split_once('=').unwrap_or_else(|| {
                            panic!(
                                "tools/replay-ci.sh line {}: `{kv}` is not NAME=VALUE",
                                i + 1
                            )
                        });
                        (k.to_string(), v.to_string())
                    })
                    .collect();
                env.sort();
                let end = lines[i + 1..]
                    .iter()
                    .position(|l| *l == "STEP")
                    .map(|p| i + 1 + p)
                    .unwrap_or_else(|| {
                        panic!(
                            "tools/replay-ci.sh line {}: a step with no closing STEP",
                            i + 1
                        )
                    });
                let code = code_lines(&lines[i + 1..end].join("\n"));
                let (_, steps) = jobs.last_mut().unwrap_or_else(|| {
                    panic!("tools/replay-ci.sh line {}: a step before any `job`", i + 1)
                });
                steps.push(ReplayStep { env, code });
                i = end;
            }
        }
        i += 1;
    }
    jobs
}

/// The workflow-wide `env:` block, which `Workflow` does not keep.
fn workflow_env(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line == "env:" {
            inside = true;
            continue;
        }
        if inside {
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            if !line.starts_with(' ') {
                break;
            }
            let (k, v) = line.trim().split_once(':').expect("env entry");
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// The local gate replays exactly the jobs, steps, environments and code that
/// `ci.yml` runs, except the jobs [`JOBS_NOT_REPLAYED`] names.
#[test]
fn the_local_gate_replays_every_ci_cell() {
    let text = read_workflow("ci.yml");
    let ci = Workflow::parse(&text);
    let script_path = repo_root().join("tools/replay-ci.sh");
    assert!(
        script_path.is_file(),
        "tools/replay-ci.sh is missing, so the local gate has no copy of ci.yml's cells to \
         run and a local green says nothing about what CI would say"
    );
    let script = read("tools/replay-ci.sh");
    let replayed = replayed_jobs(&script);

    let mut problems = Vec::new();

    for (k, v) in workflow_env(&text) {
        if !script
            .lines()
            .any(|l| l.trim() == format!("export {k}={v}"))
        {
            problems.push(format!(
                "ci.yml sets `{k}: {v}` for every job and tools/replay-ci.sh has no `export {k}={v}`"
            ));
        }
    }

    // The script hides the CLI sibling from every job whose CI version does not
    // lay it down, so its CLI cells skip locally exactly where they skip in CI.
    // The list it does that by has to be the list ci.yml implies.
    let declared: BTreeSet<String> = script
        .lines()
        .find_map(|l| l.trim().strip_prefix("JOBS_WITH_CLI_SIBLING="))
        .map(|v| {
            v.trim_matches('"')
                .split_whitespace()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let implied: BTreeSet<String> = ci
        .jobs()
        .iter()
        .filter(|j| replayed.iter().any(|(id, _)| *id == j.id))
        .filter(|j| {
            j.steps
                .iter()
                .any(|s| s.key("uses") == Some("./.github/actions/clone-cli-counterpart"))
        })
        .map(|j| j.id.clone())
        .collect();
    if declared != implied {
        problems.push(format!(
            "tools/replay-ci.sh's JOBS_WITH_CLI_SIBLING is {declared:?}, and the replayed ci.yml \
             jobs that lay the CLI sibling down are {implied:?}"
        ));
    }

    for (id, why) in JOBS_NOT_REPLAYED {
        if !ci.jobs().iter().any(|j| j.id == *id) {
            problems.push(format!(
                "JOBS_NOT_REPLAYED names `{id}` ({why}), and ci.yml has no such job"
            ));
        }
        if replayed.iter().any(|(j, _)| j == id) {
            problems.push(format!(
                "JOBS_NOT_REPLAYED names `{id}` and tools/replay-ci.sh replays it. Drop the row."
            ));
        }
    }

    for (id, _) in &replayed {
        if !ci.jobs().iter().any(|j| j.id == *id) {
            problems.push(format!(
                "tools/replay-ci.sh replays a job `{id}` that ci.yml does not have"
            ));
        }
        if replayed.iter().filter(|(j, _)| j == id).count() > 1 {
            problems.push(format!("tools/replay-ci.sh replays `{id}` more than once"));
        }
    }

    for job in ci.jobs() {
        if JOBS_NOT_REPLAYED.iter().any(|(id, _)| *id == job.id) {
            continue;
        }
        let Some((_, local)) = replayed.iter().find(|(j, _)| *j == job.id) else {
            problems.push(format!(
                "ci.yml job `{}` is not replayed by tools/replay-ci.sh and JOBS_NOT_REPLAYED does \
                 not say why, so the local gate never runs its cells",
                job.id
            ));
            continue;
        };
        let mut remote = Vec::new();
        for step in &job.steps {
            if let Some(action) = step.key("uses") {
                if !USES_THE_GATE_PROVIDES.iter().any(|u| action.starts_with(u)) {
                    problems.push(format!(
                        "ci.yml job `{}` uses `{action}`, which the local gate does not provide. \
                         Replay what it does in tools/replay-ci.sh and list it in \
                         USES_THE_GATE_PROVIDES, or move the job to JOBS_NOT_REPLAYED.",
                        job.id
                    ));
                }
                continue;
            }
            for key in ["if", "continue-on-error", "shell", "working-directory"] {
                if step.key(key).is_some() {
                    problems.push(format!(
                        "a step of ci.yml job `{}` sets `{key}`, which tools/replay-ci.sh does not \
                         reproduce: {:?}",
                        job.id,
                        step.run()
                    ));
                }
            }
            let mut env: Vec<(String, String)> = job.env.clone();
            for (k, v) in &step.env {
                env.retain(|(jk, _)| jk != k);
                env.push((k.clone(), v.clone()));
            }
            env.sort();
            let code = code_lines(&step.run_code());
            if code.iter().any(|l| l.contains("${{")) || env.iter().any(|(_, v)| v.contains("${{"))
            {
                problems.push(format!(
                    "a step of ci.yml job `{}` uses a `${{{{ }}}}` expression, which a shell replay \
                     cannot evaluate: {code:?}",
                    job.id
                ));
            }
            remote.push(ReplayStep { env, code });
        }
        if remote != *local {
            problems.push(format!(
                "ci.yml job `{}` and its replay in tools/replay-ci.sh differ.\n    ci.yml:\n      {}\n    replay:\n      {}",
                job.id,
                remote.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join("\n      "),
                local.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join("\n      "),
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "the local gate and CI do not run the same cells:\n  {}",
        problems.join("\n  ")
    );
}

/// The parity reader sees a script's jobs and steps, and would see a change.
#[test]
fn the_replay_reader_sees_what_is_there() {
    let script = "\
export RUSTFLAGS=-Dwarnings
job test
step <<'STEP'
cargo test
STEP
step VIPRS_REQUIRE_CLI=1 CODEC=all <<'STEP'
# a comment
cargo test --features \"jxl avif\" \\
  --test codec_e2e
STEP
job lint
step <<'STEP'
cargo fmt -- --check
STEP
";
    let jobs = replayed_jobs(script);
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].0, "test");
    assert_eq!(jobs[0].1.len(), 2);
    assert_eq!(
        jobs[0].1[1],
        ReplayStep {
            env: vec![
                ("CODEC".into(), "all".into()),
                ("VIPRS_REQUIRE_CLI".into(), "1".into())
            ],
            code: vec!["cargo test --features \"jxl avif\"   --test codec_e2e".into()],
        }
    );
    assert_eq!(jobs[1].1[0].code, ["cargo fmt -- --check"]);
    assert_eq!(
        workflow_env("on:\n  push:\nenv:\n  A: x\n  B: -Dwarnings\n\njobs:\n"),
        [
            ("A".to_string(), "x".to_string()),
            ("B".to_string(), "-Dwarnings".to_string())
        ]
    );
}
