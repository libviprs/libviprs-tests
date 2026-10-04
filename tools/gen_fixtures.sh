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

# ---------------------------------------------------------------------------
# Codec fixtures for tests/codec_e2e.rs (libviprs/libviprs-tests#230)
#
# A separate section with its own oracle, because the dzsave fixtures below
# are pinned to Debian's vips 8.14.1 and the codec matrix is pinned to the
# vips 8.18.4 every CLI-differential family uses. It never runs by default:
# `bash tools/gen_fixtures.sh` with no argument does exactly what it always
# did. The two codec passes run INSIDE the codec oracle image
# (tools/Dockerfile.codec-oracle, which layers on tools/Dockerfile.vips-oracle),
# with a libviprs pass between them:
#
#   docker run --rm --platform linux/amd64 -v "$PWD":/work -w /work \
#       codec-oracle:8.18.4 bash tools/gen_fixtures.sh codec-vips
#   cargo test --features 'jxl libviprs/jp2k' --test codec_e2e -- \
#       --ignored --exact generate_libviprs_encodes
#   docker run --rm --platform linux/amd64 -v "$PWD":/work -w /work \
#       codec-oracle:8.18.4 bash tools/gen_fixtures.sh codec-enc
#
# Pass one writes every format vips (or, where vips has no saver, the writer
# named in the Dockerfile) can write, from the committed canonical only, plus
# vips's own decode of each. The libviprs pass encodes the same source with
# every encoder libviprs has. Pass two is vips decoding those. Every command
# is listed in tests/fixtures/README.md beside the file it made.
# ---------------------------------------------------------------------------

codec_vips() {
    local out=codec tmp
    tmp="$(mktemp -d)"
    mkdir -p "$out"
    echo "vips: $(vips --version)"
    echo "oiiotool: $(oiiotool --version)"
    python3 -c 'import nibabel, scipy, numpy; print("nibabel", nibabel.__version__, "scipy", scipy.__version__, "numpy", numpy.__version__)'

    # Sources. Every one is a re-encoding of canonical_input.png, nothing new.
    vips extract_band canonical_input.png "$out/src_rgb.png" 0 --n 3
    vips colourspace "$out/src_rgb.png" "$out/src_gray.png" b-w
    vips cast "$out/src_rgb.png" "$tmp/f.v" float
    vips linear "$tmp/f.v" "$out/src_float.v" 0.00392156862745098 0
    vips rot "$out/src_rgb.png" "$tmp/p1.v" d90
    vips flip "$out/src_rgb.png" "$tmp/p2.v" horizontal
    vips arrayjoin "$out/src_rgb.png $tmp/p1.v $tmp/p2.v" "$tmp/pages.v" --across 1

    # Lossless rows. The reference is vips's own decode of the file.
    vips pngsave canonical_input.png "$out/png_interlaced.png" --interlace --keep none
    vips tiffsave canonical_input.png "$out/tiff_deflate.tif" --compression deflate --keep none
    vips webpsave canonical_input.png "$out/webp_lossless.webp" --lossless --keep none
    vips gifsave "$out/src_rgb.png" "$out/gif.gif" --keep none
    vips jxlsave canonical_input.png "$out/jxl_lossless.jxl" --lossless --keep none
    vips jp2ksave canonical_input.png "$out/jp2k_lossless.jp2" --lossless --keep none
    vips fitssave "$out/src_rgb.png" "$out/fits.fits"
    vips ppmsave "$out/src_rgb.png" "$out/ppm.ppm" --keep none
    vips copy canonical_input.png "$out/vips.v"
    vips radsave "$out/src_float.v" "$out/rad.hdr"
    oiiotool "$out/src_rgb.png" -d half -o "$out/exr_half.exr"

    # Lossy rows: the stated quality and one step below it, for the control.
    # JPEG at 76 and 75, not 75 and 74: on this smooth source Q75 and Q74 differ
    # only in quantiser entries the image never uses and decode to identical
    # pixels, which no tolerance could tell apart.
    for q in 76 75; do
        vips jpegsave "$out/src_rgb.png" "$out/jpeg_q${q}_444.jpg" --Q "$q" --subsample-mode off --keep none
        vips jpegsave "$out/src_rgb.png" "$out/jpeg_q${q}_420.jpg" --Q "$q" --subsample-mode on --keep none
    done
    for q in 75 74; do
        vips webpsave "$out/src_rgb.png" "$out/webp_q${q}.webp" --Q "$q" --keep none
        vips jxlsave "$out/src_rgb.png" "$out/jxl_q${q}.jxl" --Q "$q" --keep none
    done
    # AVIF at 76 and 75 for a different reason: libheif maps both 74 and 75
    # to the same AV1 quantiser and writes identical files.
    for q in 76 75; do
        vips heifsave "$out/src_rgb.png" "$out/avif_q${q}.avif" --compression av1 --Q "$q" --subsample-mode off --keep none
    done
    for q in 45 44; do
        vips jp2ksave "$out/src_rgb.png" "$out/jp2k_q${q}.jp2" --Q "$q" --keep none
    done

    # Multi-page and multi-frame: three 256x256 pages (the source, rotated,
    # flipped) so a decoder that returns page 0 for every index fails.
    vips tiffsave "$tmp/pages.v" "$out/tiff_pages.tif" --page-height 256 --compression lzw --keep none
    vips gifsave "$tmp/pages.v" "$out/gif_pages.gif" --page-height 256 --keep none
    vips webpsave "$tmp/pages.v" "$out/webp_pages.webp" --page-height 256 --lossless --keep none

    # The formats vips cannot write. nibabel writes NIfTI-1 and Analyze 7.5,
    # scipy writes MATLAB v5; the voxel array is x-fastest (shape W x H) for
    # the two neuro formats, which is the order both store on disk.
    python3 - "$out" <<'PY'
import sys
import nibabel as nib, numpy as np, scipy.io
from PIL import Image
out = sys.argv[1]
gray = np.asarray(Image.open(f"{out}/src_gray.png"))
assert gray.dtype == np.uint8 and gray.ndim == 2, (gray.dtype, gray.shape)
nib.Nifti1Image(np.ascontiguousarray(gray.T), np.eye(4)).to_filename(f"{out}/nifti.nii")
# Big-endian, because vips's analyze2vips only accepts the SPARC byte order
# the format was born with ("header size incorrect" otherwise).
nib.AnalyzeImage(np.ascontiguousarray(gray.T), np.eye(4),
                 header=nib.AnalyzeHeader(endianness=">")).to_filename(f"{out}/analyze.hdr")
scipy.io.savemat(f"{out}/mat.mat", {"im": gray}, format="5", do_compression=False)
# The SVG is the source as vectors: one pixel-aligned 8x8 rect per block,
# filled with the block's top-left pixel. An embedded <image> would be
# simpler and would test nothing, because the core refuses every <image>
# href by design (src/svg.rs). Integer edges and crispEdges leave neither
# rasteriser any antialiasing to disagree about.
rgb = np.asarray(Image.open(f"{out}/src_rgb.png"))
rects = "".join(
    f'<rect x="{x}" y="{y}" width="8" height="8" fill="#{r:02x}{g:02x}{b:02x}"/>'
    for y in range(0, 256, 8) for x in range(0, 256, 8)
    for (r, g, b) in [rgb[y, x]])
with open(f"{out}/svg.svg", "w") as f:
    f.write('<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" '
            f'viewBox="0 0 256 256" shape-rendering="crispEdges">{rects}</svg>\n')
PY

    # vips's own decode of every file above. PNG for the integer formats,
    # .v for the float ones, because PNG cannot hold a float sample.
    local png='[compression=9,filter=all]'
    local f
    for f in png_interlaced.png tiff_deflate.tif webp_lossless.webp gif.gif \
             jxl_lossless.jxl jp2k_lossless.jp2 fits.fits ppm.ppm vips.v \
             jpeg_q76_444.jpg jpeg_q75_444.jpg jpeg_q76_420.jpg jpeg_q75_420.jpg \
             webp_q75.webp webp_q74.webp jxl_q75.jxl jxl_q74.jxl \
             jp2k_q45.jp2 jp2k_q44.jp2 svg.svg mat.mat; do
        vips copy "$out/$f" "$out/${f%.*}_ref.png$png"
    done
    for q in 76 75; do
        vips heifload "$out/avif_q${q}.avif" "$out/avif_q${q}_ref.png$png"
    done
    vips analyzeload "$out/analyze.hdr" "$out/analyze_ref.png$png"
    vips rad2float "$out/rad.hdr" "$out/rad_ref.v"
    vips copy "$out/exr_half.exr" "$out/exr_half_ref.v"
    local p
    for p in 0 1 2; do
        vips tiffload "$out/tiff_pages.tif" "$out/tiff_pages_p${p}_ref.png$png" --page "$p"
        vips gifload "$out/gif_pages.gif" "$out/gif_pages_p${p}_ref.png$png" --page "$p"
        vips webpload "$out/webp_pages.webp" "$out/webp_pages_p${p}_ref.png$png" --page "$p"
    done
    rm -rf "$tmp"
}

codec_enc() {
    local enc=codec/enc png='[compression=9,filter=all]' f
    echo "vips: $(vips --version)"
    for f in "$enc"/libviprs_*; do
        case "$f" in
            *_vips.*) continue ;;
            *.hdr)    vips rad2float "$f" "${f%.*}_vips.v" ;;
            *)        vips copy "$f" "${f%.*}_vips.png$png" ;;
        esac
    done
}

case "${1:-}" in
    codec-vips) codec_vips; exit 0 ;;
    codec-enc)  codec_enc; exit 0 ;;
    "") ;;
    *) echo "usage: $0 [codec-vips|codec-enc]" >&2; exit 2 ;;
esac

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
