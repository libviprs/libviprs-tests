#!/usr/bin/env bash
# Generate all vips reference fixtures for libviprs integration tests.
#
# Every fixture is produced by libvips (vips CLI) so that tests compare
# libviprs output against an independent implementation, not against itself.
#
# Usage:
#   cd libviprs-tests
#   bash tools/gen_fixtures.sh
#
# Prerequisites:
#   - Docker must be running
#   - Source rasters must already exist (extracted_*.png files in tests/fixtures/).
#     Run `cargo test --test gen_source_rasters -- --ignored` first if missing.
#
# The script mounts the fixtures directory into a Debian container with
# libvips-tools installed, then runs vips dzsave for each fixture set.
#
# See tests/fixtures/README.md for the full command reference.

set -euo pipefail

# Pin the vips version so fixture output is reproducible.
# Changing this version requires regenerating all fixtures and updating tests.
VIPS_PACKAGE_VERSION="8.14.1-3+deb12u2"
DOCKER_IMAGE="debian:bookworm-slim"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
FIXTURES_DIR="$SCRIPT_DIR/../tests/fixtures"
cd "$FIXTURES_DIR"

# Check source rasters exist
for f in extracted_blueprint_portrait.png extracted_blueprint_mix.png; do
    if [ ! -f "$f" ]; then
        echo "ERROR: $f not found."
        echo "Run: cargo test --test gen_source_rasters -- --ignored"
        exit 1
    fi
done

# Check rendered rasters exist (generated via pdfium)
for f in rendered_blueprint_portrait.png rendered_blueprint_mix.png; do
    if [ ! -f "$f" ]; then
        echo "ERROR: $f not found."
        echo "Run: cargo test --test gen_source_rasters --features pdfium -- --ignored"
        exit 1
    fi
done

echo "==> Cleaning previous fixtures..."
rm -rf \
  blueprint_portrait_expected blueprint_portrait_expected.dzi \
  blueprint_mix_expected blueprint_mix_expected.dzi \
  blueprint_portrait_google_centre \
  blueprint_mix_google_centre \
  blueprint_mix_rendered_expected blueprint_mix_rendered_expected.dzi \
  blueprint_portrait_rendered_expected blueprint_portrait_rendered_expected.dzi

echo "==> Generating all vips reference fixtures..."

docker run --rm \
  -v "$FIXTURES_DIR":/data \
  -w /data \
  -e VIPS_PKG="$VIPS_PACKAGE_VERSION" \
  "$DOCKER_IMAGE" \
  sh -c '
    set -e
    apt-get update -qq && apt-get install -y -qq libvips-tools="$VIPS_PKG" > /dev/null 2>&1
    echo "vips version: $(vips --version)"

    echo ""
    echo "--- DeepZoom fixtures ---"

    # vips dzsave with --layout dz creates <basename>_files/ and <basename>.dzi.
    # Our tests expect the tiles in <name>/ (no _files suffix), so we rename.

    echo "[1/2] blueprint_portrait_expected (DeepZoom, 3300x5024 Gray8)"
    vips dzsave extracted_blueprint_portrait.png blueprint_portrait_expected \
      --layout dz --tile-size 256 --overlap 0 --suffix .png --strip
    mv blueprint_portrait_expected_files blueprint_portrait_expected

    echo "[2/2] blueprint_mix_expected (DeepZoom, 12738x220 RGB8)"
    vips dzsave extracted_blueprint_mix.png blueprint_mix_expected \
      --layout dz --tile-size 256 --overlap 0 --suffix .png --strip
    mv blueprint_mix_expected_files blueprint_mix_expected

    echo ""
    echo "--- Google Maps + centre fixtures ---"

    echo "[1/2] blueprint_portrait_google_centre (Google+centre, 3300x5024 Gray8)"
    vips dzsave extracted_blueprint_portrait.png blueprint_portrait_google_centre \
      --layout google --tile-size 256 --overlap 0 --centre --suffix .png --strip

    echo "[2/2] blueprint_mix_google_centre (Google+centre, 12738x220 RGB8)"
    vips dzsave extracted_blueprint_mix.png blueprint_mix_google_centre \
      --layout google --tile-size 256 --overlap 0 --centre --suffix .png --strip

    echo ""
    echo "--- PDFium-rendered DeepZoom fixtures ---"

    echo "[1/2] blueprint_mix_rendered_expected (DeepZoom, rendered 4768x3370 RGBA8)"
    vips dzsave rendered_blueprint_mix.png blueprint_mix_rendered_expected \
      --layout dz --tile-size 256 --overlap 0 --suffix .png --strip
    mv blueprint_mix_rendered_expected_files blueprint_mix_rendered_expected

    echo "[2/2] blueprint_portrait_rendered_expected (DeepZoom, rendered 792x1224 RGBA8)"
    vips dzsave rendered_blueprint_portrait.png blueprint_portrait_rendered_expected \
      --layout dz --tile-size 256 --overlap 0 --suffix .png --strip
    mv blueprint_portrait_rendered_expected_files blueprint_portrait_rendered_expected

    echo ""
    echo "--- Done ---"
  '

echo "==> All vips fixtures generated."
