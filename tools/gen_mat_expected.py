#!/usr/bin/env python3
# =============================================================================
# gen_mat_expected.py -- OFFLINE generator for the MATLAB .mat decode oracle.
#
# AUTHOR-RUN ONLY, on a machine with `scipy` installed. NEVER run in CI: this
# is the offline half of the never-call-a-live-oracle-from-the-suite rule the
# rest of this repo follows (see tools/gen_cli_expected.sh). It reads the
# libvips reference suite's sample.mat (fetched by
# tools/fetch_reference_suite.sh) with scipy.io.loadmat -- an independent
# MATLAB level-5 reader with no relationship to libviprs's own src/mat.rs --
# and writes the single committed fixture
# tests/fixtures/mat/sample_mat_expected.blake3 that
# tests/ported_foreign.rs::test_matload compares libviprs's decode against.
#
# Why a hash and not a raw pixel dump: the reference array is 290x442x3
# uint16 (769,080 bytes). Committing that raw would triple this repo's
# fixture footprint for one test. A sha256 digest is exactly as strong a
# comparison (a collision is not a realistic concern) and matches this
# repo's own checksum machinery, which supports sha256 as well as its
# blake3 default (tests/phase3_checksum.rs); sha256 is used here because
# it needs no extra Python package, only the stdlib.
#
# The byte layout hashed here has to match libviprs::Raster::data() exactly:
# row-major, band-interleaved, native-endian (little-endian, which is what
# both this repo's CI runners and every machine that has run this script are).
# scipy.io.loadmat returns the array already indexed the same way MATLAB
# indexes it (I[row, col, band]), and mat2vips_get_header's own transpose
# (dims[0] -> height, dims[1] -> width) means output pixel (x, y, band) is
# I[y, x, band] -- see src/mat.rs's module doc in the libviprs core repo for
# the full derivation. `ndarray.tobytes(order='C')` serializes exactly that
# (row-major over (height, width, band)) regardless of the array's internal
# memory layout, so no manual transpose loop is needed here.
#
# Usage (from the repo root, with the reference suite already fetched):
#   python3 tools/gen_mat_expected.py
#
# Requires: scipy (`pip install scipy`), which is not a runtime or dev
# dependency of anything else in this repo -- it is this script's oracle.
# =============================================================================
import hashlib
import pathlib
import sys

try:
    import scipy.io as sio
except ImportError:
    print("ERROR: scipy is required (pip install scipy). Not a repo dependency,", file=sys.stderr)
    print("just this generator's independent MAT-file oracle.", file=sys.stderr)
    sys.exit(1)

REPO_ROOT = pathlib.Path(__file__).resolve().parent.parent
SAMPLE_MAT = REPO_ROOT / "tmp/libvips-reference-tests/test-suite/images/sample.mat"
OUT = REPO_ROOT / "tests/fixtures/mat/sample_mat_expected.sha256"


def main() -> None:
    if not SAMPLE_MAT.exists():
        print(f"ERROR: {SAMPLE_MAT} not found.", file=sys.stderr)
        print("Run ./tools/fetch_reference_suite.sh first.", file=sys.stderr)
        sys.exit(1)

    data = sio.loadmat(str(SAMPLE_MAT))
    variables = [k for k in data if not k.startswith("__")]
    if variables != ["I"]:
        print(f"ERROR: expected exactly one variable 'I', found {variables}", file=sys.stderr)
        sys.exit(1)

    arr = data["I"]
    print(f"variable 'I': shape={arr.shape} dtype={arr.dtype}")
    if arr.ndim != 3 or arr.shape[2] != 3:
        print("ERROR: expected a rank-3 array with 3 bands", file=sys.stderr)
        sys.exit(1)
    if arr.dtype.name != "uint16":
        print(f"ERROR: expected uint16 samples, found {arr.dtype}", file=sys.stderr)
        sys.exit(1)

    # Little-endian u16, row-major (height, width, band) -- see module doc.
    raw = arr.astype("<u2").tobytes(order="C")
    digest = hashlib.sha256(raw).hexdigest()

    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(f"sha256:{digest}\n")
    print(f"wrote {OUT} (sha256:{digest})")

    # A handful of named pixels for the reader who wants a value without
    # running anything: (row, col) in scipy/MATLAB order, i.e. (y, x).
    h, w, _ = arr.shape
    for label, (y, x) in {
        "top-left": (0, 0),
        "top-right": (0, w - 1),
        "bottom-left": (h - 1, 0),
        "bottom-right": (h - 1, w - 1),
        "center": (h // 2, w // 2),
    }.items():
        print(f"{label:>12} (y={y}, x={x}): {list(int(v) for v in arr[y, x, :])}")


if __name__ == "__main__":
    main()
