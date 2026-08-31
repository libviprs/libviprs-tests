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
#                 The hook itself is a tracked file, tools/hooks/pre-push;
#                 what lands in .git/hooks is a shim that runs it
#                 (libviprs/libviprs#695).
#                 Runs against the working tree being pushed, which for a
#                 linked worktree is not the main checkout the hooks
#                 directory lives in (libviprs/libviprs#684). Only where the
#                 suite has a slot for the repo, which is libviprs and
#                 libviprs-tests (libviprs/libviprs#691).
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
    "$WORKSPACE_ROOT/pdfium-render"
)

# ---------------------------------------------------------------------------
# Per-repo cargo lint commands, in lockstep with the `cargo clippy` and
# `cargo fmt` lines in each repo's `.github/workflows/ci.yml`, feature matrix
# expanded. They run in order; the first failure aborts.
#
# `tests/install_hooks_mirror_ci.rs` enforces the lockstep by running the
# generated hook with a recording stand-in for `cargo` and comparing what it
# invokes against those workflow lines, so a list that drifts fails there
# naming the passes it lost. It used to be a comment asking a human to keep
# them in step, and the core's list sat at two of five passes for as long as
# that was true (libviprs/libviprs#715).
#
# `cargo clippy` already runs `cargo check`'s work, so we don't repeat the
# explicit `cargo check --all-targets` lines that CI also has — clippy
# covers them. The core's `cargo build --features s3` is also out: it is there
# to prove a deprecated alias still resolves, which the manifest settles
# without a whole extra build on every commit.
# ---------------------------------------------------------------------------

# libviprs lints the default cell and nine feature-gated ones. Each of the
# latter is in CI because the default pass compiles none of that code:
# object-store-sink (#382), svg (#502) and jxl (#500) are all non-default and
# all have `#[cfg(test)] mod tests` the default job never sees. jp2k, avif,
# packfile, serde and tracing joined the matrix after this list did (caught by
# tests/install_hooks_mirror_ci.rs going red on the COUNTERPART_REV bump that
# picked up the newer ci.yml, libviprs-tests#185).
LIBVIPRS_CARGO_STEPS=(
    "cargo clippy --all-targets -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features pdfium -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features object-store-sink -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features svg -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features jxl -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features jp2k -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features avif -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features packfile -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features serde -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features tracing -- -D warnings -W clippy::incompatible_msrv -W deprecated"
)

# libviprs-cli has no `pdfium` feature; one clippy pass.
LIBVIPRS_CLI_CARGO_STEPS=(
    "cargo clippy --all-targets -- -D warnings -W clippy::incompatible_msrv -W deprecated"
)

# libviprs-tests CI is plainer — no incompatible_msrv / deprecated lints.
# These mirror the ci.yml `feature-cells` matrix one-for-one so the enforced
# local mirror compiles and lints every feature cell, not just the default
# one. Without the per-feature passes a regression in the object-store-sink /
# packfile / tracing / jxl gated modules ships green locally because the
# default harness never compiles that code (the failure #55 targets). `s3`
# stood here in place of `object-store-sink` for a while, which is the same
# feature under a deprecated alias, so the `jxl` cell was the one going
# unlinted (libviprs/libviprs#715). The ported step is scoped through
# tools/run_ported_cells.sh because the three deferred codec cells do not
# compile yet (issue #77), so `--all-targets` cannot carry that feature.
LIBVIPRS_TESTS_CARGO_STEPS=(
    "cargo clippy --all-targets -- -D warnings"
    "cargo clippy --all-targets --features object-store-sink -- -D warnings"
    "cargo clippy --all-targets --features packfile -- -D warnings"
    "cargo clippy --all-targets --features tracing -- -D warnings"
    "cargo clippy --all-targets --features jxl -- -D warnings"
    "./tools/run_ported_cells.sh --clippy"
)

cargo_steps_for_repo() {
    case "$1" in
        libviprs)        printf '%s\n' "${LIBVIPRS_CARGO_STEPS[@]}" ;;
        libviprs-cli)    printf '%s\n' "${LIBVIPRS_CLI_CARGO_STEPS[@]}" ;;
        libviprs-tests)  printf '%s\n' "${LIBVIPRS_TESTS_CARGO_STEPS[@]}" ;;
        *)               printf '%s\n' "cargo clippy --all-targets -- -D warnings" ;;
    esac
}

# ---------------------------------------------------------------------------
# Which repos get the pre-push gate
# ---------------------------------------------------------------------------
# The gate runs tools/run-tests.sh, whose image holds the core crate and this
# harness and nothing else, so those two are the only repos where a green
# verdict says anything about the commits going out. libviprs-cli used to get
# the hook as well and paid a full image build plus a suite run for an answer
# that could not fail on its own change, which is exactly the trade that
# teaches people to reach for --no-verify (libviprs/libviprs#691).
#
# The cli is not left unguarded: the dedicated `cli-differential` CI job lays
# it down at CLI_COUNTERPART_REV and runs with VIPRS_REQUIRE_CLI=1, so a silent
# skip there is a hard panic (tests/common/cli.rs, CLI_CONTRACT.md §7). What
# goes away is a local promise the suite could not keep.
#
# tests/install_hooks_pre_push_slots.rs runs this script against a stand-in
# workspace and pins the answer per repo. It also asks run-tests.sh whether it
# has grown a cli slot, so giving the suite one turns that guard red pointing
# back here rather than leaving the two facts to drift.
has_suite_slot() {
    case "$1" in
        libviprs|libviprs-tests) return 0 ;;
        *)                       return 1 ;;
    esac
}

# The comment line every hook this script writes carries. It is what tells our
# own output apart from a hook somebody wrote by hand.
INSTALLER_MARKER="Installed by libviprs-tests/tools/install-hooks.sh"

# A repo that no longer gets the gate must not keep the copy an older version
# of this script left in it. Nothing but this script writes to .git/hooks, so
# "stop writing it" on its own leaves the misleading gate running in every
# clone it already reached, for as long as that clone lives.
drop_generated_pre_push() {
    local hooks_dir="$1"
    local pre_push="$hooks_dir/pre-push"

    [ -f "$pre_push" ] || return 0
    if grep -q "$INSTALLER_MARKER" "$pre_push"; then
        rm -f "$pre_push"
        echo "        removed the pre-push hook an older install left here"
    else
        echo "        left a pre-push hook this script did not write alone"
    fi
}

write_pre_commit() {
    local hooks_dir="$1"
    local repo_name="$2"
    local repo_dir="${3:-}"
    local pre_commit="$hooks_dir/pre-commit"

    # A repo that ships tools/local-ci.py gets a hook that runs THAT. It reads
    # .github/workflows/ci.yml at run time and executes whatever is in there,
    # inside a container carrying the toolchains the workflow asks for, so it
    # cannot drift from CI. The hardcoded per-repo lists further up are the
    # fallback for repos without one, and they are only ever a subset.
    if [ -n "$repo_dir" ] && [ -f "$repo_dir/tools/local-ci.py" ]; then
        cat > "$pre_commit" << 'LOCALCI'
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook: runs the fast half of this repo's real CI job list, in
# Docker, via tools/local-ci.py. That script derives its commands from
# .github/workflows/ci.yml, so this hook and CI cannot disagree.
# Installed by libviprs-tests/tools/install-hooks.sh.
# To skip (emergency only): git commit --no-verify

# A repo's main checkout and all of its linked worktrees share one hooks
# directory, and git invokes the hook with $0 inside that shared directory
# regardless of which worktree is actually being committed in, so deriving
# REPO_DIR from $0 always resolves to the main checkout (libviprs/libviprs#684,
# the same bug install_pre_push below already dodges). Ask git instead.
REPO_DIR="$(git rev-parse --show-toplevel)"
echo "Running the fast CI jobs locally (tools/local-ci.py --fast)..."
if ! python3 "$REPO_DIR/tools/local-ci.py" --fast; then
    echo ""
    echo "Failed. These are the real CI commands, so CI will fail the same way."
    echo "Run everything including tests: tools/local-ci.py"
    echo "Skip this hook once:            git commit --no-verify"
    exit 1
fi
echo "Passed. Tests and the integration job run on push,"
echo "or now with: tools/local-ci.py"
LOCALCI
        chmod +x "$pre_commit"
        return
    fi

    {
        cat << 'HEAD'
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook: a SUBSET of this repo's CI Check & Lint job, not all of
# it. A repo with tools/local-ci.py gets the real thing instead.
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

# The pre-push hook is not generated. It is a real file at tools/hooks/pre-push
# and this writes a shim that runs it, so a linter can see it, the guards can
# execute it, and a pull into this checkout updates the hook in every repo
# pointing at it instead of needing a redeploy per fix
# (libviprs/libviprs#695).
PRE_PUSH_HOOK="$SCRIPT_DIR/hooks/pre-push"

install_pre_push() {
    local hooks_dir="$1"
    local pre_push="$hooks_dir/pre-push"

    if [ ! -x "$PRE_PUSH_HOOK" ]; then
        echo "Error: no executable pre-push hook at $PRE_PUSH_HOOK." >&2
        echo "It is tracked in this repo; check the checkout is complete and" >&2
        echo "that the executable bit survived." >&2
        exit 1
    fi

    # Unquoted marker: the installed path is baked in here, once, at install
    # time. Nothing else in this chunk expands.
    cat > "$pre_push" << HOOK
#!/usr/bin/env bash
# Pre-push shim. Installed by libviprs-tests/tools/install-hooks.sh
# To skip (emergency only): git push --no-verify
#
# No behaviour lives here on purpose. The hook is checked in at
# libviprs-tests/tools/hooks/pre-push (libviprs/libviprs#695); this only points
# at it, so a pull updates every repo at once and no clone can quietly run a
# vintage nobody can name.
HOOK_PATH="$PRE_PUSH_HOOK"
HOOK

    cat >> "$pre_push" << 'HOOK'

# A harness push runs the hook it is pushing, the same way it gates on the
# suite it is pushing (libviprs/libviprs#684). Only libviprs-tests carries this
# file, so for every other repo the test below is simply false.
TREE="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [ -n "$TREE" ] && [ -x "$TREE/tools/hooks/pre-push" ]; then
    HOOK_PATH="$TREE/tools/hooks/pre-push"
fi

# Installing by reference has one failure mode copying does not: the file can
# go away, and git skips a hook it cannot execute without printing anything. A
# gate that disappears in silence is the failure #683 and #684 were both about,
# so refuse the push and say where to look.
if [ ! -x "$HOOK_PATH" ]; then
    echo "pre-push: no executable hook at $HOOK_PATH" >&2
    echo "Re-run libviprs-tests/tools/install-hooks.sh, or restore that checkout." >&2
    echo "To push without a gate: git push --no-verify" >&2
    exit 1
fi

exec "$HOOK_PATH" "$@"
HOOK
    chmod +x "$pre_push"
}

# pdfium-render gets a different pre-commit hook because we run it as a
# fork: upstream pdfium-render carries hundreds of pre-existing clippy
# lints (~470 missing-safety-doc, ~22 doc-nested-refdefs, etc.), so a
# blanket `cargo clippy -- -D warnings` would block every commit including
# merges from upstream. The scoped variant runs clippy on the whole tree
# but filters to lints whose primary span sits on a line *this fork has
# actually changed* relative to its base on `upstream/master`. Net: strict
# on lints we introduce, silent on the ambient debt we don't own.
write_pdfium_pre_commit() {
    local hooks_dir="$1"
    local pre_commit="$hooks_dir/pre-commit"

    cat > "$pre_commit" << 'HOOK'
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook for the libviprs/pdfium-render fork.
# Installed by libviprs-tests/tools/install-hooks.sh — to update, edit
# the script and re-run it. To skip (emergency only):
#   git commit --no-verify

echo "Running pre-commit checks (fork-scoped)..."

echo "  cargo fmt --check..."
if ! cargo fmt -- --check; then
    echo ""
    echo "Formatting check failed. Run 'cargo fmt' and re-stage."
    exit 1
fi

# Determine fork base.
git fetch -q upstream master 2>/dev/null || true
if git rev-parse upstream/master >/dev/null 2>&1; then
    BASE_REF="upstream/master"
elif git rev-parse origin/master >/dev/null 2>&1; then
    BASE_REF="origin/master"
else
    echo "  warn: no upstream/master or origin/master found; skipping scoped clippy"
    echo "Pre-commit checks passed."
    exit 0
fi
BASE=$(git merge-base HEAD "$BASE_REF")

echo "  cargo clippy --all-targets (scoped to fork-changed lines)..."

if ! command -v python3 >/dev/null 2>&1; then
    echo "  warn: python3 not on PATH; skipping scoped clippy"
    echo "Pre-commit checks passed."
    exit 0
fi

python3 - "$BASE" <<'PY'
import json
import re
import subprocess
import sys

base = sys.argv[1]

# Build {file: set(line_numbers)} for lines our branch added or changed
# vs the fork base. `--unified=0` makes the hunk headers line up exactly
# with added lines (no surrounding context to confuse the math).
diff_proc = subprocess.run(
    ["git", "diff", "--unified=0", base, "HEAD", "--", "*.rs"],
    check=True,
    capture_output=True,
    text=True,
)

file_lines = {}
current_file = None
hunk_re = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")
for line in diff_proc.stdout.splitlines():
    if line.startswith("+++ b/"):
        current_file = line[6:]
    elif current_file:
        m = hunk_re.match(line)
        if m:
            start = int(m.group(1))
            cnt = int(m.group(2)) if m.group(2) is not None else 1
            if cnt > 0:
                file_lines.setdefault(current_file, set()).update(
                    range(start, start + cnt)
                )

if not file_lines:
    print("  no Rust line additions/changes vs " + base[:8] + "; skipping clippy scope check")
    sys.exit(0)

# Run clippy; consume its JSON line-stream.
clippy_proc = subprocess.run(
    ["cargo", "clippy", "--all-targets", "--message-format=json"],
    capture_output=True,
    text=True,
)

hits = []
for line in clippy_proc.stdout.splitlines():
    try:
        rec = json.loads(line)
    except json.JSONDecodeError:
        continue
    if rec.get("reason") != "compiler-message":
        continue
    msg = rec.get("message") or {}
    level = msg.get("level")
    if level not in ("warning", "error"):
        continue
    for span in msg.get("spans") or []:
        if not span.get("is_primary"):
            continue
        f = span.get("file_name")
        ls = span.get("line_start")
        le = span.get("line_end") or ls
        owned = file_lines.get(f)
        if owned and any(i in owned for i in range(ls, le + 1)):
            hits.append(
                f"  {level}: {msg.get('message','')}\n"
                f"    --> {f}:{ls}:{span.get('column_start','?')}"
            )
            break

if hits:
    print("")
    print("Clippy lints on fork-changed lines:")
    print("\n".join(hits))
    print("")
    print("Fix and re-stage. (Pre-existing upstream lints on lines our fork")
    print("hasn't touched are ignored; this hook only blocks on lints whose")
    print("primary span sits on a line we added or changed.)")
    sys.exit(1)
PY

echo "Pre-commit checks passed."
HOOK
    chmod +x "$pre_commit"
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

    case "$REPO_NAME" in
        pdfium-render)
            # Heavily-rotted upstream → fork-scoped pre-commit only; no Docker
            # pre-push (no `run-tests.sh` integration on this repo).
            write_pdfium_pre_commit "$HOOKS_DIR"
            echo "  done: $REPO_NAME (scoped pre-commit)"
            drop_generated_pre_push "$HOOKS_DIR"
            ;;
        *)
            # The pre-commit hook goes everywhere: it runs the repo's own
            # `cargo fmt` and `cargo clippy` in the repo it is installed in, so
            # for the cli it gates the cli. $REPO_DIR lets it detect a
            # tools/local-ci.py in the target repo and defer to that instead.
            write_pre_commit "$HOOKS_DIR" "$REPO_NAME" "$REPO_DIR"
            if has_suite_slot "$REPO_NAME"; then
                install_pre_push "$HOOKS_DIR"
                echo "  done: $REPO_NAME (pre-commit + pre-push)"
            else
                echo "  done: $REPO_NAME (pre-commit only; the suite has no slot for it)"
                drop_generated_pre_push "$HOOKS_DIR"
            fi
            ;;
    esac
    installed=$((installed + 1))
done

echo ""
echo "Installed hooks in $installed repo(s), skipped $skipped."
