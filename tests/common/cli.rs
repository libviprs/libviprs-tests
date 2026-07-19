//! CLI-DIFFERENTIAL harness (`CLI_CONTRACT.md` §7).
//!
//! The vips-differential tests live in this repo but exercise the SEPARATE
//! `libviprs-cli` crate (`viprs` binary). This module locates that crate,
//! builds the binary **once per test process**, runs it, and decode-compares
//! its output against the committed vips references under
//! `tests/fixtures/cli/`. The tests never run vips: the references are
//! generated offline by `libviprs-cli`-side / `tools/gen_cli_expected.sh` and
//! committed (CLI_CONTRACT.md §7).
//!
//! # Build-once discipline
//!
//! Each `tests/*.rs` file is a standalone integration-test binary, so a naïve
//! "build the CLI in `viprs_bin()`" would launch one `cargo build` per test
//! binary, concurrently, all racing on the same CLI `target/` directory. The
//! guard is two-layered:
//!
//! * a per-process [`OnceLock`] so a single binary builds at most once, and
//! * a cross-process advisory **lockfile** (`target/.viprs-build.lock`) so that
//!   when several test binaries start at once, exactly one runs `cargo build`
//!   and the rest wait, then observe the built binary. (`std::sync::Once` and
//!   cargo's own target lock do not, respectively, cross processes / prevent
//!   the thundering herd of spawned cargo processes.)
//!
//! `$VIPRS_BIN` short-circuits the whole thing to a pre-built binary; the CLI
//! directory is `$VIPRS_CLI_DIR` else `<manifest>/../libviprs-cli`.

#![allow(dead_code)]

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::time::Duration;

use libviprs::Raster;

/// Locate the `libviprs-cli` crate directory.
///
/// `$VIPRS_CLI_DIR` wins; otherwise `<CARGO_MANIFEST_DIR>/../libviprs-cli`, the
/// sibling checkout the CI `cli-differential` job (and a local worktree) lays
/// down next to the core counterpart.
pub fn cli_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VIPRS_CLI_DIR") {
        return PathBuf::from(dir);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../libviprs-cli")
}

/// Whether the CLI crate is physically present (a `Cargo.toml` under
/// [`cli_dir`]) OR a pre-built `$VIPRS_BIN` is set.
///
/// A missing CLI checkout is a **clear skip**, not a confusing build failure
/// (CLI_CONTRACT.md §7): the default CI `test` job and the Docker gate clone
/// only the core counterpart, so the differential cell skips there and runs in
/// the dedicated `cli-differential` job that lays the CLI down.
///
/// # False-green guard (`$VIPRS_REQUIRE_CLI`)
///
/// A skip reads to `cargo test` as a **pass**, so a `cli-differential` CI job
/// that silently fails to lay the CLI down would go green while comparing
/// nothing. When `$VIPRS_REQUIRE_CLI=1` (set on that dedicated job, unset on dev
/// machines and the default `test` job) this function **panics loudly** instead
/// of returning `false`, converting a would-be silent skip into a hard failure.
/// Dev machines and the default job leave the var unset and still skip cleanly.
pub fn cli_available() -> bool {
    if std::env::var_os("VIPRS_BIN").is_some() {
        return true;
    }
    if cli_dir().join("Cargo.toml").is_file() {
        return true;
    }
    if require_cli() {
        panic!(
            "VIPRS_REQUIRE_CLI=1 but the libviprs-cli sibling is absent at {} and \
             $VIPRS_BIN is unset: the CLI-differential job would SKIP every comparison \
             and report a false green. Ensure clone-cli-counterpart laid the sibling \
             down (or set $VIPRS_BIN/$VIPRS_CLI_DIR).",
            cli_dir().display(),
        );
    }
    false
}

/// Whether the caller demands the CLI actually run (`$VIPRS_REQUIRE_CLI=1`).
///
/// The dedicated `cli-differential` CI job sets this so a failure to lay the CLI
/// sibling down hard-fails rather than skipping to a false green (see
/// [`cli_available`]). Any other value, or an unset var, means "skip cleanly if
/// the CLI is absent" — the dev-machine and default-`test`-job behaviour.
pub fn require_cli() -> bool {
    std::env::var("VIPRS_REQUIRE_CLI").is_ok_and(|v| v == "1")
}

/// Path to the `cargo` executable (`$CARGO` under `cargo test`, else `cargo`).
fn cargo_bin() -> PathBuf {
    std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

/// Build the `viprs` binary once and return its path.
///
/// * `$VIPRS_BIN` — used verbatim (asserted to exist).
/// * otherwise build `<cli>/Cargo.toml` with `cargo build --release
///   --no-default-features --bin viprs` under the cross-process lock, and return
///   `<cli>/target/release/viprs`.
///
/// `--release` is mandated by CLI_CONTRACT.md §7 (`--release
/// --no-default-features`); it matters for the future BOUNDED-TOL / FOURIER
/// codegen whose numerics differ between debug and release. `--no-default-features`
/// disables the CLI crate's *own* default features — it does **not** turn off
/// pdfium (see [`build_viprs_once`] for why pdfium is compiled in regardless yet
/// never links or loads libpdfium for the morphology ops).
///
/// # Panics
///
/// If the CLI directory is absent (call [`cli_available`] first to skip), if
/// the build fails, or if the built binary is missing afterwards.
pub fn viprs_bin() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(build_viprs_once).clone()
}

fn build_viprs_once() -> PathBuf {
    if let Some(pre) = std::env::var_os("VIPRS_BIN") {
        let path = PathBuf::from(pre);
        assert!(
            path.is_file(),
            "$VIPRS_BIN points at {} which does not exist",
            path.display()
        );
        return path;
    }

    let cli = cli_dir();
    let manifest = cli.join("Cargo.toml");
    assert!(
        manifest.is_file(),
        "libviprs-cli not found at {} (set $VIPRS_CLI_DIR or $VIPRS_BIN); \
         call cli_available() to skip when the sibling checkout is absent",
        cli.display()
    );

    let target_dir = cli.join("target");
    let bin = cli.join("target/release/viprs");

    let _lock = BuildLock::acquire(target_dir.join(".viprs-build.lock"));

    // Short-circuit once we hold the lock: if a sibling test binary already
    // built `viprs` and it is newer than every CLI source, there is nothing to
    // rebuild — each waiter that acquires the lock must NOT re-invoke cargo
    // (which would otherwise re-run a no-op `cargo build` per test binary).
    if bin.is_file() && !cli_sources_newer_than(&bin, &cli) {
        return bin;
    }

    let status = Command::new(cargo_bin())
        // `--release` per CLI_CONTRACT.md §7. `--no-default-features` disables
        // the CLI crate's OWN default features; it does NOT disable pdfium,
        // which `libviprs-cli/Cargo.toml` depends on non-optionally
        // (`features=["pdfium"]`) — so pdfium IS compiled into this build. That
        // links fine with no libpdfium.so present because `pdfium-render` binds
        // libpdfium dynamically at RUNTIME, not at build/link time, and the
        // morphology ops never touch a PDF path so libpdfium is never loaded.
        .args([
            "build",
            "--release",
            "--no-default-features",
            "--bin",
            "viprs",
        ])
        .arg("--manifest-path")
        .arg(&manifest)
        // Do NOT inherit the tests-repo `-Dwarnings` RUSTFLAGS: we are building
        // a crate we do not own the lint-cleanliness of, and a stray warning on
        // a newer toolchain must not fail the differential build.
        .env_remove("RUSTFLAGS")
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn `cargo build` for viprs: {e}"));
    assert!(
        status.success(),
        "`cargo build --release --bin viprs` failed: {status}"
    );

    assert!(
        bin.is_file(),
        "viprs build reported success but {} is missing",
        bin.display()
    );
    bin
}

/// Whether any CLI source (the crate `Cargo.toml` or anything under `src/`) is
/// newer than the already-built `bin`, i.e. a rebuild is actually needed. A
/// conservative `true` on any I/O error (missing metadata) forces the rebuild.
fn cli_sources_newer_than(bin: &Path, cli: &Path) -> bool {
    let bin_mtime = match fs::metadata(bin).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return true,
    };
    let mut newer = false;
    let mut newer_than = |p: &Path| {
        if let Ok(t) = fs::metadata(p).and_then(|m| m.modified()) {
            if t > bin_mtime {
                newer = true;
            }
        }
    };
    newer_than(&cli.join("Cargo.toml"));
    // Walk src/ iteratively (no external deps).
    let mut stack = vec![cli.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(path),
                Ok(_) => newer_than(&path),
                Err(_) => return true,
            }
        }
    }
    newer
}

/// Run `viprs <args…>` and capture its [`Output`]. Callers pass absolute paths
/// so the child's working directory is irrelevant.
pub fn run_viprs(args: &[&str]) -> Output {
    Command::new(viprs_bin())
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to run viprs {args:?}: {e}"))
}

/// Assert `viprs` exited 0, returning its stdout as a `String`.
pub fn run_viprs_ok(args: &[&str]) -> String {
    let out = run_viprs(args);
    assert!(
        out.status.success(),
        "viprs {args:?} exited {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout).expect("viprs stdout is valid UTF-8")
}

/// Absolute path to a committed CLI fixture under `tests/fixtures/cli/`.
pub fn cli_fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/cli")
        .join(rel)
}

// ---------------------------------------------------------------------------
// Decode-compare (NEVER byte-compare: PNG/TIFF encoders differ).
// ---------------------------------------------------------------------------

/// Decode `actual` and `expected` via the libviprs decoder and assert they
/// match: identical dimensions, band count, and format class (float-ness +
/// byte depth), and per-sample **max-abs-diff ≤ `tol`** (`tol == 0.0` for the
/// EXACT oracle class, CLI_CONTRACT.md §5). Panics with a diagnostic diff.
pub fn decode_compare(actual: &Path, expected: &Path, tol: f64) {
    let a = decode(actual);
    let e = decode(expected);

    assert_eq!(
        (a.width(), a.height()),
        (e.width(), e.height()),
        "dimension mismatch: actual {}={}x{}, expected {}={}x{}",
        actual.display(),
        a.width(),
        a.height(),
        expected.display(),
        e.width(),
        e.height(),
    );
    assert_eq!(
        a.format().channels(),
        e.format().channels(),
        "band-count mismatch: actual {:?} ({} bands) vs expected {:?} ({} bands)",
        a.format(),
        a.format().channels(),
        e.format(),
        e.format().channels(),
    );
    assert_eq!(
        (a.format().is_float(), a.format().bytes_per_channel()),
        (e.format().is_float(), e.format().bytes_per_channel()),
        "format-class mismatch: actual {:?} vs expected {:?}",
        a.format(),
        e.format(),
    );

    let diff = max_abs_diff(&a, &e);
    assert!(
        diff <= tol,
        "sample mismatch: max-abs-diff {diff} > tol {tol}\n  actual   = {}\n  expected = {}\n  \
         format = {:?}",
        actual.display(),
        expected.display(),
        a.format(),
    );
}

/// Like [`decode_compare`], but ignores a `margin`-pixel border ring on every
/// side, comparing only the interior rectangle.
///
/// This isolates an op whose ONLY core-vs-vips divergence is edge handling
/// (e.g. `stdif`, where the core CLIPS the sliding window at the image border
/// while vips MIRRORS): the border ring is compared separately at the wider
/// documented tolerance, while the interior is held to the strict `tol` the
/// "interior is exact" claim demands — so an interior regression that a flat
/// whole-image tolerance would absorb still fails. Same dimension / band-count /
/// format-class assertions as [`decode_compare`]; panics with a diagnostic diff.
pub fn decode_compare_interior(actual: &Path, expected: &Path, margin: usize, tol: f64) {
    let a = decode(actual);
    let e = decode(expected);

    assert_eq!(
        (a.width(), a.height()),
        (e.width(), e.height()),
        "dimension mismatch: actual {}={}x{}, expected {}={}x{}",
        actual.display(),
        a.width(),
        a.height(),
        expected.display(),
        e.width(),
        e.height(),
    );
    assert_eq!(
        a.format().channels(),
        e.format().channels(),
        "band-count mismatch: actual {:?} vs expected {:?}",
        a.format(),
        e.format(),
    );
    assert_eq!(
        (a.format().is_float(), a.format().bytes_per_channel()),
        (e.format().is_float(), e.format().bytes_per_channel()),
        "format-class mismatch: actual {:?} vs expected {:?}",
        a.format(),
        e.format(),
    );
    assert!(
        a.width() as usize > 2 * margin && a.height() as usize > 2 * margin,
        "interior margin {margin} leaves no interior for a {}x{} image",
        a.width(),
        a.height(),
    );

    let diff = max_abs_diff_interior(&a, &e, margin);
    assert!(
        diff <= tol,
        "interior sample mismatch (margin {margin}): max-abs-diff {diff} > tol {tol}\n  \
         actual   = {}\n  expected = {}\n  format = {:?}",
        actual.display(),
        expected.display(),
        a.format(),
    );
}

/// Max per-sample absolute difference over the interior of two rasters,
/// excluding a `margin`-pixel border ring on every side. Mirrors
/// [`max_abs_diff`]'s sample decoding.
pub fn max_abs_diff_interior(a: &Raster, b: &Raster, margin: usize) -> f64 {
    let bpc = a.format().bytes_per_channel();
    let chans = a.format().channels();
    let (as_, bs_) = (a.stride(), b.stride());
    let (ad, bd) = (a.data(), b.data());
    let (w, h) = (a.width() as usize, a.height() as usize);
    let mut max = 0.0_f64;
    for y in margin..h.saturating_sub(margin) {
        for x in margin..w.saturating_sub(margin) {
            for c in 0..chans {
                let s = x * chans + c;
                let (ao, bo) = (y * as_ + s * bpc, y * bs_ + s * bpc);
                let (av, bv) = match (a.format().is_float(), bpc) {
                    (false, 1) => (f64::from(ad[ao]), f64::from(bd[bo])),
                    (false, 2) => (
                        f64::from(u16::from_ne_bytes([ad[ao], ad[ao + 1]])),
                        f64::from(u16::from_ne_bytes([bd[bo], bd[bo + 1]])),
                    ),
                    (true, 4) => (
                        f64::from(f32::from_ne_bytes([
                            ad[ao],
                            ad[ao + 1],
                            ad[ao + 2],
                            ad[ao + 3],
                        ])),
                        f64::from(f32::from_ne_bytes([
                            bd[bo],
                            bd[bo + 1],
                            bd[bo + 2],
                            bd[bo + 3],
                        ])),
                    ),
                    (f, n) => {
                        panic!("unsupported sample class (is_float={f}, bytes_per_channel={n})")
                    }
                };
                max = max.max((av - bv).abs());
            }
        }
    }
    max
}

/// Max per-sample absolute difference between two rasters of identical
/// dimensions / band-count / format-class (see [`decode_compare`]). Reads
/// samples as `u8`, `u16` (native-endian), or `f32` per byte depth, honouring
/// each raster's row stride.
pub fn max_abs_diff(a: &Raster, b: &Raster) -> f64 {
    let bpc = a.format().bytes_per_channel();
    let samples_per_row = a.width() as usize * a.format().channels();
    let (as_, bs_) = (a.stride(), b.stride());
    let (ad, bd) = (a.data(), b.data());
    let mut max = 0.0_f64;
    for y in 0..a.height() as usize {
        for s in 0..samples_per_row {
            let (ao, bo) = (y * as_ + s * bpc, y * bs_ + s * bpc);
            let (av, bv) = match (a.format().is_float(), bpc) {
                (false, 1) => (f64::from(ad[ao]), f64::from(bd[bo])),
                (false, 2) => (
                    f64::from(u16::from_ne_bytes([ad[ao], ad[ao + 1]])),
                    f64::from(u16::from_ne_bytes([bd[bo], bd[bo + 1]])),
                ),
                (true, 4) => (
                    f64::from(f32::from_ne_bytes([
                        ad[ao],
                        ad[ao + 1],
                        ad[ao + 2],
                        ad[ao + 3],
                    ])),
                    f64::from(f32::from_ne_bytes([
                        bd[bo],
                        bd[bo + 1],
                        bd[bo + 2],
                        bd[bo + 3],
                    ])),
                ),
                (f, n) => panic!("unsupported sample class (is_float={f}, bytes_per_channel={n})"),
            };
            max = max.max((av - bv).abs());
        }
    }
    max
}

fn decode(path: &Path) -> Raster {
    libviprs::decode_file(path)
        .unwrap_or_else(|e| panic!("failed to decode {} via libviprs: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Scalar (S3) compare.
// ---------------------------------------------------------------------------

/// Tolerance for **integer-valued** scalar comparisons — segment counts, pixel
/// counts, and any exact-integer stdout scalar: bit-exact (`rel_eps = 0`).
///
/// Use this ONLY when the reference is mathematically an integer (e.g.
/// `labelregions`' segment count). It is a copy-hazard to reuse it for a
/// floating-point statistic that merely *happens* to print a short dyadic
/// value: see [`SCALAR_S3_REL_EPS`].
pub const SCALAR_INT_EXACT: f64 = 0.0;

/// Relative epsilon for **floating** S3 scalar means / order statistics
/// (CLI_CONTRACT.md §3, scalar tol).
///
/// The S3 scalar ops (`avg`, `countlines`, …) return a real-valued mean. Such a
/// value is only bit-exact against vips when it lands on a dyadic rational
/// (e.g. `countlines` on this binary input yields `0.578125 = 37/64`); a
/// different input or op would produce a non-terminating binary fraction that
/// vips prints rounded to 6 places. Comparing those with `rel_eps = 0` is a
/// latent false-red. Use this documented `1e-6` epsilon for every floating S3
/// scalar and reserve [`SCALAR_INT_EXACT`] for genuinely integer samples/counts.
pub const SCALAR_S3_REL_EPS: f64 = 1e-6;

/// Float-parse a `viprs` S3 stdout scalar and assert it matches `expected`
/// within a relative epsilon (`rel_eps == 0.0` for a bit-exact match). Compares
/// numerically, never as text (CLI_CONTRACT.md §3: vips numeric format).
pub fn compare_scalar(stdout: &str, expected: f64, rel_eps: f64) {
    let got = parse_scalar(stdout);
    let denom = expected.abs().max(1.0);
    let rel = (got - expected).abs() / denom;
    assert!(
        rel <= rel_eps,
        "scalar mismatch: got {got}, expected {expected} (relative {rel} > {rel_eps})\n  \
         raw stdout = {stdout:?}",
    );
}

/// Parse the first whitespace-trimmed line of `viprs` stdout as an `f64`.
pub fn parse_scalar(stdout: &str) -> f64 {
    let tok = stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_else(|| panic!("no scalar line in viprs stdout {stdout:?}"));
    tok.parse::<f64>()
        .unwrap_or_else(|e| panic!("viprs scalar {tok:?} is not a float: {e}"))
}

/// Read a committed scalar reference file and parse it as an `f64`.
pub fn read_scalar_fixture(rel: &str) -> f64 {
    let path = cli_fixture(rel);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read scalar fixture {}: {e}", path.display()));
    parse_scalar(&text)
}

// ---------------------------------------------------------------------------
// Cross-process advisory build lock.
// ---------------------------------------------------------------------------

/// A best-effort cross-process advisory lock backed by atomic
/// `create_new` on a lockfile. Combined with cargo's own target-directory lock
/// (which prevents corruption) this serialises the `cargo build` so that only
/// one test binary of a parallel run actually compiles the CLI.
///
/// Waiters spin on `AlreadyExists`. There is **no timed "steal" of a live
/// lock**: cargo's own target-dir lock already prevents corruption, so a slow
/// build simply blocks the waiters until it finishes — stealing at a wall-clock
/// deadline would let a second cargo start while the first is mid-link (and
/// races to delete a lock the first holder still owns). The only lock we remove
/// is one whose mtime is far older than any plausible build (a crashed holder,
/// [`STALE_LOCK`]); its age guarantees we never delete a lock a live holder
/// just created.
struct BuildLock {
    path: PathBuf,
}

/// A lockfile whose mtime is older than this is treated as a crashed holder and
/// reclaimed. Kept well above any plausible `cargo build` wall time so a live,
/// slow build is never stolen from.
const STALE_LOCK: Duration = Duration::from_secs(1800);

impl BuildLock {
    fn acquire(path: PathBuf) -> BuildLock {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return BuildLock { path };
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Someone else holds the lock. Reclaim it ONLY if it is
                    // provably stale (crashed holder); otherwise wait.
                    if lockfile_is_stale(&path) {
                        let _ = fs::remove_file(&path);
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
                // Any other error (read-only / ENOSPC / EACCES target dir) is a
                // real fault: retrying forever would hang to the CI timeout with
                // no diagnostic. Fail loudly instead.
                Err(e) => panic!(
                    "cannot create the viprs build lockfile at {} ({e}); the CLI \
                     `target/` directory is not writable",
                    path.display()
                ),
            }
        }
    }
}

impl Drop for BuildLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lockfile_is_stale(path: &Path) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().unwrap_or_default() > STALE_LOCK)
        .unwrap_or(false)
}
