#!/usr/bin/env python3
# =============================================================================
# gen_pdfium_goldens.py
#
# Regenerate the golden PNG reference renders for the libviprs-tests PDF
# rotation-correctness suite, produced by *libvips built with the pdfium PDF
# backend* (see tools/Dockerfile.libvips-pdfium).
#
# libviprs (a pure-Rust libvips port) renders PDFs via pdfium. To assert that
# libviprs matches libvips on rotated PDFs, the golden references must come from
# libvips using the SAME pdfium engine — not the poppler backend that stock
# libvips ships. Homebrew libvips is poppler-backed, and a from-source
# libvips+pdfium build on this macOS host is blocked by a broken CommandLineTools
# libc++, so libvips+pdfium is built and run inside a Linux container.
#
# This script is the single command that regenerates all four goldens:
#
#     python3 tools/gen_pdfium_goldens.py
#
# It builds the container image if needed (skips rebuild when it already exists,
# unless --rebuild), renders page 0 of each fixture PDF at 72 dpi to PNG in the
# container, writes the goldens to
# tests/fixtures/rotation_libvips_pdfium_expected/, then decodes each golden in
# pure Python (stdlib zlib + struct — no pip deps, no Pillow) and prints a
# dims + quadrant-colour table, gating it against the known-correct orientation.
#
# Modes:
#     (default)     build (if needed) + render + verify + gate
#     --rebuild     force a fresh image build before rendering
#     --verify      re-read the existing goldens and print the table (no Docker)
#
# It is idempotent and re-runnable; renders are deterministic.
# =============================================================================
import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

# --- Paths ------------------------------------------------------------------
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))          # .../tools
REPO = os.path.dirname(SCRIPT_DIR)                                # .../libviprs-tests
FIXTURES_DIR = os.path.join(REPO, "tests", "fixtures")
GOLDEN_DIR = os.path.join(FIXTURES_DIR, "rotation_libvips_pdfium_expected")
DOCKERFILE = os.path.join(SCRIPT_DIR, "Dockerfile.libvips-pdfium")

IMAGE = "libviprs-libvips-pdfium"
DPI = 72

# input fixture (basename, no ext) -> golden PNG basename
FIXTURES = [
    "canonical",
    "canonical_rotated_90",
    "canonical_rotated_180",
    "canonical_rotated_270",
]

# The CORRECT display orientation, verified independently against pdfium's form
# path AND libvips-poppler (agreement 0.08 mean/channel). Quadrant codes are
# sampled at the centre of each quadrant. This is the correctness gate.
EXPECTED = {
    "canonical.png":             {"dims": (595, 842), "TL": "R", "TR": "Y", "BL": "B", "BR": "G"},
    "canonical_rotated_90.png":  {"dims": (842, 595), "TL": "B", "TR": "R", "BL": "G", "BR": "Y"},
    "canonical_rotated_180.png": {"dims": (595, 842), "TL": "G", "TR": "B", "BL": "Y", "BR": "R"},
    "canonical_rotated_270.png": {"dims": (842, 595), "TL": "Y", "TR": "G", "BL": "R", "BR": "B"},
}


# --- Small helpers ----------------------------------------------------------
def log(msg):
    print(msg, flush=True)


def run(cmd, **kw):
    """Run a command, echoing it first. Raises on non-zero exit unless check=False."""
    log("$ " + " ".join(cmd))
    return subprocess.run(cmd, **kw)


def stream(cmd):
    """Run a command, streaming its combined stdout/stderr to our stdout live."""
    log("$ " + " ".join(cmd))
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    for line in proc.stdout:
        sys.stdout.write(line.decode("utf-8", "replace"))
        sys.stdout.flush()
    proc.wait()
    return proc.returncode


# --- Docker orchestration ---------------------------------------------------
def image_exists():
    r = subprocess.run(
        ["docker", "image", "inspect", IMAGE],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    return r.returncode == 0


def build_image(rebuild):
    if image_exists() and not rebuild:
        log("Image '%s' already exists; skipping build (use --rebuild to force)." % IMAGE)
        return
    log("Building image '%s' (this takes several minutes — libvips is built from source)..." % IMAGE)
    cmd = ["docker", "build"]
    if rebuild:
        cmd.append("--no-cache")
    cmd += ["-f", DOCKERFILE, "-t", IMAGE, SCRIPT_DIR]
    rc = stream(cmd)
    if rc != 0:
        raise SystemExit("docker build failed (exit %d)" % rc)
    log("Image built.")


def pdfium_provenance():
    """Report the pinned pdfium tag + pkg-config version baked into the image."""
    r = subprocess.run(
        ["docker", "run", "--rm", IMAGE, "sh", "-c",
         "cat /opt/pdfium/RESOLVED_TAG 2>/dev/null; "
         "echo '---'; "
         "PKG_CONFIG_PATH=/usr/local/lib/pkgconfig pkg-config --modversion pdfium 2>/dev/null; "
         "echo '---'; vips --version 2>/dev/null"],
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    out = r.stdout.decode("utf-8", "replace").strip().split("---")
    tag = out[0].strip() if len(out) > 0 else "?"
    pcver = out[1].strip() if len(out) > 1 else "?"
    vips = out[2].strip() if len(out) > 2 else "?"
    return tag, pcver, vips


def render_goldens():
    """Render all fixtures in one container run into a repo-local temp dir,
    then copy the four PNGs into the goldens directory."""
    os.makedirs(GOLDEN_DIR, exist_ok=True)
    # Repo-local temp dir (under /Users -> guaranteed shared with Docker Desktop,
    # and written files come back owned by the host user, not root).
    tmp_out = tempfile.mkdtemp(prefix=".pdfium_render_", dir=SCRIPT_DIR)
    try:
        # Build the render loop: page 0 @ 72 dpi -> RGBA PNG, for each fixture.
        names = " ".join(FIXTURES)
        render_sh = (
            "set -eu; "
            "for f in %s; do "
            "  echo \"rendering $f.pdf -> $f.png\"; "
            "  vips pdfload \"/in/$f.pdf\" \"/out/$f.png\" --page 0 --dpi %d; "
            "done" % (names, DPI)
        )
        cmd = [
            "docker", "run", "--rm",
            "-v", "%s:/in:ro" % FIXTURES_DIR,
            "-v", "%s:/out" % tmp_out,
            IMAGE, "bash", "-c", render_sh,
        ]
        rc = stream(cmd)
        if rc != 0:
            raise SystemExit("render container failed (exit %d)" % rc)
        # Validate presence, then copy into place.
        for f in FIXTURES:
            src = os.path.join(tmp_out, f + ".png")
            if not os.path.isfile(src):
                raise SystemExit("expected render missing: %s" % src)
            dst = os.path.join(GOLDEN_DIR, f + ".png")
            shutil.copyfile(src, dst)
            log("wrote %s (%d bytes)" % (dst, os.path.getsize(dst)))
    finally:
        shutil.rmtree(tmp_out, ignore_errors=True)


# --- Pure-stdlib PNG decode (no Pillow, no pip) -----------------------------
def decode_png(path):
    """Decode a non-interlaced 8-bit PNG (grayscale / RGB / RGBA) with pure
    stdlib. Returns dict(width, height, channels, pixels: bytearray)."""
    with open(path, "rb") as fh:
        data = fh.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG: %s" % path)
    pos = 8
    width = height = bit_depth = color_type = interlace = None
    idat = bytearray()
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        ctype = data[pos + 4:pos + 8]
        cdata = data[pos + 8:pos + 8 + length]
        pos += 12 + length  # 4 len + 4 type + data + 4 crc
        if ctype == b"IHDR":
            (width, height, bit_depth, color_type, _comp, _filt, interlace) = \
                struct.unpack(">IIBBBBB", cdata)
        elif ctype == b"IDAT":
            idat += cdata
        elif ctype == b"IEND":
            break
    if bit_depth != 8:
        raise ValueError("unsupported bit depth %r in %s" % (bit_depth, path))
    if interlace != 0:
        raise ValueError("interlaced PNG unsupported: %s" % path)
    channels_by_ct = {0: 1, 2: 3, 4: 2, 6: 4}
    if color_type not in channels_by_ct:
        raise ValueError("unsupported colour type %r (palette?) in %s" % (color_type, path))
    channels = channels_by_ct[color_type]

    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    bpp = channels  # bytes per pixel at 8-bit depth
    out = bytearray(width * height * channels)
    prev = bytearray(stride)
    ip = 0
    for y in range(height):
        ftype = raw[ip]; ip += 1
        line = bytearray(raw[ip:ip + stride]); ip += stride
        if ftype == 0:                       # None
            pass
        elif ftype == 1:                     # Sub
            for i in range(bpp, stride):
                line[i] = (line[i] + line[i - bpp]) & 0xFF
        elif ftype == 2:                     # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif ftype == 3:                     # Average
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 0xFF
        elif ftype == 4:                     # Paeth
            for i in range(stride):
                a = line[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 0xFF
        else:
            raise ValueError("bad PNG filter %d in %s" % (ftype, path))
        out[y * stride:(y + 1) * stride] = line
        prev = line
    return {"width": width, "height": height, "channels": channels, "pixels": out}


def pixel_rgb(img, x, y):
    ch = img["channels"]
    off = (y * img["width"] + x) * ch
    px = img["pixels"]
    if ch >= 3:
        return px[off], px[off + 1], px[off + 2]
    v = px[off]
    return v, v, v  # grayscale


def classify(rgb):
    """Classify a quadrant-centre pixel as R / Y / G / B / white, else ?(r,g,b)."""
    r, g, b = rgb
    hi, lo = 180, 80
    R, G, B = r > hi, g > hi, b > hi
    r0, g0, b0 = r < lo, g < lo, b < lo
    if R and G and B:
        return "white"
    if R and g0 and b0:
        return "R"
    if R and G and b0:
        return "Y"
    if r0 and G and b0:
        return "G"
    if r0 and g0 and B:
        return "B"
    return "?(%d,%d,%d)" % (r, g, b)


def quadrants(img):
    w, h = img["width"], img["height"]
    pts = {
        "TL": (w // 4, h // 4),
        "TR": (3 * w // 4, h // 4),
        "BL": (w // 4, 3 * h // 4),
        "BR": (3 * w // 4, 3 * h // 4),
    }
    return {k: classify(pixel_rgb(img, x, y)) for k, (x, y) in pts.items()}


# --- Verify + gate ----------------------------------------------------------
def verify_and_gate():
    header = "%-28s %-11s %-9s %-4s %-4s %-4s %-4s %s" % (
        "golden", "dims", "exp dims", "TL", "TR", "BL", "BR", "result")
    log("")
    log(header)
    log("-" * len(header))
    all_pass = True
    for name in FIXTURES:
        fname = name + ".png"
        path = os.path.join(GOLDEN_DIR, fname)
        exp = EXPECTED[fname]
        if not os.path.isfile(path):
            log("%-28s MISSING -> FAIL" % fname)
            all_pass = False
            continue
        img = decode_png(path)
        dims = (img["width"], img["height"])
        q = quadrants(img)
        dims_ok = dims == exp["dims"]
        quad_ok = all(q[k] == exp[k] for k in ("TL", "TR", "BL", "BR"))
        ok = dims_ok and quad_ok
        all_pass = all_pass and ok
        log("%-28s %-11s %-9s %-4s %-4s %-4s %-4s %s" % (
            fname,
            "%dx%d" % dims,
            "%dx%d" % exp["dims"],
            q["TL"], q["TR"], q["BL"], q["BR"],
            "PASS" if ok else "FAIL",
        ))
        if not ok:
            det = []
            if not dims_ok:
                det.append("dims %dx%d != expected %dx%d" % (dims[0], dims[1], exp["dims"][0], exp["dims"][1]))
            for k in ("TL", "TR", "BL", "BR"):
                if q[k] != exp[k]:
                    det.append("%s=%s expected %s" % (k, q[k], exp[k]))
            log("    ^ mismatch: " + "; ".join(det))
    log("")
    if all_pass:
        log("ALL GOLDENS MATCH THE EXPECTED ORIENTATION TABLE.")
    else:
        log("ONE OR MORE GOLDENS DO NOT MATCH — the build/render is wrong; do NOT")
        log("hand-massage the images. Diagnose the libvips+pdfium build/render above.")
    return all_pass


# --- CLI --------------------------------------------------------------------
def main():
    ap = argparse.ArgumentParser(description="Generate libvips+pdfium golden PDF renders.")
    ap.add_argument("--rebuild", action="store_true",
                    help="force a fresh (no-cache) image build before rendering")
    ap.add_argument("--verify", action="store_true",
                    help="only re-read existing goldens and print/gate the quadrant table (no Docker)")
    args = ap.parse_args()

    if args.verify:
        ok = verify_and_gate()
        raise SystemExit(0 if ok else 1)

    build_image(args.rebuild)
    tag, pcver, vips = pdfium_provenance()
    log("")
    log("pdfium release tag : %s" % tag)
    log("pdfium.pc version  : %s" % pcver)
    log("libvips version    : %s" % vips)

    render_goldens()
    ok = verify_and_gate()

    log("")
    log("Goldens written to: %s" % GOLDEN_DIR)
    log("Regenerate anytime with: python3 tools/gen_pdfium_goldens.py")
    raise SystemExit(0 if ok else 1)


if __name__ == "__main__":
    main()
