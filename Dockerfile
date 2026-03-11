FROM rust:1.85-bookworm

# Install PDFium shared library
RUN apt-get update && apt-get install -y curl && rm -rf /var/lib/apt/lists/* \
    && curl -L -o /tmp/pdfium.tgz https://github.com/niclasvaneyk/pdfium-linux-x64/releases/latest/download/pdfium-linux-x64.tgz \
    && tar xzf /tmp/pdfium.tgz -C /usr/local \
    && ldconfig \
    && rm /tmp/pdfium.tgz

WORKDIR /src

# Copy libviprs first (dependency)
COPY libviprs/ libviprs/

# Copy test crate
COPY libviprs-tests/ libviprs-tests/

WORKDIR /src/libviprs-tests

# Build dependencies first for layer caching
RUN cargo fetch

# Default: run all tests including pdfium
CMD ["cargo", "test", "--features", "pdfium"]
