# ---------------------------------------------------------------------------
# Dockerfile — run libviprs + libviprs-tests with PDFium (amd64 + arm64)
#
# Supports multiple run modes via RUN_MODE env var:
#   ci    (default) — fmt, clippy, tests (libviprs + libviprs-tests with pdfium)
#   test            — tests only (libviprs + libviprs-tests with pdfium)
#   miri            — cargo +nightly miri test on libviprs
#   loom            — loom concurrency tests on libviprs
#   full            — ci + miri + loom
# ---------------------------------------------------------------------------

# Stage 1: Download PDFium shared library for the target architecture
FROM debian:bookworm-slim AS pdfium

RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/*

ARG TARGETARCH
RUN case "${TARGETARCH}" in \
        amd64) PDFIUM_ARCH="linux-x64" ;; \
        arm64) PDFIUM_ARCH="linux-arm64" ;; \
        *)     echo "Unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac && \
    curl -L -o /tmp/pdfium.tgz \
        "https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-${PDFIUM_ARCH}.tgz" && \
    mkdir -p /opt/pdfium && \
    tar xzf /tmp/pdfium.tgz -C /opt/pdfium && \
    rm /tmp/pdfium.tgz

# Stage 2: Build and test environment
FROM rust:latest

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Install PDFium shared library
COPY --from=pdfium /opt/pdfium/lib/libpdfium.so /usr/local/lib/libpdfium.so
RUN ldconfig

# Install nightly toolchain + miri (used by miri/full modes)
RUN rustup toolchain install nightly \
    && rustup component add --toolchain nightly miri rustfmt clippy

WORKDIR /src

# Copy both crates
COPY libviprs/ libviprs/
COPY libviprs-tests/ libviprs-tests/

# Fetch dependencies for both crates
WORKDIR /src/libviprs
RUN cargo fetch
RUN cargo +nightly fetch

WORKDIR /src/libviprs-tests
RUN cargo fetch

# Copy entrypoint
COPY libviprs-tests/tools/entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENV RUN_MODE=ci

ENTRYPOINT ["/entrypoint.sh"]
