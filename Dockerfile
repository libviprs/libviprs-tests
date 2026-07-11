# ---------------------------------------------------------------------------
# Dockerfile — run libviprs + libviprs-tests with PDFium (amd64 + arm64)
# ---------------------------------------------------------------------------

# Stage 1: Download PDFium shared library for the target architecture
FROM debian:bookworm-slim AS pdfium

RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/*

# PDFium provenance: consume the pinned, checksum-verified binaries published by
# libviprs-dep (the branch-pinned builder that runs real ABI/symbol
# verification) rather than the floating upstream `releases/latest`. This is the
# single provenance source shared with CI. Keep PDFIUM_RELEASE and the per-arch
# SHA-256 digests in lockstep with the release consumed by
# .github/workflows/ci.yml.
ARG PDFIUM_RELEASE=pdfium-7881
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
        amd64) PDFIUM_ARCH="linux-x64";   PDFIUM_SHA256="653f24f074afe6c868f634ae0cc954a1a89821f33bc7795f16065a14022b662b" ;; \
        arm64) PDFIUM_ARCH="linux-arm64"; PDFIUM_SHA256="3a8940ae414a54601f6bc0b25fb3d589025320ee91fff378e12708259da5702d" ;; \
        *)     echo "Unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac && \
    curl -fsSL -o /tmp/pdfium.tgz \
        "https://github.com/libviprs/libviprs-dep/releases/download/${PDFIUM_RELEASE}/pdfium-${PDFIUM_ARCH}.tgz" && \
    echo "${PDFIUM_SHA256}  /tmp/pdfium.tgz" | sha256sum -c - && \
    mkdir -p /opt/pdfium && \
    tar xzf /tmp/pdfium.tgz -C /opt/pdfium --strip-components=1 && \
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

# Disable debug info to keep test binaries small enough for the container.
# Each integration test is a separate binary; full debuginfo exhausts disk space.
ENV CARGO_PROFILE_DEV_DEBUG=0

# The pdfium test suites run multi-threaded. The pdfium-render fork on the
# `libviprs/integration` branch (a direct dep of libviprs, mirrored into
# libviprs-tests via `[patch.crates-io]`) rewrites `ThreadSafePdfiumBindings`
# to take the pdfium global mutex per call, so concurrent FPDF access across
# cargo-test worker threads is safe. Running the default thread pool exercises
# that cross-test concurrency instead of hiding it behind `--test-threads=1`.
# The wall-clock perf-ratio smoke is `#[ignore]`d here and runs in the nightly
# workflow, so it never gates this container run.
CMD sh -c '\
    echo "================================================================" && \
    echo "Running libviprs unit tests (with pdfium)..." && \
    echo "================================================================" && \
    cd /src/libviprs && cargo test --features pdfium && \
    echo "" && \
    echo "Cleaning libviprs build artifacts to free disk space..." && \
    cargo clean && \
    echo "" && \
    echo "================================================================" && \
    echo "Running libviprs-tests integration tests (with pdfium)..." && \
    echo "================================================================" && \
    cd /src/libviprs-tests && cargo test --features pdfium'
