#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# install-hooks.sh — Install git hooks for all libviprs repos.
#
# Installs:
#   pre-commit  — cargo fmt --check + cargo clippy (fast, on every commit)
#   pre-push    — Docker test suite via run-tests.sh (slow, on every push)
#
# Usage:  ./tools/install-hooks.sh          # from libviprs-tests/
#         ./libviprs-tests/tools/install-hooks.sh  # from workspace root
#
# Repos detected automatically as siblings of the script's parent directory:
#   libviprs, libviprs-cli, libviprs-tests
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

REPOS=(
    "$WORKSPACE_ROOT/libviprs"
    "$WORKSPACE_ROOT/libviprs-cli"
    "$WORKSPACE_ROOT/libviprs-tests"
)

RUN_TESTS="$WORKSPACE_ROOT/libviprs-tests/tools/run-tests.sh"

installed=0
skipped=0

for REPO_DIR in "${REPOS[@]}"; do
    REPO_NAME="$(basename "$REPO_DIR")"
    HOOKS_DIR="$REPO_DIR/.git/hooks"

    if [ ! -d "$HOOKS_DIR" ]; then
        echo "  skip: $REPO_NAME (not a git repo)"
        skipped=$((skipped + 1))
        continue
    fi

    # --- pre-commit hook ---
    PRE_COMMIT="$HOOKS_DIR/pre-commit"
    cat > "$PRE_COMMIT" << 'HOOK'
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook: check formatting and lint before allowing commits.
# Installed by libviprs-tests/tools/install-hooks.sh
# To skip (emergency only): git commit --no-verify

echo "Running pre-commit checks..."

# Format check (fast — no compilation needed)
echo "  cargo fmt --check..."
if ! cargo fmt -- --check 2>/dev/null; then
    echo ""
    echo "Formatting check failed. Run 'cargo fmt' and re-stage."
    exit 1
fi

# Clippy (uses cached build artifacts, usually fast on incremental changes)
echo "  cargo clippy..."
if ! cargo clippy --all-targets -- -D warnings 2>/dev/null; then
    echo ""
    echo "Clippy check failed. Fix warnings and re-stage."
    exit 1
fi

echo "Pre-commit checks passed."
HOOK
    chmod +x "$PRE_COMMIT"

    # --- pre-push hook ---
    PRE_PUSH="$HOOKS_DIR/pre-push"
    cat > "$PRE_PUSH" << HOOK
#!/usr/bin/env bash
set -euo pipefail

# Pre-push hook: run Docker test suite before pushing.
# Installed by libviprs-tests/tools/install-hooks.sh
# To skip (emergency only): git push --no-verify

REPO_DIR="\$(cd "\$(dirname "\$0")/../.." && pwd)"
WORKSPACE_ROOT="\$(cd "\$REPO_DIR/.." && pwd)"
RUN_TESTS="\$WORKSPACE_ROOT/libviprs-tests/tools/run-tests.sh"

if [ ! -f "\$RUN_TESTS" ]; then
    echo "Warning: run-tests.sh not found at \$RUN_TESTS"
    echo "Skipping pre-push tests. Install libviprs-tests as a sibling directory."
    exit 0
fi

echo "Running pre-push test suite..."
"\$RUN_TESTS"

EXIT_CODE=\$?
if [ \$EXIT_CODE -ne 0 ]; then
    echo ""
    echo "Pre-push tests failed. Push aborted."
    echo "Fix the failures or use: git push --no-verify"
    exit 1
fi
HOOK
    chmod +x "$PRE_PUSH"

    echo "  done: $REPO_NAME (pre-commit + pre-push)"
    installed=$((installed + 1))
done

echo ""
echo "Installed hooks in $installed repo(s), skipped $skipped."
