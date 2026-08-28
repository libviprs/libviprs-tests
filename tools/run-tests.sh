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
#         ./run-tests.sh --plan           # print what would be built, run nothing
#
# Which trees get tested:
#         ./run-tests.sh --libviprs PATH        # core crate to build
#         ./run-tests.sh --libviprs-tests PATH  # test crate to build
#   or the LIBVIPRS_DIR / LIBVIPRS_TESTS_DIR environment variables. The core
#   crate defaults to the sibling of this script's grandparent directory and
#   the test crate to the checkout this script belongs to, which in the
#   expected layout are the two directories every existing caller already got.
#
# Runs libviprs unit tests and libviprs-tests integration tests, both with
# the pdfium feature enabled. Exit code reflects test results.
#
# --miri  Run Miri (requires nightly toolchain with miri component) on
#         libviprs after the Docker tests pass.
# --loom  Run Loom concurrency tests on libviprs after the Docker tests pass.
# --plan  Resolve and print the trees and the image, then exit without
#         touching Docker. The cheap way to check that a pre-push hook is
#         about to test the tree you think it is.
# ---------------------------------------------------------------------------

RUN_MIRI=false
RUN_LOOM=false
PRINT_PLAN=false
ARCH=""
LIBVIPRS_ARG=""
TESTS_ARG=""

usage() {
    awk '
        /^# -{20,}/ { fence++; if (fence == 2) exit; next }
        fence == 1 && /^#/ { line = $0; sub(/^# ?/, "", line); print line }
    ' "$0"
}

need_value() {
    if [ "$2" -lt 2 ]; then
        echo "Error: $1 needs a path argument."
        exit 1
    fi
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --miri)              RUN_MIRI=true ;;
        --loom)              RUN_LOOM=true ;;
        --plan|--dry-run)    PRINT_PLAN=true ;;
        -h|--help)           usage; exit 0 ;;
        --libviprs)          need_value "$1" "$#"; LIBVIPRS_ARG="$2"; shift ;;
        --libviprs=*)        LIBVIPRS_ARG="${1#--libviprs=}" ;;
        --libviprs-tests)    need_value "$1" "$#"; TESTS_ARG="$2"; shift ;;
        --libviprs-tests=*)  TESTS_ARG="${1#--libviprs-tests=}" ;;
        -*)
            echo "Error: unknown option '$1'."
            echo ""
            usage
            exit 1
            ;;
        *)                   ARCH="$1" ;;
    esac
    shift
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
# Resolve the trees under test
# ---------------------------------------------------------------------------
# Two different questions used to share one answer here. The script needs to
# know where it lives, to find its own helpers and its scratch directory. It
# also needs to know which working trees to test. Deriving the second from the
# first made the answer "the sibling checkouts", always, so a pre-push hook
# firing from a git worktree built `main` and never saw a line of the branch
# being pushed (libviprs/libviprs#684). The caller passes the trees now, and
# the sibling layout is only the default.
#
# Expected default layout:
#   workspace/
#     libviprs/          (core library)
#     libviprs-tests/    (integration tests + Dockerfile)
#
# Every path here is resolved with `pwd -P`, so the banner, the staged build
# context and anything comparing against them all name the same directory even
# when the workspace is reached through a symlink. A logical `pwd` would print
# the link and the physical path would be what actually got staged.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
SELF_DIR="$(cd "$SCRIPT_DIR/.." && pwd -P)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd -P)"

LIBVIPRS_DIR="${LIBVIPRS_ARG:-${LIBVIPRS_DIR:-$WORKSPACE_ROOT/libviprs}}"
# The test tree defaults to the checkout this script is part of, not to the
# sibling named libviprs-tests. In the layout above they are the same
# directory; run the script out of a worktree and only the first one is right.
TESTS_DIR="${TESTS_ARG:-${LIBVIPRS_TESTS_DIR:-$SELF_DIR}}"

if [ ! -d "$LIBVIPRS_DIR" ]; then
    echo "Error: libviprs/ not found at $LIBVIPRS_DIR"
    echo "Pass --libviprs PATH (or set LIBVIPRS_DIR), or use the sibling layout:"
    echo "  workspace/"
    echo "    libviprs/"
    echo "    libviprs-tests/"
    exit 1
fi

if [ ! -d "$TESTS_DIR" ]; then
    echo "Error: libviprs-tests/ not found at $TESTS_DIR"
    echo "Pass --libviprs-tests PATH (or set LIBVIPRS_TESTS_DIR), or use the"
    echo "sibling layout:"
    echo "  workspace/"
    echo "    libviprs/"
    echo "    libviprs-tests/"
    exit 1
fi

LIBVIPRS_DIR="$(cd "$LIBVIPRS_DIR" && pwd -P)"
TESTS_DIR="$(cd "$TESTS_DIR" && pwd -P)"

if [ ! -f "$TESTS_DIR/Dockerfile" ]; then
    echo "Error: Dockerfile not found at $TESTS_DIR/Dockerfile"
    exit 1
fi

if [ ! -f "$LIBVIPRS_DIR/Cargo.toml" ]; then
    echo "Error: no Cargo.toml under $LIBVIPRS_DIR, so it is not a libviprs checkout."
    exit 1
fi

# git exports GIT_DIR, GIT_WORK_TREE and friends into every hook it runs, and
# they beat both `-C` and the working directory. Left in place they make `git`
# answer for the repository that is pushing rather than for the directory it
# was pointed at, which is the same class of lie as #684 itself: the banner
# below reported the pushing branch's HEAD as the revision of an unrelated
# tree. Ask cleanly.
git_at() {
    env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE -u GIT_PREFIX git "$@"
}

# A one-line description of a tree for the banner. The whole of #684 is that
# "which commit did that just test?" had no answer anywhere in the output, so
# print one, twice: before the run and again with the verdict.
tree_desc() {
    local dir="$1"
    local rev
    if rev="$(git_at -C "$dir" rev-parse --short HEAD 2>/dev/null)"; then
        if [ -n "$(git_at -C "$dir" status --porcelain 2>/dev/null | head -1)" ]; then
            rev="$rev+dirty"
        fi
        printf '%s' "$rev"
    else
        printf 'not a git checkout'
    fi
}

print_trees() {
    echo "  libviprs:       $LIBVIPRS_DIR ($(tree_desc "$LIBVIPRS_DIR"))"
    echo "  libviprs-tests: $TESTS_DIR ($(tree_desc "$TESTS_DIR"))"
}

echo "Test plan (${ARCH_LABEL}):"
print_trees
echo "  image:          $IMAGE_NAME"

if [ "$PRINT_PLAN" = true ]; then
    exit 0
fi

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
echo ""
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
# Build context
# ---------------------------------------------------------------------------
# The Dockerfile copies `libviprs/` and `libviprs-tests/` out of the context
# root, so pointing LIBVIPRS_DIR at a worktree is not enough on its own: the
# context has to hold the two trees under those two names. It used to be the
# workspace root, which only worked because the two names happen to be there,
# and which also shipped every other sibling in that directory to the daemon
# on every build — on this epic, ~150 lane worktrees of the same crate.
#
# Stage the two trees into a scratch context instead. It sits beside this
# script rather than beside the tree under test, so runs from different
# worktrees reuse the same warm copy. The exclusions here are a speed measure
# only: Dockerfile.dockerignore stays the authoritative ignore set, and Docker
# reads it from next to the -f Dockerfile, which comes from the tree under
# test.

CONTEXT_ROOT="${RUN_TESTS_CONTEXT_DIR:-$SELF_DIR/tmp/build-context}"

stage_tree() {
    local src="$1"
    local dst="$2"
    mkdir -p "$dst"
    if command -v rsync >/dev/null 2>&1; then
        rsync -a --delete \
            --exclude '/.git' \
            --exclude '/target/' \
            --exclude '/tmp/' \
            "$src/" "$dst/"
    else
        rm -rf "$dst"
        mkdir -p "$dst"
        ( cd "$src" && tar -cf - \
            --exclude ./.git --exclude ./target --exclude ./tmp . ) \
          | ( cd "$dst" && tar -xf - )
    fi
}

echo ""
echo "Staging build context at $CONTEXT_ROOT..."
mkdir -p "$CONTEXT_ROOT"
stage_tree "$LIBVIPRS_DIR" "$CONTEXT_ROOT/libviprs"
stage_tree "$TESTS_DIR" "$CONTEXT_ROOT/libviprs-tests"

# ---------------------------------------------------------------------------
# Stop any previous instance
# ---------------------------------------------------------------------------

if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    docker rm -f "$CONTAINER_NAME" >/dev/null
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

echo ""
echo "Building test image '${IMAGE_NAME}' (${ARCH_LABEL})..."
print_trees
DOCKER_BUILDKIT=1 docker build \
    --platform "$PLATFORM" \
    -f "$TESTS_DIR/Dockerfile" \
    -t "$IMAGE_NAME" \
    "$CONTEXT_ROOT"

# ---------------------------------------------------------------------------
# Run tests
# ---------------------------------------------------------------------------

echo ""
echo "Running tests (${ARCH_LABEL})..."
echo "================================================================"

# `set -e` used to take the script out at this line on a failing run, so
# everything below it, the verdict, the trees it was built from, and the
# container cleanup, never happened. The push aborted on docker's status with
# no report of what had been tested.
set +e
docker run \
    --platform "$PLATFORM" \
    --name "$CONTAINER_NAME" \
    --memory=4g \
    ${DOCKER_RUN_MOUNTS[@]+"${DOCKER_RUN_MOUNTS[@]}"} \
    "$IMAGE_NAME"
EXIT_CODE=$?
set -e

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------

docker rm "$CONTAINER_NAME" >/dev/null 2>&1 || true

if [ $EXIT_CODE -eq 0 ]; then
    echo ""
    echo "================================================================"
    echo "All tests passed (${ARCH_LABEL})."
    print_trees
else
    echo ""
    echo "================================================================"
    echo "Tests FAILED (exit code ${EXIT_CODE})."
    print_trees
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
