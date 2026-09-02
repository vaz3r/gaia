# Crawler Optimization Postmortem (Sept 2026)

## Overview
The Gaia crawler's verified torrent throughput experienced a severe degradation, plummeting from an expected target of ~50,000 torrents/hour to approximately 7,000 torrents/hour. 

Through a deep architectural review and live production debugging, we uncovered a cascading series of bugs and configuration layers that were actively starving the crawler's pipeline. After applying multiple fixes, throughput was successfully restored to **sustained rates of 42,000–50,000 verified torrents per hour**.

This document archives the root causes and their respective fixes.

## 1. Fast-Lane Logic Bug (`fetch_pool.rs`)
* **The Bug:** When a stable "fast-lane" peer rejected a connection or timed out, the `verify_infohash` logic treated the `Reject` signal as a fatal `MetadataFailed` error. This instantly terminated the verification attempt for that infohash before it could try other peers.
* **The Fix:** Modified the logic to gracefully handle `Reject` signals by incrementing the `dht_meta_failures` counter and allowing the infohash to continue circulating through the pipeline (Commit `c4b2c8b`).

## 2. DB Scheduler Claim Limit 
* **The Bug:** The database retry scheduler was capped at `scheduler_claim_limit = 1000`. Running every 15 seconds, this artificially throttled the injection of retry jobs, leaving the verification workers starved for work.
* **The Fix:** Increased the limit to `4000`, allowing the scheduler to fully saturate the pipeline (Commit `233e46f`).

## 3. Docker-Compose Environment Stripping
* **The Bug:** Several critical tuning variables (e.g., `CRAW_SYBIL_BEP42_RATIO`, `CRAW_SOURCE_MAX_QUERIES`) were added to the `.env` file but were silently ignored by the crawler container. Docker Compose only injects environment variables that are explicitly declared in the `environment:` block of the `docker-compose.yml` file.
* **The Fix:** Explicitly mapped all `.env` tunables into the compose file's environment section (Commit `8a6f9b5`).

## 4. Hard-Overridden `production.toml`
* **The Bug:** Even after fixing the compose file, variables like `source_max_queries` and `sybil_bep42_ratio` lacked environment-override wiring in the `config.rs` code. Furthermore, the `apps/crawler/config/production.toml` file explicitly hardcoded `source_max_queries = 24`, completely nullifying our attempts to scale it up.
* **The Fix:** Hardcoded `source_max_queries = 48` and `sybil_bep42_ratio = 1.0` directly into `production.toml` (Commit `e7c1fa4`).

## 5. BEP-42 Sybil Identity Propagation
* **The Bug:** Due to the configuration bugs above, the crawler was running with `sybil_bep42_ratio = 0.333`. This meant 66% of our DHT crawler nodes were using non-compliant IDs, causing strict DHT nodes in the network to drop our traffic.
* **The Fix:** Successfully applying `sybil_bep42_ratio = 1.0` (100% compliance) caused the organic `inbound_get_peers` rate to skyrocket from ~2,500/min to over 5,400/min, driving massive amounts of fresh infohashes into the pipeline.

## Conclusion
The crawler is no longer pipeline-starved. The database queue naturally drains (`pending=0`), and throughput is sustained purely by organic DHT discovery and scheduled retries.
