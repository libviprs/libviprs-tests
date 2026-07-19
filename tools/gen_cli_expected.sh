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
echo "==> Using oracle: $VIPS_VERSION"

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

# BOUNDED-TOL (≤1 LSB), NOT EXACT: rgb.png is a bandjoin of three DISTINCT grays,
# so a per-pixel band sum is generally NOT divisible by 3. The core FLOORS the
# integer mean (truncating division) while vips ROUNDS to nearest, so the two
# diverge by at most one LSB — an honest, non-vacuous BOUNDED-TOL case (the old
# rgb_eq.png had three identical bands, making mean == input: divisible AND
# arithmetically vacuous). The bands cell decode-compares this at tol 1.
echo "==> [bandmean] rgb (3 distinct bands) -> 1-band mean PNG (BOUNDED-TOL ≤1 LSB)"
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
| \`bands/bandmean_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips bandmean rgb.png bandmean_expected.png\` (3 distinct bands → core floors vs vips rounds, tol 1) |
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
# distinct colour stands out wherever `insert` places it. `vips grey`/`rot` are
# pure coordinate functions, so every fixture is bit-reproducible.
echo "==> [extract input] 16x16 gray ramp + 16x16 sRGB (2-D) rgb + 6x6 sub"
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

# --- References — one vips run per differential case -------------------------
echo "==> [extract_area/crop] interior rectangle (S1 EXACT)"
"$VIPS" extract_area "$EXTRACT/rgb.png" "$EXTRACT/extract_area_expected.png" 3 4 5 6
"$VIPS" crop         "$EXTRACT/rgb.png" "$EXTRACT/crop_expected.png"         3 4 5 6

echo "==> [embed] black + copy (enum) + background vector (S1 EXACT)"
"$VIPS" embed "$EXTRACT/rgb.png"  "$EXTRACT/embed_black_expected.png" 2 3 24 24
"$VIPS" embed "$EXTRACT/rgb.png"  "$EXTRACT/embed_copy_expected.png"  2 3 24 24 --extend copy
"$VIPS" embed "$EXTRACT/gray.png" "$EXTRACT/embed_bg_expected.png"    1 1 8 8 --extend background --background 128

echo "==> [gravity] centre + south-east (dash-spelled enum) (S1 EXACT)"
"$VIPS" gravity "$EXTRACT/rgb.png" "$EXTRACT/gravity_centre_expected.png" centre     24 24
"$VIPS" gravity "$EXTRACT/rgb.png" "$EXTRACT/gravity_se_expected.png"     south-east 24 24

echo "==> [replicate/zoom/subsample] integer geometry (S1 EXACT)"
"$VIPS" replicate "$EXTRACT/rgb.png"  "$EXTRACT/replicate_expected.png" 2 3
"$VIPS" zoom      "$EXTRACT/gray.png" "$EXTRACT/zoom_expected.png"      3 2
"$VIPS" subsample "$EXTRACT/rgb.png"  "$EXTRACT/subsample_expected.png" 2 2

echo "==> [insert] non-expand + expand (canvas grows) (S2 EXACT)"
"$VIPS" insert "$EXTRACT/rgb.png" "$EXTRACT/sub.png" "$EXTRACT/insert_expected.png"        4 5
"$VIPS" insert "$EXTRACT/rgb.png" "$EXTRACT/sub.png" "$EXTRACT/insert_expand_expected.png" 13 13 --expand

echo "==> [smartcrop] centre/low/high/attention (S1 EXACT — discriminating geometry + saliency)"
"$VIPS" smartcrop "$EXTRACT/rgb.png" "$EXTRACT/smartcrop_centre_expected.png"    8 8 --interesting centre
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
  solid sRGB, the distinct insert payload).
- **Every op is EXACT** (integer-in / integer-out, decode-compare tol 0) EXCEPT
  \`smartcrop --interesting entropy\`, which is **GOLDEN-ONLY**: vips's entropy
  strategy makes a different discrete crop-window choice than the core on this
  input (measured max-abs-diff 136 — a wholesale different region, not a
  tolerance), so there is no cross-oracle. Its reference
  (\`smartcrop_entropy_golden.png\`) is generated by \`viprs\` itself
  (deterministic across runs) and the test is a regression pin.

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
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | vips command |
|---|---|---|
| \`extract/extract_area_expected.png\` | EXACT | \`vips extract_area rgb.png extract_area_expected.png 3 4 5 6\` |
| \`extract/crop_expected.png\` | EXACT | \`vips crop rgb.png crop_expected.png 3 4 5 6\` (alias of extract_area) |
| \`extract/embed_black_expected.png\` | EXACT | \`vips embed rgb.png embed_black_expected.png 2 3 24 24\` |
| \`extract/embed_copy_expected.png\` | EXACT | \`vips embed rgb.png embed_copy_expected.png 2 3 24 24 --extend copy\` |
| \`extract/embed_bg_expected.png\` | EXACT | \`vips embed gray.png embed_bg_expected.png 1 1 8 8 --extend background --background 128\` |
| \`extract/gravity_centre_expected.png\` | EXACT | \`vips gravity rgb.png gravity_centre_expected.png centre 24 24\` |
| \`extract/gravity_se_expected.png\` | EXACT | \`vips gravity rgb.png gravity_se_expected.png south-east 24 24\` |
| \`extract/replicate_expected.png\` | EXACT | \`vips replicate rgb.png replicate_expected.png 2 3\` |
| \`extract/zoom_expected.png\` | EXACT | \`vips zoom gray.png zoom_expected.png 3 2\` |
| \`extract/subsample_expected.png\` | EXACT | \`vips subsample rgb.png subsample_expected.png 2 2\` |
| \`extract/insert_expected.png\` | EXACT | \`vips insert rgb.png sub.png insert_expected.png 4 5\` |
| \`extract/insert_expand_expected.png\` | EXACT | \`vips insert rgb.png sub.png insert_expand_expected.png 13 13 --expand\` (canvas grows to 19×19) |
| \`extract/smartcrop_centre_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_centre_expected.png 8 8 --interesting centre\` |
| \`extract/smartcrop_low_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_low_expected.png 8 8 --interesting low\` |
| \`extract/smartcrop_high_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_high_expected.png 8 8 --interesting high\` |
| \`extract/smartcrop_attention_expected.png\` | EXACT | \`vips smartcrop rgb.png smartcrop_attention_expected.png 8 8 --interesting attention\` (crop 15,11 — non-vacuous, differs from low/high) |
| \`extract/smartcrop_entropy_golden.png\` | GOLDEN-ONLY | \`viprs smartcrop rgb.png smartcrop_entropy_golden.png 8 8 --interesting entropy\` (NO vips oracle — vips picks a different discrete window) |
EOF

echo "==> Done. Generated fixtures under $FIX_ROOT"
ls -1 "$FIX"
echo "--- bands ---"
ls -1 "$BANDS"
echo "--- extract ---"
ls -1 "$EXTRACT"
