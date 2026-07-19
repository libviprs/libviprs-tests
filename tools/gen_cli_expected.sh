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
echo "==> [stdif] 3x3 on 2-D eye (BOUNDED-TOL border clip-vs-mirror, PNG)"
"$VIPS" stdif "$ARITHB/eye.png" "$ARITHB/stdif_expected.png" 3 3
echo "==> [recomb] 3x3 matrix (BOUNDED-TOL 1, PNG)"
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
# pow case) and shares the pow(0,0) edge (Rust 1 vs vips 0) at x=0 → tol 1.
echo "==> [math2] atan2 (eps 1e-6) + pow + wop (BOUNDED-TOL 1, 0^0 edge; float .v)"
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
| \`arithb/stdif_expected.png\` | BOUNDED-TOL 6 | \`vips stdif eye.png stdif_expected.png 3 3\` (border clip vs mirror — core issue #490) |
| \`arithb/recomb_expected.png\` | BOUNDED-TOL 1 | \`vips recomb rgb.png recomb_expected.png recomb.mat\` (per-band round vs float — OP_MAP EAC deviation, core issue #491) |
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
| \`arithb/math2_pow_expected.v\` | BOUNDED-TOL 1 | \`vips math2 small.png small2.png math2_pow_expected.v pow\` (pow(0,0): Rust 1 vs vips 0 — core issue #489) |
| \`arithb/math2_wop_expected.v\` | BOUNDED-TOL 1 | \`vips math2 small2.png small.png math2_wop_expected.v wop\` (wop = right^left = small^small2; shares the 0^0 edge — core issue #489) |
| \`arithb/complexform_expected.v\` | FOURIER (eps 1e-6) | \`vips complexform a.png b.png cf_c.v\` then \`vips copy cf_c.v complexform_expected.v --format float --bands 2\` |
| \`arithb/complex_polar_expected.v\` | FOURIER (eps 1e-6) | \`vips complex cpx_c.v cpol.v polar\` then reinterpret \`--format float --bands 2\` |
| \`arithb/complex_rect_expected.v\` | FOURIER (eps 1e-6) | \`vips complex cpx_c.v crect.v rect\` then reinterpret |
| \`arithb/complex_conj_expected.v\` | FOURIER (eps 1e-6) | \`vips complex cpx_c.v cconj.v conj\` then reinterpret |
| \`arithb/complexget_real_expected.v\` | FOURIER (eps 1e-6) | \`vips complexget cpx_c.v complexget_real_expected.v real\` |
| \`arithb/complexget_imag_expected.v\` | FOURIER (eps 1e-6) | \`vips complexget cpx_c.v complexget_imag_expected.v imag\` |
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
echo "--- arithb ---"
ls -1 "$ARITHB"
