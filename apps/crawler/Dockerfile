FROM rust:1.97-slim AS builder
WORKDIR /app
ARG CRAW_GIT_HASH=unknown
ENV CRAW_GIT_HASH=${CRAW_GIT_HASH}
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp /app/target/release/crawler /tmp/crawler

FROM debian:bookworm-slim
RUN useradd -m -u 10001 crawler && mkdir -p /home/crawler/data && chown -R crawler:crawler /home/crawler
COPY --from=builder /tmp/crawler /usr/local/bin/crawler
COPY --from=builder /app/config /etc/crawler/config
USER crawler
WORKDIR /home/crawler
ENV CRAW_DATA_DIR=/home/crawler/data
ENV CRAW_CONFIG_DIR=/etc/crawler/config
ENV CRAW_PROFILE=production
ENTRYPOINT ["crawler"]
