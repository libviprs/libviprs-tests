#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Container entrypoint — dispatches based on RUN_MODE env var.
#
# Modes:
#   ci    (default) — fmt check, clippy, unit tests, integration tests
#   test            — unit tests + integration tests only
#   miri            — cargo +nightly miri test on libviprs
#   loom            — loom concurrency tests on libviprs
#   full            — ci + miri + loom
# ---------------------------------------------------------------------------

MODE="${RUN_MODE:-ci}"

banner() {
    echo ""
    echo "================================================================"
    echo "$1"
    echo "================================================================"
}

run_fmt() {
    banner "cargo fmt --check (libviprs)"
    cd /src/libviprs && cargo fmt -- --check

    banner "cargo fmt --check (libviprs-tests)"
    cd /src/libviprs-tests && cargo fmt -- --check
}

run_clippy() {
    banner "cargo clippy (libviprs)"
    cd /src/libviprs && RUSTFLAGS="-Dwarnings" cargo clippy --all-targets -- -D warnings

    banner "cargo clippy --features pdfium (libviprs)"
    cd /src/libviprs && RUSTFLAGS="-Dwarnings" cargo clippy --all-targets --features pdfium -- -D warnings

    banner "cargo clippy (libviprs-tests)"
    cd /src/libviprs-tests && RUSTFLAGS="-Dwarnings" cargo clippy --all-targets -- -D warnings

    banner "cargo clippy --features pdfium (libviprs-tests)"
    cd /src/libviprs-tests && RUSTFLAGS="-Dwarnings" cargo clippy --all-targets --features pdfium -- -D warnings
}

run_test() {
    banner "cargo test --features pdfium (libviprs)"
    cd /src/libviprs && cargo test --features pdfium

    banner "cargo test --features pdfium (libviprs-tests)"
    cd /src/libviprs-tests && cargo test --features pdfium
}

run_miri() {
    banner "cargo +nightly miri test (libviprs)"
    cd /src/libviprs && cargo +nightly miri test
}

run_loom() {
    banner "loom concurrency tests (libviprs)"
    cd /src/libviprs && RUSTFLAGS="--cfg loom" cargo test --lib loom_tests
}

case "$MODE" in
    ci)
        run_fmt
        run_clippy
        run_test
        ;;
    test)
        run_test
        ;;
    miri)
        run_miri
        ;;
    loom)
        run_loom
        ;;
    full)
        run_fmt
        run_clippy
        run_test
        run_miri
        run_loom
        ;;
    *)
        echo "Error: unknown RUN_MODE '${MODE}'"
        echo "Valid modes: ci, test, miri, loom, full"
        exit 1
        ;;
esac

banner "Done (mode: ${MODE})"
