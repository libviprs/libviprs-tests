# libviprs-tests

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

### Docker

Run the full test suite (including PDFium) without installing anything locally:

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

### macOS

```bash
# Apple Silicon
curl -L -o pdfium.tgz https://github.com/niclasvaneyk/pdfium-apple-silicon/releases/latest/download/pdfium-mac-arm64.tgz
tar xzf pdfium.tgz
sudo cp lib/libpdfium.dylib /usr/local/lib/

# Intel
curl -L -o pdfium.tgz https://github.com/niclasvaneyk/pdfium-macos-x64/releases/latest/download/pdfium-mac-x64.tgz
tar xzf pdfium.tgz
sudo cp lib/libpdfium.dylib /usr/local/lib/
```

### Linux

```bash
curl -L -o pdfium.tgz https://github.com/niclasvaneyk/pdfium-linux-x64/releases/latest/download/pdfium-linux-x64.tgz
tar xzf pdfium.tgz
sudo cp lib/libpdfium.so /usr/local/lib/
sudo ldconfig
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

## Project Structure

```
libviprs-tests/
├── Cargo.toml
├── Dockerfile
├── README.md
├── .github/
│   └── workflows/
│       └── ci.yml
├── tests/
│   ├── fixtures/
│   │   └── blueprint.pdf
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
