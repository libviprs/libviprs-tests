#!/usr/bin/env bash
# Fetch the libvips reference test-suite images at a pinned revision.
#
# The ported_* integration tests (feature = "ported_tests") compare libviprs
# output against the real libvips reference fixtures. Those fixtures live
# upstream in libvips at test/test-suite/images and are git-ignored here (they
# land under tmp/). Without a fetch step the reference suite can only report the
# fixtures as missing (issue #58). This script populates them deterministically
# from a single pinned commit so a CI run and a local run see identical inputs.
#
# Usage:
#   ./tools/fetch_reference_suite.sh [--force]
#
# Output:
#   tmp/libvips-reference-tests/test-suite/images/   (the reference fixtures)
#
# Verify the fetch populated everything the suite needs:
#   cargo test --features ported_tests --test fixture_audit

set -euo pipefail

# --- Pinned upstream revision -------------------------------------------------
# libvips v8.18.4. The SHA is authoritative; the tag is a human label. Bump both
# together and re-run the fixture_audit test to confirm a candidate revision
# still carries every fixture the audit registry requires.
LIBVIPS_REPO="https://github.com/libvips/libvips.git"
LIBVIPS_REV="e01a4797cabe77d457fdfa7d776b7a7e7ca6d6a7"
LIBVIPS_TAG="v8.18.4"
SUBTREE="test/test-suite"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEST="$REPO_ROOT/tmp/libvips-reference-tests"
IMAGES_DIR="$DEST/test-suite/images"
MARKER="$DEST/.pinned-rev"

FORCE=0
if [ "${1:-}" = "--force" ]; then
    FORCE=1
elif [ "$#" -gt 0 ]; then
    echo "ERROR: unknown argument: $1 (only --force is accepted)" >&2
    exit 2
fi

# Idempotent: skip when already populated at the pinned rev, unless --force.
if [ "$FORCE" -eq 0 ] && [ -f "$MARKER" ] && [ -d "$IMAGES_DIR" ] \
   && [ "$(cat "$MARKER" 2>/dev/null)" = "$LIBVIPS_REV" ]; then
    echo "Reference suite already at $LIBVIPS_REV ($LIBVIPS_TAG); nothing to do."
    echo "Pass --force to refetch."
    exit 0
fi

command -v git >/dev/null 2>&1 || { echo "ERROR: git is required." >&2; exit 1; }

echo "==> Fetching libvips reference suite ($LIBVIPS_TAG / $LIBVIPS_REV)"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Partial + sparse clone: download only the subtree's blobs at exactly one
# commit, so we never pull the whole libvips history or source tree.
git -C "$WORK" init -q
git -C "$WORK" remote add origin "$LIBVIPS_REPO"
git -C "$WORK" config core.sparseCheckout true
printf '%s/*\n' "$SUBTREE" > "$WORK/.git/info/sparse-checkout"
git -C "$WORK" fetch --depth 1 --filter=blob:none origin "$LIBVIPS_REV"
git -C "$WORK" checkout -q FETCH_HEAD

if [ ! -d "$WORK/$SUBTREE/images" ]; then
    echo "ERROR: expected $SUBTREE/images not present at $LIBVIPS_REV" >&2
    exit 1
fi

# Publish into tmp/ (replace any prior copy) and stamp the pinned rev.
rm -rf "$DEST"
mkdir -p "$DEST"
cp -R "$WORK/$SUBTREE" "$DEST/test-suite"
printf '%s\n' "$LIBVIPS_REV" > "$MARKER"

count="$(find "$IMAGES_DIR" -type f | wc -l | tr -d ' ')"
echo "==> Done: $count reference image(s) under $IMAGES_DIR"
