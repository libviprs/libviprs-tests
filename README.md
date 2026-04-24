<p align="center">
  <img src="https://raw.githubusercontent.com/libviprs/libviprs/main/images/libviprs-logo-claws.svg" alt="libviprs" width="200">
</p>

<h1 align="center">libviprs-tests</h1>

<p align="center">
  <a href="https://github.com/libviprs/libviprs-tests/actions/workflows/ci.yml"><img src="https://github.com/libviprs/libviprs-tests/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust" alt="Rust 1.85+">
  <img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT License">
</p>

Integration and system tests for [libviprs](../libviprs), a pure-Rust image pyramiding engine.

This crate is kept in a separate repository to keep the libviprs library crate clean — no test fixtures, no heavy test-only dependencies.

## Running Tests

```bash
# All integration tests (no pdfium required)
cargo test

# Include pdfium tests (requires libpdfium installed)
cargo test --features pdfium

# Stress tests (large images, slow)
cargo test -- --ignored stress

# PDFium system check (manual diagnostic)
cargo test --features pdfium -- --ignored pdfium_system
```

### Docker (via run-tests.sh)

`run-tests.sh` builds a Docker image with PDFium and runs the full test suite
without requiring any local dependencies. It can be invoked from either repo:

```bash
# From libviprs/
./run-tests.sh              # auto-detect arch
./run-tests.sh arm          # force arm64
./run-tests.sh amd64        # force amd64

# From libviprs-tests/
./tools/run-tests.sh
```

#### Miri and Loom

After the Docker tests pass, you can optionally run Miri and/or Loom on the
host (not in Docker):

```bash
./run-tests.sh --miri           # + Miri (requires nightly + miri component)
./run-tests.sh --loom           # + Loom concurrency tests
./run-tests.sh --miri --loom    # both
./run-tests.sh arm --miri       # combine arch + flags
```

Miri runs `cargo +nightly miri test` on libviprs. Loom runs
`RUSTFLAGS="--cfg loom" cargo test --lib loom_tests` on libviprs.

#### Docker directly

You can also use Docker without the script:

```bash
# From the workspace root (parent of libviprs/ and libviprs-tests/)
docker build -f libviprs-tests/Dockerfile -t libviprs-tests .

# Run all tests (default)
docker run --rm libviprs-tests

# Run specific tests
docker run --rm libviprs-tests cargo test --features pdfium pdf_to_pyramid

# Run stress tests
docker run --rm libviprs-tests cargo test -- --ignored stress

# PDFium system check
docker run --rm libviprs-tests cargo test --features pdfium -- --ignored pdfium_system
```

## Git Hooks

`install-hooks.sh` installs pre-commit and pre-push hooks into all three
repos (libviprs, libviprs-cli, libviprs-tests):

```bash
# From libviprs-tests/
./tools/install-hooks.sh
```

**Pre-commit** (runs on every `git commit`):
- `cargo fmt -- --check` — rejects unformatted code
- `cargo clippy --all-targets -- -D warnings` — rejects lint warnings

**Pre-push** (runs on every `git push`):
- Runs the full Docker test suite via `run-tests.sh`

To bypass in emergencies: `git commit --no-verify` or `git push --no-verify`.

## Test Suites

### Core Pipeline

| File | Tests | Description |
|---|---|---|
| `pdf_to_pyramid.rs` | 4 | End-to-end: PDF extract → geo-reference → tile pyramid. Core workflow from issue #142. |
| `pyramid_determinism.rs` | 2 | Output is identical across concurrency levels and tile sizes. Mirrors libvips' `test_threading.sh`. |
| `pyramid_fs_sink.rs` | 5 | Filesystem output: DeepZoom/XYZ layouts, PNG/JPEG/Raw encoding, DZI manifests. |
| `no_temp_files.rs` | 1 | Engine completes without creating temp files (read-only TMPDIR). Mirrors libvips' `test_seq.sh`. |

### PDF

| File | Tests | Description |
|---|---|---|
| `pdf_ops.rs` | 6 | PDF parsing via lopdf: page count, dimensions, image detection, extraction, error handling. |
| `pdf_cmyk.rs` | 3 | CMYK FlateDecode path: synthetic CMYK PDFs verify the DeviceCMYK → RGB conversion. |

### Observability

| File | Tests | Description |
|---|---|---|
| `observability.rs` | 4 | Progress events match tile counts, level ordering, memory bounds. |

### Stress (ignored by default)

| File | Tests | Description |
|---|---|---|
| `stress.rs` | 3 | 10K×10K image, 4K determinism under high concurrency, 100× rapid-fire small pyramids. |

### PDFium

All PDFium tests require `--features pdfium` and a PDFium shared library installed on the system.

| File | Tests | Description |
|---|---|---|
| `pdfium_integration.rs` | 6 | Library loading, page info, bitmap rendering, `render_page_pdfium` end-to-end, error handling. |
| `pdfium_system_check.rs` | 2 | **Manual diagnostic** (`--ignored`). Reports library search paths, verifies ABI compatibility, prints install instructions on failure. |

## PDFium Setup

The `pdfium` feature requires the PDFium shared library at runtime. pdfium-render 0.8.x dynamically loads it from system library paths.

### Linux

Pre-compiled binaries built from source are available from [libviprs-dep](https://github.com/libviprs/libviprs-dep/releases):

```bash
# x86_64
curl -L -o pdfium.tgz \
  https://github.com/libviprs/libviprs-dep/releases/download/pdfium-7725/pdfium-linux-x64.tgz

# arm64
curl -L -o pdfium.tgz \
  https://github.com/libviprs/libviprs-dep/releases/download/pdfium-7725/pdfium-linux-arm64.tgz

# Extract and install
tar xzf pdfium.tgz
sudo cp pdfium-linux-*/lib/libpdfium.so /usr/local/lib/
sudo ldconfig
```

See the [libviprs-dep pdfium README](https://github.com/libviprs/libviprs-dep/tree/main/pdfium) for building PDFium from source or finding other versions.

### macOS

Third-party prebuilt binaries:

```bash
# Apple Silicon
curl -L -o pdfium.tgz https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-mac-arm64.tgz
tar xzf pdfium.tgz
sudo cp lib/libpdfium.dylib /usr/local/lib/

# Intel
curl -L -o pdfium.tgz https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-mac-x64.tgz
tar xzf pdfium.tgz
sudo cp lib/libpdfium.dylib /usr/local/lib/
```

### Verify Installation

```bash
cargo test --features pdfium -- --ignored pdfium_system_check
```

This runs a diagnostic that checks library discovery, ABI compatibility, PDF loading, and rendering.

## Test Fixtures

Located in `tests/fixtures/`:

| File | Description |
|---|---|
| `blueprint.pdf` | Real scanned blueprint PDF used by PDF extraction and end-to-end tests. |
| `password.pdf` | AES-256 encrypted single-page PDF. Password: **`secret`** (both user and owner). Used by `ported_foreign::test_pdf_password`. |

## Project Structure

```
libviprs-tests/
├── Cargo.toml
├── Dockerfile
├── README.md
├── tools/
│   ├── install-hooks.sh
│   └── run-tests.sh
├── .github/
│   └── workflows/
│       └── ci.yml
├── tests/
│   ├── fixtures/
│   │   ├── blueprint.pdf
│   │   └── password.pdf
│   ├── no_temp_files.rs
│   ├── observability.rs
│   ├── pdf_cmyk.rs
│   ├── pdf_ops.rs
│   ├── pdf_to_pyramid.rs
│   ├── pdfium_integration.rs
│   ├── pdfium_system_check.rs
│   ├── pyramid_determinism.rs
│   ├── pyramid_fs_sink.rs
│   └── stress.rs
└── .gitignore
```

## Dependencies

- **libviprs** — the library under test (path dependency)
- **tempfile** — temporary directories for filesystem sink tests
- **flate2** — zlib compression for synthetic CMYK PDF construction
- **pdfium-render** (optional) — PDFium bindings for pdfium feature tests
