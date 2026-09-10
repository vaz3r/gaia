# Gaia Administrator Operational Routine & Monitoring Playbook

This playbook defines standard daily and weekly operating procedures to keep the Gaia crawler, PostgreSQL database, and machine learning pipelines running smoothly.

---

## 1. Daily Routine (~5 Minutes)

### 1. Ingestion Throughput & Health
- **Where**: **Overview Tab** -> Top Verdict Card & Ingestion Funnel
- **What to look for**:
  - Net-new unique torrents cataloged: **~5k–10k/hr** (`+X new/hr`).
  - Total verified rate: **~20k–30k/hr** (`+X refreshed/hr`).
  - Verify backlog queue: Stable (typically 1k–10k).
  - Status indicator: **Active** with live latency (< 50ms).
- **Abnormalities**: If verified rate drops to 0 or stays below 1k/hr for over 15 minutes, check `docker logs --tail 100 gaia-crawler`.

### 2. Operational Incidents Radar
- **Where**: Header Status Capsule (`X incidents` or `✓ All systems normal`)
- **What to look for**:
  - `0 active incidents`.
- **Abnormalities**: If yellow (`WARNING`) or red (`CRITICAL`) badges appear:
  - Click the badge to jump directly to **More -> Diagnostics**.
  - Inspect the incident type (`CASCADING_DROP`, `DHT_DROP_COLLAPSE`, `SOCKET_SATURATION`, `SLOW_SQL_SPIKE`, `RESTART_EVENT`).
  - Read the automated guidance and deviating metrics.
  - Address any underlying network or host issue, then click **"Resolve Alert"**.

### 3. Trust & Adjudication Spot-Check
- **Where**: **Classifier Studio Tab** -> Sub-tab: **Trust & Integrity**
- **What to look for**:
  - Review queue depth counter (`In Review`).
  - Spot-check 3–5 items at the top of the queue.
  - You **DO NOT** have to manually review thousands of items! (See Section 3 below on Automated vs Manual Review).

---

## 2. Weekly Routine (~15–20 Minutes)

### 1. Dead Peer Suppression & Socket Pool Health
- **Where**: **More -> Diagnostics** tab
- **What to look for**:
  - **Socket Permits In-Flight**: Should fluctuate between **20% – 60%** of capacity (e.g. ~1,500 / 4,000 max sockets). If constantly pinned at 100%, check if firewall or network connection is dropping outbound SYN packets.
  - **Dead Peer Suppression Cache**: Should show **Active**, holding ~70k–120k quarantined dead peers with an eviction velocity of **~200k–300k/hr**. This prevents socket starvation.
  - **Channel Backpressures**: Verify and Fresh channel buffers should read **"No Backpressure"** or **"Flowing"** (not maxed out).

### 2. Category Intelligence & Open-Set Drift
- **Where**: **Content Intelligence Tab**
- **What to look for**:
  - Healthy category distribution across Movies, Television, Music, Games, Books, Applications, Anime, etc.
  - Check that **Unclassified** content remains low (< 2% of total verified torrents).

### 3. Active Learning & Model Retraining
- **Where**: **Classifier Studio Tab** -> Sub-tab: **Category Model**
- **What to look for**:
  - Label 10–20 ambiguous items from the Review Queue using the 1-click category buttons (`1-9`, `0`).
  - Click **"Model Management"** modal:
    - If new labels have accumulated (e.g. >100 new labels), click **"Trigger Retrain"**.
    - The retrain pipeline will train a new candidate model, compute 10-fold cross-validation, and automatically promote it if Macro-F1 exceeds the baseline (>0.90).

### 4. Database & Remote Backup Verification
- **Where**: Terminal / Host
- **Commands**:
  ```bash
  # Check PostgreSQL disk storage
  df -h /home/core/gaia-data/postgres
  
  # Check nightly backup status
  docker logs --tail 30 gaia-backup
  ```
- Verify backup completed its nightly upload to Google Drive (`gdrive:/`).

---

## 3. Do I Have to Manually Review All 2,800+ Items?

**No, absolutely not!**

### How the Policy Engine Works:
- **`REVIEW` is an automated safety tier**: The machine learning models assign `REVIEW` to torrents when:
  1. The confidence score is borderline (e.g. 50%–65%), or
  2. The risk tier is ambiguous (e.g. borderline health or unconfirmed seed).
- **Graceful Degradation**: Items in `REVIEW` are safely **downranked** or quarantined from top public feeds automatically. The system continues operating normally without operator intervention.
- **Manual Overrides are for Operator Intent**:
  - You only need to review items that you specifically care about or spot-check high-popularity titles.
  - When you click **ALLOW**, **DOWNRANK**, or **SUPPRESS**, the decision source becomes `MANUAL`. This permanently locks the policy action and prevents future automated worker runs from overriding your intent.
- **Batch Reclassification**:
  - If a model version is upgraded or thresholds are tuned, you can click **"Run Auto-Reclassify"** in Classifier Studio.
  - The background reclassification daemon (`gaia-classifier-api`) will re-evaluate thousands of pending items in bulk without requiring manual clicks.

---

## 4. Troubleshooting Quick Reference

| Symptom | Cause | Resolution |
| :--- | :--- | :--- |
| **`DHT_DROP_COLLAPSE` Alert** | Temporary UDP packet loss or bootstrap timeout | Check host firewall (`ufw`), ensure UDP port 6881 is open, or verify internet connectivity. Usually auto-recovers. |
| **`RESTART_EVENT` Alert** | Daemon restarted or container recreate | Normal after deployments or updates. If unplanned, check `docker inspect gaia-crawler` for OOM exit code 137. |
| **Pending Review APIs Hanging** | Missing index on `torrents` table | Verified indexed via `idx_torrents_scoring_review` and `idx_torrents_review_queue`. Response should be < 50ms. |
| **Socket Permits Maxed (100%)** | ISP or firewall rate limiting TCP/uTP connections | Check host socket limits (`sysctl net.ipv4.tcp_max_syn_backlog`) and inspect Dead Peer Cache evictions. |
