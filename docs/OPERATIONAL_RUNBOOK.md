# Gaia Edge Gateway & Fleet Operational Runbook

## 1. Fleet Trust Architecture: Private CA vs Public Browser Trust

### Internal Fleet Trust (Private CA)
The edge gateway (`gaia-gateway` on CT 116) and internal services operate under a two-tier internal Public Key Infrastructure (PKI):
- **Internal Root CA**: `/etc/ssl/gaia-ca/ca.crt` (`CN=Gaia Internal Root CA`, 10-year validity).
- **Leaf Certificate**: `/etc/ssl/certs/gateway.crt` signed by the Root CA with Subject Alternative Names (SANs):
  `DNS:gaia-gateway, DNS:gateway.gaia.local, DNS:localhost, IP:192.168.10.111, IP:127.0.0.1`.
- **System Trust Stores**: The Root CA certificate is installed into `/usr/local/share/ca-certificates/gaia-internal-ca.crt` and registered via `update-ca-certificates` across internal fleet containers (CT 116 gateway and CT 110 workspace).

**Internal Service Validation Policy**:
- All internal microservices, proxies, and tunnel components (`gaia-wstunnel-client`, `gaia-gateway-nginx`, automated health checks, and CLI tooling) MUST perform full TLS certificate and hostname validation.
- Flags disabling verification (`-k`, `--insecure`, or `InsecureSkipVerify`) are **strictly prohibited** in production configurations and health checks.
- Internal requests (such as `curl --cacert /etc/ssl/certs/gaia-internal-ca.crt https://127.0.0.1/api/stats` or `curl https://192.168.10.111/api/stats`) succeed with complete WebPKI validation.

### Public Browser Trust (Operator Access)
When an operator accesses the dashboard or gateway ingress via a standard web browser (Chrome, Firefox, Safari) from an external workstation (e.g., `https://192.168.10.111` or `https://gateway.gaia.local`):
- **Expected Behavior**: The browser displays an untrusted certificate warning (`NET::ERR_CERT_AUTHORITY_INVALID` / `SEC_ERROR_UNKNOWN_ISSUER`).
- **Cause**: The browser's trust store does not contain the private `Gaia Internal Root CA`.
- **Resolution Options**:
  1. **Operator Workstation Trust**: Export `/etc/ssl/gaia-ca/ca.crt` and install it into the operating system's root keychain / certificate trust store (macOS Keychain, Windows Trust Store, or Linux `/etc/pki/ca-trust`).
  2. **External Domain Public CA**: If the gateway is exposed via a public FQDN, provision a publicly trusted certificate (e.g., Let's Encrypt / ACME via DNS-01 challenge) and configure Nginx accordingly.
- **Critical Operator Note**: A browser warning on an operator workstation does NOT indicate an internal tunnel failure or broken gateway health check. Internal components validate against the private CA trust anchor.

---

## 2. Content Suppression Safety Policy & TTL Invariant

### 14,400s (4-Hour) TTL Rationale
Gaia operates on a **120-minute (2-hour)** stateless index rebuild cadence. Suppression keys in Redis DB 0 use a **14,400-second (4-hour)** TTL:

$$ \text{Suppression TTL} = 2 \times \text{Cadence} = 2 \times 120\text{ min} = 240\text{ min} = 14,400\text{ sec} $$

> **Bounded Guarantee Policy**:  
> **Redis provides a 14,400-second immediate masking window intended to tolerate one missed or delayed 120-minute rebuild interval. It is not a durable authorization control for an extended multi-cycle rebuild outage. If the best-effort Meilisearch deletion fails and no successful replacement rebuild completes before the TTL expires, an old Meilisearch document can temporarily reappear until operator intervention or the next successful rebuild.**

### Defense-in-Depth Suppression Layers
1. **PostgreSQL (Permanent Source of Truth)**:
   - When a moderation action is executed (`normAction == "SUPPRESS"`), PostgreSQL immediately updates the record:
     ```sql
     UPDATE torrents SET policy_action = 'SUPPRESS', ... WHERE infohash = @ih;
     ```
   - All extraction queries in `Gaia.Sync` stream exclusively where:
     ```sql
     WHERE t.policy_action IS DISTINCT FROM 'SUPPRESS'
     ```
   - All PostgreSQL fallback search queries in `PostgresTrigramSearchProvider` permanently enforce:
     ```sql
     WHERE policy_action IS DISTINCT FROM 'SUPPRESS'
     ```
2. **Meilisearch Live Deletion (Best-Effort Immediate Purge)**:
   - The API immediately fires an asynchronous deletion request (`DELETE /indexes/torrents/documents/{ih}`) against the live serving index.
3. **Redis Real-Time Filter (DB 0: `gaia:suppressed:{ih}`)**:
   - Key is set with explicit 4-hour expiration:
     ```csharp
     await db0.StringSetAsync($"gaia:suppressed:{ih}", "1", TimeSpan.FromHours(4));
     await db0.StringIncrementAsync("gaia:search:gen");
     ```
   - Every search query hits Redis DB 0 to verify candidate hits against active suppressions in $< 1\text{ms}$.
4. **Failure Boundary & Rebuild Outage Scenarios**:
   - If a single rebuild at $+120$ minutes fails or is delayed, the 14,400-second Redis suppression key remains active to mask the document throughout the failure window.
   - However, if an extended outage occurs (e.g., rebuild at $+120$m fails and rebuild at $+240$m is delayed or takes 35+ minutes), the Redis key can expire before the atomic swap at minute ~275. If the initial best-effort Meilisearch deletion also failed, the document could reappear until the replacement rebuild completes.
   - When the next rebuild cycle completes, it reads from PostgreSQL where `policy_action = 'SUPPRESS'` is permanent, constructing a shadow index that contains zero occurrences of the suppressed item, and atomically swaps it in.
   - If un-suppressed, `KeyDeleteAsync("gaia:suppressed:{ih}")` instantly restores visibility alongside a cache generation bump (`INCR gaia:search:gen`).

---

## 3. Health Checks & Verification Architecture

### Container Health Probes

| Container | Host | Check Mechanism | Safety Guarantee |
| :--- | :--- | :--- | :--- |
| `gaia-gateway-nginx` | CT 116 | `curl --cacert /etc/ssl/certs/gaia-internal-ca.crt -s -f https://127.0.0.1/api/stats` | Enforces valid TLS against SAN `127.0.0.1` and internal CA. Zero `-k` usage. |
| `gaia-gateway-wstunnel` | CT 116 | `nc -z 127.0.0.1 8443` | Confirms TCP WebSocket listener is active on loopback. |
| `gaia-gateway-wireguard` | CT 116 | Validates interface `wg0`, IP `10.99.0.1`, and safe handshake age. | Fails safely if handshake is missing, non-numeric, or $> 180$s stale. |
| `gaia-wstunnel-client` | CT 110 | `pidof wstunnel >/dev/null 2>&1 && ss -uln \| grep -q '127.0.0.1:51820'` | Validates process liveness and active local UDP socket binding. |
| `gaia-wireguard-client` | CT 110 | Checks interface `wg0`, IP `10.99.0.2`, safe handshake age ($\le 180$s), and `ping -c 1 -W 2 10.99.0.1` | End-to-end transport validation across wstunnel bridge to gateway. |

### Handshake Timestamp Safety Edge-Case Handling
The WireGuard client health check script handles edge cases safely:
```sh
ip link show wg0 >/dev/null 2>&1 && \
ip addr show wg0 | grep -q 10.99.0.2 && \
hs=$(wg show wg0 latest-handshakes | awk '{print $2}') && \
case "$hs" in ''|*[!0-9]*) exit 1;; esac && \
[ "$hs" -gt 0 ] && \
now=$(date +%s) && \
[ $((now - hs)) -ge 0 ] && [ $((now - hs)) -le 180 ] && \
ping -c 1 -W 2 10.99.0.1 >/dev/null 2>&1 || exit 1
```
- **Empty Handshake (`""`)**: Caught by pattern match and exits 1 immediately.
- **Zero Handshake (`0`)**: Fails `[ "$hs" -gt 0 ]` and exits 1 immediately.
- **Stale Handshake ($> 180$s)**: Fails `[ $((now - hs)) -le 180 ]` and exits 1.
- **Negative Age (Clock Skew)**: Fails `[ $((now - hs)) -ge 0 ]` and exits 1.

---

## 4. Operational Runbook: Rollback & Disaster Recovery

### Emergency Systemd Rollback
If Docker Compose or container runtimes encounter unexpected host degradation:
```bash
# From workspace or deployment runner:
./deploy/scripts/gateway-rollback.sh to-systemd
```
This command:
1. Stops and removes Docker Compose stacks on CT 116 and CT 110.
2. Restores host network interfaces and starts native systemd services (`nginx`, `wstunnel-server`, `wstunnel-client`, `wg-quick@wg0`).
3. Verifies tunnel ping (`10.99.0.1`) and ingress HTTP 200 response.

### Restoring Containerized Gateway
```bash
./deploy/scripts/gateway-rollback.sh to-docker
```
This command stops host systemd units and deploys declarative Docker Compose stacks on CT 116 and CT 110.

### Status Verification
```bash
./deploy/scripts/gateway-rollback.sh status
```
Displays systemd and Docker container status across both nodes simultaneously.

---

## 5. Reliability Backlog Item: Durable Suppression Enforcement for Meilisearch Serving During Extended Rebuild Outages

### Problem Statement
Redis provides a 14,400-second immediate masking window intended to tolerate one missed or delayed 120-minute rebuild interval. It is not a durable authorization control for an extended multi-cycle rebuild outage. If the best-effort Meilisearch deletion fails and no successful replacement rebuild completes before the TTL expires, an old Meilisearch document can temporarily reappear until operator intervention or the next successful rebuild.

### Architectural Options for Evaluation
1. **Batched Durable-Policy Verification**:
   - Query PostgreSQL for the candidate infohashes returned by Meilisearch within the search provider pipeline.
   - Must be strictly batched and bounded (e.g., `WHERE infohash = ANY(@hashes) AND policy_action = 'SUPPRESS'`) to avoid N+1 queries.
2. **Durable Suppression Materialization / Read Model**:
   - Maintain a local SQLite/RocksDB or persistent Redis AOF set of all active suppressed hashes replicated independently of ephemeral cache keys.
3. **Longer-Lived Suppression Record with Reconciliation & Alerts**:
   - Decouple suppression keys from short TTLs, paired with a dedicated health probe that alerts on rebuild lags exceeding 120 minutes.

