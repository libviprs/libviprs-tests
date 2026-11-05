#!/bin/sh
set -eu

# ---------------------------------------------------------------------------
# scoped-suite.sh: run the targets of one pushed PR inside the test container
# (libviprs-tests#370).
#
# The Dockerfile CMD hands control here when LIBVIPRS_GATE_SCOPE is set, and
# runs the whole suite when it is not. The value is a plan, base64 on one line,
# which tools/gate-scope.sh writes and tools/run-tests.sh --scope carries in.
# One plan line per target:
#
#   core|test|NAME           cargo test --test NAME          in the core
#   core|lib|FILTER          cargo test --lib -- FILTER      (a module path, m::)
#   core|doc|FILTER          cargo test --doc -- FILTER      (a doctest path, src/m)
#   tests|test|NAME          cargo test --test NAME          in the harness
#   features|core|a,b        extra cargo features for that tree, on top of pdfium
#   features|tests|a,b
#   ported                   tools/run_ported_cells.sh, the green ported cells
#   lint                     nothing here: the hook already ran the lint half
#
# The build is split from the run with a `sync` between, the way the full
# chain's is, so the first memory-hungry test binary does not start on top of
# gigabytes of dirty page cache (libviprs/libviprs#683).
#
# This script never decides what to run, and it never runs "everything" when
# the plan is wrong. A plan that selects no target, or one it cannot read, is a
# refusal: scoping must never silently run nothing, and the full suite is the
# gate's call to make, not a fallback in here.
# ---------------------------------------------------------------------------

CORE_DIR="${LIBVIPRS_SCOPE_CORE_DIR:-/src/libviprs}"
TESTS_DIR="${LIBVIPRS_SCOPE_TESTS_DIR:-/src/libviprs-tests}"

die() {
    echo "scoped-suite: $*" >&2
    exit 1
}

[ -n "${LIBVIPRS_GATE_SCOPE:-}" ] || die "LIBVIPRS_GATE_SCOPE is empty, so there is no plan to run"
PLAN="$(printf '%s' "$LIBVIPRS_GATE_SCOPE" | base64 -d 2>/dev/null)" || die "LIBVIPRS_GATE_SCOPE is not base64"

CORE_TESTS=""
CORE_LIBS=""
CORE_DOCS=""
TESTS_TESTS=""
CORE_FEATURES=""
TESTS_FEATURES=""
PORTED=false

while IFS='|' read -r repo kind arg; do
    [ -n "$repo" ] || continue
    # Names end up on a command line unquoted, so they may only be made of the
    # characters a target, a module path, a doctest path or a feature has.
    case "$arg" in
        *[!A-Za-z0-9_:/.,-]*) die "the plan has a name with a character a target cannot have: $arg" ;;
    esac
    case "$repo|$kind" in
        core\|test) CORE_TESTS="$CORE_TESTS $arg" ;;
        core\|lib) CORE_LIBS="$CORE_LIBS $arg" ;;
        core\|doc) CORE_DOCS="$CORE_DOCS $arg" ;;
        tests\|test) TESTS_TESTS="$TESTS_TESTS $arg" ;;
        features\|core) CORE_FEATURES="$CORE_FEATURES,$arg" ;;
        features\|tests) TESTS_FEATURES="$TESTS_FEATURES,$arg" ;;
        ported\|) PORTED=true ;;
        lint\|) : ;;
        *) die "the plan has a line this script does not know: $repo|$kind|$arg" ;;
    esac
    case "$repo" in
        core | tests | features)
            if [ -z "$kind" ] || [ -z "$arg" ]; then
                die "the plan has an incomplete line: $repo|$kind|$arg"
            fi
            ;;
    esac
done <<EOF
$PLAN
EOF

if [ -z "$CORE_TESTS$CORE_LIBS$CORE_DOCS$TESTS_TESTS" ] && [ "$PORTED" = false ]; then
    die "the plan selects no target, and a scoped run never runs nothing"
fi

banner() {
    echo "================================================================"
    echo "$1"
    echo "================================================================"
}

# run_tree DIR FEATURES TESTS LIBS DOCS
run_tree() {
    dir="$1"
    features="pdfium$2"
    tests="$3"
    libs="$4"
    docs="$5"
    test_args=""
    for name in $tests; do
        test_args="$test_args --test $name"
    done
    lib_arg=""
    [ -z "$libs" ] || lib_arg=" --lib"
    cd "$dir"
    if [ -n "$tests$libs" ]; then
        # shellcheck disable=SC2086
        cargo test --features "$features" --no-run$lib_arg$test_args
        sync
    fi
    if [ -n "$tests" ]; then
        # shellcheck disable=SC2086
        cargo test --features "$features"$test_args
    fi
    if [ -n "$libs" ]; then
        # shellcheck disable=SC2086
        cargo test --features "$features" --lib -- $libs
    fi
    if [ -n "$docs" ]; then
        # shellcheck disable=SC2086
        cargo test --features "$features" --doc -- $docs
    fi
}

if [ -n "$CORE_TESTS$CORE_LIBS$CORE_DOCS" ]; then
    banner "Running the libviprs targets of this PR (with pdfium)..."
    run_tree "$CORE_DIR" "$CORE_FEATURES" "$CORE_TESTS" "$CORE_LIBS" "$CORE_DOCS"
    echo ""
    echo "Cleaning libviprs build artifacts to free disk space..."
    cargo clean
fi

if [ -n "$TESTS_TESTS" ]; then
    banner "Running the libviprs-tests targets of this PR (with pdfium)..."
    run_tree "$TESTS_DIR" "$TESTS_FEATURES" "$TESTS_TESTS" "" ""
fi

if [ "$PORTED" = true ]; then
    banner "Running libviprs-tests green ported cells (ported_tests)..."
    cd "$TESTS_DIR"
    ./tools/run_ported_cells.sh
fi
