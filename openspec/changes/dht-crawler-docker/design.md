## Context

Building on `dht-crawler-verify` (committed). Discovery and fetch are healthy; the NAT egress is the verify-rate ceiling. The user has a WireGuard server on Oracle Cloud. This change containers the crawler behind a Gluetun WireGuard client so all traffic egresses from the Oracle public IP.

## Goals / Non-Goals

**Goals:**
- Run the crawler in Docker behind Gluetun, with all UDP/TCP egressing from the Oracle public IP.
- Make the DHT node reachable inbound through the tunnel (open 6881-6884 in the Gluetun tunnel firewall).
- Keep secrets out of the repo; keep data (DB + routing state) on a mounted volume so restarts are warm.
- Replace the pm2 deployment while preserving run.sh/ecosystem.config.cjs as fallback.

**Non-Goals:**
- No changes to crawler discovery/fetch logic (already tuned).
- No changes to the Oracle WireGuard server (already configured with this client's peer).
- No multi-host orchestration.

## Decisions

### D24 — Containerize the crawler with a multi-stage Dockerfile
Build the release binary in a `rust:1-slim` builder (rusqlite's bundled SQLite compiles there), then copy just the binary + `ca-certificates` into a `debian:bookworm-slim` runtime. Entrypoint runs `dht-crawler run --db /data/crawler.sqlite --state-dir /data/state --port 6881 --instances 4`.
- *Rationale:* minimal image, no runtime C toolchain needed (rusqlite is bundled).

### D25 — Route through Gluetun via `network_mode: "service:gluetun"`
The `dht-crawler` service shares the Gluetun container's network namespace, so all its traffic egresses through the WireGuard tunnel to the Oracle public IP. Gluetun configures the tunnel from the user's peer details (`10.8.0.9/24`, server `132.145.189.201:443`).
- *Alternatives considered:* host-level WireGuard + `ip route` policy routing — rejected: more moving parts, root-level network surgery. Docker netns is the supported, self-contained path.
- *Trade-off:* all crawler traffic goes through the tunnel (full-tunnel `AllowedIPs 0.0.0.0/0`); acceptable since the crawler only talks to DHT peers and DB is local.

### D26 — Open DHT ports in the Gluetun tunnel firewall
Gluetun's firewall blocks inbound to the tunnel except ports in `FIREWALL_VPN_INPUT_PORTS`. Set it to `6881,6882,6883,6884` so the DHT node is reachable inbound (announce_peer / get_peers / sample_infohashes routed to us).
- *Rationale:* without this, the node is effectively one-way (outbound-only), defeating the public-IP benefit.

### D27 — Secrets in a gitignored `.env`; data on a volume
WireGuard keys are injected via `docker compose --env-file .env` (or compose's implicit `.env`), which is gitignored. The crawler mounts `./data:/data` so `crawler.sqlite` + `state/` persist across container restarts and are shared with the fallback pm2 config.
- *Rationale:* never commit private keys; keep warm routing state and crawl history.

## Risks / Trade-offs

- **Full-tunnel routing** routes all crawler traffic via Oracle → adds latency but improves peer reachability; expected net positive for verify rate.
- **Oracle 51820 block avoided** by using port 443 (already open).
- **pm2 vs Docker port overlap** — only one runs at a time; pm2 stopped before compose up.
- **Container rebuilds** require `docker compose up -d --build`; source changes need a rebuild (no hot reload).

## Migration Plan

1. Stop pm2 crawler.
2. Create `dht-crawler/data/` and copy `crawler.sqlite` + `state/` into it.
3. Create gitignored `dht-crawler/.env` with the WireGuard secrets.
4. `docker compose up -d --build`.
5. Verify egress IP via the container; confirm 4 instances and rising verify rate.
Rollback: `docker compose down`, then `pm2 start ecosystem.config.cjs` (same DB path).
