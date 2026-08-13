# syntax=docker/dockerfile:1

# ---- Builder: compile the release binary ----
FROM rust:1-slim AS builder
WORKDIR /build

# jemalloc-sys needs a C toolchain (make/cc) in the build stage.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies: copy manifests first (workspace + crawler + owned gaia-*).
COPY Cargo.toml ./
COPY crawler/Cargo.toml crawler/
COPY crawler/crates/gaia-bencode/Cargo.toml crawler/crates/gaia-bencode/
COPY crawler/crates/gaia-core/Cargo.toml crawler/crates/gaia-core/
COPY crawler/crates/gaia-wire/Cargo.toml crawler/crates/gaia-wire/
COPY crawler/crates/gaia-dht/Cargo.toml crawler/crates/gaia-dht/
RUN mkdir -p crawler/src && \
    echo 'fn main() {}' > crawler/src/main.rs && \
    cargo build --release -p crawler 2>/dev/null || true

# Real sources.
COPY crawler/src crawler/src
COPY crawler/crates/gaia-bencode/src crawler/crates/gaia-bencode/src/
COPY crawler/crates/gaia-core/src crawler/crates/gaia-core/src/
COPY crawler/crates/gaia-wire/src crawler/crates/gaia-wire/src/
COPY crawler/crates/gaia-dht/src crawler/crates/gaia-dht/src/
RUN touch crawler/src/main.rs && \
    cargo build --release -p crawler

# ---- Runtime: slim image with just the binary ----
# trixie (Debian 13) matches the rust:1-slim builder's glibc (>= 2.38),
# which the compiled binary links against.
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/crawler /usr/local/bin/crawler

# Data + state are mounted here.
VOLUME ["/data"]

ENTRYPOINT ["crawler"]
