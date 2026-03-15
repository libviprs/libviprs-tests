# ---------------------------------------------------------------------------
# Dockerfile — run libviprs + libviprs-tests with PDFium (amd64 + arm64)
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

# Stage 2: Build and test
FROM rust:latest AS builder

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# Install PDFium shared library
COPY --from=pdfium /opt/pdfium/lib/libpdfium.so /usr/local/lib/libpdfium.so
RUN ldconfig

WORKDIR /src

# Copy both crates
COPY libviprs/ libviprs/
COPY libviprs-tests/ libviprs-tests/

# Fetch dependencies for both crates
WORKDIR /src/libviprs
RUN cargo fetch

WORKDIR /src/libviprs-tests
RUN cargo fetch

# Default: run libviprs tests first, then libviprs-tests with pdfium
CMD sh -c '\
    echo "================================================================" && \
    echo "Running libviprs unit tests (with pdfium)..." && \
    echo "================================================================" && \
    cd /src/libviprs && cargo test --features pdfium && \
    echo "" && \
    echo "================================================================" && \
    echo "Running libviprs-tests integration tests (with pdfium)..." && \
    echo "================================================================" && \
    cd /src/libviprs-tests && cargo test --features pdfium'
