#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# run_ported_cells.sh — Run the green subset of the ported_* test cells.
#
# The ported_* integration tests (feature = "ported_tests") are ports of the
# libvips reference test suite. Twelve cells compile and run green against
# the current core (224 tests, no per-test `#[ignore]` reds remaining).
# Two codec cells (ported_foreign, ported_connection) target an
# external-decoder surface the core does not expose yet and do not compile,
# so a blanket `cargo test --features ported_tests` build of every test
# target fails. This script is the single source of truth for the green-cell
# list: the Docker mirror (Dockerfile CMD via run-tests.sh), the pre-commit
# hooks (install-hooks.sh), and .github/workflows/ci.yml all call it, so
# promoting a cell to green is a one-file change. The deferred remainder (the
# two codec cells) is tracked in issue #77.
#
# After the ported cells this script also runs phase3_tracing under the
# `tracing` feature so the per-tile span tests (core PR #308, issue #83) are
# exercised in the mirror; the default no-tracing `cargo test` build cfg's
# those span tests out.
#
# ported_infrastructure runs single-threaded: its tests mutate process-global
# state (TMPDIR, the max-coord limit) that individual tests save and restore
# but cannot make atomic across concurrently running tests. The cell header
# documents this.
#
# Usage:
#   ./tools/run_ported_cells.sh                     # run tests; skip if fixtures missing
#   ./tools/run_ported_cells.sh --require-fixtures  # hard-fail if fixtures missing (CI)
#   ./tools/run_ported_cells.sh --clippy            # clippy -D warnings on the green cells
#
# Fixtures: the cells read the pinned libvips reference suite under
# tmp/libvips-reference-tests/ (populate with tools/fetch_reference_suite.sh).
# Without --require-fixtures a missing suite SKIPS the test run with a clear
# message instead of failing, so an offline pre-push hook stays usable; CI
# fetches first and passes --require-fixtures. tools/gen_fixtures.sh output is
# not needed here: the green cells use only the reference suite plus synthetic
# rasters generated in-test.
# ---------------------------------------------------------------------------

# Green cells that run with the default cargo-test thread pool.
PARALLEL_CELLS=(
    ported_arithmetic
    ported_colour
    ported_conversion
    ported_convolution
    ported_create
    ported_draw
    ported_histogram
    ported_iofuncs
    ported_morphology
    ported_mosaicing
    ported_resample
)
# Green cell that must run with --test-threads=1 (see header).
SERIAL_CELL="ported_infrastructure"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

MODE="test"
REQUIRE_FIXTURES=0
for arg in "$@"; do
    case "$arg" in
        --clippy)           MODE="clippy" ;;
        --require-fixtures) REQUIRE_FIXTURES=1 ;;
        *)
            echo "ERROR: unknown argument: $arg" >&2
            echo "Usage: $0 [--clippy] [--require-fixtures]" >&2
            exit 2
            ;;
    esac
done

# Target-selection flags for every green cell plus the fixture audit.
TARGETS=(--test fixture_audit)
for cell in "${PARALLEL_CELLS[@]}" "$SERIAL_CELL"; do
    TARGETS+=(--test "$cell")
done

if [ "$MODE" = "clippy" ]; then
    # Scoped to the green cells: --all-targets would also compile the three
    # deferred codec cells, which do not build yet (issue #77).
    echo "cargo clippy --features ported_tests (green ported cells) -D warnings..."
    cargo clippy --features ported_tests "${TARGETS[@]}" -- -D warnings
    exit 0
fi

IMAGES_DIR="$REPO_ROOT/tmp/libvips-reference-tests/test-suite/images"
if [ ! -d "$IMAGES_DIR" ]; then
    if [ "$REQUIRE_FIXTURES" -eq 1 ]; then
        echo "ERROR: libvips reference suite not found at $IMAGES_DIR" >&2
        echo "Run ./tools/fetch_reference_suite.sh first." >&2
        exit 1
    fi
    echo "SKIP: libvips reference suite not found at $IMAGES_DIR."
    echo "The ported_tests cells need it; run ./tools/fetch_reference_suite.sh"
    echo "(requires network). Skipping the ported cells for this run."
    exit 0
fi

echo "Running the green ported cells (--features ported_tests)..."

# fixture_audit first: it proves the fetched suite is complete at the pinned
# revision before the cells consume it. Then the ten parallel-safe cells in
# one invocation, then the serial cell.
PARALLEL_TARGETS=(--test fixture_audit)
for cell in "${PARALLEL_CELLS[@]}"; do
    PARALLEL_TARGETS+=(--test "$cell")
done

cargo test --features ported_tests "${PARALLEL_TARGETS[@]}"

echo ""
echo "Running $SERIAL_CELL with --test-threads=1 (mutates process-global state)..."
cargo test --features ported_tests --test "$SERIAL_CELL" -- --test-threads=1

# Exercise the per-tile tracing spans under the `tracing` feature. The default
# `cargo test` build compiles phase3_tracing WITHOUT `tracing`, so its span
# tests (emits_span_per_tile, tile_span_carries_coords) are cfg'd out and never
# run; only its no-tracing counter tests do. Run the whole cell under the
# feature here so the per-tile `libviprs::tile` span wired in core PR #308
# actually gates the mirror (issue #83). phase3_tracing reads only the
# checked-in canonical PNG, not the reference suite fetched above.
echo ""
echo "Running phase3_tracing under --features tracing (per-tile span tests)..."
cargo test --features "ported_tests tracing" --test phase3_tracing

echo ""
echo "Green ported cells passed."
