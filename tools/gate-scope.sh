#!/usr/bin/env bash
set -u
# Names from a commit message and from the tree are data, never patterns.
set -f

# ---------------------------------------------------------------------------
# gate-scope.sh: choose the test targets of one pushed PR (libviprs-tests#370).
#
# The pre-push gate used to run the whole suite on every push that was not
# inert. The owner decision of 2026-11-04 is that a PR runs its own tests, so
# this decides which ones. It is pure: trees in, plan out. It runs nothing and
# talks to nobody; tools/mars-run.sh calls it once both trees are known and
# hands the answer to tools/run-tests.sh --scope.
#
#   gate-scope.sh --repo libviprs|libviprs-tests --changed FILE
#                 --core-git DIR --core-tree OID --tests-git DIR --tests-tree OID
#                 [--message FILE]...
#
#   --changed FILE   the paths the push changes in --repo, one per line. The hook
#                    leaves out the inert ones (docs and friends no test names).
#   --core-tree      the trees that are about to run, exactly the ones mars-run.sh
#   --tests-tree     resolved, so COUNTERPART_REV and LIBVIPRS_TESTS_REV keep
#                    working: a core push is selected against the harness at
#                    origin/main and a harness push against the pinned core.
#   --message FILE   a tip commit message, for the Tests: trailer.
#
# Output on stdout, and nothing else there:
#
#   FULL|<why>                      run the whole suite
#   lint                            run the lint half (the pre-commit hook) on the tips
#   core|test|NAME  tests|test|NAME   one integration target in that repo
#   core|lib|m::                    the unit tests of module m
#   core|doc|src/m                  the doctests of module m
#   features|core|a,b               required features of the selected targets
#   features|tests|a,b
#   ported                          the green ported cells
#
# A human line goes to stderr: what it is scoped to and why, or why it is not.
# Exit 0 for a plan or FULL, 3 for a push that must be refused (a Tests: name
# that is not in the tree), anything else is a crash and the caller runs the
# full suite.
#
# IT FAILS CLOSED. Scoping never silently runs nothing. These run the full
# suite: the files that change what every test builds or sees (Cargo manifests
# and lockfiles, build scripts, fuzz, Dockerfiles, toolchain pins, the
# counterpart pins, run-tests.sh and the scoping scripts, tests/common/mod.rs,
# src/lib.rs) and the hub modules below; a glob re-export; a failing git grep;
# a selection over the cap (LIBVIPRS_GATE_SCOPE_CAP, default 40 targets); and
# any path that maps to zero targets. LIBVIPRS_PREPUSH_ALL=1 is the full suite
# too, as it always was.
#
# What it cannot see is behaviour reached with no name a test spells out: a path
# built with no literal, or a module reached only through another one. The hub
# list and the cap are what stand in for that, and the full run before a
# release is the net under it.
#
# bash 3.2 is still /bin/bash on macOS. No `case` inside "$( ... )" (it mis-parses
# the closing paren of a pattern), no associative arrays, no mapfile.
# ---------------------------------------------------------------------------

REPO=""
CHANGED=""
CORE_GIT=""
CORE_TREE=""
TESTS_GIT=""
TESTS_TREE=""
MESSAGES=()

usage() {
    awk '/^# -{20,}/ { fence++; if (fence == 2) exit; next } fence == 1 && /^#/ { sub(/^# ?/, ""); print }' "$0" >&2
    exit 2
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --repo) REPO="${2:-}"; shift ;;
        --changed) CHANGED="${2:-}"; shift ;;
        --core-git) CORE_GIT="${2:-}"; shift ;;
        --core-tree) CORE_TREE="${2:-}"; shift ;;
        --tests-git) TESTS_GIT="${2:-}"; shift ;;
        --tests-tree) TESTS_TREE="${2:-}"; shift ;;
        --message) MESSAGES+=("${2:-}"); shift ;;
        *) usage ;;
    esac
    shift
done

case "$REPO" in
    libviprs) PUSHED=core ;;
    libviprs-tests) PUSHED=tests ;;
    *) echo "gate-scope: --repo must be libviprs or libviprs-tests" >&2; exit 2 ;;
esac
for need in "$CHANGED" "$CORE_GIT" "$CORE_TREE" "$TESTS_GIT" "$TESTS_TREE"; do
    [ -n "$need" ] || usage
done
[ -r "$CHANGED" ] || { echo "gate-scope: cannot read $CHANGED" >&2; exit 2; }

W="$(mktemp -d "${TMPDIR:-/tmp}/gate-scope.XXXXXX")" || exit 2
FILES="core_test tests_test core_lib core_doc ported why hits trailer_core_test trailer_tests_test trailer_core_lib trailer_core_doc"
cleanup() {
    local f
    for f in $FILES; do rm -f "$W/$f"; done
    rmdir "$W" 2>/dev/null || true
}
trap cleanup EXIT
for f in $FILES; do : > "$W/$f"; done

CAP="${LIBVIPRS_GATE_SCOPE_CAP:-40}"
case "$CAP" in
    "" | *[!0-9]*) CAP=40 ;;
esac

g() {
    env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE -u GIT_PREFIX -u GIT_QUARANTINE_PATH git "$@"
}

full() {
    printf 'FULL|%s\n' "$1"
    echo "Pre-push: FULL because $1" >&2
    exit 0
}

refuse() {
    echo "Pre-push: $1" >&2
    exit 3
}

# sel core|tests sets GITD and TREE for that repo.
GITD=""
TREE=""
sel() {
    if [ "$1" = core ]; then
        GITD="$CORE_GIT"
        TREE="$CORE_TREE"
    else
        GITD="$TESTS_GIT"
        TREE="$TESTS_TREE"
    fi
}

tree_has() {
    sel "$1"
    g -C "$GITD" cat-file -e "$TREE:$2" 2>/dev/null
}

tree_show() {
    sel "$1"
    g -C "$GITD" show "$TREE:$2" 2>/dev/null
}

# gg KIND MODE PATTERN PATHSPEC...  puts the matching paths of that tree in
# GG_HITS, one per line, without the tree prefix. A grep that could not run is
# not "no match": it is the full suite.
GG_HITS=""
gg() {
    local kind="$1" mode="$2" pat="$3" out rc
    shift 3
    sel "$kind"
    out="$(g -C "$GITD" grep "-l$mode" -e "$pat" "$TREE" -- "$@" 2>/dev/null)"
    rc=$?
    if [ "$rc" -gt 1 ]; then
        full "git grep failed in the $kind tree (status $rc), so I cannot tell what names this"
    fi
    GG_HITS="$(printf '%s\n' "$out" | sed -e 's/^[0-9a-f]*://' -e '/^$/d')"
}

esc() {
    printf '%s' "$1" | sed 's/[][\.*^$+?(){}|]/\\&/g'
}

ADDED=0
add() {
    printf '%s\n' "$2" >> "$W/$1"
    ADDED=$((ADDED + 1))
    printf '%s\n' "$3" >> "$W/why"
}

# The hub modules: so much of the crate leans on them that "the tests that name
# this module" is most of the suite, or, for error.rs, so little of it that the
# tests that name it are not the ones that break. A selection built from naming
# them would only look precise. Measured on origin/main of both repos: pixel
# pulls in 172 targets and engine 67, and error names 4 for code every
# operation returns through. The cap catches the other fat ones (planner 149,
# sink 98, draw 84, source 75, manifest 49); engine_builder is a hub already,
# since src/engine* covers it.
is_hub() {
    case "$1" in
        src/error.rs | src/error/* | src/pixel* | src/raster* | src/engine*) return 0 ;;
    esac
    return 1
}

full_path_rule() {
    case "$1" in
        Cargo.toml | Cargo.lock | */Cargo.toml | */Cargo.lock) echo "a manifest or lockfile changes what every target builds" ;;
        build.rs | */build.rs) echo "a build script changes what every target builds" ;;
        fuzz/*) echo "the fuzz workspace shares the crate's build" ;;
        Dockerfile* | */Dockerfile*) echo "the image every run is built in" ;;
        .dockerignore | *.dockerignore) echo "the build context every run is built from" ;;
        rust-toolchain | rust-toolchain.* | .cargo/*) echo "the toolchain every run compiles with" ;;
        COUNTERPART_REV | CLI_COUNTERPART_REV) echo "the pin decides which core the harness is gated against" ;;
        tools/run-tests.sh | tools/gate-scope.sh | tools/scoped-suite.sh) echo "this is the machinery that picks and runs the scope" ;;
        tests/common/mod.rs) echo "a shared test helper every integration target pulls in" ;;
        src/lib.rs) echo "the crate root decides what every target can see" ;;
        *)
            if is_hub "$1"; then
                echo "a hub module most of the crate leans on"
            fi
            ;;
    esac
}

# Everything the tests/ and src/ of a tree name by a path-shaped string literal,
# the way the hook's own scan reads them: the file itself, or a directory above
# it. The top-level tests, src and .github directories are not counted as a
# directory above, since a literal "tests" names nothing in particular and
# the bare ".github" a shared workflows helper spells would pull in every test
# that helper reaches for a change to one issue form (#375).
scan_named() {
    local p="$1" why="$2" kind target above targets hits h pat
    targets="$p"
    above="$p"
    while [ "${above%/*}" != "$above" ]; do
        above="${above%/*}"
        if [ "$above" != tests ] && [ "$above" != src ] && [ "$above" != .github ]; then
            targets="$targets
$above"
        fi
    done
    for kind in core tests; do
        while IFS= read -r target; do
            [ -n "$target" ] || continue
            pat="\"([^\"]*/)?$(esc "$target")/?\""
            gg "$kind" E "$pat" 'tests/*.rs' 'src/*.rs'
            hits="$GG_HITS"
            while IFS= read -r h; do
                [ -n "$h" ] || continue
                map_file "$kind" "$h" "$why"
            done <<HITS
$hits
HITS
        done <<TARGETS
$targets
TARGETS
    done
}

VISITED=" "
map_file() {
    local kind="$1" file="$2" why="$3" f name hits h

    case "$VISITED" in
        *" $kind:$file "*) return 0 ;;
    esac
    VISITED="$VISITED$kind:$file "

    case "$file" in
        tests/common/mod.rs)
            full "$file is a shared test helper every target pulls in ($why)"
            ;;
        tests/common/*)
            f="${file#tests/common/}"
            f="${f%%/*}"
            f="${f%.rs}"
            gg "$kind" E "common::$f([^A-Za-z0-9_]|\$)|\"common/$f(\\.rs)?\"" 'tests/*.rs'
            hits="$GG_HITS"
            while IFS= read -r h; do
                [ -n "$h" ] || continue
                map_file "$kind" "$h" "$why"
            done <<HITS
$hits
HITS
            ;;
        tests/*/main.rs)
            name="${file#tests/}"
            name="${name%%/*}"
            if tree_has "$kind" "$file"; then
                add "${kind}_test" "$name" "$why -> target $name"
            fi
            ;;
        tests/*/*.rs)
            full "$file is a test module no target of its own owns ($why)"
            ;;
        tests/ported_*.rs)
            if tree_has "$kind" "$file"; then
                if [ "$kind" = tests ]; then
                    printf '1\n' > "$W/ported"
                    ADDED=$((ADDED + 1))
                    printf '%s\n' "$why -> the ported cells" >> "$W/why"
                else
                    add "${kind}_test" "${file#tests/}" "$why -> target"
                fi
            fi
            ;;
        tests/*.rs)
            name="${file#tests/}"
            name="${name%.rs}"
            if tree_has "$kind" "$file"; then
                add "${kind}_test" "$name" "$why -> target $name"
            fi
            ;;
        src/bin/* | src/main.rs)
            # A binary is not a module of the library, so no unit-test filter
            # reaches it. Only a test that names its path does.
            scan_named "$file" "$why"
            ;;
        src/*.rs | src/*/*)
            if ! tree_has "$kind" "$file"; then
                full "$file is gone from the tree (a deleted or renamed source file), so nothing says what it ran ($why)"
            fi
            if [ "$kind" = core ]; then
                module_targets "${file#src/}" "$why"
            else
                full "$file is source in the harness, which has no module rule ($why)"
            fi
            ;;
        *)
            scan_named "$file" "$why"
            ;;
    esac
}

# The identifiers src/lib.rs re-exports from module $1, one per line, or a lone
# * for a glob re-export.
reexports() {
    tree_show core src/lib.rs | awk -v m="$1" '
        function process(b,   s, i, mod, rest, k, n, arr, t) {
            s = b
            sub(/^[[:space:]]*pub[[:space:]]+use[[:space:]]+(crate::)?/, "", s)
            i = index(s, "::")
            if (i == 0) return
            mod = substr(s, 1, i - 1)
            if (mod != m) return
            rest = substr(s, i + 2)
            sub(/;.*/, "", rest)
            if (rest ~ /\*/) { print "*"; return }
            n = split(rest, arr, /[^A-Za-z0-9_]+/)
            for (k = 1; k <= n; k++) {
                t = arr[k]
                if (t != "" && t != "as" && t != "self" && t != "crate" && t != "super") print t
            }
        }
        /^[[:space:]]*pub[[:space:]]+use[[:space:]]/ { buf = ""; collecting = 1 }
        collecting { buf = buf " " $0; if ($0 ~ /;/) { collecting = 0; process(buf) } }
    ' | sort -u
}

module_targets() {
    local rest="$1" why="$2" m idents ident_pat pat kind hits h named mentions
    m="${rest%%/*}"
    m="${m%.rs}"
    add core_lib "$m::" "$why -> unit tests of $m"
    add core_doc "src/$m" "$why -> doctests of $m"
    # The two filters exist for every module, so they cannot be what says a
    # module has a test. Only a test that names the module counts as a target
    # for the "maps to zero targets" rule; without one the push runs everything.
    ADDED=$((ADDED - 2))

    pat="libviprs::$m([^A-Za-z0-9_]|\$)|(^|[^A-Za-z0-9_:])$m::|libviprs::\\{([^}]*[^A-Za-z0-9_])?$m([^A-Za-z0-9_]|\$)|^[[:space:]]+$m,?[[:space:]]*\$"
    for kind in core tests; do
        gg "$kind" E "$pat" 'tests/*.rs'
        hits="$GG_HITS"
        while IFS= read -r h; do
            [ -n "$h" ] || continue
            map_file "$kind" "$h" "$why"
        done <<HITS
$hits
HITS
    done

    idents="$(reexports "$m")"
    if [ "$idents" = "*" ]; then
        full "src/$m is re-exported with a glob from the crate root, so I cannot list what names it ($why)"
    fi
    if [ -n "$idents" ]; then
        ident_pat="(^|[^A-Za-z0-9_])($(printf '%s' "$idents" | paste -sd'|' -))([^A-Za-z0-9_]|\$)"
        for kind in core tests; do
            gg "$kind" E "$ident_pat" 'tests/*.rs'
            named="$GG_HITS"
            gg "$kind" F libviprs 'tests/*.rs'
            mentions="$GG_HITS"
            printf '%s\n' "$mentions" | sort -u > "$W/hits"
            hits="$(printf '%s\n' "$named" | sort -u | comm -12 - "$W/hits")"
            while IFS= read -r h; do
                [ -n "$h" ] || continue
                map_file "$kind" "$h" "$why"
            done <<HITS
$hits
HITS
        done
    fi
}

add_guard() {
    # add_guard KIND NAME WHY: a guard that must exist in the tree it runs in.
    if tree_has "$1" "tests/$2.rs"; then
        add "${1}_test" "$2" "$3"
    else
        echo "Pre-push: the guard $1:$2 is not in the tree being gated, so it is not in the plan." >&2
    fi
}

# ---------------------------------------------------------------------------
# Overrides, then the Tests: trailer
# ---------------------------------------------------------------------------

if [ "${LIBVIPRS_PREPUSH_ALL:-0}" = "1" ]; then
    full "LIBVIPRS_PREPUSH_ALL=1 asks for everything"
fi

TRAILER_ITEMS=""
for message in ${MESSAGES[@]+"${MESSAGES[@]}"}; do
    [ -r "$message" ] || continue
    line="$(awk 'BEGIN { RS = "" } { p = $0 } END { print p }' "$message" |
        { grep -iE '^Tests:[[:space:]]*[^[:space:]]' || true; })"
    [ -n "$line" ] || continue
    TRAILER_ITEMS="$TRAILER_ITEMS $(printf '%s\n' "$line" | sed -e 's/^[Tt][Ee][Ss][Tt][Ss]:[[:space:]]*//' | tr ',' ' ')"
done

for item in $TRAILER_ITEMS; do
    case "$item" in
        *[!A-Za-z0-9_:/.-]*) refuse "the Tests: trailer has a name with a character a target cannot have: $item" ;;
    esac
    kind="${item%%:*}"
    rest="${item#*:}"
    if [ "$kind" = "$item" ] || [ -z "$rest" ]; then
        refuse "the Tests: trailer names $item, which is not core:NAME, tests:NAME, core:lib:FILTER or core:doc:FILTER."
    fi
    case "$kind:$rest" in
        core:lib:* | core:doc:*)
            sub="${rest%%:*}"
            filter="${rest#*:}"
            if [ "$sub" = lib ]; then
                mod="${filter%%::*}"
            else
                mod="${filter#src/}"
                mod="${mod%%/*}"
                mod="${mod%.rs}"
            fi
            if [ -z "$mod" ] || { ! tree_has core "src/$mod.rs" && ! tree_has core "src/$mod"; }; then
                refuse "the Tests: trailer names $item, but there is no module $mod in the libviprs tree being gated."
            fi
            printf '%s\n' "$filter" >> "$W/trailer_core_$sub"
            ;;
        core:* | tests:*)
            if tree_has "$kind" "tests/$rest.rs" || tree_has "$kind" "tests/$rest/main.rs"; then
                printf '%s\n' "$rest" >> "$W/trailer_${kind}_test"
            else
                label=libviprs
                [ "$kind" = tests ] && label=libviprs-tests
                refuse "the Tests: trailer names $item, but there is no tests/$rest.rs in the $label tree being gated."
            fi
            ;;
        *)
            refuse "the Tests: trailer names $item, which is not core:NAME, tests:NAME, core:lib:FILTER or core:doc:FILTER."
            ;;
    esac
done

# ---------------------------------------------------------------------------
# The changed paths
# ---------------------------------------------------------------------------

CORE_RS=false
SAW_ANY=false
while IFS= read -r p; do
    [ -n "$p" ] || continue
    SAW_ANY=true
    rule="$(full_path_rule "$p")"
    if [ -n "$rule" ]; then
        full "$p ($rule)"
    fi
    ADDED=0
    VISITED=" "

    if [ "$PUSHED" = core ]; then
        case "$p" in
            src/*.rs | src/*/* | tests/*.rs | tests/*/*)
                case "$p" in
                    *.rs) CORE_RS=true ;;
                esac
                ;;
        esac
        case "$p" in
            .github/* | tools/*)
                for guard in local_gate_is_the_job_list local_gate_native_dwg_1295 miri_invocation_parity; do
                    add_guard core "$guard" "$p -> the CI parity guard $guard"
                done
                ;;
        esac
    else
        case "$p" in
            tools/hooks/* | tools/mars-run.sh | tools/install-hooks.sh | .github/*)
                for guard in hooks_run_on_mars prepush_gate_cost_controls prepush_gate_tests_the_pushed_tree install_hooks_mirror_ci; do
                    add_guard tests "$guard" "$p -> the hook guard $guard"
                done
                ;;
        esac
        case "$p" in
            tools/run_ported_cells.sh)
                printf '1\n' > "$W/ported"
                ADDED=$((ADDED + 1))
                printf '%s\n' "$p -> the ported cells" >> "$W/why"
                ;;
        esac
    fi

    if [ "$ADDED" -eq 0 ] || [ "$p" != tools/run_ported_cells.sh ]; then
        map_file "$PUSHED" "$p" "$p"
    fi

    # A tracked dotfile at the root that no test names (libviprs main has carried
    # .commitmsg-631 since #660) is not code. It gets the cheap guards, the
    # ones every .github path gets, and not the whole suite (#375).
    if [ "$ADDED" -eq 0 ]; then
        case "$p" in
            .*)
                case "$p" in
                    */*) ;;
                    *)
                        if [ "$PUSHED" = core ]; then
                            for guard in local_gate_is_the_job_list local_gate_native_dwg_1295 miri_invocation_parity; do
                                add_guard core "$guard" "$p -> the CI parity guard $guard"
                            done
                        else
                            for guard in hooks_run_on_mars prepush_gate_cost_controls prepush_gate_tests_the_pushed_tree install_hooks_mirror_ci; do
                                add_guard tests "$guard" "$p -> the hook guard $guard"
                            done
                        fi
                        ;;
                esac
                ;;
        esac
    fi

    if [ "$ADDED" -eq 0 ]; then
        full "$p maps to no test target, and a scoped run never runs nothing"
    fi
done < "$CHANGED"

if [ "$SAW_ANY" = false ] && [ -z "$TRAILER_ITEMS" ]; then
    full "the push changes no path I was given, so there is nothing to scope to"
fi

# Always on: cheap, and no change to a path can scope them away.
# They are not what the push is "because" of, so they leave no reason behind.
always_on() {
    if tree_has "$1" "tests/$2.rs"; then
        printf '%s\n' "$2" >> "$W/${1}_test"
    else
        echo "Pre-push: the guard $1:$2 is not in the tree being gated, so it is not in the plan." >&2
    fi
}
if [ "$CORE_RS" = true ]; then
    always_on core miri_ignore_convention
fi
if [ "$PUSHED" = tests ]; then
    always_on tests install_hooks_mirror_ci
fi

# The Tests: trailer only ever adds.
for kind in core tests; do
    cat "$W/trailer_${kind}_test" >> "$W/${kind}_test"
done
cat "$W/trailer_core_lib" >> "$W/core_lib"
cat "$W/trailer_core_doc" >> "$W/core_doc"

for f in core_test tests_test core_lib core_doc; do
    sort -u "$W/$f" > "$W/$f.sorted" 2>/dev/null && mv "$W/$f.sorted" "$W/$f"
done

TOTAL=0
for f in core_test tests_test core_lib core_doc; do
    TOTAL=$((TOTAL + $(grep -c . "$W/$f")))
done
if [ -s "$W/ported" ]; then
    TOTAL=$((TOTAL + 1))
fi
if [ "$TOTAL" -eq 0 ]; then
    full "the selection came out empty, and a scoped run never runs nothing"
fi
if [ "$TOTAL" -gt "$CAP" ]; then
    full "the selection is $TOTAL targets, over the cap of $CAP (LIBVIPRS_GATE_SCOPE_CAP), so this is not the PR's own tests any more"
fi

# Required features, from the [[test]] entries of the tree each target is in.
features_for() {
    local kind="$1" want="$2" table
    table="$(tree_show "$kind" Cargo.toml | awk '
        function flush() { if (intest && name != "" && feats != "") print name, feats }
        /^\[\[test\]\]/ { flush(); intest = 1; name = ""; feats = ""; next }
        /^\[/ { flush(); intest = 0; next }
        intest && /^name[[:space:]]*=/ { s = $0; sub(/^[^"]*"/, "", s); sub(/".*/, "", s); name = s }
        intest && /^required-features[[:space:]]*=/ { s = $0; sub(/^[^\[]*\[/, "", s); sub(/\].*/, "", s); gsub(/[" ]/, "", s); feats = s }
        END { flush() }
    ')"
    [ -n "$table" ] || return 0
    while IFS= read -r t; do
        [ -n "$t" ] || continue
        printf '%s\n' "$table" | awk -v t="$t" '$1 == t { print $2 }'
    done < "$want" | tr ',' '\n' | sort -u | paste -sd, -
}

{
    echo lint
    for kind in core tests; do
        feats="$(features_for "$kind" "$W/${kind}_test")"
        [ -z "$feats" ] || echo "features|$kind|$feats"
    done
    sed 's/^/core|doc|/' "$W/core_doc"
    sed 's/^/core|lib|/' "$W/core_lib"
    sed 's/^/core|test|/' "$W/core_test"
    sed 's/^/tests|test|/' "$W/tests_test"
    [ ! -s "$W/ported" ] || echo ported
} | sed '/^$/d'

core_names="$(sed -e '/^$/d' "$W/core_test" "$W/core_lib" "$W/core_doc" | paste -sd' ' -)"
tests_names="$(sed -e '/^$/d' "$W/tests_test" | paste -sd' ' -)"
[ ! -s "$W/ported" ] || tests_names="$tests_names ported"
because="$(sort -u "$W/why" | sed -n '1,3p' | paste -sd';' - | sed 's/;/; /g')"
echo "Pre-push: scoped to $TOTAL targets (core: ${core_names:-none}; tests: ${tests_names:-none}) because $because" >&2
