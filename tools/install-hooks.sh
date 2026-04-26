#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# install-hooks.sh — Install git hooks for all libviprs repos.
#
# Installs:
#   pre-commit  — cargo fmt --check + cargo clippy (fast, on every commit)
#                 Mirrors each repo's `.github/workflows/ci.yml` Check & Lint
#                 job *exactly*, so a clean local commit means a clean remote
#                 Check & Lint. Update the per-repo cargo command lists below
#                 when a repo's CI matrix changes, and re-run this script.
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

# ---------------------------------------------------------------------------
# Per-repo cargo lint/check commands. Keep these in lockstep with the
# `cargo` lines under the Check & Lint / lint-and-build jobs in each repo's
# `.github/workflows/ci.yml`. They run in order; the first failure aborts.
#
# `cargo clippy` already runs `cargo check`'s work, so we don't repeat the
# explicit `cargo check --all-targets` lines that CI also has — clippy
# covers them.
# ---------------------------------------------------------------------------

# libviprs has both default-features and `--features pdfium` clippy passes.
LIBVIPRS_CARGO_STEPS=(
    "cargo clippy --all-targets -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features pdfium -- -D warnings -W clippy::incompatible_msrv -W deprecated"
)

# libviprs-cli has no `pdfium` feature; one clippy pass.
LIBVIPRS_CLI_CARGO_STEPS=(
    "cargo clippy --all-targets -- -D warnings -W clippy::incompatible_msrv -W deprecated"
)

# libviprs-tests CI is plainer — no incompatible_msrv / deprecated lints.
LIBVIPRS_TESTS_CARGO_STEPS=(
    "cargo clippy --all-targets -- -D warnings"
)

cargo_steps_for_repo() {
    case "$1" in
        libviprs)        printf '%s\n' "${LIBVIPRS_CARGO_STEPS[@]}" ;;
        libviprs-cli)    printf '%s\n' "${LIBVIPRS_CLI_CARGO_STEPS[@]}" ;;
        libviprs-tests)  printf '%s\n' "${LIBVIPRS_TESTS_CARGO_STEPS[@]}" ;;
        *)               printf '%s\n' "cargo clippy --all-targets -- -D warnings" ;;
    esac
}

write_pre_commit() {
    local hooks_dir="$1"
    local repo_name="$2"
    local pre_commit="$hooks_dir/pre-commit"

    {
        cat << 'HEAD'
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook: mirrors this repo's CI Check & Lint job exactly.
# Installed by libviprs-tests/tools/install-hooks.sh — to update, edit
# the script and re-run it. To skip (emergency only):
#   git commit --no-verify

echo "Running pre-commit checks (mirrors CI)..."

# Format check (fast — no compilation needed).
echo "  cargo fmt --check..."
if ! cargo fmt -- --check; then
    echo ""
    echo "Formatting check failed. Run 'cargo fmt' and re-stage."
    exit 1
fi

HEAD
        # Emit one cargo command per CI step so a failure prints exactly
        # which CI line it would have flunked.
        while IFS= read -r cmd; do
            [ -z "$cmd" ] && continue
            cat <<HOOK
echo "  $cmd"
if ! $cmd; then
    echo ""
    echo "Failed: $cmd"
    echo "Fix and re-stage. (This command mirrors a CI step; if it"
    echo "passes locally the matching CI step will too.)"
    exit 1
fi

HOOK
        done < <(cargo_steps_for_repo "$repo_name")

        echo 'echo "Pre-commit checks passed."'
    } > "$pre_commit"
    chmod +x "$pre_commit"
}

write_pre_push() {
    local hooks_dir="$1"
    local pre_push="$hooks_dir/pre-push"
    cat > "$pre_push" << 'HOOK'
#!/usr/bin/env bash
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
"$RUN_TESTS"

EXIT_CODE=$?
if [ $EXIT_CODE -ne 0 ]; then
    echo ""
    echo "Pre-push tests failed. Push aborted."
    echo "Fix the failures or use: git push --no-verify"
    exit 1
fi
HOOK
    chmod +x "$pre_push"
}

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

    write_pre_commit "$HOOKS_DIR" "$REPO_NAME"
    write_pre_push "$HOOKS_DIR"

    echo "  done: $REPO_NAME (pre-commit + pre-push)"
    installed=$((installed + 1))
done

echo ""
echo "Installed hooks in $installed repo(s), skipped $skipped."
