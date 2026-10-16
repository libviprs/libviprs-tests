//! The `pdfium-static` cell compiles `tests/page_size_libvips_pixel_parity.rs`
//! clean, and its big-sheet test checks what its leftover bindings were for
//! (#275).
//!
//! # The two halves of the bug
//!
//! `cargo clippy --all-targets --features pdfium-static -- -D warnings`, the
//! `feature-cells` cell in ci.yml, is red on main on three rustc lints in
//! `page_size_libvips_pixel_parity.rs`: `LandmarkPoints` imported and never
//! named, and `t` and `pt` bound in
//! `big_sheets_match_the_on_demand_libvips_reference` and never read. Being
//! rustc lints, they fail every cell that compiles the file with warnings
//! denied, which includes the `test-pdfium` job's `cargo test --features pdfium`
//! under the workflow-wide `RUSTFLAGS: -Dwarnings`.
//!
//! The three names are not dead code. In #268 `pt` (the `landmark_points(..)`
//! result, where the page geometry says the title-block square and the corner
//! triangle land) drove the landmark probe of both rasters, and `t` timed each
//! sheet for a `BIG {label} ..` report line. #270 moved the probe into
//! `common::page_probe::compare_big`, which works the points out again itself,
//! and dropped the report line from this suite but not from the CLI twin. What
//! the suite lost on the way is the one check the big sheets get nowhere else:
//! `pdfium_page_size_exact.rs` holds every sheet it renders to the formula
//! (square midpoint within a pixel of where `landmark_points` puts it) but caps
//! renders at 24 Mpx, and the on-demand big sheets all sit above that cap. So
//! `compare_big` saying libviprs' landmarks match libvips' is the whole of the
//! landmark evidence for them, and it holds just as well when both rasters are
//! a pixel off the formula together.
//!
//! # What this file checks
//!
//! [`the_pdfium_static_cell_compiles_the_parity_suite_clean_275`] replays the
//! cell for the one target, `cargo check --features pdfium-static --test
//! page_size_libvips_pixel_parity`, with `RUSTFLAGS` cleared the way the cell
//! clears it, reads the compiler messages as JSON, and fails on any warning or
//! error whose span is in that file. `check` rather than `clippy`: the three
//! lints are rustc's, so `check` sees them and needs no clippy component.
//!
//! The other two pin the behaviour the bindings were for, by name of what they
//! bind rather than by the variables' names, so a rename passes and a deletion
//! or an underscore does not:
//!
//! * [`the_big_sheet_test_holds_the_landmarks_to_the_formula_275`]: at least one
//!   `let` in the test binds a `landmark_points(..)` result, and every such
//!   binding is read by an assertion or a `failures.push(..)`, directly or
//!   through one intermediate `let`.
//! * [`the_big_sheet_test_reports_each_sheet_275`]: an `Instant::now()` is
//!   bound inside the `for` loop over the sheets, and every `Instant::now()`
//!   binding in the test is read by a printing macro or an assertion.
//!
//! `let _pt = ..` binds nothing a later statement can read, `let _ = pt;` reads
//! it in a statement that asserts nothing, and deleting the binding leaves no
//! `landmark_points(..)` result to be checked. All three stay red, and the
//! controls at the bottom pin each of those shapes along with the two that are
//! meant to pass.
//!
//! # How the source is read
//!
//! The scanner relies on rustfmt layout and on a small masking pass that blanks
//! string literals and comments (the suite's format strings are full of braces),
//! then splits the function body into statements by `;` and block, recursively.
//! Anything it cannot find, the function or the bindings it expects, is a panic
//! naming what it looked for, never a silent pass.
//!
//! # Cost
//!
//! The compile half builds the core and `pdfium-render` under `pdfium-static` in
//! its own target directory under `CARGO_TARGET_TMPDIR`, so it neither waits on
//! the outer build's lock nor disturbs its artifacts, and `Swatinem/rust-cache`
//! keeps it between CI runs. The structural half is a text read.

use std::path::{Path, PathBuf};
use std::process::Command;

const PARITY_SUITE: &str = "page_size_libvips_pixel_parity";
const PARITY_FILE: &str = "tests/page_size_libvips_pixel_parity.rs";
const BIG_SHEETS_TEST: &str = "big_sheets_match_the_on_demand_libvips_reference";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// The cell, replayed for one target
// ---------------------------------------------------------------------------

/// Run `cargo check <args>` in `dir` with `RUSTFLAGS` cleared, as the
/// pdfium-static cell runs, and return the rendered text of every warning and
/// error whose span lands in a file ending in `file`. A `cargo` that fails
/// without producing such a diagnostic is a panic with its stderr, so an
/// environment problem (the core not compiling, a missing dependency) reads as
/// what it is rather than as a clean file.
fn diagnostics_in(dir: &Path, args: &[&str], target_dir: &Path, file: &str) -> Vec<String> {
    let out = Command::new(cargo())
        .current_dir(dir)
        .arg("check")
        .args(args)
        .args(["--message-format", "json", "--target-dir"])
        .arg(target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn cargo check: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut hits = Vec::new();
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v["reason"] != "compiler-message" {
            continue;
        }
        let msg = &v["message"];
        let level = msg["level"].as_str().unwrap_or("");
        if level != "warning" && level != "error" {
            continue;
        }
        let in_file = msg["spans"].as_array().is_some_and(|spans| {
            spans
                .iter()
                .any(|s| s["file_name"].as_str().is_some_and(|f| f.ends_with(file)))
        });
        if in_file {
            hits.push(
                msg["rendered"]
                    .as_str()
                    .unwrap_or("(diagnostic without rendered text)")
                    .trim_end()
                    .to_owned(),
            );
        }
    }
    assert!(
        out.status.success() || !hits.is_empty(),
        "`cargo check {}` failed before reaching {file}:\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    hits
}

/// Where the replayed cell builds. Under `CARGO_TARGET_TMPDIR`, so it lives
/// beside the outer build's artifacts (and inside whatever `CARGO_TARGET_DIR`
/// points at) without sharing their lock.
fn cell_target_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("pdfium-static-cell-275")
}

#[test]
fn the_pdfium_static_cell_compiles_the_parity_suite_clean_275() {
    let hits = diagnostics_in(
        &repo_root(),
        &["--features", "pdfium-static", "--test", PARITY_SUITE],
        &cell_target_dir(),
        PARITY_FILE,
    );
    assert!(
        hits.is_empty(),
        "the pdfium-static cell is not clean on {PARITY_FILE} ({} diagnostics):\n\n{}",
        hits.len(),
        hits.join("\n\n")
    );
}

/// The harness has to be able to see a lint at all, or a clean result above
/// means nothing. A throwaway crate with one unused binding yields exactly one
/// warning in the file asked about.
#[test]
fn control_the_cell_harness_reports_an_unused_binding_275() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"probe\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\n",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        "pub fn f() {\n    let t = 1;\n}\n",
    )
    .unwrap();
    let hits = diagnostics_in(dir.path(), &[], &dir.path().join("target"), "src/lib.rs");
    assert_eq!(
        hits.len(),
        1,
        "expected the one unused-variable warning, got: {hits:#?}"
    );
    assert!(
        hits[0].contains("unused variable: `t`"),
        "unexpected diagnostic: {}",
        hits[0]
    );
}

// ---------------------------------------------------------------------------
// Reading the test body
// ---------------------------------------------------------------------------

/// `src` with every string literal, char literal and comment blanked to spaces,
/// same length, so brace matching and identifier searches see only code.
fn mask(src: &str) -> String {
    let b = src.as_bytes();
    let mut out: Vec<u8> = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                let end = src[i..].find('\n').map_or(b.len(), |n| i + n);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                let end = src[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
                blank(&mut out, i, end);
                i = end;
            }
            b'r' if i + 1 < b.len() && (b[i + 1] == b'"' || b[i + 1] == b'#') => {
                // Raw string: r"..", r#".."#, r##".."##.
                let mut hashes = 0;
                while i + 1 + hashes < b.len() && b[i + 1 + hashes] == b'#' {
                    hashes += 1;
                }
                if i + 1 + hashes < b.len() && b[i + 1 + hashes] == b'"' {
                    let close = format!("\"{}", "#".repeat(hashes));
                    let start = i + 2 + hashes;
                    let end = src[start..]
                        .find(&close)
                        .map_or(b.len(), |n| start + n + close.len());
                    blank(&mut out, i + 1, end);
                    i = end;
                } else {
                    i += 1;
                }
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let end = (j + 1).min(b.len());
                blank(&mut out, i + 1, end.saturating_sub(1));
                i = end;
            }
            b'\'' => {
                // A char literal is 'x' or '\x..' ; anything else ('a lifetime)
                // is left alone.
                let lit_end = if i + 2 < b.len() && b[i + 1] != b'\\' && b[i + 2] == b'\'' {
                    Some(i + 3)
                } else if i + 1 < b.len() && b[i + 1] == b'\\' {
                    src[i + 2..].find('\'').map(|n| i + 2 + n + 1)
                } else {
                    None
                };
                match lit_end {
                    Some(end) if end - i <= 12 => {
                        blank(&mut out, i + 1, end - 1);
                        i = end;
                    }
                    _ => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("masking only writes ASCII spaces over ASCII")
}

/// The body of `fn name(..) { .. }` in `src`, between its braces, as the
/// original text, plus the masked copy of the same range.
fn function_body(src: &str, name: &str) -> (String, String) {
    let masked = mask(src);
    let needle = format!("fn {name}(");
    let at = masked
        .find(&needle)
        .unwrap_or_else(|| panic!("no `{needle}..` in the file"));
    let open = at + masked[at..].find('{').expect("function without a body");
    let mut depth = 0usize;
    let mut close = None;
    for (k, c) in masked[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + k);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close.expect("unbalanced braces in the function body");
    (
        src[open + 1..close].to_owned(),
        masked[open + 1..close].to_owned(),
    )
}

/// One statement of a body: its text outside any block it owns, masked
/// (`head`) and as written (`text`), and the statements inside those blocks
/// (`children`). `if c { a; } else { b; }` is one statement with head
/// `if c  else` and children `a`, `b`; a plain `x;` has no children.
#[derive(Debug)]
struct Stmt {
    head: String,
    text: String,
    children: Vec<Stmt>,
}

/// Split a body into statements. A statement ends at a `;` outside any
/// bracket, or at the `}` that closes a block opened at the statement's own
/// level when nothing follows it on the line but whitespace (an `if`, `for`,
/// `match`, `while` or bare block). An expression statement that ends the body
/// without a `;` is kept too.
fn statements(orig: &str, masked: &str) -> Vec<Stmt> {
    let mb = masked.as_bytes();
    let ob = orig.as_bytes();
    let mut out = Vec::new();
    let mut head = String::new();
    let mut text: Vec<u8> = Vec::new();
    let mut children = Vec::new();
    let mut paren = 0i32;
    let mut i = 0usize;
    let flush =
        |out: &mut Vec<Stmt>, head: &mut String, text: &mut Vec<u8>, children: &mut Vec<Stmt>| {
            if !head.trim().is_empty() || !children.is_empty() {
                out.push(Stmt {
                    head: std::mem::take(head),
                    text: String::from_utf8_lossy(&std::mem::take(text)).into_owned(),
                    children: std::mem::take(children),
                });
            }
        };
    while i < mb.len() {
        match mb[i] {
            b'(' | b'[' => {
                paren += 1;
                head.push(mb[i] as char);
                text.push(ob[i]);
                i += 1;
            }
            b')' | b']' => {
                paren -= 1;
                head.push(mb[i] as char);
                text.push(ob[i]);
                i += 1;
            }
            b'{' if paren == 0 => {
                // A block at statement level: recurse, then see whether the
                // statement continues (`} else {`, `}.method()`, `} ;`).
                let mut depth = 0usize;
                let mut j = i;
                while j < mb.len() {
                    match mb[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                children.extend(statements(&orig[i + 1..j], &masked[i + 1..j]));
                i = j + 1;
                let rest_of_line = masked[i..].split('\n').next().unwrap_or("").trim();
                if rest_of_line.is_empty() {
                    flush(&mut out, &mut head, &mut text, &mut children);
                }
            }
            b'{' => {
                head.push('{');
                text.push(b'{');
                i += 1;
            }
            b';' if paren == 0 => {
                head.push(';');
                text.push(b';');
                flush(&mut out, &mut head, &mut text, &mut children);
                i += 1;
            }
            c => {
                head.push(c as char);
                text.push(ob[i]);
                i += 1;
            }
        }
    }
    flush(&mut out, &mut head, &mut text, &mut children);
    out
}

/// Every statement in the tree, depth first.
fn walk<'a>(stmts: &'a [Stmt], into: &mut Vec<&'a Stmt>) {
    for s in stmts {
        into.push(s);
        walk(&s.children, into);
    }
}

/// Whether `text` uses `ident` as a whole identifier.
fn names(text: &str, ident: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(k) = text[from..].find(ident) {
        let at = from + k;
        let before = text[..at].chars().next_back();
        let after = text[at + ident.len()..].chars().next();
        if !before.is_some_and(is_ident) && !after.is_some_and(is_ident) {
            return true;
        }
        from = at + ident.len();
    }
    false
}

/// Whether the statement reads `ident`: as an identifier in its code, or as an
/// inline format argument (`{ident}`, `{ident:?}`) in one of its strings,
/// which rustc counts as a use too.
fn reads(s: &Stmt, ident: &str) -> bool {
    names(&s.head, ident)
        || s.text.contains(&format!("{{{ident}}}"))
        || s.text.contains(&format!("{{{ident}:"))
}

/// The names a `let` statement binds, or empty for anything else. `let (a, b)
/// = ..`, `let mut x = ..` and `let x: T = ..` all come back by name.
fn let_bindings(head: &str) -> Vec<String> {
    let t = head.trim_start();
    let Some(rest) = t.strip_prefix("let ") else {
        return Vec::new();
    };
    let pat = rest.split('=').next().unwrap_or("");
    let pat = pat.split(':').next().unwrap_or("");
    pat.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty() && *w != "mut" && *w != "_")
        .map(str::to_owned)
        .collect()
}

/// Whether the statement is one of the shapes that fails the test when it is
/// unhappy: an assertion, or a push onto the per-sheet `failures` list.
fn asserts(head: &str) -> bool {
    ["assert!(", "assert_eq!(", "assert_ne!(", "failures.push("]
        .iter()
        .any(|m| head.contains(m))
}

/// Whether the statement prints: the per-sheet report line.
fn prints(head: &str) -> bool {
    ["eprintln!(", "println!(", "eprint!(", "print!("]
        .iter()
        .any(|m| head.contains(m))
}

/// The bindings whose initialiser contains `init`, as `(name, statement
/// depth)`, with depth 0 for the body's own statements.
fn bindings_of(stmts: &[Stmt], init: &str) -> Vec<(String, usize)> {
    fn go(stmts: &[Stmt], init: &str, depth: usize, out: &mut Vec<(String, usize)>) {
        for s in stmts {
            if s.head.contains(init) {
                for name in let_bindings(&s.head) {
                    out.push((name, depth));
                }
            }
            go(&s.children, init, depth + 1, out);
        }
    }
    let mut out = Vec::new();
    go(stmts, init, 0, &mut out);
    out
}

/// Whether `name` is read by a statement satisfying `sink`, directly (the
/// statement's head reads it, or it is an `if`/`match` whose head reads it and
/// whose children contain such a statement), or through one `let` that reads
/// it and whose own bindings are read that way.
fn reaches(stmts: &[Stmt], name: &str, sink: &dyn Fn(&str) -> bool) -> bool {
    let mut all = Vec::new();
    walk(stmts, &mut all);
    let direct = |n: &str| {
        all.iter().any(|s| {
            // The binding itself names `n` on its left-hand side and reads
            // nothing.
            if !reads(s, n) || let_bindings(&s.head).iter().any(|b| b == n) {
                return false;
            }
            if sink(&s.head) {
                return true;
            }
            let mut inner = Vec::new();
            walk(&s.children, &mut inner);
            inner.iter().any(|c| sink(&c.head))
        })
    };
    if direct(name) {
        return true;
    }
    // One level of indirection: `let x = f(name);` then `x` reaching the sink.
    all.iter()
        .filter(|s| reads(s, name) && !let_bindings(&s.head).is_empty())
        .filter(|s| !let_bindings(&s.head).iter().any(|b| b == name))
        .flat_map(|s| let_bindings(&s.head))
        .any(|x| direct(&x))
}

/// Everything wrong with the big-sheet test body `src`, as far as the formula
/// check goes. Empty means it holds the landmarks to the formula.
fn formula_problems(body: &str, masked: &str) -> Vec<String> {
    let stmts = statements(body, masked);
    let bound = bindings_of(&stmts, "landmark_points(");
    if bound.is_empty() {
        return vec![
            "the test never asks `landmark_points(..)` where the landmarks should be, so \
             nothing can compare the probed extents with the formula (#275)"
                .to_owned(),
        ];
    }
    bound
        .iter()
        .filter(|(name, _)| !reaches(&stmts, name, &asserts))
        .map(|(name, _)| {
            format!(
                "`{name}` holds where the formula puts the landmarks and nothing asserts on it: \
                 compare the square's midpoint and the corner run from `compare_big` with it \
                 and push a miss onto `failures` (#275)"
            )
        })
        .collect()
}

/// Everything wrong with the big-sheet test body as far as the per-sheet
/// report goes.
fn report_problems(body: &str, masked: &str) -> Vec<String> {
    let stmts = statements(body, masked);
    let bound = bindings_of(&stmts, "Instant::now()");
    let mut problems = Vec::new();
    if !bound.iter().any(|(_, depth)| *depth > 0) {
        problems.push(
            "no `Instant::now()` is taken inside the loop over the sheets, so there is \
             nothing to report per sheet (#275)"
                .to_owned(),
        );
    }
    for (name, _) in &bound {
        if !reaches(&stmts, name, &|h| prints(h) || asserts(h)) {
            problems.push(format!(
                "`{name}` is a stopwatch nothing reads: print the per-sheet `BIG` line with \
                 its elapsed time, as the CLI twin does (#275)"
            ));
        }
    }
    problems
}

#[test]
fn the_big_sheet_test_holds_the_landmarks_to_the_formula_275() {
    let (body, masked) = function_body(&read(PARITY_FILE), BIG_SHEETS_TEST);
    let problems = formula_problems(&body, &masked);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_big_sheet_test_reports_each_sheet_275() {
    let (body, masked) = function_body(&read(PARITY_FILE), BIG_SHEETS_TEST);
    let problems = report_problems(&body, &masked);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

// ---------------------------------------------------------------------------
// Controls: the shapes the scanner has to reject, and the two it has to accept
// ---------------------------------------------------------------------------

/// A body in the shape of main's today, parameterised on the lines inside the
/// loop after the comparison.
fn body_with(after_compare: &str) -> (String, String) {
    let src = format!(
        "fn big_sheets_match_the_on_demand_libvips_reference() {{\n\
         \x20   let started = std::time::Instant::now();\n\
         \x20   let (mut compared, mut failures) = (0, Vec::new());\n\
         \x20   for e in matrix().iter().filter(|e| e.on_demand) {{\n\
         \x20       let label = format!(\"{{}} @ {{}}\", e.label, e.dpi);\n\
         \x20       let t = std::time::Instant::now();\n\
         \x20       let (w, h) = (10, 20);\n\
         \x20       let pt = landmark_points((e.spec.w, e.spec.h), 0, (w, h));\n\
         \x20       let big = compare_big(&label, (w, h));\n\
         {after_compare}\n\
         \x20       if !big.stats.within(&tol) {{\n\
         \x20           failures.push(format!(\"{{label}}: {{}}\", big.stats.summary()));\n\
         \x20       }}\n\
         \x20       compared += 1;\n\
         \x20   }}\n\
         \x20   eprintln!(\"BIG total: {{compared}} compared, {{:.1}}s\", started.elapsed().as_secs_f64());\n\
         \x20   assert!(failures.is_empty(), \"outside:\\n{{}}\", failures.join(\"\\n\"));\n\
         }}\n"
    );
    function_body(&src, BIG_SHEETS_TEST)
}

fn assert_rejected(problems: &[String], expected: &str) {
    assert!(
        problems.iter().any(|p| p.contains(expected)),
        "expected a problem mentioning `{expected}`, got: {problems:#?}"
    );
}

/// Main's shape: both bindings taken, neither read. Both guards reject it.
#[test]
fn control_bindings_that_are_never_read_are_rejected_275() {
    let (body, masked) = body_with("");
    assert_rejected(&formula_problems(&body, &masked), "nothing asserts on it");
    assert_rejected(&report_problems(&body, &masked), "stopwatch nothing reads");
}

/// The lint's own suggestion, `_pt` and `_t`, binds nothing a later statement
/// reads, so the guards see no formula check and no report, same as before.
#[test]
fn control_underscored_bindings_are_rejected_275() {
    let (body, masked) = body_with("");
    let body = body
        .replace("let t =", "let _t =")
        .replace("let pt =", "let _pt =");
    let masked = masked
        .replace("let t =", "let _t =")
        .replace("let pt =", "let _pt =");
    assert_rejected(&formula_problems(&body, &masked), "nothing asserts on it");
    assert_rejected(&report_problems(&body, &masked), "stopwatch nothing reads");
}

/// `let _ = pt;` reads the binding in a statement that checks nothing.
#[test]
fn control_a_discard_is_rejected_275() {
    let (body, masked) = body_with("        let _ = pt;\n        let _ = t;");
    assert_rejected(&formula_problems(&body, &masked), "nothing asserts on it");
    assert_rejected(&report_problems(&body, &masked), "stopwatch nothing reads");
}

/// Deleting the binding outright leaves nothing to check the formula with.
#[test]
fn control_a_deleted_formula_binding_is_rejected_275() {
    let (body, masked) = body_with("");
    let body = body.replace(
        "let pt = landmark_points((e.spec.w, e.spec.h), 0, (w, h));",
        "",
    );
    let masked = masked.replace(
        "let pt = landmark_points((e.spec.w, e.spec.h), 0, (w, h));",
        "",
    );
    assert_rejected(
        &formula_problems(&body, &masked),
        "never asks `landmark_points(..)`",
    );
}

/// The check written inline: the probed midpoint against `pt`, a miss pushed
/// onto `failures`, and the per-sheet line with `t.elapsed()`.
#[test]
fn control_an_inline_formula_check_and_report_are_accepted_275() {
    let (body, masked) = body_with(
        "        let ((x0, x1), (y0, y1), run) = big.ours_landmarks;\n\
         \x20       let mid = ((x0 + x1 + 1) as f64 / 2.0, (y0 + y1 + 1) as f64 / 2.0);\n\
         \x20       if (mid.0 - pt.px as f64).abs() > 1.0 || (mid.1 - pt.py as f64).abs() > 1.0 || run == 0 {\n\
         \x20           failures.push(format!(\"{label}: landmarks {mid:?} vs the formula ({}, {})\", pt.px, pt.py));\n\
         \x20       }\n\
         \x20       eprintln!(\"BIG {label} {w}x{h} ({:.1}s)\", t.elapsed().as_secs_f64());",
    );
    assert_eq!(formula_problems(&body, &masked), Vec::<String>::new());
    assert_eq!(report_problems(&body, &masked), Vec::<String>::new());
}

/// The same through a helper: `pt` goes into a `let`, and the result of that
/// `let` is what the assertion reads. One level of indirection is allowed.
#[test]
fn control_a_check_through_a_helper_is_accepted_275() {
    let (body, masked) = body_with(
        "        let formula = landmarks_vs_formula(big.ours_landmarks, pt);\n\
         \x20       if let Err(why) = formula {\n\
         \x20           failures.push(format!(\"{label}: {why}\"));\n\
         \x20       }\n\
         \x20       let took = t.elapsed();\n\
         \x20       eprintln!(\"BIG {label} ({:.1}s)\", took.as_secs_f64());",
    );
    assert_eq!(formula_problems(&body, &masked), Vec::<String>::new());
    assert_eq!(report_problems(&body, &masked), Vec::<String>::new());
}

/// The real file has the function and both bindings the guards are about, so
/// a rename of either leaves the guards something to say rather than nothing.
#[test]
fn the_scanner_finds_the_function_and_its_bindings_275() {
    let (body, masked) = function_body(&read(PARITY_FILE), BIG_SHEETS_TEST);
    let stmts = statements(&body, &masked);
    let lm = bindings_of(&stmts, "landmark_points(");
    let clocks = bindings_of(&stmts, "Instant::now()");
    assert!(
        !lm.is_empty(),
        "no `let .. = landmark_points(..)` in {BIG_SHEETS_TEST}"
    );
    assert!(
        clocks.len() >= 2,
        "expected the run's stopwatch and a per-sheet one in {BIG_SHEETS_TEST}, found {clocks:?}"
    );
    // And the body is read as more than one statement, or the splitter is
    // looking at the wrong thing.
    let mut all = Vec::new();
    walk(&stmts, &mut all);
    assert!(all.len() > 10, "only {} statements parsed", all.len());
}
