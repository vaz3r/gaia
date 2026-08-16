# Tasks: Bitmagnet Parity & Surpassing Plan

## 1. Remove Artificial Liveness Delay in Docker Compose
- [x] 1.1 Update `docker-compose.yml` to set `--min-seen 1` and `--min-sightings 1` so hashes are dispatched on first discovery.
- [x] 1.2 Update `docker-compose.yml` to set `--scale 10` matching Bitmagnet's default concurrency and channel capacities.
- [x] 1.3 Update `docker-compose.fleet.yml` to align `--min-seen 1` and `--min-sightings 1`.

## 2. Rebuild & Deploy Updated Crawler
- [x] 2.1 Build and deploy the updated crawler container via `docker compose up -d --build crawler`.
- [x] 2.2 Verify container logs for zero discriminator drops and high first-sighting throughput.

## 3. Real-World Performance Validation
- [x] 3.1 Monitor crawl statistics over 5–10 minutes: verify routing table growth >10,000 nodes and indexing conversion rate >1,000/hr.
