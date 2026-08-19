FROM rust:1.97-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --locked

FROM rust:1.97-slim
RUN useradd -m -u 10001 crawler && mkdir -p /home/crawler/data && chown -R crawler:crawler /home/crawler
COPY --from=builder /app/target/release/craw /usr/local/bin/craw
USER crawler
WORKDIR /home/crawler
ENV CRAW_DATA_DIR=/home/crawler/data
ENTRYPOINT ["craw"]
