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
# These mirror the ci.yml jobs one-for-one so the enforced local mirror
# compiles and lints every feature cell, not just the default one. Without
# the per-feature passes a regression in the s3 / packfile / tracing gated
# modules ships green locally because the default harness never compiles that
# code (the failure #55 targets). The ported step is scoped through
# tools/run_ported_cells.sh because the three deferred codec cells do not
# compile yet (issue #77), so `--all-targets` cannot carry that feature.
LIBVIPRS_TESTS_CARGO_STEPS=(
    "cargo clippy --all-targets -- -D warnings"
    "cargo clippy --all-targets --features s3 -- -D warnings"
    "cargo clippy --all-targets --features packfile -- -D warnings"
    "cargo clippy --all-targets --features tracing -- -D warnings"
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

# A repo's main checkout and all of its linked worktrees share one hooks
# directory, and git invokes the hook with $0 inside that shared directory.
# Anything resolved from $0 is therefore the main checkout, whichever tree is
# actually being pushed, which is how every lane worktree on epic #520 gated
# against `main` instead of against its own branch (libviprs/libviprs#684).
# A worktree's .git is a file holding a gitdir: pointer rather than a
# directory, so walking up from it does not work either. Ask git, which runs
# hooks from the top of the working tree whose commits are going out.
TREE="$(git rev-parse --show-toplevel)"

# --git-common-dir is the *main* checkout's .git for a linked worktree, and a
# path relative to here for the main checkout itself, so resolve it from the
# tree rather than assuming it is absolute. Its parent tells us which repo
# this is, and where the sibling libviprs-tests checkout lives.
MAIN_CHECKOUT="$(cd "$TREE" && cd "$(dirname "$(git rev-parse --git-common-dir)")" && pwd)"
REPO_NAME="$(basename "$MAIN_CHECKOUT")"
WORKSPACE_ROOT="$(cd "$MAIN_CHECKOUT/.." && pwd)"
RUN_TESTS="$WORKSPACE_ROOT/libviprs-tests/tools/run-tests.sh"

# Hand the pushed tree to the suite in whichever slot it belongs. Without
# this run-tests.sh falls back to the sibling checkouts, which is the whole
# defect. There are two slots and this hook is only installed into the two
# repos that have one (libviprs/libviprs#691).
# Both slots get pinned, not just the one being pushed. run-tests.sh falls
# back to the siblings of wherever the script itself sits, and the script the
# line below may pick sits inside the worktree, whose neighbours are other
# lanes rather than the workspace. Naming both leaves nothing to infer.
case "$REPO_NAME" in
    libviprs)
        export LIBVIPRS_DIR="$TREE"
        export LIBVIPRS_TESTS_DIR="$WORKSPACE_ROOT/libviprs-tests"
        ;;
    libviprs-tests)
        export LIBVIPRS_TESTS_DIR="$TREE"
        export LIBVIPRS_DIR="$WORKSPACE_ROOT/libviprs"
        # A push that changes the harness gates on the harness it changes,
        # not on the copy sitting in the main checkout.
        if [ -x "$TREE/tools/run-tests.sh" ]; then
            RUN_TESTS="$TREE/tools/run-tests.sh"
        fi
        ;;
esac

if [ ! -f "$RUN_TESTS" ]; then
    echo "Warning: run-tests.sh not found at $RUN_TESTS"
    echo "Skipping pre-push tests. Install libviprs-tests as a sibling directory."
    exit 0
fi

# ---------------------------------------------------------------------------
# Is there anything in this push the suite could see?
# ---------------------------------------------------------------------------
# A full image build and suite run for a two-file workflow edit is what taught
# everyone to reach for --no-verify, and a hook nobody runs protects nothing
# (libviprs/libviprs#683). So skip, but only where the answer is knowable:
# paths that no test in either repo reads. That list is deliberately short.
#
# README.md and CHANGELOG.md are NOT on it. This suite's documentation guards
# read both, and they read them out of the *core* checkout as well as its own
# (tests/feature_rename_docs_present.rs), so a docs-only push to either repo
# can genuinely fail. Two entries are per-repo for the same reason: .github/
# and .gitignore are inert for a libviprs push and read by a libviprs-tests
# one, whose pinning guards read .github/workflows/*.yml (tests/common/
# workflows.rs, tests/counterpart_pinning.rs) and whose provenance guard reads
# .gitignore for the native-binary patterns (tests/pdfium_provenance.rs, issue
# #56). A .gitignore-only push is exactly the push that can drop those
# patterns, so it is the last one that should skip the guard on them.
#
# tests/prepush_gate_cost_controls.rs runs every tracked path in both repos
# through this function for real and pins the set that comes back inert, so
# adding an entry here fails there with the files it would newly exempt.
#
# Set LIBVIPRS_PREPUSH_ALL=1 to run the suite whatever the paths say.

inert_path() {
    case "$1" in
        .github/*|.gitignore)
            [ "$REPO_NAME" != "libviprs-tests" ]
            ;;
        docs/*|.gitattributes|.editorconfig|LICENSE|LICENSE-*)
            true
            ;;
        *)
            false
            ;;
    esac
}

REF_UPDATES="$(cat)"
CHANGED=""
SKIP_SUITE=false

if [ "${LIBVIPRS_PREPUSH_ALL:-0}" != "1" ] && [ -n "$REF_UPDATES" ]; then
    UNKNOWN=false
    while read -r _local_ref local_oid _remote_ref remote_oid; do
        if [ -z "${local_oid:-}" ]; then
            continue
        fi
        # An all-zero local oid is a ref deletion: no content goes out.
        case "$local_oid" in
            *[!0]*) : ;;
            *)      continue ;;
        esac

        BASE=""
        case "${remote_oid:-}" in
            *[!0]*)
                if git cat-file -e "${remote_oid}^{commit}" 2>/dev/null; then
                    BASE="$remote_oid"
                fi
                ;;
        esac
        if [ -z "$BASE" ]; then
            # New branch. Compare against whatever the remote's default branch
            # already has, and give up rather than guess if that is not here.
            UPSTREAM="$(git symbolic-ref --quiet --short refs/remotes/origin/HEAD 2>/dev/null || true)"
            if [ -z "$UPSTREAM" ]; then
                UPSTREAM="origin/main"
            fi
            BASE="$(git merge-base "$local_oid" "$UPSTREAM" 2>/dev/null || true)"
        fi
        if [ -z "$BASE" ]; then
            UNKNOWN=true
            break
        fi

        # Unguarded this is a `set -e` exit: a git failure here would abort
        # the push with nothing printed. A range we cannot diff is a range we
        # cannot judge, which is the same answer as a base we could not find.
        if ! REF_PATHS="$(git diff --name-only "$BASE" "$local_oid" --)"; then
            UNKNOWN=true
            break
        fi
        CHANGED="$CHANGED
$REF_PATHS"
    done <<REFS
$REF_UPDATES
REFS

    if [ "$UNKNOWN" = false ]; then
        SKIP_SUITE=true
        while IFS= read -r changed_path; do
            if [ -z "$changed_path" ]; then
                continue
            fi
            if inert_path "$changed_path"; then
                continue
            fi
            SKIP_SUITE=false
            echo "Pre-push: $changed_path can reach the suite, running it."
            break
        done <<PATHS
$CHANGED
PATHS
    fi
fi

if [ "$SKIP_SUITE" = true ]; then
    echo "Pre-push: nothing in this push reaches the test suite, skipping it."
    printf '%s\n' "$CHANGED" | sed '/^$/d; s/^/  /'
    echo "  (LIBVIPRS_PREPUSH_ALL=1 runs it anyway)"
    exit 0
fi

# git hands every hook a GIT_DIR (and, in a worktree, one pointing at the
# per-worktree admin directory), and it wins over -C and over the working
# directory for every git command anything below runs. Leaving it set made the
# suite report this push's HEAD as the revision of trees it had never looked
# at. The paths above are already resolved, so drop it here.
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_PREFIX GIT_QUARANTINE_PATH

echo "Running pre-push test suite on $TREE..."
if ! "$RUN_TESTS"; then
    echo ""
    echo "Pre-push tests failed. Push aborted."
    echo "Fix the failures or use: git push --no-verify"
    exit 1
fi
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
            # for the cli it gates the cli.
            write_pre_commit "$HOOKS_DIR" "$REPO_NAME"
            if has_suite_slot "$REPO_NAME"; then
                write_pre_push "$HOOKS_DIR"
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
