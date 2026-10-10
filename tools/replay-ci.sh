#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# replay-ci.sh: run ci.yml's jobs here, step for step, so a local green means
# what a CI green means (libviprs-tests#234).
#
#   tools/replay-ci.sh                 every replayed job, in the order below
#   tools/replay-ci.sh JOB [JOB...]    just those jobs
#   tools/replay-ci.sh --list          print the jobs and steps, run nothing
#
# Run it from a libviprs-tests checkout laid out the way CI lays it out: the
# core at ../libviprs and the CLI at ../libviprs-cli, at COUNTERPART_REV and
# CLI_COUNTERPART_REV. It has to be a Linux x86_64 host (or container), because
# that is what the runners are and what the pinned PDFium and go-pmtiles
# downloads are built for. It needs curl, sha256sum, tar, git, shellcheck, and
# cargo with rustfmt and clippy. Run as root it installs PDFium and go-pmtiles
# where the CI steps install them; otherwise those steps call the real sudo.
#
# Why a copy and not a parser
#
# Every job below is a copy of its ci.yml job: the same steps in the same
# order, each with the same environment and the same code. I copied rather
# than parsed on purpose. A shell YAML reader would be a second, worse parser
# beside the one in tests/common/workflows.rs, and the copy can be read side
# by side with the workflow. What keeps the copy honest is
# tests/feature_cells_ci_and_local.rs: it parses ci.yml and this file and fails
# on any difference in jobs, step order, environment or code, and on any
# ci.yml job that is neither replayed here nor listed in its
# JOBS_NOT_REPLAYED with a reason. So edit ci.yml and this file in the same
# commit, and let that test tell you when they disagree.
#
# What a step gets, so it behaves the way it does on a runner
#
#   * the workflow-wide env (below), plus the step's own env;
#   * $RUNNER_TEMP, a scratch dir of this run's own;
#   * $GITHUB_ENV, a file whose NAME=VALUE lines are exported to the later
#     steps of the same job and dropped when the job ends, as on a runner;
#   * bash -e, which is what a runner uses for a `run:` with no `shell:`;
#   * the checkout as its working directory, except a step that verifies a
#     download (its env names a *_SHA256), which gets an empty dir of its own
#     so the tarballs it unpacks never land in the tree the cargo steps read.
#
# A job whose ci.yml version does not lay the CLI sibling down
# (clone-cli-counterpart) gets VIPRS_CLI_DIR pointed at a path that does not
# exist, so its CLI cells skip here exactly as they skip there. The CLI cells
# are meant to run in cli-differential, under VIPRS_REQUIRE_CLI=1, and they do.
#
# A failing step fails its job and skips the rest of that job, as on a runner.
# Every selected job still runs, and the summary at the end names every step
# with its result. The exit status is 1 if any step failed.
#
# Not replayed (see JOBS_NOT_REPLAYED in the test for the reasons):
# feature-cells and hook-mirror.
# ---------------------------------------------------------------------------
set -uo pipefail

# The workflow-wide env block of ci.yml.
export CARGO_TERM_COLOR=always
export RUSTFLAGS=-Dwarnings

# The replayed jobs whose ci.yml version uses clone-cli-counterpart.
JOBS_WITH_CLI_SIBLING="cli-differential"

SELF_DIR="$(cd "$(dirname "$0")" && pwd)"
TREE="$(cd "$SELF_DIR/.." && pwd)"

LIST_ONLY=0
SELECTED=()
for arg in "$@"; do
    case "$arg" in
        --list) LIST_ONLY=1 ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        -*) echo "replay-ci: unknown option $arg" >&2; exit 2 ;;
        *) SELECTED+=("$arg") ;;
    esac
done

fail_setup() {
    echo "replay-ci: $*" >&2
    exit 2
}

if [ "$LIST_ONLY" = 0 ]; then
    [ "$(uname -s) $(uname -m)" = "Linux x86_64" ] \
        || fail_setup "this is $(uname -s) $(uname -m); CI runs on Linux x86_64 and the pinned downloads are built for it"
    for tool in curl sha256sum tar git shellcheck cargo; do
        command -v "$tool" >/dev/null 2>&1 || fail_setup "needs $tool on PATH"
    done
    if ! cargo fmt --version >/dev/null 2>&1 || ! cargo clippy --version >/dev/null 2>&1; then
        fail_setup "needs rustfmt and clippy (rustup component add rustfmt clippy)"
    fi
    for sib in libviprs libviprs-cli; do
        [ -f "$TREE/../$sib/Cargo.toml" ] || fail_setup "no $sib checkout at $TREE/../$sib"
    done
    RUNNER_TEMP="${REPLAY_TEMP:-$(mktemp -d "${TMPDIR:-/tmp}/replay-ci.XXXXXX")}"
    mkdir -p "$RUNNER_TEMP"
    export RUNNER_TEMP
    echo "replay-ci: tree $TREE, scratch $RUNNER_TEMP"
    echo "replay-ci: $(rustc --version) on $(uname -m)"
    pin() { grep -E '^[0-9a-f]{40}$' "$TREE/$1" | head -1; }
    for pair in libviprs:COUNTERPART_REV libviprs-cli:CLI_COUNTERPART_REV; do
        sib=${pair%%:*}; file=${pair#*:}
        have="no .git, so not checked"
        if git -C "$TREE/../$sib" rev-parse HEAD >/dev/null 2>&1; then
            have=$(git -C "$TREE/../$sib" rev-parse HEAD)
        fi
        echo "replay-ci: ../$sib is at $have, $file pins $(pin "$file")"
    done
fi

# On a runner `sudo` is there and passwordless. In a root container it is
# usually not installed, and as root it would change nothing anyway.
sudo() {
    if [ "$(id -u)" -eq 0 ]; then "$@"; else command sudo "$@"; fi
}
export -f sudo

RESULTS=()
FAILED=0
JOB=""
JOB_ON=0
JOB_BROKEN=0
JOB_STEP=0
JOB_VARS=()

selected() {
    [ "${#SELECTED[@]}" -eq 0 ] && return 0
    local s
    for s in "${SELECTED[@]}"; do [ "$s" = "$1" ] && return 0; done
    return 1
}

end_job() {
    local v
    for v in "${JOB_VARS[@]}"; do unset "$v"; done
    JOB_VARS=()
    unset VIPRS_CLI_DIR
}

SEEN_JOBS=" "

job() {
    end_job
    JOB=$1
    SEEN_JOBS="$SEEN_JOBS$JOB "
    JOB_STEP=0
    JOB_BROKEN=0
    JOB_ON=0
    selected "$JOB" && JOB_ON=1
    [ "$JOB_ON" = 1 ] || return 0
    echo ""
    echo "================================================================"
    echo "job $JOB"
    echo "================================================================"
    [ "$LIST_ONLY" = 1 ] && return 0
    case " $JOBS_WITH_CLI_SIBLING " in
        *" $JOB "*) ;;
        *) export VIPRS_CLI_DIR="$RUNNER_TEMP/no-cli-sibling-in-$JOB" ;;
    esac
    GITHUB_ENV="$RUNNER_TEMP/$JOB.github_env"
    : > "$GITHUB_ENV"
    export GITHUB_ENV
}

# step [NAME=VALUE ...] <<'STEP' ... STEP
step() {
    local body
    body=$(cat)
    [ "$JOB_ON" = 1 ] || return 0
    JOB_STEP=$((JOB_STEP + 1))
    local label="$JOB step $JOB_STEP"
    local first
    first=$(printf '%s\n' "$body" | grep -v '^[[:space:]]*#' | grep -v '^[[:space:]]*$' | head -1)
    echo ""
    echo "---- $label: ${*:+$* }$first"
    if [ "$LIST_ONLY" = 1 ]; then
        return 0
    fi
    if [ "$JOB_BROKEN" = 1 ]; then
        echo "skipped: an earlier step of $JOB failed"
        RESULTS+=("SKIP  $label: $first")
        return 0
    fi
    local cwd="$TREE" kv
    for kv in "$@"; do
        case "${kv%%=*}" in
            *_SHA256) cwd="$RUNNER_TEMP/$JOB-step-$JOB_STEP"; mkdir -p "$cwd" ;;
        esac
    done
    local start rc
    start=$(date +%s)
    (cd "$cwd" && env "$@" bash -e -c "$body")
    rc=$?
    local took=$(( $(date +%s) - start ))
    if [ "$rc" -eq 0 ]; then
        RESULTS+=("PASS  $label (${took}s): $first")
    else
        RESULTS+=("FAIL  $label (${took}s, exit $rc): $first")
        JOB_BROKEN=1
        FAILED=1
    fi
    local line
    while IFS= read -r line; do
        case "$line" in
            *=*)
                export "${line%%=*}=${line#*=}"
                JOB_VARS+=("${line%%=*}")
                ;;
        esac
    done < "$GITHUB_ENV"
    : > "$GITHUB_ENV"
}

# ---------------------------------------------------------------------------
# The jobs. Each one is its ci.yml job with the `uses:` steps left out: the
# checkout and the two pinned siblings are the tree this runs in, the
# toolchain is whatever cargo is on PATH, and the cache is the target dir.
# ---------------------------------------------------------------------------

job test
step <<'STEP'
cargo test
STEP
step <<'STEP'
cargo test --features "ported_tests tracing" --test phase3_tracing
STEP
step <<'STEP'
cargo test --features object-store-sink --test phase3_object_store_sink --test phase3_validation_stress --test sink_object_store_stub_contract --test objectstore_list_trait_default
STEP
step <<'STEP'
cargo test --features packfile --test phase3_packfile --test builder_sink_packfile
STEP
step CODEC_E2E_REQUIRE_FEATURES=all <<'STEP'
cargo test --features "jxl avif svg jp2k" --test codec_e2e
STEP

job test-pdfium
step PDFIUM_RELEASE=pdfium-8085 PDFIUM_SHA256=ec671f549717c8c3e1642680cd834a9dbc580fb3c239bc3a426e9475166aaa63 <<'STEP'
curl -fsSL -o pdfium.tgz "https://github.com/libviprs/libviprs-dep/releases/download/${PDFIUM_RELEASE}/pdfium-linux-x64.tgz"
echo "${PDFIUM_SHA256}  pdfium.tgz" | sha256sum -c -
mkdir -p pdfium
tar xzf pdfium.tgz -C pdfium --strip-components=1
sudo cp pdfium/lib/libpdfium.so /usr/local/lib/
sudo cp -r pdfium/include/* /usr/local/include/
sudo ldconfig
STEP
step <<'STEP'
cargo test --features pdfium
STEP
step <<'STEP'
cargo test --features "ported_tests pdfium" --test ported_foreign --test core_review_followups -- --exact test_pdf_dpi_scale test_pdf_background test_pdf_password pdf_background_typed_matches_vips
STEP

job ported-tests
step <<'STEP'
./tools/fetch_reference_suite.sh
STEP
step <<'STEP'
./tools/run_ported_cells.sh --clippy
STEP
step <<'STEP'
./tools/run_ported_cells.sh --require-fixtures
STEP

job cli-differential
step VIPRS_REQUIRE_CLI=1 <<'STEP'
cargo test --test install_hooks_mirror_ci --test cli_morphology_diff --test cli_bands_diff --test cli_extract_diff --test cli_conversion_diff --test cli_core_diff --test cli_convolution_diff --test cli_matrix_diff --test cli_colour_diff --test cli_resample_diff --test cli_histogram_diff --test cli_composite_diff --test cli_freqfilt_diff --test cli_mosaicing_diff --test cli_create_diff --test cli_draw_diff --test cli_aritha_diff --test cli_arithb_diff --test cli_iocleanup_diff --test cli_pmtiles --test cli_features --test cli_builtins_e2e --test cli_foreign_diff --test cli_op_map_counts --test cli_pyramid_pipeline --test cli_surface_coverage
STEP
step VIPRS_REQUIRE_CLI=1 <<'STEP'
cargo test --features "jxl jp2k avif svg" --test cli_foreign_diff
STEP
step PDFIUM_RELEASE=pdfium-8085 PDFIUM_SHA256=ec671f549717c8c3e1642680cd834a9dbc580fb3c239bc3a426e9475166aaa63 <<'STEP'
curl -fsSL -o pdfium.tgz "https://github.com/libviprs/libviprs-dep/releases/download/${PDFIUM_RELEASE}/pdfium-linux-x64.tgz"
echo "${PDFIUM_SHA256}  pdfium.tgz" | sha256sum -c -
mkdir -p "$RUNNER_TEMP/pdfium"
tar xzf pdfium.tgz -C "$RUNNER_TEMP/pdfium" --strip-components=1
echo "PDFIUM_PATH=$RUNNER_TEMP/pdfium/lib/libpdfium.so" >> "$GITHUB_ENV"
STEP
step <<'STEP'
cargo build --release --bin viprs --manifest-path ../libviprs-cli/Cargo.toml --target-dir "$RUNNER_TEMP/viprs-pdfium"
echo "VIPRS_BIN=$RUNNER_TEMP/viprs-pdfium/release/viprs" >> "$GITHUB_ENV"
STEP
step VIPRS_REQUIRE_CLI=1 VIPRS_REQUIRE_PDFIUM=1 <<'STEP'
cargo test --test cli_pdf_geo_plan --test cli_page_size_parity
STEP
step VIPRS_REQUIRE_CLI=1 <<'STEP'
cargo test --features s3 --test cli_pyramid_pipeline -- the_object_store_sink retries_then_fail
STEP

job pmtiles-interop
step GO_PMTILES_VERSION=1.31.2 GO_PMTILES_TARBALL_SHA256=3ed7dbf4ec2e6dfe5e25b6f70d1ffc932729f93c86db353bf514dd71010a312f GO_PMTILES_BINARY_SHA256=a7e9ae10184d109c83f456ccdf6df4f3e2a64ba6cf69d9ed0f9f1840305055c1 <<'STEP'
set -euo pipefail
curl -fsSL -o go-pmtiles.tgz \
  "https://github.com/protomaps/go-pmtiles/releases/download/v${GO_PMTILES_VERSION}/go-pmtiles_${GO_PMTILES_VERSION}_Linux_x86_64.tar.gz"
echo "${GO_PMTILES_TARBALL_SHA256}  go-pmtiles.tgz" | sha256sum -c -
mkdir -p go-pmtiles
tar xzf go-pmtiles.tgz -C go-pmtiles pmtiles
echo "${GO_PMTILES_BINARY_SHA256}  go-pmtiles/pmtiles" | sha256sum -c -
sudo install -m 0755 go-pmtiles/pmtiles /usr/local/bin/pmtiles
pmtiles version
STEP
step VIPRS_REQUIRE_GO_PMTILES=1 <<'STEP'
cargo test --test pmtiles_interop --test phase_pmtiles --test pmtiles_bounded --test pmtiles_ci_wiring --test pmtiles_sweep --test pmtiles_migrate_google --test pipeline_e2e
STEP

job lint
step <<'STEP'
cargo fmt -- --check
STEP
step <<'STEP'
cargo clippy --all-targets -- -D warnings
STEP
step <<'STEP'
shellcheck tools/*.sh tools/hooks/pre-push
STEP

end_job

for s in "${SELECTED[@]}"; do
    case "$SEEN_JOBS" in
        *" $s "*) ;;
        *) echo "replay-ci: no replayed job called $s (have:$SEEN_JOBS)" >&2; exit 2 ;;
    esac
done

[ "$LIST_ONLY" = 1 ] && exit 0

echo ""
echo "================================================================"
echo "replay-ci summary"
echo "================================================================"
if [ "${#RESULTS[@]}" -eq 0 ]; then
    echo "no step ran: no job matched ${SELECTED[*]}" >&2
    exit 2
fi
printf '%s\n' "${RESULTS[@]}"
if [ "$FAILED" = 1 ]; then
    echo "replay-ci: FAILED"
    exit 1
fi
echo "replay-ci: every step passed"
