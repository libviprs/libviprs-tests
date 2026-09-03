#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# install-hooks.sh: install git hooks for every repo in the libviprs org.
#
# THE CONTRACT
#
# There are two hooks and the split between them is by cost, not by taste.
#
#   pre-commit  the LINT HALF of a repo's CI. `mirror_jobs` below names, per
#               repo, the CI jobs this hook stands in for. It runs every `run:`
#               step in those jobs, except the ones `exempt_steps` names with a
#               reason. So a clean pre-commit means those jobs will be clean.
#
#   pre-push    the TEST HALF: tools/run-tests.sh, in the two repos that suite
#               actually holds (`has_suite_slot`, libviprs/libviprs#691).
#
#   deferred_jobs names every CI job that is in neither half, and why. Between
#               the two lists every job in a repo's workflow is accounted for,
#               and tests/install_hooks_mirror_ci.rs goes red on a job that is
#               in neither, so a new CI job cannot slip past unnoticed.
#
# WHY THE SPLIT FALLS WHERE IT DOES
#
# Measured in this repo on 2026-09-02, warm target directory, Apple M-series:
# the lint half is 39s (fmt 1s, five clippy cells 36s, the ported clippy scope
# 1s, shellcheck 1s) and `cargo test` alone is 279s, before the pre-push gate
# has even built its image. A commit costs 39s and a push costs minutes, which
# is the trade that keeps people off `--no-verify` (libviprs/libviprs#683).
# Anything that runs a test binary is therefore on push, with one deliberate
# exception argued from the same measurement:
#
#   libviprs-org's entire CI is four node scripts, one shell diff and a small
#   cargo extractor, and it runs in 4s cold and under 1s warm. Splitting that
#   buys nothing and leaving half of it unguarded locally costs something, so
#   the pre-commit hook there mirrors the whole workflow.
#   libviprs-dep is the same story at 2s, so it gets the same treatment.
#
# The lists here are not maintained by hand and hope. `install_hooks_mirror_ci`
# runs the generated hook with recording stand-ins in front of every tool it
# shells out to, and compares what it actually invoked against the workflow.
# Grepping either file would be worth nothing, which is the standing lesson of
# libviprs/libviprs#695.
#
# Usage:  ./tools/install-hooks.sh            # from libviprs-tests/
#         ./libviprs-tests/tools/install-hooks.sh   # from the workspace root
#         ./tools/install-hooks.sh --describe # print the contract, install
#                                             # nothing (this is what the
#                                             # guard reads)
#
# Repos are found as siblings of this checkout's parent directory.
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

REPOS=(
    "$WORKSPACE_ROOT/libviprs"
    "$WORKSPACE_ROOT/libviprs-cli"
    "$WORKSPACE_ROOT/libviprs-tests"
    "$WORKSPACE_ROOT/libviprs-bench"
    "$WORKSPACE_ROOT/libviprs-org"
    "$WORKSPACE_ROOT/libviprs-dep"
    "$WORKSPACE_ROOT/pdfium-render"
)

# ---------------------------------------------------------------------------
# Where each repo keeps its workflow. Everything but the pdfium fork uses the
# same path; the fork carries upstream's file name.
# ---------------------------------------------------------------------------
workflow_for_repo() {
    case "$1" in
        pdfium-render) printf '%s\n' ".github/workflows/build_test.yml" ;;
        *)             printf '%s\n' ".github/workflows/ci.yml" ;;
    esac
}

# Some workflows check the repo out into a named subdirectory so a sibling can
# sit beside it, which puts that prefix on every path in those jobs' commands.
# Locally the repo IS the working directory, so the prefix comes off.
ci_checkout_path() {
    case "$1" in
        libviprs-bench) printf '%s\n' "libviprs-bench" ;;
        libviprs-org)   printf '%s\n' "libviprs-org" ;;
        *)              : ;;
    esac
}

# ---------------------------------------------------------------------------
# Which CI jobs the pre-commit hook stands in for.
# ---------------------------------------------------------------------------
mirror_jobs() {
    case "$1" in
        libviprs)       printf '%s\n' check ;;
        libviprs-cli)   printf '%s\n' check ;;
        libviprs-tests) printf '%s\n' lint feature-cells ported-tests ;;
        libviprs-bench) printf '%s\n' check ;;
        # The whole workflow, on the cost argument in the header.
        libviprs-org)   printf '%s\n' sync extract test-flags gen-op-sections ;;
        libviprs-dep)   printf '%s\n' lint test shellcheck ;;
        # Nothing. The fork's CI runs no lint at all, so there is nothing here
        # to mirror; see the note above write_pdfium_pre_commit.
        pdfium-render)  : ;;
        *)              : ;;
    esac
}

# Every CI job that is in neither half, and why. `job<TAB>reason`.
deferred_jobs() {
    case "$1" in
        libviprs)
            printf '%s\t%s\n' \
                msrv 'pinned to a 1.97 toolchain, and checking with the toolchain you happen to have installed answers a different question' \
                docs 'rustdoc over every feature; it cannot fail on anything the clippy cells pass' \
                test 'the test half' \
                integration-test 'the test half, and it lays this harness down beside the core'
            ;;
        libviprs-cli)
            printf '%s\t%s\n' \
                msrv 'pinned to a 1.97 toolchain, so a local run answers a different question' \
                test 'the test half'
            ;;
        libviprs-tests)
            printf '%s\t%s\n' \
                test 'the test half; 279s measured, and the pre-push gate runs it' \
                test-pdfium 'the test half, and it needs libpdfium.so installed' \
                cli-differential 'the test half; 18 differential binaries against the cli at CLI_COUNTERPART_REV' \
                hook-mirror 'the job that holds these hooks to every repo'"'"'s CI, which needs all seven laid down side by side'
            ;;
        libviprs-bench)
            printf '%s\t%s\n' \
                test 'the test half, and linking it needs libvips on the linker path rather than merely installed'
            ;;
        libviprs-org) : ;;
        libviprs-dep) : ;;
        pdfium-render)
            printf '%s\t%s\n' \
                build 'upstream'"'"'s file: one job of 205 compatibility cells plus a wasm-pack build, and it runs no lint at all'
            ;;
        *) : ;;
    esac
}

# Steps inside a mirrored job that the hook deliberately does not run, and why.
# `step<TAB>reason`, where step is the workflow step's `name:` when it has one
# and its `run:` text when it does not. A stale entry here is a failure too:
# the guard requires every one of these to still match a step CI actually runs.
exempt_steps() {
    case "$1" in
        libviprs)
            printf '%s\t%s\n' \
                'cargo build --features s3' 'there to prove a deprecated feature alias still resolves, which the manifest settles without a whole extra build on every commit'
            ;;
        libviprs-cli)
            printf '%s\t%s\n' \
                'Clone libviprs (matching branch or main)' 'CI has to fetch the core to get a sibling; locally it already is one, which is the layout this script assumes'
            ;;
        libviprs-tests)
            printf '%s\t%s\n' \
                'Fetch libvips reference suite at pinned revision' 'a network fetch of the pinned reference suite, so it is setup rather than a check' \
                './tools/run_ported_cells.sh --require-fixtures' 'this one runs the ported cells, so it is the test half, and it needs the fixtures the fetch above brings down'
            ;;
        libviprs-bench)
            printf '%s\t%s\n' \
                'Clone libviprs core (sibling path dep)' 'CI has to fetch the core to get a sibling; locally it already is one' \
                'Install libvips' 'apt-get on the runner image; locally libvips is a prerequisite, and the cells below fail loudly without it'
            ;;
        libviprs-org)
            printf '%s\t%s\n' \
                'Skip note (canonical CLI unavailable)' 'the other arm of the sync step, which only prints a warning; the hook prints its own when the sibling is missing'
            ;;
        libviprs-dep)
            printf '%s\t%s\n' \
                'pip install ruff' 'installs the tool rather than checking anything; the hook fails loudly if ruff is not on PATH' \
                'pip install pytest' 'same, for pytest'
            ;;
        *) : ;;
    esac
}

# ---------------------------------------------------------------------------
# What the pre-commit hook runs, in order; the first failure aborts.
#
# These have to be the mirrored jobs' steps, minus the exemptions above, or
# tests/install_hooks_mirror_ci.rs fails naming what drifted. `cargo clippy`
# already does `cargo check`'s work, so where a job runs both over the same
# features only the clippy line is here; libviprs-bench is the exception,
# because three of its four check cells have no clippy pass to hide behind.
#
# A step may carry a condition, written `test%%skip note%%command`. Only
# libviprs-org needs one, and it mirrors a step-level `if:` in the workflow.
# ---------------------------------------------------------------------------

# The core lints the default cell and nine feature-gated ones. Each of the
# latter is in CI because the default pass compiles none of that code, and the
# list sat at two of them for as long as a comment was the only thing asking
# anyone to keep it in step (libviprs/libviprs#715).
LIBVIPRS_STEPS=(
    "cargo fmt -- --check"
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

# The cli lints five configurations (libviprs-cli#48). The bare pass IS the
# pdfium pass, because pdfium is the cli's default feature, and the
# `--no-default-features` one is the only build that ever compiles the
# `#[cfg(not(feature = "pdfium"))]` halves.
LIBVIPRS_CLI_STEPS=(
    "cargo fmt -- --check"
    "cargo clippy --all-targets -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features tracing -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features packfile -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --features s3 -- -D warnings -W clippy::incompatible_msrv -W deprecated"
    "cargo clippy --all-targets --no-default-features -- -D warnings -W clippy::incompatible_msrv -W deprecated"
)

# This harness. The feature cells are the `feature-cells` matrix expanded; the
# ported step is scoped through tools/run_ported_cells.sh because the deferred
# codec cells do not compile, so `--all-targets` cannot carry that feature
# (issue #77). shellcheck is here because the `lint` job runs it and this hook
# did not, which left the one check that reads the hook's own shell to CI.
LIBVIPRS_TESTS_STEPS=(
    "cargo fmt -- --check"
    "cargo clippy --all-targets -- -D warnings"
    "cargo clippy --all-targets --features object-store-sink -- -D warnings"
    "cargo clippy --all-targets --features packfile -- -D warnings"
    "cargo clippy --all-targets --features tracing -- -D warnings"
    "cargo clippy --all-targets --features jxl -- -D warnings"
    "./tools/run_ported_cells.sh --clippy"
    "shellcheck tools/*.sh tools/hooks/pre-push"
)

# libviprs-bench has no `cargo fmt` line in CI, so there is none here: a hook
# step CI does not have is drift in the other direction and the guard refuses
# it. Its check job is four `cargo check` cells and one clippy, and the checks
# are not redundant with the clippy the way they are elsewhere, because that
# clippy pass covers only the `libvips` feature.
LIBVIPRS_BENCH_STEPS=(
    "cargo check --lib --bins --tests"
    "cargo check --lib --bins --tests --features libvips"
    "cargo check --lib --bins --features pdfium"
    "cargo check --lib --bins --features polars"
    "cargo clippy --lib --tests --features libvips"
)

# The doc site. Not Rust, and the four jobs are regenerate-and-assert-no-drift
# gates, which is exactly the check you want before a commit rather than after
# it: they catch "you edited the source and did not re-run the generator".
#
# The sync step is conditional because the workflow's is. CI runs it only when
# the canonical cli checkout succeeded; locally the equivalent question is
# whether the sibling is there at all. Unlike CI's, this hook's sibling is
# whatever you have checked out rather than CLI_COUNTERPART_REV, so a green
# here is a weaker claim than a green there. The note it prints says so.
LIBVIPRS_ORG_STEPS=(
    "test -d ../libviprs-cli%%no libviprs-cli sibling, so the frozen-copy sync check has nothing to compare (CI skips it the same way)%%cli/tools/sync-cli-src.sh --check"
    "cargo run --manifest-path cli/tools/extract-snippets/Cargo.toml"
    "git diff --exit-code cli/js/snippets.generated.json"
    "cd cli/tools/extract-snippets && cargo test --quiet"
    "node cli/tools/test-flags/test.js"
    "node cli/tools/test-flags/anchors.js"
    "node cli/tools/gen-op-sections/index.js --out cli/tools/gen-op-sections/generated-op-sections.html"
    "git diff --exit-code cli/tools/gen-op-sections/generated-op-sections.html"
    "node cli/tools/gen-op-sections/placeholder-substitution.test.js"
)

# The pdfium build inputs. Python and shell rather than Rust, and the whole
# workflow runs in about 2s locally, so all of it is here.
#
# This repo also ships its own tools/install-hooks.sh, which writes a hook that
# skips shellcheck when shellcheck is not installed. That is the false green
# this whole file exists to refuse, and nothing held that script to the
# workflow, so the hook this one writes is the one that should be in place.
# CI runs pytest on 3.9 and on 3.12; the hook runs it on whatever `pytest`
# resolves to, which is one of the two cells rather than both.
LIBVIPRS_DEP_STEPS=(
    "ruff check pdfium/"
    "ruff format --check pdfium/"
    "shellcheck pdfium/patches/*.sh"
    "pytest pdfium/tests/ -v"
)

steps_for_repo() {
    case "$1" in
        libviprs)       printf '%s\n' "${LIBVIPRS_STEPS[@]}" ;;
        libviprs-cli)   printf '%s\n' "${LIBVIPRS_CLI_STEPS[@]}" ;;
        libviprs-tests) printf '%s\n' "${LIBVIPRS_TESTS_STEPS[@]}" ;;
        libviprs-bench) printf '%s\n' "${LIBVIPRS_BENCH_STEPS[@]}" ;;
        libviprs-org)   printf '%s\n' "${LIBVIPRS_ORG_STEPS[@]}" ;;
        libviprs-dep)   printf '%s\n' "${LIBVIPRS_DEP_STEPS[@]}" ;;
        *)              : ;;
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
# goes away is a local promise the suite could not keep. The same is true of
# the three repos added since: the image holds no bench, no doc site and no
# pdfium build inputs.
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

# ---------------------------------------------------------------------------
# --describe: print the contract and install nothing.
#
# The guard reads this rather than the arrays, so the script stays the single
# place any of it is written down and the guard is still driving the real
# thing rather than parsing it.
# ---------------------------------------------------------------------------
describe() {
    local repo_dir repo name reason job
    for repo_dir in "${REPOS[@]}"; do
        repo="$(basename "$repo_dir")"
        printf 'repo\t%s\n' "$repo"
        printf 'workflow\t%s\t%s\n' "$repo" "$(workflow_for_repo "$repo")"
        if name="$(ci_checkout_path "$repo")" && [ -n "$name" ]; then
            printf 'checkout-path\t%s\t%s\n' "$repo" "$name"
        fi
        while IFS= read -r job; do
            [ -z "$job" ] && continue
            printf 'mirror\t%s\t%s\n' "$repo" "$job"
        done < <(mirror_jobs "$repo")
        while IFS=$'\t' read -r job reason; do
            [ -z "$job" ] && continue
            printf 'defer\t%s\t%s\t%s\n' "$repo" "$job" "$reason"
        done < <(deferred_jobs "$repo")
        while IFS=$'\t' read -r job reason; do
            [ -z "$job" ] && continue
            printf 'exempt\t%s\t%s\t%s\n' "$repo" "$job" "$reason"
        done < <(exempt_steps "$repo")
        if has_suite_slot "$repo"; then
            printf 'prepush\t%s\tyes\n' "$repo"
        else
            printf 'prepush\t%s\tno\n' "$repo"
        fi
    done
}

if [ "${1:-}" = "--describe" ]; then
    describe
    exit 0
fi
if [ $# -gt 0 ]; then
    echo "usage: $0 [--describe]" >&2
    exit 2
fi

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
    # cannot drift from CI. The step list above is the fallback for repos
    # without one, and it is only ever a subset.
    if [ -n "$repo_dir" ] && [ -f "$repo_dir/tools/local-ci.py" ]; then
        cat > "$pre_commit" << 'LOCALCI'
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook: runs the fast half of this repo's real CI job list, in
# Docker, via tools/local-ci.py. That script derives its commands from
# .github/workflows/ci.yml, so the COMMANDS here cannot drift from CI.
#
# It runs with --worktree, which bind-mounts the tree instead of provisioning
# it from git. That is the difference between about 12s and about 107s per
# commit, measured, and 107s per commit is not a thing anyone will tolerate.
#
# The cost is real and you should know it: a Docker Desktop bind mount off an
# APFS host is CASE-INSENSITIVE and it carries untracked files. So this hook
# can pass on a tree CI will reject, and it has: libviprs#977 was a fixture
# whose name differed from the committed file only in case, it resolved fine
# through the bind mount, and it was red on ubuntu-latest for two days.
#
# So this hook is the fast check, NOT the gate. The gate is `make ci`, which
# provisions from git and is case-exact. Run that before you push anything you
# care about. libviprs/tests/fixture_paths_are_committed.rs catches the one
# specific shape that bit us, but it does not make the bind mount case-exact.
#
# Installed by libviprs-tests/tools/install-hooks.sh.
# To skip (emergency only): git commit --no-verify

# A repo's main checkout and all of its linked worktrees share one hooks
# directory, and git invokes the hook with $0 inside that shared directory
# regardless of which worktree is actually being committed in, so deriving
# REPO_DIR from $0 always resolves to the main checkout (libviprs/libviprs#684,
# the same bug install_pre_push below already dodges). Ask git instead.
REPO_DIR="$(git rev-parse --show-toplevel)"
echo "Running the fast CI jobs locally (tools/local-ci.py --fast --worktree)..."
if ! python3 "$REPO_DIR/tools/local-ci.py" --fast --worktree; then
    echo ""
    echo "Failed. These are the real CI commands, so CI will fail the same way."
    echo "Run everything including tests: make ci"
    echo "Skip this hook once:            git commit --no-verify"
    exit 1
fi
echo "Passed, but this was the FAST check, not the gate: --worktree bind-mounts"
echo "the tree, which is case-insensitive here and carries untracked files."
echo "Before pushing, run the real gate: make ci"
LOCALCI
        chmod +x "$pre_commit"
        return
    fi

    {
        cat << HEAD
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook: the lint half of this repo's CI, which is the jobs
# install-hooks.sh lists under mirror_jobs for $repo_name. Tests run on push.
# A repo with tools/local-ci.py gets the real job list instead of this.
# $INSTALLER_MARKER
# To update it, edit that script and re-run it. To skip (emergency only):
#   git commit --no-verify

echo "Running pre-commit checks (the lint half of CI)..."

HEAD
        # One command per CI step, so a failure prints exactly which CI line it
        # would have flunked.
        local step guard note cmd
        while IFS= read -r step; do
            [ -z "$step" ] && continue
            if [[ "$step" == *"%%"* ]]; then
                guard="${step%%\%\%*}"
                cmd="${step##*%%}"
                note="${step#*%%}"
                note="${note%%\%\%*}"
                cat <<HOOK
if $guard; then
HOOK
                emit_step "$cmd" "    "
                cat <<HOOK
else
    echo "  skipping: $cmd"
    echo "    $note"
fi

HOOK
            else
                emit_step "$step" ""
                printf '\n'
            fi
        done < <(steps_for_repo "$repo_name")

        echo 'echo "Pre-commit checks passed. Tests run on push."'
    } > "$pre_commit"
    chmod +x "$pre_commit"
}

# One step of the generated hook. `$2` is the indent, so a conditional step
# nests without the heredoc having to know.
#
# The subshell around the command is load-bearing twice over. `if ! cd x &&
# cargo test` parses as `(! cd x) && (cargo test)`, so with the `cd` working
# the second half never runs at all and the step reports a pass having done
# nothing, which is the exact false green these hooks exist to refuse. And a
# `cd` that is not contained moves the working directory for every step after
# it. `( ... )` fixes both.
emit_step() {
    local cmd="$1"
    local pad="$2"
    local hint="Fix and re-stage. (This command is a CI step; if it passes"

    case "$cmd" in
        "cargo fmt"*) hint="Run 'cargo fmt' and re-stage. (This command is a CI step; if it passes" ;;
        "ruff format"*) hint="Run 'ruff format pdfium/' and re-stage. (This command is a CI step; if it passes" ;;
        "git diff --exit-code"*) hint="Re-run the generator above and commit what it wrote. (This command is a CI step; if it passes" ;;
    esac

    cat <<HOOK
${pad}echo "  $cmd"
${pad}if ! ( $cmd ); then
${pad}    echo ""
${pad}    echo "Failed: $cmd"
${pad}    echo "$hint"
${pad}    echo "locally the matching CI step will too.)"
${pad}    exit 1
${pad}fi
HOOK
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
# Pre-push shim. $INSTALLER_MARKER
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

# ---------------------------------------------------------------------------
# pdfium-render gets a hook of its own, and it is the one place here that is
# not a CI mirror. Upstream's workflow runs `cargo build`, `cargo test`, a
# wasm-pack build and 205 `cargo check` compatibility cells, and not one line
# of `cargo fmt` or `cargo clippy`. So there is nothing to mirror, and what
# this hook enforces is fork policy instead: keep the lines we write clean.
#
# It cannot be a blanket `cargo clippy -- -D warnings`, because upstream
# carries hundreds of pre-existing lints (~470 missing-safety-doc, ~22
# doc-nested-refdefs) and that would block every commit including merges from
# upstream. So clippy runs over the whole tree and the result is filtered to
# lints whose primary span sits on a line the commit in hand is changing.
#
# It used to filter against the whole fork delta vs upstream/master, and that
# does not survive a clippy upgrade: a newer clippy grew `deref on an
# immutable reference`, 54 of them landed on fork lines written months ago,
# and the hook then refused every commit on an unmodified `origin/master`
# checkout with nothing staged. Measured 2026-09-02: 54 warnings, exit 1, on a
# clean tree. A gate that is red before you have typed anything is a gate
# people delete.
#
# The scope is `git diff HEAD`, the working tree against the commit you are
# on, and not `git diff --cached`, because clippy compiles the working tree.
# Scoping to the index would number the lints against content clippy never
# saw, and the failure mode of that is a lint pointing at an innocent line or
# a real one slipping through because its line moved. For the ordinary `git
# add` then `git commit` the two sets are identical anyway.
# ---------------------------------------------------------------------------
write_pdfium_pre_commit() {
    local hooks_dir="$1"
    local pre_commit="$hooks_dir/pre-commit"

    cat > "$pre_commit" << HOOK
#!/usr/bin/env bash
set -euo pipefail

# Pre-commit hook for the libviprs/pdfium-render fork. This is fork policy,
# not a CI mirror: upstream's workflow runs no lint at all.
# $INSTALLER_MARKER
# To update it, edit that script and re-run it. To skip (emergency only):
#   git commit --no-verify
HOOK

    cat >> "$pre_commit" << 'HOOK'

echo "Running pre-commit checks (fork-scoped)..."

echo "  cargo fmt --check..."
if ! cargo fmt -- --check; then
    echo ""
    echo "Formatting check failed. Run 'cargo fmt' and re-stage."
    exit 1
fi

echo "  cargo clippy --all-targets (scoped to the lines this commit changes)..."

if ! command -v python3 >/dev/null 2>&1; then
    echo "  python3 is not on PATH, and the scoping needs it." >&2
    echo "  Install python3, or commit with --no-verify and say so." >&2
    exit 1
fi

python3 - <<'PY'
import json
import os
import subprocess
import sys

# Lines the working tree changes against HEAD. That is the commit in hand,
# and it is also exactly the content clippy is about to compile, so the line
# numbers on both sides refer to the same file. `--unified=0` makes the hunk
# headers line up with the changed lines and nothing else.
diff = subprocess.run(
    ["git", "diff", "--unified=0", "HEAD", "--", "*.rs"],
    capture_output=True,
    text=True,
)
if diff.returncode != 0:
    print("  git diff failed, so the scope cannot be computed:", file=sys.stderr)
    print(diff.stderr, file=sys.stderr)
    sys.exit(1)

file_lines = {}
current_file = None
for line in diff.stdout.splitlines():
    if line.startswith("+++ b/"):
        current_file = line[6:]
    elif line.startswith("@@") and current_file:
        # "@@ -a,b +c,d @@", where c is the first changed line on the new side and
        # d how many there are, defaulting to 1 when it is left off.
        plus = line.split("+", 1)[1].split(" ", 1)[0]
        start, _, count = plus.partition(",")
        count = int(count) if count else 1
        if count:
            file_lines.setdefault(current_file, set()).update(
                range(int(start), int(start) + count)
            )

if not diff.stdout.strip():
    print("  no Rust file differs from HEAD, so there is nothing to check")
    sys.exit(0)

# A commit can be all deletions, which gives a real diff and no added lines to
# scope to. Skipping there would be a commit that changes the crate and gets no
# check at all, and deleting the wrong line is a perfectly ordinary way to
# break a build, so clippy still runs and the exit code below still counts.
if not file_lines:
    print("  only deletions against HEAD, so nothing is scoped, but the tree")
    print("  still has to build")

# An ambient `-D warnings` has to come off for this one call. The whole
# arrangement here rests on clippy reporting an inherited lint as a warning so
# the scope filter can drop it; with `-D warnings` in RUSTFLAGS every one of
# them arrives as an error instead, clippy exits non-zero, and the hook refuses
# a commit over debt it was written to ignore. That is not hypothetical: this
# repo's own CI sets RUSTFLAGS at the workflow level, and the guard for this
# hook went red there the first time it ran. Real compile errors are unaffected,
# because they are errors without it.
env = dict(os.environ)
flags = env.get("RUSTFLAGS", "").split()
kept = []
i = 0
while i < len(flags):
    if flags[i] in ("-Dwarnings", "--deny=warnings"):
        i += 1
        continue
    if flags[i] in ("-D", "--deny") and i + 1 < len(flags) and flags[i + 1] == "warnings":
        i += 2
        continue
    kept.append(flags[i])
    i += 1
env["RUSTFLAGS"] = " ".join(kept)

clippy = subprocess.run(
    ["cargo", "clippy", "--all-targets", "--message-format=json"],
    capture_output=True,
    text=True,
    env=env,
)

hits = []
errors = []
for line in clippy.stdout.splitlines():
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
    if level == "error":
        errors.append(msg.get("rendered") or msg.get("message", ""))
    for span in msg.get("spans") or []:
        if not span.get("is_primary"):
            continue
        owned = file_lines.get(span.get("file_name"))
        first = span.get("line_start")
        last = span.get("line_end") or first
        if owned and any(i in owned for i in range(first, last + 1)):
            hits.append(
                f"  {level}: {msg.get('message','')}\n"
                f"    --> {span.get('file_name')}:{first}:{span.get('column_start','?')}"
            )
            break

# A crate that does not build is not a pass. The filter above only ever looks
# at lines this commit touches, so a hard error anywhere else would otherwise
# come back green, which is the shape of false green this hook is here to
# avoid rather than to grow. Deleting a line another file depends on is the
# everyday way to land one of those.
if clippy.returncode != 0 and not hits:
    print("")
    print("cargo clippy exited %d, so the tree does not build." % clippy.returncode)
    print("None of it lands on a line this commit changes, so it is not")
    print("something you introduced, but it still has to be fixed to commit.")
    print("")
    print("".join(errors[:5]) if errors else clippy.stderr[-2000:])
    sys.exit(1)

if hits:
    print("")
    print("Clippy lints on lines this commit changes:")
    print("\n".join(hits))
    print("")
    print("Fix and re-stage. (Lints on lines this commit leaves alone are")
    print("ignored, whether they came from upstream or from an older commit")
    print("of ours that a newer clippy has since caught up with.)")
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
            write_pdfium_pre_commit "$HOOKS_DIR"
            echo "  done: $REPO_NAME (fork-scoped pre-commit)"
            drop_generated_pre_push "$HOOKS_DIR"
            ;;
        *)
            # The pre-commit hook goes everywhere: it runs the repo's own
            # checks in the repo it is installed in, so for the cli it gates
            # the cli. $REPO_DIR lets it detect a tools/local-ci.py in the
            # target repo and defer to that instead.
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
