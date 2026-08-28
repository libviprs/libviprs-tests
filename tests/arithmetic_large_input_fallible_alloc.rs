//! Follow-up to libviprs/libviprs#339 (tracked by #350): PR #339 made each
//! arithmetic op's *output* allocation fallible (`alloc_op_output`, issue
//! #280) but left the far-larger *scratch* buffers inside `try_hough_circle`
//! and `try_stdif` as infallible `vec![..]` allocations, which call
//! `handle_alloc_error` and abort (SIGABRT) the whole process on an
//! attacker-influenced size — exactly the remote-DoS abort #280 set out to
//! remove (issues #433 / #434 / #435).
//!
//! The dominant hole is `try_hough_circle`'s vote accumulator
//! `acc = vec![0u32; w * h * radii]`: `radii` (= `max_radius - min_radius + 1`)
//! is caller-controlled up to `u16::MAX`, so a modest raster with a wide
//! radius range requests a multi-hundred-terabyte `vec!` that aborts before
//! the now-fallible output allocation is ever reached. These tests drive the
//! actual op (not just the `alloc_op_output` constructor boundary) at an
//! over-capacity scratch size and assert it returns a typed `Err` / panics
//! rather than aborting. Against the pristine tree they abort the process
//! (RED); once the scratch is routed through the fallible path they degrade to
//! `Err` (GREEN).
//!
//! `try_stdif` has the identical defect in its two `f64` integral-image
//! scratch buffers and is fixed by the same fallible helper, but its scratch
//! is only ~16x its input, so the input needed to overflow it (~8 TiB) exceeds
//! the 8 GiB `DEFAULT_MAX_ALLOC_BYTES` construction budget and cannot be built
//! in a test — the `hough_circle` accumulator, whose depth is caller-scaled,
//! is the case a test can actually drive.
//!
//! The abort-safety assertion is a pure behavioural contract with no libvips
//! analogue (vips-differential is N/A there). Alongside it we pin a
//! vips-differential reference for `Raster::mul`: libvips `multiply` on two
//! `uchar` images promotes to `ushort` with value `a*b`, which is exactly
//! what `mul` produces (`binary_map` widens to 16-bit, `a*b <= 65535` so no
//! saturation). The reference was produced offline with
//!   `vips multiply input_l.png input_r.png multiply_expected.png`
//! (vips-8.18.4) and committed under
//! `tests/fixtures/arithmetic_large_input_fallible_alloc_expected/`.

use libviprs::{ArithmeticError, PixelFormat, Raster, RasterError};

/// A raster whose `hough_circle` vote accumulator is far larger than this
/// process is allowed to have. A 24000×24000 Gray8 raster (~576 MB, well
/// within the 8 GiB construction budget) with a full `1..=65535` radius range
/// makes `acc = 24000 * 24000 * 65535 * 4 bytes ≈ 137 TiB`.
///
/// "Larger than the process is allowed to have" is deliberately not the same
/// claim as "larger than the allocator will hand out". 137 TiB is refused
/// outright on macOS and on x86-64 Linux, and it is *not* refused on aarch64
/// Linux, where the reservation fits in the 48-bit address space and the
/// kernel kills the process partway through zeroing it. That difference is
/// libviprs/libviprs#683 and it is why [`bounded_child`] exists: the cells run
/// under an `RLIMIT_AS` of their own, so the refusal is a property of the test
/// rather than a guess about the host. See its doc for the whole of it.
const OVERSIZE_DIM: u32 = 24_000;
const RADIUS_MIN: u32 = 1;
const RADIUS_MAX: u32 = 65_535;

/// A slot the two oversize cells take in turn.
///
/// Both build the same 24000x24000 Gray8 raster, ~576 MB apiece, and cargo
/// runs the cells of one binary on a thread pool, so the binary's resident
/// peak was twice what either cell needs. The slot is taken in the parent, so
/// it also serialises the bounded children below.
fn oversize_slot() -> std::sync::MutexGuard<'static, ()> {
    static SLOT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A poisoned slot still serialises. The second cell panics on purpose, so
    // refusing to hand the slot out after it would be the wrong answer.
    SLOT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Set in the child so it runs the cell body instead of spawning again.
const BOUNDED_CHILD: &str = "LIBVIPRS_TESTS_BOUNDED_ADDRESS_SPACE";

/// The child prints this before it execs the test binary, carrying whatever
/// `ulimit -v` reports *after* the attempt to set it. The parent quotes it in
/// every message, because "the child ran under a 2 GiB cap" and "the child ran
/// uncapped and the allocator refused instead" are two different results and
/// the run has to say which one it got. macOS returns `unlimited` here: its
/// `ulimit -v` fails with `Invalid argument` and the cell falls back on the
/// allocator refusal, which is exactly where it already was.
const LIMIT_MARKER: &str = "address-space-kib:";

/// The name of the enclosing `#[test] fn`, derived rather than typed.
///
/// The child is a re-exec of this binary with `--exact <name>`, and libtest
/// exits 0 when a filter matches nothing (`running 0 tests` / `test result:
/// ok. 0 passed`). A hand-typed literal that drifts from the function name
/// therefore leaves both cells permanently green having executed nothing,
/// which a reviewer demonstrated by changing one character. Nothing in the
/// toolchain ties a string literal to a function name, so take the name from
/// the function: `type_name` of a fn item declared inside it is
/// `<krate>::<test fn>::probe`. [`bounded_child`] also insists on seeing
/// `1 passed` come back, so if this ever resolves to something else the cell
/// goes red rather than quiet.
macro_rules! this_test_name {
    () => {{
        fn probe() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let path = type_name_of(probe);
        let path = path.strip_suffix("::probe").unwrap_or(path);
        path.rsplit("::").next().unwrap_or(path)
    }};
}

/// 2 GiB, in KiB, which is what `ulimit -v` wants. Room for the 576 MB raster
/// and the harness several times over, and nowhere near the ~137 TiB the vote
/// accumulator asks for.
const ADDRESS_SPACE_KIB: &str = "2097152";

/// Re-run one oversize cell in a child process whose address space is capped,
/// and return `false` in the parent once the child has been checked.
///
/// Both cells assert that an over-capacity scratch allocation is *refused*,
/// and they get there by asking `try_hough_circle` for a vote accumulator of
/// roughly 137 TiB. The premise underneath is that the allocator refuses a
/// reservation that large outright. That holds on macOS, which does not
/// over-commit, and on x86-64 Linux, whose 47-bit user address space (128
/// TiB) has nowhere to put the mapping. It does not hold on aarch64 Linux:
/// the 48-bit space (256 TiB) has room, `Vec::try_reserve_exact` succeeds,
/// `Vec::resize` starts writing zeros into it, and the kernel kills the
/// process somewhere past three gigabytes.
///
/// That is the `signal: 9, SIGKILL` in libviprs/libviprs#683, and it is not a
/// bug in libviprs. The size these cells pick simply is not over-capacity on
/// that host, so the fallible path they mean to exercise is never reached. It
/// went unnoticed because the same cells pass on the x86-64 GitHub runners
/// and on a macOS host, and the local Docker mirror is the one place that
/// runs them on aarch64 Linux.
///
/// So the cells stop asking the host what it will refuse and bring their own
/// ceiling. Each re-runs itself in a child with `RLIMIT_AS` set far below the
/// request, where the reservation is refused everywhere, immediately, with no
/// page ever touched. The contract under test is unchanged and the sizes stay
/// where they are: the accumulator is still caller-scaled and still far past
/// anything the process can have. What changes is that "past anything this
/// process can have" is now a property of the test rather than a guess about
/// the machine.
///
/// A host that ignores `ulimit -v` (Darwin does) lands back on the allocator,
/// which is where these cells already were and where they already pass. If it
/// ignores the limit *and* over-commits, the child dies rather than the whole
/// test binary, and the parent says which cell and why instead of leaving a
/// bare SIGKILL against a test name.
fn bounded_child(test_name: &str) -> bool {
    if std::env::var_os(BOUNDED_CHILD).is_some() {
        return true;
    }

    let exe = std::env::current_exe().expect("path of the running test binary");
    // `ulimit -v` is a no-op on Darwin and fails there with `Invalid argument`.
    // Swallowing that and carrying on is right, the allocator refusal still
    // holds, but the run has to say so, so read the limit back and print it
    // before the exec. Output is captured rather than streamed, so the parent
    // can both quote it and check that a test actually ran.
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -v {ADDRESS_SPACE_KIB} 2>/dev/null || :; \
             echo \"{LIMIT_MARKER} $(ulimit -v)\"; \
             exec \"$0\" \"$1\" --exact --test-threads=1"
        ))
        .arg(&exe)
        .arg(test_name)
        .env(BOUNDED_CHILD, "1")
        .output()
        .unwrap_or_else(|e| panic!("re-run {test_name} under a bounded address space: {e}"));

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let regime = address_space_regime(&stdout);
    let transcript = format!("--- child stdout ---\n{stdout}\n--- child stderr ---\n{stderr}\n---");

    assert!(
        stdout.contains(LIMIT_MARKER),
        "the child never reached the exec: it printed no `{LIMIT_MARKER}` line, so \
         nothing below can be said about {test_name}.\n{transcript}"
    );

    if !out.status.success() {
        panic!(
            "{test_name} did not pass in a child process ({}).\n{}\n{}",
            out.status,
            diagnose(&out.status, regime),
            transcript
        );
    }

    // libtest exits 0 when the filter matches nothing, so a successful status
    // on its own proves only that the binary ran. Insist on a test having
    // passed inside it.
    assert!(
        stdout.contains("1 passed"),
        "the child exited 0 but no test ran: `--exact {test_name}` matched \
         nothing, so this cell proved nothing. The name is derived from the \
         function by this_test_name!(), so if it no longer matches, libtest's \
         naming has moved.\n{transcript}"
    );
    false
}

/// What `ulimit -v` reported in the child, as a sentence for the messages.
fn address_space_regime(child_stdout: &str) -> &'static str {
    let reported = child_stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix(LIMIT_MARKER))
        .map(str::trim)
        .unwrap_or("");
    if reported == ADDRESS_SPACE_KIB {
        "the child ran under the RLIMIT_AS cap"
    } else if reported.is_empty() {
        "the child never reported its address-space limit"
    } else {
        "the cap did not apply (this host refused `ulimit -v`), so the child \
         ran uncapped and the assertion rests on the allocator refusing"
    }
}

/// A kill is not one thing. Signal 6 and signal 9 mean opposite things here and
/// the message used to name only one of them, the wrong one: reverting
/// `try_hough_circle`'s accumulator to an infallible `vec![0u32; acc_len]`,
/// which is the regression these cells exist to catch, kills the child with
/// SIGABRT rather than SIGKILL.
#[cfg(unix)]
fn diagnose(status: &std::process::ExitStatus, regime: &str) -> String {
    use std::os::unix::process::ExitStatusExt;
    match status.signal() {
        Some(6) => format!(
            "SIGABRT: the process aborted rather than returning. That is \
             `handle_alloc_error` firing on an infallible allocation, which is \
             the regression these cells exist to catch: the vote accumulator is \
             not going through the fallible path. ({regime}.)"
        ),
        Some(9) => format!(
            "SIGKILL: the process was killed rather than refused. Nothing \
             declined the {ADDRESS_SPACE_KIB} KiB request, so it was served \
             lazily and then written into until the kernel picked the process \
             off. ({regime}.)"
        ),
        Some(sig) => format!("killed by signal {sig}. ({regime}.)"),
        None => format!("the cell failed on its own assertions. ({regime}.)"),
    }
}

#[cfg(not(unix))]
fn diagnose(_status: &std::process::ExitStatus, regime: &str) -> String {
    format!("({regime}.)")
}

/// Directory holding the committed input fixtures and the offline libvips
/// reference output for the differential `mul` check.
fn fixture_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arithmetic_large_input_fallible_alloc_expected")
}

/// Load a committed 8-bit RGB PNG fixture as an `Rgb8` [`Raster`], reading it
/// through the `image` crate so the differential compares libviprs' `mul`
/// against libvips on byte-identical inputs.
fn load_rgb8(name: &str) -> Raster {
    let img = image::open(fixture_dir().join(name))
        .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
        .to_rgb8();
    let (w, h) = img.dimensions();
    Raster::new(w, h, PixelFormat::Rgb8, img.into_raw()).expect("fixture raster is well-formed")
}

/// Read a native-endian `u16` sample stream out of a raster buffer.
fn samples_u16(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
        .collect()
}

/// `try_hough_circle` with a caller-controlled radius range must allocate its
/// vote accumulator fallibly: an over-capacity scratch size returns a typed
/// `Err` (routed through `RasterError::AllocationFailed`), never a process
/// abort through `handle_alloc_error`.
#[test]
fn hough_circle_oversize_scratch_returns_typed_error_not_abort() {
    let _slot = oversize_slot();
    if !bounded_child(this_test_name!()) {
        return;
    }
    let raster = Raster::zeroed(OVERSIZE_DIM, OVERSIZE_DIM, PixelFormat::Gray8)
        .expect("oversize Gray8 raster is within the construction budget");
    // radii = 65535 is exactly the u16::MAX band ceiling, so `format_for`
    // succeeds and execution reaches the accumulator allocation — the buffer
    // this test is about — rather than short-circuiting on TooManyBands.
    let result = raster.try_hough_circle(RADIUS_MIN, RADIUS_MAX);
    // Naming the variant, not just `is_err()`. Under RLIMIT_AS the set of
    // things that can return `Err` here is much wider than it was, and an
    // early `if radii > K { return Err(..) }` added upstream would keep a bare
    // `is_err()` green while it stopped the accumulator being allocated at all.
    assert!(
        matches!(
            result,
            Err(ArithmeticError::Raster(
                RasterError::AllocationFailed { .. }
            ))
        ),
        "oversized hough_circle scratch must fail as \
         ArithmeticError::Raster(RasterError::AllocationFailed), which is the \
         fallible accumulator path. Got: {}",
        describe(&result)
    );
}

/// A one-line description of a `try_hough_circle` result for the message
/// above. `Raster` is large and its `Debug` prints the pixel buffer.
fn describe(result: &Result<Raster, ArithmeticError>) -> String {
    match result {
        Ok(r) => format!("Ok(a {}x{} {:?} raster)", r.width(), r.height(), r.format()),
        Err(e) => format!("Err({e:?})"),
    }
}

/// The same fallible-scratch guarantee, driven through the panicking wrapper:
/// `hough_circle` must surface the over-capacity scratch as a panic (its only
/// error channel) rather than aborting the process. A panic unwinds and is
/// catchable; an abort is not.
#[test]
fn hough_circle_oversize_scratch_panics_not_aborts() {
    let _slot = oversize_slot();
    if !bounded_child(this_test_name!()) {
        return;
    }
    // The raster is built *outside* catch_unwind on purpose. Under the 2 GiB
    // cap a 576 MB construction is comfortable but no longer unfailable, and
    // inside the closure a panicking `.expect(..)` would satisfy
    // `caught.is_err()` without `hough_circle` ever being called, so the cell
    // would pass having tested the constructor. Before this branch there was
    // no cap and that could not happen.
    let raster = Raster::zeroed(OVERSIZE_DIM, OVERSIZE_DIM, PixelFormat::Gray8)
        .expect("oversize Gray8 raster is within the construction budget");
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let caught = std::panic::catch_unwind(|| raster.hough_circle(RADIUS_MIN, RADIUS_MAX));
    std::panic::set_hook(prev);
    assert!(
        caught.is_err(),
        "oversized hough_circle scratch must panic (unwindable), not abort"
    );
}

/// vips-differential reference for `Raster::mul`: on byte-identical `uchar`
/// inputs, libviprs' `mul` must match libvips `multiply` sample-for-sample
/// (both promote to `ushort` and store `a*b`). Guards that the fallible-scratch
/// rework left the arithmetic result untouched.
#[test]
fn mul_matches_libvips_multiply_reference() {
    let left = load_rgb8("input_l.png");
    let right = load_rgb8("input_r.png");

    let product = left.mul(&right);
    assert_eq!(
        product.format(),
        PixelFormat::Rgb16,
        "mul of two 8-bit rasters promotes to 16-bit"
    );

    let expected = image::open(fixture_dir().join("multiply_expected.png"))
        .expect("read multiply_expected.png")
        .to_rgb16();
    let (ew, eh) = expected.dimensions();
    assert_eq!(
        (product.width(), product.height()),
        (ew, eh),
        "output dimensions match the libvips reference"
    );

    let got = samples_u16(product.data());
    let want: Vec<u16> = expected.into_raw();
    assert_eq!(
        got, want,
        "mul must equal the libvips multiply reference sample-for-sample"
    );
}
