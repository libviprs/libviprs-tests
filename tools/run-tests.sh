#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# run-tests.sh — Build and run libviprs + libviprs-tests in Docker with PDFium.
#
# Can be invoked from either the libviprs-tests/ or libviprs/ directory.
#
# Usage:  ./run-tests.sh                  # default: ci mode, auto-detect arch
#         ./run-tests.sh [arch] [mode]
#
# Architecture (first arg, optional):
#   arm, arm64       — build for arm64
#   amd64, x86_64    — build for amd64
#   (auto-detected if omitted or if first arg is a mode)
#
# Mode (second arg, or first arg if not an arch):
#   ci     (default) — fmt check, clippy, unit tests, integration tests
#   test             — unit tests + integration tests only
#   miri             — cargo +nightly miri test on libviprs
#   loom             — loom concurrency tests on libviprs
#   full             — ci + miri + loom (the full merge-gate pipeline)
#
# Examples:
#   ./run-tests.sh              # ci mode, auto-detect arch
#   ./run-tests.sh arm          # ci mode, arm64
#   ./run-tests.sh full         # full mode, auto-detect arch
#   ./run-tests.sh arm full     # full mode, arm64
#   ./run-tests.sh amd64 miri   # miri mode, amd64
# ---------------------------------------------------------------------------

VALID_MODES="ci test miri loom full"

is_mode() {
    for m in $VALID_MODES; do
        [ "$1" = "$m" ] && return 0
    done
    return 1
}

auto_detect_arch() {
    case "$(uname -m)" in
        arm64|aarch64) echo "arm64" ;;
        *)             echo "amd64" ;;
    esac
}

# Parse arguments
ARCH=""
MODE="ci"

for arg in "$@"; do
    if is_mode "$arg"; then
        MODE="$arg"
    elif [ -z "$ARCH" ]; then
        ARCH="$arg"
    else
        echo "Error: unexpected argument '${arg}'"
        echo "Usage: ./run-tests.sh [arch] [mode]"
        echo "  arch: arm|arm64|amd64|x86_64 (auto-detected if omitted)"
        echo "  mode: $VALID_MODES"
        exit 1
    fi
done

if [ -z "$ARCH" ]; then
    ARCH="$(auto_detect_arch)"
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

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_ROOT="$(cd "$TESTS_DIR/.." && pwd)"
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

if [ ! -f "$TESTS_DIR/tools/Dockerfile" ]; then
    echo "Error: Dockerfile not found at $TESTS_DIR/tools/Dockerfile"
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
    -f "$TESTS_DIR/tools/Dockerfile" \
    -t "$IMAGE_NAME" \
    "$WORKSPACE_ROOT"

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

echo ""
echo "Running mode '${MODE}' (${ARCH_LABEL})..."
echo "================================================================"

docker run \
    --platform "$PLATFORM" \
    --name "$CONTAINER_NAME" \
    --memory=4g \
    -e "RUN_MODE=${MODE}" \
    "$IMAGE_NAME"

EXIT_CODE=$?

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------

docker rm "$CONTAINER_NAME" >/dev/null 2>&1 || true

if [ $EXIT_CODE -eq 0 ]; then
    echo ""
    echo "================================================================"
    echo "All checks passed (mode: ${MODE}, arch: ${ARCH_LABEL})."
else
    echo ""
    echo "================================================================"
    echo "FAILED (mode: ${MODE}, exit code ${EXIT_CODE})."
fi

exit $EXIT_CODE
