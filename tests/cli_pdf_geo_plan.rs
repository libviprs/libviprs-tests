//! The CLI half of libviprs/libviprs-cli#68: the PDF options past the
//! defaults (`viprs pdf`), the geo transforms (`viprs geo`) and the planner
//! queries on `viprs plan`.
//!
//! These are the same three surfaces the library already has and the CLI did
//! not. Every cell here drives the real `viprs` binary and compares what it
//! printed or wrote against the library function it is supposed to be a thin
//! door onto, or against a committed reference, so a flag that parses and does
//! nothing cannot pass.
//!
//! # Exit codes are not evidence
//!
//! A cell that asserts exit 0 passes when the command is a stub that returns
//! success. Each success cell below asserts on a value or on pixels:
//!
//! * the planner queries are compared byte for byte with the string or number
//!   `PyramidPlanner` itself returns for the same inputs, and every query is
//!   also run with a second input that must change the answer, so a flag that
//!   was swallowed (and a default printed instead) fails;
//! * the geo cells compare against `GeoTransform` and then push the result back
//!   through the inverse command, at a probe that the transform does not leave
//!   where it was;
//! * the background cell compares decoded pixels with the committed vips
//!   reference `pdf_bg_red_expected.png` and then shows a second colour gives
//!   different pixels, so "the flag was ignored" and "the flag was applied to
//!   the wrong region" both fail;
//! * the password cells separate the three outcomes (no password, wrong
//!   password, right password) rather than lumping them under "non-zero".
//!
//! # The password cells
//!
//! `password.pdf` is AES-256 encrypted (R6) and its only page is text, with no
//! embedded image. `lopdf` 0.36 cannot decrypt it (`Decryption(InvalidKeyLength)`),
//! so the library opens it through pdfium: `pdf_info_with_password` and
//! `extract_page_image_with_password` take the password, and they answer a
//! missing or empty one with `PdfError::PasswordRequired` and a wrong one with
//! `PdfError::WrongPassword`. The CLI maps those two onto its own messages, so
//! the cells pin the wording a person sees ("a password is needed" with the
//! `--password` hint, and "wrong password") rather than just "non-zero".
//!
//! The password reaches the CLI three ways (`--password`, `--password-file`
//! with `-` for stdin, and `VIPRS_PDF_PASSWORD`), and each has a cell. Every
//! password cell also checks that neither the right nor the wrong password
//! turns up in stdout or stderr. Every run here clears `VIPRS_PDF_PASSWORD`
//! first, so a value left in the caller's environment cannot answer for a
//! cell that means to give none.
//!
//! `owner-password-only.pdf` is the other side of it: AES-256 too, but with an
//! empty user password, so it opens without one and `pdf extract` must hand
//! back its stored 64x48 image rather than a 72-DPI render of the page.
//!
//! All of them need a pdfium-enabled `viprs` and a libpdfium, the same as the
//! cells in the next section, because the encrypted files only open through
//! pdfium. `password_fixture_is_encrypted_and_not_open_without_a_password`
//! pins the premise (the fixture really is protected) and records how `secret`
//! was established, so a cell cannot be red for a reason that is the fixture's
//! fault.
//!
//! # The pdfium cells
//!
//! Background, DPI and the render budget all go through pdfium. They need two
//! things this crate's default CLI build does not give them:
//!
//! * a `viprs` built with the CLI's default `pdfium` feature, handed in as
//!   `$VIPRS_BIN` (`tests/common/cli.rs` builds `--no-default-features` when it
//!   builds the sibling itself);
//! * a libpdfium for the host, found through `$PDFIUM_PATH`.
//!
//! Without `$PDFIUM_PATH` those cells print SKIP and return, the same shape
//! `cli_pmtiles.rs` uses for a missing CLI. A skip reads as a pass to
//! `cargo test`, so `$VIPRS_REQUIRE_PDFIUM=1` turns it into a panic, and whoever
//! wires a job that is meant to run them sets it.

mod common;

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::cli::{cli_available, viprs_bin};

use libviprs::{
    GeoCoord, GeoTransform, Layout, PixelCoord, PixelFormat, PyramidPlanner, decode_file,
    planner::TileCoord,
};

use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Skip guards
// ---------------------------------------------------------------------------

fn skip_if_no_cli(test: &str) -> bool {
    if cli_available() {
        return false;
    }
    eprintln!(
        "SKIP {test}: libviprs-cli sibling not checked out \
         (set $VIPRS_CLI_DIR / $VIPRS_BIN, or run in the cli-differential job)."
    );
    true
}

/// `true` (with a printed reason) when the pdfium cells cannot run here.
///
/// Also skips when the CLI is absent. Under `$VIPRS_REQUIRE_PDFIUM=1` it panics
/// instead, so a job that is supposed to run these cells cannot go green by
/// not running them.
fn skip_if_no_pdfium(test: &str) -> bool {
    if skip_if_no_cli(test) {
        return true;
    }
    if std::env::var_os("PDFIUM_PATH").is_some() {
        return false;
    }
    if std::env::var("VIPRS_REQUIRE_PDFIUM").is_ok_and(|v| v == "1") {
        panic!(
            "VIPRS_REQUIRE_PDFIUM=1 but $PDFIUM_PATH is unset: {test} would skip and \
             report a false green. Point $PDFIUM_PATH at a libpdfium and $VIPRS_BIN at a \
             `viprs` built with the default `pdfium` feature."
        );
    }
    eprintln!("SKIP {test}: no $PDFIUM_PATH (needs libpdfium and a pdfium-enabled $VIPRS_BIN).");
    true
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn s(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// The environment variable the CLI reads a password from.
const PASSWORD_ENV: &str = "VIPRS_PDF_PASSWORD";

/// Run `viprs` with `VIPRS_PDF_PASSWORD` cleared, then set to `env_password`
/// if one is given, feeding `stdin` if one is given.
fn run_with(args: &[&str], env_password: Option<&str>, stdin: Option<&[u8]>) -> Output {
    let mut cmd = Command::new(viprs_bin());
    cmd.args(args).env_remove(PASSWORD_ENV);
    if let Some(password) = env_password {
        cmd.env(PASSWORD_ENV, password);
    }
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to run viprs {args:?}: {e}"));
    if let Some(bytes) = stdin {
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(bytes)
            .expect("viprs must take its stdin");
    }
    child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("failed to wait for viprs {args:?}: {e}"))
}

/// Run `viprs` with no password in its environment.
fn run_viprs(args: &[&str]) -> Output {
    run_with(args, None, None)
}

/// Neither stream of `out` may carry `password`.
fn assert_password_not_echoed(out: &Output, password: &str, what: &str) {
    assert!(
        !stdout(out).contains(password) && !stderr(out).contains(password),
        "{what}: the password {password:?} must not appear in the output\n\
         --- stdout ---\n{}\n--- stderr ---\n{}",
        stdout(out),
        stderr(out),
    );
}

/// Run `viprs` and require exit 0, returning stdout. The failure message
/// carries both streams, because "exited 2" alone says nothing.
fn ok(args: &[&str]) -> String {
    let out = run_viprs(args);
    assert!(
        out.status.success(),
        "viprs {args:?} exited {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status,
        stdout(&out),
        stderr(&out),
    );
    stdout(&out)
}

/// Run `viprs` and require it to exit with exactly `code`.
fn exits(code: i32, args: &[&str]) -> Output {
    let out = run_viprs(args);
    assert_eq!(
        out.status.code(),
        Some(code),
        "viprs {args:?} should exit {code}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        stdout(&out),
        stderr(&out),
    );
    out
}

/// Parse `"a,b"` into two floats.
fn pair(text: &str) -> (f64, f64) {
    let line = text.trim();
    let (a, b) = line
        .split_once(',')
        .unwrap_or_else(|| panic!("expected \"x,y\", got {line:?}"));
    (
        a.trim().parse().unwrap_or_else(|e| panic!("{a:?}: {e}")),
        b.trim().parse().unwrap_or_else(|e| panic!("{b:?}: {e}")),
    )
}

fn close(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

// ===========================================================================
// PDF: password
// ===========================================================================

const SECRET: &str = "secret";

/// The premise every password cell stands on, checked without the CLI: the
/// fixture really carries an `/Encrypt` dictionary, and `lopdf` does not hand
/// it over for the empty password.
///
/// That `secret` is its password was established with qpdf 11.3.0 (`qpdf
/// --password=secret --decrypt password.pdf out.pdf` succeeds, `--password=` and
/// `--password=wrong` report "Incorrect password supplied", and `--show-encryption`
/// reports R = 6, user password `secret`). It is not asserted here through `lopdf`:
/// lopdf 0.36's `Document::decrypt("secret")` fails on this file with
/// `Decryption(InvalidKeyLength)` rather than opening it, which is a second reason
/// the library has no decryption path today.
#[test]
fn password_fixture_is_encrypted_and_not_open_without_a_password() {
    let doc = lopdf::Document::load(fixture("password.pdf")).expect("password.pdf parses");
    assert!(
        doc.is_encrypted(),
        "password.pdf must carry an /Encrypt dictionary"
    );
    assert!(
        doc.authenticate_password("").is_err(),
        "the empty password must not open password.pdf"
    );
}

/// 68: no `--password` on an encrypted file is exit 1 and says a password is
/// needed. Checked on both `pdf extract` and `pdf info`, since either is where
/// someone meets it first.
#[test]
fn pdf_68_password_pdf_without_password_exits_1_saying_a_password_is_needed() {
    if skip_if_no_pdfium("pdf_68_password_pdf_without_password") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let out_png = dir.path().join("o.png");
    let pdf = fixture("password.pdf");

    let extract = exits(1, &["pdf", "extract", s(&pdf), s(&out_png)]);
    let msg = stderr(&extract).to_lowercase();
    assert!(
        msg.contains("a password is needed"),
        "stderr must say a password is needed, got: {msg}"
    );
    assert!(
        msg.contains("--password"),
        "stderr must point at --password, got: {msg}"
    );
    assert!(
        !out_png.exists(),
        "a refused extract must not leave an output"
    );

    let info = exits(1, &["pdf", "info", s(&pdf)]);
    assert!(
        stderr(&info)
            .to_lowercase()
            .contains("a password is needed"),
        "pdf info must say it too, got: {}",
        stderr(&info)
    );
}

/// 68: the right password gets through, on both commands. `pdf info` has to
/// surface the page the file really has, and `pdf extract` has to write a
/// decodable, non-empty PNG of it. The fixture's page is text on white with no
/// embedded image, so a render that never decrypted the content stream would
/// come back blank: the pixels are checked for ink, not just for a size.
#[test]
fn pdf_68_password_pdf_with_the_right_password_opens() {
    if skip_if_no_pdfium("pdf_68_password_pdf_with_the_right_password") {
        return;
    }
    let pdf = fixture("password.pdf");
    let info = run_viprs(&["pdf", "info", s(&pdf), "--password", SECRET]);
    assert!(info.status.success(), "pdf info: {}", stderr(&info));
    assert_password_not_echoed(&info, SECRET, "pdf info with the right password");
    let text = stdout(&info);
    assert!(text.contains("Pages: 1"), "pdf info output was: {text}");
    // The page is A4, 595.28 x 841.89 pt, which is what the fixture's MediaBox
    // says and what unlocking it has to surface.
    assert!(
        text.contains("595.3 x 841.9"),
        "pdf info output was: {text}"
    );

    let dir = TempDir::new().unwrap();
    let out_png = dir.path().join("unlocked.png");
    let extract = run_viprs(&["pdf", "extract", s(&pdf), s(&out_png), "--password", SECRET]);
    assert!(
        extract.status.success(),
        "pdf extract: {}",
        stderr(&extract)
    );
    assert_password_not_echoed(&extract, SECRET, "pdf extract with the right password");
    let raster = decode_file(&out_png).expect("the unlocked extract decodes as a PNG");
    assert!(
        raster.width() > 0 && raster.height() > 0 && !raster.data().is_empty(),
        "the unlocked extract is empty: {}x{}",
        raster.width(),
        raster.height()
    );
    let bpp = raster.format().bytes_per_pixel();
    assert!(
        raster
            .data()
            .chunks_exact(bpp)
            .any(|px| px[..bpp.min(3)].iter().any(|&c| c != 255)),
        "the unlocked page rendered blank, so it was never decrypted"
    );
}

/// 68: a wrong password is exit 1 and says so ("wrong password"), which is
/// neither the "a password is needed" of a missing one nor a message about a
/// missing file or a usage mistake.
#[test]
fn pdf_68_password_pdf_with_a_wrong_password_exits_1_naming_the_password() {
    if skip_if_no_pdfium("pdf_68_password_pdf_with_a_wrong_password") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let out_png = dir.path().join("o.png");
    let pdf = fixture("password.pdf");
    let out = exits(
        1,
        &[
            "pdf",
            "extract",
            s(&pdf),
            s(&out_png),
            "--password",
            "not-the-password",
        ],
    );
    assert_password_not_echoed(
        &out,
        "not-the-password",
        "pdf extract with a wrong password",
    );
    let msg = stderr(&out).to_lowercase();
    assert!(
        msg.contains("wrong password"),
        "stderr must say \"wrong password\": {msg}"
    );
    assert!(
        !msg.contains("a password is needed"),
        "a password WAS given, so it must not claim none was: {msg}"
    );
    assert!(
        !out_png.exists(),
        "a refused extract must not leave an output"
    );
}

/// Decode an extract of `password.pdf` and require the decrypted page: A4 at
/// 72 DPI with ink on it.
fn assert_unlocked_page(png: &Path) {
    let raster = decode_file(png).expect("the unlocked extract decodes as a PNG");
    assert_eq!(
        (raster.width(), raster.height()),
        (595, 841),
        "a page that needs a user password is rendered at 72 DPI"
    );
    let bpp = raster.format().bytes_per_pixel();
    assert!(
        raster
            .data()
            .chunks_exact(bpp)
            .any(|px| px[..bpp.min(3)].iter().any(|&c| c != 255)),
        "the unlocked page rendered blank, so it was never decrypted"
    );
}

/// 68: `VIPRS_PDF_PASSWORD` opens the file when no flag is given, on both
/// commands, and is never echoed.
#[test]
fn pdf_68_password_from_the_environment_opens_the_file() {
    if skip_if_no_pdfium("pdf_68_password_from_the_environment") {
        return;
    }
    let pdf = fixture("password.pdf");
    let info = run_with(&["pdf", "info", s(&pdf)], Some(SECRET), None);
    assert!(info.status.success(), "pdf info: {}", stderr(&info));
    assert!(stdout(&info).contains("595.3 x 841.9"), "{}", stdout(&info));
    assert_password_not_echoed(&info, SECRET, "pdf info with VIPRS_PDF_PASSWORD");

    let dir = TempDir::new().unwrap();
    let out_png = dir.path().join("env.png");
    let extract = run_with(
        &["pdf", "extract", s(&pdf), s(&out_png)],
        Some(SECRET),
        None,
    );
    assert!(
        extract.status.success(),
        "pdf extract: {}",
        stderr(&extract)
    );
    assert_password_not_echoed(&extract, SECRET, "pdf extract with VIPRS_PDF_PASSWORD");
    assert_unlocked_page(&out_png);
}

/// 68: a wrong `VIPRS_PDF_PASSWORD` is a wrong password, said as such and not
/// echoed.
#[test]
fn pdf_68_a_wrong_password_from_the_environment_exits_1_without_echoing_it() {
    if skip_if_no_pdfium("pdf_68_a_wrong_password_from_the_environment") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let out_png = dir.path().join("o.png");
    let pdf = fixture("password.pdf");
    let wrong = "env-wrong-password";
    let out = run_with(&["pdf", "extract", s(&pdf), s(&out_png)], Some(wrong), None);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).to_lowercase().contains("wrong password"),
        "stderr: {}",
        stderr(&out)
    );
    assert_password_not_echoed(&out, wrong, "pdf extract with a wrong VIPRS_PDF_PASSWORD");
    assert!(
        !out_png.exists(),
        "a refused extract must not leave an output"
    );
}

/// 68: `--password-file PATH` opens the file (the trailing newline an editor
/// adds is not part of the password), on both commands, and is never echoed.
#[test]
fn pdf_68_password_from_a_file_opens_the_file() {
    if skip_if_no_pdfium("pdf_68_password_from_a_file") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("pw.txt");
    std::fs::write(&file, format!("{SECRET}\n")).unwrap();
    let pdf = fixture("password.pdf");

    let info = run_viprs(&["pdf", "info", s(&pdf), "--password-file", s(&file)]);
    assert!(info.status.success(), "pdf info: {}", stderr(&info));
    assert!(stdout(&info).contains("595.3 x 841.9"), "{}", stdout(&info));
    assert_password_not_echoed(&info, SECRET, "pdf info with --password-file");

    let out_png = dir.path().join("file.png");
    let extract = run_viprs(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&out_png),
        "--password-file",
        s(&file),
    ]);
    assert!(
        extract.status.success(),
        "pdf extract: {}",
        stderr(&extract)
    );
    assert_password_not_echoed(&extract, SECRET, "pdf extract with --password-file");
    assert_unlocked_page(&out_png);
}

/// 68: `--password-file -` reads the password from stdin, so a pipe can hand
/// it over without it touching argv or the disk.
#[test]
fn pdf_68_password_from_stdin_opens_the_file() {
    if skip_if_no_pdfium("pdf_68_password_from_stdin") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let out_png = dir.path().join("stdin.png");
    let pdf = fixture("password.pdf");
    let stdin = format!("{SECRET}\n");
    let extract = run_with(
        &[
            "pdf",
            "extract",
            s(&pdf),
            s(&out_png),
            "--password-file",
            "-",
        ],
        None,
        Some(stdin.as_bytes()),
    );
    assert!(
        extract.status.success(),
        "pdf extract: {}",
        stderr(&extract)
    );
    assert_password_not_echoed(&extract, SECRET, "pdf extract with --password-file -");
    assert_unlocked_page(&out_png);
}

/// 68: a wrong password in a file is a wrong password, not echoed.
#[test]
fn pdf_68_a_wrong_password_from_a_file_exits_1_without_echoing_it() {
    if skip_if_no_pdfium("pdf_68_a_wrong_password_from_a_file") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("pw.txt");
    let wrong = "file-wrong-password";
    std::fs::write(&file, format!("{wrong}\n")).unwrap();
    let pdf = fixture("password.pdf");
    let out = run_viprs(&["pdf", "info", s(&pdf), "--password-file", s(&file)]);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out).to_lowercase().contains("wrong password"),
        "stderr: {}",
        stderr(&out)
    );
    assert_password_not_echoed(&out, wrong, "pdf info with a wrong --password-file");
}

/// The stored image `owner-password-only.pdf` carries: 64x48, pixel (x, y) =
/// (4x, 5y, 128). See `tests/fixtures/README.md` for how it was made.
fn assert_owner_only_stored_image(png: &Path) {
    let raster = decode_file(png).expect("the owner-only extract decodes as a PNG");
    assert_eq!(
        (raster.width(), raster.height()),
        (64, 48),
        "an owner-password-only file opens without a password, so extract must give its \
         embedded image at the stored 64x48, not a 200x150 render of the page"
    );
    let bpp = raster.format().bytes_per_pixel();
    assert!(
        bpp >= 3,
        "expected colour pixels, got {:?}",
        raster.format()
    );
    for (x, y) in [(0u32, 0u32), (63, 0), (0, 47), (63, 47), (10, 20)] {
        let at = ((y * 64 + x) as usize) * bpp;
        assert_eq!(
            &raster.data()[at..at + 3],
            &[(4 * x) as u8, (5 * y) as u8, 128],
            "stored pixel ({x}, {y})"
        );
    }
}

/// 68: an owner password only restricts permissions, so `pdf extract` with no
/// password hands back the embedded image at its stored resolution, as for an
/// unencrypted file. lopdf cannot decrypt this AES-256 file, so this is the
/// pdfium route, and it must still extract rather than render.
#[test]
fn pdf_68_owner_password_only_pdf_extracts_its_embedded_image() {
    if skip_if_no_pdfium("pdf_68_owner_password_only_pdf") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let pdf = fixture("owner-password-only.pdf");
    let out_png = dir.path().join("owner.png");
    ok(&["pdf", "extract", s(&pdf), s(&out_png)]);
    assert_owner_only_stored_image(&out_png);

    let text = ok(&["pdf", "info", s(&pdf)]);
    assert!(text.contains("Pages: 1"), "pdf info output was: {text}");
    assert!(
        text.contains("200.0 x 150.0"),
        "pdf info output was: {text}"
    );
}

/// 68: a password given for a file that needs none is ignored, as it is for an
/// unencrypted file: the stored image still comes back, not a render.
#[test]
fn pdf_68_owner_password_only_pdf_ignores_a_password_it_does_not_need() {
    if skip_if_no_pdfium("pdf_68_owner_password_only_pdf_ignores") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let pdf = fixture("owner-password-only.pdf");
    let out_png = dir.path().join("owner-pw.png");
    let out = run_viprs(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&out_png),
        "--password",
        "owner-only-secret",
    ]);
    assert!(out.status.success(), "pdf extract: {}", stderr(&out));
    assert_password_not_echoed(
        &out,
        "owner-only-secret",
        "pdf extract on an owner-only file",
    );
    assert_owner_only_stored_image(&out_png);
}

/// 68: `--password` on a file that is not encrypted is accepted and changes
/// nothing, matching the library ("unencrypted documents extract regardless").
/// This is the cell that stops `--password` being a flag that breaks plain
/// files.
#[test]
fn pdf_68_password_on_an_unencrypted_pdf_extracts_the_same_pixels() {
    if skip_if_no_cli("pdf_68_password_on_an_unencrypted_pdf") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let pdf = fixture("canonical_cmyk.pdf");
    let plain = dir.path().join("plain.png");
    let with_pw = dir.path().join("with_pw.png");
    ok(&["pdf", "extract", s(&pdf), s(&plain)]);
    ok(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&with_pw),
        "--password",
        "anything",
    ]);
    let a = decode_file(&plain).expect("plain extract decodes");
    let b = decode_file(&with_pw).expect("password extract decodes");
    assert_eq!(
        (a.width(), a.height()),
        (16, 16),
        "the fixture image is 16x16"
    );
    assert_eq!(
        a.data(),
        b.data(),
        "a password on a plain file must not change pixels"
    );
}

// ===========================================================================
// PDF: background, DPI, render budget (pdfium)
// ===========================================================================

/// 68: `--background` fills the region the page leaves transparent. The
/// fixture is an empty page, so the whole render is the background, and the
/// committed vips reference is solid red.
#[test]
fn pdf_68_background_changes_the_pixels_of_a_transparent_region() {
    if skip_if_no_pdfium("pdf_68_background") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let pdf = fixture("canonical_solid_white.pdf");
    let red = dir.path().join("red.png");
    let green = dir.path().join("green.png");
    ok(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&red),
        "--background",
        "255,0,0,255",
    ]);
    ok(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&green),
        "--background",
        "0,255,0,255",
    ]);

    let reference = decode_file(&fixture("pdf_bg_red_expected.png")).unwrap();
    let got = decode_file(&red).expect("red render decodes");
    assert_eq!(
        (got.width(), got.height()),
        (reference.width(), reference.height()),
        "the render must be the page at 72 DPI, as the vips reference is"
    );
    let diff = got.max_diff(&reference);
    assert!(
        diff <= 1.0,
        "red background differs from the vips reference by {diff}"
    );

    // A pixel that is certainly in the transparent region.
    let px = got.width() as usize / 2 + (got.height() as usize / 2) * got.width() as usize;
    let bpp = got.format().bytes_per_pixel();
    assert_eq!(
        &got.data()[px * bpp..px * bpp + 3],
        &[255, 0, 0],
        "centre pixel must be the requested red"
    );

    let other = decode_file(&green).expect("green render decodes");
    assert!(
        other.max_diff(&got) > 0.0,
        "a different --background must give different pixels, or the flag is ignored"
    );
    let gpx = &other.data()[px * bpp..px * bpp + 3];
    assert_eq!(
        gpx,
        &[0, 255, 0],
        "centre pixel must be the requested green"
    );
}

/// 68: three channels are opaque RGB, and a channel count that is neither 3
/// nor 4 is refused rather than guessed at.
#[test]
fn pdf_68_background_refuses_a_bad_channel_count_and_accepts_rgb() {
    if skip_if_no_pdfium("pdf_68_background_channels") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let pdf = fixture("canonical_solid_white.pdf");

    let rgb = dir.path().join("rgb.png");
    ok(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&rgb),
        "--background",
        "255,0,0",
    ]);
    let rgba = dir.path().join("rgba.png");
    ok(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&rgba),
        "--background",
        "255,0,0,255",
    ]);
    assert_eq!(
        decode_file(&rgb).unwrap().data(),
        decode_file(&rgba).unwrap().data(),
        "r,g,b is the same as r,g,b,255"
    );

    let bad = dir.path().join("bad.png");
    let out = run_viprs(&["pdf", "extract", s(&pdf), s(&bad), "--background", "1,2"]);
    assert!(!out.status.success(), "two channels must be refused");
    assert!(!bad.exists(), "a refused extract must not leave an output");
}

/// 68: `--dpi` scales the render. A4 at 144 DPI is twice A4 at 72.
#[test]
fn pdf_68_dpi_scales_the_extracted_page() {
    if skip_if_no_pdfium("pdf_68_dpi") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let pdf = fixture("canonical.pdf");
    let lo = dir.path().join("lo.png");
    let hi = dir.path().join("hi.png");
    ok(&["pdf", "extract", s(&pdf), s(&lo), "--dpi", "72"]);
    ok(&["pdf", "extract", s(&pdf), s(&hi), "--dpi", "144"]);
    let a = decode_file(&lo).unwrap();
    let b = decode_file(&hi).unwrap();
    assert_eq!((a.width(), a.height()), (595, 842), "A4 at 72 DPI");
    assert!(
        close(b.width() as f64, 2.0 * a.width() as f64, 2.0)
            && close(b.height() as f64, 2.0 * a.height() as f64, 2.0),
        "144 DPI must be about twice 72 DPI, got {}x{} vs {}x{}",
        b.width(),
        b.height(),
        a.width(),
        a.height()
    );
}

/// 68: `--render-budget` (a pixel ceiling) lowers the DPI until the render fits
/// and says so. Without it the same `--dpi` is bigger than the ceiling.
#[test]
fn pdf_68_render_budget_caps_the_pixel_count_and_reports_the_dpi() {
    if skip_if_no_pdfium("pdf_68_render_budget") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let pdf = fixture("canonical.pdf");
    let free = dir.path().join("free.png");
    let capped = dir.path().join("capped.png");
    let budget: u64 = 500_000;

    ok(&["pdf", "extract", s(&pdf), s(&free), "--dpi", "300"]);
    let out = run_viprs(&[
        "pdf",
        "extract",
        s(&pdf),
        s(&capped),
        "--dpi",
        "300",
        "--render-budget",
        &budget.to_string(),
    ]);
    assert!(
        out.status.success(),
        "budgeted extract failed: {}",
        stderr(&out)
    );

    let f = decode_file(&free).unwrap();
    let c = decode_file(&capped).unwrap();
    let free_px = f.width() as u64 * f.height() as u64;
    let capped_px = c.width() as u64 * c.height() as u64;
    assert!(
        free_px > budget,
        "premise: 300 DPI A4 is {free_px} px, over {budget}"
    );
    assert!(
        capped_px <= budget,
        "budgeted render is {capped_px} px, over the {budget} ceiling"
    );
    assert!(
        capped_px > 0 && capped_px < free_px,
        "budget must shrink the render"
    );
    let msg = stderr(&out).to_lowercase();
    assert!(
        msg.contains("dpi") && msg.contains("capped"),
        "the DPI actually used, and that it was capped, must be reported: {msg}"
    );
}

/// 68: a page's `/Rotate` is honoured by the render and reported by
/// `pdf rotation`. The 90 degree fixture is the 0 degree page on its side.
#[test]
fn pdf_68_rotation_is_reported_and_honoured() {
    if skip_if_no_cli("pdf_68_rotation_reported") {
        return;
    }
    for (name, degrees) in [
        ("canonical.pdf", "0"),
        ("canonical_rotated_90.pdf", "90"),
        ("canonical_rotated_180.pdf", "180"),
        ("canonical_rotated_270.pdf", "270"),
    ] {
        let pdf = fixture(name);
        let text = ok(&["pdf", "rotation", s(&pdf)]);
        assert_eq!(text.trim(), degrees, "pdf rotation {name}");
    }
    // `--page` is live: the fixtures have one page, so page 2 is out of range.
    let pdf = fixture("canonical.pdf");
    let out = exits(1, &["pdf", "rotation", s(&pdf), "--page", "2"]);
    assert!(
        stderr(&out).contains("out of range"),
        "page 2 of a one page file must say so: {}",
        stderr(&out)
    );

    if skip_if_no_pdfium("pdf_68_rotation_honoured") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let upright = dir.path().join("upright.png");
    let turned = dir.path().join("turned.png");
    ok(&[
        "pdf",
        "extract",
        s(&fixture("canonical.pdf")),
        s(&upright),
        "--dpi",
        "72",
    ]);
    ok(&[
        "pdf",
        "extract",
        s(&fixture("canonical_rotated_90.pdf")),
        s(&turned),
        "--dpi",
        "72",
    ]);
    let a = decode_file(&upright).unwrap();
    let b = decode_file(&turned).unwrap();
    assert_eq!(
        (a.width(), a.height()),
        (b.height(), b.width()),
        "a /Rotate 90 page renders on its side"
    );
}

/// 68: the PDF options that need pdfium say so plainly when the binary has no
/// pdfium, and the ones that cannot combine are a usage error (exit 2), not a
/// silent pick of one.
#[test]
fn pdf_68_incompatible_pdf_options_are_a_usage_error() {
    if skip_if_no_cli("pdf_68_incompatible_pdf_options") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let o = dir.path().join("o.png");
    let pdf = fixture("canonical.pdf");
    // The core has one DPI-controlled render and one background render, and no
    // render that takes both, so asking for both has no defined meaning.
    let both = exits(
        2,
        &[
            "pdf",
            "extract",
            s(&pdf),
            s(&o),
            "--dpi",
            "144",
            "--background",
            "255,0,0",
        ],
    );
    assert!(
        stderr(&both).contains("cannot be used with"),
        "the refusal must name the clash, got: {}",
        stderr(&both)
    );
    // The render path cannot take a password, so the pair has no meaning either.
    let locked = exits(
        2,
        &[
            "pdf",
            "extract",
            s(&pdf),
            s(&o),
            "--dpi",
            "144",
            "--password",
            "x",
        ],
    );
    assert!(
        stderr(&locked).contains("cannot be used with"),
        "the refusal must name the clash, got: {}",
        stderr(&locked)
    );
    assert!(!o.exists(), "a usage error must not write anything");
}

// ===========================================================================
// geo
// ===========================================================================

/// A plan sheet that is rotated and skewed, with an origin well away from
/// zero: nothing about it leaves a pixel where it was, so a command that
/// returned its input unchanged, or applied only the scale, fails.
const AFFINE: [f64; 6] = [0.5, 0.125, 1000.0, -0.0625, -0.75, 2000.0];

fn affine_arg() -> String {
    AFFINE.map(|v| v.to_string()).join(",")
}

fn affine() -> GeoTransform {
    let [a, b, c, d, e, f] = AFFINE;
    GeoTransform::new(a, b, c, d, e, f)
}

/// 68: pixel to geo to pixel returns the probe, at a probe the transform moves.
#[test]
fn geo_68_round_trips_a_pixel_that_is_not_a_fixed_point() {
    if skip_if_no_cli("geo_68_round_trip") {
        return;
    }
    let probe = PixelCoord {
        x: 123.25,
        y: 456.75,
    };
    let expected = affine().pixel_to_geo(probe);
    // The premise of the cell: this transform really does move this pixel.
    assert!(
        !close(expected.x, probe.x, 1e-6) && !close(expected.y, probe.y, 1e-6),
        "the probe must not be a fixed point of the transform"
    );

    let a = affine_arg();
    let fwd = ok(&["geo", "pixel-to-geo", "123.25", "456.75", "--affine", &a]);
    let (gx, gy) = pair(&fwd);
    assert!(
        close(gx, expected.x, 1e-9) && close(gy, expected.y, 1e-9),
        "forward: {fwd}"
    );
    assert!(
        !close(gx, probe.x, 1e-6) || !close(gy, probe.y, 1e-6),
        "the forward result must differ from the probe"
    );

    let back = ok(&[
        "geo",
        "geo-to-pixel",
        &gx.to_string(),
        &gy.to_string(),
        "--affine",
        &a,
    ]);
    let (px, py) = pair(&back);
    assert!(
        close(px, probe.x, 1e-6) && close(py, probe.y, 1e-6),
        "round trip returned {back:?}, probe was {},{}",
        probe.x,
        probe.y
    );
}

/// 68: `--geo-origin` and `--geo-scale` are the same pair `viprs pyramid`
/// takes, and mean what `GeoTransform::from_origin_and_scale` means.
#[test]
fn geo_68_origin_and_scale_match_the_library_and_the_flags_are_live() {
    if skip_if_no_cli("geo_68_origin_scale") {
        return;
    }
    let t = GeoTransform::from_origin_and_scale(GeoCoord::new(-122.5, 37.75), 0.001, -0.002);
    let want = t.pixel_to_geo(PixelCoord { x: 640.0, y: 480.0 });
    let got = pair(&ok(&[
        "geo",
        "pixel-to-geo",
        "640",
        "480",
        "--geo-origin",
        "-122.5,37.75",
        "--geo-scale",
        "0.001,-0.002",
    ]));
    assert!(
        close(got.0, want.x, 1e-9) && close(got.1, want.y, 1e-9),
        "{got:?} vs {want:?}"
    );

    let moved = pair(&ok(&[
        "geo",
        "pixel-to-geo",
        "640",
        "480",
        "--geo-origin",
        "10,20",
        "--geo-scale",
        "0.001,-0.002",
    ]));
    assert!(!close(moved.0, got.0, 1e-6), "--geo-origin was ignored");
    let scaled = pair(&ok(&[
        "geo",
        "pixel-to-geo",
        "640",
        "480",
        "--geo-origin",
        "-122.5,37.75",
        "--geo-scale",
        "0.01,-0.002",
    ]));
    assert!(!close(scaled.0, got.0, 1e-6), "--geo-scale was ignored");
}

/// 68: `tile-center` is the library's, and `--tile-size` is live.
#[test]
fn geo_68_tile_center_matches_the_library() {
    if skip_if_no_cli("geo_68_tile_center") {
        return;
    }
    let a = affine_arg();
    let want = affine().tile_center(3, 2, 256);
    let got = pair(&ok(&[
        "geo",
        "tile-center",
        "3",
        "2",
        "--tile-size",
        "256",
        "--affine",
        &a,
    ]));
    assert!(
        close(got.0, want.x, 1e-9) && close(got.1, want.y, 1e-9),
        "{got:?} vs {want:?}"
    );

    let want512 = affine().tile_center(3, 2, 512);
    let got512 = pair(&ok(&[
        "geo",
        "tile-center",
        "3",
        "2",
        "--tile-size",
        "512",
        "--affine",
        &a,
    ]));
    assert!(
        close(got512.0, want512.x, 1e-9) && close(got512.1, want512.y, 1e-9),
        "{got512:?} vs {want512:?}"
    );
    assert!(!close(got.0, got512.0, 1e-6), "--tile-size was ignored");
}

/// 68: a transform that cannot be inverted is exit 1 for `geo-to-pixel`, not a
/// NaN on stdout.
#[test]
fn geo_68_singular_transform_cannot_be_inverted() {
    if skip_if_no_cli("geo_68_singular") {
        return;
    }
    let out = exits(
        1,
        &["geo", "geo-to-pixel", "1", "2", "--affine", "1,2,0,2,4,0"],
    );
    assert!(
        !stdout(&out).to_lowercase().contains("nan"),
        "nothing numeric may be printed for a singular transform"
    );
    assert!(
        stderr(&out).to_lowercase().contains("invert")
            || stderr(&out).to_lowercase().contains("singular"),
        "stderr must say why: {}",
        stderr(&out)
    );
}

/// 68: `viprs pyramid --geo-origin` takes a negative longitude. Found while
/// writing the `geo` cells: `-122.5,37.75` starts with a hyphen, so clap read it
/// as an unknown flag and every western-hemisphere origin had to be spelled
/// `--geo-origin=-122.5,37.75`. The pair has to mean what the library's
/// `from_origin_and_scale` means, so the printed bounds are compared with it.
#[test]
fn geo_68_pyramid_accepts_a_negative_geo_origin() {
    if skip_if_no_cli("geo_68_pyramid_negative_origin") {
        return;
    }
    let dir = TempDir::new().unwrap();
    let src = common::fixtures::canonical_raster_scaled(640, 480);
    let input = dir.path().join("in.png");
    image::save_buffer(
        &input,
        src.data(),
        src.width(),
        src.height(),
        match src.format() {
            PixelFormat::Rgb8 => image::ColorType::Rgb8,
            PixelFormat::Rgba8 => image::ColorType::Rgba8,
            other => panic!("the canonical fixture is {other:?}; teach this cell about it"),
        },
    )
    .expect("write the PNG the CLI will read");
    let tiles = dir.path().join("tiles");

    let out = run_viprs(&[
        "pyramid",
        s(&input),
        s(&tiles),
        "--storage",
        "directory",
        "--geo-origin",
        "-122.5,37.75",
        "--geo-scale",
        "0.001,-0.002",
    ]);
    assert!(
        out.status.success(),
        "a negative --geo-origin must parse\n--- stderr ---\n{}",
        stderr(&out)
    );
    let b = GeoTransform::from_origin_and_scale(GeoCoord::new(-122.5, 37.75), 0.001, -0.002)
        .image_bounds(640, 480);
    let want = format!(
        "Geo bounds: ({:.6}, {:.6}) \u{2192} ({:.6}, {:.6})",
        b.min.x, b.min.y, b.max.x, b.max.y
    );
    assert!(
        stderr(&out).contains(&want),
        "expected {want:?} in stderr, got: {}",
        stderr(&out)
    );
}

// ===========================================================================
// plan queries
// ===========================================================================

fn planner(w: u32, h: u32, tile: u32, overlap: u32, layout: Layout) -> libviprs::PyramidPlan {
    PyramidPlanner::new(w, h, tile, overlap, layout)
        .expect("valid plan")
        .plan()
}

fn plan_args<'a>(layout: &'a str, overlap: &'a str) -> Vec<&'a str> {
    vec![
        "plan",
        "5000",
        "--height",
        "3000",
        "--tile-size",
        "256",
        "--overlap",
        overlap,
        "--layout",
        layout,
    ]
}

fn with<'a>(mut base: Vec<&'a str>, extra: &[&'a str]) -> Vec<&'a str> {
    base.extend_from_slice(extra);
    base
}

/// 68: `--estimate-memory` agrees with `estimate_streaming_peak_memory`, on a
/// numeric size and on the dimensions of a committed PDF, and both of its
/// inputs (the strip height and the pixel format) are live.
#[test]
fn plan_68_estimate_memory_agrees_with_the_library() {
    if skip_if_no_cli("plan_68_estimate_memory") {
        return;
    }
    let p = planner(5000, 3000, 256, 0, Layout::DeepZoom);
    let want = p.estimate_streaming_peak_memory(PixelFormat::Rgba8, 256);
    let got = ok(&with(
        plan_args("deep-zoom", "0"),
        &["--estimate-memory", "256"],
    ));
    assert_eq!(
        got.trim(),
        want.to_string(),
        "default pixel format is rgba8"
    );

    let want_strip = p.estimate_streaming_peak_memory(PixelFormat::Rgba8, 64);
    let got_strip = ok(&with(
        plan_args("deep-zoom", "0"),
        &["--estimate-memory", "64"],
    ));
    assert_eq!(got_strip.trim(), want_strip.to_string());
    assert_ne!(
        want, want_strip,
        "premise: strip height changes the estimate"
    );

    for (name, format) in [
        ("rgb8", PixelFormat::Rgb8),
        ("gray8", PixelFormat::Gray8),
        ("rgba16", PixelFormat::Rgba16),
    ] {
        let want_fmt = p.estimate_streaming_peak_memory(format, 256);
        let got_fmt = ok(&with(
            plan_args("deep-zoom", "0"),
            &["--estimate-memory", "256", "--pixel-format", name],
        ));
        assert_eq!(
            got_fmt.trim(),
            want_fmt.to_string(),
            "--pixel-format {name}"
        );
        assert_ne!(want_fmt, want, "premise: {name} differs from rgba8");
    }

    // A committed plan source: the canvas comes from the PDF's page size.
    let info = libviprs::pdf_info(&fixture("canonical.pdf")).expect("canonical.pdf info");
    let page = &info.pages[0];
    let (w, h) = (page.width_pts as u32, page.height_pts as u32);
    let from_pdf = planner(w, h, 256, 0, Layout::DeepZoom);
    let pdf = fixture("canonical.pdf");
    let got_pdf = ok(&["plan", s(&pdf), "--estimate-memory", "128"]);
    assert_eq!(
        got_pdf.trim(),
        from_pdf
            .estimate_streaming_peak_memory(PixelFormat::Rgba8, 128)
            .to_string()
    );
}

/// 68: `--dzi-manifest[=FORMAT]` is the library's `.dzi`, and the format is
/// live. Only Deep Zoom has one. The format takes an `=`: a bare
/// `--dzi-manifest` means png and never swallows the next word, so
/// `--dzi-manifest png` leaves `png` as a stray argument (exit 2) instead of
/// quietly taking it, and the input given after the flag stays the input.
#[test]
fn plan_68_dzi_manifest_matches_the_library() {
    if skip_if_no_cli("plan_68_dzi_manifest") {
        return;
    }
    let p = planner(5000, 3000, 256, 2, Layout::DeepZoom);
    let png = p.dzi_manifest("png").unwrap();
    let jpg = p.dzi_manifest("jpg").unwrap();
    assert_ne!(png, jpg);

    let base = plan_args("deep-zoom", "2");
    assert_eq!(
        ok(&with(base.clone(), &["--dzi-manifest=png"])).trim(),
        png.trim()
    );
    assert_eq!(
        ok(&with(base.clone(), &["--dzi-manifest=jpg"])).trim(),
        jpg.trim()
    );
    // Bare flag means the default format.
    assert_eq!(
        ok(&with(base.clone(), &["--dzi-manifest"])).trim(),
        png.trim()
    );
    // And it never takes the next word as its format.
    exits(2, &with(base, &["--dzi-manifest", "jpg"]));

    let out = exits(1, &with(plan_args("xyz", "0"), &["--dzi-manifest=png"]));
    assert!(
        stderr(&out).contains("no .dzi manifest"),
        "a layout without a manifest must say so: {}",
        stderr(&out)
    );
}

/// 68: `--properties-sidecar EXT` prints the sidecar's relative path on the
/// first line and its content after, as the library returns them, for the two
/// layouts that have one.
#[test]
fn plan_68_properties_sidecar_matches_the_library() {
    if skip_if_no_cli("plan_68_properties_sidecar") {
        return;
    }
    for (flag, layout) in [("zoomify", Layout::Zoomify), ("iiif", Layout::Iiif)] {
        let p = planner(5000, 3000, 256, 0, layout);
        let (rel, body) = p.properties_sidecar("png").expect("layout has a sidecar");
        let text = ok(&with(
            plan_args(flag, "0"),
            &["--properties-sidecar", "png"],
        ));
        let (first, rest) = text.split_once('\n').expect("path line then content");
        assert_eq!(first, rel, "{flag}: relative path");
        assert_eq!(rest, body, "{flag}: content");
    }
    // The extension is live where the sidecar mentions it (IIIF lists formats).
    let (_, png) = planner(5000, 3000, 256, 0, Layout::Iiif)
        .properties_sidecar("png")
        .unwrap();
    let (_, jpg) = planner(5000, 3000, 256, 0, Layout::Iiif)
        .properties_sidecar("jpg")
        .unwrap();
    assert_ne!(
        png, jpg,
        "premise: the extension appears in the IIIF info.json"
    );
    let text = ok(&with(
        plan_args("iiif", "0"),
        &["--properties-sidecar", "jpg"],
    ));
    assert_eq!(text.split_once('\n').unwrap().1, jpg);

    // Layouts with no in-directory sidecar are exit 1.
    for flag in ["deep-zoom", "xyz", "google"] {
        exits(
            1,
            &with(plan_args(flag, "0"), &["--properties-sidecar", "png"]),
        );
    }
}

/// 68: `--tile-path L,COL,ROW` is the library's path for every layout, with
/// `--tile-ext` live, and an out-of-range coordinate is exit 1.
#[test]
fn plan_68_tile_path_matches_the_library() {
    if skip_if_no_cli("plan_68_tile_path") {
        return;
    }
    let layouts = [
        ("deep-zoom", Layout::DeepZoom),
        ("xyz", Layout::Xyz),
        ("google", Layout::Google),
        ("zoomify", Layout::Zoomify),
        ("iiif", Layout::Iiif),
    ];
    for (flag, layout) in layouts {
        let p = planner(5000, 3000, 256, 0, layout);
        let top = p.levels.len() as u32 - 1;
        let coord = TileCoord::new(top, 3, 2);
        let want = p.tile_path(coord, "png").expect("in range");
        let spec = format!("{top},3,2");
        let got = ok(&with(plan_args(flag, "0"), &["--tile-path", &spec]));
        assert_eq!(got.trim(), want, "{flag}");
        let want_jpg = p.tile_path(coord, "jpg").unwrap();
        let got_jpg = ok(&with(
            plan_args(flag, "0"),
            &["--tile-path", &spec, "--tile-ext", "jpg"],
        ));
        assert_eq!(got_jpg.trim(), want_jpg, "{flag} --tile-ext");
        assert_ne!(want, want_jpg, "premise: the extension is part of the path");
    }
    let out = exits(
        1,
        &with(plan_args("deep-zoom", "0"), &["--tile-path", "0,9,9"]),
    );
    assert!(
        stderr(&out).contains("out of range"),
        "an out-of-range tile must say so: {}",
        stderr(&out)
    );
}

/// 68: `--tile-rect L,COL,ROW` prints `x,y,width,height` as the library
/// computes it, including the overlap that makes an interior tile differ from
/// a corner one, and an out-of-range coordinate is exit 1.
#[test]
fn plan_68_tile_rect_matches_the_library() {
    if skip_if_no_cli("plan_68_tile_rect") {
        return;
    }
    let p = planner(5000, 3000, 256, 4, Layout::DeepZoom);
    let top = p.levels.len() as u32 - 1;
    let mut seen = std::collections::BTreeSet::new();
    for (col, row) in [(0u32, 0u32), (3, 2), (19, 11)] {
        let r = p
            .tile_rect(TileCoord::new(top, col, row))
            .expect("in range");
        let want = format!("{},{},{},{}", r.x, r.y, r.width, r.height);
        let spec = format!("{top},{col},{row}");
        let got = ok(&with(plan_args("deep-zoom", "4"), &["--tile-rect", &spec]));
        assert_eq!(got.trim(), want, "tile {spec}");
        seen.insert(want);
    }
    assert_eq!(
        seen.len(),
        3,
        "premise: corner, interior and edge tiles all differ"
    );

    // `--overlap` reaches the query: the same tile with no overlap is another rect.
    let flat = planner(5000, 3000, 256, 0, Layout::DeepZoom);
    let r = flat.tile_rect(TileCoord::new(top, 3, 2)).unwrap();
    let got = ok(&with(
        plan_args("deep-zoom", "0"),
        &["--tile-rect", &format!("{top},3,2")],
    ));
    assert_eq!(
        got.trim(),
        format!("{},{},{},{}", r.x, r.y, r.width, r.height)
    );
    assert!(!seen.contains(got.trim()), "overlap was ignored");

    exits(
        1,
        &with(plan_args("deep-zoom", "4"), &["--tile-rect", "0,9,9"]),
    );
}

/// 68: the query flags are one query at a time, and every one of them is in
/// `--help`, so none is a flag that exists only in the parser's imagination.
#[test]
fn plan_68_query_flags_are_exclusive_and_documented() {
    if skip_if_no_cli("plan_68_query_flags") {
        return;
    }
    let help = stdout(&run_viprs(&["plan", "--help"]));
    for flag in [
        "--estimate-memory",
        "--pixel-format",
        "--dzi-manifest",
        "--properties-sidecar",
        "--tile-path",
        "--tile-ext",
        "--tile-rect",
    ] {
        assert!(
            help.contains(flag),
            "`viprs plan --help` does not list {flag}:\n{help}"
        );
    }
    exits(
        2,
        &with(
            plan_args("deep-zoom", "0"),
            &["--tile-path", "0,0,0", "--tile-rect", "0,0,0"],
        ),
    );
    // A modifier without its query is a usage error, not a silent no-op.
    exits(
        2,
        &with(plan_args("deep-zoom", "0"), &["--tile-ext", "jpg"]),
    );
    exits(
        2,
        &with(plan_args("deep-zoom", "0"), &["--pixel-format", "rgb8"]),
    );
}

/// 68: with no query flag, `viprs plan` still prints the table it always did.
#[test]
fn plan_68_plain_plan_output_is_unchanged() {
    if skip_if_no_cli("plan_68_plain_plan") {
        return;
    }
    let text = ok(&plan_args("deep-zoom", "0"));
    assert!(text.contains("Image: 5000x3000"), "{text}");
    assert!(text.contains("Estimated peak memory:"), "{text}");
}
