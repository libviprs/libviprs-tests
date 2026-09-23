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
ARG PDFIUM_RELEASE=pdfium-8054
ARG TARGETARCH
RUN case "${TARGETARCH}" in \
        amd64) PDFIUM_ARCH="linux-x64";   PDFIUM_SHA256="b42d1731f07fb73edea38cbd294afe9be4bdcf8e4ed8523de51cd5d12fc8d271" ;; \
        arm64) PDFIUM_ARCH="linux-arm64"; PDFIUM_SHA256="3e1fe6e6ea1a53b1f801a071be256240513c1cad17b8d27fd80330da8f8c3640" ;; \
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

# shellcheck is here because the pre-commit hook runs it and `ubuntu-latest`
# ships it, so CI has it and this image did not. Nothing noticed until the local
# mirror got far enough to run the lint half at all (libviprs-tests#205): before
# that the run stopped in the counterpart's unit tests, and
# `install_hooks_localci_selection` reported `shellcheck: command not found`
# from inside a hook whose other commands were all stubbed to succeed.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates shellcheck \
 && rm -rf /var/lib/apt/lists/*

# rustfmt and clippy are here for the same reason as shellcheck above, and they
# are the sharper version of it. `rust:latest` used to ship both in its default
# profile, so nothing in this file ever had to ask, and the omission was not a
# decision anybody made. It stopped shipping them: 1.98.1 installs exactly
# `cargo`, `rust-std` and `rustc`.
#
# CI never saw it, because libviprs' ci.yml uses `dtolnay/rust-toolchain@stable`
# and that action installs both. So the local mirror and CI quietly disagreed
# about what a toolchain contains, which is the one thing this image exists not
# to do. What it looked like from the outside was `install_hooks_pdfium_scope`
# failing 6 of 7 cells on every tree, ours included, with `'cargo-fmt' is not
# installed for the toolchain`, because the hook that test drives runs
# `cargo fmt -- --check`. A gate that cannot pass for any input is not a gate.
#
# Pinning the tag instead would hide the next drift rather than catch it. The
# components are named here so the image says what it needs, and
# tests/image_has_the_toolchain_the_hooks_run.rs holds it to that.
RUN rustup component add rustfmt clippy

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

# The pdfium test suites run multi-threaded. Upstream pdfium-render 0.9.4's
# `thread_safe` feature, which the core requests in its own `Cargo.toml`, makes
# `ThreadSafePdfiumBindings` take the pdfium global mutex per call, so
# concurrent FPDF access across cargo-test worker threads is safe. There is no
# fork and no `[patch.crates-io]` in the graph any more; the per-call lock is
# upstream's. Running the default thread pool exercises that cross-test
# concurrency instead of hiding it behind `--test-threads=1`.
# The wall-clock perf-ratio smoke is `#[ignore]`d here and runs in the nightly
# workflow, so it never gates this container run.
#
# The integration step compiles and runs in two calls with a `sync` between
# them, rather than in the one `cargo test` that does both. Building ~130 test
# binaries leaves gigabytes of dirty page cache in the cgroup, and dirty pages
# cannot be reclaimed until they have been written back, so the first
# memory-hungry test binary to start could push the group over its ceiling
# faster than reclaim could answer. The kernel then killed it, and it came back
# as a SIGKILL against whichever test was running (libviprs/libviprs#683).
# Flushing first costs a second and makes that cache cheap to give up.
#
# The final step runs the green ported_* cells (feature = "ported_tests") via
# tools/run_ported_cells.sh, the single source of truth for the green-cell
# list (deferred remainder: issue #77). The cells read the pinned libvips
# reference suite under tmp/, which .dockerignore excludes from the build
# context; run-tests.sh fetches it on the host and bind-mounts it read-only
# into /src/libviprs-tests/tmp/. When the mount is absent (offline host with
# no cached copy) the script skips that step with a clear message instead of
# failing, matching how the repo treats optional fixtures.
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
    cd /src/libviprs-tests && cargo test --features pdfium --no-run && sync && \
    cargo test --features pdfium && \
    echo "" && \
    echo "================================================================" && \
    echo "Running libviprs-tests green ported cells (ported_tests)..." && \
    echo "================================================================" && \
    ./tools/run_ported_cells.sh'
