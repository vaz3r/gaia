# ---- Builder: compile the release binary ----
FROM rust:1-slim AS builder
WORKDIR /build

# jemalloc-sys needs a C toolchain (make/cc) in the build stage.
RUN --mount=type=cache,target=/var/cache/apt \
    apt-get update \
    && apt-get install -y --no-install-recommends build-essential \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies: copy manifests first (workspace + crawler + owned gaia-*).
# The cargo registry + target are BuildKit cache mounts so dependency
# compilation is reused across rebuilds instead of starting from scratch every
# time (was ~5 min for the dep graph; now only the changed crate rebuilds).
COPY Cargo.toml ./
COPY crawler/Cargo.toml crawler/
COPY crawler/crates/gaia-bencode/Cargo.toml crawler/crates/gaia-bencode/
COPY crawler/crates/gaia-core/Cargo.toml crawler/crates/gaia-core/
COPY crawler/crates/gaia-wire/Cargo.toml crawler/crates/gaia-wire/
COPY crawler/crates/gaia-dht/Cargo.toml crawler/crates/gaia-dht/
COPY tools/sqlite-to-pg/Cargo.toml tools/sqlite-to-pg/
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    mkdir -p crawler/src && \
    echo 'fn main() {}' > crawler/src/main.rs && \
    cargo build --release -p crawler 2>/dev/null || true

# Real sources.
COPY crawler/src crawler/src
COPY crawler/crates/gaia-bencode/src crawler/crates/gaia-bencode/src/
COPY crawler/crates/gaia-core/src crawler/crates/gaia-core/src/
COPY crawler/crates/gaia-wire/src crawler/crates/gaia-wire/src/
COPY crawler/crates/gaia-dht/src crawler/crates/gaia-dht/src/
COPY db/migrations db/migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    touch crawler/src/main.rs && \
    cargo build --release -p crawler && \
    # The target dir is a cache mount (ephemeral across RUN steps), so copy the
    # finished binary into a normal layer path the runtime stage can COPY.
    mkdir -p /out && cp target/release/crawler /out/crawler

# ---- Runtime: slim image with just the binary ----
# trixie (Debian 13) matches the rust:1-slim builder's glibc (>= 2.38),
# which the compiled binary links against.
FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl postgresql-client gosu \
    && useradd --create-home --uid 1000 crawler \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out/crawler /usr/local/bin/crawler
COPY crawler/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Data + state are mounted here.
VOLUME ["/data"]

# Entrypoint runs as root, fixes volume ownership, then drops to the
# non-root `crawler` user via gosu before exec'ing the crawler.
ENTRYPOINT ["docker-entrypoint.sh"]
