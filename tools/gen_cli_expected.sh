#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# gen_cli_expected.sh — OFFLINE generator for the CLI-differential references.
#
# AUTHOR-RUN ONLY, on a machine with the pinned vips oracle installed. NEVER
# run in CI: CI has no libvips (CLI_CONTRACT.md §7). This script produces the
# committed input fixture(s), the morphological mask, and the vips 8.18.4
# reference outputs the morphology differential cell
# (tests/cli_morphology_diff.rs) decode-compares against. It also writes
# tests/fixtures/cli/PROVENANCE.md recording the exact vips version and the
# exact command behind every reference.
#
# The SAME committed input.png is consumed by BOTH this generator (to make the
# references) and the harness (which feeds it to `viprs`), so the two sides
# compare like against like — the test never runs vips.
#
# Usage (from the repo root):
#   ./tools/gen_cli_expected.sh
#
# Override the oracle with VIPS=/path/to/vips (default: /opt/homebrew/bin/vips).
# ---------------------------------------------------------------------------

VIPS="${VIPS:-/opt/homebrew/bin/vips}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# The sibling `viprs` CLI, used ONLY to (re)generate the single GOLDEN-ONLY
# extract reference (`smartcrop --interesting entropy`), which has no vips oracle
# (vips's smartcrop-entropy picks a different discrete crop window on the small
# committed input, so there is nothing to cross-check against — the reference is
# a libviprs-generated regression pin). Override with VIPRS=/path/to/viprs.
CLI_DIR="${VIPRS_CLI_DIR:-$REPO_ROOT/../libviprs-cli}"
VIPRS="${VIPRS:-$CLI_DIR/target/release/viprs}"

FIX_ROOT="$REPO_ROOT/tests/fixtures/cli"
FIX="$FIX_ROOT/morphology"
mkdir -p "$FIX"

if ! command -v "$VIPS" >/dev/null 2>&1; then
    echo "ERROR: vips oracle not found at '$VIPS'. Install libvips or set VIPS=." >&2
    exit 1
fi

VIPS_VERSION="$("$VIPS" --version)"
# The SIMD target set of the oracle build. Recorded because whether libvips's
# vector integer-convolution path runs at all is a build-and-CPU property, not a
# version property (issue #558). It is NOT recorded because the target set
# changes the arithmetic — it does not: dropping NEON_BF16 and leaving NEON
# gives byte-identical output at nine sigmas, while the same comparison against
# VIPS_NOVECTOR=1 diverges at eight of the nine.
VIPS_TARGETS="$("$VIPS" --targets 2>/dev/null | sed 's/  */ /g' | paste -sd';' - | sed 's/;/; /g')"
echo "==> Using oracle: $VIPS_VERSION"
echo "==> Oracle targets: $VIPS_TARGETS"

# ---------------------------------------------------------------------------
# The portable-C arm of libvips's integer convolution (issue #558).
#
# On UCHAR images `vips_convi` has TWO implementations that disagree, and
# libvips does not bound the disagreement:
#
#   * `vips_convi_gen`, the portable C loop. libvips's own documentation names
#     it as the specification — "@mask is converted to an integer mask with
#     rint() of each element ... For UCHAR images, vips_convi uses a fast vector
#     path based on half-float arithmetic. THIS CAN PRODUCE SLIGHTLY DIFFERENT
#     RESULTS. Disable the vector path with --vips-novector or VIPS_NOVECTOR or
#     vips_vector_set_enabled()" (convi.c:1271-1284) — and it is also the path
#     libvips falls back to whenever `vips_convi_intize` declines a mask.
#   * a HWY fixed-point vector path, which convolves with REQUANTISED
#     coefficients. A 3x3 box blur, scale 9, on a window summing to 1147: the C
#     path gives (1147 + 4) / 9 = 127, and so does floor, while the vector path
#     gives (57 * 1147 + 256) >> 9 = 128, because it is filtering with
#     57/512 = 0.111328 rather than 1/9.
#
# libviprs implements the documented C one, so every uchar integer-convolution
# reference below is generated through `novector_vips`. That makes these
# references EXACT rather than tolerance-banded, and it makes the claim
# checkable: `VIPS_NOVECTOR=1 vips` reproduces libviprs byte for byte.
#
# Do NOT "fix" a failure here by widening a tolerance. A tolerance is what hid
# this for two waves.
# ---------------------------------------------------------------------------
novector_vips() { VIPS_NOVECTOR=1 "$VIPS" "$@"; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ---------------------------------------------------------------------------
# 1. Common input — a deterministic 64x64 single-band binary (Gray8, 0/255)
#    image where morphology is meaningful. `vips eye` is a pure function of
#    pixel coordinates (a zone-plate grating), so the input is bit-for-bit
#    reproducible; thresholding at 0 binarises it to 0/255.
# ---------------------------------------------------------------------------
echo "==> [input] 64x64 binary Gray8 (vips eye -> threshold)"
"$VIPS" eye "$TMP/eye.v" 64 64
"$VIPS" relational_const "$TMP/eye.v" "$FIX/input.png" more 0.0

# ---------------------------------------------------------------------------
# 2. Structuring element — a 3x3 cross in the vips text-matrix format
#    (values 0 / 128 / 255 = must-be-zero / don't-care / must-be-set). Read by
#    BOTH `vips morph` and `viprs morph` (CLI_CONTRACT.md §3 matrix file arg).
# ---------------------------------------------------------------------------
echo "==> [mask] 3x3 cross structuring element (cross.mat)"
printf '3 3\n128 255 128\n255 255 255\n128 255 128\n' > "$FIX/cross.mat"

# A second structuring element that exercises ALL THREE trit values — the
# must-be-ZERO level 0, which cross.mat lacks (it uses only 128/255). Two 0
# corners, two 128 corners, 255 elsewhere, so erode/dilate must honour a
# must-be-zero constraint (F7 trit coverage).
echo "==> [mask] 3x3 corner structuring element with a 0 cell (corner.mat)"
printf '3 3\n0 255 128\n255 255 255\n128 255 0\n' > "$FIX/corner.mat"

# ---------------------------------------------------------------------------
# 2b. Multi-level input — a 16x16 Gray8 horizontal ramp (0,17,…,255: sixteen
#     distinct gray levels). `rank` on the binary input.png only ever picks
#     between two values; a multi-level input pins the true order-statistic
#     semantics (median vs a non-median index) (F8 coverage). `vips grey` is a
#     pure function of coordinates, scaled to the 0..255 uchar range.
# ---------------------------------------------------------------------------
echo "==> [input] 16x16 multi-level Gray8 ramp (input_gray.png)"
"$VIPS" grey "$TMP/grey.v" 16 16
"$VIPS" linear "$TMP/grey.v" "$FIX/input_gray.png" 255 0 --uchar

# ---------------------------------------------------------------------------
# 3. References — one vips run per differential case.
# ---------------------------------------------------------------------------
echo "==> [morph]  erode + dilate, cross mask (EXACT, Gray8 PNG)"
"$VIPS" morph "$FIX/input.png" "$FIX/morph_erode_expected.png"  "$FIX/cross.mat" erode
"$VIPS" morph "$FIX/input.png" "$FIX/morph_dilate_expected.png" "$FIX/cross.mat" dilate

echo "==> [morph]  erode + dilate, corner mask w/ 0 cell (EXACT, Gray8 PNG)"
"$VIPS" morph "$FIX/input.png" "$FIX/morph_erode_corner_expected.png"  "$FIX/corner.mat" erode
"$VIPS" morph "$FIX/input.png" "$FIX/morph_dilate_corner_expected.png" "$FIX/corner.mat" dilate

echo "==> [rank]   median 3x3 (index 4) on binary input (EXACT, Gray8 PNG)"
"$VIPS" rank "$FIX/input.png" "$FIX/rank_median_expected.png" 3 3 4

echo "==> [rank]   multi-level: median (index 4) + max (index 8) (EXACT, Gray8 PNG)"
"$VIPS" rank "$FIX/input_gray.png" "$FIX/rank_gray_median_expected.png" 3 3 4
"$VIPS" rank "$FIX/input_gray.png" "$FIX/rank_gray_max_expected.png"    3 3 8

echo "==> [countlines] horizontal + vertical (EXACT, S3 scalar)"
"$VIPS" countlines "$FIX/input.png" horizontal > "$FIX/countlines_horizontal_expected.txt"
"$VIPS" countlines "$FIX/input.png" vertical   > "$FIX/countlines_vertical_expected.txt"

echo "==> [labelregions] label mask + segment count (EXACT, S4)"
# vips labelregions emits an INT-band mask (band format 5), which neither PNG
# nor the libviprs decoder round-trips (a ushort PNG comes out all-zero). Cast
# to ushort and carry the reference as a 16-bit TIFF, which both vips and the
# libviprs decoder handle losslessly. The `--segments` output arg is printed to
# stdout.
"$VIPS" labelregions "$FIX/input.png" "$TMP/lab_int.v" --segments \
    > "$FIX/labelregions_segments_expected.txt"
"$VIPS" cast "$TMP/lab_int.v" "$FIX/labelregions_mask_expected.tif" ushort

# ---------------------------------------------------------------------------
# 4. Provenance — vips version + the exact command behind every reference.
# ---------------------------------------------------------------------------
echo "==> [provenance] $FIX_ROOT/PROVENANCE.md"
cat > "$FIX_ROOT/PROVENANCE.md" <<EOF
# CLI-differential reference provenance

These fixtures are the **committed vips oracle references** the CLI-differential
suite (\`tests/cli_morphology_diff.rs\`) decode-compares \`viprs\` output against.
They are generated **offline** by \`tools/gen_cli_expected.sh\` and NEVER by CI
(CLI_CONTRACT.md §7): CI has no libvips and asserts fixture PRESENCE only.

- **Oracle**: \`$VIPS_VERSION\`
- **Generated by**: \`tools/gen_cli_expected.sh\`
- **Common input**: \`morphology/input.png\` — 64×64 single-band Gray8 binary
  (0/255), consumed by both the generator and the harness.
- **Multi-level input**: \`morphology/input_gray.png\` — 16×16 single-band Gray8
  horizontal ramp (0,17,…,255; sixteen distinct levels), for the \`rank\`
  order-statistic cases a 2-level input cannot pin.
- **Mask**: \`morphology/cross.mat\` — 3×3 cross (vips text matrix, 128/255 only).
- **Mask**: \`morphology/corner.mat\` — 3×3 with a must-be-zero \`0\` cell, so
  erode/dilate exercise all three trit values 0/128/255.

## Exact commands

Inputs + masks:

\`\`\`
vips eye eye.v 64 64
vips relational_const eye.v morphology/input.png more 0.0
vips grey grey.v 16 16
vips linear grey.v morphology/input_gray.png 255 0 --uchar
printf '3 3\\n128 255 128\\n255 255 255\\n128 255 128\\n' > morphology/cross.mat
printf '3 3\\n0 255 128\\n255 255 255\\n128 255 0\\n'     > morphology/corner.mat
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | vips command |
|---|---|---|
| \`morphology/morph_erode_expected.png\` | EXACT | \`vips morph input.png morph_erode_expected.png cross.mat erode\` |
| \`morphology/morph_dilate_expected.png\` | EXACT | \`vips morph input.png morph_dilate_expected.png cross.mat dilate\` |
| \`morphology/morph_erode_corner_expected.png\` | EXACT | \`vips morph input.png morph_erode_corner_expected.png corner.mat erode\` |
| \`morphology/morph_dilate_corner_expected.png\` | EXACT | \`vips morph input.png morph_dilate_corner_expected.png corner.mat dilate\` |
| \`morphology/rank_median_expected.png\` | EXACT | \`vips rank input.png rank_median_expected.png 3 3 4\` |
| \`morphology/rank_gray_median_expected.png\` | EXACT | \`vips rank input_gray.png rank_gray_median_expected.png 3 3 4\` |
| \`morphology/rank_gray_max_expected.png\` | EXACT | \`vips rank input_gray.png rank_gray_max_expected.png 3 3 8\` |
| \`morphology/countlines_horizontal_expected.txt\` | EXACT (S3, float mean) | \`vips countlines input.png horizontal\` |
| \`morphology/countlines_vertical_expected.txt\` | EXACT (S3, float mean) | \`vips countlines input.png vertical\` |
| \`morphology/labelregions_mask_expected.tif\` | EXACT (S4) | \`vips labelregions input.png lab_int.v --segments\` then \`vips cast lab_int.v labelregions_mask_expected.tif ushort\` |
| \`morphology/labelregions_segments_expected.txt\` | EXACT (S4, integer) | \`vips labelregions input.png lab_int.v --segments\` (stdout) |

The label mask is carried as a 16-bit TIFF because vips emits an INT-band mask
that does not round-trip through PNG or the libviprs \`.v\` decoder; the ushort
cast is lossless for the label range (region ids ≤ segment count).
EOF

# ===========================================================================
# BANDS FAMILY (the first per-family Wave-2 lane; OP_MAP.md bands section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_bands_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. Every bands op is oracle class EXACT
# (integer-in / integer-out, decode-compare tol 0). All outputs are chosen to
# land on a 1/3/4-band uchar PNG so the committed reference round-trips through
# the libviprs decoder losslessly (2-band and ≥5-band carriers would need `.v`,
# which vips writes in its own container the libviprs decoder is not pinned to).
# ===========================================================================
BANDS="$FIX_ROOT/bands"
mkdir -p "$BANDS"

# --- Common inputs -----------------------------------------------------------
# Three DISTINCT single-band 16x16 Gray8 images (a horizontal ramp, a scaled +
# offset horizontal ramp, and a vertical ramp), then an RGB built by joining
# them (re-tagged sRGB so a 3-band PNG saves as clean RGB, NOT a b-w image that
# vips pngsave would alpha-pad to 4 bands) and an RGBA (rgb + the first gray).
# `vips grey` is a pure coordinate function (0..1 float ramp), so every fixture
# is bit-reproducible.
echo "==> [bands input] three 16x16 Gray8 sources + rgb (srgb) + rgba (srgb)"
"$VIPS" grey "$TMP/bgrey.v" 16 16
"$VIPS" linear "$TMP/bgrey.v" "$BANDS/gray.png"  255 0  --uchar
"$VIPS" linear "$TMP/bgrey.v" "$BANDS/gray2.png" 200 10 --uchar
"$VIPS" rot "$TMP/bgrey.v" "$TMP/bgrey_v.v" d90
"$VIPS" linear "$TMP/bgrey_v.v" "$BANDS/gray3.png" 255 0 --uchar
"$VIPS" bandjoin "$BANDS/gray.png $BANDS/gray2.png $BANDS/gray3.png" "$TMP/rgb.v"
"$VIPS" copy "$TMP/rgb.v" "$BANDS/rgb.png" --interpretation srgb
"$VIPS" bandjoin "$BANDS/rgb.png $BANDS/gray.png" "$TMP/rgba.v"
"$VIPS" copy "$TMP/rgba.v" "$BANDS/rgba.png" --interpretation srgb

# --- References — one vips run per differential case -------------------------
# Carrier choice: 1-band and sRGB-tagged 3/4-band outputs go to PNG (which the
# libviprs decoder round-trips). bandfold and bandjoin_const produce a b-w
# multiband result — vips's PNG encoder colour-PROMOTES that (gray→RGB, dropping
# bands) and the libviprs TIFF decoder rejects a 4-band multiband TIFF — so they
# are carried as the native `.v` container, which preserves the raw bands and
# which the libviprs decoder reads back losslessly (CLI_CONTRACT.md §2 N-band).
echo "==> [bandjoin] rgb + gray -> 4-band rgba PNG (S2 variadic, 2 inputs)"
"$VIPS" bandjoin "$BANDS/rgb.png $BANDS/gray.png" "$BANDS/bandjoin_expected.png"

# A THREE-input bandjoin so the run_bandjoin accumulation loop runs MORE THAN
# ONE iteration — the true >=3 variadic fold (the S2 template) the 2-input case
# above cannot exercise (B3). Joining three DISTINCT single-band grays yields a
# 3-band b-w multiband result; carried as native `.v` (vips's PNG encoder would
# colour-promote a b-w multiband image), which the libviprs decoder round-trips.
echo "==> [bandjoin] 3 distinct grays -> 3-band b-w .v (>=3 variadic fold)"
"$VIPS" bandjoin "$BANDS/gray.png $BANDS/gray2.png $BANDS/gray3.png" \
    "$BANDS/bandjoin3_expected.v"

echo "==> [bandjoin_const] gray + \"10 20 30\" -> 4-band b-w .v (multi-element vector)"
"$VIPS" bandjoin_const "$BANDS/gray.png" "$BANDS/bandjoin_const_expected.v" "10 20 30"

echo "==> [bandfold] gray --factor 4 -> 4x16 4-band b-w .v"
"$VIPS" bandfold "$BANDS/gray.png" "$BANDS/bandfold_expected.v" --factor 4

echo "==> [bandunfold] rgb (default factor = unfold all) -> 48x16 1-band PNG"
"$VIPS" bandunfold "$BANDS/rgb.png" "$BANDS/bandunfold_expected.png"

# EXACT (tol 0): rgb.png is a bandjoin of three DISTINCT grays, so a per-pixel
# band sum is generally NOT divisible by 3 — an honest, non-vacuous case (the old
# rgb_eq.png had three identical bands, making mean == input: divisible AND
# arithmetically vacuous). Core issue #482 made the core round the integer mean to
# nearest (previously it FLOORED via truncating division, ≤1 LSB below vips), so
# the two now match bit-for-bit. The bands cell decode-compares this at tol 0.
echo "==> [bandmean] rgb (3 distinct bands) -> 1-band mean PNG (EXACT)"
"$VIPS" bandmean "$BANDS/rgb.png" "$BANDS/bandmean_expected.png"

echo "==> [bandrank] 3 grays: median (default) + min (--index 0)"
"$VIPS" bandrank "$BANDS/gray.png $BANDS/gray2.png $BANDS/gray3.png" \
    "$BANDS/bandrank_median_expected.png"
"$VIPS" bandrank "$BANDS/gray.png $BANDS/gray2.png $BANDS/gray3.png" \
    "$BANDS/bandrank_min_expected.png" --index 0

echo "==> [bandbool] rgb -> 1-band and | or | eor"
"$VIPS" bandbool "$BANDS/rgb.png" "$BANDS/bandbool_and_expected.png" and
"$VIPS" bandbool "$BANDS/rgb.png" "$BANDS/bandbool_or_expected.png"  or
"$VIPS" bandbool "$BANDS/rgb.png" "$BANDS/bandbool_eor_expected.png" eor

echo "==> [extract_band] rgb band 1 -> 1-band; rgba band 1 --n 3 -> 3-band"
"$VIPS" extract_band "$BANDS/rgb.png"  "$BANDS/extract_band1_expected.png" 1
"$VIPS" extract_band "$BANDS/rgba.png" "$BANDS/extract_bandn_expected.png" 1 --n 3

# --- Provenance (append the bands section) -----------------------------------
echo "==> [provenance] appending bands section to $FIX_ROOT/PROVENANCE.md"
cat >> "$FIX_ROOT/PROVENANCE.md" <<EOF

---

# bands family CLI-differential reference provenance

These fixtures are the committed vips oracle references the bands
CLI-differential suite (\`tests/cli_bands_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`bands/\`): \`gray.png\`, \`gray2.png\`, \`gray3.png\`
  (three distinct 16×16 Gray8 sources), \`rgb.png\` (their bandjoin re-tagged
  sRGB, 3-band — also the DISTINCT-band, non-divisible input for the honest
  \`bandmean\` BOUNDED-TOL case), \`rgba.png\` (\`rgb\` + \`gray\`, sRGB, 4-band).
- **Carriers**: 1-band and sRGB 3/4-band outputs → PNG. \`bandfold\`,
  \`bandjoin_const\`, and the ≥3-input \`bandjoin3\` produce a b-w multiband
  result vips's PNG encoder would colour-promote (and the libviprs TIFF decoder
  rejects at 4 bands), so they are carried as the native \`.v\` container (raw
  bands, libviprs-decodable).

## Exact commands

Inputs:

\`\`\`
vips grey bgrey.v 16 16
vips linear bgrey.v bands/gray.png  255 0  --uchar
vips linear bgrey.v bands/gray2.png 200 10 --uchar
vips rot bgrey.v bgrey_v.v d90
vips linear bgrey_v.v bands/gray3.png 255 0 --uchar
vips bandjoin "bands/gray.png bands/gray2.png bands/gray3.png" rgb.v
vips copy rgb.v bands/rgb.png --interpretation srgb
vips bandjoin "bands/rgb.png bands/gray.png" rgba.v
vips copy rgba.v bands/rgba.png --interpretation srgb
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | vips command |
|---|---|---|
| \`bands/bandjoin_expected.png\` | EXACT | \`vips bandjoin "rgb.png gray.png" bandjoin_expected.png\` (2 inputs) |
| \`bands/bandjoin3_expected.v\` | EXACT | \`vips bandjoin "gray.png gray2.png gray3.png" bandjoin3_expected.v\` (≥3 inputs → multi-iteration fold) |
| \`bands/bandjoin_const_expected.v\` | EXACT | \`vips bandjoin_const gray.png bandjoin_const_expected.v "10 20 30"\` |
| \`bands/bandfold_expected.v\` | EXACT | \`vips bandfold gray.png bandfold_expected.v --factor 4\` |
| \`bands/bandunfold_expected.png\` | EXACT | \`vips bandunfold rgb.png bandunfold_expected.png\` (default factor = unfold all) |
| \`bands/bandmean_expected.png\` | EXACT (tol 0) | \`vips bandmean rgb.png bandmean_expected.png\` (3 distinct bands; core #482 now rounds the integer mean to nearest, matching vips bit-for-bit) |
| \`bands/bandrank_median_expected.png\` | EXACT | \`vips bandrank "gray.png gray2.png gray3.png" bandrank_median_expected.png\` |
| \`bands/bandrank_min_expected.png\` | EXACT | \`vips bandrank "gray.png gray2.png gray3.png" bandrank_min_expected.png --index 0\` |
| \`bands/bandbool_and_expected.png\` | EXACT | \`vips bandbool rgb.png bandbool_and_expected.png and\` |
| \`bands/bandbool_or_expected.png\` | EXACT | \`vips bandbool rgb.png bandbool_or_expected.png or\` |
| \`bands/bandbool_eor_expected.png\` | EXACT | \`vips bandbool rgb.png bandbool_eor_expected.png eor\` |
| \`bands/extract_band1_expected.png\` | EXACT | \`vips extract_band rgb.png extract_band1_expected.png 1\` |
| \`bands/extract_bandn_expected.png\` | EXACT | \`vips extract_band rgba.png extract_bandn_expected.png 1 --n 3\` |
EOF

# ===========================================================================
# EXTRACT FAMILY (per-family Wave-2 lane; OP_MAP.md extract section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_extract_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. Every extract op is oracle class EXACT
# (integer-in / integer-out, decode-compare tol 0) EXCEPT `smartcrop
# --interesting entropy`, which is GOLDEN-ONLY: vips's entropy strategy makes a
# DIFFERENT discrete crop-window choice than the core on the small committed
# input (measured max-abs-diff 136 — a wholesale different region, not a
# tolerance), so there is no meaningful cross-oracle. Its reference is generated
# by `viprs` itself (deterministic across runs) and the test is a regression pin.
#
# Carriers: every output is an integer uchar raster with a clean interpretation
# (1-band, or sRGB-tagged 3-band), so all references round-trip losslessly
# through PNG (no b-w-multiband promotion, no ≥5-band carrier needed).
# ===========================================================================
EXTRACT="$FIX_ROOT/extract"
mkdir -p "$EXTRACT"

# --- Common inputs -----------------------------------------------------------
# A 16x16 single-band Gray8 horizontal ramp, a 16x16 3-band sRGB image with TRUE
# 2-D structure (band0 = horizontal ramp, band1 = a scaled horizontal ramp,
# band2 = a VERTICAL ramp, so a crop/placement is fully determined in both axes
# and a wrong offset fails loudly), and a small 6x6 solid sRGB sub-image whose
# distinct colour stands out wherever `insert` places it, plus a 6x6 SINGLE-band
# Gray8 ramp (`sub1.png`) that pins vips's 1-band-sub → multi-band-main bandalike
# broadcast on `insert`. `vips grey`/`rot` are pure coordinate functions, so
# every fixture is bit-reproducible.
echo "==> [extract input] 16x16 gray ramp + 16x16 sRGB (2-D) rgb + 6x6 sub (3-band + 1-band)"
"$VIPS" grey "$TMP/egrey.v" 16 16
"$VIPS" linear "$TMP/egrey.v" "$EXTRACT/gray.png"  255 0 --uchar
"$VIPS" linear "$TMP/egrey.v" "$TMP/egray2.png" 200 10 --uchar
"$VIPS" rot "$TMP/egrey.v" "$TMP/egrey_v.v" d90
"$VIPS" linear "$TMP/egrey_v.v" "$TMP/egray3.png" 255 0 --uchar
"$VIPS" bandjoin "$EXTRACT/gray.png $TMP/egray2.png $TMP/egray3.png" "$TMP/ergb.v"
"$VIPS" copy "$TMP/ergb.v" "$EXTRACT/rgb.png" --interpretation srgb
"$VIPS" black "$TMP/eb.v" 6 6 --bands 3
"$VIPS" linear "$TMP/eb.v" "$TMP/esub.v" "0 0 0" "200 50 100" --uchar
"$VIPS" copy "$TMP/esub.v" "$EXTRACT/sub.png" --interpretation srgb
"$VIPS" grey "$TMP/esub1.v" 6 6
"$VIPS" linear "$TMP/esub1.v" "$EXTRACT/sub1.png" 255 0 --uchar

# --- References — one vips run per differential case -------------------------
echo "==> [extract_area/crop] interior rectangle (S1 EXACT)"
"$VIPS" extract_area "$EXTRACT/rgb.png" "$EXTRACT/extract_area_expected.png" 3 4 5 6
"$VIPS" crop         "$EXTRACT/rgb.png" "$EXTRACT/crop_expected.png"         3 4 5 6

echo "==> [embed] black + copy/repeat/mirror/white (enum) + background vector (S1 EXACT)"
"$VIPS" embed "$EXTRACT/rgb.png"  "$EXTRACT/embed_black_expected.png" 2 3 24 24
"$VIPS" embed "$EXTRACT/rgb.png"  "$EXTRACT/embed_copy_expected.png"  2 3 24 24 --extend copy
"$VIPS" embed "$EXTRACT/rgb.png"  "$EXTRACT/embed_repeat_expected.png" 2 3 24 24 --extend repeat
"$VIPS" embed "$EXTRACT/rgb.png"  "$EXTRACT/embed_mirror_expected.png" 2 3 24 24 --extend mirror
"$VIPS" embed "$EXTRACT/rgb.png"  "$EXTRACT/embed_white_expected.png"  2 3 24 24 --extend white
"$VIPS" embed "$EXTRACT/gray.png" "$EXTRACT/embed_bg_expected.png"    1 1 8 8 --extend background --background 128

echo "==> [gravity] centre + south-east + north-west (dash-spelled enum) (S1 EXACT)"
"$VIPS" gravity "$EXTRACT/rgb.png" "$EXTRACT/gravity_centre_expected.png" centre     24 24
"$VIPS" gravity "$EXTRACT/rgb.png" "$EXTRACT/gravity_se_expected.png"     south-east 24 24
"$VIPS" gravity "$EXTRACT/rgb.png" "$EXTRACT/gravity_nw_expected.png"     north-west 24 24

echo "==> [replicate/zoom/subsample] integer geometry (S1 EXACT)"
"$VIPS" replicate "$EXTRACT/rgb.png"  "$EXTRACT/replicate_expected.png" 2 3
"$VIPS" zoom      "$EXTRACT/gray.png" "$EXTRACT/zoom_expected.png"      3 2
"$VIPS" subsample "$EXTRACT/rgb.png"  "$EXTRACT/subsample_expected.png" 2 2

echo "==> [insert] non-expand + expand (canvas grows) + 1-band-sub bandalike broadcast (S2 EXACT)"
"$VIPS" insert "$EXTRACT/rgb.png" "$EXTRACT/sub.png" "$EXTRACT/insert_expected.png"        4 5
"$VIPS" insert "$EXTRACT/rgb.png" "$EXTRACT/sub.png" "$EXTRACT/insert_expand_expected.png" 13 13 --expand
"$VIPS" insert "$EXTRACT/rgb.png" "$EXTRACT/sub1.png" "$EXTRACT/insert_bandalike_expected.png" 4 5

echo "==> [smartcrop] centre/low/high/attention/all (S1 EXACT — discriminating geometry + saliency)"
"$VIPS" smartcrop "$EXTRACT/rgb.png" "$EXTRACT/smartcrop_centre_expected.png"    8 8 --interesting centre
"$VIPS" smartcrop "$EXTRACT/rgb.png" "$EXTRACT/smartcrop_all_expected.png"       8 8 --interesting all
"$VIPS" smartcrop "$EXTRACT/rgb.png" "$EXTRACT/smartcrop_low_expected.png"       8 8 --interesting low
"$VIPS" smartcrop "$EXTRACT/rgb.png" "$EXTRACT/smartcrop_high_expected.png"      8 8 --interesting high
"$VIPS" smartcrop "$EXTRACT/rgb.png" "$EXTRACT/smartcrop_attention_expected.png" 8 8 --interesting attention

# GOLDEN-ONLY (no vips oracle): vips's smartcrop-entropy chooses a DIFFERENT
# discrete crop window than the core on this input, so the reference is generated
# by viprs itself (deterministic) as a regression pin. Build the CLI if absent.
echo "==> [smartcrop] entropy (GOLDEN-ONLY viprs pin — no vips oracle)"
if [ ! -x "$VIPRS" ]; then
    echo "    (building $VIPRS: cargo build --release --no-default-features --bin viprs)"
    ( cd "$CLI_DIR" && cargo build --release --no-default-features --bin viprs )
fi
"$VIPRS" smartcrop "$EXTRACT/rgb.png" "$EXTRACT/smartcrop_entropy_golden.png" 8 8 --interesting entropy >/dev/null

# --- Provenance (append the extract section) ---------------------------------
echo "==> [provenance] appending extract section to $FIX_ROOT/PROVENANCE.md"
# CONVERSION FAMILY (the Wave-2 conversion lane; OP_MAP.md conversion section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_conversion_diff.rs (which feeds them to `viprs`).
# Most commands are oracle class EXACT (integer-in / integer-out, decode-compare
# tol 0); `flatten` is BOUNDED-TOL (≤1 LSB alpha-blend rounding) and `grey` is
# BOUNDED-TOL (float ramp `.v` eps 1e-6 / uchar ≤1 LSB). Carriers: 1/3/4-band
# uchar outputs → PNG; float (`cast`→float, `grey` float ramp) and 16-bit
# (`byteswap` source `nb16`, `msb` source `mb16`) inputs whose byte order / raw
# bands PNG cannot carry → the native `.v` container (CLI_CONTRACT.md §2).
#
# Inputs are chosen to make every differential DISCRIMINATING (adversarial-review
# conversion findings 1/2/3/5): `grad` is a 2-D gradient (flip-vertical, wrap,
# rot-d180 are not no-ops); `nb16` has differing high/low bytes (byteswap moves
# data); `mb16` is 3-band with band0 != band1 (msb --band actually selects); and
# `ramp256` spans the full 0..255 domain (gamma/falsecolour LUTs fully covered).
#
# `autorot` is the exception (see its block): libviprs' decoders read neither
# the vips `.v` XML `orientation` field nor the TIFF Orientation tag (274) that
# vips sets, so a vips-oriented input is a no-op under `viprs` — the two
# orientation metadata channels are mutually unreadable (the globalbalance
# situation). Its oriented input is therefore produced by `viprs` itself
# (`$VIPRS copy … --orientation 6`), and the vips reference exploits the
# identity autorot(orientation=6) == rot(d90), giving a genuine vips oracle.
# ===========================================================================
CONV="$FIX_ROOT/conversion"
mkdir -p "$CONV"

# Optional path to a pre-built `viprs` binary, used ONLY to mint the autorot
# oriented `.v` input (which vips cannot write in a libviprs-readable way). If
# unset the script tries the release binary in the sibling CLI checkout.
VIPRS="${VIPRS:-${CLI_DIR:-$REPO_ROOT/../libviprs-cli}/target/release/viprs}"

# --- Common inputs -----------------------------------------------------------
# `vips grey` is a pure coordinate function (0..1 float ramp), so every fixture
# below is bit-for-bit reproducible.
echo "==> [conversion input] gray / gray2 / gray3 (16x16 Gray8), rgb, rgba"
"$VIPS" grey "$TMP/cg.v" 16 16
"$VIPS" linear "$TMP/cg.v" "$CONV/gray.png"  255 0  --uchar
"$VIPS" linear "$TMP/cg.v" "$CONV/gray2.png" 200 10 --uchar
"$VIPS" rot "$TMP/cg.v" "$TMP/cg_v.v" d90
"$VIPS" linear "$TMP/cg_v.v" "$CONV/gray3.png" 255 0 --uchar
"$VIPS" bandjoin "$CONV/gray.png $CONV/gray2.png $CONV/gray3.png" "$TMP/crgb.v"
"$VIPS" copy "$TMP/crgb.v" "$CONV/rgb.png" --interpretation srgb
"$VIPS" bandjoin "$CONV/rgb.png $CONV/gray.png" "$TMP/crgba.v"
"$VIPS" copy "$TMP/crgba.v" "$CONV/rgba.png" --interpretation srgb

# grad: a 2-D gradient varying in BOTH axes (x*85 + y*170), so vertical flip, the
# wrap h/2 shift and rot d180 are NON-VACUOUS. gray.png is a horizontal ramp
# (constant in Y), which made flip-vertical / wrap-vertical no-ops and collapsed
# rot d180 to a horizontal flip (adversarial-review conversion finding 2). Max
# sample 85+170 = 255, so it fills uchar without clipping.
echo "==> [conversion input] grad (16x16 2-D gradient, both axes)"
"$VIPS" rot "$TMP/cg.v" "$TMP/cgrot.v" d90
"$VIPS" linear "$TMP/cg.v"    "$TMP/cgx.v" 85  0
"$VIPS" linear "$TMP/cgrot.v" "$TMP/cgy.v" 170 0
"$VIPS" add "$TMP/cgx.v" "$TMP/cgy.v" "$TMP/cgsum.v"
"$VIPS" cast "$TMP/cgsum.v" "$CONV/grad.png" uchar

# nb16: a 16-bit input whose high and low bytes DIFFER (multiples of 0x1000:
# 0x0000,0x1000,…,0xF000 — low byte 0, high byte varies), so byteswap actually
# moves data. A gray16 ramp (multiples of 0x1111) is all byte-palindromes, so a
# no-op byteswap passed GREEN (adversarial-review conversion finding 1). Carried
# as `.v` (a raw 16-bit container PNG bit-depth minimisation cannot distort).
echo "==> [conversion input] nb16 (16x16 Gray16, non-palindromic bytes; for byteswap)"
"$VIPS" linear "$TMP/cg.v" "$TMP/cnb16f.v" 61440 0
"$VIPS" cast "$TMP/cnb16f.v" "$CONV/nb16.v" ushort

# mb16: a 3-band 16-bit input where band 0 != band 1 (distinct per-band scales),
# so `msb --band 0` (→ 1 band, band-0 high byte) is provably distinct from `msb`
# default (→ 3 bands). A 1-band gray16 made the --band selection path vacuous
# (adversarial-review conversion finding 3). Carried as `.v` (multiband 16-bit).
echo "==> [conversion input] mb16 (16x16 3-band Gray16, distinct bands; for msb --band)"
"$VIPS" linear "$TMP/cg.v"    "$TMP/cm16a.v" 65535 0
"$VIPS" linear "$TMP/cg.v"    "$TMP/cm16b.v" 40000 0
"$VIPS" linear "$TMP/cgrot.v" "$TMP/cm16c.v" 50000 0
"$VIPS" bandjoin "$TMP/cm16a.v $TMP/cm16b.v $TMP/cm16c.v" "$TMP/cmb16f.v"
"$VIPS" cast "$TMP/cmb16f.v" "$CONV/mb16.v" ushort

echo "==> [conversion input] odd (15x15 Gray8, for rot45)"
"$VIPS" grey "$TMP/codd.v" 15 15
"$VIPS" linear "$TMP/codd.v" "$CONV/odd.png" 255 0 --uchar

# ramp256: a 256x1 grey ramp covering EVERY 0..255 sample value once, so the LUT
# ops (gamma, falsecolour) are compared over the FULL domain — a 16-value input
# verified only 16 of 256 LUT entries (adversarial-review conversion finding 5).
echo "==> [conversion input] ramp256 (256x1 Gray8, full 0..255 domain; for LUT ops)"
"$VIPS" grey "$TMP/cr256.v" 256 1
"$VIPS" linear "$TMP/cr256.v" "$CONV/ramp256.png" 255 0 --uchar

echo "==> [conversion input] stack (8x16 vertical ramp for grid: distinct tiles)"
"$VIPS" grey "$TMP/cs.v" 16 8
"$VIPS" rot "$TMP/cs.v" "$TMP/cs_v.v" d90
"$VIPS" linear "$TMP/cs_v.v" "$CONV/stack.png" 255 0 --uchar

echo "==> [conversion input] cond / cond2 (16x16 0/255 masks at distinct thresholds)"
"$VIPS" relational_const "$CONV/gray.png" "$CONV/cond.png"  more 127
"$VIPS" relational_const "$CONV/gray.png" "$CONV/cond2.png" more 63

# --- References — one vips run per differential case -------------------------
echo "==> [copy] header tweak (interpretation) — pixels unchanged (EXACT PNG)"
"$VIPS" copy "$CONV/rgb.png" "$CONV/copy_expected.png" --interpretation srgb

# Both cast targets are carried as `.v`: vips's pngsave MINIMISES bit depth, so a
# ushort image whose samples all fit in 8 bits is written as an 8-bit PNG,
# defeating the widen-to-16-bit differential. `.v` preserves the true ushort /
# float result losslessly (the libviprs decoder reads it back at full width).
echo "==> [cast] gray -> ushort (.v) and gray -> float (.v) (EXACT)"
"$VIPS" cast "$CONV/gray.png" "$CONV/cast_ushort_expected.v" ushort
"$VIPS" cast "$CONV/gray.png" "$CONV/cast_float_expected.v" float

echo "==> [flip] horizontal + vertical on grad (2-D; both arms discriminate) (EXACT PNG)"
"$VIPS" flip "$CONV/grad.png" "$CONV/flip_horizontal_expected.png" horizontal
"$VIPS" flip "$CONV/grad.png" "$CONV/flip_vertical_expected.png"   vertical

echo "==> [rot] d90 + d180 on grad (2-D; d180 != a horizontal flip) (EXACT PNG)"
"$VIPS" rot "$CONV/grad.png" "$CONV/rot_d90_expected.png"  d90
"$VIPS" rot "$CONV/grad.png" "$CONV/rot_d180_expected.png" d180

echo "==> [rot45] odd-square, --angle d45 (EXACT PNG)"
"$VIPS" rot45 "$CONV/odd.png" "$CONV/rot45_d45_expected.png" --angle d45

echo "==> [byteswap] nb16 (non-palindromic) -> .v (16-bit; byteswap truly moves bytes) (EXACT)"
"$VIPS" byteswap "$CONV/nb16.v" "$CONV/byteswap_expected.v"

echo "==> [msb] mb16 default (3 bands) + --band 0 (1 band) -> Gray8 (EXACT PNG)"
"$VIPS" msb "$CONV/mb16.v" "$CONV/msb_expected.png"
"$VIPS" msb "$CONV/mb16.v" "$CONV/msb_band0_expected.png" --band 0

echo "==> [grid] stack tile-height 4 across 2 down 2 -> 16x8 (EXACT PNG)"
"$VIPS" grid "$CONV/stack.png" "$CONV/grid_expected.png" 4 2 2

echo "==> [flatten] rgba --background \"0 0 0\" -> 3-band (BOUNDED-TOL ≤1 LSB)"
"$VIPS" flatten "$CONV/rgba.png" "$CONV/flatten_expected.png" --background "0 0 0"

echo "==> [ifthenelse] cond ? gray : gray2 -> 1-band (EXACT PNG)"
"$VIPS" ifthenelse "$CONV/cond.png" "$CONV/gray.png" "$CONV/gray2.png" \
    "$CONV/ifthenelse_expected.png"

# autorot: NO vips-oriented cross-oracle exists (libviprs ignores the vips `.v`
# XML orientation field and the TIFF Orientation tag). We exploit the identity
# autorot(orientation=6) == rot(d90): the vips reference is a plain `rot d90` of
# the base, and the `viprs`-readable oriented input is minted by `viprs` itself.
echo "==> [autorot] base (8x6) + rot-d90 reference; oriented input via viprs"
"$VIPS" grey "$TMP/cab.v" 8 6
"$VIPS" linear "$TMP/cab.v" "$CONV/autorot_base.png" 255 0 --uchar
"$VIPS" rot "$CONV/autorot_base.png" "$CONV/autorot_expected.png" d90
if [ -x "$VIPRS" ]; then
    "$VIPRS" copy "$CONV/autorot_base.png" "$CONV/autorot_oriented.v" --orientation 6
    echo "    (minted autorot_oriented.v with $VIPRS)"
else
    echo "    WARNING: viprs binary not found at '$VIPRS'; autorot_oriented.v NOT" >&2
    echo "    regenerated (kept the committed copy). Set VIPRS=/path/to/viprs." >&2
fi

echo "==> [wrap] grad (2-D; the h/2 vertical shift is exercised) (EXACT PNG)"
"$VIPS" wrap "$CONV/grad.png" "$CONV/wrap_expected.png"

# gamma is EXACT (tol 0): core issue #486 rebuilt the per-sample power LUT to be
# vips-exact over the full 0..255 domain, so the core and vips now agree
# bit-for-bit (previously a measured ≤1-LSB independent-rounding divergence).
echo "==> [gamma] ramp256 (full 0..255 domain) default + --exponent 2.0 (EXACT PNG)"
"$VIPS" gamma "$CONV/ramp256.png" "$CONV/gamma_expected.png"
"$VIPS" gamma "$CONV/ramp256.png" "$CONV/gamma_exp2_expected.png" --exponent 2.0

echo "==> [falsecolour] ramp256 (full domain) -> 3-band sRGB via PET LUT (EXACT PNG)"
"$VIPS" falsecolour "$CONV/ramp256.png" "$CONV/falsecolour_expected.png"

echo "==> [addalpha] rgb -> 4-band rgba (EXACT PNG)"
"$VIPS" addalpha "$CONV/rgb.png" "$CONV/addalpha_expected.png"

echo "==> [arrayjoin] 3 grays --across 2 -> 2x2 grid (EXACT PNG, >=3 variadic)"
"$VIPS" arrayjoin "$CONV/gray.png $CONV/gray2.png $CONV/gray3.png" \
    "$CONV/arrayjoin_expected.png" --across 2

echo "==> [grey] float ramp (.v, eps 1e-6) + --uchar (PNG, ≤1 LSB)"
"$VIPS" grey "$CONV/grey_float_expected.v" 16 16
"$VIPS" grey "$CONV/grey_uchar_expected.png" 16 16 --uchar

# vips tags identity output `histogram`, and pngsave has no histogram->b-w
# colourspace route, so the reference is carried as `.v` (the libviprs decoder
# reads the raw Gray8/Gray16 samples regardless of the interpretation tag).
echo "==> [identity] 256x1 Gray8 + --ushort 65536x1 Gray16 -> .v (EXACT)"
"$VIPS" identity "$CONV/identity_expected.v"
"$VIPS" identity "$CONV/identity_ushort_expected.v" --ushort

echo "==> [switch] cond, cond2 -> Gray8 index image (EXACT PNG, all 3 indices)"
"$VIPS" switch "$CONV/cond.png $CONV/cond2.png" "$CONV/switch_expected.png"

# --- Provenance (append the conversion section) ------------------------------
echo "==> [provenance] appending conversion section to $FIX_ROOT/PROVENANCE.md"
# CORE FAMILY (add / getpoint; OP_MAP.md core section — raster_ops.rs ops the
# frozen contract hard-codes, CLI_CONTRACT.md §1/§2).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_core_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. `add` is EXACT-AFTER-CAST (uchar+uchar
# widens to ushort; the output is integer so the §2 save-cast is a no-op and the
# differential compares at tol 0). `getpoint` is EXACT (integer pixel values).
#
# Carrier choice: `add` output is a 16-bit (ushort) raster — carried as the
# native `.v` container, which both vips and the libviprs decoder round-trip
# losslessly (a 16-bit PNG's encode path differs between the two libraries).
# `getpoint` prints an S3 scalar/vector to stdout, captured as a `.txt`.
# ===========================================================================
CORE="$FIX_ROOT/core"
mkdir -p "$CORE"

# --- Common inputs -----------------------------------------------------------
# Two DISTINCT 8x8 RGB uchar images (sRGB-tagged so a 3-band PNG saves as clean
# RGB, not a b-w image vips would alpha-pad) whose per-band sums EXCEED 255 in
# some bands, so `add` must widen 8-bit -> 16-bit (a broken add that stayed
# 8-bit would clip at 255 — the case is discriminating). `vips grey` is a pure
# coordinate function (0..1 ramp), so every input is bit-reproducible.
echo "==> [core input] two 8x8 sRGB RGB sources + two Gray8 sources (sums > 255)"
"$VIPS" grey "$TMP/cg.v" 8 8
"$VIPS" rot  "$TMP/cg.v" "$TMP/cgv.v" d90
# add_a bands (horizontal / vertical / horizontal ramps).
"$VIPS" linear "$TMP/cg.v"  "$TMP/a1.png" 180 20 --uchar
"$VIPS" linear "$TMP/cgv.v" "$TMP/a2.png" 180 30 --uchar
"$VIPS" linear "$TMP/cg.v"  "$TMP/a3.png" 150 40 --uchar
"$VIPS" bandjoin "$TMP/a1.png $TMP/a2.png $TMP/a3.png" "$TMP/a_rgb.v"
"$VIPS" copy "$TMP/a_rgb.v" "$CORE/add_a.png" --interpretation srgb
# add_b bands (distinct from add_a).
"$VIPS" linear "$TMP/cgv.v" "$TMP/b1.png" 150 50 --uchar
"$VIPS" linear "$TMP/cg.v"  "$TMP/b2.png" 150 60 --uchar
"$VIPS" linear "$TMP/cgv.v" "$TMP/b3.png" 150 40 --uchar
"$VIPS" bandjoin "$TMP/b1.png $TMP/b2.png $TMP/b3.png" "$TMP/b_rgb.v"
"$VIPS" copy "$TMP/b_rgb.v" "$CORE/add_b.png" --interpretation srgb
# Single-band Gray8 sources whose sums exceed 255 (Gray8 -> Gray16 widening).
"$VIPS" linear "$TMP/cg.v"  "$CORE/gray_a.png" 200 30 --uchar
"$VIPS" linear "$TMP/cgv.v" "$CORE/gray_b.png" 200 40 --uchar
# A CONSTANT 8x8 3-band FLOAT `.v` image ([0.5, 1.25, 2.5] everywhere): a clean
# dyadic value at every pixel, so `getpoint` prints it exactly and the case
# proves getpoint reads FLOAT samples (not just uchar) without a print-precision
# mismatch on a non-dyadic ramp value.
"$VIPS" black "$TMP/cblk.v" 8 8 --bands 3
"$VIPS" linear "$TMP/cblk.v" "$CORE/getpoint_float.v" "1 1 1" "0.5 1.25 2.5"
# A CONSTANT 8x8 3-band NON-DYADIC FLOAT `.v` image ([0.1, 0.2, 0.3] everywhere):
# none of these store exactly in f32. getpoint's stdout TEXT is NOT a contracted
# parity surface (§9): core prints f64::to_string of the widened f32 (e.g.
# 0.10000000149011612). This DE-RIGS the deliberately-dyadic getpoint_float
# fixture: it proves the differential's numeric float-parse + epsilon compare
# (NOT a dyadic text match) is what carries a float getpoint case
# (CLI_CONTRACT.md §3), robust to any text-format difference.
"$VIPS" linear "$TMP/cblk.v" "$CORE/getpoint_float_nd.v" "1 1 1" "0.1 0.2 0.3"
# Two CONSTANT 8x8 1-band 16-bit (ushort) `.v` images (40000 everywhere): their
# per-pixel sum (80000) OVERFLOWS ushort. vips `add` PROMOTES ushort→uint and
# returns 80000, but core keeps the input at 16-bit and SATURATES the sum at
# 65535 — a silent divergence the uchar-only fixtures structurally hide. The CLI
# now REJECTS 16-bit inputs (exit 1) so it never emits a wrong 65535 "success";
# the `add_rejects_16bit_input_without_panicking` error case pins that. These are
# committed INPUT fixtures only — an error case needs no vips reference output.
"$VIPS" black "$TMP/cblk1.v" 8 8 --bands 1
"$VIPS" linear "$TMP/cblk1.v" "$TMP/u16a_f.v" 1 40000
"$VIPS" cast   "$TMP/u16a_f.v" "$CORE/u16_a.v" ushort
"$VIPS" linear "$TMP/cblk1.v" "$TMP/u16b_f.v" 1 40000
"$VIPS" cast   "$TMP/u16b_f.v" "$CORE/u16_b.v" ushort

# --- References — one vips run per differential case -------------------------
echo "==> [add] rgb + rgb -> 16-bit ushort .v (per-band, 8->16 widening)"
"$VIPS" add "$CORE/add_a.png" "$CORE/add_b.png" "$CORE/add_rgb_expected.v"
echo "==> [add] gray + gray -> 16-bit ushort .v (Gray8 -> Gray16 widening)"
"$VIPS" add "$CORE/gray_a.png" "$CORE/gray_b.png" "$CORE/add_gray_expected.v"

echo "==> [getpoint] rgb (3-band) + gray (1-band) + float (3-band) pixel reads (S3)"
"$VIPS" getpoint "$CORE/add_a.png"          5 2 > "$CORE/getpoint_rgb_expected.txt"
"$VIPS" getpoint "$CORE/gray_a.png"         5 2 > "$CORE/getpoint_gray_expected.txt"
"$VIPS" getpoint "$CORE/getpoint_float.v"   0 0 > "$CORE/getpoint_float_expected.txt"
echo "==> [getpoint] NON-DYADIC float (3-band) pixel read (S3, numeric-eps compare)"
"$VIPS" getpoint "$CORE/getpoint_float_nd.v" 0 0 > "$CORE/getpoint_float_nd_expected.txt"

# --- Provenance (append the core section) ------------------------------------
echo "==> [provenance] appending core section to $FIX_ROOT/PROVENANCE.md"
cat >> "$FIX_ROOT/PROVENANCE.md" <<EOF

---

# extract family CLI-differential reference provenance

These fixtures are the committed vips oracle references the extract
CLI-differential suite (\`tests/cli_extract_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`extract/\`): \`gray.png\` (16×16 Gray8 horizontal
  ramp), \`rgb.png\` (16×16 3-band sRGB with 2-D structure — band0 horizontal
  ramp, band1 scaled horizontal ramp, band2 vertical ramp), \`sub.png\` (6×6
  solid sRGB, the distinct insert payload), \`sub1.png\` (6×6 SINGLE-band Gray8
  ramp, the 1-band-sub → multi-band-main \`insert\` bandalike-broadcast payload).
- **Every op is EXACT** (integer-in / integer-out, decode-compare tol 0) EXCEPT
  \`smartcrop --interesting entropy\`, which is **GOLDEN-ONLY**: vips's entropy
  strategy makes a different discrete crop-window choice than the core on this
  input (measured max-abs-diff 136 — a wholesale different region, not a
  tolerance), so there is no cross-oracle. Its reference
  (\`smartcrop_entropy_golden.png\`) is generated by \`viprs\` itself
  (deterministic across runs) and the test is a regression pin.
# conversion family CLI-differential reference provenance

These fixtures are the committed vips oracle references the conversion
CLI-differential suite (\`tests/cli_conversion_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`conversion/\`): \`gray.png\`/\`gray2.png\`/\`gray3.png\`
  (16×16 Gray8), \`rgb.png\`/\`rgba.png\` (sRGB 3/4-band), \`grad.png\` (16×16 2-D
  gradient varying in BOTH axes, for \`flip\`/\`rot\`/\`wrap\` — a Y-constant ramp
  made the vertical arms no-ops), \`nb16.v\` (16×16 Gray16, non-palindromic bytes,
  for \`byteswap\` — a byte-palindrome would pass a no-op), \`mb16.v\` (16×16
  3-band Gray16, band0 != band1, for \`msb --band\`), \`ramp256.png\` (256×1 Gray8
  full 0..255 domain, for the \`gamma\`/\`falsecolour\` LUTs), \`odd.png\` (15×15
  Gray8, for \`rot45\`), \`stack.png\` (8×16 vertical ramp with distinct tiles, for
  \`grid\`), \`cond.png\`/\`cond2.png\` (0/255 masks at thresholds 127/63, for
  \`ifthenelse\`/\`switch\`).
- **Carriers**: 1/3/4-band uchar → PNG. \`cast\`→float and \`grey\` float ramp →
  \`.v\` (float); \`byteswap\` → \`.v\` (16-bit byte order PNG would normalise). The
  16-bit \`nb16\`/\`mb16\` inputs are \`.v\` (raw byte order / multiband 16-bit).

## autorot — no vips cross-oracle (a real limitation, flagged)

libviprs' decoders read neither the vips \`.v\` XML \`orientation\` field nor the
TIFF Orientation tag (274) that vips writes, so a vips-oriented input is a
**no-op** under \`viprs\` while vips rotates — the two orientation metadata
channels are mutually unreadable (the same shape as \`globalbalance\`). The
oriented input \`autorot_oriented.v\` is therefore minted by **\`viprs\`**
(\`viprs copy autorot_base.png autorot_oriented.v --orientation 6\`), the only
producer of an orientation \`viprs\` reads back. The reference remains a genuine
vips oracle via the identity **autorot(orientation=6) == rot(d90)**: verified
pixel-for-pixel that \`viprs autorot autorot_oriented.v\` equals
\`vips rot autorot_base.png … d90\`. See the open question about teaching the
libviprs decoders to read the EXIF/TIFF orientation tag.
# core family (add / getpoint) CLI-differential reference provenance

These fixtures are the committed vips oracle references the core
CLI-differential suite (\`tests/cli_core_diff.rs\`) compares \`viprs\` output
against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI. \`add\`
and \`getpoint\` are the two \`raster_ops.rs\` ops the frozen contract hard-codes
(CLI_CONTRACT.md §1/§2).

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`core/\`): \`add_a.png\`, \`add_b.png\` (two distinct
  8×8 sRGB RGB uchar images whose per-band sums exceed 255, exercising the
  8→16-bit widening), \`gray_a.png\`, \`gray_b.png\` (single-band Gray8 sums > 255),
  \`getpoint_float.v\` (constant 8×8 3-band float \`[0.5, 1.25, 2.5]\`, for the
  float-sample getpoint read), \`getpoint_float_nd.v\` (constant 8×8 3-band
  NON-DYADIC float \`[0.1, 0.2, 0.3]\` — de-rigs the dyadic fixture: proves the
  numeric float-parse + eps compare, not a dyadic text match, carries the case),
  \`u16_a.v\`, \`u16_b.v\` (constant 8×8 1-band 16-bit ushort \`40000\`, sum
  overflows ushort — INPUTS for the 16-bit-reject error case; no reference output).
- **Carriers**: \`add\` output is a 16-bit ushort raster carried as native \`.v\`
  (a 16-bit PNG encode differs between vips and libviprs; \`.v\` round-trips
  losslessly on both sides). \`getpoint\` prints an S3 scalar/vector, captured to
  \`.txt\` and compared numerically (never as text; CLI_CONTRACT.md §3).

## Exact commands

Inputs:

\`\`\`
vips grey egrey.v 16 16
vips linear egrey.v extract/gray.png 255 0 --uchar
vips linear egrey.v egray2.png 200 10 --uchar
vips rot egrey.v egrey_v.v d90
vips linear egrey_v.v egray3.png 255 0 --uchar
vips bandjoin "extract/gray.png egray2.png egray3.png" ergb.v
vips copy ergb.v extract/rgb.png --interpretation srgb
vips black eb.v 6 6 --bands 3
vips linear eb.v esub.v "0 0 0" "200 50 100" --uchar
vips copy esub.v extract/sub.png --interpretation srgb
vips grey esub1.v 6 6
vips linear esub1.v extract/sub1.png 255 0 --uchar
vips grey cg.v 16 16
vips linear cg.v conversion/gray.png  255 0  --uchar
vips linear cg.v conversion/gray2.png 200 10 --uchar
vips rot cg.v cgrot.v d90 ; vips linear cgrot.v conversion/gray3.png 255 0 --uchar
vips bandjoin "conversion/gray.png conversion/gray2.png conversion/gray3.png" crgb.v
vips copy crgb.v conversion/rgb.png --interpretation srgb
vips bandjoin "conversion/rgb.png conversion/gray.png" crgba.v
vips copy crgba.v conversion/rgba.png --interpretation srgb
# grad: 2-D gradient (x*85 + y*170) so vertical flip / wrap / rot-d180 discriminate
vips linear cg.v cgx.v 85 0 ; vips linear cgrot.v cgy.v 170 0
vips add cgx.v cgy.v cgsum.v ; vips cast cgsum.v conversion/grad.png uchar
# nb16: non-palindromic 16-bit (multiples of 0x1000) so byteswap moves data
vips linear cg.v cnb16f.v 61440 0 ; vips cast cnb16f.v conversion/nb16.v ushort
# mb16: 3-band 16-bit with band0 != band1 so msb --band 0 differs from the default
vips linear cg.v cm16a.v 65535 0 ; vips linear cg.v cm16b.v 40000 0 ; vips linear cgrot.v cm16c.v 50000 0
vips bandjoin "cm16a.v cm16b.v cm16c.v" cmb16f.v ; vips cast cmb16f.v conversion/mb16.v ushort
vips grey codd.v 15 15 ; vips linear codd.v conversion/odd.png 255 0 --uchar
# ramp256: full 0..255 domain so gamma/falsecolour LUTs are fully covered
vips grey cr256.v 256 1 ; vips linear cr256.v conversion/ramp256.png 255 0 --uchar
vips grey cs.v 16 8 ; vips rot cs.v cs_v.v d90 ; vips linear cs_v.v conversion/stack.png 255 0 --uchar
vips relational_const conversion/gray.png conversion/cond.png  more 127
vips relational_const conversion/gray.png conversion/cond2.png more 63
vips grey cab.v 8 6 ; vips linear cab.v conversion/autorot_base.png 255 0 --uchar
viprs copy conversion/autorot_base.png conversion/autorot_oriented.v --orientation 6
vips grey cg.v 8 8
vips rot  cg.v cgv.v d90
vips linear cg.v  a1.png 180 20 --uchar
vips linear cgv.v a2.png 180 30 --uchar
vips linear cg.v  a3.png 150 40 --uchar
vips bandjoin "a1.png a2.png a3.png" a_rgb.v
vips copy a_rgb.v core/add_a.png --interpretation srgb
vips linear cgv.v b1.png 150 50 --uchar
vips linear cg.v  b2.png 150 60 --uchar
vips linear cgv.v b3.png 150 40 --uchar
vips bandjoin "b1.png b2.png b3.png" b_rgb.v
vips copy b_rgb.v core/add_b.png --interpretation srgb
vips linear cg.v  core/gray_a.png 200 30 --uchar
vips linear cgv.v core/gray_b.png 200 40 --uchar
vips black cblk.v 8 8 --bands 3
vips linear cblk.v core/getpoint_float.v "1 1 1" "0.5 1.25 2.5"
vips linear cblk.v core/getpoint_float_nd.v "1 1 1" "0.1 0.2 0.3"
vips black cblk1.v 8 8 --bands 1
vips linear cblk1.v u16a_f.v 1 40000
vips cast   u16a_f.v core/u16_a.v ushort
vips linear cblk1.v u16b_f.v 1 40000
vips cast   u16b_f.v core/u16_b.v ushort
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | vips command |
|---|---|---|
| \`extract/extract_area_expected.png\` | EXACT | \`vips extract_area rgb.png extract_area_expected.png 3 4 5 6\` |
| \`extract/crop_expected.png\` | EXACT | \`vips crop rgb.png crop_expected.png 3 4 5 6\` (alias of extract_area) |
| \`extract/embed_black_expected.png\` | EXACT | \`vips embed rgb.png embed_black_expected.png 2 3 24 24\` |
| \`extract/embed_copy_expected.png\` | EXACT | \`vips embed rgb.png embed_copy_expected.png 2 3 24 24 --extend copy\` |
| \`extract/embed_repeat_expected.png\` | EXACT | \`vips embed rgb.png embed_repeat_expected.png 2 3 24 24 --extend repeat\` |
| \`extract/embed_mirror_expected.png\` | EXACT | \`vips embed rgb.png embed_mirror_expected.png 2 3 24 24 --extend mirror\` |
| \`extract/embed_white_expected.png\` | EXACT | \`vips embed rgb.png embed_white_expected.png 2 3 24 24 --extend white\` |
| \`extract/embed_bg_expected.png\` | EXACT | \`vips embed gray.png embed_bg_expected.png 1 1 8 8 --extend background --background 128\` |
| \`extract/gravity_centre_expected.png\` | EXACT | \`vips gravity rgb.png gravity_centre_expected.png centre 24 24\` |
| \`extract/gravity_se_expected.png\` | EXACT | \`vips gravity rgb.png gravity_se_expected.png south-east 24 24\` |
| \`extract/gravity_nw_expected.png\` | EXACT | \`vips gravity rgb.png gravity_nw_expected.png north-west 24 24\` |
| \`extract/replicate_expected.png\` | EXACT | \`vips replicate rgb.png replicate_expected.png 2 3\` |
| \`extract/zoom_expected.png\` | EXACT | \`vips zoom gray.png zoom_expected.png 3 2\` |
| \`extract/subsample_expected.png\` | EXACT | \`vips subsample rgb.png subsample_expected.png 2 2\` |
| \`extract/insert_expected.png\` | EXACT | \`vips insert rgb.png sub.png insert_expected.png 4 5\` |
| \`extract/insert_expand_expected.png\` | EXACT | \`vips insert rgb.png sub.png insert_expand_expected.png 13 13 --expand\` (canvas grows to 19×19) |
| \`extract/insert_bandalike_expected.png\` | EXACT | \`vips insert rgb.png sub1.png insert_bandalike_expected.png 4 5\` (1-band sub broadcast onto 3-band main) |
| \`extract/smartcrop_centre_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_centre_expected.png 8 8 --interesting centre\` |
| \`extract/smartcrop_all_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_all_expected.png 8 8 --interesting all\` |
| \`extract/smartcrop_low_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_low_expected.png 8 8 --interesting low\` |
| \`extract/smartcrop_high_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_high_expected.png 8 8 --interesting high\` |
| \`extract/smartcrop_attention_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_attention_expected.png 8 8 --interesting attention\` (crop 15,11 — non-vacuous, differs from low/high) |
| \`extract/smartcrop_entropy_golden.png\` | GOLDEN-ONLY | \`viprs smartcrop rgb.png smartcrop_entropy_golden.png 8 8 --interesting entropy\` (NO vips oracle — vips picks a different discrete window) |
| \`conversion/copy_expected.png\` | EXACT | \`vips copy rgb.png copy_expected.png --interpretation srgb\` |
| \`conversion/cast_ushort_expected.v\` | EXACT | \`vips cast gray.png cast_ushort_expected.v ushort\` (pngsave minimises depth → \`.v\`) |
| \`conversion/cast_float_expected.v\` | EXACT | \`vips cast gray.png cast_float_expected.v float\` |
| \`conversion/flip_horizontal_expected.png\` | EXACT | \`vips flip grad.png flip_horizontal_expected.png horizontal\` |
| \`conversion/flip_vertical_expected.png\` | EXACT | \`vips flip grad.png flip_vertical_expected.png vertical\` |
| \`conversion/rot_d90_expected.png\` | EXACT | \`vips rot grad.png rot_d90_expected.png d90\` |
| \`conversion/rot_d180_expected.png\` | EXACT | \`vips rot grad.png rot_d180_expected.png d180\` |
| \`conversion/rot45_d45_expected.png\` | EXACT | \`vips rot45 odd.png rot45_d45_expected.png --angle d45\` |
| \`conversion/byteswap_expected.v\` | EXACT | \`vips byteswap nb16.v byteswap_expected.v\` (non-palindromic 16-bit) |
| \`conversion/msb_expected.png\` | EXACT | \`vips msb mb16.v msb_expected.png\` (3-band) |
| \`conversion/msb_band0_expected.png\` | EXACT | \`vips msb mb16.v msb_band0_expected.png --band 0\` (1-band) |
| \`conversion/grid_expected.png\` | EXACT | \`vips grid stack.png grid_expected.png 4 2 2\` |
| \`conversion/flatten_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips flatten rgba.png flatten_expected.png --background "0 0 0"\` |
| \`conversion/ifthenelse_expected.png\` | EXACT | \`vips ifthenelse cond.png gray.png gray2.png ifthenelse_expected.png\` |
| \`conversion/autorot_expected.png\` | EXACT (golden via rot-d90 identity; see above) | \`vips rot autorot_base.png autorot_expected.png d90\` |
| \`conversion/wrap_expected.png\` | EXACT | \`vips wrap grad.png wrap_expected.png\` |
| \`conversion/gamma_expected.png\` | EXACT (tol 0) | \`vips gamma ramp256.png gamma_expected.png\` (full domain; core #486 rebuilt the power LUT vips-exact) |
| \`conversion/gamma_exp2_expected.png\` | EXACT (tol 0) | \`vips gamma ramp256.png gamma_exp2_expected.png --exponent 2.0\` (full domain) |
| \`conversion/falsecolour_expected.png\` | EXACT | \`vips falsecolour ramp256.png falsecolour_expected.png\` (full domain) |
| \`conversion/addalpha_expected.png\` | EXACT | \`vips addalpha rgb.png addalpha_expected.png\` |
| \`conversion/arrayjoin_expected.png\` | EXACT | \`vips arrayjoin "gray.png gray2.png gray3.png" arrayjoin_expected.png --across 2\` |
| \`conversion/grey_float_expected.v\` | BOUNDED-TOL (float eps 1e-6) | \`vips grey grey_float_expected.v 16 16\` |
| \`conversion/grey_uchar_expected.png\` | BOUNDED-TOL (≤1 LSB) | \`vips grey grey_uchar_expected.png 16 16 --uchar\` |
| \`conversion/identity_expected.v\` | EXACT | \`vips identity identity_expected.v\` (histogram-tagged → \`.v\`) |
| \`conversion/identity_ushort_expected.v\` | EXACT | \`vips identity identity_ushort_expected.v --ushort\` |
| \`conversion/switch_expected.png\` | EXACT | \`vips switch "cond.png cond2.png" switch_expected.png\` |
| \`core/add_rgb_expected.v\` | EXACT-AFTER-CAST (tol 0) | \`vips add add_a.png add_b.png add_rgb_expected.v\` (ushort, per-band, 8→16 widening) |
| \`core/add_gray_expected.v\` | EXACT-AFTER-CAST (tol 0) | \`vips add gray_a.png gray_b.png add_gray_expected.v\` (Gray8→Gray16 widening) |
| \`core/getpoint_rgb_expected.txt\` | EXACT (S3 vector) | \`vips getpoint add_a.png 5 2\` (3-band pixel) |
| \`core/getpoint_gray_expected.txt\` | EXACT (S3 scalar) | \`vips getpoint gray_a.png 5 2\` (1-band pixel) |
| \`core/getpoint_float_expected.txt\` | EXACT (S3 vector, float) | \`vips getpoint getpoint_float.v 0 0\` (float [0.5 1.25 2.5]) |
| \`core/getpoint_float_nd_expected.txt\` | BOUNDED-TOL (S3 vector, float; numeric eps 1e-6) | \`vips getpoint getpoint_float_nd.v 0 0\` (non-dyadic [0.1 0.2 0.3]; stdout text is §9 out-of-scope, numeric-eps compare carries it) |

\`add\` rejects float inputs, **16-bit inputs** (core saturates the sum at 65535
while vips promotes ushort→uint — the \`u16_a.v\`/\`u16_b.v\` inputs pin this), and
a dimension / channel-count mismatch with a typed exit-1 error, never a panic;
\`getpoint\` rejects an out-of-bounds coordinate the same way. Those error paths
are asserted in \`cli_core_diff.rs\` (nonzero exit + a viprs-side message
substring; CLI_CONTRACT.md §8) and need no vips reference. Note: vips \`add\`
BAND-BROADCASTS a 1-band operand across a multi-band one; core requires EQUAL
band counts, so the channel-mismatch case is a documented SUBSET limitation (core
cannot broadcast without a core change), not a parity claim.
EOF

# ===========================================================================
# CONVOLUTION FAMILY (the Wave-2 convolution lane; OP_MAP.md convolution
# section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_convolution_diff.rs (which feeds them to `viprs`), so
# the two sides compare like against like.
#
# HONEST oracle classes (measured against vips 8.18.4 on the author Mac):
#  * `compass --combine max` at INTEGER precision with a scale-1 (sobel) mask and
#    `fastcor` are EXACT (tol 0): a scale-1 mask needs no coefficient rounding, and
#    fastcor is an integer sum-of-squared-differences, so neither hits the
#    integer-precision rounding-scheme gap below. `compass --combine sum` at
#    INTEGER precision is likewise EXACT — its distinct 16-bit-promotion +
#    saturation branch (out_fmt = ushort) is deterministic integer arithmetic.
#  * conv / convsep / gaussblur at INTEGER precision on uchar are **EXACT
#    (tol 0)** against a `VIPS_NOVECTOR=1` reference, and are generated that way
#    (issue #558). They used to be BOUNDED-TOL ≤1 LSB against a default-`vips`
#    reference. That tolerance was wrong twice over. Its stated cause was vips
#    shifting by `>> sexp` where the core divides `(sum + scale/2) / scale` —
#    but `sexp` is the ORC variable, never set in a HWY build, so the
#    justification described a code path absent from the binary it was measured
#    against. And "at most one LSB" is false: `gaussblur … 1.6` on this same
#    eye.png moves 256 samples by up to 4, and `convsep … sep.mat` by 4, both of
#    which the ≤1 tolerance would have failed the moment anybody varied a
#    parameter. What actually differs is that the vector path convolves with
#    REQUANTISED COEFFICIENTS, and nobody has ever bounded that: see
#    `conv_hostile_int_expected.png`, where a mask libvips's own accuracy gate
#    ACCEPTS moves samples by 57.
#  * convsep / conv / compass / gaussblur / gaussmat / logmat at FLOAT precision
#    and spcor are float surfaces: compared at a small BOUNDED-TOL (the core
#    accumulates in a different order than vips's vectorised path; measured
#    ≤1.5e-5 on the author Mac, with headroom for cross-platform libm/FMA drift).
#  * gaussmat / logmat at INTEGER precision are integer-valued matrices: tol 0.
#  * sharpen keeps its OWN BOUNDED-TOL ≤1 LSB, and it is NOT the tolerance
#    above. sharpen convolves the L of LabS, which is 16-bit, and the vector
#    path is gated on `BandFmt == VIPS_FORMAT_UCHAR` (convi.c:1151), so sharpen
#    takes the portable C path on BOTH libvips builds — verified with
#    VIPS_INFO=1, which reports "convi: using C path". Whatever sharpen's
#    deviation is, #558 cannot be its mechanism; it is a real libviprs bug,
#    tracked as issue #581, and it was hiding under the shared constant.
#
# Every input is chosen DISCRIMINATING (a broken/identity op fails loudly): the
# `eye.png` zone-plate is high-frequency, so a box blur moves pixels by up to 59
# (≫ the ≤1 tol), compass edge-detection by 254, and the extracted `patch.png`
# gives correlation surfaces with a sharp peak. Carriers: integer uchar outputs →
# PNG; float / matrix outputs → the native `.v` container (CLI_CONTRACT.md §2).
# vips writes the gaussmat/logmat matrix and fastcor surface as double/uint the
# libviprs decoder cannot read, so they are `vips cast … float`ed to a float `.v`.
# ===========================================================================
CONVOL="$FIX_ROOT/convolution"
mkdir -p "$CONVOL"

# --- Common inputs -----------------------------------------------------------
# `vips eye` (a zone-plate grating) and `vips grey` are pure coordinate
# functions, so every input below is bit-for-bit reproducible.
echo "==> [convolution input] eye.png (16x16 Gray8 zone-plate, high frequency)"
"$VIPS" eye "$TMP/veye.v" 16 16
"$VIPS" linear "$TMP/veye.v" "$CONVOL/eye.png" 127.5 127.5 --uchar

# A 5x5 patch of the zone-plate for the correlations (a clear best-match peak).
echo "==> [convolution input] patch.png (5x5 extract of eye.png; correlation template)"
"$VIPS" extract_area "$CONVOL/eye.png" "$CONVOL/patch.png" 4 4 5 5

# --- #558 discriminating inputs ---------------------------------------------
# The old convolution cell could not tell libvips's two integer-convolution
# paths apart on purpose: it happened to, at one sigma, by one unit. These
# three inputs are chosen so the difference is loud, so that a future
# regeneration against the wrong path fails instead of drifting.

# A 64x64 pseudo-random field. `vips gaussnoise --seed` is deterministic (same
# seed, byte-identical output — verified), so this stays reproducible. High
# entropy is what makes a hostile mask hostile: on the 16x16 zone-plate the same
# mask only reaches 2, on this it reaches 57.
echo "==> [convolution input] noise64.png (64x64 Gray8, gaussnoise seed 42)"
"$VIPS" gaussnoise "$TMP/gn.v" 64 64 --mean 128 --sigma 50 --seed 42
"$VIPS" cast "$TMP/gn.v" "$CONVOL/noise64.png" uchar

# A 3x3 field whose every 3x3 window (edges replicated) sums to exactly 1147:
# eight 127s around a single 131. That is the arithmetic the panel used to kill
# "it is just the rounding mode":
#     C path      (1147 + 9/2) / 9        = 127
#     floor       floor(1147 / 9)         = 127   <- SAME, so not the rounding
#     vector path (57 * 1147 + 256) >> 9  = 128   <- different COEFFICIENTS
# The vector path is filtering with 57/512 = 0.111328, not 1/9 = 0.111111.
echo "==> [convolution input] boxsum1147.png (3x3; every window sums to 1147)"
"$VIPS" black "$TMP/bx.v" 3 3
"$VIPS" linear "$TMP/bx.v" "$TMP/bx127.v" 0 127 --uchar
"$VIPS" copy "$TMP/bx127.v" "$TMP/bxdraw.v"
"$VIPS" draw_rect "$TMP/bxdraw.v" 131 1 1 1 1
"$VIPS" copy "$TMP/bxdraw.v" "$CONVOL/boxsum1147.png"

# A 16-bit carrier for the ushort regime boundary. `vips_convi`'s vector path is
# gated on `BandFmt == VIPS_FORMAT_UCHAR` (convi.c:1151), so a ushort image
# takes the portable C path on every libvips build and libviprs cannot be wrong
# about it. Nothing in the suite tested that until now. `.v` rather than PNG
# because that is the carrier the 16-bit iocleanup fixtures already use.
echo "==> [convolution input] eye16.v (16x16 ushort zone-plate)"
"$VIPS" linear "$TMP/veye.v" "$TMP/eye16f.v" 32767.5 32767.5
"$VIPS" cast "$TMP/eye16f.v" "$CONVOL/eye16.v" ushort

# --- Mask files (vips text matrix; read by BOTH vips and viprs) --------------
# blur: a 3x3 box (scale 9 -> the two libvips paths differ; see above).
# sobel: a 3x3 edge detector (scale 1 -> `vips_convi_intize` is the identity, so
#        the vector path runs AND agrees; odd-square, so compass can rotate it).
# sep: a 1x5 separable smoother (scale 10).
# hostile: a mask libvips's own accuracy gate ACCEPTS — VIPS_INFO=1 reports
#        "convi: using vector path" — on which the two paths differ by 57.
#        `vips_convi_intize`'s check (convi.c:1096-1113) is a DC-gain test
#        against exact real arithmetic at one grey level on a flat field; it
#        constrains sum(w_hat - w) and says nothing about per-pixel error,
#        which is sum((w_hat - w) * p). This is the regression that fails if
#        anybody ever writes a tolerance for integer conv again.
echo "==> [convolution mask] blur.mat / sobel.mat / sep.mat / hostile.mat"
printf '3 3 9\n1 1 1\n1 1 1\n1 1 1\n'   > "$CONVOL/blur.mat"
printf '3 3 1\n1 2 1\n0 0 0\n-1 -2 -1\n' > "$CONVOL/sobel.mat"
printf '5 1 10\n1 2 4 2 1\n'            > "$CONVOL/sep.mat"
printf '3 3 3 0\n45 -17 -25\n-33 -15 -34\n55 53 -26\n' > "$CONVOL/hostile.mat"

# --- References — one vips run per differential case -------------------------
# gaussmat / logmat: the double matrix is `cast … float`ed so the libviprs
# decoder can read it (no double PixelFormat). The header scale is mask metadata
# the raster differential does not compare.
echo "==> [gaussmat] integer + separable + float precision -> float .v"
"$VIPS" gaussmat "$TMP/gm.v"  2 0.2;              "$VIPS" cast "$TMP/gm.v"  "$CONVOL/gaussmat_int_expected.v"   float
"$VIPS" gaussmat "$TMP/gms.v" 2 0.2 --separable;  "$VIPS" cast "$TMP/gms.v" "$CONVOL/gaussmat_sep_expected.v"   float
"$VIPS" gaussmat "$TMP/gmf.v" 2 0.2 --precision float; "$VIPS" cast "$TMP/gmf.v" "$CONVOL/gaussmat_float_expected.v" float

echo "==> [logmat] integer + float separable -> float .v"
"$VIPS" logmat "$TMP/lm.v"  2 0.1;                             "$VIPS" cast "$TMP/lm.v"  "$CONVOL/logmat_int_expected.v"   float
"$VIPS" logmat "$TMP/lmf.v" 2 0.1 --separable --precision float; "$VIPS" cast "$TMP/lmf.v" "$CONVOL/logmat_float_expected.v" float

echo "==> [conv] box blur integer (PNG, EXACT via VIPS_NOVECTOR=1) + sobel float (.v)"
novector_vips conv "$CONVOL/eye.png" "$CONVOL/conv_blur_int_expected.png"   "$CONVOL/blur.mat"  --precision integer
"$VIPS"       conv "$CONVOL/eye.png" "$CONVOL/conv_sobel_float_expected.v"  "$CONVOL/sobel.mat" --precision float

echo "==> [convsep] separable smoother, integer (PNG, EXACT) + float (.v)"
novector_vips convsep "$CONVOL/eye.png" "$CONVOL/convsep_int_expected.png"   "$CONVOL/sep.mat" --precision integer
"$VIPS"       convsep "$CONVOL/eye.png" "$CONVOL/convsep_float_expected.v"   "$CONVOL/sep.mat" --precision float

echo "==> [compass] max integer (PNG, EXACT — scale-1 mask) + sum float/integer (.v)"
novector_vips compass "$CONVOL/eye.png" "$CONVOL/compass_max_int_expected.png" "$CONVOL/sobel.mat" \
    --times 4 --angle d45 --combine max --precision integer
"$VIPS" compass "$CONVOL/eye.png" "$CONVOL/compass_sum_float_expected.v" "$CONVOL/sobel.mat" \
    --times 4 --angle d45 --combine sum --precision float
# combine=sum at INTEGER precision: the distinct 16-bit-promotion + saturation
# branch (out_fmt promotes uchar to 16-bit for Sum; the summed sobel edges reach
# 812, above the uchar range). Integer arithmetic is deterministic → EXACT (tol
# 0). vips emits this surface as **uint** (band format 4), which the libviprs
# decoder does not read; the core (viprs) emits it as **ushort** (Gray16). The
# decode-compare requires matching format classes, so the reference is
# `vips cast … ushort` — lossless for these values (max 812 « 65535) and the same
# Gray16 class viprs produces (unlike fastcor, whose viprs surface is float).
novector_vips compass "$CONVOL/eye.png" "$TMP/compass_sum_int_uint.v" "$CONVOL/sobel.mat" \
    --times 4 --angle d45 --combine sum --precision integer
"$VIPS" cast "$TMP/compass_sum_int_uint.v" "$CONVOL/compass_sum_int_expected.v" ushort

echo "==> [gaussblur] integer (PNG, EXACT via VIPS_NOVECTOR=1) + float (.v)"
novector_vips gaussblur "$CONVOL/eye.png" "$CONVOL/gaussblur_int_expected.png" 1.5 --precision integer
"$VIPS"       gaussblur "$CONVOL/eye.png" "$CONVOL/gaussblur_float_expected.v" 1.5 --precision float

# --- #558 discriminating references -----------------------------------------
# Each of these is EXACT against the portable-C path AND has a large, measured
# gap to the vectorised path, so a reference regenerated against the wrong
# libvips fails loudly rather than sliding under a tolerance. The gaps below
# were measured on vips 8.18.4 with targets: NEON_BF16 NEON.
echo "==> [#558] gaussblur sigma 1.6 (vector-path gap 4 — the case tol=1 would fail)"
novector_vips gaussblur "$CONVOL/eye.png" "$CONVOL/gaussblur_s1.6_int_expected.png" 1.6 --precision integer

echo "==> [#558] gaussblur sigma 0.8 on noise64 (vector-path gap 2)"
novector_vips gaussblur "$CONVOL/noise64.png" "$CONVOL/gaussblur_s0.8_int_expected.png" 0.8 --precision integer

echo "==> [#558] box blur on the 1147 window (vector-path gap 1, whole image)"
novector_vips conv "$CONVOL/boxsum1147.png" "$CONVOL/conv_boxsum1147_int_expected.png" \
    "$CONVOL/blur.mat" --precision integer

echo "==> [#558] hostile mask libvips's own accuracy gate accepts (vector-path gap 57)"
novector_vips conv "$CONVOL/noise64.png" "$CONVOL/conv_hostile_int_expected.png" \
    "$CONVOL/hostile.mat" --precision integer

# --- #558 regime boundaries --------------------------------------------------
# Three regimes, not two, and nothing on the API surface says which one a mask
# is in. These pin the two where the paths CANNOT differ, so a future change
# that moves a mask across `vips_convi_intize`'s accept/reject boundary is
# visible as a test failure rather than as a quiet re-pin.
echo "==> [#558 regime] ushort never diverges (convi.c:1151 gates on UCHAR)"
novector_vips conv "$CONVOL/eye16.v" "$CONVOL/conv_ushort_int_expected.v" \
    "$CONVOL/blur.mat" --precision integer

echo "==> [#558 regime] sigma 0.6: intize declines the mask, libvips runs C itself"
novector_vips gaussblur "$CONVOL/eye.png" "$CONVOL/gaussblur_s0.6_int_expected.png" 0.6 --precision integer

echo "==> [#558 regime] scale-1 mask: the vector path RUNS and still agrees"
novector_vips conv "$CONVOL/eye.png" "$CONVOL/conv_sobel_int_expected.png" \
    "$CONVOL/sobel.mat" --precision integer

# sharpen is deliberately NOT run through novector_vips. It convolves the L of
# LabS, which is 16-bit, so `vips_convi`'s uchar gate (convi.c:1151) never opens
# and VIPS_INFO=1 reports "convi: using C path" on the plain binary. Setting
# VIPS_NOVECTOR here would change nothing and would wrongly imply #558 reaches
# it. Its remaining deviation is a separate libviprs bug, issue #581.
echo "==> [sharpen] eye (mono, high-freq) --sigma 1 --m1 1 --m2 2 (PNG, ≤1 LSB, issue #581)"
"$VIPS" sharpen "$CONVOL/eye.png" "$CONVOL/sharpen_expected.png" --sigma 1 --m1 1 --m2 2

echo "==> [spcor] normalised cross-correlation (.v float, eps 1e-5)"
"$VIPS" spcor "$CONVOL/eye.png" "$CONVOL/patch.png" "$CONVOL/spcor_expected.v"

echo "==> [fastcor] SSD surface (vips uint -> cast float .v, EXACT)"
"$VIPS" fastcor "$CONVOL/eye.png" "$CONVOL/patch.png" "$TMP/fc_uint.v"
"$VIPS" cast "$TMP/fc_uint.v" "$CONVOL/fastcor_expected.v" float

# --- Provenance (append the convolution section) -----------------------------
echo "==> [provenance] appending convolution section to $FIX_ROOT/PROVENANCE.md"
# MATRIX FAMILY (the Wave-2 matrix lane; OP_MAP.md matrix section).
#
# The same committed matrix FILES feed BOTH this generator (to make the vips
# references) and tests/cli_matrix_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. Both ops are oracle class BOUNDED-TOL:
# the core computes in f64 but STORES results as f32 (libvips stores double), so
# the vips double result is CAST TO FLOAT (`vips cast … float`) before it is
# committed — the libviprs `.v` decoder rejects a DOUBLE band format and the
# compare harness reads f32 samples. The core is a faithful port of libvips'
# matrixinvert.c / invertlut.c (identical operation order), so the measured
# max-abs-diff is 0 for matrixinvert (both the direct and PLU paths) and 5.96e-8
# — one f32 ULP at 1.0 — for invertlut (whose tail extrapolates to 1.0). The
# OP_MAP's 1e-9 tol assumed a double carrier and is unreachable with f32; the
# honest measured f32 tol (1e-6) is used in the test.
#
# Inputs are DISCRIMINATING (a no-op / identity op FAILS): matrixinvert of a
# non-identity 3x3 (direct cofactor path, n<4) and 4x4 (PLU decomposition path,
# n>=4) yields a visibly different inverse; invertlut of a 3x3 measured-points
# matrix yields a 256x1 (or 64x1) LUT an identity op would mismatch on shape.
# ===========================================================================
MATRIX="$FIX_ROOT/matrix"
mkdir -p "$MATRIX"

# --- Common inputs (vips text-matrix files, header `width height`) -----------
# A .mat is a pure text file, deterministic and consumed by both sides. No scale
# / offset header (default 1 / 0): matrixinvert / invertlut read the raw cell
# values, so both vips and the libviprs MatFile loader see the same matrix.
echo "==> [matrix input] m3 (3x3, direct path), m4 (4x4, PLU path), lut (3x3 measured points)"
printf '3 3\n2 0 1\n1 3 0\n0 1 4\n'            > "$MATRIX/m3.mat"
printf '4 4\n2 1 0.5 0\n1 3 0 1\n0 1 4 2\n1 0 2 5\n' > "$MATRIX/m4.mat"
printf '3 3\n0.1 0.2 0.3\n0.2 0.4 0.4\n0.7 0.5 0.6\n' > "$MATRIX/lut.mat"

# --- References — one vips run per differential case, cast double -> float ---
echo "==> [matrixinvert] 3x3 direct + 4x4 PLU -> float .v (BOUNDED-TOL, measured 0)"
"$VIPS" matrixinvert "$MATRIX/m3.mat" "$TMP/mi3.v"
"$VIPS" cast "$TMP/mi3.v" "$MATRIX/matrixinvert3_expected.v" float
"$VIPS" matrixinvert "$MATRIX/m4.mat" "$TMP/mi4.v"
"$VIPS" cast "$TMP/mi4.v" "$MATRIX/matrixinvert4_expected.v" float

echo "==> [invertlut] default size 256 + --size 64 -> float .v (BOUNDED-TOL, 1 f32 ULP)"
"$VIPS" invertlut "$MATRIX/lut.mat" "$TMP/il.v"
"$VIPS" cast "$TMP/il.v" "$MATRIX/invertlut_expected.v" float
"$VIPS" invertlut "$MATRIX/lut.mat" "$TMP/il64.v" --size 64
"$VIPS" cast "$TMP/il64.v" "$MATRIX/invertlut_size64_expected.v" float

# --- Provenance (append the matrix section) ----------------------------------
echo "==> [provenance] appending matrix section to $FIX_ROOT/PROVENANCE.md"
# COLOUR FAMILY (the Wave-2 colour lane; OP_MAP.md colour section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_colour_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. Every colour op outputs a NON-RGB
# interpretation (LAB/XYZ/scRGB float, a float ΔE, or a re-profiled device
# image), so every case is oracle class BOUNDED-TOL at a MEASURED tolerance —
# EXCEPT `dECMC`, which is GOLDEN-ONLY: the core computes the published CMC(1:1)
# ΔE while vips approximates dECMC as Euclidean distance in its CMC uniform
# space, a DIFFERENT formula (measured max-abs-diff ~297; vips range [13,311] vs
# core [8,100]) — there is no cross-oracle, so its reference is generated by
# `viprs` itself (deterministic) as a regression pin.
#
# Carriers: float LAB/XYZ/scRGB and the float ΔE go to `.v`; an sRGB / device
# uchar target goes to PNG (the integer sink runs the interpretation-aware
# `→ sRGB` conversion in io::save — libviprs-cli #36, so `colourspace … lab`
# written to PNG matches vips's own LAB→sRGB pngsave).
#
# ICC caveat (OP_MAP colour notes): libviprs ships a native moxcms ICC engine
# while homebrew vips uses lcms2. The two agree on MATRIX-SHAPER RGB profiles
# (sRGB) — the device-space round trips (icc_export, icc_transform) match vips
# EXACTLY (measured 0) and the Lab PCS (icc_import) agrees to ~0.31 Lab units
# (moxcms-vs-lcms matrix-shaper divergence, a measured BOUNDED-TOL, NOT a bug).
# CMYK / LUT profiles diverge by design and are out of scope here.
# ===========================================================================
COLOUR="$FIX_ROOT/colour"
mkdir -p "$COLOUR"

# Ensure the viprs binary exists for the GOLDEN-ONLY dECMC pin.
# HISTOGRAM FAMILY (per-family Wave-2 lane; OP_MAP.md histogram section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_histogram_diff.rs (which feeds them to `viprs`).
#
# Format note (the whole family): vips writes histogram COUNTS as `uint`, but
# `PixelFormat` has no depth wider than 16 bits, so the core writes `ushort`
# (saturating at 65535). Every committed count reference is therefore CAST to
# ushort (a lossless narrowing while no count saturates on these tiny inputs),
# after which the two agree bit-for-bit (EXACT, tol 0). Count / LUT / histogram
# outputs are carried as the native `.v` container — vips tags them `histogram`,
# which pngsave has no colour route for (the `identity` situation); plain b-w
# images (`hist_equal`, `maplut`) go to PNG.
#
# Oracle classes (MEASURED — not hidden by input choice):
#   EXACT       hist_find (+ --band 0, full-range), hist_find_indexed,
#               hist_find_ndim, hist_cum, maplut, hist_ismonotonic (bool).
#   BOUNDED-TOL hist_norm (≤1 LSB), hist_equal (≤1 LSB), hist_entropy (float
#               scalar, rel eps 1e-6).
#   GOLDEN-ONLY hist_match, hist_plot, hist_local, percent — the core genuinely
#               diverges from vips (hist_match uint-vs-uchar LUT diff 254;
#               hist_plot height max+1 vs max; hist_local window/border algo
#               diff 51 at 5x5, CLAHE 60; percent definition differs, core =
#               vips-2 on a dense ramp). Their references are minted by `viprs`
#               itself (deterministic) and the tests are regression pins, NOT
#               parity claims. See the wave report's open questions.
# ===========================================================================
HIST="$FIX_ROOT/histogram"
mkdir -p "$HIST"

# `viprs` mints the GOLDEN-ONLY references (no vips oracle); build it if absent.
# MOSAICING FAMILY (Wave-2 mosaicing lane; OP_MAP.md mosaicing section).
#
# The same committed inputs feed BOTH this generator (to make the references)
# and tests/cli_mosaicing_diff.rs (which feeds them to `viprs`). `merge` and
# `mosaic` are oracle class EXACT (decode-compare tol 0): their integer feather
# ramp (`merge`) and DISCRETE tie-point search (`mosaic`) must agree with vips
# bit-for-bit or the whole output diverges — the differential doubles as the pin
# on that agreement (empirically verified max-abs-diff 0 on these fixtures).
# `globalbalance` is GOLDEN-ONLY: the core reads the `mosaic-join-tree` metadata
# blob that only viprs merge/mosaic outputs carry, while vips's globalbalance
# reads its own filename-based image history — the two channels are mutually
# unreadable, so there is NO vips cross-oracle. Its input (`balance_input.v`,
# carrying the blob) is minted by `viprs merge`, and its reference
# (`balance_expected.v`) by `viprs globalbalance` — a deterministic regression
# pin (like smartcrop-entropy and autorot).
#
# Carriers: `merge`/`mosaic` outputs are 1-band uchar (b-w) → PNG (round-trips).
# `globalbalance` output is always float → the native `.v` container.
#
# vips's CLI parses a bare leading `-` as an option, so a negative `merge` DX/DY
# is passed after a `--` separator (`vips merge … horizontal -- -28 0`); `viprs`
# accepts the negative positional directly (clap `allow_negative_numbers`). Both
# sides therefore see the SAME displacement.
#
# `mosaic` needs a LARGE overlap: its tie-point search splits the overlap into 3
# strips and requires 20 high-contrast 11x11 windows PER strip, so the inputs are
# ~100x150 (much bigger than the §7 "≤64x64 canonical" guideline — an inherent
# property of the op, not a fixture-design choice). `vips gaussnoise --seed` is a
# deterministic high-contrast texture; the committed crops are cut from ONE noise
# scene so ref/sec share a pixel-exact overlap the correlation locks onto.
# ===========================================================================
MOS="$FIX_ROOT/mosaicing"
mkdir -p "$MOS"

# --- merge inputs: two DIFFERENT 40x32 textures (distinct seeds) so the seam
#     blend is non-vacuous (a no-blend paste would diverge). ------------------
echo "==> [mosaicing input] merge_ref / merge_sec (40x32 gaussnoise, distinct seeds)"
"$VIPS" gaussnoise "$TMP/mg1.v" 40 32 --seed 1 --mean 140 --sigma 45
"$VIPS" cast "$TMP/mg1.v" "$MOS/merge_ref.png" uchar
"$VIPS" gaussnoise "$TMP/mg2.v" 40 32 --seed 2 --mean 130 --sigma 45
"$VIPS" cast "$TMP/mg2.v" "$MOS/merge_sec.png" uchar
# A 3-band RGB (same geometry) for the format-mismatch error case AND — reused —
# as the REF of the RGB (multi-band) merge case (adversarial-review finding 1:
# the committed merge/mosaic fixtures were all single-band Gray8, leaving the
# multi-band render_merge band-handling path unpinned).
echo "==> [mosaicing input] merge_rgb (40x32 sRGB) — mismatch SEC + RGB-merge REF"
"$VIPS" gaussnoise "$TMP/mg3a.v" 40 32 --seed 5 --mean 128 --sigma 45
"$VIPS" gaussnoise "$TMP/mg3b.v" 40 32 --seed 6 --mean 128 --sigma 45
"$VIPS" gaussnoise "$TMP/mg3c.v" 40 32 --seed 7 --mean 128 --sigma 45
"$VIPS" cast "$TMP/mg3a.v" "$TMP/mg3a.png" uchar
"$VIPS" cast "$TMP/mg3b.v" "$TMP/mg3b.png" uchar
"$VIPS" cast "$TMP/mg3c.v" "$TMP/mg3c.png" uchar
"$VIPS" bandjoin "$TMP/mg3a.png $TMP/mg3b.png $TMP/mg3c.png" "$TMP/mrgb.v"
"$VIPS" copy "$TMP/mrgb.v" "$MOS/merge_rgb.png" --interpretation srgb
# A second, distinct-seed 40x32 sRGB image = the SEC of the RGB merge case.
echo "==> [mosaicing input] merge_rgb_sec (40x32 sRGB, distinct seeds) — RGB-merge SEC"
"$VIPS" gaussnoise "$TMP/ms3a.v" 40 32 --seed 31 --mean 120 --sigma 45
"$VIPS" gaussnoise "$TMP/ms3b.v" 40 32 --seed 32 --mean 135 --sigma 45
"$VIPS" gaussnoise "$TMP/ms3c.v" 40 32 --seed 33 --mean 145 --sigma 45
"$VIPS" cast "$TMP/ms3a.v" "$TMP/ms3a.png" uchar
"$VIPS" cast "$TMP/ms3b.v" "$TMP/ms3b.png" uchar
"$VIPS" cast "$TMP/ms3c.v" "$TMP/ms3c.png" uchar
"$VIPS" bandjoin "$TMP/ms3a.png $TMP/ms3b.png $TMP/ms3c.png" "$TMP/msrgb.v"
"$VIPS" copy "$TMP/msrgb.v" "$MOS/merge_rgb_sec.png" --interpretation srgb

# --- mosaic HORIZONTAL inputs: two 100x150 crops of a 110x150 noise scene, sec
#     offset +10 in x (overlap 90x150, exceeds the 3-strip search geometry). ---
echo "==> [mosaicing input] mosaic_h_ref / mosaic_h_sec (100x150 crops of a 110x150 scene, x-offset 10)"
"$VIPS" gaussnoise "$TMP/mbh.v" 110 150 --seed 7 --mean 128 --sigma 60
"$VIPS" cast "$TMP/mbh.v" "$TMP/mbh.png" uchar
"$VIPS" extract_area "$TMP/mbh.png" "$MOS/mosaic_h_ref.png" 0 0 100 150
"$VIPS" extract_area "$TMP/mbh.png" "$MOS/mosaic_h_sec.png" 10 0 100 150

# --- mosaic VERTICAL inputs: two 150x100 crops of a 150x110 noise scene, sec
#     offset +10 in y (the tb-search path, distinct from the lr path). --------
echo "==> [mosaicing input] mosaic_v_ref / mosaic_v_sec (150x100 crops of a 150x110 scene, y-offset 10)"
"$VIPS" gaussnoise "$TMP/mbv.v" 150 110 --seed 11 --mean 128 --sigma 60
"$VIPS" cast "$TMP/mbv.v" "$TMP/mbv.png" uchar
"$VIPS" extract_area "$TMP/mbv.png" "$MOS/mosaic_v_ref.png" 0 0 150 100
"$VIPS" extract_area "$TMP/mbv.png" "$MOS/mosaic_v_sec.png" 0 10 150 100

# --- References — merge (EXACT). Negative DX/DY after a `--` separator. -------
echo "==> [merge] horizontal (dx -28) + vertical (dy -22) (EXACT, Gray8 PNG)"
"$VIPS" merge "$MOS/merge_ref.png" "$MOS/merge_sec.png" "$MOS/merge_h_expected.png" horizontal -- -28 0
"$VIPS" merge "$MOS/merge_ref.png" "$MOS/merge_sec.png" "$MOS/merge_v_expected.png" vertical   -- 0 -22

# --- References — merge, multi-band + insert-fallback paths (EXACT; finding 1).
# RGB (3-band) horizontal merge pins the multi-band render_merge band-handling
# path the single-band Gray8 fixtures left unprotected (verified max-abs-diff 0).
echo "==> [merge] RGB 3-band horizontal (dx -28) (EXACT, sRGB PNG)"
"$VIPS" merge "$MOS/merge_rgb.png" "$MOS/merge_rgb_sec.png" "$MOS/merge_rgb_expected.png" horizontal -- -28 0
# A POSITIVE dx (dx>0) takes the wrong-side/disjoint INSERT-FALLBACK branch
# (`merge_impl`: paste both, no blend, output sized by `rarea.union(&sarea)`) —
# a distinct code path from the feathered blend, previously unpinned. vips falls
# back to vips_insert identically here (verified max-abs-diff 0, 52x32 union).
echo "==> [merge] insert-fallback, positive dx 12 (EXACT, Gray8 PNG)"
"$VIPS" merge "$MOS/merge_ref.png" "$MOS/merge_sec.png" "$MOS/merge_fallback_expected.png" horizontal 12 0

# --- References — mosaic (EXACT). Tie-point = the true correspondence; the full
#     discrete search still runs and must agree with vips bit-for-bit. ---------
echo "==> [mosaic] horizontal + vertical tie-point search (EXACT, Gray8 PNG)"
# horizontal: sec(40,75) == scene(50,75) == ref(50,75).
"$VIPS" mosaic "$MOS/mosaic_h_ref.png" "$MOS/mosaic_h_sec.png" "$MOS/mosaic_h_expected.png" \
    horizontal 50 75 40 75
# vertical: sec(75,40) == scene(75,50) == ref(75,50).
"$VIPS" mosaic "$MOS/mosaic_v_ref.png" "$MOS/mosaic_v_sec.png" "$MOS/mosaic_v_expected.png" \
    vertical 75 50 75 40

# --- References — globalbalance (GOLDEN-ONLY, viprs pipeline; no vips oracle). -
# The input is a viprs `merge` of two 60x50 textures with DIFFERENT brightness
# (mean 100 vs 160) and a 20-column overlap (20x50 = 1000 px > the 20x20=400
# significance threshold), so the balance factors are non-trivial. It carries the
# join-tree blob only viprs writes, and the blob survives the `.v` JSON trailer.
echo "==> [globalbalance] viprs merge -> viprs globalbalance pipeline pin (GOLDEN-ONLY .v)"
# CREATE FAMILY (the Wave-2 create lane; OP_MAP.md create section).
#
# Creators are §3 shape S5 (OUT first, no image input) except `buildlut` (S1: a
# matrix `.mat` file input). Oracle classes are MIXED per OP_MAP.md
# (CLI_CONTRACT.md §5):
#   * EXACT (tol 0): black, xyz.
#   * BOUNDED-TOL (float trig/exp → f32 carrier): eye, zone, sines, tonelut,
#     buildlut, sdf, and every mask_* op. Measured max-abs-diff on these inputs is
#     0 for every mask, buildlut and tonelut, and <= 4e-6 for eye/zone/sines/sdf
#     (f32 trig/hypot rounding) — a documented f32 bound, not a CLI bug.
#   * GOLDEN-ONLY (no vips oracle — PRNG / Pango differs even seeded): gaussnoise,
#     perlin, worley, fractsurf, text. Their references are generated by `viprs`
#     itself (deterministic across runs) and the tests are regression pins.
#
# Carriers (CLI_CONTRACT.md §2): float creators (xyz, eye, zone, sines, buildlut,
# masks, sdf, the noise ops) → the native `.v` container the libviprs decoder
# reads back losslessly; `--uchar` variants → PNG; `tonelut` is a Gray16 curve →
# `.v`. vips emits `uint` for `xyz` and `double` for `buildlut`, band formats the
# libviprs `.v` decoder does not read (it supports only uchar/ushort/float), so
# BOTH are cast to `float` before the reference `.v` is written — the values are
# integer coordinates / f32-exact LUT entries, so the cast is lossless.
# ===========================================================================
CREATE="$FIX_ROOT/create"
mkdir -p "$CREATE"

# Ensure the viprs binary exists for the GOLDEN-ONLY pins (built here, not by
# vips). Mirrors the extract/autorot blocks.
# DRAW FAMILY (the Wave-2 draw lane; OP_MAP.md draw section).
#
# EVERY draw op is GOLDEN-ONLY: `vips draw_*` are in-place mutators whose CLI
# DISCARDS the mutated image (the op returns nothing to save), so there is NO
# vips CLI oracle to cross-check against. Each reference is therefore generated
# ONCE by `viprs` itself (deterministic) and committed; the differential cell
# (tests/cli_draw_diff.rs) is a REGRESSION PIN that says so, NOT a parity claim.
#
# The COMMON inputs are still built with vips (a deterministic function of pixel
# coordinates — `grey`/`eye`/`black` — so every input is bit-reproducible) and
# committed; they are consumed only as `viprs` INPUTS (never as an oracle). The
# inputs are chosen so each draw is DISCRIMINATING: a no-op / broken draw would
# fail the pin (the cell also asserts each output differs from its input, and
# that outline≠fill and bounded-flood≠blob-flood).
# ===========================================================================
DRAW="$FIX_ROOT/draw"
mkdir -p "$DRAW"

# `viprs` mints EVERY draw reference (there is no vips oracle). Build it if the
# release binary is absent, exactly as the smartcrop-entropy golden pin does.
if [ ! -x "$VIPRS" ]; then
    echo "    (building $VIPRS: cargo build --release --no-default-features --bin viprs)"
    ( cd "$CLI_DIR" && cargo build --release --no-default-features --bin viprs )
fi

# --- Common inputs -----------------------------------------------------------
# Two DISTINCT 16x16 sRGB images with TRUE 2-D structure (band0 horizontal ramp,
# band1 scaled horizontal ramp, band2 VERTICAL ramp), re-tagged sRGB so a 3-band
# PNG saves as clean RGB. `vips grey`/`rot` are pure coordinate functions, so
# every input is bit-reproducible. rgb2 is distinct from rgb so the ΔE metrics
# are NON-VACUOUS (a==b would give ΔE≡0).
echo "==> [colour input] two 16x16 sRGB images (distinct, 2-D structure)"
"$VIPS" grey "$TMP/clg.v" 16 16
"$VIPS" rot "$TMP/clg.v" "$TMP/clg_v.v" d90
"$VIPS" linear "$TMP/clg.v"   "$TMP/cl_b0.png" 255 0  --uchar
"$VIPS" linear "$TMP/clg.v"   "$TMP/cl_b1.png" 200 20 --uchar
"$VIPS" linear "$TMP/clg_v.v" "$TMP/cl_b2.png" 255 0  --uchar
"$VIPS" bandjoin "$TMP/cl_b0.png $TMP/cl_b1.png $TMP/cl_b2.png" "$TMP/cl_rgb.v"
"$VIPS" copy "$TMP/cl_rgb.v" "$COLOUR/rgb.png" --interpretation srgb
"$VIPS" linear "$TMP/clg.v"   "$TMP/cl_c0.png" 150 40 --uchar
"$VIPS" linear "$TMP/clg_v.v" "$TMP/cl_c1.png" 150 50 --uchar
"$VIPS" linear "$TMP/clg.v"   "$TMP/cl_c2.png" 120 60 --uchar
"$VIPS" bandjoin "$TMP/cl_c0.png $TMP/cl_c1.png $TMP/cl_c2.png" "$TMP/cl_rgb2.v"
"$VIPS" copy "$TMP/cl_rgb2.v" "$COLOUR/rgb2.png" --interpretation srgb

# sRGB MATRIX-SHAPER profile (v2.1, rXYZ/gXYZ/bXYZ colorants + rTRC/gTRC/bTRC
# tone curves, NO A2B LUT) — both moxcms and lcms2 evaluate it on the exact
# matrix-shaper path. Copied from the author-Mac ColorSync store.
echo "==> [colour input] sRGB matrix-shaper ICC profile"
SRGB_SRC="/System/Library/ColorSync/Profiles/sRGB Profile.icc"
if [ -f "$SRGB_SRC" ]; then
    cp "$SRGB_SRC" "$COLOUR/sRGB.icc"
else
    echo "    WARNING: $SRGB_SRC not found; kept the committed colour/sRGB.icc" >&2
fi

# Lab PCS input for icc_export: import rgb.png through the sRGB profile with vips
# (a genuine D50 Lab PCS image). Committed and fed to BOTH sides of the export
# differential, so the export route is compared like-for-like.
echo "==> [colour input] icc_pcs_lab.v (vips icc_import of rgb.png through sRGB)"
"$VIPS" icc_import "$COLOUR/rgb.png" "$COLOUR/icc_pcs_lab.v" \
    --input-profile "$COLOUR/sRGB.icc" --intent relative

# --- References — one vips run per differential case -------------------------
# colourspace: sRGB -> LAB / XYZ / scRGB, real round-trips through the D65 XYZ
# hub (float .v, BOUNDED-TOL eps 1e-4; measured LAB 4.6e-5, XYZ 1.5e-5,
# scRGB 1e-6).
echo "==> [colourspace] srgb -> lab / xyz / scrgb (.v float, eps 1e-4)"
"$VIPS" colourspace "$COLOUR/rgb.png" "$COLOUR/colourspace_lab_expected.v"   lab
"$VIPS" colourspace "$COLOUR/rgb.png" "$COLOUR/colourspace_xyz_expected.v"   xyz
"$VIPS" colourspace "$COLOUR/rgb.png" "$COLOUR/colourspace_scrgb_expected.v" scrgb

# #36 interpretation-aware PNG save: `colourspace … lab` written to an INTEGER
# sink. vips's pngsave converts the LAB result to sRGB before encoding; io::save
# must do the same (not cast the raw Lab channels). uchar, ≤1 LSB (measured 0).
echo "==> [colourspace] srgb -> lab written to PNG (#36 interp-aware save; ≤1 LSB)"
"$VIPS" colourspace "$COLOUR/rgb.png" "$COLOUR/colourspace_lab_png_expected.png" lab

# #36 discriminator strengthener: a GENUINELY Lab-tagged input (icc_pcs_lab.v, a
# D50 Lab PCS image) converted to sRGB and written to PNG. Unlike the round-trip
# above (whose reference equals rgb.png, so an identity colourspace would still
# pass), here the reference DIFFERS from the input — a no-op / raw-cast
# colourspace would garble it — so the PNG path discriminates the colourspace
# transform itself, not only the interpretation-aware save. uchar, ≤1 LSB
# (measured 1).
echo "==> [colourspace] LAB-tagged input -> srgb PNG (non-round-trip discriminator; ≤1 LSB)"
"$VIPS" colourspace "$COLOUR/icc_pcs_lab.v" "$COLOUR/colourspace_lab_input_png_expected.png" srgb

# --source-space override: force the sRGB-tagged input to be read as LAB, then
# convert to sRGB. Genuinely discriminating (vs the srgb->srgb identity the flag
# would collapse to if ignored: 255 apart). uchar, ≤1 LSB (measured 1).
echo "==> [colourspace] --source-space lab override -> srgb PNG (≤1 LSB)"
"$VIPS" colourspace "$COLOUR/rgb.png" "$COLOUR/colourspace_srcspace_expected.png" srgb \
    --source-space lab

# dE76 / dE00: two distinct sRGB inputs -> float ΔE (.v, eps 1e-4; measured
# 6.5e-5). dE00 pins the libvips vips_col_dE00 hue-wrap parity.
echo "==> [dE76/dE00] rgb vs rgb2 -> float ΔE (.v, eps 1e-4)"
"$VIPS" dE76 "$COLOUR/rgb.png" "$COLOUR/rgb2.png" "$COLOUR/dE76_expected.v"
"$VIPS" dE00 "$COLOUR/rgb.png" "$COLOUR/rgb2.png" "$COLOUR/dE00_expected.v"

# dECMC: GOLDEN-ONLY. vips computes Euclidean distance in its CMC uniform space;
# the core computes the published CMC(1:1) ΔE — a DIFFERENT formula (measured
# max-abs-diff ~297). No cross-oracle: the reference is a viprs regression pin.
echo "==> [dECMC] rgb vs rgb2 -> float ΔE (GOLDEN-ONLY viprs pin — no vips oracle)"
"$VIPRS" dECMC "$COLOUR/rgb.png" "$COLOUR/rgb2.png" "$COLOUR/dECMC_golden.v"

# icc_import: sRGB device -> D50 Lab PCS (.v float). BOUNDED-TOL at the measured
# moxcms-vs-lcms matrix-shaper divergence (~0.31 Lab units).
echo "==> [icc_import] rgb through sRGB -> Lab PCS (.v, BOUNDED-TOL ~0.31)"
"$VIPS" icc_import "$COLOUR/rgb.png" "$COLOUR/icc_import_lab_expected.v" \
    --input-profile "$COLOUR/sRGB.icc" --intent relative

# icc_export: Lab PCS -> sRGB device (PNG uchar). Matrix-shaper round trip
# matches vips EXACTLY (measured 0); compared at ≤2 LSB.
echo "==> [icc_export] Lab PCS through sRGB -> device PNG (≤2 LSB, measured 0)"
"$VIPS" icc_export "$COLOUR/icc_pcs_lab.v" "$COLOUR/icc_export_expected.png" \
    --output-profile "$COLOUR/sRGB.icc" --intent relative --depth 8

# icc_export --depth 16: pin the 16-bit device-output path. At 16-bit precision
# the native moxcms engine and vips's lcms2 diverge by ~13/65535 on the matrix
# -shaper sRGB profile (the 8-bit case rounds that away to 0) — a real, measured
# cross-CMS BOUNDED-TOL, NOT a bug. 16-bit PNG, compared at ≤16 LSB (measured 13).
echo "==> [icc_export] Lab PCS through sRGB -> 16-bit device PNG (--depth 16; ≤16 LSB, measured 13)"
"$VIPS" icc_export "$COLOUR/icc_pcs_lab.v" "$COLOUR/icc_export_d16_expected.png" \
    --output-profile "$COLOUR/sRGB.icc" --intent relative --depth 16

# icc_transform: sRGB device -> sRGB device in one step (import+export). Matrix
# -shaper round trip matches vips EXACTLY (measured 0); compared at ≤2 LSB.
echo "==> [icc_transform] rgb sRGB->sRGB round trip -> device PNG (≤2 LSB, measured 0)"
"$VIPS" icc_transform "$COLOUR/rgb.png" "$COLOUR/icc_transform_expected.png" \
    "$COLOUR/sRGB.icc" --input-profile "$COLOUR/sRGB.icc" --intent relative

# --- Provenance (append the colour section) ----------------------------------
echo "==> [provenance] appending colour section to $FIX_ROOT/PROVENANCE.md"
# --- Common image inputs -----------------------------------------------------
# `vips grey` is a pure coordinate function (0..1 ramp), so every input is
# bit-reproducible. gray2/gray3 stay in TMP (only rgb + the histogram derivations
# use them); gray/rgb/index are the committed image inputs.
echo "==> [histogram input] gray (16x16 Gray8) + rgb (srgb 3-band) + index (0..3)"
"$VIPS" grey "$TMP/hg.v" 16 16
"$VIPS" linear "$TMP/hg.v" "$HIST/gray.png" 255 0 --uchar
"$VIPS" linear "$TMP/hg.v" "$TMP/hg2.png" 200 10 --uchar
"$VIPS" rot "$TMP/hg.v" "$TMP/hg_v.v" d90
# band 2: a 2-D DIAGONAL gradient (x+y), scaled so the far corner reaches 255.
# A plain VERTICAL ramp (rot d90 of the horizontal ramp) reaches 255 too, but its
# value histogram is IDENTICAL to band 0's uniform ramp histogram — so a
# "always reads band 0" bug would still pass a `--band 2` case (the band index is
# never actually exercised). The diagonal has a TRIANGULAR value histogram (31
# nonzero bins, counts 1..16..1) that is provably DISTINCT from band 0's (16 bins,
# each count 16), so `hist_find --band 2` genuinely verifies band-index honouring.
# It still reaches 255 (round((x+y)*127.5)=255 at the corner), so vips does not
# trailing-zero-trim and the case stays a genuine EXACT (width 256 both sides).
"$VIPS" add "$TMP/hg.v" "$TMP/hg_v.v" "$TMP/hg_diag.v"
"$VIPS" linear "$TMP/hg_diag.v" "$TMP/hg3.png" 127.5 0 --uchar
"$VIPS" bandjoin "$HIST/gray.png $TMP/hg2.png $TMP/hg3.png" "$TMP/hrgb.v"
"$VIPS" copy "$TMP/hrgb.v" "$HIST/rgb.png" --interpretation srgb
# index: a 4-level (0..3) image so hist_find_indexed sums into distinct bins.
"$VIPS" linear "$TMP/hg.v" "$TMP/hidxf.v" 3 0
"$VIPS" cast "$TMP/hidxf.v" "$HIST/index.png" uchar

# --- Committed histogram-shaped inputs (consumed by histogram-consuming ops) --
# Cast the vips uint histograms to ushort so both sides read the SAME committed
# file at the core's carrier depth. `lut.v` is a real (non-identity) equalisation
# LUT: norm ∘ cum ∘ find, so `maplut` through it is NON-vacuous.
echo "==> [histogram input] hist.v / histcum.v / hist2.v (ushort) + lut.v (uchar)"
"$VIPS" hist_find "$HIST/gray.png" "$TMP/hf_u.v"; "$VIPS" cast "$TMP/hf_u.v" "$HIST/hist.v" ushort
"$VIPS" hist_cum "$HIST/hist.v" "$TMP/hc_u.v";    "$VIPS" cast "$TMP/hc_u.v" "$HIST/histcum.v" ushort
"$VIPS" hist_find "$TMP/hg2.png" "$TMP/hf2_u.v";  "$VIPS" cast "$TMP/hf2_u.v" "$HIST/hist2.v" ushort
"$VIPS" hist_norm "$HIST/histcum.v" "$HIST/lut.v"

# --- EXACT references (vips count outputs cast to ushort) ---------------------
# NOTE: vips hist_find TRIMS trailing all-zero bins (output width = max sample
# value + 1) while the core always writes the full 256-bin range. They match
# bit-for-bit only when the histogrammed data reaches 255 (no trailing zeros to
# trim). The full-range gray ramp (0..255) satisfies this for both the all-bands
# case and the `--band 0` case (band 0 IS the gray ramp), so the count
# comparison is a genuine EXACT — the trailing-zero-trim width difference is a
# representational nicety, not a count divergence. A band whose max < 255 (e.g.
# band 1, gray2 max 210 → width 211) would mismatch on width alone; see the wave
# report's open question. The `--band 2` case pins band-index honouring: band 2
# is the DIAGONAL gradient (distinct triangular histogram from band 0), so a bug
# that ignored N and always read band 0 would fail here (band 0 and band 2 both
# reach 255, so neither is trailing-zero-trimmed — both are genuine EXACT).
echo "==> [hist_find] gray (all bands) + rgb --band 0 + rgb --band 2 (full 0..255 range) -> ushort .v (EXACT)"
"$VIPS" hist_find "$HIST/gray.png" "$TMP/rf.v";  "$VIPS" cast "$TMP/rf.v"  "$HIST/hist_find_expected.v" ushort
"$VIPS" hist_find "$HIST/rgb.png" "$TMP/rfb.v" --band 0
"$VIPS" cast "$TMP/rfb.v" "$HIST/hist_find_band_expected.v" ushort
"$VIPS" hist_find "$HIST/rgb.png" "$TMP/rfb2.v" --band 2
"$VIPS" cast "$TMP/rfb2.v" "$HIST/hist_find_band2_expected.v" ushort

echo "==> [hist_find_indexed] gray + index -> ushort .v (EXACT)"
"$VIPS" hist_find_indexed "$HIST/gray.png" "$HIST/index.png" "$TMP/ri.v"
"$VIPS" cast "$TMP/ri.v" "$HIST/hist_find_indexed_expected.v" ushort

echo "==> [hist_find_ndim] rgb --bins 4 -> 4x4x4 ushort .v (EXACT)"
"$VIPS" hist_find_ndim "$HIST/rgb.png" "$TMP/rn.v" --bins 4
"$VIPS" cast "$TMP/rn.v" "$HIST/hist_find_ndim_expected.v" ushort

echo "==> [hist_cum] hist.v -> ushort .v (EXACT)"
"$VIPS" hist_cum "$HIST/hist.v" "$TMP/rc.v"; "$VIPS" cast "$TMP/rc.v" "$HIST/hist_cum_expected.v" ushort

# hist_norm is BOUNDED-TOL (≤1 LSB), NOT EXACT as OP_MAP.md provisionally listed:
# normalising the CUMULATIVE histogram rounds `(v * (n-1) / max)` independently
# in the core and in vips, and the two land ±1 apart on some entries (a measured
# core-vs-vips rounding difference; norming the RAW histogram happens to give 0,
# but we do NOT switch inputs to fake EXACT — see the wave report open question).
echo "==> [hist_norm] histcum.v -> uchar .v (BOUNDED-TOL <=1 LSB)"
"$VIPS" hist_norm "$HIST/histcum.v" "$HIST/hist_norm_expected.v"

echo "==> [hist_equal] gray -> uchar PNG (BOUNDED-TOL <=1 LSB)"
"$VIPS" hist_equal "$HIST/gray.png" "$HIST/hist_equal_expected.png"

echo "==> [maplut] gray through the equalisation lut -> uchar PNG (EXACT)"
"$VIPS" maplut "$HIST/gray.png" "$HIST/maplut_expected.png" "$HIST/lut.v"

echo "==> [hist_entropy] hist.v (uniform, log2 16 = 4) + histcum.v (7.76335) (S3, rel eps 1e-6)"
"$VIPS" hist_entropy "$HIST/hist.v"    > "$HIST/hist_entropy_expected.txt"
"$VIPS" hist_entropy "$HIST/histcum.v" > "$HIST/hist_entropy_cum_expected.txt"

echo "==> [hist_ismonotonic] hist.v (FALSE) + histcum.v (TRUE) (S3, bool)"
"$VIPS" hist_ismonotonic "$HIST/hist.v"    > "$HIST/hist_ismonotonic_false_expected.txt"
"$VIPS" hist_ismonotonic "$HIST/histcum.v" > "$HIST/hist_ismonotonic_true_expected.txt"

# --- GOLDEN-ONLY references (viprs pins — NO vips oracle) ---------------------
echo "==> [hist_match/hist_plot/hist_local/percent] GOLDEN-ONLY viprs regression pins"
"$VIPRS" hist_match "$HIST/hist.v" "$HIST/hist2.v" "$HIST/hist_match_golden.v"
"$VIPRS" hist_plot  "$HIST/hist.v" "$HIST/hist_plot_golden.v"
"$VIPRS" hist_local "$HIST/gray.png" "$HIST/hist_local_golden.png" 5 5
"$VIPRS" hist_local "$HIST/gray.png" "$HIST/hist_local_clahe_golden.png" 5 5 --max-slope 3
"$VIPRS" percent "$HIST/gray.png" 50 > "$HIST/percent_golden.txt"

# --- Provenance (append the histogram section) -------------------------------
echo "==> [provenance] appending histogram section to $FIX_ROOT/PROVENANCE.md"
# COMPOSITE FAMILY (the Wave-2 composite lane; OP_MAP.md composite section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_composite_diff.rs (which feeds them to `viprs`), so
# the two sides compare like against like. The core op (`try_composite2`) blends
# exactly two images with one blend mode; both vips nicknames (`composite` array
# form, `composite2` pair form) map onto it. Oracle class BOUNDED-TOL ≤1 LSB
# (the blend is done premultiplied in f64 then rounded to the integer container).
#
# HONEST ORACLE SPLIT (measured against vips 8.18.4 — see the open question in
# the wave report). Compositing OPAQUE inputs, ALL 25 modes agree with vips to
# ≤1 LSB. With TRANSLUCENT alpha, only the Porter-Duff *simple* operators agree
# (clear/source/over/in/out/dest/dest-over/dest-in/dest-out/xor/add ≤1 LSB); the
# 11 PDF separable blends AND the alpha-weighted Porter-Duff operators
# atop/dest-atop/saturate diverge WHOLESALE (measured max-abs-diff up to 215),
# because the core's translucent blend-composite formula differs from libvips'.
# So:
#   * Porter-Duff simple modes are pinned on the TRANSLUCENT inputs (real vips
#     oracle, non-vacuous, arbitrary alpha) at tol 1.
#   * The PDF separable blends are pinned on OPAQUE inputs (real vips oracle,
#     non-vacuous blend-function coverage) at tol 1 — the translucent divergence
#     is NOT hidden: it is pinned separately below.
#   * The translucent divergence itself is captured as GOLDEN-ONLY `viprs`
#     regression pins (multiply/atop/saturate on the translucent inputs): there
#     is NO cross-oracle for translucent blend compositing, so the reference is
#     generated by `viprs` (deterministic) and the test is a regression pin that
#     records the measured vips divergence. This mirrors the smartcrop-entropy
#     GOLDEN-ONLY precedent (vips makes a different choice → no cross-oracle).
#
# Carriers: every input/output is uchar sRGB with ≤4 bands, so all references
# round-trip losslessly through PNG (no b-w-multiband promotion, no .v needed).
# ===========================================================================
COMP="$FIX_ROOT/composite"
mkdir -p "$COMP"

# `viprs` is used for the GOLDEN-ONLY translucent divergence pins (no vips
# oracle). Build it if absent (as the smartcrop/autorot blocks do).
VIPRS="${VIPRS:-${CLI_DIR:-$REPO_ROOT/../libviprs-cli}/target/release/viprs}"

# --- Common inputs -----------------------------------------------------------
# Two 8x8 sRGB RGBA images with TRUE 2-D structure (band0 horizontal ramp, band1
# vertical ramp, band2 a distinct scaled ramp) and a VARYING alpha (so the
# translucent compositing math is exercised at every alpha, not a single value),
# plus their opaque 3-band RGB counterparts. `vips grey`/`rot` are pure
# coordinate functions, so every fixture is bit-for-bit reproducible.
echo "==> [composite input] 8x8 sRGB RGBA base + overlay (2-D + varying alpha) + opaque RGB"
"$VIPS" grey "$TMP/pg.v" 8 8
"$VIPS" rot "$TMP/pg.v" "$TMP/pgv.v" d90
# base bands.
"$VIPS" linear "$TMP/pg.v"  "$TMP/pb0.v" 255 0  --uchar
"$VIPS" linear "$TMP/pgv.v" "$TMP/pb1.v" 255 0  --uchar
"$VIPS" linear "$TMP/pg.v"  "$TMP/pb2.v" 128 40 --uchar
"$VIPS" linear "$TMP/pgv.v" "$TMP/pba.v" 200 55 --uchar
"$VIPS" bandjoin "$TMP/pb0.v $TMP/pb1.v $TMP/pb2.v" "$TMP/pbase_op.v"
"$VIPS" copy "$TMP/pbase_op.v" "$COMP/base_op.png" --interpretation srgb
"$VIPS" bandjoin "$TMP/pb0.v $TMP/pb1.v $TMP/pb2.v $TMP/pba.v" "$TMP/pbase.v"
"$VIPS" copy "$TMP/pbase.v" "$COMP/base.png" --interpretation srgb
# overlay bands (distinct from base).
"$VIPS" linear "$TMP/pgv.v" "$TMP/po0.v" 200 20 --uchar
"$VIPS" linear "$TMP/pg.v"  "$TMP/po1.v" 150 30 --uchar
"$VIPS" linear "$TMP/pgv.v" "$TMP/po2.v" 255 0  --uchar
"$VIPS" linear "$TMP/pg.v"  "$TMP/poa.v" 180 40 --uchar
"$VIPS" bandjoin "$TMP/po0.v $TMP/po1.v $TMP/po2.v" "$TMP/pover_op.v"
"$VIPS" copy "$TMP/pover_op.v" "$COMP/overlay_op.png" --interpretation srgb
"$VIPS" bandjoin "$TMP/po0.v $TMP/po1.v $TMP/po2.v $TMP/poa.v" "$TMP/pover.v"
"$VIPS" copy "$TMP/pover.v" "$COMP/overlay.png" --interpretation srgb
# A 1-band 8x8 Gray8 image: an INPUT-only fixture for the band-count-mismatch
# error case (compositing a 1-colour-band image against a 3-colour-band one is a
# typed exit-1 BandMismatch). No reference output — it is an error case.
"$VIPS" linear "$TMP/pg.v" "$COMP/gray.png" 255 0 --uchar

# --- References — Porter-Duff simple modes on TRANSLUCENT inputs (real oracle) -
echo "==> [composite2] Porter-Duff simple modes on translucent RGBA (BOUNDED-TOL ≤1 LSB)"
"$VIPS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_over_expected.png"      over
"$VIPS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_source_expected.png"    source
"$VIPS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_in_expected.png"        in
"$VIPS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_xor_expected.png"       xor
"$VIPS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_add_expected.png"       add
"$VIPS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_dest_over_expected.png" dest-over

# --- References — composite (vips array form) on translucent inputs -----------
# vips `composite` takes an image ARRAY and an INTEGER blend-mode array; over = 2
# in VipsBlendMode. `viprs composite … over` produces the identical pixels.
echo "==> [composite] array form (over = int 2) on translucent RGBA (BOUNDED-TOL ≤1 LSB)"
"$VIPS" composite "$COMP/base.png $COMP/overlay.png" "$COMP/composite_over_expected.png" 2

# --- References — PDF separable blends on OPAQUE inputs (real oracle) ----------
# The blend FUNCTION B(cb,cs) is exercised non-vacuously here (a broken blend
# fails); opaque agrees with vips ≤1 LSB. The translucent divergence of these
# same modes is NOT hidden — it is pinned as GOLDEN-ONLY below.
echo "==> [composite2] PDF separable blends on OPAQUE RGB (BOUNDED-TOL ≤1 LSB)"
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_multiply_expected.png"     multiply
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_screen_expected.png"       screen
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_overlay_expected.png"      overlay
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_darken_expected.png"       darken
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_hardlight_expected.png"    hard-light
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_difference_expected.png"   difference
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_exclusion_expected.png"    exclusion
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_colourdodge_expected.png"  colour-dodge

# --- References — the remaining 9 modes on OPAQUE inputs (real oracle) ---------
# Every one of the 25 VipsBlendMode spellings must be discriminated against a
# real vips oracle so a mode->CompositeMode mis-wiring (a swap to a valid-but-
# wrong variant, e.g. lighten->Darken or out->In) cannot pass CI unnoticed. The
# eight PDF/Porter-Duff modes above leave nine spellings uncovered; on OPAQUE
# inputs all nine agree with vips ≤1 LSB (measured: clear/out/dest/dest-in/
# dest-out/dest-atop/lighten = 0, colour-burn/soft-light = 1 LSB), so pin them
# here as ordinary BOUNDED-TOL differentials. This closes the mode-wiring hole.
echo "==> [composite2] remaining 9 modes on OPAQUE RGB — mode-wiring coverage (BOUNDED-TOL ≤1 LSB)"
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_clear_expected.png"       clear
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_out_expected.png"         out
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_dest_expected.png"        dest
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_dest_in_expected.png"     dest-in
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_dest_out_expected.png"    dest-out
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_dest_atop_expected.png"   dest-atop
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_lighten_expected.png"     lighten
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_colourburn_expected.png"  colour-burn
"$VIPS" composite2 "$COMP/base_op.png" "$COMP/overlay_op.png" "$COMP/composite2_softlight_expected.png"   soft-light

# --- References — GOLDEN-ONLY translucent divergence pins (NO vips oracle) -----
# On the translucent inputs these modes diverge WHOLESALE from vips (measured
# max-abs-diff: multiply 38, atop 191, saturate 37 — a different translucent
# blend-composite formula, not a tolerance). There is no meaningful cross-oracle,
# so the reference is generated by `viprs` itself (deterministic) and the test is
# a regression pin. This EXPOSES the divergence rather than hiding it (tracked as
# a filed GitHub issue — see the composite-translucent-divergence issue).
#
# NOTE: if a future core change ALIGNS the translucent blend-composite formula
# with libvips, these three pins WILL fail. That is EXPECTED and correct: the fix
# is to REGENERATE these goldens (re-run this script) and re-bless them, NOT to
# revert the core change. Promote them to real vips-oracle differentials once the
# core matches libvips.
echo "==> [composite2] translucent multiply/atop/saturate GOLDEN-ONLY viprs pins (no vips oracle)"
# ARITHA FAMILY — arith part-A lane (statistics / const-linear / unary-rounding
# / hough; OP_MAP.md arithmetic section, part A rows).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_aritha_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. `vips grey`/`linear`/`black`/`embed`/
# `insert` are pure functions, so every fixture is bit-reproducible.
#
# Honest oracle classes (measured against vips 8.18.4):
#  - S3 scalars (avg/deviate/min/max/find_trim) — numeric compare; min/max are
#    integer-exact on an integer input, avg/deviate a rational mean at S3 rel-eps.
#  - stats/measure — a double matrix; vips has no f64 PNG route and libviprs no
#    f64 pixel format, so the reference is `vips … ; vips cast … float` → `.v`
#    (f32). `stats` also drops vips's 4 position columns (6..10): the core
#    computes only the first 6 (min/max/sum/sum2/mean/sd), a documented subset,
#    so the reference is cropped to 6 columns. Both measured max-abs-diff 0.
#  - profile/project — 16-bit; vips emits INT/UINT, cast to ushort (lossless for
#    the small position/sum values) to match the core's ushort carrier. `.v`.
#  - linear (float `.v` + `--uchar` PNG), remainder_const/clamp (PNG),
#    math2_const pow / abs / sign / round ceil|floor (float `.v`) — all measured 0.
#  - round rint — EXACT (tol 0): core issue #494 made the core round half TO EVEN,
#    matching vips's C `rint` at exact half-integers on the honest afloat (which
#    reaches x.5). Previously the core's `f64::round` (half away from zero)
#    diverged by 1 at every x.5 and forced a GOLDEN-ONLY pin; the reference is now
#    a genuine vips oracle (round_rint_expected.v). ceil/floor stay EXACT.
#  - HOUGH — hough_line's distance binning is now vips-exact (core issue #495
#    fixed the one-cell offset), but an INHERENT format/saturation gap remains:
#    the core accumulates into Gray16 (u16) while vips uses a uint accumulator, so
#    a peak of >65535 collinear votes saturates in the core (no u32 carrier).
#    hough_circle still diverges structurally (a different circle vote model: a
#    single point yields core max 1 vs vips max 4). Neither has a full-width vips
#    oracle, so both stay GOLDEN-ONLY viprs regression pins.
# ===========================================================================
ARITHA="$FIX_ROOT/aritha"
mkdir -p "$ARITHA"

# `viprs` is needed to mint the two GOLDEN-ONLY hough references (no vips oracle).
VIPRS="${VIPRS:-${CLI_DIR:-$REPO_ROOT/../libviprs-cli}/target/release/viprs}"
if [ ! -x "$VIPRS" ]; then
    echo "    (building $VIPRS: cargo build --release --no-default-features --bin viprs)"
    ( cd "${CLI_DIR:-$REPO_ROOT/../libviprs-cli}" && cargo build --release --no-default-features --bin viprs )
fi
"$VIPRS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_multiply_translucent_golden.png" multiply
"$VIPRS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_atop_translucent_golden.png"     atop
"$VIPRS" composite2 "$COMP/base.png" "$COMP/overlay.png" "$COMP/composite2_saturate_translucent_golden.png" saturate

# --- Provenance (append the composite section) -------------------------------
echo "==> [provenance] appending composite section to $FIX_ROOT/PROVENANCE.md"
# FREQFILT FAMILY (the Wave-2 Fourier lane; OP_MAP.md freqfilt section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_freqfilt_diff.rs (which feeds them to `viprs`), so
# the two sides compare like against like. EVERY freqfilt op is oracle class
# FOURIER (CLI_CONTRACT.md §5): FFT-derived floats compared as f64 band-pairs
# from the native `.v` container at a MEASURED epsilon (the core runs each
# transform in f64 through pure-Rust `rustfft` and stores f32; the vips oracle
# runs FFTW in double and stores dpcomplex/double — the two agree to the f32
# quantisation floor, so a small absolute eps sized to each op's magnitude is
# the honest tolerance, NOT tol 0). `spectrum` and `freqmult` fold their float
# result back to a uchar raster and are compared at an integer tolerance.
#
# Carrier choice (CRITICAL — the libviprs .v decoder accepts ONLY uchar/ushort/
# float band formats, and REJECTS vips `dpcomplex`(10) / `double`(8)): every
# complex vips output (fwfft, invfft) is normalised OFFLINE to a 2-band FLOAT
# raster via `complexget real` + `complexget imag` + `bandjoin` + `cast float`
# (band0 = re, band1 = im — the exact (re, im)-pair layout libviprs' fwfft
# writes), and every real vips output (invfft --real, phasecor) is `cast float`
# to an f32 `.v`. `spectrum`/`freqmult` are uchar → PNG. This makes every
# reference libviprs-decodable and compares like-for-like (verified: fwfft
# max-abs-diff 1.1e-16, invfft 2.8e-14, invfft --real 0, phasecor 0, spectrum 0,
# freqmult 1).
#
# Inputs are chosen DISCRIMINATING (the bands `bandmean` lesson — no vacuous
# no-op passes): `in.png` is a 2-D gradient varying in BOTH axes so the
# transforms are non-trivial; the `freqmult` mask is `mask_ideal 0.3` (a low-pass
# that changes the gradient by max-abs 114 — cutoff 0.1 was measured to change it
# by only 1, an all-but-no-op, and is deliberately NOT used); `phasecor` uses a
# wrap-shifted copy so the correlation peak sits at the (3,2) translation, not the
# origin. `invfft`/`fwfft` on a REAL input are genuine vips oracles (vips casts
# the real input to complex; the core transforms the real band — identical
# results). The invfft(fwfft(in)) --real ROUND-TRIP additionally exercises the
# core's COMPLEX-input path (`viprs invfft` reading its own Fourier-stamped 2-band
# f32 `.v`), the only differential that does — see the open question about a
# single-file complex-input carrier (vips dpcomplex vs libviprs f32-pairs are
# mutually unreadable, so no committed complex INPUT can feed both sides directly).
# ===========================================================================
FREQ="$FIX_ROOT/freqfilt"
mkdir -p "$FREQ"

# --- Common inputs -----------------------------------------------------------
# in.png: a 16x16 2-D gradient (x*85 + y*170, both axes vary), so every transform
# is non-vacuous. `vips grey`/`rot` are pure coordinate functions, so the input is
# bit-for-bit reproducible.
echo "==> [freqfilt input] 16x16 2-D gradient in.png"
"$VIPS" grey "$TMP/fg.v" 16 16
"$VIPS" rot "$TMP/fg.v" "$TMP/fgrot.v" d90
"$VIPS" linear "$TMP/fg.v"    "$TMP/fgx.v" 85  0
"$VIPS" linear "$TMP/fgrot.v" "$TMP/fgy.v" 170 0
"$VIPS" add "$TMP/fgx.v" "$TMP/fgy.v" "$TMP/fgsum.v"
"$VIPS" cast "$TMP/fgsum.v" "$FREQ/in.png" uchar

# mask.v: an ideal low-pass mask (float, fourier-tagged, 1-band), the 2nd input to
# freqmult. Cutoff 0.3 changes the gradient by max-abs 114 (discriminating); a
# cutoff of 0.1 was measured to change it by only 1 (near-no-op) and is NOT used.
echo "==> [freqfilt input] mask_ideal 16x16 cutoff 0.3 (mask.v, float)"
"$VIPS" mask_ideal "$FREQ/mask.v" 16 16 0.3

# shifted.png: in.png wrapped by (3,2), so phasecor's peak is the (3,2) shift (a
# discriminating, non-origin peak; phasecor(in,in) would peak at the origin).
echo "==> [freqfilt input] wrap-shifted copy shifted.png (x=3 y=2)"
"$VIPS" wrap "$FREQ/in.png" "$FREQ/shifted.png" --x 3 --y 2

# small.png: an 8x8 Gray8 image, a DIFFERENT size to in.png — the self-contained
# wrong-size input for the freqmult / phasecor dimension-mismatch error cases
# (CLI_CONTRACT.md §8: op failure -> exit 1). An error case needs no reference
# output, only a committed input.
echo "==> [freqfilt input] 8x8 small.png (dimension-mismatch error input)"
"$VIPS" grey "$TMP/fsm.v" 8 8
"$VIPS" linear "$TMP/fsm.v" "$FREQ/small.png" 255 0 --uchar

# --- References — one vips run per differential case -------------------------
# Helper: normalise a complex vips output (dpcomplex, band format 10 — which the
# libviprs .v decoder rejects) to a 2-band f32 `.v` (band0 = re, band1 = im).
freq_complex_to_pair() { # <complex_in.v> <pair_out.v>
    "$VIPS" complexget "$1" "$TMP/fcre.v" real
    "$VIPS" complexget "$1" "$TMP/fcim.v" imag
    "$VIPS" bandjoin "$TMP/fcre.v $TMP/fcim.v" "$TMP/fcpair.v"
    "$VIPS" cast "$TMP/fcpair.v" "$2" float
}

echo "==> [fwfft] real in -> complex spectrum -> 2-band f32 .v (FOURIER eps 1e-2)"
"$VIPS" fwfft "$FREQ/in.png" "$TMP/fwfft_dpc.v"
freq_complex_to_pair "$TMP/fwfft_dpc.v" "$FREQ/fwfft_expected.v"

echo "==> [invfft] real in -> complex out -> 2-band f32 .v (FOURIER eps 5e-2)"
"$VIPS" invfft "$FREQ/in.png" "$TMP/invfft_dpc.v"
freq_complex_to_pair "$TMP/invfft_dpc.v" "$FREQ/invfft_expected.v"

echo "==> [invfft --real] real in -> real out -> 1-band f32 .v (FOURIER eps 5e-2)"
"$VIPS" invfft "$FREQ/in.png" "$TMP/invfft_real_dbl.v" --real
"$VIPS" cast "$TMP/invfft_real_dbl.v" "$FREQ/invfft_real_expected.v" float

# Round-trip invfft(fwfft(in)) --real: recovers the input AND exercises the
# COMPLEX-input path on the viprs side (viprs invfft reads its own Fourier .v).
echo "==> [invfft roundtrip] invfft(fwfft(in)) --real -> 1-band f32 .v (complex-in path, FOURIER eps 1e-2)"
"$VIPS" fwfft "$FREQ/in.png" "$TMP/rt_dpc.v"
"$VIPS" invfft "$TMP/rt_dpc.v" "$TMP/rt_dbl.v" --real
"$VIPS" cast "$TMP/rt_dbl.v" "$FREQ/roundtrip_expected.v" float

echo "==> [freqmult] in * mask (low-pass) -> uchar PNG (FOURIER, BOUNDED-TOL ≤1 LSB)"
"$VIPS" freqmult "$FREQ/in.png" "$FREQ/mask.v" "$FREQ/freqmult_expected.png"

echo "==> [spectrum] displayable log-magnitude -> uchar PNG (FOURIER, tol 0)"
"$VIPS" spectrum "$FREQ/in.png" "$FREQ/spectrum_expected.png"

echo "==> [phasecor] in vs shifted -> real correlation surface -> 1-band f32 .v (FOURIER eps 1e-2)"
"$VIPS" phasecor "$FREQ/in.png" "$FREQ/shifted.png" "$TMP/phasecor_dbl.v"
"$VIPS" cast "$TMP/phasecor_dbl.v" "$FREQ/phasecor_expected.v" float

# --- Provenance (append the freqfilt section) --------------------------------
echo "==> [provenance] appending freqfilt section to $FIX_ROOT/PROVENANCE.md"
"$VIPS" gaussnoise "$TMP/gb1.v" 60 50 --seed 3 --mean 100 --sigma 20
"$VIPS" cast "$TMP/gb1.v" "$TMP/gb1.png" uchar
"$VIPS" gaussnoise "$TMP/gb2.v" 60 50 --seed 4 --mean 160 --sigma 20
"$VIPS" cast "$TMP/gb2.v" "$TMP/gb2.png" uchar
# dx = overlap(20) - ref.width(60) = -40 -> a 100x50 mosaic, overlap 20x50.
"$VIPRS" merge "$TMP/gb1.png" "$TMP/gb2.png" "$MOS/balance_input.v" horizontal -40 0
"$VIPRS" globalbalance "$MOS/balance_input.v" "$MOS/balance_expected.v" >/dev/null

# --- Provenance (append the mosaicing section) -------------------------------
echo "==> [provenance] appending mosaicing section to $FIX_ROOT/PROVENANCE.md"
# --- Common input: a buildlut control-point matrix (.mat) --------------------
# Two control points [0->0, 255->100] in the vips text-matrix format (a 2-column
# 2-row matrix). Read by BOTH `vips buildlut` (as a matrix image) and `viprs
# buildlut` (via the shared MatFile loader), so the two sides interpolate the
# SAME control points (CLI_CONTRACT.md §3 matrix-file arg).
echo "==> [create input] buildlut control points (buildlut_points.mat)"
printf '2 2\n0 0\n255 100\n' > "$CREATE/buildlut_points.mat"

# A >=3-control-point matrix with a NON-COLLINEAR breakpoint (0->0, 128->200,
# 255->255), so multi-segment interpolation + control-point sort are pinned: a
# buildlut that ignored the intermediate point and interpolated only the two
# endpoints would give ~100 at x=128, but the true piecewise curve gives 200
# (adversarial review create finding 2; verified with vips: 200 at x=128,
# 100 at x=64). The 2-point matrix above is a single linear segment and cannot
# discriminate this. '2 3' = 2 columns, 3 rows.
echo "==> [create input] buildlut 3-point non-collinear (buildlut_points3.mat)"
printf '2 3\n0 0\n128 200\n255 255\n' > "$CREATE/buildlut_points3.mat"

# --- EXACT references (tol 0) -----------------------------------------------
echo "==> [black] 1-band + --bands 3 all-zero uchar -> .v (EXACT)"
"$VIPS" black "$CREATE/black_expected.v"       8 8
"$VIPS" black "$CREATE/black_bands3_expected.v" 8 8 --bands 3

# xyz: vips emits uint; cast to float so the libviprs `.v` decoder reads it (the
# coordinates 0..15 are integer-exact in f32). viprs xyz is float natively.
echo "==> [xyz] coordinate ramp (vips uint -> float .v, EXACT)"
"$VIPS" xyz "$TMP/xyz_u.v" 16 16
"$VIPS" cast "$TMP/xyz_u.v" "$CREATE/xyz_expected.v" float

# --- BOUNDED-TOL references (float f32) -------------------------------------
echo "==> [eye] float + --uchar (BOUNDED-TOL f32 / <=1 LSB uchar)"
"$VIPS" eye "$CREATE/eye_expected.v"       32 32
"$VIPS" eye "$CREATE/eye_uchar_expected.png" 32 32 --uchar

echo "==> [zone] + [sines] float trig patterns (BOUNDED-TOL f32)"
"$VIPS" zone  "$CREATE/zone_expected.v"  32 32
"$VIPS" sines "$CREATE/sines_expected.v" 32 32

# buildlut: vips emits double; cast to float (the LUT entries are f32-exact) so
# the libviprs `.v` decoder reads it and the format class matches viprs's f32.
echo "==> [buildlut] control-point LUT (vips double -> float .v, BOUNDED-TOL)"
"$VIPS" buildlut "$CREATE/buildlut_points.mat" "$TMP/buildlut_d.v"
"$VIPS" cast "$TMP/buildlut_d.v" "$CREATE/buildlut_expected.v" float
# >=3-control-point (multi-segment) LUT — pins piecewise interpolation + point sort.
echo "==> [buildlut] 3-point non-collinear LUT (multi-segment interp, BOUNDED-TOL)"
"$VIPS" buildlut "$CREATE/buildlut_points3.mat" "$TMP/buildlut3_d.v"
"$VIPS" cast "$TMP/buildlut3_d.v" "$CREATE/buildlut3_expected.v" float

echo "==> [tonelut] Gray16 tone curve -> .v (BOUNDED-TOL, measured exact)"
"$VIPS" tonelut "$CREATE/tonelut_expected.v"

echo "==> [mask_ideal] highpass + --nodc (BOUNDED-TOL f32)"
"$VIPS" mask_ideal "$CREATE/mask_ideal_expected.v"      64 64 0.5
"$VIPS" mask_ideal "$CREATE/mask_ideal_nodc_expected.v" 64 64 0.5 --nodc
echo "==> [mask_ideal_ring/band]"
"$VIPS" mask_ideal_ring "$CREATE/mask_ideal_ring_expected.v" 64 64 0.5 0.2
"$VIPS" mask_ideal_band "$CREATE/mask_ideal_band_expected.v" 64 64 0.3 0.3 0.1
# DISTINCT frequency-cutoff != amplitude-cutoff (0.4 != 0.6) so a handler that
# SWAPPED the fc/ac positionals would flip the output and fail the compare —
# equal 0.5/0.5 values left the two positionals interchangeable (adversarial
# review create finding 1). Verified with vips: mask_gaussian 0.4 0.6 vs 0.6 0.4
# differ (max-abs-diff 0.083).
echo "==> [mask_gaussian family] (distinct fc 0.4 != ac 0.6)"
"$VIPS" mask_gaussian      "$CREATE/mask_gaussian_expected.v"      64 64 0.4 0.6
"$VIPS" mask_gaussian_ring "$CREATE/mask_gaussian_ring_expected.v" 64 64 0.4 0.6 0.2
"$VIPS" mask_gaussian_band "$CREATE/mask_gaussian_band_expected.v" 64 64 0.3 0.3 0.1 0.5
# Isolated --nodc on a gaussian mask (float `.v`): --nodc is otherwise pinned
# only on mask_ideal, so a per-op nodc mis-wire on the gaussian family would not
# be caught by any differential (adversarial review create finding 4). vips:
# --nodc changes the DC pixel (max-abs-diff 1.0 vs the plain mask).
echo "==> [mask_gaussian] isolated --nodc (float .v; DC pixel changes)"
"$VIPS" mask_gaussian "$CREATE/mask_gaussian_nodc_expected.v" 64 64 0.4 0.6 --nodc
echo "==> [mask_butterworth family incl. --uchar --optical] (distinct fc 0.4 != ac 0.6)"
"$VIPS" mask_butterworth      "$CREATE/mask_butterworth_expected.v"       64 64 2 0.4 0.6
"$VIPS" mask_butterworth      "$CREATE/mask_butterworth_uchar_expected.png" 64 64 2 0.5 0.5 --uchar --optical
"$VIPS" mask_butterworth_ring "$CREATE/mask_butterworth_ring_expected.v"  64 64 2 0.4 0.6 0.2
"$VIPS" mask_butterworth_band "$CREATE/mask_butterworth_band_expected.v"  64 64 2 0.3 0.3 0.1 0.5
# Isolated --optical on a FLOAT `.v` mask: --optical was exercised only jointly
# with --uchar on a PNG, never bit-checkable in isolation (adversarial review
# create finding 4). vips: --optical rotates quadrants (max-abs-diff ~1.0).
echo "==> [mask_butterworth] isolated --optical (float .v; quadrant rotation)"
"$VIPS" mask_butterworth "$CREATE/mask_butterworth_optical_expected.v" 64 64 2 0.4 0.6 --optical
echo "==> [mask_fractal]"
"$VIPS" mask_fractal "$CREATE/mask_fractal_expected.v" 64 64 2.5

echo "==> [sdf] circle/box/line/rounded-box (BOUNDED-TOL f32 hypot)"
"$VIPS" sdf "$CREATE/sdf_circle_expected.v"      64 64 circle      --a "32 32" --r 16
"$VIPS" sdf "$CREATE/sdf_box_expected.v"         64 64 box         --a "10 10" --b "50 40"
"$VIPS" sdf "$CREATE/sdf_line_expected.v"        64 64 line        --a "10 10" --b "50 40"
"$VIPS" sdf "$CREATE/sdf_rounded_expected.v"     64 64 rounded-box --a "10 10" --b "50 40" --corners "20 0 0 0"

# --- GOLDEN-ONLY pins (NO vips oracle; generated by viprs, regression pins) ---
# vips's PRNG (gaussnoise/perlin/worley/fractsurf) and Pango text rendering
# differ from libviprs even with a fixed seed, so there is nothing to
# cross-check against. These references are the deterministic `viprs` output
# itself; the tests pin that output against future regressions (CLI_CONTRACT §5).
echo "==> [gaussnoise/perlin/worley/fractsurf/text] GOLDEN-ONLY viprs pins (no vips oracle)"
"$VIPRS" gaussnoise "$CREATE/gaussnoise_golden.v" 16 16 --seed 42 --sigma 10 --mean 128 >/dev/null
"$VIPRS" perlin     "$CREATE/perlin_golden.v"     64 64 --seed 7 >/dev/null
"$VIPRS" worley     "$CREATE/worley_golden.v"     64 64 --seed 7 >/dev/null
"$VIPRS" fractsurf  "$CREATE/fractsurf_golden.v"  64 48 2.5 >/dev/null
"$VIPRS" text       "$CREATE/text_golden.png"     "Hi" --dpi 72 >/dev/null

# --- Provenance (append the create section) ----------------------------------
echo "==> [provenance] appending create section to $FIX_ROOT/PROVENANCE.md"
# rgb: a 32x32 sRGB 2-D gradient (band0 = horizontal ramp, band1 = a scaled
# VERTICAL ramp, band2 = a third horizontal ramp), so a solid-ink shape drawn
# over it changes pixels in a fully 2-D-determined way. sRGB-tagged so the 3-band
# PNG saves as clean RGB (not a b-w image vips would alpha-pad).
echo "==> [draw input] 32x32 sRGB 2-D gradient (rgb.png) — circle/rect/line/mask/image base"
"$VIPS" grey "$TMP/dg.v" 32 32
"$VIPS" linear "$TMP/dg.v" "$TMP/dgx.png" 255 0 --uchar
"$VIPS" rot "$TMP/dg.v" "$TMP/dgv.v" d90
"$VIPS" linear "$TMP/dgv.v" "$TMP/dgy.png" 200 20 --uchar
"$VIPS" linear "$TMP/dg.v" "$TMP/dgz.png" 120 60 --uchar
"$VIPS" bandjoin "$TMP/dgx.png $TMP/dgy.png $TMP/dgz.png" "$TMP/drgb.v"
"$VIPS" copy "$TMP/drgb.v" "$DRAW/rgb.png" --interpretation srgb

# flood: a 32x32 Gray8 image of THREE FLAT vertical stripes (values 0 / 100 /
# 200), so the flood ops pick a genuine bounded region. A bounded flood from the
# left (0) stripe with ink 100 fills the 0 stripe up to the 100 wall (→ 100 100
# 200 — it STOPS, leaving the 200 stripe untouched: a broken flood that filled
# everything would fail). A blob flood (--equal) recolours only the seed's own
# equal-valued (0) stripe (→ 50 100 200). The two are distinct and region-limited.
echo "==> [draw input] 32x32 Gray8 three flat stripes 0/100/200 (flood.png)"
"$VIPS" linear "$TMP/dg.v" "$TMP/dramp.v" 255 0 --uchar
"$VIPS" relational_const "$TMP/dramp.v" "$TMP/dr1.v" moreeq 85
"$VIPS" relational_const "$TMP/dramp.v" "$TMP/dr2.v" moreeq 170
"$VIPS" add "$TMP/dr1.v" "$TMP/dr2.v" "$TMP/dr12.v"
# (0,255,510) * (100/255) = (0,100,200), exact.
"$VIPS" linear "$TMP/dr12.v" "$DRAW/flood.png" 0.39215686274509803 0 --uchar

# mask: a 16x16 single-band Gray8 horizontal ramp (0..255), used as the
# draw_mask OPACITY stencil (0 = no paint, 255 = full ink, partial blend between).
echo "==> [draw input] 16x16 Gray8 ramp stencil (mask.png)"
"$VIPS" grey "$TMP/dm.v" 16 16
"$VIPS" linear "$TMP/dm.v" "$DRAW/mask.png" 255 0 --uchar

# sub: an 8x8 SOLID magenta sRGB sub-image whose distinct colour stands out
# wherever draw_image pastes it. Must share rgb.png's RGB8 format (the core paste
# is a no-op on a format mismatch), so it is 3-band sRGB like the base.
echo "==> [draw input] 8x8 solid magenta sRGB sub-image (sub.png)"
"$VIPS" black "$TMP/db.v" 8 8 --bands 3
"$VIPS" linear "$TMP/db.v" "$TMP/dsub.v" "0 0 0" "255 0 255" --uchar
"$VIPS" copy "$TMP/dsub.v" "$DRAW/sub.png" --interpretation srgb

# smudge: a 16x16 Gray8 HIGH-FREQUENCY pattern (a thresholded `eye` zone-plate,
# 0/255) so the 3x3 box-blur genuinely changes many pixels. A smooth gradient
# would be a near-no-op in its interior (the mean of a linear ramp ≈ its centre),
# which would make the smudge pin vacuous — the high-frequency input avoids that.
echo "==> [draw input] 16x16 Gray8 high-frequency thresholded eye (smudge.png)"
"$VIPS" eye "$TMP/de.v" 16 16
"$VIPS" relational_const "$TMP/de.v" "$DRAW/smudge.png" more 0.0

# gray16: a 16x16 SINGLE-BAND Gray16 (ushort) ramp, carried as native `.v` (a
# 16-bit PNG neither vips nor the libviprs decoder round-trips cleanly — a ushort
# PNG whose samples fit in 8 bits comes back 8-bit). Its samples run 0..~60000 so
# a >255 ink genuinely lands in the high byte. This is the ONLY 16-bit draw target
# in the suite: it exercises build_ink's 2-byte native-endian encode branch and
# the core mask/rect 16-bit sample path end-to-end at the CLI level.
echo "==> [draw input] 16x16 Gray16 ramp (gray16.v) — 16-bit ink encode/draw pin"
"$VIPS" grey "$TMP/dg16.v" 16 16
"$VIPS" linear "$TMP/dg16.v" "$TMP/dg16s.v" 60000 0
"$VIPS" cast "$TMP/dg16s.v" "$DRAW/gray16.v" ushort

# --- References — every one minted by `viprs` (GOLDEN-ONLY, no vips oracle) ---
echo "==> [draw_circle] outline + --fill disc, red ink (GOLDEN-ONLY)"
"$VIPRS" draw_circle "$DRAW/rgb.png" "$DRAW/draw_circle_golden.png"      16 16 8 --ink "255 0 0" >/dev/null
"$VIPRS" draw_circle "$DRAW/rgb.png" "$DRAW/draw_circle_fill_golden.png" 16 16 8 --ink "255 0 0" --fill >/dev/null

echo "==> [draw_rect] outline + --fill, green ink (GOLDEN-ONLY)"
"$VIPRS" draw_rect "$DRAW/rgb.png" "$DRAW/draw_rect_golden.png"      4 4 20 16 --ink "0 255 0" >/dev/null
"$VIPRS" draw_rect "$DRAW/rgb.png" "$DRAW/draw_rect_fill_golden.png" 4 4 20 16 --ink "0 255 0" --fill >/dev/null

echo "==> [draw_line] diagonal, blue ink (GOLDEN-ONLY)"
"$VIPRS" draw_line "$DRAW/rgb.png" "$DRAW/draw_line_golden.png" 0 0 31 31 --ink "0 0 255" >/dev/null

echo "==> [draw_flood] bounded (ink 100) + blob --equal (ink 50) on the stripes (GOLDEN-ONLY)"
"$VIPRS" draw_flood "$DRAW/flood.png" "$DRAW/draw_flood_golden.png"      0 0 --ink "100" >/dev/null
"$VIPRS" draw_flood "$DRAW/flood.png" "$DRAW/draw_flood_blob_golden.png" 0 0 --ink "50" --equal >/dev/null

echo "==> [draw_mask] yellow ink through the ramp stencil at (8,8) (GOLDEN-ONLY)"
"$VIPRS" draw_mask "$DRAW/rgb.png" "$DRAW/draw_mask_golden.png" "$DRAW/mask.png" 8 8 --ink "255 255 0" >/dev/null

echo "==> [draw_smudge] box-blur a rect over the high-frequency input (GOLDEN-ONLY)"
"$VIPRS" draw_smudge "$DRAW/smudge.png" "$DRAW/draw_smudge_golden.png" 4 4 8 8 >/dev/null

echo "==> [draw_image] paste the magenta sub-image at (10,10) (GOLDEN-ONLY)"
"$VIPRS" draw_image "$DRAW/rgb.png" "$DRAW/draw_image_golden.png" "$DRAW/sub.png" 10 10 >/dev/null

# 16-bit target pin: draw_rect a >255 ink onto the Gray16 ramp, output as `.v`.
# This is the only golden that pins build_ink's 2-byte native-endian encode + the
# core 16-bit draw/save round trip; a byte-order regression would be caught here.
echo "==> [draw_rect 16-bit] ink 40000 onto the Gray16 ramp -> .v (GOLDEN-ONLY, 16-bit ink pin)"
"$VIPRS" draw_rect "$DRAW/gray16.v" "$DRAW/draw_rect_16bit_golden.v" 4 4 8 8 --ink "40000" >/dev/null

# --- Provenance (append the draw section) ------------------------------------
echo "==> [provenance] appending draw section to $FIX_ROOT/PROVENANCE.md"
# RESAMPLE FAMILY (the Wave-2 resample lane; OP_MAP.md resample section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_resample_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. EVERY resample op is oracle class
# BOUNDED-TOL (the premultiply / rounding campaign #406-418): the core computes
# the reduce / interpolate masks in f64 per output position while vips quantises
# the sub-pixel offset into fixed-point tables, so the two agree to ≤1 LSB. The
# MEASURED per-case max-abs-diff is recorded in the provenance table; all cases
# land at 0 or 1 LSB EXCEPT `affine … --interpolate bicubic`, which measures 2
# LSB (the bicubic Catmull-Rom coefficients quantise more coarsely in vips — a
# genuine, honest core-vs-vips divergence, NOT a CLI bug; see the open question).
#
# Carriers: every reference is an integer uchar raster (1-band grad, or sRGB
# 3-band rgb), so all references round-trip losslessly through PNG. `mapim`'s
# index is a FLOAT 2-band `.v` coordinate image (committed input, exact). vips
# PNG inputs only for `thumbnail` (no jpeg shrink-on-load divergence).
#
# Inputs are DISCRIMINATING (an identity / no-op op would FAIL): `grad` is a 2-D
# gradient varying in BOTH axes (so shrinkv / reducev / rot are non-vacuous), and
# `index.v` maps every output to HALF its source coordinate (a real 2× zoom, not
# the identity map, so `mapim` moves data — verified: vips mapim vs the original
# differs by 128).
# ===========================================================================
RESAMPLE="$FIX_ROOT/resample"
mkdir -p "$RESAMPLE"

# --- Common inputs -----------------------------------------------------------
# `vips grey`/`rot`/`xyz` are pure coordinate functions, so every fixture below
# is bit-for-bit reproducible.
echo "==> [resample input] 32x32 grad (2-D gradient) + rgb (2-D sRGB) + index.v (2× map)"
"$VIPS" grey "$TMP/rg.v" 32 32
"$VIPS" rot "$TMP/rg.v" "$TMP/rgr.v" d90
# grad: x*85 + y*170, a genuine 2-D gradient (max ~247, fills uchar).
"$VIPS" linear "$TMP/rg.v"  "$TMP/rgx.v" 85  0
"$VIPS" linear "$TMP/rgr.v" "$TMP/rgy.v" 170 0
"$VIPS" add "$TMP/rgx.v" "$TMP/rgy.v" "$TMP/rgsum.v"
"$VIPS" cast "$TMP/rgsum.v" "$RESAMPLE/grad.png" uchar
# rgb: band0 horizontal ramp, band1 scaled+offset, band2 vertical ramp (sRGB).
"$VIPS" linear "$TMP/rg.v"  "$TMP/rc0.png" 255 0  --uchar
"$VIPS" linear "$TMP/rg.v"  "$TMP/rc1.png" 200 20 --uchar
"$VIPS" linear "$TMP/rgr.v" "$TMP/rc2.png" 255 0  --uchar
"$VIPS" bandjoin "$TMP/rc0.png $TMP/rc1.png $TMP/rc2.png" "$TMP/rcj.v"
"$VIPS" copy "$TMP/rcj.v" "$RESAMPLE/rgb.png" --interpretation srgb
# index.v: a FLOAT 2-band coordinate map sampling every output at HALF its source
# coordinate (band0 = x/2, band1 = y/2) — a 2× zoom of the top-left quadrant with
# fractional taps, so mapim's interpolation is genuinely exercised (non-vacuous).
"$VIPS" xyz "$TMP/rxyz.v" 32 32
"$VIPS" linear "$TMP/rxyz.v" "$TMP/ridxf.v" 0.5 0
"$VIPS" cast "$TMP/ridxf.v" "$RESAMPLE/index.v" float

# --- References — one vips run per differential case -------------------------
echo "==> [shrink] grad 2 2 + shrinkh/shrinkv 2 (S1 box shrink, ≤1 LSB)"
"$VIPS" shrink  "$RESAMPLE/grad.png" "$RESAMPLE/shrink_expected.png"  2 2
"$VIPS" shrinkh "$RESAMPLE/grad.png" "$RESAMPLE/shrinkh_expected.png" 2
"$VIPS" shrinkv "$RESAMPLE/grad.png" "$RESAMPLE/shrinkv_expected.png" 2

echo "==> [reduce] rgb 2 2 lanczos3 (default) + cubic (enum) + reduceh/reducev grad 2 (≤1 LSB)"
"$VIPS" reduce  "$RESAMPLE/rgb.png"  "$RESAMPLE/reduce_lanczos3_expected.png" 2 2
"$VIPS" reduce  "$RESAMPLE/rgb.png"  "$RESAMPLE/reduce_cubic_expected.png"    2 2 --kernel cubic
"$VIPS" reduceh "$RESAMPLE/grad.png" "$RESAMPLE/reduceh_expected.png" 2
"$VIPS" reducev "$RESAMPLE/grad.png" "$RESAMPLE/reducev_expected.png" 2

echo "==> [resize] rgb 0.5 + --vscale + upscale (affine path) + --kernel nearest (≤1 LSB)"
"$VIPS" resize "$RESAMPLE/rgb.png"  "$RESAMPLE/resize_half_expected.png"    0.5
"$VIPS" resize "$RESAMPLE/rgb.png"  "$RESAMPLE/resize_vscale_expected.png"  0.5 --vscale 0.75
"$VIPS" resize "$RESAMPLE/grad.png" "$RESAMPLE/resize_up_expected.png"      2.0
"$VIPS" resize "$RESAMPLE/rgb.png"  "$RESAMPLE/resize_nearest_expected.png" 0.5 --kernel nearest

echo "==> [affine] rgb 1.5x bilinear (≤1 LSB) + bicubic (2 LSB — noted divergence)"
"$VIPS" affine "$RESAMPLE/rgb.png" "$RESAMPLE/affine_bilinear_expected.png" "1.5 0 0 1.5"
"$VIPS" affine "$RESAMPLE/rgb.png" "$RESAMPLE/affine_bicubic_expected.png"  "1.5 0 0 1.5" --interpolate bicubic

echo "==> [similarity] rgb --angle 30 + --scale 1.5 (≤1 LSB)"
"$VIPS" similarity "$RESAMPLE/rgb.png" "$RESAMPLE/similarity_angle_expected.png" --angle 30
"$VIPS" similarity "$RESAMPLE/rgb.png" "$RESAMPLE/similarity_scale_expected.png" --scale 1.5

echo "==> [rotate] rgb 30 (≤1 LSB)"
"$VIPS" rotate "$RESAMPLE/rgb.png" "$RESAMPLE/rotate_expected.png" 30

echo "==> [mapim] rgb + index.v bilinear (default) + bicubic (S2, ≤1 LSB)"
"$VIPS" mapim "$RESAMPLE/rgb.png" "$RESAMPLE/mapim_bilinear_expected.png" "$RESAMPLE/index.v"
"$VIPS" mapim "$RESAMPLE/rgb.png" "$RESAMPLE/mapim_bicubic_expected.png"  "$RESAMPLE/index.v" --interpolate bicubic

echo "==> [thumbnail] rgb file 16 + non-square --crop centre + --linear; thumbnail_image rgb 16 (≤1 LSB)"
"$VIPS" thumbnail "$RESAMPLE/rgb.png" "$RESAMPLE/thumbnail_expected.png"      16
# --crop is DISCRIMINATED with a NON-square target (16x8): a centre-crop of a
# square source into a non-square box actually removes pixels, so the reference
# is DISTINCT from the no-crop 16x16 fixtures (a build that ignored / dropped
# --crop would fail). A square 16 box on a square source crops nothing (the
# identity case) and would be vacuous — see the adversarial-review finding.
"$VIPS" thumbnail "$RESAMPLE/rgb.png" "$RESAMPLE/thumbnail_crop_expected.png" 16 --height 8 --crop centre
# --linear reduces in linear light (a DISTINCT core entry point from the default
# sRGB-space reduce), so this reference guards the linear-light path directly.
"$VIPS" thumbnail "$RESAMPLE/rgb.png" "$RESAMPLE/thumbnail_linear_expected.png" 16 --linear
"$VIPS" thumbnail_image "$RESAMPLE/rgb.png" "$RESAMPLE/thumbnail_image_expected.png" 16

# --- Provenance (append the resample section) --------------------------------
echo "==> [provenance] appending resample section to $FIX_ROOT/PROVENANCE.md"

# --- Common inputs -----------------------------------------------------------
echo "==> [aritha input] agray (16x16 Gray8 ramp) + afloat (4x4 float, discriminating)"
"$VIPS" grey "$TMP/ag.v" 16 16
"$VIPS" linear "$TMP/ag.v" "$ARITHA/agray.png" 255 0 --uchar
# afloat: a HAND-PICKED 4x4 float image that reaches the domains the unary/rounding
# ops actually branch on (adversarial-review aritha findings 2 & 4). The prior
# `510*grey - 255` ramp landed only on near-integer values (34*i-255), so it (a)
# NEVER hit an exact half-integer — hiding that the core `round rint` (f64::round,
# half AWAY from zero) diverges from vips's C `rint` (half TO EVEN) at x.5 — and
# (b) NEVER hit exactly 0.0, so `sign`'s zero→0 branch was untested. This input
# fixes both: it crosses zero, includes exactly 0.0, several exact half-integers
# (−2.5/−0.5/0.5/2.5 where core≠vips, plus −3.5/1.5/3.5/17.5 where they agree), and
# non-half fractionals (0.25/−0.75) so `rint`≠`floor`≠`ceil` is discriminating.
# Every value is exactly representable in f32 (dyadic), so there is no float wobble.
# Built via csvload → cast float (vips linear cannot emit arbitrary per-pixel data).
cat > "$TMP/afloat.csv" <<'CSV'
-255.0,-3.5,-2.5,-0.75
-0.5,0.0,0.25,0.5
1.5,2.5,3.5,17.5
85.25,128.0,223.125,255.0
CSV
"$VIPS" csvload "$TMP/afloat.csv" "$TMP/afloat_d.v"
"$VIPS" cast "$TMP/afloat_d.v" "$ARITHA/afloat.v" float

# find_trim content: a black 6x7 block embedded at (4,5) into a 20x20 WHITE
# background (default background 255 → the block is content). find_trim must
# return left=4 top=5 width=6 height=7 (a non-vacuous interior box).
echo "==> [aritha input] content (black block on white, for find_trim default)"
"$VIPS" black "$TMP/blk.v" 6 7 --bands 1
"$VIPS" linear "$TMP/blk.v" "$TMP/blk.png" 0 0 --uchar
"$VIPS" embed "$TMP/blk.png" "$ARITHA/content.png" 4 5 20 20 --extend white

# find_trim --background 0 content: a white 5x4 block inserted at (3,2) into a
# 20x20 BLACK background (background 0 → the white block is content).
echo "==> [aritha input] content2 (white block on black, for find_trim --background 0)"
"$VIPS" black "$TMP/blkbg.v" 20 20 --bands 1
"$VIPS" linear "$TMP/blkbg.v" "$TMP/blkbg.png" 0 0 --uchar
"$VIPS" black "$TMP/wblk.v" 5 4 --bands 1
"$VIPS" linear "$TMP/wblk.v" "$TMP/wblk.png" 0 255 --uchar
"$VIPS" insert "$TMP/blkbg.png" "$TMP/wblk.png" "$ARITHA/content2.png" 3 2

# profile input: an 8x8 all-zero image with a 3x3 white block at (2,3), so the
# first-non-zero column/row positions are discriminating (columns 2..4 → row 3;
# rows 3..5 → column 2; every other line has no non-zero and reports 8).
echo "==> [aritha input] pzero (8x8 zero + 3x3 block at 2,3, for profile)"
"$VIPS" black "$TMP/pz.v" 8 8 --bands 1
"$VIPS" linear "$TMP/pz.v" "$TMP/pz.png" 0 0 --uchar
"$VIPS" black "$TMP/pblk.v" 3 3 --bands 1
"$VIPS" linear "$TMP/pblk.v" "$TMP/pblk.png" 0 255 --uchar
"$VIPS" insert "$TMP/pz.png" "$TMP/pblk.png" "$ARITHA/pzero.png" 2 3

# hough input: a 32x32 black image with a single white pixel at (10,6). A single
# voting point is a discriminating, non-vacuous Hough input (it votes for every
# line through it / every circle centred one radius away).
echo "==> [aritha input] point (32x32 black, one white pixel at 10,6, for hough)"
"$VIPS" black "$TMP/hb.v" 32 32 --bands 1
"$VIPS" linear "$TMP/hb.v" "$TMP/hblack.png" 0 0 --uchar
"$VIPS" black "$TMP/pt.v" 1 1 --bands 1
"$VIPS" linear "$TMP/pt.v" "$TMP/pt.png" 0 255 --uchar
"$VIPS" insert "$TMP/hblack.png" "$TMP/pt.png" "$ARITHA/point.png" 10 6

# --- References — one vips run per differential case -------------------------
echo "==> [avg/deviate/min/max] S3 scalars"
"$VIPS" avg     "$ARITHA/agray.png"          > "$ARITHA/avg_expected.txt"
"$VIPS" deviate "$ARITHA/agray.png"          > "$ARITHA/deviate_expected.txt"
"$VIPS" min     "$ARITHA/agray.png"          > "$ARITHA/min_expected.txt"
"$VIPS" max     "$ARITHA/agray.png"          > "$ARITHA/max_expected.txt"
# min/max with --x --y print x, then y, then the value (three lines).
"$VIPS" min     "$ARITHA/agray.png" --x --y  > "$ARITHA/min_xy_expected.txt"
"$VIPS" max     "$ARITHA/agray.png" --x --y  > "$ARITHA/max_xy_expected.txt"

echo "==> [find_trim] default (white bg) + --background 0 (black bg), 4 ints each"
"$VIPS" find_trim "$ARITHA/content.png"                 > "$ARITHA/find_trim_expected.txt"
"$VIPS" find_trim "$ARITHA/content2.png" --background 0 > "$ARITHA/find_trim_bg_expected.txt"

# stats: vips emits a 10-column double matrix (cols 6..10 are min/max positions
# the core does not compute); crop to the first 6 columns (min/max/sum/sum2/mean/
# sd) and cast to float so the reference matches the core's f32 6-col matrix.
echo "==> [stats] 6-column per-band matrix -> float .v"
"$VIPS" stats "$ARITHA/agray.png" "$TMP/st.v"
"$VIPS" extract_area "$TMP/st.v" "$TMP/st6.v" 0 0 6 2
"$VIPS" cast "$TMP/st6.v" "$ARITHA/stats_expected.v" float

echo "==> [measure] 2x2 patch means -> float .v"
"$VIPS" measure "$ARITHA/agray.png" "$TMP/ms.v" 2 2
"$VIPS" cast "$TMP/ms.v" "$ARITHA/measure_expected.v" float

echo "==> [profile] first non-zero position per col/row -> ushort .v"
"$VIPS" profile "$ARITHA/pzero.png" "$TMP/pcol.v" "$TMP/prow.v"
"$VIPS" cast "$TMP/pcol.v" "$ARITHA/profile_cols_expected.v" ushort
"$VIPS" cast "$TMP/prow.v" "$ARITHA/profile_rows_expected.v" ushort

echo "==> [project] col/row sums -> ushort .v (sums < 65535)"
"$VIPS" project "$ARITHA/agray.png" "$TMP/qcol.v" "$TMP/qrow.v"
"$VIPS" cast "$TMP/qcol.v" "$ARITHA/project_cols_expected.v" ushort
"$VIPS" cast "$TMP/qrow.v" "$ARITHA/project_rows_expected.v" ushort

echo "==> [linear] scalar a·in+b: float .v + --uchar PNG"
"$VIPS" linear "$ARITHA/agray.png" "$ARITHA/linear_expected.v"      2 10
"$VIPS" linear "$ARITHA/agray.png" "$ARITHA/linear_uchar_expected.png" 2 10 --uchar

echo "==> [remainder_const] c=100 -> PNG (format-preserving int)"
"$VIPS" remainder_const "$ARITHA/agray.png" "$ARITHA/remainder_const_expected.png" 100

# The core `pow_const` rounds-and-saturates into a ushort (16-bit) output, while
# vips `math2_const pow` emits float; cast the vips reference to ushort to match
# the core carrier. On the integer input pow(v,2) is an exact integer (<=65025),
# so no rounding occurs and the comparison is bit-exact (EXACT-AFTER-CAST, tol 0).
echo "==> [math2_const] pow 2 -> ushort .v (core rounds pow into ushort)"
"$VIPS" math2_const "$ARITHA/agray.png" "$TMP/pow.v" pow 2
"$VIPS" cast "$TMP/pow.v" "$ARITHA/math2_const_pow_expected.v" ushort

# `abs` on a float input stays float. `sign` on a float input: vips emits a
# SIGNED CHAR (-1/0/1) the libviprs decoder cannot read; cast it to float (which
# preserves -1/0/1) to match the core's float sign output — and to exercise the
# NEGATIVE-sign parity that a uchar `.v` would clip to 0 (the #283 gap).
echo "==> [abs] on afloat (crosses zero) -> float .v"
"$VIPS" abs  "$ARITHA/afloat.v" "$ARITHA/abs_expected.v"
echo "==> [sign] on afloat -> float .v (vips emits signed char; cast to float)"
"$VIPS" sign "$ARITHA/afloat.v" "$TMP/sign.v"
"$VIPS" cast "$TMP/sign.v" "$ARITHA/sign_expected.v" float

# ceil/floor have no tie-break ambiguity, so they match vips EXACTLY (tol 0).
echo "==> [round] rint/ceil/floor on afloat -> float .v (EXACT vips oracle)"
"$VIPS" round "$ARITHA/afloat.v" "$ARITHA/round_ceil_expected.v"  ceil
"$VIPS" round "$ARITHA/afloat.v" "$ARITHA/round_floor_expected.v" floor

# rint is now EXACT against a real vips oracle: core issue #494 changed the core
# to round half TO EVEN, matching vips's C `rint` at exact half-integers on the
# honest afloat (which reaches x.5: 0.5→0, 2.5→2, −2.5→−2, −0.5→0). Previously the
# core mapped `f64::round` (half AWAY from zero) and diverged by 1 at every x.5,
# which forced a GOLDEN-ONLY viprs pin; that pin (round_rint_golden.v) is retired.
"$VIPS" round "$ARITHA/afloat.v" "$ARITHA/round_rint_expected.v" rint

echo "==> [clamp] --min 50 --max 200 on agray -> PNG"
"$VIPS" clamp "$ARITHA/agray.png" "$ARITHA/clamp_expected.png" --min 50 --max 200

# GOLDEN-ONLY (no vips oracle): the core Hough numerics diverge structurally from
# vips 8.18.4 (see the header), so these references are generated by `viprs`
# itself (deterministic) as regression pins, NOT cross-checked against vips.
echo "==> [hough_line] GOLDEN-ONLY viprs pin (256x256 accumulator .v)"
"$VIPRS" hough_line "$ARITHA/point.png" "$ARITHA/hough_line_golden.v" >/dev/null
echo "==> [hough_circle] GOLDEN-ONLY viprs pin (radii 2..4, scale 1, .v)"
"$VIPRS" hough_circle "$ARITHA/point.png" "$ARITHA/hough_circle_golden.v" 2 4 >/dev/null

# --- Provenance (append the aritha section) ----------------------------------
echo "==> [provenance] appending aritha section to $FIX_ROOT/PROVENANCE.md"
# ARITHMETIC — part B (the Wave-2 arith-b lane; OP_MAP.md arithmetic section,
# binary/relational/boolean/windowed/math/complex ops).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_arithb_diff.rs (which feeds them to `viprs`), so the
# two sides compare like against like. Carrier + oracle-class choices, all
# MEASURED against vips 8.18.4 on the committed inputs (max-abs-diff in raw
# sample units):
#
#   * subtract  → PNG, EXACT-AFTER-CAST tol 0 (core `try_sub` saturates the diff
#     at 0 for the unsigned output; integer inputs so no rounding — measured 0).
#     The a-b case has a clip dead-zone where a<b (both sides pinned at 0), so a
#     SECOND case subtracts `small` from `a` (a>=small everywhere) to exercise the
#     full range with no dead-zone (review finding 4).
#   * multiply  → `.v` ushort, EXACT-AFTER-CAST tol 0 (uchar*uchar widens to
#     ushort in both — measured 0).
#   * divide    → `.v` float, EXACT-AFTER-CAST tol 0 (both compute the quotient
#     in float — measured 0; the uchar save-cast path instead diverges by 1 LSB,
#     so the float `.v` carrier is used).
#   * minpair / maxpair → PNG, EXACT tol 0.
#   * sum       → `.v` ushort. vips `sum` promotes to UINT (band format the
#     libviprs decoder rejects) while core clamps into ushort; the reference is
#     therefore `vips cast … ushort` — lossless here (the 3-image sum ≤ 543 never
#     overflows) and matching core's ushort output exactly (measured 0).
#   * relational / relational_const → PNG, EXACT tol 0 (0/255 masks).
#   * boolean / boolean_const       → PNG, EXACT tol 0.
#   * scale (linear + --log)        → PNG, BOUNDED-TOL ≤1 LSB (log path is
#     transcendental; measured 0 on these inputs, tol 1 is the honest bound).
#   * stdif    → PNG, BOUNDED-TOL. Core CLIPS the window at the image border
#     while vips MIRRORS, so a genuinely 2-D input (the `eye` zone-plate) diverges
#     by up to 6 raw units in the 1px border ring. The interior is EXACT: the
#     differential compares the interior (1px margin cropped) at tol 0 and keeps
#     the tol-6 band ONLY for the whole-image border-inclusive case, so an
#     interior regression a flat tol 6 would absorb still fails (review finding 2).
#     Measured 6 whole-image / 0 interior on eye 3x3 — a documented core-vs-vips
#     edge-handling divergence (core issue #490), NOT hidden by a y-degenerate ramp.
#   * recomb   → PNG, BOUNDED-TOL 1. OP_MAP predicted EAC, but core rounds &
#     saturates per output band into the input depth while vips computes in float
#     and casts once, so they diverge by 1 LSB (measured 1). Honest class is
#     BOUNDED-TOL — a documented deviation from OP_MAP's EAC prediction.
#   * premultiply / unpremultiply   → PNG, BOUNDED-TOL 1 (vips emits float, core
#     rounds into the uchar depth; ≤1 LSB post-cast, measured 1).
#   * math (ALL 16 arms)            → `.v` float, FOURIER tol 1e-6 (measured 0).
#     sin/cos/atan on a.png; the other 13 on in-domain float inputs (msmall for
#     tan/asin/acos/log/log10/exp/exp10/sinh/cosh/tanh/asinh/atanh, macosh for
#     acosh) so EVERY dispatch arm has its own reference (review finding 1). The
#     float `.v` carrier is the honest class, not OP_MAP's EAC prediction (an
#     EAC-uchar save of sin's [-1,1] output would be near-vacuous).
#   * math2 atan2                   → `.v` float, FOURIER tol 1e-6 (measured 0).
#   * math2 pow / wop               → `.v` float, BOUNDED-TOL 1. The ONLY
#     divergent sample is `pow(0,0)`: Rust's `f64::powf(0,0)=1` vs libvips `0`.
#     Kept in the input (base and exponent both span 0) so the 0^0 edge is
#     EXERCISED, not hidden; tol 1, documented + filed as core issue #489. wop
#     (`right^left`) is the third math2 arm (review finding 1), sharing the edge.
#   * complexform / complex / complexget → `.v` float band-pairs, FOURIER eps
#     1e-6 (measured 0). vips complex-format outputs are REINTERPRETED to a
#     2-band float `.v` (`vips copy … --format float --bands 2`) because the
#     libviprs decoder rejects the native complex band format; the reinterpret is
#     a pure header relabel of the same interleaved (re,im) f32 bytes, matching
#     core's float-pair layout exactly.
# ===========================================================================
ARITHB="$FIX_ROOT/arithb"
mkdir -p "$ARITHB"

# --- Common inputs -----------------------------------------------------------
# `vips grey` is a pure coordinate function (0..1 float ramp), so every fixture
# is bit-reproducible. a: 16x16 Gray8 horizontal ramp 0..255; b: vertical ramp
# 40..168 (all > 0 — a safe, non-zero divisor for `divide`, and a and b straddle
# so subtract/relational/boolean vary in 2-D); c: a third distinct horizontal
# ramp 20..120 for the >=3-input `sum` fold.
echo "==> [arithb input] a/b/c 16x16 Gray8 ramps + rgb/rgba + small/small2 + eye"
"$VIPS" grey "$TMP/ag.v" 16 16
"$VIPS" linear "$TMP/ag.v" "$ARITHB/a.png" 255 0 --uchar
"$VIPS" rot "$TMP/ag.v" "$TMP/agv.v" d90
"$VIPS" linear "$TMP/agv.v" "$ARITHB/b.png" 128 40 --uchar
"$VIPS" linear "$TMP/ag.v" "$ARITHB/c.png" 100 20 --uchar
# rgb (srgb, 3-band) for recomb; rgba (srgb, 4-band, varying alpha = a) for
# (un)premultiply.
"$VIPS" bandjoin "$ARITHB/a.png $ARITHB/b.png $ARITHB/c.png" "$TMP/rgb.v"
"$VIPS" copy "$TMP/rgb.v" "$ARITHB/rgb.png" --interpretation srgb
"$VIPS" bandjoin "$ARITHB/rgb.png $ARITHB/a.png" "$TMP/rgba.v"
"$VIPS" copy "$TMP/rgba.v" "$ARITHB/rgba.png" --interpretation srgb
# small base (0..15) + small exponent (0..3) for math2 pow — both span 0 so the
# pow(0,0) edge is exercised; results stay small (<=3375) for f32 headroom.
"$VIPS" linear "$TMP/ag.v" "$ARITHB/small.png" 15 0 --uchar
"$VIPS" linear "$TMP/ag.v" "$ARITHB/small2.png" 3 0 --uchar
# eye: a 2-D zone-plate (varies in BOTH axes) so stdif's window statistics — and
# its border clip-vs-mirror divergence from vips — are genuinely exercised.
"$VIPS" eye "$TMP/eye.v" 16 16
"$VIPS" linear "$TMP/eye.v" "$ARITHB/eye.png" 127 128 --uchar
# avert: a.png rotated 90° (a vertical ramp over the SAME 0..255 value set), so
# `relational a.png avert.png equal` is TRUE on the x==y diagonal (16 pixels) and
# FALSE elsewhere. This makes the enum-discriminating relational cases non-vacuous:
# with a non-empty equality set, `lesseq`/`moreeq` provably differ from
# `less`/`more`, so a `lesseq`<->`less` (or `moreeq`<->`more`) dispatch swap fails
# (review finding 1 — enum coverage).
"$VIPS" rot "$ARITHB/a.png" "$ARITHB/avert.png" d90
# msmall / macosh: float `.v` inputs in the math ops' domains so EVERY math enum
# arm is compared against its own vips reference (review finding 1). msmall spans
# (0.1, 0.95): in-domain for asin/acos/atanh (|x|<1), log/log10 (>0), and bounded
# for exp/exp10/sinh/cosh (O(1) outputs, no f32 overflow), plus small trig angles
# (tan has no pole here). macosh spans [1, 6] for acosh (domain >=1), which msmall
# (<1 -> NaN) cannot cover. Both are pure `grey`-ramp linears, bit-reproducible.
"$VIPS" linear "$TMP/ag.v" "$ARITHB/msmall.v" 0.85 0.1
"$VIPS" linear "$TMP/ag.v" "$ARITHB/macosh.v" 5 1
# recomb coefficient matrix (3 output bands x 3 input bands), a vips text matrix.
printf '3 3\n0.5 0.25 0.25\n0.2 0.6 0.2\n0.1 0.3 0.6\n' > "$ARITHB/recomb.mat"
# complex input in TWO carriers from complexform(a,b): the complex-format `.v`
# (used ONLY here to drive the vips complex/complexget references) and the
# 2-band float reinterpret committed as the viprs-side input (identical bytes).
"$VIPS" complexform "$ARITHB/a.png" "$ARITHB/b.png" "$TMP/cpx_c.v"
"$VIPS" copy "$TMP/cpx_c.v" "$ARITHB/complex_in.v" --format float --bands 2

# --- References — one vips run per differential case -------------------------
echo "==> [subtract] a - b (EAC, PNG; negatives clip to 0)"
"$VIPS" subtract "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/subtract_expected.png"
# a>=b everywhere (a horizontal ramp 0..255, small horizontal ramp 0..15, so a(x)
# >= small(x) at every pixel): the difference is non-negative across the WHOLE
# image, so there is no clip dead-zone and the full subtraction range is exercised
# (review finding 4 — the a<b half of the a-b case is vacuous because BOTH core
# and vips pin negatives to 0; core's `try_sub` itself saturates at 0, so a `.v`
# carrier could not recover the negatives — the a>=b case is the honest fix).
echo "==> [subtract] a - small (EAC, PNG; a>=small everywhere, full range)"
"$VIPS" subtract "$ARITHB/a.png" "$ARITHB/small.png" "$ARITHB/subtract_pos_expected.png"
echo "==> [multiply] a * b (EAC, ushort .v)"
"$VIPS" multiply "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/multiply_expected.v"
echo "==> [divide] a / b (EAC, float .v)"
"$VIPS" divide "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/divide_expected.v"
echo "==> [minpair/maxpair] (EXACT, PNG)"
"$VIPS" minpair "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/minpair_expected.png"
"$VIPS" maxpair "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/maxpair_expected.png"
echo "==> [sum] a + b + c (>=3 variadic, EAC; vips UINT cast to ushort .v)"
"$VIPS" sum "$ARITHB/a.png $ARITHB/b.png $ARITHB/c.png" "$TMP/sum_u32.v"
"$VIPS" cast "$TMP/sum_u32.v" "$ARITHB/sum_expected.v" ushort
# ALL SIX relational enum arms (review finding 1): more/less on (a, b); and
# equal/noteq/lesseq/moreeq on (a, avert), whose x==y diagonal makes equality
# non-empty so each arm is discriminated (lesseq != less, moreeq != more, equal !=
# noteq). One vips reference per arm → a dispatch swap in any arm fails.
echo "==> [relational] more + less on (a,b); equal/noteq/lesseq/moreeq on (a,avert) (EXACT, PNG)"
"$VIPS" relational "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/relational_more_expected.png" more
"$VIPS" relational "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/relational_less_expected.png" less
"$VIPS" relational "$ARITHB/a.png" "$ARITHB/avert.png" "$ARITHB/relational_equal_expected.png" equal
"$VIPS" relational "$ARITHB/a.png" "$ARITHB/avert.png" "$ARITHB/relational_noteq_expected.png" noteq
"$VIPS" relational "$ARITHB/a.png" "$ARITHB/avert.png" "$ARITHB/relational_lesseq_expected.png" lesseq
"$VIPS" relational "$ARITHB/a.png" "$ARITHB/avert.png" "$ARITHB/relational_moreeq_expected.png" moreeq
# ALL SIX relational_const enum arms (review finding 1): more 128 on a; and
# equal/noteq/lesseq/moreeq against C=7 on small.png, whose samples are EXACTLY
# 0..15 so `== 7` is a non-empty column → lesseq != less, equal != noteq.
echo "==> [relational_const] more 128 on a; equal/noteq/lesseq/moreeq 7 on small (EXACT, PNG)"
"$VIPS" relational_const "$ARITHB/a.png" "$ARITHB/relational_const_more_expected.png" more 128
"$VIPS" relational_const "$ARITHB/small.png" "$ARITHB/relational_const_equal_expected.png" equal 7
"$VIPS" relational_const "$ARITHB/small.png" "$ARITHB/relational_const_noteq_expected.png" noteq 7
"$VIPS" relational_const "$ARITHB/small.png" "$ARITHB/relational_const_lesseq_expected.png" lesseq 7
"$VIPS" relational_const "$ARITHB/small.png" "$ARITHB/relational_const_moreeq_expected.png" moreeq 7
# ALL THREE boolean enum arms (review finding 1): and/eor already; add or.
echo "==> [boolean] eor + and + or (EXACT, PNG)"
"$VIPS" boolean "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/boolean_eor_expected.png" eor
"$VIPS" boolean "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/boolean_and_expected.png" and
"$VIPS" boolean "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/boolean_or_expected.png" or
# ALL FIVE boolean_const enum arms (review finding 1): and/lshift already; add
# or, eor (mask 200) and rshift 2 (the second shift arm, so an rshift-maps-to-
# lshift bug fails since lshift is also tested).
echo "==> [boolean_const] and 200 + or 200 + eor 200 + lshift 2 + rshift 2 (EXACT, PNG)"
"$VIPS" boolean_const "$ARITHB/a.png" "$ARITHB/boolean_const_and_expected.png" and 200
"$VIPS" boolean_const "$ARITHB/a.png" "$ARITHB/boolean_const_or_expected.png" or 200
"$VIPS" boolean_const "$ARITHB/a.png" "$ARITHB/boolean_const_eor_expected.png" eor 200
"$VIPS" boolean_const "$ARITHB/a.png" "$ARITHB/boolean_const_lshift_expected.png" lshift 2
"$VIPS" boolean_const "$ARITHB/a.png" "$ARITHB/boolean_const_rshift_expected.png" rshift 2
echo "==> [scale] linear + --log (BOUNDED-TOL <=1 LSB, PNG)"
"$VIPS" scale "$ARITHB/rgb.png" "$ARITHB/scale_expected.png"
"$VIPS" scale "$ARITHB/rgb.png" "$ARITHB/scale_log_expected.png" --log
echo "==> [stdif] 3x3 on 2-D eye (EXACT; core #490 edge-replicate matches vips whole-image, PNG)"
"$VIPS" stdif "$ARITHB/eye.png" "$ARITHB/stdif_expected.png" 3 3
echo "==> [recomb] 3x3 matrix (EXACT; core #491 f32-then-truncate bit-exact, PNG)"
"$VIPS" recomb "$ARITHB/rgb.png" "$ARITHB/recomb_expected.png" "$ARITHB/recomb.mat"
echo "==> [premultiply/unpremultiply] rgba (BOUNDED-TOL 1, PNG)"
"$VIPS" premultiply "$ARITHB/rgba.png" "$ARITHB/premultiply_expected.png"
"$VIPS" unpremultiply "$ARITHB/rgba.png" "$ARITHB/unpremultiply_expected.png"
# ALL SIXTEEN math enum arms (review finding 1): sin/cos/atan on a.png (degrees,
# defined over 0..255); the other thirteen on the in-domain float inputs so each
# dispatch arm is compared against its OWN vips reference (a log<->log10,
# exp<->exp10 or sinh<->cosh swap fails). tan/asin/acos/log/log10/exp/exp10/
# sinh/cosh/tanh/asinh/atanh on msmall (0.1..0.95); acosh on macosh (>=1).
echo "==> [math] all 16 arms (float .v, eps 1e-6; measured 0)"
"$VIPS" math "$ARITHB/a.png" "$ARITHB/math_sin_expected.v" sin
"$VIPS" math "$ARITHB/a.png" "$ARITHB/math_cos_expected.v" cos
"$VIPS" math "$ARITHB/a.png" "$ARITHB/math_atan_expected.v" atan
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_tan_expected.v" tan
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_asin_expected.v" asin
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_acos_expected.v" acos
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_log_expected.v" log
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_log10_expected.v" log10
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_exp_expected.v" exp
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_exp10_expected.v" exp10
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_sinh_expected.v" sinh
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_cosh_expected.v" cosh
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_tanh_expected.v" tanh
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_asinh_expected.v" asinh
"$VIPS" math "$ARITHB/msmall.v" "$ARITHB/math_atanh_expected.v" atanh
"$VIPS" math "$ARITHB/macosh.v" "$ARITHB/math_acosh_expected.v" acosh
# ALL THREE math2 arms (review finding 1): atan2 + pow already; add wop. wop is
# `right^left`, so `math2 small2 small wop` = small^small2 (same magnitude as the
# pow case) and shares the 0^0 edge at x=0. Core issue #489 made pow(0,≤0)=0
# (matching vips; previously Rust f64::powf(0,0)=1), so pow/wop now match vips
# bit-for-bit (tol 0); atan2 stays FOURIER float (eps 1e-6).
echo "==> [math2] atan2 (eps 1e-6) + pow + wop (EXACT tol 0, 0^0 edge; float .v)"
"$VIPS" math2 "$ARITHB/a.png" "$ARITHB/b.png" "$ARITHB/math2_atan2_expected.v" atan2
"$VIPS" math2 "$ARITHB/small.png" "$ARITHB/small2.png" "$ARITHB/math2_pow_expected.v" pow
"$VIPS" math2 "$ARITHB/small2.png" "$ARITHB/small.png" "$ARITHB/math2_wop_expected.v" wop
echo "==> [complexform] a + i*b (FOURIER, float band-pair .v via reinterpret)"
"$VIPS" complexform "$ARITHB/a.png" "$ARITHB/b.png" "$TMP/cf_c.v"
"$VIPS" copy "$TMP/cf_c.v" "$ARITHB/complexform_expected.v" --format float --bands 2
echo "==> [complex] polar + rect + conj (FOURIER, reinterpret)"
"$VIPS" complex "$TMP/cpx_c.v" "$TMP/cpol.v" polar
"$VIPS" copy "$TMP/cpol.v" "$ARITHB/complex_polar_expected.v" --format float --bands 2
"$VIPS" complex "$TMP/cpx_c.v" "$TMP/crect.v" rect
"$VIPS" copy "$TMP/crect.v" "$ARITHB/complex_rect_expected.v" --format float --bands 2
"$VIPS" complex "$TMP/cpx_c.v" "$TMP/cconj.v" conj
"$VIPS" copy "$TMP/cconj.v" "$ARITHB/complex_conj_expected.v" --format float --bands 2
echo "==> [complexget] real + imag (FOURIER, plain float .v)"
"$VIPS" complexget "$TMP/cpx_c.v" "$ARITHB/complexget_real_expected.v" real
"$VIPS" complexget "$TMP/cpx_c.v" "$ARITHB/complexget_imag_expected.v" imag

# --- Provenance (append the arithb section) ----------------------------------
echo "==> [provenance] appending arithb section to $FIX_ROOT/PROVENANCE.md"
cat >> "$FIX_ROOT/PROVENANCE.md" <<EOF

---

# convolution family CLI-differential reference provenance

These fixtures are the committed vips oracle references the convolution
CLI-differential suite (\`tests/cli_convolution_diff.rs\`) decode-compares
# aritha family (arith part-A) CLI-differential reference provenance

These fixtures are the committed vips oracle references (and two viprs GOLDEN-ONLY
pins) the aritha CLI-differential suite (\`tests/cli_aritha_diff.rs\`) compares
\`viprs\` output against. Generated offline by \`tools/gen_cli_expected.sh\`,
NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`, SIMD targets \`$VIPS_TARGETS\`
- **libvips path pinned**: every **uchar integer-precision convolution**
  reference here is generated with \`VIPS_NOVECTOR=1\`, i.e. against
  \`vips_convi_gen\`, the portable C loop libvips's own docs call the
  specification (\`convi.c:1271-1284\`: "For UCHAR images, vips_convi uses a
  fast vector path based on half-float arithmetic. **This can produce slightly
  different results.**"). libviprs implements that path, so these references are
  EXACT rather than tolerance-banded. Float precision, ushort inputs and
  \`sharpen\` are path-independent and are generated on the plain binary.
  Issue #558.
- **Common inputs** (under \`convolution/\`): \`eye.png\` (16×16 Gray8 zone-plate,
  high-frequency so a blur/edge op is non-vacuous — a box blur moves it by up to
  59, compass by 254; \`sharpen\` runs on that same mono zone-plate),
  \`patch.png\` (5×5 extract of \`eye.png\`, the correlation template with a
  sharp best-match peak).
- **#558 discriminating inputs**: \`noise64.png\` (64×64 Gray8,
  \`gaussnoise --seed 42\`; high entropy is what makes a hostile mask hostile —
  the same mask reaches 2 on the zone-plate and 57 here), \`boxsum1147.png\`
  (3×3, eight 127s around a 131, so every replicated window sums to 1147),
  \`eye16.v\` (16×16 **ushort** zone-plate, the regime where the vector path is
  gated off entirely by \`convi.c:1151\`).
- **Masks**: \`blur.mat\` (3×3 box, scale 9), \`sobel.mat\` (3×3 edge, scale 1,
  odd-square for \`compass\`), \`sep.mat\` (1×5 separable smoother, scale 10),
  \`hostile.mat\` (3×3 scale 3, \`[45 -17 -25 / -33 -15 -34 / 55 53 -26]\` — a
  mask \`vips_convi_intize\` **accepts** and on which the two libvips paths
  differ by 57).
- **Carriers**: integer uchar outputs → PNG; float / matrix / correlation
  surfaces → native \`.v\`. vips writes the gaussmat/logmat matrix as **double**
  and fastcor as **uint**, neither of which the libviprs decoder reads, so they
  are \`vips cast … float\`ed to a float \`.v\` (lossless for these values).

## Honest oracle classes (measured)

- **EXACT (tol 0)**: \`compass --combine max\` integer (scale-1 sobel mask needs
  no coefficient rounding), \`compass --combine sum\` integer (the deterministic
  16-bit-promotion + saturation branch; summed sobel edges reach 812; vips
  emits uint, core emits Gray16, cast to ushort \`.v\`), and \`fastcor\` (integer SSD). Also gaussmat/logmat at
  **integer** precision (integer-valued matrices).
- **EXACT (tol 0), integer convolution on uchar**: \`conv\`, \`convsep\` and
  \`gaussblur\` at **integer** precision, against the \`VIPS_NOVECTOR=1\`
  references. These used to carry a ≤1 LSB tolerance whose justification cited
  vips shifting by \`>> sexp\`; \`sexp\` is the **ORC** variable and is never set
  in a HWY build, so that described a code path absent from the binary it was
  measured against. And ≤1 was false: on this same \`eye.png\`,
  \`gaussblur … 1.6\` moves 256 samples by up to 4 and \`convsep … sep.mat\` by
  4. The real mechanism is that the vector path convolves with **requantised
  coefficients** — \`conv_boxsum1147_int_expected.png\` isolates it: on a window
  summing to 1147, C gives \`(1147 + 4) / 9 = 127\`, **floor gives 127 too**, and
  the vector path gives \`(57 * 1147 + 256) >> 9 = 128\` because it filters with
  \`57/512\`, not \`1/9\`.
- **BOUNDED-TOL ≤1 LSB (tol 1), \`sharpen\` ONLY**: the LabS unsharp round trip.
  This is **not** the tolerance above and must not be merged back into it.
  \`sharpen\` convolves the L of LabS, which is 16-bit, and the vector path is
  gated on \`BandFmt == VIPS_FORMAT_UCHAR\` (\`convi.c:1151\`), so it takes the
  portable C path on both builds — \`VIPS_INFO=1\` reports "convi: using C
  path". #558 therefore cannot be its mechanism. The deviation is a real
  libviprs bug, tracked as issue **#581**, which the shared constant was hiding.
- **BOUNDED-TOL float (small eps)**: \`conv\`/\`compass\`/\`gaussblur\` at float
  precision, \`convsep\` (measured ≤1.5e-5 on the author Mac), \`gaussmat\`/\`logmat\`
  float precision, and \`spcor\` (eps 1e-5): float surfaces whose accumulation
  order / transcendental libm differs slightly from vips.

## Exact commands

Inputs + masks:

\`\`\`
vips eye veye.v 16 16 ; vips linear veye.v convolution/eye.png 127.5 127.5 --uchar
vips extract_area convolution/eye.png convolution/patch.png 4 4 5 5
vips gaussnoise gn.v 64 64 --mean 128 --sigma 50 --seed 42
vips cast gn.v convolution/noise64.png uchar
vips black bx.v 3 3 ; vips linear bx.v bx127.v 0 127 --uchar
vips copy bx127.v bxdraw.v ; vips draw_rect bxdraw.v 131 1 1 1 1
vips copy bxdraw.v convolution/boxsum1147.png
vips linear veye.v eye16f.v 32767.5 32767.5 ; vips cast eye16f.v convolution/eye16.v ushort
printf '3 3 9\\n1 1 1\\n1 1 1\\n1 1 1\\n'    > convolution/blur.mat
printf '3 3 1\\n1 2 1\\n0 0 0\\n-1 -2 -1\\n' > convolution/sobel.mat
printf '5 1 10\\n1 2 4 2 1\\n'              > convolution/sep.mat
printf '3 3 3 0\\n45 -17 -25\\n-33 -15 -34\\n55 53 -26\\n' > convolution/hostile.mat
# matrix family CLI-differential reference provenance

These fixtures are the committed vips oracle references the matrix
CLI-differential suite (\`tests/cli_matrix_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`matrix/\`, vips text-matrix files): \`m3.mat\` (3x3,
  the matrixinvert **direct cofactor** path, n<4), \`m4.mat\` (4x4, the
  matrixinvert **PLU decomposition** path, n>=4), \`lut.mat\` (3x3 measured
  points: column 0 = input level, columns 1/2 = two bands' responses, all in
  0..=1, for invertlut). Consumed identically by both vips and the \`viprs\`
  \`MatFile\` loader (no scale/offset header, so the raw cells are read on both
  sides).
- **Oracle class BOUNDED-TOL (f32 carrier)**: the core computes in f64 but stores
  results as **f32** (libvips stores double), so every reference is the vips
  double result **cast to float** (\`vips cast … float\`) — the libviprs \`.v\`
  decoder rejects a DOUBLE band format, and the compare is f32-vs-f32. Measured
  max-abs-diff: \`0\` for \`matrixinvert\` (both paths), \`5.96e-8\` (one f32 ULP
  at 1.0) for \`invertlut\`. The test tol is \`1e-6\`; the OP_MAP \`1e-9\` was
  written assuming a double carrier and is unreachable with f32.
# colour family CLI-differential reference provenance

These fixtures are the committed vips oracle references the colour
CLI-differential suite (\`tests/cli_colour_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`colour/\`): \`rgb.png\` / \`rgb2.png\` (two DISTINCT
  16×16 sRGB images with 2-D structure — band0 horizontal ramp, band1 scaled
  horizontal ramp, band2 vertical ramp; distinct so the ΔE metrics are
  non-vacuous), \`sRGB.icc\` (a v2.1 **matrix-shaper** sRGB profile: colorant
  matrix + per-channel TRC, no A2B LUT), \`icc_pcs_lab.v\` (a D50 Lab PCS image =
  \`vips icc_import rgb.png … --input-profile sRGB.icc\`, the shared \`icc_export\`
  input).
- **Oracle classes**: every case is **BOUNDED-TOL** at a MEASURED tolerance
  EXCEPT \`dECMC\`, which is **GOLDEN-ONLY**. Measured max-abs-diff per case:
  colourspace LAB 4.6e-5 / XYZ 1.5e-5 / scRGB 1e-6 (float, tol 1e-4); the #36
  LAB→PNG save 0, the LAB-tagged-input→sRGB PNG non-round-trip discriminator 1,
  and the \`--source-space\` override 1 (uchar, tol 1 = ≤1 LSB); dE76 / dE00
  6.5e-5 (float, tol 1e-4); icc_import 0.303 (Lab float, tol 0.35 —
  moxcms-vs-lcms2 matrix-shaper divergence, NOT a bug); icc_export / icc_transform
  0 (uchar, tol 2 = ≤2 LSB margin for cross-CMS / cross-arch); icc_export --depth
  16 13 (16-bit, tol 16 — the moxcms-vs-lcms2 divergence the 8-bit path rounds away).

## dECMC — no vips cross-oracle (a real formula difference, GOLDEN-ONLY)

vips \`dECMC\` approximates the CMC colour difference as **Euclidean distance in
its CMC uniform space**; libviprs computes the **published CMC(l:c) ΔE at
l=c=1** (BS 6923). These are different functions — on \`rgb\`/\`rgb2\` the two
diverge by ~297 (vips range [13.17, 311.39] vs core [7.75, 99.71]) — so there is
no meaningful cross-oracle. \`dECMC_golden.v\` is generated by **\`viprs\`**
itself (deterministic across runs) and the test is a regression pin, NOT a vips
comparison. (OP_MAP.md provisionally listed \`dECMC\` BOUNDED-TOL 1e-4; this
honest measurement corrects it to GOLDEN-ONLY.)

## ICC — matrix-shaper only (moxcms vs lcms2)

libviprs ships a native pure-Rust ICC engine (moxcms); homebrew vips uses lcms2.
The two agree on the matrix-shaper sRGB profile used here: the device-space
round trips (\`icc_export\`, \`icc_transform\`) reproduce vips's output EXACTLY
(measured 0), and the intermediate D50 Lab PCS (\`icc_import\`) agrees to ~0.31
Lab units. CMYK / LUT profiles interpolate different grids between the two CMSs
and diverge by design — they are out of scope for this cross-oracle.
# composite family CLI-differential reference provenance

These fixtures are the committed vips oracle references (and \`viprs\` golden
pins) the composite CLI-differential suite (\`tests/cli_composite_diff.rs\`)
decode-compares \`viprs\` output against. Generated offline by
\`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`composite/\`): \`base.png\`, \`overlay.png\` (8×8 sRGB
  RGBA with 2-D structure and a VARYING alpha, so translucent compositing is
  exercised at every alpha), \`base_op.png\`, \`overlay_op.png\` (their opaque
  3-band RGB counterparts).
- **Op**: the core blends exactly two images with one blend mode
  (\`try_composite2\`); both \`composite\` (vips array form) and \`composite2\`
  (vips pair form) map onto it. \`MODE\` is one of the 25 vips \`VipsBlendMode\`
  spellings; the four core-only non-separable modes
  (hue/saturation/colour/luminosity) are NOT exposed (no vips oracle). vips's
  \`--x\`/\`--y\`/\`--compositing-space\`/\`--premultiplied\` are not in core and
  keep their defaults (0/0/srgb/false).

## Honest oracle split — a measured core-vs-vips divergence (flagged)

Compositing **opaque** inputs, all 25 modes agree with vips to **≤1 LSB**. With
**translucent** alpha, only the Porter-Duff *simple* operators
(clear/source/over/in/out/dest/dest-over/dest-in/dest-out/xor/add) still agree
≤1 LSB; the 11 PDF separable blends AND the alpha-weighted Porter-Duff operators
**atop/dest-atop/saturate** diverge **wholesale** (measured max-abs-diff up to
215), because the core's translucent blend-composite formula differs from
libvips'. Accordingly:

- Porter-Duff simple modes are pinned on the **translucent** inputs (real vips
  oracle, arbitrary alpha, tol 1).
- PDF separable blends are pinned on the **opaque** inputs (real vips oracle,
  non-vacuous blend-function coverage, tol 1).
- The translucent divergence is pinned separately as **GOLDEN-ONLY** \`viprs\`
  regression pins (multiply/atop/saturate on the translucent inputs) — there is
  no cross-oracle for translucent blend compositing. This is tracked as a filed
  GitHub issue (aligning the core's translucent blend-composite formula with
  libvips), NOT a comment-only open question. If a core translucent-blend fix
  lands, these pins are EXPECTED to fail: REGENERATE and re-bless them, do not
  revert the core change.
- The remaining nine modes (clear/out/dest/dest-in/dest-out/dest-atop/lighten/
  colour-burn/soft-light) are pinned on the **opaque** inputs (real vips oracle,
  tol 1) so that every one of the 25 \`VipsBlendMode\` spellings is discriminated
  against vips — a mode->variant mis-wiring cannot pass CI unnoticed.
# freqfilt family CLI-differential reference provenance

These fixtures are the committed vips oracle references the freqfilt
CLI-differential suite (\`tests/cli_freqfilt_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`freqfilt/\`): \`in.png\` (16×16 Gray8 2-D gradient,
  \`x*85 + y*170\`, varying in BOTH axes so every transform is non-vacuous),
  \`mask.v\` (\`mask_ideal 16 16 0.3\` — a float low-pass mask that changes the
  gradient by max-abs 114; the 2nd input to \`freqmult\`), \`shifted.png\`
  (\`in.png\` wrapped by (3,2), so \`phasecor\`'s peak sits at that translation).
- **Every op is oracle class FOURIER** (CLI_CONTRACT.md §5). Measured
  viprs-vs-vips max-abs-diff (release build): fwfft 1.1e-16, invfft 2.8e-14,
  invfft --real 0, roundtrip 3.8e-6, phasecor 0 (all float \`.v\`); spectrum 0,
  freqmult 1 (uchar PNG). The float ops are compared at a small absolute eps
  sized above each op's f32-quantisation floor (fwfft 1e-2 at peak ~127; invfft
  5e-2 at peak ~3.3e4; phasecor / roundtrip 1e-2); spectrum at tol 0; freqmult at
  tol 1 (the float round-trip cast back to uchar rounds ±1 vs vips — a genuine,
  measured BOUNDED-TOL divergence, the same shape as the bands \`bandmean\` and
  conversion \`gamma\` cases).
- **Carriers** (CRITICAL): the libviprs \`.v\` decoder accepts ONLY uchar/ushort/
  float band formats and REJECTS vips \`dpcomplex\`(10) / \`double\`(8). So every
  complex vips output is normalised OFFLINE to a 2-band float \`.v\` (band0 = re,
  band1 = im) via \`complexget real\` + \`complexget imag\` + \`bandjoin\` +
  \`cast float\` — the exact (re, im)-pair layout libviprs' \`fwfft\` writes — and
  every real vips output is \`cast float\` to an f32 \`.v\`. \`spectrum\`/\`freqmult\`
  are uchar → PNG.

## Complex-INPUT carrier limitation (flagged; open question)

A single committed \`.v\` cannot feed a COMPLEX input to BOTH sides: vips reads a
complex image only in its own \`dpcomplex\`/\`complex\` band format (which the
libviprs decoder rejects), while libviprs treats a Fourier-domain image as an
even-band FLOAT raster stamped \`fourier\` (which vips reads as N independent real
bands, not one complex pair). The two complex carriers are mutually unreadable in
one file. The differential therefore drives the complex-input code path via the
invfft(fwfft(in)) --real ROUND-TRIP (each side chains its OWN complex carrier),
which is the only case exercising \`viprs invfft\`'s \`is_fourier_complex\` branch;
the core's direct complex-input paths are covered by the core crate's own
freqfilt unit tests. Teaching one decoder to read the other's complex carrier
(or a shared interchange format) would let a committed complex INPUT feed both.
# mosaicing family CLI-differential reference provenance

These fixtures are the committed references the mosaicing CLI-differential suite
(\`tests/cli_mosaicing_diff.rs\`) decode-compares \`viprs\` output against.
Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **\`merge\` / \`mosaic\` are EXACT** (decode-compare tol 0): the integer feather
  ramp and the DISCRETE tie-point search agree with vips bit-for-bit on these
  inputs (verified max-abs-diff 0). vips's CLI needs a \`--\` separator to pass a
  negative \`merge\` displacement (\`vips merge … horizontal -- -28 0\`); \`viprs\`
  takes the negative positional directly, so both see the same DX/DY.
- **\`globalbalance\` is GOLDEN-ONLY**: NO vips cross-oracle exists — the core
  reads the \`mosaic-join-tree\` blob only viprs merge/mosaic writes, while vips's
  globalbalance reads its own filename-based history (mutually unreadable). The
  input \`balance_input.v\` is minted by \`viprs merge\` and the reference
  \`balance_expected.v\` by \`viprs globalbalance\` — a deterministic regression
  pin, not a parity check.
- **Common inputs** (under \`mosaicing/\`): \`merge_ref.png\`/\`merge_sec.png\`
  (40x32 Gray8, distinct-seed textures for the non-vacuous seam blend),
  \`merge_rgb.png\` (40x32 sRGB — the format-mismatch error SEC AND the RGB-merge
  REF), \`merge_rgb_sec.png\` (40x32 sRGB, distinct seeds — the RGB-merge SEC),
  \`mosaic_h_ref.png\`/\`mosaic_h_sec.png\` (100x150 crops of a 110x150 noise
  scene, x-offset 10 → 90x150 overlap), \`mosaic_v_ref.png\`/\`mosaic_v_sec.png\`
  (150x100 crops of a 150x110 scene, y-offset 10). \`mosaic\` inputs are large
  because its search needs 3 strips × 20 high-contrast windows — an inherent
  property of the op, not a fixture choice. \`balance_input.v\` is a viprs mosaic
  (INPUT only, carries the blob).
- **Multi-band + insert-fallback merge coverage** (adversarial-review finding 1):
  the single-band Gray8 fixtures left the multi-band \`render_merge\` band path
  and the wrong-side/disjoint INSERT-FALLBACK branch (paste both, no blend,
  output sized by \`rarea.union(&sarea)\`) unpinned. \`merge_rgb_expected.png\`
  (RGB horizontal) pins the former and \`merge_fallback_expected.png\` (positive
  dx 12) the latter — both verified bit-exact vs vips (max-abs-diff 0).
- **Common inputs** (under \`aritha/\`): \`agray.png\` (16×16 Gray8 ramp),
  \`afloat.v\` (4×4 float, hand-picked discriminating samples — crosses zero,
  includes exactly 0.0 for \`sign\`'s zero branch, exact half-integers for \`rint\`'s
  half-rule, and non-half fractionals so \`rint\`≠\`floor\`≠\`ceil\`; all dyadic, no
  float wobble — for abs/sign/round),
  \`content.png\` (black 6×7 block on white 20×20, for \`find_trim\`),
  \`content2.png\` (white 5×4 block on black 20×20, for \`find_trim --background 0\`),
  \`pzero.png\` (8×8 zero + 3×3 block at 2,3, for \`profile\`), \`point.png\` (32×32
  black, one white pixel at 10,6, for the hough golden pins).
- **Carriers**: S3 scalars → \`.txt\` (numeric compare, never text). \`stats\`/
  \`measure\` double matrices → \`vips … ; vips cast … float\` \`.v\` (libviprs has no
  f64 pixel format); \`stats\` is cropped to the 6 core columns (min/max/sum/sum2/
  mean/sd — vips's 4 position columns 6..10 are a documented core subset gap).
  \`profile\`/\`project\` are 16-bit — vips emits INT/UINT, cast to ushort (lossless
  for the small values) to match the core's ushort carrier — \`.v\`. \`linear\`
  (float), \`math2_const\`, \`abs\`, \`sign\`, \`round\` → float \`.v\`; \`linear --uchar\`,
  \`remainder_const\`, \`clamp\` → PNG.

## \`round rint\` — now EXACT against a real vips oracle

\`round rint\` once GENUINELY diverged from vips 8.18.4 at exact half-integers: the
core mapped \`f64::round\` (rounds half AWAY from zero: 0.5→1, 2.5→3, −2.5→−3)
while vips's C \`rint\` rounds half TO EVEN (0.5→0, 2.5→2, −2.5→−2). Core issue
#494 changed the core to round half-to-even, so on the honest afloat (which
reaches x.5) \`round rint\` now matches vips bit-for-bit and is carried EXACT
(tol 0) against a genuine vips oracle (\`round_rint_expected.v\`). The former
GOLDEN-ONLY viprs pin (\`round_rint_golden.v\`) is retired. \`ceil\`/\`floor\` have no
tie and were and remain EXACT against vips.

## Hough (still GOLDEN-ONLY, for narrower reasons)

Core issue #495 fixed \`hough_line\`'s distance binning, so its accumulator vote
PATTERN now matches vips 8.18.4 bit-for-bit; what remains is an INHERENT
format/saturation gap — the core accumulates into Gray16 (u16) while vips uses a
uint accumulator, so at a peak of >65535 collinear votes the core saturates and
vips does not (the core has no u32 carrier). \`hough_circle\` still diverges
STRUCTURALLY: a single voting point yields a core per-cell max of **1** but a
vips per-cell max of **4** (a different vote model, not a bounded tolerance).
Neither has a full-width vips oracle, so the references \`hough_line_golden.v\` /
\`hough_circle_golden.v\` are minted by \`viprs\` itself (deterministic) and the
tests are regression pins.
# arithmetic part-B (arith-b lane) CLI-differential reference provenance

Committed vips oracle references the arith-b CLI-differential suite
(\`tests/cli_arithb_diff.rs\`) decode-compares \`viprs\` output against. Generated
offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`arithb/\`): \`a.png\` (16×16 Gray8 horizontal ramp
  0..255), \`b.png\` (vertical ramp 40..168; > 0 divisor, straddles \`a\` in 2-D),
  \`c.png\` (horizontal ramp 20..120; third \`sum\` input), \`rgb.png\` (their
  bandjoin, sRGB) and \`rgba.png\` (\`rgb\` + \`a\` alpha, sRGB) for
  recomb/(un)premultiply, \`small.png\` (0..15) + \`small2.png\` (0..3) for the
  \`math2 pow\`/\`wop\` 0^0 edge (and \`small\` for \`subtract a - small\` a>=b and
  the \`relational_const\` == 7 arm), \`avert.png\` (\`a\` rotated d90 → x==y-diagonal
  equality for the relational enum arms), \`msmall.v\` (16×16 float 0.1..0.95, the
  math ops' domain) + \`macosh.v\` (16×16 float 1..6, for \`acosh\`), \`eye.png\`
  (2-D zone-plate) for \`stdif\`, \`recomb.mat\` (3×3 coefficient matrix), and
  \`complex_in.v\` (a 2-band float reinterpret of \`complexform a b\`, the
  viprs-side complex input).
- **Carriers / oracle classes**: PNG + tol 0 for the integer-exact ops
  (subtract EAC, minpair/maxpair EXACT, relational(_const)/boolean(_const)
  EXACT); native \`.v\` for the ushort (\`multiply\`, \`sum\` cast from vips UINT)
  and float (\`divide\`, \`math\`, \`math2\`, complex) outputs the PNG path would
  lose. **BOUNDED-TOL** (measured, honest): \`scale\` ≤1 LSB (log transcendental),
  \`recomb\` 1 (per-band round+saturate vs vips float-then-cast — a documented
  deviation from OP_MAP's EAC prediction), \`premultiply\`/\`unpremultiply\` 1,
  \`stdif\` 6 (core clips the border window while vips mirrors — an edge-handling
  divergence, core issue #490, exercised on a 2-D input not hidden), \`math2
  pow\` 1 (the single \`pow(0,0)\` sample: Rust \`f64::powf(0,0)=1\` vs libvips 0,
  filed as an issue). **FOURIER** eps 1e-6 for the complex band-pair ops; the
  vips complex-format outputs are reinterpreted to 2-band float \`.v\`
  (\`vips copy … --format float --bands 2\`, a header relabel of the same
  interleaved (re,im) f32 bytes) because the libviprs decoder rejects the native
  complex band format.

## Exact commands

Inputs:

\`\`\`
printf '3 3\\n2 0 1\\n1 3 0\\n0 1 4\\n'                > matrix/m3.mat
printf '4 4\\n2 1 0.5 0\\n1 3 0 1\\n0 1 4 2\\n1 0 2 5\\n' > matrix/m4.mat
printf '3 3\\n0.1 0.2 0.3\\n0.2 0.4 0.4\\n0.7 0.5 0.6\\n' > matrix/lut.mat
vips grey clg.v 16 16 ; vips rot clg.v clg_v.v d90
vips linear clg.v   cl_b0.png 255 0  --uchar
vips linear clg.v   cl_b1.png 200 20 --uchar
vips linear clg_v.v cl_b2.png 255 0  --uchar
vips bandjoin "cl_b0.png cl_b1.png cl_b2.png" cl_rgb.v
vips copy cl_rgb.v colour/rgb.png --interpretation srgb
vips linear clg.v   cl_c0.png 150 40 --uchar
vips linear clg_v.v cl_c1.png 150 50 --uchar
vips linear clg.v   cl_c2.png 120 60 --uchar
vips bandjoin "cl_c0.png cl_c1.png cl_c2.png" cl_rgb2.v
vips copy cl_rgb2.v colour/rgb2.png --interpretation srgb
cp "/System/Library/ColorSync/Profiles/sRGB Profile.icc" colour/sRGB.icc
vips icc_import colour/rgb.png colour/icc_pcs_lab.v --input-profile colour/sRGB.icc --intent relative
# histogram family CLI-differential reference provenance

These fixtures are the committed vips oracle references (EXACT / BOUNDED-TOL) and
the \`viprs\`-generated regression pins (GOLDEN-ONLY) the histogram
CLI-differential suite (\`tests/cli_histogram_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common image inputs** (under \`histogram/\`): \`gray.png\` (16×16 Gray8 ramp),
  \`rgb.png\` (16×16 sRGB 3-band: band 0 = horizontal ramp, band 1 = a scaled
  ramp (max 210), band 2 = a DIAGONAL gradient reaching 255 with a triangular
  histogram distinct from band 0's — so \`--band 2\` pins band-index honouring),
  \`index.png\` (16×16 Gray8, four levels 0..3).
- **Committed histogram inputs**: \`hist.v\`/\`hist2.v\` (256×1 ushort histograms
  of gray/gray2), \`histcum.v\` (256×1 ushort cumulative), \`lut.v\` (256×1 uchar
  equalisation LUT = norm ∘ cum ∘ find). Fed to BOTH vips and \`viprs\`.
- **Carriers**: vips writes counts as \`uint\`; the core writes \`ushort\`
  (\`PixelFormat\` caps at 16 bits, saturating at 65535), so every count
  reference is CAST to ushort — lossless here (no saturation) — and both sides
  then agree bit-for-bit. Histogram-tagged outputs → \`.v\`; plain b-w
  (\`hist_equal\`, \`maplut\`) → PNG.

## Honest oracle classes (MEASURED)

- **EXACT** (tol 0): \`hist_find\` (+ \`--band 0\`, full 0..255 range),
  \`hist_find_indexed\`, \`hist_find_ndim\`, \`hist_cum\`, \`maplut\`, and the
  boolean \`hist_ismonotonic\`. (\`hist_find\` matches only where the data
  reaches 255: vips trims trailing-zero bins to width max+1, the core keeps the
  full 256 — a representational trim, not a count divergence.)
- **BOUNDED-TOL**: \`hist_norm\` (≤1 LSB — normalising a cumulative histogram
  rounds ±1 vs vips); \`hist_equal\` (≤1 LSB equalisation-LUT rounding, measured
  max-abs-diff 1); \`hist_entropy\` (float log2 scalar, relative eps 1e-6 —
  matched to 6 places on both a uniform and a non-uniform histogram).
- **GOLDEN-ONLY** (NO vips cross-oracle — the core genuinely diverges from vips;
  references minted by \`viprs\`, tests are regression pins): \`hist_match\`
  (vips emits a \`uint\` LUT with a wholesale-different mapping, measured diff
  254), \`hist_plot\` (core plots \`max+1\` rows, vips \`max\` — heights never
  match), \`hist_local\` (window/border algorithm matches vips only for a 3×3
  window; 5×5 diff 51, CLAHE \`--max-slope\` diff 60 — the coincidental 3×3
  match is deliberately NOT used as an oracle), \`percent\` (core = "smallest
  value whose cumulative reaches P%", vips = "threshold above which P% lie";
  measured core = vips−2 on a dense ramp).

## Exact commands

Inputs (paths relative to \`tests/fixtures/cli/\`):

\`\`\`
vips grey hg.v 16 16
vips linear hg.v histogram/gray.png 255 0 --uchar
vips linear hg.v hg2.png 200 10 --uchar
vips rot hg.v hg_v.v d90
vips add hg.v hg_v.v hg_diag.v ; vips linear hg_diag.v hg3.png 127.5 0 --uchar  # band 2 diagonal, reaches 255
vips bandjoin "histogram/gray.png hg2.png hg3.png" hrgb.v
vips copy hrgb.v histogram/rgb.png --interpretation srgb
vips linear hg.v hidxf.v 3 0 ; vips cast hidxf.v histogram/index.png uchar
vips hist_find histogram/gray.png hf_u.v ; vips cast hf_u.v histogram/hist.v ushort
vips hist_cum histogram/hist.v hc_u.v ; vips cast hc_u.v histogram/histcum.v ushort
vips hist_find hg2.png hf2_u.v ; vips cast hf2_u.v histogram/hist2.v ushort
vips hist_norm histogram/histcum.v histogram/lut.v
vips grey pg.v 8 8 ; vips rot pg.v pgv.v d90
vips linear pg.v  pb0.v 255 0  --uchar   # base band0 (horizontal ramp)
vips linear pgv.v pb1.v 255 0  --uchar   # base band1 (vertical ramp)
vips linear pg.v  pb2.v 128 40 --uchar   # base band2
vips linear pgv.v pba.v 200 55 --uchar   # base alpha (varying)
vips bandjoin "pb0.v pb1.v pb2.v"        pbase_op.v ; vips copy pbase_op.v composite/base_op.png --interpretation srgb
vips bandjoin "pb0.v pb1.v pb2.v pba.v"  pbase.v    ; vips copy pbase.v    composite/base.png    --interpretation srgb
vips linear pgv.v po0.v 200 20 --uchar ; vips linear pg.v po1.v 150 30 --uchar
vips linear pgv.v po2.v 255 0  --uchar ; vips linear pg.v poa.v 180 40 --uchar
vips bandjoin "po0.v po1.v po2.v"        pover_op.v ; vips copy pover_op.v composite/overlay_op.png --interpretation srgb
vips bandjoin "po0.v po1.v po2.v poa.v"  pover.v    ; vips copy pover.v    composite/overlay.png    --interpretation srgb
vips grey fg.v 16 16
vips rot fg.v fgrot.v d90
vips linear fg.v fgx.v 85 0 ; vips linear fgrot.v fgy.v 170 0
vips add fgx.v fgy.v fgsum.v ; vips cast fgsum.v freqfilt/in.png uchar
vips mask_ideal freqfilt/mask.v 16 16 0.3
vips wrap freqfilt/in.png freqfilt/shifted.png --x 3 --y 2
vips grey fsm.v 8 8 ; vips linear fsm.v freqfilt/small.png 255 0 --uchar
\`\`\`

\`small.png\` (8×8) is the wrong-size input for the \`freqmult\`/\`phasecor\`
dimension-mismatch error cases (exit 1, no reference output; CLI_CONTRACT.md §8).

Complex→pair normalisation (applied to fwfft/invfft complex outputs):

\`\`\`
vips complexget <complex.v> re.v real
vips complexget <complex.v> im.v imag
vips bandjoin "re.v im.v" pair.v
vips cast pair.v <expected.v> float
vips gaussnoise mg1.v 40 32 --seed 1 --mean 140 --sigma 45 ; vips cast mg1.v mosaicing/merge_ref.png uchar
vips gaussnoise mg2.v 40 32 --seed 2 --mean 130 --sigma 45 ; vips cast mg2.v mosaicing/merge_sec.png uchar
# merge_rgb: bandjoin of three distinct-seed 40x32 grays, re-tagged sRGB
vips gaussnoise mbh.v 110 150 --seed 7  --mean 128 --sigma 60 ; vips cast mbh.v mbh.png uchar
vips extract_area mbh.png mosaicing/mosaic_h_ref.png 0  0 100 150
vips extract_area mbh.png mosaicing/mosaic_h_sec.png 10 0 100 150
vips gaussnoise mbv.v 150 110 --seed 11 --mean 128 --sigma 60 ; vips cast mbv.v mbv.png uchar
vips extract_area mbv.png mosaicing/mosaic_v_ref.png 0 0  150 100
vips extract_area mbv.png mosaicing/mosaic_v_sec.png 0 10 150 100
# globalbalance input (minted by viprs, carries the join-tree blob):
viprs merge gb1.png gb2.png mosaicing/balance_input.v horizontal -40 0
vips grey ag.v 16 16
vips linear ag.v aritha/agray.png 255 0 --uchar
vips linear ag.v aritha/afloat.v 510 " -255"
vips black blk.v 6 7 --bands 1 ; vips linear blk.v blk.png 0 0 --uchar
vips embed blk.png aritha/content.png 4 5 20 20 --extend white
vips black blkbg.v 20 20 --bands 1 ; vips linear blkbg.v blkbg.png 0 0 --uchar
vips black wblk.v 5 4 --bands 1 ; vips linear wblk.v wblk.png 0 255 --uchar
vips insert blkbg.png wblk.png aritha/content2.png 3 2
vips black pz.v 8 8 --bands 1 ; vips linear pz.v pz.png 0 0 --uchar
vips black pblk.v 3 3 --bands 1 ; vips linear pblk.v pblk.png 0 255 --uchar
vips insert pz.png pblk.png aritha/pzero.png 2 3
vips black hb.v 32 32 --bands 1 ; vips linear hb.v hblack.png 0 0 --uchar
vips black pt.v 1 1 --bands 1 ; vips linear pt.v pt.png 0 255 --uchar
vips insert hblack.png pt.png aritha/point.png 10 6
vips grey ag.v 16 16
vips linear ag.v arithb/a.png 255 0 --uchar
vips rot ag.v agv.v d90
vips linear agv.v arithb/b.png 128 40 --uchar
vips linear ag.v arithb/c.png 100 20 --uchar
vips bandjoin "arithb/a.png arithb/b.png arithb/c.png" rgb.v
vips copy rgb.v arithb/rgb.png --interpretation srgb
vips bandjoin "arithb/rgb.png arithb/a.png" rgba.v
vips copy rgba.v arithb/rgba.png --interpretation srgb
vips linear ag.v arithb/small.png 15 0 --uchar
vips linear ag.v arithb/small2.png 3 0 --uchar
vips eye eye.v 16 16
vips linear eye.v arithb/eye.png 127 128 --uchar
vips rot arithb/a.png arithb/avert.png d90
vips linear ag.v arithb/msmall.v 0.85 0.1
vips linear ag.v arithb/macosh.v 5 1
printf '3 3\\n0.5 0.25 0.25\\n0.2 0.6 0.2\\n0.1 0.3 0.6\\n' > arithb/recomb.mat
vips complexform arithb/a.png arithb/b.png cpx_c.v
vips copy cpx_c.v arithb/complex_in.v --format float --bands 2
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | vips command |
|---|---|---|
| \`convolution/gaussmat_int_expected.v\` | BOUNDED-TOL (tol 0) | \`vips gaussmat gm.v 2 0.2\` then \`vips cast gm.v … float\` |
| \`convolution/gaussmat_sep_expected.v\` | BOUNDED-TOL (tol 0) | \`vips gaussmat gms.v 2 0.2 --separable\` then cast float |
| \`convolution/gaussmat_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips gaussmat gmf.v 2 0.2 --precision float\` then cast float |
| \`convolution/logmat_int_expected.v\` | BOUNDED-TOL (tol 0) | \`vips logmat lm.v 2 0.1\` then cast float |
| \`convolution/logmat_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips logmat lmf.v 2 0.1 --separable --precision float\` then cast float |
| \`convolution/conv_blur_int_expected.png\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips conv eye.png conv_blur_int_expected.png blur.mat --precision integer\` (vector-path gap 1) |
| \`convolution/conv_sobel_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips conv eye.png conv_sobel_float_expected.v sobel.mat --precision float\` |
| \`convolution/convsep_int_expected.png\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips convsep eye.png convsep_int_expected.png sep.mat --precision integer\` (vector-path gap **4**) |
| \`convolution/convsep_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips convsep eye.png convsep_float_expected.v sep.mat --precision float\` |
| \`convolution/compass_max_int_expected.png\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips compass eye.png … sobel.mat --times 4 --angle d45 --combine max --precision integer\` (scale-1 mask: gap 0 either way) |
| \`convolution/compass_sum_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips compass eye.png … sobel.mat --times 4 --angle d45 --combine sum --precision float\` |
| \`convolution/compass_sum_int_expected.v\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips compass eye.png … sobel.mat --times 4 --angle d45 --combine sum --precision integer\` then \`vips cast … ushort\` (vips emits uint; core emits Gray16; 16-bit-promotion path) |
| \`convolution/gaussblur_int_expected.png\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips gaussblur eye.png gaussblur_int_expected.png 1.5 --precision integer\` (vector-path gap 1) |
| \`convolution/gaussblur_s1.6_int_expected.png\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips gaussblur eye.png … 1.6 --precision integer\` (vector-path gap **4** — the case a tol of 1 would fail) |
| \`convolution/gaussblur_s0.8_int_expected.png\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips gaussblur noise64.png … 0.8 --precision integer\` (vector-path gap 2; separable gaussmat scale 38) |
| \`convolution/gaussblur_s0.6_int_expected.png\` | EXACT (tol 0), regime pin | \`VIPS_NOVECTOR=1 vips gaussblur eye.png … 0.6 --precision integer\` (\`vips_convi_intize\` declines the 3×1 scale-30 mask as "too inaccurate", so libvips runs C itself: gap 0) |
| \`convolution/conv_boxsum1147_int_expected.png\` | EXACT (tol 0) | \`VIPS_NOVECTOR=1 vips conv boxsum1147.png … blur.mat --precision integer\` (127 here, 128 on the vector path, **floor also 127** — isolates the requantised coefficients from the rounding mode) |
| \`convolution/conv_hostile_int_expected.png\` | EXACT (tol 0), bound-breaking | \`VIPS_NOVECTOR=1 vips conv noise64.png … hostile.mat --precision integer\` (vector-path gap **57**, on a mask \`vips_convi_intize\` accepts) |
| \`convolution/conv_sobel_int_expected.png\` | EXACT (tol 0), regime pin | \`VIPS_NOVECTOR=1 vips conv eye.png … sobel.mat --precision integer\` (scale 1: the vector path RUNS and still agrees, because intize is the identity) |
| \`convolution/conv_ushort_int_expected.v\` | EXACT (tol 0), regime pin | \`VIPS_NOVECTOR=1 vips conv eye16.v … blur.mat --precision integer\` (ushort: the vector path is gated off at \`convi.c:1151\`, so gap 0 by construction) |
| \`convolution/gaussblur_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips gaussblur eye.png gaussblur_float_expected.v 1.5 --precision float\` |
| \`convolution/sharpen_expected.png\` | BOUNDED-TOL ≤1 LSB (**issue #581**, NOT #558) | \`vips sharpen eye.png sharpen_expected.png --sigma 1 --m1 1 --m2 2\` (plain binary on purpose: LabS is 16-bit, so \`VIPS_INFO=1\` reports "convi: using C path" and \`VIPS_NOVECTOR\` changes nothing) |
| \`convolution/spcor_expected.v\` | BOUNDED-TOL (eps 1e-5) | \`vips spcor eye.png patch.png spcor_expected.v\` |
| \`convolution/fastcor_expected.v\` | EXACT (tol 0) | \`vips fastcor eye.png patch.png fc_uint.v\` then \`vips cast fc_uint.v … float\` |
| \`matrix/matrixinvert3_expected.v\` | BOUNDED-TOL (f32, measured 0) | \`vips matrixinvert m3.mat mi3.v\` then \`vips cast mi3.v matrixinvert3_expected.v float\` (direct cofactor path) |
| \`matrix/matrixinvert4_expected.v\` | BOUNDED-TOL (f32, measured 0) | \`vips matrixinvert m4.mat mi4.v\` then \`vips cast mi4.v matrixinvert4_expected.v float\` (PLU path) |
| \`matrix/invertlut_expected.v\` | BOUNDED-TOL (f32, 1 ULP = 5.96e-8) | \`vips invertlut lut.mat il.v\` then \`vips cast il.v invertlut_expected.v float\` (default size 256) |
| \`matrix/invertlut_size64_expected.v\` | BOUNDED-TOL (f32, 1 ULP = 5.96e-8) | \`vips invertlut lut.mat il64.v --size 64\` then \`vips cast il64.v invertlut_size64_expected.v float\` |

Bounds rejection (singular / non-square matrixinvert, out-of-range /
too-few-columns invertlut, sub-range \`--size\`, and a \`--size\` in the
clap-accepts / core-rejects band 65537..=1000000) is a typed exit-1 (or usage
exit-2) error with a \`viprs\`-side message, never a panic (CLI_CONTRACT.md §8);
those cases build their tiny matrices in-test (or reuse \`lut.mat\`) and need no
committed reference. The size band is a PARITY quirk: clap mirrors vips's
declared \`1..=1000000\` metadata while the core caps at \`1..=65536\`, and vips
itself independently rejects a size above 65536 despite its declared max — so a
\`--size 100000\` is a clean exit-1 \`BadSize\` on both sides, not a panic.

The \`matrixinvert\` cases compare at tol \`0.0\` (EXACT-AFTER-CAST — the f32-cast
core result is bit-identical to the vips-double-cast-to-float reference); only the
\`invertlut\` cases use the nonzero \`1e-6\` f32 tol.
| \`colour/colourspace_lab_expected.v\` | BOUNDED-TOL (1e-4) | \`vips colourspace rgb.png colourspace_lab_expected.v lab\` |
| \`colour/colourspace_xyz_expected.v\` | BOUNDED-TOL (1e-4) | \`vips colourspace rgb.png colourspace_xyz_expected.v xyz\` |
| \`colour/colourspace_scrgb_expected.v\` | BOUNDED-TOL (1e-4) | \`vips colourspace rgb.png colourspace_scrgb_expected.v scrgb\` |
| \`colour/colourspace_lab_png_expected.png\` | BOUNDED-TOL (≤1 LSB) | \`vips colourspace rgb.png colourspace_lab_png_expected.png lab\` (#36 interp-aware save) |
| \`colour/colourspace_lab_input_png_expected.png\` | BOUNDED-TOL (≤1 LSB) | \`vips colourspace icc_pcs_lab.v colourspace_lab_input_png_expected.png srgb\` (#36 non-round-trip discriminator) |
| \`colour/colourspace_srcspace_expected.png\` | BOUNDED-TOL (≤1 LSB) | \`vips colourspace rgb.png colourspace_srcspace_expected.png srgb --source-space lab\` |
| \`colour/dE76_expected.v\` | BOUNDED-TOL (1e-4) | \`vips dE76 rgb.png rgb2.png dE76_expected.v\` |
| \`colour/dE00_expected.v\` | BOUNDED-TOL (1e-4) | \`vips dE00 rgb.png rgb2.png dE00_expected.v\` (vips_col_dE00 parity) |
| \`colour/dECMC_golden.v\` | GOLDEN-ONLY | \`viprs dECMC rgb.png rgb2.png dECMC_golden.v\` (NO vips oracle — vips computes a different formula) |
| \`colour/icc_import_lab_expected.v\` | BOUNDED-TOL (~0.31) | \`vips icc_import rgb.png icc_import_lab_expected.v --input-profile sRGB.icc --intent relative\` |
| \`colour/icc_export_expected.png\` | BOUNDED-TOL (≤2 LSB) | \`vips icc_export icc_pcs_lab.v icc_export_expected.png --output-profile sRGB.icc --intent relative --depth 8\` |
| \`colour/icc_export_d16_expected.png\` | BOUNDED-TOL (≤16 LSB @ 16-bit) | \`vips icc_export icc_pcs_lab.v icc_export_d16_expected.png --output-profile sRGB.icc --intent relative --depth 16\` |
| \`colour/icc_transform_expected.png\` | BOUNDED-TOL (≤2 LSB) | \`vips icc_transform rgb.png icc_transform_expected.png sRGB.icc --input-profile sRGB.icc --intent relative\` |
| reference | oracle class | command |
|---|---|---|
| \`histogram/hist_find_expected.v\` | EXACT | \`vips hist_find gray.png rf.v\` → \`vips cast rf.v … ushort\` |
| \`histogram/hist_find_band_expected.v\` | EXACT | \`vips hist_find rgb.png rfb.v --band 0\` → cast ushort (band 0 = full 0..255 ramp; vips trims trailing-zero bins so a band with max < 255 would mismatch on width) |
| \`histogram/hist_find_band2_expected.v\` | EXACT | \`vips hist_find rgb.png rfb2.v --band 2\` → cast ushort (band 2 = diagonal gradient, triangular histogram distinct from band 0 — pins band-index honouring; reaches 255 so no trailing-zero trim) |
| \`histogram/hist_find_indexed_expected.v\` | EXACT | \`vips hist_find_indexed gray.png index.png ri.v\` → cast ushort |
| \`histogram/hist_find_ndim_expected.v\` | EXACT | \`vips hist_find_ndim rgb.png rn.v --bins 4\` → cast ushort |
| \`histogram/hist_cum_expected.v\` | EXACT | \`vips hist_cum hist.v rc.v\` → cast ushort |
| \`histogram/hist_norm_expected.v\` | BOUNDED-TOL ≤1 LSB | \`vips hist_norm histcum.v hist_norm_expected.v\` (cumulative-norm rounding core-vs-vips ±1) |
| \`histogram/hist_equal_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips hist_equal gray.png hist_equal_expected.png\` |
| \`histogram/maplut_expected.png\` | EXACT | \`vips maplut gray.png maplut_expected.png lut.v\` |
| \`histogram/hist_entropy_expected.txt\` | BOUNDED-TOL (rel 1e-6) | \`vips hist_entropy hist.v\` (uniform → 4.000000) |
| \`histogram/hist_entropy_cum_expected.txt\` | BOUNDED-TOL (rel 1e-6) | \`vips hist_entropy histcum.v\` (non-uniform → 7.763350) |
| \`histogram/hist_ismonotonic_false_expected.txt\` | EXACT (bool) | \`vips hist_ismonotonic hist.v\` (FALSE) |
| \`histogram/hist_ismonotonic_true_expected.txt\` | EXACT (bool) | \`vips hist_ismonotonic histcum.v\` (TRUE) |
| \`histogram/hist_match_golden.v\` | GOLDEN-ONLY | \`viprs hist_match hist.v hist2.v hist_match_golden.v\` (NO vips oracle) |
| \`histogram/hist_plot_golden.v\` | GOLDEN-ONLY | \`viprs hist_plot hist.v hist_plot_golden.v\` (NO vips oracle) |
| \`histogram/hist_local_golden.png\` | GOLDEN-ONLY | \`viprs hist_local gray.png hist_local_golden.png 5 5\` |
| \`histogram/hist_local_clahe_golden.png\` | GOLDEN-ONLY | \`viprs hist_local gray.png hist_local_clahe_golden.png 5 5 --max-slope 3\` |
| \`histogram/percent_golden.txt\` | GOLDEN-ONLY | \`viprs percent gray.png 50\` (NO vips oracle) |

\`hist_find\` / \`hist_find_indexed\` / \`hist_find_ndim\` / \`hist_cum\` /
\`hist_norm\` / \`hist_equal\` / \`hist_local\` / \`maplut\` / \`hist_entropy\` /
\`hist_ismonotonic\` / \`percent\` all REJECT a float / non-integer input with a
typed exit-1 error (the core's \`read_flat\` would otherwise abort on a float
raster; the CLI guards it before touching the core — CLI_CONTRACT.md §8), pinned
by \`hist_find_rejects_float_input_without_panicking\`.
| \`composite/composite2_over_expected.png\`      | BOUNDED-TOL ≤1 LSB | \`vips composite2 base.png overlay.png composite2_over_expected.png over\` |
| \`composite/composite2_source_expected.png\`    | BOUNDED-TOL ≤1 LSB | \`vips composite2 base.png overlay.png composite2_source_expected.png source\` |
| \`composite/composite2_in_expected.png\`        | BOUNDED-TOL ≤1 LSB | \`vips composite2 base.png overlay.png composite2_in_expected.png in\` |
| \`composite/composite2_xor_expected.png\`       | BOUNDED-TOL ≤1 LSB | \`vips composite2 base.png overlay.png composite2_xor_expected.png xor\` |
| \`composite/composite2_add_expected.png\`       | BOUNDED-TOL ≤1 LSB | \`vips composite2 base.png overlay.png composite2_add_expected.png add\` |
| \`composite/composite2_dest_over_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips composite2 base.png overlay.png composite2_dest_over_expected.png dest-over\` |
| \`composite/composite_over_expected.png\`       | BOUNDED-TOL ≤1 LSB | \`vips composite "base.png overlay.png" composite_over_expected.png 2\` (array form; over = int 2) |
| \`composite/composite2_multiply_expected.png\`    | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_multiply_expected.png multiply\` |
| \`composite/composite2_screen_expected.png\`      | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_screen_expected.png screen\` |
| \`composite/composite2_overlay_expected.png\`     | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_overlay_expected.png overlay\` |
| \`composite/composite2_darken_expected.png\`      | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_darken_expected.png darken\` |
| \`composite/composite2_hardlight_expected.png\`   | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_hardlight_expected.png hard-light\` |
| \`composite/composite2_difference_expected.png\`  | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_difference_expected.png difference\` |
| \`composite/composite2_exclusion_expected.png\`   | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_exclusion_expected.png exclusion\` |
| \`composite/composite2_colourdodge_expected.png\` | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_colourdodge_expected.png colour-dodge\` |
| \`composite/composite2_clear_expected.png\`     | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_clear_expected.png clear\` |
| \`composite/composite2_out_expected.png\`       | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_out_expected.png out\` |
| \`composite/composite2_dest_expected.png\`      | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_dest_expected.png dest\` |
| \`composite/composite2_dest_in_expected.png\`   | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_dest_in_expected.png dest-in\` |
| \`composite/composite2_dest_out_expected.png\`  | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_dest_out_expected.png dest-out\` |
| \`composite/composite2_dest_atop_expected.png\` | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_dest_atop_expected.png dest-atop\` |
| \`composite/composite2_lighten_expected.png\`   | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_lighten_expected.png lighten\` |
| \`composite/composite2_colourburn_expected.png\`| BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_colourburn_expected.png colour-burn\` |
| \`composite/composite2_softlight_expected.png\` | BOUNDED-TOL ≤1 LSB (OPAQUE) | \`vips composite2 base_op.png overlay_op.png composite2_softlight_expected.png soft-light\` |
| \`composite/composite2_multiply_translucent_golden.png\` | GOLDEN-ONLY | \`viprs composite2 base.png overlay.png composite2_multiply_translucent_golden.png multiply\` (NO vips oracle — translucent divergence ~38; a core translucent-blend fix requires REGENERATING this pin, not reverting) |
| \`composite/composite2_atop_translucent_golden.png\`     | GOLDEN-ONLY | \`viprs composite2 base.png overlay.png composite2_atop_translucent_golden.png atop\` (NO vips oracle — translucent divergence ~191; a core translucent-blend fix requires REGENERATING this pin, not reverting) |
| \`composite/composite2_saturate_translucent_golden.png\` | GOLDEN-ONLY | \`viprs composite2 base.png overlay.png composite2_saturate_translucent_golden.png saturate\` (NO vips oracle — translucent divergence ~37; a core translucent-blend fix requires REGENERATING this pin, not reverting) |

Error cases (asserted in \`cli_composite_diff.rs\`, no vips reference): a size /
band-count mismatch is a typed exit-1 error; an unknown or core-only
non-separable (\`hue\`/\`saturation\`/\`colour\`/\`luminosity\`) mode and a third input
are clap usage exit-2 errors (CLI_CONTRACT.md §8).
| \`freqfilt/fwfft_expected.v\` | FOURIER (2-band f32, eps 1e-2) | \`vips fwfft in.png fwfft_dpc.v\` → complexget/bandjoin/cast float |
| \`freqfilt/invfft_expected.v\` | FOURIER (2-band f32, eps 5e-2) | \`vips invfft in.png invfft_dpc.v\` → complexget/bandjoin/cast float |
| \`freqfilt/invfft_real_expected.v\` | FOURIER (1-band f32, eps 5e-2) | \`vips invfft in.png invfft_real_dbl.v --real\` → cast float |
| \`freqfilt/roundtrip_expected.v\` | FOURIER (1-band f32, eps 1e-2; complex-in path) | \`vips fwfft in.png rt_dpc.v\` → \`vips invfft rt_dpc.v rt_dbl.v --real\` → cast float |
| \`freqfilt/freqmult_expected.png\` | FOURIER (uchar, BOUNDED-TOL ≤1 LSB) | \`vips freqmult in.png mask.v freqmult_expected.png\` |
| \`freqfilt/spectrum_expected.png\` | FOURIER (uchar, tol 0) | \`vips spectrum in.png spectrum_expected.png\` |
| \`freqfilt/phasecor_expected.v\` | FOURIER (1-band f32, eps 1e-2) | \`vips phasecor in.png shifted.png phasecor_dbl.v\` → cast float |
| reference | oracle class | command |
|---|---|---|
| \`mosaicing/merge_h_expected.png\` | EXACT | \`vips merge merge_ref.png merge_sec.png merge_h_expected.png horizontal -- -28 0\` |
| \`mosaicing/merge_v_expected.png\` | EXACT | \`vips merge merge_ref.png merge_sec.png merge_v_expected.png vertical -- 0 -22\` |
| \`mosaicing/merge_rgb_expected.png\` | EXACT | \`vips merge merge_rgb.png merge_rgb_sec.png merge_rgb_expected.png horizontal -- -28 0\` (3-band multi-band path) |
| \`mosaicing/merge_fallback_expected.png\` | EXACT | \`vips merge merge_ref.png merge_sec.png merge_fallback_expected.png horizontal 12 0\` (positive dx → insert-fallback, no blend) |
| \`mosaicing/mosaic_h_expected.png\` | EXACT | \`vips mosaic mosaic_h_ref.png mosaic_h_sec.png mosaic_h_expected.png horizontal 50 75 40 75\` |
| \`mosaicing/mosaic_v_expected.png\` | EXACT | \`vips mosaic mosaic_v_ref.png mosaic_v_sec.png mosaic_v_expected.png vertical 75 50 75 40\` |
| \`mosaicing/balance_expected.v\` | GOLDEN-ONLY | \`viprs globalbalance balance_input.v balance_expected.v\` (NO vips oracle) |

\`merge\` rejects a format mismatch (a Gray8 REF + an sRGB SEC) with a typed
exit-1 error, and \`globalbalance\` rejects an input without the join-tree blob
(a plain PNG) the same way; both are asserted in \`cli_mosaicing_diff.rs\`
(nonzero exit + a viprs-side message substring; CLI_CONTRACT.md §8) and need no
reference output.

**\`merge --mblend\` — a deliberate CLI-surface divergence from vips (finding 2):**
vips \`merge\` honours \`--mblend N\` and produces a valid, different blend for any
N; \`viprs merge\` exposes the flag for parity but the core public \`try_merge\`
fixes the blend width at the vips default 10 and offers no API to vary it, so
\`viprs\` **exits 1 on any non-default \`--mblend\`** where vips would succeed. This
is intentional (loud-fail beats a silently-wrong "success"; the \`add\` 16-bit
lesson) but IS a real divergence from the oracle — recorded here so a parity
auditor is not surprised, and left uncovered by the differential precisely
because the core cannot reproduce what vips does at a non-default mblend.
# create family CLI-differential reference provenance

These fixtures are the committed references the create CLI-differential suite
(\`tests/cli_create_diff.rs\`) decode-compares \`viprs\` output against. The EXACT
and BOUNDED-TOL references are the **vips 8.18.4 oracle**; the GOLDEN-ONLY
references are **\`viprs\`-generated regression pins** (no vips oracle exists —
PRNG / Pango differ even seeded). Generated offline by \`tools/gen_cli_expected.sh\`,
NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common input** (under \`create/\`): \`buildlut_points.mat\` — a 2-point control
  matrix (\`0->0\`, \`255->100\`) in the vips text-matrix format, read by BOTH
  \`vips buildlut\` (matrix image) and \`viprs buildlut\` (MatFile loader); and
  \`buildlut_points3.mat\` — a >=3-point NON-COLLINEAR matrix (\`0->0\`, \`128->200\`,
  \`255->255\`) that pins multi-segment interpolation + control-point sort (a
  2-point single-segment matrix cannot; create finding 2).
- **Carriers**: float creators → \`.v\`; \`--uchar\` variants → PNG; \`tonelut\`
  (Gray16) → \`.v\`. vips \`xyz\` (uint) and \`buildlut\` (double) are cast to
  \`float\` before the reference \`.v\` is written, because the libviprs \`.v\`
  decoder reads only uchar/ushort/float band formats — the cast is lossless (the
  values are integer coordinates / f32-exact LUT entries).

## Oracle classes and measured tolerances

- **EXACT (tol 0)**: \`black\`, \`xyz\`.
- **BOUNDED-TOL (f32)**: \`eye\`, \`zone\`, \`sines\`, \`sdf\` — measured max-abs-diff
  <= 4e-6 (f32 trig / hypot rounding); every \`mask_*\` op, \`buildlut\` and
  \`tonelut\` measured **exactly 0**, but are classified BOUNDED-TOL (float
  trig/exp/pow) and compared with a small f32 epsilon as an honest upper bound.
- **GOLDEN-ONLY (tol 0 vs the viprs pin)**: \`gaussnoise\`, \`perlin\`, \`worley\`,
  \`fractsurf\`, \`text\`.

## Exact commands

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | command |
|---|---|---|
| \`create/black_expected.v\` | EXACT | \`vips black black_expected.v 8 8\` |
| \`create/black_bands3_expected.v\` | EXACT | \`vips black black_bands3_expected.v 8 8 --bands 3\` |
| \`create/xyz_expected.v\` | EXACT | \`vips xyz xyz_u.v 16 16\` then \`vips cast xyz_u.v xyz_expected.v float\` |
| \`create/eye_expected.v\` | BOUNDED-TOL | \`vips eye eye_expected.v 32 32\` |
| \`create/eye_uchar_expected.png\` | BOUNDED-TOL (<=1 LSB) | \`vips eye eye_uchar_expected.png 32 32 --uchar\` |
| \`create/zone_expected.v\` | BOUNDED-TOL | \`vips zone zone_expected.v 32 32\` |
| \`create/sines_expected.v\` | BOUNDED-TOL | \`vips sines sines_expected.v 32 32\` |
| \`create/buildlut_expected.v\` | BOUNDED-TOL | \`vips buildlut buildlut_points.mat buildlut_d.v\` then \`vips cast buildlut_d.v buildlut_expected.v float\` |
| \`create/buildlut3_expected.v\` | BOUNDED-TOL | \`vips buildlut buildlut_points3.mat buildlut3_d.v\` then \`vips cast buildlut3_d.v buildlut3_expected.v float\` (>=3-point multi-segment) |
| \`create/tonelut_expected.v\` | BOUNDED-TOL (measured 0) | \`vips tonelut tonelut_expected.v\` |
| \`create/mask_ideal_expected.v\` | BOUNDED-TOL | \`vips mask_ideal mask_ideal_expected.v 64 64 0.5\` |
| \`create/mask_ideal_nodc_expected.v\` | BOUNDED-TOL | \`vips mask_ideal mask_ideal_nodc_expected.v 64 64 0.5 --nodc\` |
| \`create/mask_ideal_ring_expected.v\` | BOUNDED-TOL | \`vips mask_ideal_ring mask_ideal_ring_expected.v 64 64 0.5 0.2\` |
| \`create/mask_ideal_band_expected.v\` | BOUNDED-TOL | \`vips mask_ideal_band mask_ideal_band_expected.v 64 64 0.3 0.3 0.1\` |
| \`create/mask_gaussian_expected.v\` | BOUNDED-TOL | \`vips mask_gaussian mask_gaussian_expected.v 64 64 0.4 0.6\` (distinct fc != ac) |
| \`create/mask_gaussian_nodc_expected.v\` | BOUNDED-TOL | \`vips mask_gaussian mask_gaussian_nodc_expected.v 64 64 0.4 0.6 --nodc\` (isolated --nodc) |
| \`create/mask_gaussian_ring_expected.v\` | BOUNDED-TOL | \`vips mask_gaussian_ring mask_gaussian_ring_expected.v 64 64 0.4 0.6 0.2\` |
| \`create/mask_gaussian_band_expected.v\` | BOUNDED-TOL | \`vips mask_gaussian_band mask_gaussian_band_expected.v 64 64 0.3 0.3 0.1 0.5\` |
| \`create/mask_butterworth_expected.v\` | BOUNDED-TOL | \`vips mask_butterworth mask_butterworth_expected.v 64 64 2 0.4 0.6\` (distinct fc != ac) |
| \`create/mask_butterworth_uchar_expected.png\` | BOUNDED-TOL (<=1 LSB) | \`vips mask_butterworth mask_butterworth_uchar_expected.png 64 64 2 0.5 0.5 --uchar --optical\` |
| \`create/mask_butterworth_optical_expected.v\` | BOUNDED-TOL | \`vips mask_butterworth mask_butterworth_optical_expected.v 64 64 2 0.4 0.6 --optical\` (isolated --optical on float .v) |
| \`create/mask_butterworth_ring_expected.v\` | BOUNDED-TOL | \`vips mask_butterworth_ring mask_butterworth_ring_expected.v 64 64 2 0.4 0.6 0.2\` |
| \`create/mask_butterworth_band_expected.v\` | BOUNDED-TOL | \`vips mask_butterworth_band mask_butterworth_band_expected.v 64 64 2 0.3 0.3 0.1 0.5\` |
| \`create/mask_fractal_expected.v\` | BOUNDED-TOL | \`vips mask_fractal mask_fractal_expected.v 64 64 2.5\` |
| \`create/sdf_circle_expected.v\` | BOUNDED-TOL | \`vips sdf sdf_circle_expected.v 64 64 circle --a "32 32" --r 16\` |
| \`create/sdf_box_expected.v\` | BOUNDED-TOL | \`vips sdf sdf_box_expected.v 64 64 box --a "10 10" --b "50 40"\` |
| \`create/sdf_line_expected.v\` | BOUNDED-TOL | \`vips sdf sdf_line_expected.v 64 64 line --a "10 10" --b "50 40"\` |
| \`create/sdf_rounded_expected.v\` | BOUNDED-TOL | \`vips sdf sdf_rounded_expected.v 64 64 rounded-box --a "10 10" --b "50 40" --corners "20 0 0 0"\` |
| \`create/gaussnoise_golden.v\` | GOLDEN-ONLY | \`viprs gaussnoise gaussnoise_golden.v 16 16 --seed 42 --sigma 10 --mean 128\` (no vips oracle) |
| \`create/perlin_golden.v\` | GOLDEN-ONLY | \`viprs perlin perlin_golden.v 64 64 --seed 7\` (no vips oracle) |
| \`create/worley_golden.v\` | GOLDEN-ONLY | \`viprs worley worley_golden.v 64 64 --seed 7\` (no vips oracle) |
| \`create/fractsurf_golden.v\` | GOLDEN-ONLY | \`viprs fractsurf fractsurf_golden.v 64 48 2.5\` (no vips oracle) |
| \`create/text_golden.png\` | GOLDEN-ONLY | \`viprs text text_golden.png "Hi" --dpi 72\` (no vips oracle — Pango differs) |
# draw family CLI-differential reference provenance (GOLDEN-ONLY)

Every \`draw_*\` op is **GOLDEN-ONLY**: \`vips draw_*\` are in-place mutators whose
CLI **discards** the mutated image, so there is **NO vips CLI oracle**. Each
reference below is generated **once by \`viprs\` itself** (deterministic) and
committed; the differential cell (\`tests/cli_draw_diff.rs\`) is a **regression
pin** that states there is no vips cross-oracle — NOT a parity claim.

- **Generator**: \`viprs\` (\`$VIPRS\`), built \`--release --no-default-features\`.
- **Oracle**: none (GOLDEN-ONLY). The vips version above is recorded only because
  the COMMON INPUTS are built with vips as a deterministic coordinate function
  (\`grey\`/\`eye\`/\`black\`); vips is never used to produce a draw reference.
- **Common inputs** (under \`draw/\`): \`rgb.png\` (32×32 sRGB 2-D gradient),
  \`flood.png\` (32×32 Gray8 three flat stripes 0/100/200), \`mask.png\` (16×16
  Gray8 ramp opacity stencil), \`sub.png\` (8×8 solid magenta sRGB paste source),
  \`smudge.png\` (16×16 Gray8 high-frequency thresholded \`eye\`), \`gray16.v\`
  (16×16 single-band Gray16 ramp, native \`.v\` — the 16-bit ink/draw target).

## Exact commands

Inputs (built with vips, deterministic):

\`\`\`
vips grey dg.v 32 32
vips linear dg.v dgx.png 255 0 --uchar
vips rot dg.v dgv.v d90
vips linear dgv.v dgy.png 200 20 --uchar
vips linear dg.v dgz.png 120 60 --uchar
vips bandjoin "dgx.png dgy.png dgz.png" drgb.v
vips copy drgb.v draw/rgb.png --interpretation srgb
vips linear dg.v dramp.v 255 0 --uchar
vips relational_const dramp.v dr1.v moreeq 85
vips relational_const dramp.v dr2.v moreeq 170
vips add dr1.v dr2.v dr12.v
vips linear dr12.v draw/flood.png 0.39215686274509803 0 --uchar   # -> 0/100/200
vips grey dm.v 16 16
vips linear dm.v draw/mask.png 255 0 --uchar
vips black db.v 8 8 --bands 3
vips linear db.v dsub.v "0 0 0" "255 0 255" --uchar
vips copy dsub.v draw/sub.png --interpretation srgb
vips eye de.v 16 16
vips relational_const de.v draw/smudge.png more 0.0
vips grey dg16.v 16 16
vips linear dg16.v dg16s.v 60000 0
vips cast dg16s.v draw/gray16.v ushort
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`, ALL GOLDEN-ONLY — minted by \`viprs\`):

| reference | oracle class | viprs command |
|---|---|---|
| \`draw/draw_circle_golden.png\` | GOLDEN-ONLY | \`viprs draw_circle rgb.png draw_circle_golden.png 16 16 8 --ink "255 0 0"\` |
| \`draw/draw_circle_fill_golden.png\` | GOLDEN-ONLY | \`viprs draw_circle rgb.png draw_circle_fill_golden.png 16 16 8 --ink "255 0 0" --fill\` |
| \`draw/draw_rect_golden.png\` | GOLDEN-ONLY | \`viprs draw_rect rgb.png draw_rect_golden.png 4 4 20 16 --ink "0 255 0"\` |
| \`draw/draw_rect_fill_golden.png\` | GOLDEN-ONLY | \`viprs draw_rect rgb.png draw_rect_fill_golden.png 4 4 20 16 --ink "0 255 0" --fill\` |
| \`draw/draw_line_golden.png\` | GOLDEN-ONLY | \`viprs draw_line rgb.png draw_line_golden.png 0 0 31 31 --ink "0 0 255"\` |
| \`draw/draw_flood_golden.png\` | GOLDEN-ONLY | \`viprs draw_flood flood.png draw_flood_golden.png 0 0 --ink "100"\` (bounded: 0-stripe → 100, stops at the 100 wall, 200-stripe untouched) |
| \`draw/draw_flood_blob_golden.png\` | GOLDEN-ONLY | \`viprs draw_flood flood.png draw_flood_blob_golden.png 0 0 --ink "50" --equal\` (blob: recolours only the seed's equal-valued 0-stripe → 50) |
| \`draw/draw_mask_golden.png\` | GOLDEN-ONLY | \`viprs draw_mask rgb.png draw_mask_golden.png mask.png 8 8 --ink "255 255 0"\` |
| \`draw/draw_smudge_golden.png\` | GOLDEN-ONLY | \`viprs draw_smudge smudge.png draw_smudge_golden.png 4 4 8 8\` |
| \`draw/draw_image_golden.png\` | GOLDEN-ONLY | \`viprs draw_image rgb.png draw_image_golden.png sub.png 10 10\` |
| \`draw/draw_rect_16bit_golden.v\` | GOLDEN-ONLY | \`viprs draw_rect gray16.v draw_rect_16bit_golden.v 4 4 8 8 --ink "40000"\` (16-bit ink encode/draw pin; \`.v\` carrier) |

The draw ops mutate their input in place; \`viprs\` materialises the result to
\`OUT\` (the S6 shape). \`draw_smudge\` and \`draw_image\` take no ink; \`draw_image\`
requires \`SUB\` to share \`IN\`'s pixel format (a documented core no-op otherwise,
surfaced as a typed exit-1 error); \`draw_mask\` requires a single-band 8-bit
\`MASK\` (the core paints nothing for any other mask format — a 3-band or 16-bit
mask is a silent no-op in the core, so the CLI rejects it with a typed exit-1
error rather than writing an unchanged image); and \`draw_smudge\`/\`--ink\` reject
a float target with a typed exit-1 error (never a panic). Most committed inputs
are 8-bit; \`gray16.v\` is a single-band Gray16 target that pins the 16-bit
native-endian ink encode + draw/save round trip (\`CLI_CONTRACT.md\` §9: 16-bit
ink byte order is native-endian, now regression-pinned by the committed golden).
# resample family CLI-differential reference provenance

These fixtures are the committed vips oracle references the resample
CLI-differential suite (\`tests/cli_resample_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`resample/\`): \`grad.png\` (32×32 Gray8 2-D gradient
  \`x*85 + y*170\`, so shrinkv / reducev / rot are non-vacuous), \`rgb.png\` (32×32
  3-band sRGB with 2-D structure), \`index.v\` (32×32 FLOAT 2-band coordinate map
  sampling each output at HALF its source coordinate — a real 2× zoom with
  fractional taps, so \`mapim\` moves data and interpolates).
- **EVERY op is BOUNDED-TOL for NON-ALPHA inputs** (the premultiply / rounding
  campaign #406-418): the core computes reduce / interpolate masks in f64 per
  output position while vips quantises the sub-pixel offset into fixed-point
  tables, so the two agree to ≤1 LSB. Non-alpha inputs are used deliberately:
  the core \`reduce\`/\`shrink\`/\`resize\` premultiply alpha before resampling (a
  documented, intentional divergence from bare \`vips_reduce\`/\`vips_shrink\`,
  which do not), so on an RGBA / GrayA input these ops diverge from vips
  WHOLESALE — well beyond ≤1 LSB (measured max-abs-diff 4 for \`shrink 2 2\` on a
  4-band sRGB ramp). That divergence is deliberate and OUT of the ≤1 LSB oracle
  class; the differential exercises only the no-alpha \`grad\`/\`rgb\` carriers.

## Measured max-abs-diff (per case)

References (paths relative to \`tests/fixtures/cli/\`):

| reference | tol (measured) | vips command |
|---|---|---|
| \`resample/shrink_expected.png\` | ≤1 LSB (0) | \`vips shrink grad.png shrink_expected.png 2 2\` |
| \`resample/shrinkh_expected.png\` | ≤1 LSB (0) | \`vips shrinkh grad.png shrinkh_expected.png 2\` |
| \`resample/shrinkv_expected.png\` | ≤1 LSB (0) | \`vips shrinkv grad.png shrinkv_expected.png 2\` |
| \`resample/reduce_lanczos3_expected.png\` | ≤1 LSB (0) | \`vips reduce rgb.png reduce_lanczos3_expected.png 2 2\` (default lanczos3) |
| \`resample/reduce_cubic_expected.png\` | ≤1 LSB (0) | \`vips reduce rgb.png reduce_cubic_expected.png 2 2 --kernel cubic\` |
| \`resample/reduceh_expected.png\` | ≤1 LSB (1) | \`vips reduceh grad.png reduceh_expected.png 2\` |
| \`resample/reducev_expected.png\` | ≤1 LSB (1) | \`vips reducev grad.png reducev_expected.png 2\` |
| \`resample/resize_half_expected.png\` | ≤1 LSB (0) | \`vips resize rgb.png resize_half_expected.png 0.5\` |
| \`resample/resize_vscale_expected.png\` | ≤1 LSB (0) | \`vips resize rgb.png resize_vscale_expected.png 0.5 --vscale 0.75\` |
| \`resample/resize_up_expected.png\` | ≤1 LSB (1) | \`vips resize grad.png resize_up_expected.png 2.0\` (upscale → affine path) |
| \`resample/resize_nearest_expected.png\` | ≤1 LSB (0) | \`vips resize rgb.png resize_nearest_expected.png 0.5 --kernel nearest\` |
| \`resample/affine_bilinear_expected.png\` | ≤1 LSB (1) | \`vips affine rgb.png affine_bilinear_expected.png "1.5 0 0 1.5"\` (default bilinear) |
| \`resample/affine_bicubic_expected.png\` | **2 LSB (2)** | \`vips affine rgb.png affine_bicubic_expected.png "1.5 0 0 1.5" --interpolate bicubic\` (bicubic quantises to 2 LSB — noted divergence) |
| \`resample/similarity_angle_expected.png\` | ≤1 LSB (1) | \`vips similarity rgb.png similarity_angle_expected.png --angle 30\` |
| \`resample/similarity_scale_expected.png\` | ≤1 LSB (1) | \`vips similarity rgb.png similarity_scale_expected.png --scale 1.5\` |
| \`resample/rotate_expected.png\` | ≤1 LSB (1) | \`vips rotate rgb.png rotate_expected.png 30\` |
| \`resample/mapim_bilinear_expected.png\` | ≤1 LSB (0) | \`vips mapim rgb.png mapim_bilinear_expected.png index.v\` (S2; index is a 2nd input) |
| \`resample/mapim_bicubic_expected.png\` | ≤1 LSB (1) | \`vips mapim rgb.png mapim_bicubic_expected.png index.v --interpolate bicubic\` |
| \`resample/thumbnail_expected.png\` | ≤1 LSB (0) | \`vips thumbnail rgb.png thumbnail_expected.png 16\` (FILENAME input) |
| \`resample/thumbnail_crop_expected.png\` | ≤1 LSB (0) | \`vips thumbnail rgb.png thumbnail_crop_expected.png 16 --height 8 --crop centre\` (NON-square target — centre-crop removes pixels, 16×8, distinct from the no-crop fixtures) |
| \`resample/thumbnail_linear_expected.png\` | ≤1 LSB (1) | \`vips thumbnail rgb.png thumbnail_linear_expected.png 16 --linear\` (linear-light reduce path) |
| \`resample/thumbnail_image_expected.png\` | ≤1 LSB (0) | \`vips thumbnail_image rgb.png thumbnail_image_expected.png 16\` |

**Open question**: \`affine … --interpolate bicubic\` measures **2 LSB** (not the
≤1 LSB the rest of the family hits). The core evaluates exact f64 Catmull-Rom
coefficients while vips's \`VipsInterpolateBicubic\` uses a coarser fixed-point
coefficient table, so some interior samples land 2 apart. This is a genuine,
measured core-vs-vips rounding difference (not a CLI bug); the differential
compares that one case at tol 2. A follow-up could tighten the core bicubic
coefficient path toward vips's table if exact bicubic parity is ever required.
| \`aritha/avg_expected.txt\` | EXACT (S3, rational mean → rel-eps) | \`vips avg agray.png\` |
| \`aritha/deviate_expected.txt\` | BOUNDED-TOL (S3, rel-eps) | \`vips deviate agray.png\` |
| \`aritha/min_expected.txt\` | EXACT (S3, integer) | \`vips min agray.png\` |
| \`aritha/max_expected.txt\` | EXACT (S3, integer) | \`vips max agray.png\` |
| \`aritha/min_xy_expected.txt\` | EXACT (S3, x/y/value) | \`vips min agray.png --x --y\` |
| \`aritha/max_xy_expected.txt\` | EXACT (S3, x/y/value) | \`vips max agray.png --x --y\` |
| \`aritha/find_trim_expected.txt\` | EXACT (S3, 4 ints) | \`vips find_trim content.png\` |
| \`aritha/find_trim_bg_expected.txt\` | EXACT (S3, 4 ints) | \`vips find_trim content2.png --background 0\` |
| \`aritha/stats_expected.v\` | BOUNDED-TOL (matrix, meas. 0) | \`vips stats agray.png st.v\` → \`extract_area … 0 0 6 2\` → \`cast … float\` |
| \`aritha/measure_expected.v\` | BOUNDED-TOL (matrix, meas. 0) | \`vips measure agray.png ms.v 2 2\` → \`cast … float\` |
| \`aritha/profile_cols_expected.v\` / \`_rows_\` | EXACT | \`vips profile pzero.png pcol.v prow.v\` → \`cast … ushort\` |
| \`aritha/project_cols_expected.v\` / \`_rows_\` | EXACT | \`vips project agray.png qcol.v qrow.v\` → \`cast … ushort\` |
| \`aritha/linear_expected.v\` | EXACT-AFTER-CAST (float, meas. 0) | \`vips linear agray.png linear_expected.v 2 10\` |
| \`aritha/linear_uchar_expected.png\` | EXACT | \`vips linear agray.png linear_uchar_expected.png 2 10 --uchar\` |
| \`aritha/remainder_const_expected.png\` | EXACT | \`vips remainder_const agray.png remainder_const_expected.png 100\` |
| \`aritha/math2_const_pow_expected.v\` | EXACT-AFTER-CAST (ushort, meas. 0) | \`vips math2_const agray.png pow.v pow 2\` → \`cast … ushort\` (core rounds pow into ushort) |
| \`aritha/abs_expected.v\` | EXACT (float) | \`vips abs afloat.v abs_expected.v\` |
| \`aritha/sign_expected.v\` | EXACT-AFTER-CAST (float) | \`vips sign afloat.v sign.v\` → \`cast … float\` (vips emits signed char; float preserves −1/0/1; afloat now includes exactly 0.0 so the zero→0 branch is exercised) |
| \`aritha/round_ceil_expected.v\` / \`_floor_\` | EXACT (float) | \`vips round afloat.v … ceil\|floor\` (no tie-break; matches vips exactly) |
| \`aritha/round_rint_expected.v\` | EXACT (float) | \`vips round afloat.v round_rint_expected.v rint\` — core #494 rounds half TO EVEN, matching vips's C \`rint\` at exact half-integers (tol 0); the former GOLDEN-ONLY \`round_rint_golden.v\` pin is retired |
| \`aritha/clamp_expected.png\` | EXACT | \`vips clamp agray.png clamp_expected.png --min 50 --max 200\` |
| \`aritha/hough_line_golden.v\` | GOLDEN-ONLY (no full-width vips oracle) | \`viprs hough_line point.png hough_line_golden.v\` (binning now vips-exact per core #495, but Gray16-vs-uint saturation remains) |
| \`aritha/hough_circle_golden.v\` | GOLDEN-ONLY (no vips oracle) | \`viprs hough_circle point.png hough_circle_golden.v 2 4\` (core vote model diverges from vips) |

The two hough references are viprs-generated regression pins, not full-width vips
oracles: core #495 made \`hough_line\`'s binning match vips bit-for-bit, but the
core's Gray16 (u16) accumulator saturates where vips's uint one does not, and
\`hough_circle\`'s per-cell vote model still differs structurally (a core per-cell
max of 1 vs a vips max of 4 for a single point).
| \`arithb/subtract_expected.png\` | EXACT-AFTER-CAST (tol 0) | \`vips subtract a.png b.png subtract_expected.png\` |
| \`arithb/subtract_pos_expected.png\` | EXACT-AFTER-CAST (tol 0) | \`vips subtract a.png small.png subtract_pos_expected.png\` (a>=small everywhere → no clip dead-zone, full range) |
| \`arithb/multiply_expected.v\` | EXACT-AFTER-CAST (tol 0) | \`vips multiply a.png b.png multiply_expected.v\` (ushort) |
| \`arithb/divide_expected.v\` | EXACT-AFTER-CAST (tol 0) | \`vips divide a.png b.png divide_expected.v\` (float) |
| \`arithb/minpair_expected.png\` | EXACT | \`vips minpair a.png b.png minpair_expected.png\` |
| \`arithb/maxpair_expected.png\` | EXACT | \`vips maxpair a.png b.png maxpair_expected.png\` |
| \`arithb/sum_expected.v\` | EXACT-AFTER-CAST (tol 0) | \`vips sum "a.png b.png c.png" sum_u32.v\` then \`vips cast sum_u32.v sum_expected.v ushort\` (>=3 inputs; vips UINT→ushort) |
| \`arithb/relational_more_expected.png\` | EXACT | \`vips relational a.png b.png relational_more_expected.png more\` |
| \`arithb/relational_less_expected.png\` | EXACT | \`vips relational a.png b.png relational_less_expected.png less\` |
| \`arithb/relational_equal_expected.png\` | EXACT | \`vips relational a.png avert.png relational_equal_expected.png equal\` (x==y diagonal) |
| \`arithb/relational_noteq_expected.png\` | EXACT | \`vips relational a.png avert.png relational_noteq_expected.png noteq\` |
| \`arithb/relational_lesseq_expected.png\` | EXACT | \`vips relational a.png avert.png relational_lesseq_expected.png lesseq\` (differs from less on the diagonal) |
| \`arithb/relational_moreeq_expected.png\` | EXACT | \`vips relational a.png avert.png relational_moreeq_expected.png moreeq\` (differs from more on the diagonal) |
| \`arithb/relational_const_more_expected.png\` | EXACT | \`vips relational_const a.png relational_const_more_expected.png more 128\` |
| \`arithb/relational_const_equal_expected.png\` | EXACT | \`vips relational_const small.png relational_const_equal_expected.png equal 7\` |
| \`arithb/relational_const_noteq_expected.png\` | EXACT | \`vips relational_const small.png relational_const_noteq_expected.png noteq 7\` |
| \`arithb/relational_const_lesseq_expected.png\` | EXACT | \`vips relational_const small.png relational_const_lesseq_expected.png lesseq 7\` |
| \`arithb/relational_const_moreeq_expected.png\` | EXACT | \`vips relational_const small.png relational_const_moreeq_expected.png moreeq 7\` |
| \`arithb/boolean_eor_expected.png\` | EXACT | \`vips boolean a.png b.png boolean_eor_expected.png eor\` |
| \`arithb/boolean_and_expected.png\` | EXACT | \`vips boolean a.png b.png boolean_and_expected.png and\` |
| \`arithb/boolean_or_expected.png\` | EXACT | \`vips boolean a.png b.png boolean_or_expected.png or\` |
| \`arithb/boolean_const_and_expected.png\` | EXACT | \`vips boolean_const a.png boolean_const_and_expected.png and 200\` |
| \`arithb/boolean_const_or_expected.png\` | EXACT | \`vips boolean_const a.png boolean_const_or_expected.png or 200\` |
| \`arithb/boolean_const_eor_expected.png\` | EXACT | \`vips boolean_const a.png boolean_const_eor_expected.png eor 200\` |
| \`arithb/boolean_const_lshift_expected.png\` | EXACT | \`vips boolean_const a.png boolean_const_lshift_expected.png lshift 2\` |
| \`arithb/boolean_const_rshift_expected.png\` | EXACT | \`vips boolean_const a.png boolean_const_rshift_expected.png rshift 2\` |
| \`arithb/scale_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips scale rgb.png scale_expected.png\` (linear) |
| \`arithb/scale_log_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips scale rgb.png scale_log_expected.png --log\` |
| \`arithb/stdif_expected.png\` | EXACT (tol 0) | \`vips stdif eye.png stdif_expected.png 3 3\` (core #490 edge-replicate now matches vips whole-image, border ring included) |
| \`arithb/recomb_expected.png\` | EXACT (tol 0) | \`vips recomb rgb.png recomb_expected.png recomb.mat\` (core #491 f32-then-truncate now bit-exact) |
| \`arithb/premultiply_expected.png\` | BOUNDED-TOL 1 | \`vips premultiply rgba.png premultiply_expected.png\` |
| \`arithb/unpremultiply_expected.png\` | BOUNDED-TOL 1 | \`vips unpremultiply rgba.png unpremultiply_expected.png\` |
| \`arithb/math_sin_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math a.png math_sin_expected.v sin\` |
| \`arithb/math_cos_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math a.png math_cos_expected.v cos\` |
| \`arithb/math_atan_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math a.png math_atan_expected.v atan\` |
| \`arithb/math_tan_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_tan_expected.v tan\` |
| \`arithb/math_asin_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_asin_expected.v asin\` |
| \`arithb/math_acos_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_acos_expected.v acos\` |
| \`arithb/math_log_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_log_expected.v log\` |
| \`arithb/math_log10_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_log10_expected.v log10\` |
| \`arithb/math_exp_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_exp_expected.v exp\` |
| \`arithb/math_exp10_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_exp10_expected.v exp10\` |
| \`arithb/math_sinh_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_sinh_expected.v sinh\` |
| \`arithb/math_cosh_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_cosh_expected.v cosh\` |
| \`arithb/math_tanh_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_tanh_expected.v tanh\` |
| \`arithb/math_asinh_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_asinh_expected.v asinh\` |
| \`arithb/math_atanh_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math msmall.v math_atanh_expected.v atanh\` |
| \`arithb/math_acosh_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math macosh.v math_acosh_expected.v acosh\` |
| \`arithb/math2_atan2_expected.v\` | FOURIER (float, eps 1e-6) | \`vips math2 a.png b.png math2_atan2_expected.v atan2\` |
| \`arithb/math2_pow_expected.v\` | EXACT (tol 0) | \`vips math2 small.png small2.png math2_pow_expected.v pow\` (core #489 made pow(0,≤0)=0, matching vips on the 0^0 sample) |
| \`arithb/math2_wop_expected.v\` | EXACT (tol 0) | \`vips math2 small2.png small.png math2_wop_expected.v wop\` (wop = right^left = small^small2; shares the now-fixed 0^0 edge — core #489) |
| \`arithb/complexform_expected.v\` | FOURIER (eps 1e-6) | \`vips complexform a.png b.png cf_c.v\` then \`vips copy cf_c.v complexform_expected.v --format float --bands 2\` |
| \`arithb/complex_polar_expected.v\` | FOURIER (eps 1e-6) | \`vips complex cpx_c.v cpol.v polar\` then reinterpret \`--format float --bands 2\` |
| \`arithb/complex_rect_expected.v\` | FOURIER (eps 1e-6) | \`vips complex cpx_c.v crect.v rect\` then reinterpret |
| \`arithb/complex_conj_expected.v\` | FOURIER (eps 1e-6) | \`vips complex cpx_c.v cconj.v conj\` then reinterpret |
| \`arithb/complexget_real_expected.v\` | FOURIER (eps 1e-6) | \`vips complexget cpx_c.v complexget_real_expected.v real\` |
| \`arithb/complexget_imag_expected.v\` | FOURIER (eps 1e-6) | \`vips complexget cpx_c.v complexget_imag_expected.v imag\` |
EOF

# ===========================================================================
# io save-path cleanup (libviprs-cli #37 / #38) — cli_iocleanup_diff.rs
# ===========================================================================
IOCLEAN="$FIX_ROOT/iocleanup"
mkdir -p "$IOCLEAN"

# Strip vips's non-pixel `#vips2ppm - <timestamp>` header comment so the committed
# PNM reference is deterministic (no wall-clock churn) and the differential can do
# a whole-file BYTE compare. The pixel payload and the w/h/maxval tokens are vips's
# own bytes — only the timestamp comment line is removed. Author-run only (python3
# on the oracle machine); NEVER runs at test time.
ppm_strip_comment() {
    python3 - "$1" "$2" <<'PY'
import sys
raw = open(sys.argv[1], "rb").read()
assert raw[:2] in (b"P5", b"P6"), raw[:2]
def read_tokens(buf, start, n):
    toks, i = [], start
    while len(toks) < n:
        while i < len(buf) and buf[i:i+1].isspace():
            i += 1
        if buf[i:i+1] == b"#":          # comment: to end of line
            while i < len(buf) and buf[i:i+1] != b"\n":
                i += 1
            continue
        j = i
        while j < len(buf) and not buf[j:j+1].isspace():
            j += 1
        toks.append(buf[i:j]); i = j
    return toks, i
(w, h, mx), i = read_tokens(raw, 2, 3)
assert raw[i:i+1].isspace(), "one whitespace must follow maxval"
data = raw[i+1:]                        # exactly one whitespace, then raw samples
open(sys.argv[2], "wb").write(raw[:2] + b"\n" + w + b" " + h + b"\n" + mx + b"\n" + data)
PY
}

# --- Common inputs (read by BOTH this generator and `viprs`) -----------------
# 8×8 8-bit rgb (three DISTINCT grey ramps joined, re-tagged srgb) + its 1-band
# gray, then the same scaled to the FULL 0..65535 range (×257) and cast ushort so
# the 16-bit samples exceed 255 — a silent 8-bit downcast is then caught on value
# as well as on format-class. `vips grey` is a pure coordinate function, so every
# fixture is bit-reproducible.
echo "==> [iocleanup input] 8x8 rgb/gray (8-bit) + rgb16/gray16 (>255 samples)"
"$VIPS" grey "$TMP/iogrey.v" 8 8
"$VIPS" linear "$TMP/iogrey.v" "$IOCLEAN/gray.png"  255 0  --uchar
"$VIPS" linear "$TMP/iogrey.v" "$TMP/iog2.v"        200 10 --uchar
"$VIPS" rot "$TMP/iogrey.v" "$TMP/iogrey_v.v" d90
"$VIPS" linear "$TMP/iogrey_v.v" "$TMP/iog3.v" 255 0 --uchar
"$VIPS" bandjoin "$IOCLEAN/gray.png $TMP/iog2.v $TMP/iog3.v" "$TMP/iorgb.v"
"$VIPS" copy "$TMP/iorgb.v" "$IOCLEAN/rgb.png" --interpretation srgb
# 16-bit inputs (×257 maps 0..255 → 0..65535).
"$VIPS" linear "$IOCLEAN/rgb.png"  "$TMP/iorgb16f.v" 257 0
"$VIPS" cast   "$TMP/iorgb16f.v"   "$TMP/iorgb16u.v" ushort
"$VIPS" copy   "$TMP/iorgb16u.v"   "$IOCLEAN/rgb16.v" --interpretation rgb16
"$VIPS" linear "$IOCLEAN/gray.png" "$TMP/iog16f.v" 257 0
"$VIPS" cast   "$TMP/iog16f.v"     "$TMP/iog16u.v" ushort
"$VIPS" copy   "$TMP/iog16u.v"     "$IOCLEAN/gray16.v" --interpretation grey16

# --- #38 PNM references (byte-exact; comment stripped) -----------------------
echo "==> [iocleanup] PNM references (P6/P5 × 8/16-bit)"
# P6 8-bit: flip horizontal on rgb.
"$VIPS" flip "$IOCLEAN/rgb.png" "$TMP/io_flip.ppm" horizontal
ppm_strip_comment "$TMP/io_flip.ppm" "$IOCLEAN/flip_ppm_expected.ppm"
# P5 8-bit: flip horizontal on gray.
"$VIPS" flip "$IOCLEAN/gray.png" "$TMP/io_flip.pgm" horizontal
ppm_strip_comment "$TMP/io_flip.pgm" "$IOCLEAN/flip_pgm_expected.pgm"
# P6 16-bit: copy of rgb16 (big-endian, maxval 65535).
"$VIPS" copy "$IOCLEAN/rgb16.v" "$TMP/io_copy16.ppm"
ppm_strip_comment "$TMP/io_copy16.ppm" "$IOCLEAN/copy_ppm16_expected.ppm"
# P5 16-bit: copy of gray16.
"$VIPS" copy "$IOCLEAN/gray16.v" "$TMP/io_copy16.pgm"
ppm_strip_comment "$TMP/io_copy16.pgm" "$IOCLEAN/copy_pgm16_expected.pgm"

# --- #37 16-bit PNG save-path reference (decode-compare, EXACT) --------------
echo "==> [iocleanup] 16-bit PNG passthrough reference (copy rgb16 -> png)"
"$VIPS" copy "$IOCLEAN/rgb16.v" "$IOCLEAN/copy_png16_expected.png"

echo "==> [provenance] appending iocleanup section to $FIX_ROOT/PROVENANCE.md"
cat >> "$FIX_ROOT/PROVENANCE.md" <<EOF

# iocleanup — io save-path cleanup (libviprs-cli #37 / #38)

Consumed by \`libviprs-tests/tests/cli_iocleanup_diff.rs\`. Exercises the shared
save harness (\`libviprs-cli/src/ops/io.rs\`), not one op family.

## #38 — PNM (.ppm/.pgm) sink, BYTE-exact

PNM is uncompressed and canonical (fixed \`Pn\n<w> <h>\n<maxval>\n\` header then raw
**big-endian** samples), so a conformant encoder that agrees on the pixels emits
byte-identical files. These references are therefore whole-file BYTE-compared
(stronger than the decode-compare PNG/TIFF need). Each was produced by the vips
command below and then had its single non-pixel \`#vips2ppm - <timestamp>\` header
comment line removed (\`ppm_strip_comment\`, python3) so the committed fixture is
deterministic; the pixel payload and the w/h/maxval tokens are vips's own bytes.

| fixture | magic | oracle command (pre-strip) |
|---|---|---|
| \`iocleanup/flip_ppm_expected.ppm\`   | P6 8-bit  | \`vips flip rgb.png  out.ppm horizontal\` |
| \`iocleanup/flip_pgm_expected.pgm\`   | P5 8-bit  | \`vips flip gray.png out.pgm horizontal\` |
| \`iocleanup/copy_ppm16_expected.ppm\` | P6 16-bit | \`vips copy rgb16.v  out.ppm\` (maxval 65535, big-endian) |
| \`iocleanup/copy_pgm16_expected.pgm\` | P5 16-bit | \`vips copy gray16.v out.pgm\` (maxval 65535, big-endian) |

Note: vips's \`.ppm\` suffix ALWAYS writes P6 (a 1-band image is upsampled to
3-band srgb); its \`.pgm\` suffix writes P5. \`viprs\` instead chooses the magic by
band count (1 → P5, 3 → P6) regardless of suffix, so the gray P5 references use
\`.pgm\`. For 3-band inputs both sides agree on P6 and the byte compare holds.

## #37 — 16-bit depth on the integer PNG sink

vips's pngsave picks bit depth from the raster INTERPRETATION (grey16/rgb16 →
16-bit; b-w/multiband/srgb → 8-bit — even for a ushort raster), which
\`io::save\` mirrors. The honest limitation (verified on this oracle): libviprs'
EXACT-AFTER-CAST ops drop grey16→multiband on their float result, so a float EAC
op → .png saves 8-bit end-to-end (unlike vips) — 16-bit EAC is covered losslessly
by \`.v\` (\`core/add_gray_expected.v\`). What IS pinned as a .png here is the 16-bit
save PATH through a format-preserving op that keeps rgb16:

| fixture | class | oracle command |
|---|---|---|
| \`iocleanup/copy_png16_expected.png\` | EXACT (decode) | \`vips copy rgb16.v out.png\` (stays 16-bit) |
EOF

echo "==> Done. Generated fixtures under $FIX_ROOT"
ls -1 "$FIX"
echo "--- bands ---"
ls -1 "$BANDS"
echo "--- extract ---"
ls -1 "$EXTRACT"
echo "--- conversion ---"
ls -1 "$CONV"
echo "--- core ---"
ls -1 "$CORE"
echo "--- convolution ---"
ls -1 "$CONVOL"
echo "--- matrix ---"
ls -1 "$MATRIX"
echo "--- colour ---"
ls -1 "$COLOUR"
echo "--- histogram ---"
ls -1 "$HIST"
echo "--- composite ---"
ls -1 "$COMP"
echo "--- freqfilt ---"
ls -1 "$FREQ"
echo "--- mosaicing ---"
ls -1 "$MOS"
echo "--- create ---"
ls -1 "$CREATE"
echo "--- draw ---"
ls -1 "$DRAW"
echo "--- resample ---"
ls -1 "$RESAMPLE"
echo "--- aritha ---"
ls -1 "$ARITHA"
echo "--- arithb ---"
ls -1 "$ARITHB"
echo "--- iocleanup ---"
ls -1 "$IOCLEAN"
