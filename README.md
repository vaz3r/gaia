# gaia

BitTorrent DHT crawler indexing movie/TV torrents into SQLite, deployed behind
a WireGuard tunnel.

- **Crawler**: [`crawler/README.md`](crawler/README.md) — build, run, CLI, flags
- **Architecture**: [`ARCHITECTURE.md`](ARCHITECTURE.md) — source-traced system overview
- **Validation**: [`VALIDATION.md`](VALIDATION.md) — benchmark findings F1–F15 + closing decision
- **Privacy/visibility**: [`docs/PRIVACY.md`](docs/PRIVACY.md)
- **Benchmarks**: [`benchmark/`](benchmark/) — `bench.sh` (windowed report), `liveness.sh` (dashboard), `experiments/` (archived one-offs)

## Quick start

```sh
cd crawler
cp .env.example .env   # fill in WireGuard keys
docker compose up -d --build
```
