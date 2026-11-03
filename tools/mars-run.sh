#!/usr/bin/env bash
# mars-run.sh: run a git hook's checks on MARS, never on this machine.
#
# The hooks used to run cargo, clippy and docker on the Mac, which is not where
# that work belongs and made a hook's verdict depend on this machine's state
# (libviprs-tests#311). This is the one place that knows how to do it somewhere
# else. It needs ssh, git and tar here and nothing more.
#
#   mars-run.sh hook REPO HOOKFILE     pre-commit: ship the staged tree (or the
#                                      tree of $LIBVIPRS_HOOK_TREEISH) and run
#                                      HOOKFILE there, inside the toolchain image
#   mars-run.sh gate ARGS...           pre-push: ship the trees named by
#                                      $LIBVIPRS_DIR and $LIBVIPRS_TESTS_DIR and
#                                      run tools/run-tests.sh ARGS... there
#   mars-run.sh clean                  remove the volumes and image this made
#
# Where things go on MARS (devshell@192.168.0.100, native x86_64):
#   * hook mode keeps no scratch directory at all. Sources live in one docker
#     volume per repo and worktree (viprs-hook-src-REPO-WORKTREE), cargo's
#     registry in viprs-hook-cargo and the build output in viprs-hook-target-REPO-WORKTREE (also per
#     worktree, so one worktree's newer artifacts can never make another skip compiling
#     its own sources, libviprs-tests#349), which is CARGO_TARGET_DIR=/tmp/target in the container. Each run streams a tar in
#     and rsyncs it over the volume by checksum, so only files that really
#     changed get a new mtime and cargo rebuilds only what changed.
#   * gate mode needs the host's docker (run-tests.sh builds an image), so it
#     gets a scratch dir /work/nas-work/hook-gate-* mounted at the same path
#     inside and out, plus the docker socket, and the dir is removed from a
#     container when the run ends, pass or fail.
#
# If MARS can't be reached this fails and says how to override. It never falls
# back to running anything here. LIBVIPRS_HOOKS_SKIP_MARS=1 skips the run, out
# loud, and exits 0: that is the whole override.
#
# Environment:
#   LIBVIPRS_HOOKS_SKIP_MARS=1   skip the run with a banner (MARS is down)
#   LIBVIPRS_HOOKS_MARS=U@HOST   default devshell@192.168.0.100
#   LIBVIPRS_HOOKS_RUST=X.Y.Z    toolchain tag of the image, default 1.99.1
#   LIBVIPRS_HOOKS_MEM=8g        memory ceiling of the lint container
#   LIBVIPRS_HOOKS_GATE_MEM=8g   memory ceiling of the suite's test container
#   LIBVIPRS_HOOKS_WAIT=1800     seconds to wait for another run of the same repo,
#                                or for a free suite slot
#   LIBVIPRS_HOOKS_GATE_SLOTS=2  suite runs allowed on MARS at once
#   LIBVIPRS_HOOK_TREEISH=REV    hook mode: gate REV's tree instead of the index
#   LIBVIPRS_TESTS_REV=REV       gate mode: use the harness at REV, not its checkout
#   LIBVIPRS_CORE_REV=pin|REV    gate mode: use the core COUNTERPART_REV names (pin) or REV
set -euo pipefail

MARS="${LIBVIPRS_HOOKS_MARS:-devshell@192.168.0.100}"
RUST="${LIBVIPRS_HOOKS_RUST:-1.99.1}"
MEM="${LIBVIPRS_HOOKS_MEM:-8g}"
WAIT="${LIBVIPRS_HOOKS_WAIT:-1800}"
IMAGE="viprs-hooks:$RUST"
SCRATCH_PARENT=/work/nas-work
SELF_DIR="$(cd "$(dirname "$0")" && pwd)"

say() { printf 'mars-run: %s\n' "$*" >&2; }
die() { printf 'mars-run: %s\n' "$*" >&2; exit 1; }

mssh() {
    ssh -o BatchMode=yes -o ConnectTimeout="${LIBVIPRS_HOOKS_CONNECT_TIMEOUT:-8}" \
        -o LogLevel=ERROR -o ServerAliveInterval=30 "$MARS" "$@"
}

# Skip or check, once, before anything is shipped.
preflight() {
    if [ "${LIBVIPRS_HOOKS_SKIP_MARS:-}" = 1 ]; then
        say "=================================================================="
        say "SKIPPED: LIBVIPRS_HOOKS_SKIP_MARS=1, so this hook ran NOTHING."
        say "No format, lint or test check was made. Nothing was run here either."
        say "Run it for real when MARS is back; the merge gate still applies."
        say "=================================================================="
        exit 0
    fi
    if ! mssh 'docker info >/dev/null 2>&1'; then
        say "cannot reach MARS ($MARS) or its docker is not answering."
        say "This hook runs on MARS only, so I'm stopping instead of passing."
        say "If MARS is down on purpose: LIBVIPRS_HOOKS_SKIP_MARS=1 git ... skips this"
        say "check loudly. Do not use --no-verify for that."
        exit 1
    fi
}

# Build the toolchain image on MARS the first time. The Dockerfile is read from
# stdin, so there is no build context to ship.
ensure_image() {
    if mssh "docker image inspect $IMAGE >/dev/null 2>&1"; then
        return 0
    fi
    say "building $IMAGE on MARS (first run only)"
    # DOCKER_CONFIG because ~/.docker on MARS is a symlink to a directory that
    # does not exist, which stops buildx before it starts. The classic builder
    # is enough for a Dockerfile this small.
    mssh "DOCKER_BUILDKIT=0 DOCKER_CONFIG=/tmp/viprs-docker-config docker build -q --platform linux/amd64 --build-arg RUST_VERSION=$RUST -t $IMAGE -" \
        < "$SELF_DIR/Dockerfile.hooks" >/dev/null \
        || die "building $IMAGE on MARS failed"
}

# git without the variables a hook inherits. They beat -C and the working
# directory, so leaving them set makes every git call below answer for the
# repository that is committing instead of the one it was pointed at.
cleangit() {
    env -u GIT_DIR -u GIT_WORK_TREE -u GIT_INDEX_FILE -u GIT_PREFIX -u GIT_QUARANTINE_PATH git "$@"
}

# The tree object of DIR's tracked files exactly as they are on disk now. Built
# in a throwaway index so DIR's own index is never touched.
snapshot_tree() {
    local dir="$1" gd idx tree
    gd="$(cleangit -C "$dir" rev-parse --absolute-git-dir)"
    idx="$gd/mars-run-index.$$"
    rm -f "$idx"
    tree="$(
        export GIT_INDEX_FILE="$idx"
        cleangit() { env -u GIT_DIR -u GIT_WORK_TREE -u GIT_PREFIX git "$@"; }
        cleangit -C "$dir" read-tree HEAD
        cleangit -C "$dir" add -u
        cleangit -C "$dir" write-tree
    )"
    rm -f "$idx"
    printf '%s\n' "$tree"
}

# First non-comment, non-blank line of a file in a tree.
pinned_rev() {
    local gitdir_of="$1" tree="$2" file="$3"
    cleangit -C "$gitdir_of" show "$tree:$file" 2>/dev/null \
        | { grep -v -E '^[[:space:]]*(#|$)' || true; } | head -1 | tr -d '[:space:]'
}

# Make sure REV exists in the core checkout, fetching once if it does not.
have_rev() {
    local core="$1" rev="$2"
    cleangit -C "$core" cat-file -e "$rev^{commit}" 2>/dev/null && return 0
    cleangit -C "$core" fetch -q origin 2>/dev/null || true
    cleangit -C "$core" cat-file -e "$rev^{commit}" 2>/dev/null
}

# ---------------------------------------------------------------------------
# hook mode
# ---------------------------------------------------------------------------
hook_mode() {
    local repo="$1" hookfile="$2"
    [ -f "$hookfile" ] || die "no hook file at $hookfile"
    # First, so an unreachable MARS (or the override) is answered before any
    # tree is worked out or shipped.
    preflight

    local top main ws tree sib_rev="" core=""
    top="$(cleangit rev-parse --show-toplevel)"
    main="$(cd "$top" && cd "$(dirname "$(cleangit rev-parse --git-common-dir)")" && pwd)"
    ws="$(dirname "$main")"

    # The tree being checked. A commit checks what is staged; a push in
    # compile-only mode (Red-tests:) checks the commit going out.
    if [ -n "${LIBVIPRS_HOOK_TREEISH:-}" ]; then
        tree="$(cleangit -C "$top" rev-parse "${LIBVIPRS_HOOK_TREEISH}^{tree}")"
    else
        tree="$(cd "$top" && git write-tree)"
    fi

    # The core is a path dependency of the other two. Pin it the way CI does:
    # the harness at COUNTERPART_REV, the bench at the same-named branch or
    # main. Never whatever the sibling checkout happens to have checked out.
    case "$repo" in
        libviprs-tests)
            core="$ws/libviprs"
            sib_rev="$(pinned_rev "$top" "$tree" COUNTERPART_REV)"
            [ -n "$sib_rev" ] || die "COUNTERPART_REV has no revision in the tree being checked"
            ;;
        libviprs-bench)
            core="$ws/libviprs"
            cleangit -C "$core" fetch -q origin 2>/dev/null || say "could not fetch the core, using what is here"
            local branch
            branch="$(cleangit -C "$top" symbolic-ref --quiet --short HEAD 2>/dev/null || true)"
            if [ -n "$branch" ] && sib_rev="$(cleangit -C "$core" rev-parse --verify --quiet "origin/$branch^{commit}")"; then
                :
            else
                sib_rev="$(cleangit -C "$core" rev-parse --verify --quiet "origin/main^{commit}")" \
                    || die "the core checkout at $core has no origin/main"
            fi
            ;;
        *) : ;;
    esac
    if [ -n "$core" ]; then
        [ -d "$core/.git" ] || [ -f "$core/.git" ] || die "no libviprs checkout at $core (the path dependency)"
        have_rev "$core" "$sib_rev" || die "libviprs revision $sib_rev is not in $core, even after a fetch"
    fi

    ensure_image

    # One name per run: repo, worktree (its directory name plus a checksum of
    # its path, so two checkouts called the same do not meet) and this pid. The
    # source volumes are per worktree for the same reason, so runs from
    # different worktrees of one repo go side by side. Two runs from the SAME
    # worktree would still sync one tree over the other's build, so those wait
    # for each other (libviprs-tests#317).
    local wt wtid b64 mounts name
    wt="$(basename "$top" | tr -c 'A-Za-z0-9_.\n-' '_')"
    wtid="$wt-$(printf %s "$top" | cksum | cut -d' ' -f1)"
    name="viprs-hook-$repo-$wtid-$$"
    b64="$(base64 < "$hookfile" | tr -d '\n')"
    mounts="-v viprs-hook-src-$repo-$wtid:/w/$repo"
    [ -n "$core" ] && mounts="$mounts -v viprs-hook-sib-$repo-$wtid:/w/libviprs"

    wait_for_worktree "viprs-hook-$repo-$wtid-"
    say "running the $repo checks on MARS ($(printf '%.12s' "$tree"))"
    trap 'mssh "docker kill $name >/dev/null 2>&1" || true' INT TERM
    # The driver below runs in the container: unpack, sync, run the hook file.
    # It is single-quoted on purpose, nothing in it should expand here.
    # shellcheck disable=SC2016
    local driver='set -euo pipefail
mkdir -p /new
tar -xi -f - -C /new
rsync -rlpc --delete "/new/$MARS_REPO/" "/w/$MARS_REPO/"
if [ -d /new/libviprs ]; then rsync -rlpc --delete /new/libviprs/ /w/libviprs/; fi
rm -rf /new
printf %s "$MARS_HOOK_B64" | base64 -d > /tmp/hook.sh
cd "/w/$MARS_REPO"
LIBVIPRS_ON_MARS=1 exec bash /tmp/hook.sh'
    local rc=0
    {
        cleangit -C "$top" archive --format=tar --prefix="$repo/" "$tree"
        if [ -n "$core" ]; then
            cleangit -C "$core" archive --format=tar --prefix=libviprs/ "$sib_rev"
        fi
    } | mssh "docker run --rm -i --name $name --platform linux/amd64 --memory $MEM \
        -e CARGO_HOME=/cargo -e CARGO_TARGET_DIR=/tmp/target -e CARGO_PROFILE_DEV_DEBUG=0 -e CARGO_INCREMENTAL=0 -e MARS_REPO=$repo -e MARS_HOOK_B64=$b64 \
        -v viprs-hook-cargo:/cargo -v viprs-hook-target-$repo-$wtid:/tmp/target $mounts \
        $IMAGE bash -c $(printf '%q' "$driver")" || rc=$?
    trap - INT TERM
    if [ "$rc" -eq 255 ]; then
        die "lost the connection to MARS mid-run, so the checks did not finish. Run again."
    fi
    return "$rc"
}

# Another run from this worktree is using its source volume: wait for it rather
# than racing it, which would sync one tree over another's build. Runs from
# other worktrees have their own volumes and are not waited for.
wait_for_worktree() {
    local prefix="$1" waited=0 holder
    while holder="$(mssh "docker ps --filter name=^$prefix --format '{{.Names}}'" 2>/dev/null || true)"; [ -n "$holder" ]; do
        if [ "$waited" -ge "$WAIT" ]; then
            die "$holder has held this worktree's volume on MARS for ${WAIT}s, giving up"
        fi
        [ "$waited" -eq 0 ] && say "$holder is still running from this worktree, waiting"
        sleep 5
        waited=$((waited + 5))
    done
}

# ---------------------------------------------------------------------------
# gate mode
# ---------------------------------------------------------------------------
# MARS is shared and a suite run is a docker build plus a test run, so at most
# LIBVIPRS_HOOKS_GATE_SLOTS (default 2) go at once and the rest queue. The count
# is the live driver containers, so there is no lock to go stale.
wait_for_gate_slot() {
    local slots="${LIBVIPRS_HOOKS_GATE_SLOTS:-2}" waited=0 running holders
    while true; do
        running="$(mssh 'docker ps -q --filter name=hook-gate-' 2>/dev/null | grep -c . || true)"
        [ "${running:-0}" -lt "$slots" ] && return 0
        holders="$(mssh "docker ps --filter name=hook-gate- --format '{{.Names}} ({{.RunningFor}})'" 2>/dev/null | tr '\n' ' ' || true)"
        if [ "$waited" -ge "$WAIT" ]; then
            die "$running suite runs have held MARS for ${WAIT}s, giving up. Holders: ${holders:-unknown}"
        fi
        [ "$waited" -eq 0 ] && say "$running suite runs are already going on MARS (limit $slots), waiting. Holders: ${holders:-unknown}"
        sleep 15
        waited=$((waited + 15))
    done
}

GATE_JOB=""
gate_cleanup() {
    [ -n "$GATE_JOB" ] || return 0
    mssh "docker run --rm --platform linux/amd64 -v $SCRATCH_PARENT:/w alpine:3.20 rm -rf /w/$GATE_JOB" \
        >/dev/null 2>&1 || say "could not remove $SCRATCH_PARENT/$GATE_JOB on MARS, remove it from a container"
}

gate_mode() {
    preflight
    # The toolchain image, not the bare rust one: run-tests.sh only copies the
    # refs into its build context when rsync is there (the tags the changelog
    # guard reads), and rsync is in this image.
    ensure_image
    wait_for_gate_slot
    local core="${LIBVIPRS_DIR:-}" tests="${LIBVIPRS_TESTS_DIR:-}"
    if [ -z "$core" ] || [ -z "$tests" ]; then
        die "gate mode needs LIBVIPRS_DIR and LIBVIPRS_TESTS_DIR"
    fi
    local core_tree tests_tree
    # LIBVIPRS_TESTS_REV names a revision of the harness to use instead of the
    # checkout's working tree. A core push sets it to origin/main: the sibling
    # checkout is somebody's working branch, and gating a core change against
    # it answers a question about that branch.
    if [ -n "${LIBVIPRS_TESTS_REV:-}" ]; then
        cleangit -C "$tests" fetch -q origin 2>/dev/null || say "could not fetch libviprs-tests, using what is here"
        tests_tree="$(cleangit -C "$tests" rev-parse --verify "${LIBVIPRS_TESTS_REV}^{tree}")" \
            || die "no ${LIBVIPRS_TESTS_REV} in the libviprs-tests checkout at $tests"
    else
        tests_tree="$(snapshot_tree "$tests")"
    fi
    # LIBVIPRS_CORE_REV is the same idea for the core. `pin` means the revision
    # the harness being gated names in COUNTERPART_REV, which is what hosted CI
    # and the merge gate pair it with; any other value is a revision of the core.
    if [ -n "${LIBVIPRS_CORE_REV:-}" ]; then
        local core_rev="$LIBVIPRS_CORE_REV"
        if [ "$core_rev" = pin ]; then
            core_rev="$(pinned_rev "$tests" "$tests_tree" COUNTERPART_REV)"
            [ -n "$core_rev" ] || die "COUNTERPART_REV has no revision in the harness being gated"
        fi
        have_rev "$core" "$core_rev" || die "libviprs revision $core_rev is not in $core, even after a fetch"
        core_tree="$(cleangit -C "$core" rev-parse --verify "${core_rev}^{tree}")"
    else
        core_tree="$(snapshot_tree "$core")"
    fi

    local dir
    local args=""
    # Global: the EXIT trap below runs after this function has returned.
    GATE_JOB="hook-gate-$$-$(date +%s)"
    local job="$GATE_JOB"
    dir="$SCRATCH_PARENT/$job"
    local a
    for a in ${@+"$@"}; do args="${args} $(printf '%q' "$a")"; done

    trap gate_cleanup EXIT
    trap 'mssh "docker kill $GATE_JOB >/dev/null 2>&1" || true; exit 130' INT TERM

    # The trees go over as git archive output, which has no .git, and
    # run-tests.sh wants one in each: it stages the index and the refs into its
    # build context so the guards that ask git what is tracked (and the tags)
    # can answer. So a container unpacks them and makes each a one-commit
    # repository, with the tags the real repository has as bare refs. That is
    # the state the guards already treat as "tags present, history absent".
    say "shipping libviprs ($(printf '%.12s' "$core_tree")) and libviprs-tests ($(printf '%.12s' "$tests_tree")) to MARS"
    # shellcheck disable=SC2016
    local setup='set -euo pipefail
tar -xi -f - -C "$MARS_DIR"
mkrepo() {
    cd "$MARS_DIR/$1"
    git init -q
    git config core.fileMode true
    git add -A
    git -c user.name=gate -c user.email=gate@localhost commit -q -m gate
    printf "%s\n" "$2" | while read -r sha name; do
        [ -n "${name:-}" ] || continue
        mkdir -p ".git/refs/tags/$(dirname "$name")"
        printf "%s\n" "$sha" > ".git/refs/tags/$name"
    done
}
git config --global --add safe.directory "*"
mkrepo libviprs "$MARS_TAGS_CORE"
mkrepo libviprs-tests "$MARS_TAGS_TESTS"'
    local core_tags tests_tags
    core_tags="$(cleangit -C "$core" for-each-ref --format='%(objectname) %(refname:strip=2)' refs/tags)"
    tests_tags="$(cleangit -C "$tests" for-each-ref --format='%(objectname) %(refname:strip=2)' refs/tags)"
    mssh "mkdir -p $dir" || die "could not make the scratch dir on MARS"
    {
        cleangit -C "$core" archive --format=tar --prefix=libviprs/ "$core_tree"
        cleangit -C "$tests" archive --format=tar --prefix=libviprs-tests/ "$tests_tree"
    } | mssh "docker run --rm -i --platform linux/amd64 -v $dir:$dir \
        -e MARS_DIR=$dir -e MARS_TAGS_CORE=$(printf '%q' "$core_tags") -e MARS_TAGS_TESTS=$(printf '%q' "$tests_tags") \
        $IMAGE bash -c $(printf '%q' "$setup")" || die "shipping the trees to MARS failed"

    say "running tools/run-tests.sh on MARS"
    local rc=0
    mssh "docker run --rm --name $job --platform linux/amd64 \
        -v $dir:$dir -v /var/run/docker.sock:/var/run/docker.sock -v /usr/bin/docker:/usr/bin/docker:ro \
        -v /usr/libexec/docker/cli-plugins:/usr/libexec/docker/cli-plugins:ro \
        -e LIBVIPRS_DIR=$dir/libviprs -e LIBVIPRS_TESTS_DIR=$dir/libviprs-tests \
        -e RUN_TESTS_MEMORY=${LIBVIPRS_HOOKS_GATE_MEM:-8g} \
        -w $dir/libviprs-tests $IMAGE \
        bash -c $(printf '%q' "tools/run-tests.sh$args")" </dev/null || rc=$?
    if [ "$rc" -eq 255 ]; then
        die "lost the connection to MARS mid-run, so the suite did not finish. Run again."
    fi
    return "$rc"
}

# ---------------------------------------------------------------------------
clean_mode() {
    local v
    for v in $(mssh "docker volume ls -q --filter name=viprs-hook-"); do
        mssh "docker volume rm $v" >/dev/null && say "removed volume $v"
    done
    if mssh "docker rmi $IMAGE" >/dev/null 2>&1; then
        say "removed image $IMAGE"
    fi
}

cmd="${1:-}"
[ $# -gt 0 ] && shift
case "$cmd" in
    hook)
        [ $# -eq 2 ] || die "usage: mars-run.sh hook REPO HOOKFILE"
        hook_mode "$1" "$2"
        ;;
    gate) gate_mode "$@" ;;
    clean) clean_mode ;;
    *)
        sed -n '2,/^set -euo pipefail$/p' "$0" | sed '$d' >&2
        exit 2
        ;;
esac
