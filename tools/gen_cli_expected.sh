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

echo "==> Done. Generated fixtures under $FIX_ROOT"
ls -1 "$FIX"
echo "--- bands ---"
ls -1 "$BANDS"
echo "--- core ---"
ls -1 "$CORE"
