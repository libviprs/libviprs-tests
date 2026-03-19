#!/usr/bin/env bash
# Generate Google Maps + centre reference fixtures using vips via Docker.
#
# Usage:
#   cd libviprs-tests/tests/fixtures
#   bash gen_google_centre_fixtures.sh
#
# Prerequisites: Docker must be running.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

echo "==> Generating Google+centre fixtures via Docker (debian:bookworm-slim + libvips-tools)..."

docker run --rm \
  -v "$SCRIPT_DIR":/data \
  -w /data \
  debian:bookworm-slim \
  sh -c '
    apt-get update -qq && apt-get install -y -qq libvips-tools > /dev/null 2>&1 && \
    vips dzsave blueprint.pdf blueprint_google_centre --layout google --centre --suffix .png && \
    vips dzsave blueprint-portrait.pdf blueprint_portrait_google_centre --layout google --centre --suffix .png
  '

echo "==> Done. Generated:"
echo "    fixtures/blueprint_google_centre/"
echo "    fixtures/blueprint_portrait_google_centre/"
