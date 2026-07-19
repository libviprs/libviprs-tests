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
# CONVERSION FAMILY (the Wave-2 conversion lane; OP_MAP.md conversion section).
#
# The same committed inputs feed BOTH this generator (to make the vips
# references) and tests/cli_conversion_diff.rs (which feeds them to `viprs`).
# Most commands are oracle class EXACT (integer-in / integer-out, decode-compare
# tol 0); `flatten` is BOUNDED-TOL (≤1 LSB alpha-blend rounding) and `grey` is
# BOUNDED-TOL (float ramp `.v` eps 1e-6 / uchar ≤1 LSB). Carriers: 1/3/4-band
# uchar outputs → PNG; float (`cast`→float, `grey` float ramp) and 16-bit
# (`byteswap`, `msb` source) outputs whose byte order / raw bands PNG cannot
# carry → the native `.v` container (CLI_CONTRACT.md §2).
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

echo "==> [conversion input] gray16 (16x16 Gray16 ramp), odd (15x15 Gray8)"
"$VIPS" linear "$TMP/cg.v" "$TMP/cg16.v" 65535 0
"$VIPS" cast "$TMP/cg16.v" "$CONV/gray16.png" ushort
"$VIPS" grey "$TMP/codd.v" 15 15
"$VIPS" linear "$TMP/codd.v" "$CONV/odd.png" 255 0 --uchar

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

echo "==> [flip] horizontal + vertical (EXACT PNG)"
"$VIPS" flip "$CONV/gray.png" "$CONV/flip_horizontal_expected.png" horizontal
"$VIPS" flip "$CONV/gray.png" "$CONV/flip_vertical_expected.png"   vertical

echo "==> [rot] d90 + d180 (EXACT PNG)"
"$VIPS" rot "$CONV/gray.png" "$CONV/rot_d90_expected.png"  d90
"$VIPS" rot "$CONV/gray.png" "$CONV/rot_d180_expected.png" d180

echo "==> [rot45] odd-square, --angle d45 (EXACT PNG)"
"$VIPS" rot45 "$CONV/odd.png" "$CONV/rot45_d45_expected.png" --angle d45

echo "==> [byteswap] gray16 -> .v (16-bit; PNG re-encode would normalise) (EXACT)"
"$VIPS" byteswap "$CONV/gray16.png" "$CONV/byteswap_expected.v"

echo "==> [msb] gray16 default (all bands) + --band 0 -> Gray8 (EXACT PNG)"
"$VIPS" msb "$CONV/gray16.png" "$CONV/msb_expected.png"
"$VIPS" msb "$CONV/gray16.png" "$CONV/msb_band0_expected.png" --band 0

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

echo "==> [wrap] gray (default centre shift) (EXACT PNG)"
"$VIPS" wrap "$CONV/gray.png" "$CONV/wrap_expected.png"

# gamma is BOUNDED-TOL (≤1 LSB), NOT EXACT/EAC as OP_MAP.md provisionally listed:
# the per-sample power LUT is computed and rounded independently by the core and
# by vips, and the two land ±1 apart on some samples (a measured, honest
# core-vs-vips rounding difference — see the open question in the wave report).
echo "==> [gamma] gray default + --exponent 2.0 (BOUNDED-TOL ≤1 LSB PNG)"
"$VIPS" gamma "$CONV/gray.png" "$CONV/gamma_expected.png"
"$VIPS" gamma "$CONV/gray.png" "$CONV/gamma_exp2_expected.png" --exponent 2.0

echo "==> [falsecolour] gray -> 3-band sRGB via PET LUT (EXACT PNG)"
"$VIPS" falsecolour "$CONV/gray.png" "$CONV/falsecolour_expected.png"

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
cat >> "$FIX_ROOT/PROVENANCE.md" <<EOF

---

# conversion family CLI-differential reference provenance

These fixtures are the committed vips oracle references the conversion
CLI-differential suite (\`tests/cli_conversion_diff.rs\`) decode-compares \`viprs\`
output against. Generated offline by \`tools/gen_cli_expected.sh\`, NEVER by CI.

- **Oracle**: \`$VIPS_VERSION\`
- **Common inputs** (under \`conversion/\`): \`gray.png\`/\`gray2.png\`/\`gray3.png\`
  (16×16 Gray8), \`rgb.png\`/\`rgba.png\` (sRGB 3/4-band), \`gray16.png\` (16×16
  Gray16 ramp), \`odd.png\` (15×15 Gray8, for \`rot45\`), \`stack.png\` (8×16
  vertical ramp with distinct tiles, for \`grid\`), \`cond.png\`/\`cond2.png\`
  (0/255 masks at thresholds 127/63, for \`ifthenelse\`/\`switch\`).
- **Carriers**: 1/3/4-band uchar → PNG. \`cast\`→float and \`grey\` float ramp →
  \`.v\` (float); \`byteswap\` → \`.v\` (16-bit byte order PNG would normalise).

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

## Exact commands

Inputs:

\`\`\`
vips grey cg.v 16 16
vips linear cg.v conversion/gray.png  255 0  --uchar
vips linear cg.v conversion/gray2.png 200 10 --uchar
vips rot cg.v cg_v.v d90 ; vips linear cg_v.v conversion/gray3.png 255 0 --uchar
vips bandjoin "conversion/gray.png conversion/gray2.png conversion/gray3.png" crgb.v
vips copy crgb.v conversion/rgb.png --interpretation srgb
vips bandjoin "conversion/rgb.png conversion/gray.png" crgba.v
vips copy crgba.v conversion/rgba.png --interpretation srgb
vips linear cg.v cg16.v 65535 0 ; vips cast cg16.v conversion/gray16.png ushort
vips grey codd.v 15 15 ; vips linear codd.v conversion/odd.png 255 0 --uchar
vips grey cs.v 16 8 ; vips rot cs.v cs_v.v d90 ; vips linear cs_v.v conversion/stack.png 255 0 --uchar
vips relational_const conversion/gray.png conversion/cond.png  more 127
vips relational_const conversion/gray.png conversion/cond2.png more 63
vips grey cab.v 8 6 ; vips linear cab.v conversion/autorot_base.png 255 0 --uchar
viprs copy conversion/autorot_base.png conversion/autorot_oriented.v --orientation 6
\`\`\`

References (paths relative to \`tests/fixtures/cli/\`):

| reference | oracle class | vips command |
|---|---|---|
| \`conversion/copy_expected.png\` | EXACT | \`vips copy rgb.png copy_expected.png --interpretation srgb\` |
| \`conversion/cast_ushort_expected.v\` | EXACT | \`vips cast gray.png cast_ushort_expected.v ushort\` (pngsave minimises depth → \`.v\`) |
| \`conversion/cast_float_expected.v\` | EXACT | \`vips cast gray.png cast_float_expected.v float\` |
| \`conversion/flip_horizontal_expected.png\` | EXACT | \`vips flip gray.png flip_horizontal_expected.png horizontal\` |
| \`conversion/flip_vertical_expected.png\` | EXACT | \`vips flip gray.png flip_vertical_expected.png vertical\` |
| \`conversion/rot_d90_expected.png\` | EXACT | \`vips rot gray.png rot_d90_expected.png d90\` |
| \`conversion/rot_d180_expected.png\` | EXACT | \`vips rot gray.png rot_d180_expected.png d180\` |
| \`conversion/rot45_d45_expected.png\` | EXACT | \`vips rot45 odd.png rot45_d45_expected.png --angle d45\` |
| \`conversion/byteswap_expected.v\` | EXACT | \`vips byteswap gray16.png byteswap_expected.v\` |
| \`conversion/msb_expected.png\` | EXACT | \`vips msb gray16.png msb_expected.png\` |
| \`conversion/msb_band0_expected.png\` | EXACT | \`vips msb gray16.png msb_band0_expected.png --band 0\` |
| \`conversion/grid_expected.png\` | EXACT | \`vips grid stack.png grid_expected.png 4 2 2\` |
| \`conversion/flatten_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips flatten rgba.png flatten_expected.png --background "0 0 0"\` |
| \`conversion/ifthenelse_expected.png\` | EXACT | \`vips ifthenelse cond.png gray.png gray2.png ifthenelse_expected.png\` |
| \`conversion/autorot_expected.png\` | EXACT (golden via rot-d90 identity; see above) | \`vips rot autorot_base.png autorot_expected.png d90\` |
| \`conversion/wrap_expected.png\` | EXACT | \`vips wrap gray.png wrap_expected.png\` |
| \`conversion/gamma_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips gamma gray.png gamma_expected.png\` (core vs vips LUT rounding ±1) |
| \`conversion/gamma_exp2_expected.png\` | BOUNDED-TOL ≤1 LSB | \`vips gamma gray.png gamma_exp2_expected.png --exponent 2.0\` |
| \`conversion/falsecolour_expected.png\` | EXACT | \`vips falsecolour gray.png falsecolour_expected.png\` |
| \`conversion/addalpha_expected.png\` | EXACT | \`vips addalpha rgb.png addalpha_expected.png\` |
| \`conversion/arrayjoin_expected.png\` | EXACT | \`vips arrayjoin "gray.png gray2.png gray3.png" arrayjoin_expected.png --across 2\` |
| \`conversion/grey_float_expected.v\` | BOUNDED-TOL (float eps 1e-6) | \`vips grey grey_float_expected.v 16 16\` |
| \`conversion/grey_uchar_expected.png\` | BOUNDED-TOL (≤1 LSB) | \`vips grey grey_uchar_expected.png 16 16 --uchar\` |
| \`conversion/identity_expected.v\` | EXACT | \`vips identity identity_expected.v\` (histogram-tagged → \`.v\`) |
| \`conversion/identity_ushort_expected.v\` | EXACT | \`vips identity identity_ushort_expected.v --ushort\` |
| \`conversion/switch_expected.png\` | EXACT | \`vips switch "cond.png cond2.png" switch_expected.png\` |
EOF

echo "==> Done. Generated fixtures under $FIX_ROOT"
ls -1 "$FIX"
echo "--- bands ---"
ls -1 "$BANDS"
echo "--- conversion ---"
ls -1 "$CONV"
