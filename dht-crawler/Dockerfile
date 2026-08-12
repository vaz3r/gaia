# syntax=docker/dockerfile:1

# ---- Builder: compile the release binary ----
FROM rust:1-slim AS builder
WORKDIR /build

# Cache dependencies: copy manifests first (workspace + crawler + owned gaia-*).
COPY Cargo.toml ./
COPY dht-crawler/Cargo.toml dht-crawler/
COPY vendor/gaia-bencode/Cargo.toml vendor/gaia-bencode/
COPY vendor/gaia-core/Cargo.toml vendor/gaia-core/
COPY vendor/gaia-wire/Cargo.toml vendor/gaia-wire/
COPY vendor/gaia-dht/Cargo.toml vendor/gaia-dht/
RUN mkdir -p dht-crawler/src && \
    echo 'fn main() {}' > dht-crawler/src/main.rs && \
    cargo build --release -p dht-crawler 2>/dev/null || true

# Real sources.
COPY dht-crawler/src dht-crawler/src
COPY vendor/gaia-bencode/src vendor/gaia-bencode/src/
COPY vendor/gaia-core/src vendor/gaia-core/src/
COPY vendor/gaia-wire/src vendor/gaia-wire/src/
COPY vendor/gaia-dht/src vendor/gaia-dht/src/
RUN touch dht-crawler/src/main.rs && \
    cargo build --release -p dht-crawler

# ---- Runtime: slim image with just the binary ----
# trixie (Debian 13) matches the rust:1-slim builder's glibc (>= 2.38),
# which the compiled binary links against.
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/dht-crawler /usr/local/bin/dht-crawler

# Data + state are mounted here.
VOLUME ["/data"]

ENTRYPOINT ["dht-crawler"]
