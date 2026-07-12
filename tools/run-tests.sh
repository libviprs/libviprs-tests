#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# run-tests.sh — Build and run libviprs + libviprs-tests in Docker with PDFium.
#
# Can be invoked from either the libviprs-tests/ or libviprs/ directory.
#
# Usage:  ./run-tests.sh                  # auto-detect arch (arm64 on Apple Silicon)
#         ./run-tests.sh arm              # build for arm64
#         ./run-tests.sh amd64            # build for amd64
#         ./run-tests.sh --miri           # also run Miri after Docker tests
#         ./run-tests.sh --loom           # also run Loom after Docker tests
#         ./run-tests.sh --miri --loom    # run both
#         ./run-tests.sh arm --miri       # combine arch + flags
#
# Runs libviprs unit tests and libviprs-tests integration tests, both with
# the pdfium feature enabled. Exit code reflects test results.
#
# --miri  Run Miri (requires nightly toolchain with miri component) on
#         libviprs after the Docker tests pass.
# --loom  Run Loom concurrency tests on libviprs after the Docker tests pass.
# ---------------------------------------------------------------------------

RUN_MIRI=false
RUN_LOOM=false
ARCH=""

for arg in "$@"; do
    case "$arg" in
        --miri) RUN_MIRI=true ;;
        --loom) RUN_LOOM=true ;;
        *)      ARCH="$arg" ;;
    esac
done

# Auto-detect architecture if not specified
if [ -z "$ARCH" ]; then
    HOST_ARCH="$(uname -m)"
    case "$HOST_ARCH" in
        arm64|aarch64) ARCH="arm64" ;;
        *)             ARCH="amd64" ;;
    esac
fi

case "$ARCH" in
    arm|arm64|aarch64)
        PLATFORM="linux/arm64"
        ARCH_LABEL="arm64"
        ;;
    amd64|x86_64|x64)
        PLATFORM="linux/amd64"
        ARCH_LABEL="amd64"
        ;;
    *)
        echo "Error: unsupported architecture '${ARCH}'. Use 'arm' or 'amd64'."
        exit 1
        ;;
esac

IMAGE_NAME="libviprs-tests:local"
CONTAINER_NAME="libviprs-tests-run"

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------

if ! docker info >/dev/null 2>&1; then
    echo "Warning: Docker is not running, attempting to start it..."
    open -a Docker 2>/dev/null || systemctl start docker.service 2>/dev/null || dockerd &>/dev/null &
    echo "Waiting for Docker to be ready..."
    while ! docker info >/dev/null 2>&1; do
        sleep 1
    done
    echo "Docker is running."
fi

# ---------------------------------------------------------------------------
# Resolve workspace layout
# ---------------------------------------------------------------------------
# Supports invocation from either libviprs-tests/ or libviprs/.
# Expected sibling layout:
#   workspace/
#     libviprs/          (core library)
#     libviprs-tests/    (integration tests + Dockerfile)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# The Dockerfile and .dockerignore live in libviprs-tests/
TESTS_DIR="$WORKSPACE_ROOT/libviprs-tests"
LIBVIPRS_DIR="$WORKSPACE_ROOT/libviprs"

if [ ! -d "$LIBVIPRS_DIR" ]; then
    echo "Error: libviprs/ not found at $LIBVIPRS_DIR"
    echo "Expected sibling layout:"
    echo "  workspace/"
    echo "    libviprs/"
    echo "    libviprs-tests/"
    exit 1
fi

if [ ! -d "$TESTS_DIR" ]; then
    echo "Error: libviprs-tests/ not found at $TESTS_DIR"
    echo "Expected sibling layout:"
    echo "  workspace/"
    echo "    libviprs/"
    echo "    libviprs-tests/"
    exit 1
fi

if [ ! -f "$TESTS_DIR/Dockerfile" ]; then
    echo "Error: Dockerfile not found at $TESTS_DIR/Dockerfile"
    exit 1
fi

# ---------------------------------------------------------------------------
# Reference fixtures for the ported_tests cells (idempotent, offline-tolerant)
# ---------------------------------------------------------------------------
# The green ported_* cells (tools/run_ported_cells.sh) compare against the
# pinned libvips reference suite under tmp/, which is git- and docker-ignored.
# Fetch it here on the host (the fetch script is a no-op when the pinned
# revision is already present) and hand it to the container as a read-only
# mount. Offline is not fatal: with a previously fetched copy the mount still
# happens, and with no copy at all the ported step inside the container skips
# with a clear message instead of failing, matching how the repo treats
# optional fixtures. gen_fixtures.sh is not needed for the ported cells; they
# use only the reference suite plus in-test synthetics.

FIXTURES_DIR="$TESTS_DIR/tmp/libvips-reference-tests"
echo "Fetching libvips reference suite (pinned, idempotent)..."
if ! "$TESTS_DIR/tools/fetch_reference_suite.sh"; then
    if [ -d "$FIXTURES_DIR/test-suite/images" ]; then
        echo "Warning: reference-suite fetch failed (offline?); using the existing copy."
    else
        echo "Warning: reference-suite fetch failed and no local copy exists."
        echo "The ported_tests step will be skipped inside the container."
    fi
fi

DOCKER_RUN_MOUNTS=()
if [ -d "$FIXTURES_DIR/test-suite/images" ]; then
    DOCKER_RUN_MOUNTS+=( -v "$FIXTURES_DIR:/src/libviprs-tests/tmp/libvips-reference-tests:ro" )
fi

# ---------------------------------------------------------------------------
# Stop any previous instance
# ---------------------------------------------------------------------------

if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    docker rm -f "$CONTAINER_NAME" >/dev/null
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

echo "Building test image '${IMAGE_NAME}' (${ARCH_LABEL})..."
echo "  libviprs:       $LIBVIPRS_DIR"
echo "  libviprs-tests: $TESTS_DIR"
DOCKER_BUILDKIT=1 docker build \
    --platform "$PLATFORM" \
    -f "$TESTS_DIR/Dockerfile" \
    -t "$IMAGE_NAME" \
    "$WORKSPACE_ROOT"

# ---------------------------------------------------------------------------
# Run tests
# ---------------------------------------------------------------------------

echo ""
echo "Running tests (${ARCH_LABEL})..."
echo "================================================================"

docker run \
    --platform "$PLATFORM" \
    --name "$CONTAINER_NAME" \
    --memory=4g \
    ${DOCKER_RUN_MOUNTS[@]+"${DOCKER_RUN_MOUNTS[@]}"} \
    "$IMAGE_NAME"

EXIT_CODE=$?

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------

docker rm "$CONTAINER_NAME" >/dev/null 2>&1 || true

if [ $EXIT_CODE -eq 0 ]; then
    echo ""
    echo "================================================================"
    echo "All tests passed (${ARCH_LABEL})."
else
    echo ""
    echo "================================================================"
    echo "Tests FAILED (exit code ${EXIT_CODE})."
    exit $EXIT_CODE
fi

# ---------------------------------------------------------------------------
# Miri (optional, runs on host — requires nightly + miri component)
# ---------------------------------------------------------------------------

if [ "$RUN_MIRI" = true ]; then
    echo ""
    echo "================================================================"
    echo "Running Miri on libviprs..."
    echo "================================================================"
    cd "$LIBVIPRS_DIR"
    cargo +nightly miri test
    echo ""
    echo "================================================================"
    echo "Miri passed."
fi

# ---------------------------------------------------------------------------
# Loom (optional, runs on host)
# ---------------------------------------------------------------------------

if [ "$RUN_LOOM" = true ]; then
    echo ""
    echo "================================================================"
    echo "Running Loom concurrency tests on libviprs..."
    echo "================================================================"
    cd "$LIBVIPRS_DIR"
    RUSTFLAGS="--cfg loom" cargo test --lib loom_tests
    echo ""
    echo "================================================================"
    echo "Loom passed."
fi
