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

# gamma is BOUNDED-TOL (≤1 LSB), NOT EXACT/EAC as OP_MAP.md provisionally listed:
# the per-sample power LUT is computed and rounded independently by the core and
# by vips, and the two land ±1 apart on some samples (a measured, honest
# core-vs-vips rounding difference — see the open question in the wave report).
echo "==> [gamma] ramp256 (full 0..255 domain) default + --exponent 2.0 (BOUNDED-TOL ≤1 LSB PNG)"
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
| \`conversion/gamma_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips gamma ramp256.png gamma_expected.png\` (full domain; core vs vips LUT rounding ±1) |
| \`conversion/gamma_exp2_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips gamma ramp256.png gamma_exp2_expected.png --exponent 2.0\` (full domain) |
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
#  * conv / gaussblur at INTEGER precision with a scaled mask are **BOUNDED-TOL
#    ≤1 LSB** (tol 1), NOT the EXACT that OP_MAP.md provisionally listed: vips's
#    `vips_convi` bakes the mask into a power-of-two fixed-point form and shifts
#    (`>> sexp`), while the core divides `(sum + scale/2) / scale`; the two
#    round differently by at most one LSB. This is a MEASURED core-vs-vips
#    rounding difference (a scale-1 mask — as compass uses — is exact, proving
#    it is the scale division, not a CLI bug). Flagged as an open question.
#  * convsep / conv / compass / gaussblur / gaussmat / logmat at FLOAT precision
#    and spcor are float surfaces: compared at a small BOUNDED-TOL (the core
#    accumulates in a different order than vips's vectorised path; measured
#    ≤1.5e-5 on the author Mac, with headroom for cross-platform libm/FMA drift).
#  * gaussmat / logmat at INTEGER precision are integer-valued matrices: tol 0.
#  * sharpen is BOUNDED-TOL ≤1 LSB (the LabS unsharp round trip; measured 1).
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

# --- Mask files (vips text matrix; read by BOTH vips and viprs) --------------
# blur: a 3x3 box (scale 9 -> the fixed-point rounding-scheme gap shows).
# sobel: a 3x3 edge detector (scale 1 -> no coefficient rounding; odd-square, so
#        compass can rotate it). sep: a 1x5 separable smoother (scale 10).
echo "==> [convolution mask] blur.mat / sobel.mat / sep.mat"
printf '3 3 9\n1 1 1\n1 1 1\n1 1 1\n'   > "$CONVOL/blur.mat"
printf '3 3 1\n1 2 1\n0 0 0\n-1 -2 -1\n' > "$CONVOL/sobel.mat"
printf '5 1 10\n1 2 4 2 1\n'            > "$CONVOL/sep.mat"

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

echo "==> [conv] box blur integer (PNG, ≤1 LSB) + sobel float (.v)"
"$VIPS" conv "$CONVOL/eye.png" "$CONVOL/conv_blur_int_expected.png"   "$CONVOL/blur.mat"  --precision integer
"$VIPS" conv "$CONVOL/eye.png" "$CONVOL/conv_sobel_float_expected.v"  "$CONVOL/sobel.mat" --precision float

echo "==> [convsep] separable smoother, float (.v)"
"$VIPS" convsep "$CONVOL/eye.png" "$CONVOL/convsep_float_expected.v" "$CONVOL/sep.mat" --precision float

echo "==> [compass] max integer (PNG, EXACT — scale-1 mask) + sum float/integer (.v)"
"$VIPS" compass "$CONVOL/eye.png" "$CONVOL/compass_max_int_expected.png" "$CONVOL/sobel.mat" \
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
"$VIPS" compass "$CONVOL/eye.png" "$TMP/compass_sum_int_uint.v" "$CONVOL/sobel.mat" \
    --times 4 --angle d45 --combine sum --precision integer
"$VIPS" cast "$TMP/compass_sum_int_uint.v" "$CONVOL/compass_sum_int_expected.v" ushort

echo "==> [gaussblur] integer (PNG, ≤1 LSB) + float (.v)"
"$VIPS" gaussblur "$CONVOL/eye.png" "$CONVOL/gaussblur_int_expected.png" 1.5 --precision integer
"$VIPS" gaussblur "$CONVOL/eye.png" "$CONVOL/gaussblur_float_expected.v" 1.5 --precision float

echo "==> [sharpen] eye (mono, high-freq) --sigma 1 --m1 1 --m2 2 (PNG, ≤1 LSB)"
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
cat >> "$FIX_ROOT/PROVENANCE.md" <<EOF

---

# convolution family CLI-differential reference provenance

These fixtures are the committed vips oracle references the convolution
CLI-differential suite (\`tests/cli_convolution_diff.rs\`) decode-compares
\`viprs\` output against. Generated offline by \`tools/gen_cli_expected.sh\`,
NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`convolution/\`): \`eye.png\` (16×16 Gray8 zone-plate,
  high-frequency so a blur/edge op is non-vacuous — a box blur moves it by up to
  59, compass by 254; \`sharpen\` runs on that same mono zone-plate),
  \`patch.png\` (5×5 extract of \`eye.png\`, the correlation template with a
  sharp best-match peak).
- **Masks**: \`blur.mat\` (3×3 box, scale 9), \`sobel.mat\` (3×3 edge, scale 1,
  odd-square for \`compass\`), \`sep.mat\` (1×5 separable smoother, scale 10).
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
- **BOUNDED-TOL ≤1 LSB (tol 1)**: \`conv\` and \`gaussblur\` at **integer**
  precision with a **scaled** mask. This is a MEASURED core-vs-vips rounding-scheme
  difference — vips's \`vips_convi\` uses a power-of-two fixed-point shift while the
  core divides \`(sum + scale/2)/scale\` — NOT the EXACT that OP_MAP.md
  provisionally listed. The scale-1 compass case proves it is the scale division
  (a scale-1 mask is exact). Also \`sharpen\` (LabS unsharp round trip).
- **BOUNDED-TOL float (small eps)**: \`conv\`/\`compass\`/\`gaussblur\` at float
  precision, \`convsep\` (measured ≤1.5e-5 on the author Mac), \`gaussmat\`/\`logmat\`
  float precision, and \`spcor\` (eps 1e-5): float surfaces whose accumulation
  order / transcendental libm differs slightly from vips.

## Exact commands

Inputs + masks:

\`\`\`
vips eye veye.v 16 16 ; vips linear veye.v convolution/eye.png 127.5 127.5 --uchar
vips extract_area convolution/eye.png convolution/patch.png 4 4 5 5
printf '3 3 9\\n1 1 1\\n1 1 1\\n1 1 1\\n'    > convolution/blur.mat
printf '3 3 1\\n1 2 1\\n0 0 0\\n-1 -2 -1\\n' > convolution/sobel.mat
printf '5 1 10\\n1 2 4 2 1\\n'              > convolution/sep.mat
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
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | vips command |
|---|---|---|
| \`convolution/gaussmat_int_expected.v\` | BOUNDED-TOL (tol 0) | \`vips gaussmat gm.v 2 0.2\` then \`vips cast gm.v … float\` |
| \`convolution/gaussmat_sep_expected.v\` | BOUNDED-TOL (tol 0) | \`vips gaussmat gms.v 2 0.2 --separable\` then cast float |
| \`convolution/gaussmat_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips gaussmat gmf.v 2 0.2 --precision float\` then cast float |
| \`convolution/logmat_int_expected.v\` | BOUNDED-TOL (tol 0) | \`vips logmat lm.v 2 0.1\` then cast float |
| \`convolution/logmat_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips logmat lmf.v 2 0.1 --separable --precision float\` then cast float |
| \`convolution/conv_blur_int_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips conv eye.png conv_blur_int_expected.png blur.mat --precision integer\` |
| \`convolution/conv_sobel_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips conv eye.png conv_sobel_float_expected.v sobel.mat --precision float\` |
| \`convolution/convsep_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips convsep eye.png convsep_float_expected.v sep.mat --precision float\` |
| \`convolution/compass_max_int_expected.png\` | EXACT (tol 0) | \`vips compass eye.png … sobel.mat --times 4 --angle d45 --combine max --precision integer\` |
| \`convolution/compass_sum_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips compass eye.png … sobel.mat --times 4 --angle d45 --combine sum --precision float\` |
| \`convolution/compass_sum_int_expected.v\` | EXACT (tol 0) | \`vips compass eye.png … sobel.mat --times 4 --angle d45 --combine sum --precision integer\` then \`vips cast … ushort\` (vips emits uint; core emits Gray16; 16-bit-promotion path) |
| \`convolution/gaussblur_int_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips gaussblur eye.png gaussblur_int_expected.png 1.5 --precision integer\` |
| \`convolution/gaussblur_float_expected.v\` | BOUNDED-TOL (float eps) | \`vips gaussblur eye.png gaussblur_float_expected.v 1.5 --precision float\` |
| \`convolution/sharpen_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips sharpen eye.png sharpen_expected.png --sigma 1 --m1 1 --m2 2\` |
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
