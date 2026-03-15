#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# run-tests.sh — Build and run libviprs + libviprs-tests in Docker with PDFium.
#
# Can be invoked from either the libviprs-tests/ or libviprs/ directory.
#
# Usage:  ./run-tests.sh          # auto-detect arch (arm64 on Apple Silicon)
#         ./run-tests.sh arm      # build for arm64
#         ./run-tests.sh amd64    # build for amd64
#
# Runs libviprs unit tests and libviprs-tests integration tests, both with
# the pdfium feature enabled. Exit code reflects test results.
# ---------------------------------------------------------------------------

# Auto-detect architecture if not specified
if [ $# -eq 0 ]; then
    HOST_ARCH="$(uname -m)"
    case "$HOST_ARCH" in
        arm64|aarch64) ARCH="arm64" ;;
        *)             ARCH="amd64" ;;
    esac
else
    ARCH="$1"
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
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

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
fi

exit $EXIT_CODE
