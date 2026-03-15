#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# install-hooks.sh — Install git pre-push hooks for libviprs + libviprs-tests.
#
# Usage:  ./tools/install-hooks.sh
#
# Installs a pre-push hook in both sibling repos that runs the Docker
# test suite before allowing a push. Can be re-run safely.
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TESTS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_ROOT="$(cd "$TESTS_DIR/.." && pwd)"
LIBVIPRS_DIR="$WORKSPACE_ROOT/libviprs"

HOOK_CONTENT='#!/usr/bin/env bash
set -euo pipefail

# Pre-push hook: run Docker test suite before pushing.
# Installed by libviprs-tests/tools/install-hooks.sh
# To skip (emergency only): git push --no-verify

REPO_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
WORKSPACE_ROOT="$(cd "$REPO_DIR/.." && pwd)"
RUN_TESTS="$WORKSPACE_ROOT/libviprs-tests/tools/run-tests.sh"

if [ ! -f "$RUN_TESTS" ]; then
    echo "Warning: run-tests.sh not found at $RUN_TESTS"
    echo "Skipping pre-push tests. Install libviprs-tests as a sibling directory."
    exit 0
fi

echo "Running pre-push test suite..."
"$RUN_TESTS" test

EXIT_CODE=$?
if [ $EXIT_CODE -ne 0 ]; then
    echo ""
    echo "Pre-push tests failed. Push aborted."
    echo "Fix the failures or use: git push --no-verify"
    exit 1
fi
'

install_hook() {
    local repo_dir="$1"
    local repo_name="$2"
    local hooks_dir="$repo_dir/.git/hooks"

    if [ ! -d "$hooks_dir" ]; then
        echo "Skipping $repo_name: $hooks_dir not found"
        return
    fi

    local hook_path="$hooks_dir/pre-push"

    if [ -f "$hook_path" ]; then
        echo "Replacing existing pre-push hook in $repo_name"
    else
        echo "Installing pre-push hook in $repo_name"
    fi

    echo "$HOOK_CONTENT" > "$hook_path"
    chmod +x "$hook_path"
}

install_hook "$LIBVIPRS_DIR" "libviprs"
install_hook "$TESTS_DIR" "libviprs-tests"

echo "Done. Pre-push hooks installed."
echo "  Pushes will run: ./tools/run-tests.sh test"
echo "  To skip (emergency): git push --no-verify"
