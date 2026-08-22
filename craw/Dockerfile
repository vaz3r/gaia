FROM --platform=linux/arm64 rust:1.97-slim AS builder
WORKDIR /app
ARG CRAW_GIT_HASH=unknown
ENV CRAW_GIT_HASH=${CRAW_GIT_HASH}
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --release

FROM --platform=linux/arm64 debian:bookworm-slim
RUN useradd -m -u 10001 crawler && mkdir -p /home/crawler/data && chown -R crawler:crawler /home/crawler
COPY --from=builder /app/target/release/craw /usr/local/bin/craw
USER crawler
WORKDIR /home/crawler
ENV CRAW_DATA_DIR=/home/crawler/data
ENTRYPOINT ["craw"]
