# GAIA Dashboard — Complete Feature Reference Manual

This document provides a comprehensive breakdown of all features, telemetry gauges, interactive controls, and analytics available across every tab in the **GAIA BitTorrent & DHT Intelligence Dashboard**.

---

## Architecture & Navigation Overview

The GAIA Dashboard is organized into primary operational tabs, accessible directly from the top navigation bar and dropdown menu:

1. [Overview](#1-overview-tab)
2. [Explorer (Torrent Browser)](#2-explorer-torrent-browser)
3. [Classifier Studio](#3-classifier-studio)
4. [Analysis & Content Landscape](#4-analysis--content-landscape)
5. [DHT Routing Mesh & Stable Peers](#5-dht-routing-mesh--stable-peers-more)
6. [Diagnostics & System Telemetry](#6-diagnostics--telemetry-more)
7. [Global Torrent Inspector Drawer](#7-global-torrent-inspector-drawer)

---

## 1. Overview Tab

The **Overview** tab serves as the central command cockpit for the crawler cluster, displaying real-time ingest rates, historical throughput, transport statistics, and cluster health.

### 1.1 Header Telemetry & Status Badges
- **Cluster Node Identifier**: Displays the active deployment context (e.g., `GAIA / cluster-eu-01`).
- **Real-Time Heartbeat Indicator**: Pulsing green status pill confirming sub-second connection with cluster telemetry.
- **Total Cataloged Counter**: Displays the total count of verified infohashes stored in the PostgreSQL database.

### 1.2 Metric Cards Grid (Key Performance Indicators)
- **Verified Infohashes**: Total verified torrents cataloged with real-time increment counter.
- **Harvesting Velocity**:
  - Raw announce/infohash discovery rate per hour (e.g., `2.3M / hr`).
  - Metadata verification rate per hour (e.g., `35.2k / hr`).
- **Crawler Node Sockets**: Active in-flight UDP/TCP sockets out of max permit pool (e.g., `1,250 / 3,000`).
- **BEP 9/10 Ingestion Rate**: Successful metadata payloads downloaded and verified against cryptographic SHA-1 hashes per second/minute.
- **Average Swarm Health Index**: Aggregate health percentage across the entire tracked DHT network.

### 1.3 60-Minute Real-Time Telemetry Line Chart
- **Dual-Axis Time Series**:
  - **Discovered Ingest Curve (White/Gray)**: Tracking millions of raw DHT announcements per hour.
  - **Verified Payloads Curve (Emerald)**: Tracking verified `.torrent` metadata saved to disk/DB.
- **Scale Selector**: Toggle between `Linear` and `Logarithmic` scaling to inspect subtle micro-trends without losing high-magnitude spikes.
- **Interactive Scrubber Tooltip**: Hovering over any 1-minute bucket displays the exact local time (Asia/Dubai GST UTC+4), discovery rate, and verified rate.

### 1.4 24-Hour Verified Ingestion Velocity (Hourly Bar Chart)
- **24-Hour Trailing Ingest Bars**: 24 individual hourly bars showing exact volume of torrents verified per hour.
- **Dynamic Color Ranks**:
  - High Volume (`≥ 30,000 / hr`): Bright Emerald with glow.
  - Moderate Volume (`≥ 15,000 / hr`): Darker Emerald.
  - Standard Volume (`< 15,000 / hr`): Neutral dark gray.
- **Bar Hover Tooltips**: Displays specific hour timestamp (Dubai GST) and verified count.
- **Summary Metrics**: Last 24h total verified volume and trailing 1h volume.

### 1.5 Protocol & Cache Tri-Card Grid
- **Transport Protocols (TCP vs. uTP)**:
  - Percentage share and per-hour volume comparison between TCP (standard wire protocol) and BEP 29 uTP (micro transport protocol over UDP).
  - Stacked proportional progress bars.
- **Dead Peer Suppression Cache**:
  - Size of the in-memory LRU cache holding unreachable/refused peer endpoints.
  - Sizing estimator and memory utilization diagnostics (`keyspace vs. RAM`).
  - Active eviction rate per hour protecting file descriptors and kernel sockets.
- **Cluster Node Health**:
  - Daemon uptime counter (formatted in hours/minutes).
  - PostgreSQL transaction rate and query buffer depth.

---

## 2. Explorer (Torrent Browser)

The **Explorer** tab provides a high-performance, server-side paginated search engine and data grid for querying millions of verified torrents.

### 2.1 Search & Filter Controls
- **Full-Text & Exact Trigram Search**:
  - Search across torrent titles, release keywords, or 40-character hex infohashes.
  - Debounced input with instant auto-switch to relevance sorting on active queries.
  - One-click clear search button (`X`).
- **Category Filter Dropdown**:
  - Instant filtering by canonical content categories: `Adult`, `Anime`, `Applications`, `Audiobooks`, `Books & Learning`, `Documentaries`, `Games`, `Movies`, `Music`, `Television`.
- **Multi-Field Sorting**:
  - Sort by: `Newest Verified`, `Oldest Verified`, `Largest Size`, `Smallest Size`, `Most Files`, `Most Active Swarms`, `Name (A-Z)`, or `Relevance`.
  - Ascending/Descending toggle buttons on column headers.
- **Page Size Limits**:
  - Switch between `25`, `50`, or `100` torrents per page.

### 2.2 Results Data Grid
- **Payload Description**:
  - Primary release title with fallback to `payload-<infohash>` if untitled.
  - Sub-label indicating whether the release is a single file or a multi-file bundle.
- **Infohash (Hex)**: Monospace truncated SHA-1 cryptographic hash (`0123456789...abcdef01`).
- **Category Pill**:
  - Color-coded category badge with the machine learning model's confidence percentage (e.g., `Movies 97%`).
- **Payload Size**: Clean byte formatting (`KB`, `MB`, `GB`, `TB`).
- **File Count**: Total file count contained within the verified `.torrent` payload dictionary.
- **Swarm Health Bar**:
  - Tri-color progress meter based on real-time peer scrape:
    - High Health (`≥ 70%`): Green.
    - Moderate Health (`40% - 69%`): Amber.
    - Low/Stale (`< 40%`): Rose.
- **Popularity Score**: Relative ranking based on cumulative DHT sightings and peer activity.
- **Discovered Timestamp**: Time elapsed since first sight or verification (Asia/Dubai GST).
- **Row Actions**:
  - **Copy Magnet URI**: Instantly copies a sanitized `magnet:?xt=urn:btih:...` link with standard public trackers.
  - **Inspect Details**: Opens the full-screen [Global Torrent Inspector Drawer](#7-global-torrent-inspector-drawer).

### 2.3 Pagination Navigation Bar
- Current page / total pages indicator.
- Fast navigation buttons: `First`, `Previous`, `Next`, `Last`.
- Numeric direct page jump input box.

---

## 3. Classifier Studio

The **Classifier Studio** provides complete active learning, continuous retraining, explainability diagnostics, and dataset management for the GAIA machine learning categorization engine.

### 3.1 Real-Time ML Telemetry Banner
- **Classifier Daemon Status**: Live heartbeat indicating Python API and worker execution state.
- **Classification Ingestion Rate**: Number of torrents classified per minute by the background daemon.
- **Ground Truth Count**: Total human-verified labels stored in PostgreSQL `labeled_results`.
- **Review Queue Depth**: Torrents requiring human verification due to low confidence or close score margins.
- **System Model Version Badge**: Displays active model artifact (`v2`, `v3`, etc.) with macro-F1.

### 3.2 Dual Queue Workspace Tabs
- **Needs Review Queue**:
  - Displays torrents where `needs_review = true` (confidence `< 0.65` or margin `< 0.35`).
  - Prioritizes ambiguous edge cases to maximize active learning feedback.
- **All Classified Catalog**:
  - Explores the entire catalog of already classified torrents.
  - Filterable by individual category (`Adult`, `Games`, `Television`, etc.) to audit model behavior.

### 3.3 Active Learning Labeling Deck (Human-in-the-Loop)
- **Top Prediction Quick-Confirm**: One-click confirmation button to accept the model's predicted category with highest probability.
- **Category Override Selector**: Dropdown to select any of the 10 canonical categories or `Other`.
- **Apply Correction Button**:
  - Atomically writes the human correction to PostgreSQL `labeled_results` with confidence `high` and source `manual_web`.
  - Sets `needs_review = false` on the torrent.
  - Updates the active model's future retraining dataset.
- **Mark as Other**: Dedicated shortcut to classify non-canonical content (e.g. disk images, raw dumps).

### 3.4 Feature Importance & Probability Explainability
- **Class Probability Spectrum**:
  - Visual distribution bars displaying probabilities across all 10 classes for the selected torrent.
  - Green indicator on the winning category with percentage score.
- **Top Extracted Features**:
  - Real-time display of tokenized text n-grams, extension tokens, and metadata signals that triggered the decision (e.g., `ext:.mkv`, `token:1080p`, `token:s01e04`).

### 3.5 Model Management & Retraining Pipeline Modal
Accessed via the **"Model Management"** button in the header.

- **Tab 1: Active Diagnostics**:
  - **Model Header**: Active version, filename, activation date (Dubai GST), Macro-F1, and validation accuracy.
  - **Continuous Quality Gate Cards**: Outlines the 3 automated validation gates:
    1. *Macro F1 Guard*: Rejects candidate if F1 regresses by > 0.5%.
    2. *Class Collapse Prevention*: Rejects if any single class F1 drops by > 3.0%.
    3. *Shadow Traffic Canary*: Rejects if production traffic slice shifts category distribution by > 25%.
  - **Per-Class F1 Breakdown**: Progress meters showing trained performance for each of the 10 canonical classes.
- **Tab 2: Version Artifacts & Atomic Rollback**:
  - Complete historical list of all `.joblib` model binary files stored in the cluster.
  - Displays file size, sample training size (e.g., 30,869 samples), accuracy, macro-F1, and trained date.
  - **One-Click Rollback**: Atomically updates `active_model.json`, restores historical target metrics, and hot-reloads the Python daemon with zero downtime.
- **Tab 3: Pipeline Terminal**:
  - Real-time streaming log drawer showing stdout/stderr from background `retrain.py` execution with exit codes and step indicators.
- **Trigger Retraining**: Background trigger that pulls PostgreSQL ground truth, trains a candidate SGD/Huber model, tests quality gates, and promotes if criteria pass.

---

## 4. Analysis & Content Landscape

The **Analysis** tab provides high-level content intelligence, catalog distribution matrices, storage footprint analytics, and category-filtered swarm velocity discovery.

### 4.1 Telemetry Header Cards
- **Active Swarms (24h)**: Torrents with active peer sightings in the past 24 hours.
- **New Releases (48h)**: Fresh DHT swarms first discovered within the last 48 hours.
- **High Activity Swarms**: Torrents sighted ≥ 10 times across crawler walks.
- **ML Classified Metric**: Total count of categorized torrents and percentage share of the 2.4M catalog.
- **Total Storage Footprint**: Aggregate payload size in Terabytes/Petabytes across verified content.
- **Total Tracked Swarms**: Total torrents indexed in the PostgreSQL database.

### 4.2 Content Category Landscape & Distribution Matrix
- **Interactive Proportional Distribution Bar**:
  - Stacked color-coded bar representing relative percentage share of each category across the catalog.
  - Hovering reveals volume count and percentage; clicking any segment filters the discovery table below.
- **Category Intelligence Cards Grid**:
  - 10 structured cards (one for each category) detailing:
    - **Percentage Share**: Relative portion of the catalog (e.g., `Adult 33.6%`, `Television 18.3%`, `Movies 16.3%`).
    - **Total Volume**: Number of categorized torrents.
    - **Total Storage (TB)**: Sum of all payload data in Terabytes.
    - **Average Release Size**: Median size per release (e.g., `Games 25.0 GB`, `TV 9.9 GB`, `Audiobooks 0.7 GB`).
    - **Avg Peers & Health**: Average active swarm peers and health score percentage.

### 4.3 Category-Filtered Swarm Discovery Table
- **Sub-Tab Navigation**:
  - **Trending Swarms**: Torrents scored by combining swarm peer activity and recent velocity.
  - **Rising Velocity (<48h)**: Fresh releases spreading fastest across DHT nodes.
  - **Top Swarms (All-Time)**: Cumulative all-time sightings since cluster inception.
- **Category Filter Pills**:
  - Interactive pills (`All Categories`, `Adult`, `Television`, `Movies`, `Games`, etc.) enabling rapid drill-down into specific content niches.
- **Discovery Grid Columns**:
  - Rank `#`, Release Name, Category Badge with Confidence %, Payload Size, Total Sightings, Velocity / Trend Score, Swarm Health Badge, Discovered Time, and Quick Action buttons (Copy Magnet, Inspect).

---

## 5. DHT Routing Mesh & Stable Peers (More)

Located under the **"More" → "DHT Routing"** dropdown menu, this tab provides diagnostics on Kademlia 160-bit XOR routing tables and the stable peer caching engine.

### 5.1 Kademlia Routing Mesh Status
- **Active Routing Buckets**: Total active buckets currently in use.
- **Good Nodes Count**: Verified healthy DHT node endpoints maintained in memory.
- **Table Density**: Percentage of routing table fill rate (target: > 80%).

### 5.2 Routing Table Bucket Heatmap (k=8 Node Fill Rate)
- **Bucket Matrix Grid**: Visualizes 32 sample bucket ranges across the 160-bit keyspace.
- **Bucket Fill States**:
  - `Full (8/8)`: Solid white card indicating saturated bucket.
  - `Partial (<8/8)`: Dark gray card indicating filling bucket.
  - `Stale / Pinging`: Highlighted status for nodes undergoing active BEP 5 ping checks.

### 5.3 Inbound RPC Query Velocity
- Real-time rate meters and percentage share for primary DHT RPC methods:
  - `get_peers`: Inbound torrent queries searching for swarms.
  - `find_node`: Routing table walk requests.
  - `announce_peer`: Seed publishing announcements.

### 5.4 Configured Bootstrap Relays
- Live connectivity status to canonical BitTorrent bootstrap relays:
  - `router.bittorrent.com:6881` (Primary)
  - `router.utorrent.com:6881`
  - `dht.transmissionbt.com:6881`
  - `dht.libtorrent.org:25401`
  - `router.bitcomet.com:6881` (Fallback)

### 5.5 Stable Peers Explorer (Technique D Sighting Correlation)
- **Verified Endpoints Table**: Browse hundreds of thousands of stable peer endpoints discovered via BEP 9/10 metadata exchange.
- **Search by IP/Port**: Instant filtering of peers by IPv4 address or listening port.
- **Sorting**: Sort by `Metadata Provided Count`, `Last Seen`, `IP`, or `Port`.
- **Peer Seeded Torrents Modal**:
  - Clicking any peer opens a dedicated modal showing all torrents seeded by that specific peer.
  - Allows cross-referencing high-value seeders and copying magnet links.

---

## 6. Diagnostics & Telemetry (More)

Located under the **"More" → "Diagnostics"** dropdown menu, this view exposes internal crawler engine queues, PostgreSQL query performance, and live syslog output.

### 6.1 Channel Buffer & Permit Gauges
- **Verify Channel Buffer**: Real-time queue buffer depth for raw infohashes waiting for BEP 9 metadata download (alerting on backpressure).
- **Fresh Channel Buffer**: Ingestion queue for newly discovered swarms awaiting initial database insertion.
- **Socket Permits In-Flight**: Active concurrency gauge showing UDP/TCP socket permits allocated from the global pipeline pool.

### 6.2 Interactive Peer Cache Tuner
- **Live Memory Sizing Estimator**:
  - Interactive slider (`100k` to `1,000k` keys) showing projected RAM footprint.
  - Real-time statistics on active dead peer entries and eviction rates.

### 6.3 Discovery Source Yield & Efficiency
- Breakdown of metadata verification success rates by discovery channel:
  - **Direct Peer Sightings**: Highest yield (~27.6%).
  - **Announce Cache**: Moderate yield (~9.4%).
  - **DHT Routing Walks**: Broad high-volume discovery (~3.8%).

### 6.4 Slow SQL Statement Alert Panel
- Real-time monitor querying statements taking `> 1.0s` to execute.
- Shows timestamp, query snippet, row count, and execution time in milliseconds.

### 6.5 Live Crawler Syslog Stream
- Streaming terminal log viewer displaying `gaia-daemon` output.
- Log level filtering: `ALL`, `INFO`, `WARN`, `DEBUG`.
- Color-coded severity pills and monospace timestamps.

---

## 7. Global Torrent Inspector Drawer

Clicking on any torrent across the Explorer, Classifier Studio, or Analysis views opens the comprehensive **Torrent Inspector Drawer**.

### 7.1 Header & Cryptographic Verification
- **Bundle Classification**: Badges for `Multi-File Bundle` vs `Single Payload`.
- **Full Release Title**: Complete UTF-8 name extracted from the `.torrent` dictionary.
- **40-Character Hex Infohash**: With one-click clipboard copy button and visual confirmation toast.

### 7.2 Storage & Piece Geometry
- **Total Payload Size**: Formatted in GB/MB.
- **Piece Length**: Cryptographic piece size (e.g., `2.0 MB`, `4.0 MB`).
- **File Count**: Total file count in the bundle.

### 7.3 Verified File Manifest Tree
- Scrollable file structure list displaying relative file paths and individual file byte sizes.
- Verified against SHA-1 piece hashes stored in PostgreSQL `torrents.files`.

### 7.4 Swarm Health & Seed Probe
- **Swarm Health Bar**: Real-time health score percentage.
- **Active Seed Confirmation**: Status badge confirming whether an active, connectable seed was detected.
- **DHT Peer Count**: Number of reachable peers currently announcing in the swarm.
- **Instant Health Re-Probe**: Interactive button to trigger an on-demand real-time health probe against DHT nodes.

### 7.5 Content Classification Summary
- Machine learning predicted category badge with model confidence percentage.
- Link to immediately jump to the **Classifier Studio** for active learning relabeling.

### 7.6 Sighting & Timestamp Telemetry
- **Last Sighted in DHT**: Time and relative recency indicator (green if < 48h, red if stale).
- **Last Health Check Probe**: Timestamp of last automated health worker scrape.
- **Verified At**: Exact date and time (Asia/Dubai GST) when metadata was downloaded.
- **First Discovered**: Initial DHT announce timestamp.
- **Total Sightings Count**: Cumulative sightings across all crawler discovery cycles.

### 7.7 One-Click Magnet Link Generator
- Full-width button that generates a standard BitTorrent magnet link:
  ```text
  magnet:?xt=urn:btih:<infohash>&dn=<encoded_name>&tr=udp://tracker.opentrackr.org:1337/announce...
  ```
- Copies directly to the user's clipboard with instant visual feedback.

---

## Summary of Storage & Performance Configurations

| System Component | Value | Notes |
| :--- | :--- | :--- |
| **PostgreSQL Shared Memory (`/dev/shm`)** | `2.0 GB` | Configured for high-throughput parallel queries over 2.4M records |
| **Active Classifier Model** | `v2 (SGD/Huber)` | 90.40% Macro-F1 across 10 canonical classes |
| **Verified Infohash Catalog** | `~2.40 Million` | Live growing catalog |
| **Total Analyzed Payload Size** | `~14,000+ Terabytes` | Measured across verified `.torrent` manifests |
| **Timezone Standard** | `Asia/Dubai (GST UTC+4)` | Standardized across charts, logs, and tables |
