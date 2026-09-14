use crate::metrics::{Add1, Metrics};
use dashmap::{DashMap, DashSet};
use sqlx::PgPool;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbuseCategory {
    SybilRotator,
    PassiveMonitor,
    RoutingScraper,
    UnreciprocatingLeecher,
    CryptoSpoofer,
    QueryFlooder,
    LegitimatePeer,
}

impl AbuseCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SybilRotator => "Sybil Node Rotator",
            Self::PassiveMonitor => "Passive Swarm Monitor",
            Self::RoutingScraper => "DHT Table Scraper",
            Self::UnreciprocatingLeecher => "Unreciprocating Leecher",
            Self::CryptoSpoofer => "Cryptographic Spoofing Node",
            Self::QueryFlooder => "High-Rate Query Flooder",
            Self::LegitimatePeer => "Legitimate Peer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SurveillanceNodeEntry {
    pub node_ids: Vec<[u8; 20]>,
    pub total_node_ids_seen: u32,
    pub get_peers_count: u64,
    pub find_node_count: u64,
    pub announce_peer_count: u64,
    pub bep42_violations: u32,
    pub bep42_valid: u32,
    pub sample_hashes: Vec<[u8; 20]>,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub is_dirty: bool,
}

impl Default for SurveillanceNodeEntry {
    fn default() -> Self {
        let now = chrono::Utc::now();
        Self {
            node_ids: Vec::with_capacity(2),
            total_node_ids_seen: 0,
            get_peers_count: 0,
            find_node_count: 0,
            announce_peer_count: 0,
            bep42_violations: 0,
            bep42_valid: 0,
            sample_hashes: Vec::with_capacity(5),
            first_seen: now,
            last_seen: now,
            is_dirty: true,
        }
    }
}

pub struct SurveillanceRecorder {
    pool: Option<PgPool>,
    nodes: Arc<DashMap<IpAddr, SurveillanceNodeEntry>>,
    blocked_ips: Arc<DashSet<IpAddr>>,
    immune_ips: Arc<DashSet<IpAddr>>,
    mesh_endpoints: Arc<RwLock<Vec<[u8; 6]>>>,
    metrics: Arc<Metrics>,
}

impl SurveillanceRecorder {
    pub fn new(pool: Option<PgPool>, metrics: Arc<Metrics>) -> Arc<Self> {
        Arc::new(Self {
            pool,
            nodes: Arc::new(DashMap::new()),
            blocked_ips: Arc::new(DashSet::new()),
            immune_ips: Arc::new(DashSet::new()),
            mesh_endpoints: Arc::new(RwLock::new(Vec::with_capacity(2048))),
            metrics,
        })
    }

    /// Preload confirmed malicious surveillance IPs from Postgres into memory,
    /// strictly excluding any proven seedboxes present in stable_peers.
    pub async fn load_blocked_nodes(&self) {
        if let Some(pool) = &self.pool {
            // First preload all proven seedbox IPs from stable_peers into immune_ips
            let immune_rows: Result<Vec<(String,)>, sqlx::Error> =
                sqlx::query_as("SELECT DISTINCT ip::text FROM stable_peers")
                    .fetch_all(pool)
                    .await;
            if let Ok(ips) = immune_rows {
                let mut immune_count = 0;
                for (ip_str,) in ips {
                    let bare_ip = ip_str.split('/').next().unwrap_or(&ip_str);
                    if let Ok(parsed) = bare_ip.parse::<IpAddr>() {
                        self.record_legitimate_peer(parsed);
                        immune_count += 1;
                    }
                }
                tracing::info!(count = immune_count, "surveillance: preloaded proven seedboxes from stable_peers into immunity list");
            }

            let rows: Result<Vec<(String,)>, sqlx::Error> =
                sqlx::query_as("SELECT ip::text FROM dht_surveillance_nodes WHERE is_blocked = TRUE AND ip NOT IN (SELECT ip FROM stable_peers)")
                    .fetch_all(pool)
                    .await;
            match rows {
                Ok(ips) => {
                    let mut count = 0;
                    let mut endpoints = Vec::new();
                    for (ip_str,) in ips {
                        let bare_ip = ip_str.split('/').next().unwrap_or(&ip_str);
                        if let Ok(parsed) = bare_ip.parse::<IpAddr>() {
                            if !self.immune_ips.contains(&parsed) {
                                self.blocked_ips.insert(parsed);
                                if let IpAddr::V4(v4) = parsed {
                                    let mut ep = [0u8; 6];
                                    ep[0..4].copy_from_slice(&v4.octets());
                                    ep[4..6].copy_from_slice(&6881u16.to_be_bytes());
                                    endpoints.push(ep);
                                }
                                count += 1;
                            }
                        }
                    }
                    if !endpoints.is_empty() {
                        if let Ok(mut mesh) = self.mesh_endpoints.write() {
                            mesh.extend(endpoints);
                            if mesh.len() > 4096 {
                                mesh.truncate(4096);
                            }
                        }
                    }
                    tracing::info!(loaded = count, "surveillance: preloaded blocked surveillance IPs into redirection mesh");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "surveillance: failed to preload blocked IPs");
                }
            }
        }
    }

    /// Mark an IP as a proven legitimate peer or seedbox, removing it from blocked_ips
    /// and granting permanent immunity against surveillance blocking.
    pub fn record_legitimate_peer(&self, ip: IpAddr) {
        self.blocked_ips.remove(&ip);
        self.immune_ips.insert(ip);
    }

    /// Fast lock-free check if an IP is a known surveillance/abusive bot
    #[inline]
    pub fn is_blocked(&self, ip: &IpAddr) -> bool {
        !self.immune_ips.contains(ip) && self.blocked_ips.contains(ip)
    }

    /// Returns 4 compact peer endpoints (24 bytes) from the active surveillance mesh,
    /// strictly excluding caller_ip so competing surveillance operations cross-probe each other.
    pub fn get_mesh_peers(&self, caller_ip: &IpAddr) -> [[u8; 6]; 4] {
        let caller_v4 = match caller_ip {
            IpAddr::V4(v4) => Some(v4.octets()),
            IpAddr::V6(_) => None,
        };

        if let Ok(mesh) = self.mesh_endpoints.read() {
            let len = mesh.len();
            if len >= 4 {
                let mut chosen = [[0u8; 6]; 4];
                let mut found = 0;
                let start_idx = rand::random::<u64>() as usize % len;

                for i in 0..len {
                    let idx = (start_idx + i) % len;
                    let ep = mesh[idx];
                    // Never redirect a surveillance bot to itself
                    if let Some(c_octets) = caller_v4 {
                        if ep[0..4] == c_octets {
                            continue;
                        }
                    }
                    // Avoid duplicate peers in the same 4-peer set
                    if chosen[..found].iter().any(|c| *c == ep) {
                        continue;
                    }
                    chosen[found] = ep;
                    found += 1;
                    if found == 4 {
                        return chosen;
                    }
                }
                if found == 4 {
                    return chosen;
                }
            }
        }

        // Fallback to RFC 5737 dummy documentation peers if mesh is not populated yet
        Self::fallback_dummy_peers()
    }

    pub fn fallback_dummy_peers() -> [[u8; 6]; 4] {
        let dummy_ips: [([u8; 4], u16); 4] = [
            ([192, 0, 2, 1], 6881),
            ([198, 51, 100, 1], 6881),
            ([203, 0, 113, 1], 6881),
            ([192, 0, 2, 42], 6881),
        ];
        let mut out = [[0u8; 6]; 4];
        for (i, (ip, port)) in dummy_ips.iter().enumerate() {
            out[i][0..4].copy_from_slice(ip);
            out[i][4..6].copy_from_slice(&port.to_be_bytes());
        }
        out
    }

    /// Update node ID tracking for an entry
    fn track_node_id(entry: &mut SurveillanceNodeEntry, sender_id: Option<&[u8; 20]>) {
        if let Some(id) = sender_id {
            if !entry.node_ids.contains(id) {
                entry.total_node_ids_seen = entry.total_node_ids_seen.saturating_add(1);
                if entry.node_ids.len() < 10 {
                    entry.node_ids.push(*id);
                }
            }
        }
    }

    /// Record an inbound `get_peers` query event
    pub fn record_get_peers(&self, ip: IpAddr, sender_id: Option<&[u8; 20]>, bep42_valid: bool, ih: [u8; 20]) {
        if !bep42_valid {
            self.metrics.surveillance_bep42_violations.add(1);
        }

        let mut entry = self.nodes.entry(ip).or_default();

        entry.get_peers_count = entry.get_peers_count.saturating_add(1);
        Self::track_node_id(&mut entry, sender_id);

        if bep42_valid {
            entry.bep42_valid = entry.bep42_valid.saturating_add(1);
        } else {
            entry.bep42_violations = entry.bep42_violations.saturating_add(1);
        }

        if entry.sample_hashes.len() < 10 && !entry.sample_hashes.contains(&ih) {
            entry.sample_hashes.push(ih);
        }
        entry.last_seen = chrono::Utc::now();
        entry.is_dirty = true;
    }

    /// Record an inbound `find_node` query event (detecting DHT topology scrapers)
    pub fn record_find_node(&self, ip: IpAddr, sender_id: Option<&[u8; 20]>, bep42_valid: bool, _target: [u8; 20]) {
        if !bep42_valid {
            self.metrics.surveillance_bep42_violations.add(1);
        }

        let mut entry = self.nodes.entry(ip).or_default();

        entry.find_node_count = entry.find_node_count.saturating_add(1);
        Self::track_node_id(&mut entry, sender_id);

        if bep42_valid {
            entry.bep42_valid = entry.bep42_valid.saturating_add(1);
        } else {
            entry.bep42_violations = entry.bep42_violations.saturating_add(1);
        }
        entry.last_seen = chrono::Utc::now();
        entry.is_dirty = true;
    }

    /// Record an inbound `announce_peer` event (crucial: proves legitimate swarm participation)
    pub fn record_announce_peer(&self, ip: IpAddr, sender_id: Option<&[u8; 20]>, bep42_valid: bool, ih: [u8; 20]) {
        let mut entry = self.nodes.entry(ip).or_default();

        entry.announce_peer_count = entry.announce_peer_count.saturating_add(1);
        Self::track_node_id(&mut entry, sender_id);

        if bep42_valid {
            entry.bep42_valid = entry.bep42_valid.saturating_add(1);
        } else {
            entry.bep42_violations = entry.bep42_violations.saturating_add(1);
        }

        if entry.sample_hashes.len() < 10 && !entry.sample_hashes.contains(&ih) {
            entry.sample_hashes.push(ih);
        }
        entry.last_seen = chrono::Utc::now();
        entry.is_dirty = true;
    }

    /// Legacy record method forwarding to record_get_peers
    pub fn record(&self, ip: IpAddr, bep42_valid: bool, ih: [u8; 20]) {
        self.record_get_peers(ip, None, bep42_valid, ih);
    }

    /// Sweeps in-memory stats, scores suspicious entities, persists to PostgreSQL in small chunks,
    /// and purges inactive/benign entries from RAM.
    pub async fn flush(&self) {
        if self.nodes.is_empty() {
            return;
        }

        let mut candidates = Vec::new();
        let mut to_purge = Vec::new();
        let now = chrono::Utc::now();

        // 1. Scan DashMap entries
        for mut item in self.nodes.iter_mut() {
            let ip = *item.key();
            let entry = item.value().clone();
            let asn_hint = detect_asn_hint(&ip);
            let total_queries = entry.get_peers_count + entry.find_node_count;

            // Benign single-query home users and seedboxes (single node ID, low queries, non-surveillance)
            let is_known_spy = asn_hint.map(|h| h.is_known_surveillance).unwrap_or(false);
            if entry.total_node_ids_seen <= 1 
                && total_queries < 10 
                && !is_known_spy 
                && !self.blocked_ips.contains(&ip) 
            {
                if (now - entry.last_seen).num_seconds() > 300 {
                    to_purge.push(ip);
                }
                continue;
            }

            // Inactive flushed nodes: prune from memory after 1 hour of inactivity
            if !entry.is_dirty && (now - entry.last_seen).num_seconds() > 3600 {
                to_purge.push(ip);
                continue;
            }

            // Only flush dirty candidates!
            if entry.is_dirty {
                item.is_dirty = false;
                let (score, category, suspected) = calculate_universal_threat_score(&entry, asn_hint);
                candidates.push((ip, entry, asn_hint, score, category, suspected));
            }
        }

        // Purge old entries
        for ip in to_purge {
            self.nodes.remove(&ip);
        }

        if candidates.is_empty() {
            return;
        }

        // 2. Persist to Postgres in small chunks of 50 to avoid long-lived transaction locks
        if let Some(pool) = &self.pool {
            for chunk in candidates.chunks(50) {
                let mut tx = match pool.begin().await {
                    Ok(tx) => tx,
                    Err(e) => {
                        tracing::warn!(error = %e, "surveillance: failed to begin tx for chunk");
                        break;
                    }
                };

                for (ip, entry, asn_hint, score, category, suspected) in chunk {
                    let asn = asn_hint.map(|h| h.asn.to_string());
                    let org = asn_hint.map(|h| h.org.to_string());
                    let is_blocked = *score >= 70 && !self.immune_ips.contains(ip);
                    let sample_hashes: Vec<Vec<u8>> = entry.sample_hashes.iter().map(|h| h.to_vec()).collect();
                    let total_queries = (entry.get_peers_count + entry.find_node_count) as i64;
                    let distinct_node_ids = entry.total_node_ids_seen.max(1) as i32;

                    if is_blocked {
                        if !self.blocked_ips.contains(ip) {
                            self.blocked_ips.insert(*ip);
                            self.metrics.surveillance_nodes_flagged.add(1);
                            if let IpAddr::V4(v4) = ip {
                                let mut ep = [0u8; 6];
                                ep[0..4].copy_from_slice(&v4.octets());
                                ep[4..6].copy_from_slice(&6881u16.to_be_bytes());
                                if let Ok(mut mesh) = self.mesh_endpoints.write() {
                                    if mesh.len() < 4096 {
                                        mesh.push(ep);
                                    } else {
                                        let idx = rand::random::<u64>() as usize % 4096;
                                        mesh[idx] = ep;
                                    }
                                }
                            }
                            tracing::info!(
                                ip = %ip,
                                score = score,
                                category = category.as_str(),
                                entity = %suspected,
                                "surveillance: intercepted & blocked non-contributing/spying node into redirection mesh"
                            );
                        }
                    } else if self.blocked_ips.contains(ip) {
                        self.blocked_ips.remove(ip);
                        if let IpAddr::V4(v4) = ip {
                            let octets = v4.octets();
                            if let Ok(mut mesh) = self.mesh_endpoints.write() {
                                mesh.retain(|ep| ep[0..4] != octets);
                            }
                        }
                        tracing::info!(
                            ip = %ip,
                            score = score,
                            category = category.as_str(),
                            "surveillance: unblocked legitimate peer / seedbox"
                        );
                    }

                    let query_result = sqlx::query(
                        r#"
                        INSERT INTO dht_surveillance_nodes (
                            ip, asn, org, score, query_count, distinct_hashes,
                            bep42_violations, bep42_compliant_count, suspected_entity,
                            sample_hashes, is_blocked, first_seen, last_seen,
                            find_node_count, get_peers_count, announce_peer_count,
                            distinct_node_ids, abuse_category
                        )
                        VALUES ($1::inet, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
                        ON CONFLICT (ip) DO UPDATE SET
                            asn = COALESCE(dht_surveillance_nodes.asn, EXCLUDED.asn),
                            org = COALESCE(dht_surveillance_nodes.org, EXCLUDED.org),
                            score = EXCLUDED.score,
                            query_count = dht_surveillance_nodes.query_count + EXCLUDED.query_count,
                            distinct_hashes = GREATEST(dht_surveillance_nodes.distinct_hashes, EXCLUDED.distinct_hashes),
                            bep42_violations = dht_surveillance_nodes.bep42_violations + EXCLUDED.bep42_violations,
                            bep42_compliant_count = dht_surveillance_nodes.bep42_compliant_count + EXCLUDED.bep42_compliant_count,
                            find_node_count = dht_surveillance_nodes.find_node_count + EXCLUDED.find_node_count,
                            get_peers_count = dht_surveillance_nodes.get_peers_count + EXCLUDED.get_peers_count,
                            announce_peer_count = dht_surveillance_nodes.announce_peer_count + EXCLUDED.announce_peer_count,
                            distinct_node_ids = GREATEST(dht_surveillance_nodes.distinct_node_ids, EXCLUDED.distinct_node_ids),
                            abuse_category = EXCLUDED.abuse_category,
                            suspected_entity = CASE 
                                WHEN dht_surveillance_nodes.suspected_entity = 'Unknown Monitor' 
                                     OR dht_surveillance_nodes.suspected_entity = 'Suspicious Query Node'
                                     OR dht_surveillance_nodes.suspected_entity = 'Suspicious Non-Contributing Node'
                                THEN EXCLUDED.suspected_entity 
                                ELSE dht_surveillance_nodes.suspected_entity 
                            END,
                            sample_hashes = ARRAY(
                                SELECT DISTINCT elem FROM UNNEST(dht_surveillance_nodes.sample_hashes || EXCLUDED.sample_hashes) AS elem LIMIT 10
                            ),
                            is_blocked = EXCLUDED.is_blocked,
                            last_seen = EXCLUDED.last_seen
                        "#,
                    )
                    .bind(ip.to_string())
                    .bind(asn)
                    .bind(org)
                    .bind(score)
                    .bind(total_queries)
                    .bind(entry.sample_hashes.len() as i32)
                    .bind(entry.bep42_violations as i32)
                    .bind(entry.bep42_valid as i32)
                    .bind(suspected)
                    .bind(&sample_hashes)
                    .bind(is_blocked)
                    .bind(entry.first_seen)
                    .bind(entry.last_seen)
                    .bind(entry.find_node_count as i64)
                    .bind(entry.get_peers_count as i64)
                    .bind(entry.announce_peer_count as i64)
                    .bind(distinct_node_ids)
                    .bind(category.as_str())
                    .execute(&mut *tx)
                    .await;

                    if let Err(e) = query_result {
                        tracing::warn!(error = %e, ip = %ip, "surveillance: failed to upsert node");
                    }
                }

                if let Err(e) = tx.commit().await {
                    tracing::warn!(error = %e, "surveillance: commit failed for chunk");
                }
            }
        }
    }

    /// Periodic background flush runner
    pub async fn run(self: Arc<Self>, interval: Duration) {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;
            self.flush().await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsnHint {
    pub asn: &'static str,
    pub org: &'static str,
    pub hint_name: &'static str,
    pub is_known_surveillance: bool,
}

/// Fast subnet bitwise check
fn match_prefix(ip: &Ipv4Addr, net: [u8; 4], mask_bits: u8) -> bool {
    let ip_u32 = u32::from_be_bytes(ip.octets());
    let net_u32 = u32::from_be_bytes(net);
    let mask = if mask_bits == 0 {
        0
    } else {
        !((1u64 << (32 - mask_bits)) - 1) as u32
    };
    (ip_u32 & mask) == (net_u32 & mask)
}

/// Detect known datacenter and surveillance autonomous systems
pub fn detect_asn_hint(ip: &IpAddr) -> Option<AsnHint> {
    let v4 = match ip {
        IpAddr::V4(v4) => *v4,
        IpAddr::V6(_) => return None,
    };

    // ── Dedicated Surveillance & Copyright Monitoring Networks ──

    // Selectel (IKWYD primary surveillance infrastructure)
    if match_prefix(&v4, [95, 213, 0, 0], 16)
        || match_prefix(&v4, [185, 129, 100, 0], 22)
        || match_prefix(&v4, [188, 93, 16, 0], 21)
        || match_prefix(&v4, [188, 225, 0, 0], 17)
        || match_prefix(&v4, [85, 119, 144, 0], 20)
    {
        return Some(AsnHint {
            asn: "AS49505",
            org: "Selectel Network",
            hint_name: "IKWYD Spying Node (Selectel)",
            is_known_surveillance: true,
        });
    }

    // DataCamp / CDN77 (Passive Torrent Harvesters)
    if match_prefix(&v4, [185, 220, 100, 0], 22)
        || match_prefix(&v4, [194, 26, 29, 0], 24)
        || match_prefix(&v4, [185, 107, 56, 0], 22)
        || match_prefix(&v4, [195, 181, 160, 0], 19)
    {
        return Some(AsnHint {
            asn: "AS60068",
            org: "DataCamp Limited",
            hint_name: "DataCamp Passive Scraper",
            is_known_surveillance: true,
        });
    }

    // M247 Ltd (Hosting & VPN surveillance egress)
    if match_prefix(&v4, [185, 244, 24, 0], 22)
        || match_prefix(&v4, [185, 246, 128, 0], 22)
        || match_prefix(&v4, [193, 29, 104, 0], 22)
        || match_prefix(&v4, [89, 34, 24, 0], 21)
        || match_prefix(&v4, [45, 138, 16, 0], 22)
    {
        return Some(AsnHint {
            asn: "AS9009",
            org: "M247 Ltd",
            hint_name: "M247 DHT Probe",
            is_known_surveillance: true,
        });
    }

    // Cogent (MarkMonitor & commercial copyright monitors)
    if match_prefix(&v4, [130, 117, 0, 0], 16)
        || match_prefix(&v4, [154, 54, 0, 0], 16)
        || match_prefix(&v4, [38, 0, 0, 0], 8)
        || match_prefix(&v4, [66, 28, 0, 0], 16)
    {
        return Some(AsnHint {
            asn: "AS174",
            org: "Cogent Communications",
            hint_name: "Copyright Monitor (MarkMonitor/Cogent)",
            is_known_surveillance: true,
        });
    }

    // ── Generic Cloud & Hosting Networks (Where legitimate seedboxes ALSO live) ──

    // Hetzner Online (Primary European seedbox hosting)
    if match_prefix(&v4, [78, 46, 0, 0], 15)
        || match_prefix(&v4, [88, 198, 0, 0], 16)
        || match_prefix(&v4, [94, 130, 0, 0], 16)
        || match_prefix(&v4, [95, 216, 0, 0], 15)
        || match_prefix(&v4, [135, 181, 0, 0], 16)
        || match_prefix(&v4, [136, 243, 0, 0], 16)
        || match_prefix(&v4, [138, 201, 0, 0], 16)
        || match_prefix(&v4, [144, 76, 0, 0], 16)
        || match_prefix(&v4, [159, 69, 0, 0], 16)
        || match_prefix(&v4, [162, 55, 0, 0], 16)
        || match_prefix(&v4, [168, 119, 0, 0], 16)
    {
        return Some(AsnHint {
            asn: "AS24940",
            org: "Hetzner Online GmbH",
            hint_name: "Hetzner Hosting",
            is_known_surveillance: false,
        });
    }

    // OVH SAS (Popular European & Canadian seedbox hosting)
    if match_prefix(&v4, [51, 68, 0, 0], 14)
        || match_prefix(&v4, [147, 135, 0, 0], 16)
        || match_prefix(&v4, [145, 239, 0, 0], 16)
        || match_prefix(&v4, [188, 165, 0, 0], 16)
        || match_prefix(&v4, [54, 36, 0, 0], 14)
        || match_prefix(&v4, [144, 217, 0, 0], 16)
        || match_prefix(&v4, [37, 187, 0, 0], 16)
        || match_prefix(&v4, [5, 196, 0, 0], 16)
        || match_prefix(&v4, [5, 39, 0, 0], 16)
        || match_prefix(&v4, [135, 125, 0, 0], 16)
        || match_prefix(&v4, [158, 69, 0, 0], 16)
        || match_prefix(&v4, [217, 182, 0, 0], 16)
        || match_prefix(&v4, [51, 75, 0, 0], 16)
        || match_prefix(&v4, [51, 81, 0, 0], 16)
        || match_prefix(&v4, [57, 129, 0, 0], 16)
    {
        return Some(AsnHint {
            asn: "AS16276",
            org: "OVH SAS",
            hint_name: "OVH Hosting",
            is_known_surveillance: false,
        });
    }

    // LeaseWeb
    if match_prefix(&v4, [85, 17, 0, 0], 16)
        || match_prefix(&v4, [178, 162, 0, 0], 16)
        || match_prefix(&v4, [95, 211, 0, 0], 16)
        || match_prefix(&v4, [37, 48, 0, 0], 16)
        || match_prefix(&v4, [84, 16, 0, 0], 16)
    {
        return Some(AsnHint {
            asn: "AS60781",
            org: "LeaseWeb Netherlands B.V.",
            hint_name: "LeaseWeb Hosting",
            is_known_surveillance: false,
        });
    }

    // DigitalOcean
    if match_prefix(&v4, [104, 131, 0, 0], 16)
        || match_prefix(&v4, [104, 248, 0, 0], 16)
        || match_prefix(&v4, [134, 209, 0, 0], 16)
        || match_prefix(&v4, [138, 68, 0, 0], 16)
        || match_prefix(&v4, [138, 197, 0, 0], 16)
        || match_prefix(&v4, [142, 93, 0, 0], 16)
        || match_prefix(&v4, [159, 203, 0, 0], 16)
        || match_prefix(&v4, [159, 89, 0, 0], 16)
        || match_prefix(&v4, [165, 22, 0, 0], 16)
        || match_prefix(&v4, [167, 99, 0, 0], 16)
        || match_prefix(&v4, [178, 62, 0, 0], 16)
        || match_prefix(&v4, [188, 166, 0, 0], 16)
        || match_prefix(&v4, [206, 189, 0, 0], 16)
        || match_prefix(&v4, [46, 101, 0, 0], 16)
    {
        return Some(AsnHint {
            asn: "AS14061",
            org: "DigitalOcean LLC",
            hint_name: "DigitalOcean Cloud",
            is_known_surveillance: false,
        });
    }

    // Amazon AWS
    if match_prefix(&v4, [3, 0, 0, 0], 9)
        || match_prefix(&v4, [18, 128, 0, 0], 9)
        || match_prefix(&v4, [34, 192, 0, 0], 10)
        || match_prefix(&v4, [35, 152, 0, 0], 13)
        || match_prefix(&v4, [52, 0, 0, 0], 11)
        || match_prefix(&v4, [54, 0, 0, 0], 12)
    {
        return Some(AsnHint {
            asn: "AS16509",
            org: "Amazon.com Inc",
            hint_name: "Amazon AWS Cloud",
            is_known_surveillance: false,
        });
    }

    None
}

/// Universal threat and abuse scoring algorithm:
/// Evaluates Sybil node rotation, zero-contribution asymmetry, routing table scraping,
/// multi-hash dispersion, and applies strict guardrails for seedboxes and normal BitTorrent peers.
pub fn calculate_universal_threat_score(
    entry: &SurveillanceNodeEntry,
    asn_hint: Option<AsnHint>,
) -> (i32, AbuseCategory, String) {
    let total_queries = entry.get_peers_count + entry.find_node_count;
    let is_known_spy = asn_hint.as_ref().map(|h| h.is_known_surveillance).unwrap_or(false);

    // 1. HARD IMMUNITY: Legitimate Swarm Announcer / Seedbox
    // Any node with only 1 node ID that announced peers is actively participating in swarms.
    if entry.announce_peer_count > 0 && entry.total_node_ids_seen <= 1 {
        let name = if let Some(h) = &asn_hint {
            format!("Verified Seedbox / Announcer ({})", h.org)
        } else {
            "Legitimate Swarm Participant".to_string()
        };
        return (0, AbuseCategory::LegitimatePeer, name);
    }

    // 2. MINIMUM EVIDENCE GUARDRAIL: Low-volume casual peers
    // A node with < 10 queries cannot be blocked unless it is a clear Sybil rotator (>=3 IDs)
    // or a confirmed commercial surveillance operator (e.g. Selectel / IKWYD).
    if total_queries < 10 && entry.total_node_ids_seen < 3 && !is_known_spy {
        return (
            (total_queries as i32 * 2).min(20),
            AbuseCategory::LegitimatePeer,
            "Provisional / Casual DHT Peer (Low Volume)".to_string(),
        );
    }

    let mut score: i32 = 0;

    // 3. Sybil Attack: Multiple Node IDs from single IP
    // 1 ID: 0 pts (normal BitTorrent client)
    // 2 IDs: +10 pts (provisional / reboot / dual client)
    // 3..4 IDs: +45 pts (clear Sybil rotation)
    // 5+ IDs: +65 pts (aggressive Sybil swarm)
    if entry.total_node_ids_seen >= 5 {
        score += 65;
    } else if entry.total_node_ids_seen >= 3 {
        score += 45;
    } else if entry.total_node_ids_seen == 2 {
        score += 10;
    }

    // 4. Traffic Asymmetry: Harvesting get_peers without announcing
    if entry.get_peers_count >= 50 && entry.announce_peer_count == 0 {
        score += 40;
    } else if entry.get_peers_count >= 20 && entry.announce_peer_count == 0 {
        score += 25;
    } else if entry.get_peers_count >= 10 && entry.announce_peer_count == 0 {
        score += 10;
    }

    // 5. Routing Table Scraping: High find_node volume without announcing
    if entry.find_node_count >= 100 && entry.announce_peer_count == 0 {
        score += 45;
    } else if entry.find_node_count >= 50 && entry.announce_peer_count == 0 {
        score += 30;
    } else if entry.find_node_count >= 25 && entry.announce_peer_count == 0 {
        score += 15;
    }

    // 6. Multi-Hash Dispersion: Scanning many distinct swarms without announcing
    if entry.sample_hashes.len() >= 10 && entry.announce_peer_count == 0 {
        score += 20;
    } else if entry.sample_hashes.len() >= 5 && entry.announce_peer_count == 0 {
        score += 10;
    }

    // 7. ASN & Infrastructure Profiling
    if is_known_spy {
        score += 35;
    } else if asn_hint.is_some() {
        // Generic hosting (Hetzner, OVH, Leaseweb, DO, AWS):
        // Only penalize if high unreciprocated query volume!
        if total_queries >= 30 && entry.announce_peer_count == 0 {
            score += 10;
        }
    }

    // 8. Extreme Query Volume
    if total_queries >= 200 && entry.announce_peer_count == 0 {
        score += 25;
    } else if total_queries >= 100 && entry.announce_peer_count == 0 {
        score += 15;
    }

    // 9. BEP 42 Cryptographic Verification
    // BEP 42 compliance is a POSITIVE trust credential (IP ownership proven).
    // Non-compliance is normal (random Node IDs in Transmission/rTorrent/libtorrent).
    if entry.bep42_valid > 0 && entry.bep42_violations == 0 && entry.total_node_ids_seen <= 1 {
        score = score.saturating_sub(20);
    } else if entry.bep42_violations > 0 && entry.bep42_valid == 0 {
        // Only add points if already suspicious (high unreciprocated query volume)
        if total_queries >= 30 && entry.announce_peer_count == 0 {
            score += 10;
        }
    }

    // 10. Legitimacy Credits (Announce discounts for multi-ID or high-query nodes)
    if entry.announce_peer_count > 0 {
        let announce_discount = (entry.announce_peer_count as i32 * 20).min(60);
        score = score.saturating_sub(announce_discount);
    }

    // Absolute guardrail for standard BitTorrent clients & seedboxes:
    // Any node querying swarms (get_peers > 0) that does NOT belong to a confirmed
    // copyright surveillance operator (Selectel/IKWYD, DataCamp, M247, Cogent)
    // and uses <= 4 node IDs is a standard BitTorrent peer or seedbox.
    // It can NEVER exceed score 45 and can NEVER be blocked.
    let final_score = if !is_known_spy && entry.get_peers_count > 0 && entry.total_node_ids_seen <= 4 && total_queries < 300 {
        score.clamp(0, 45)
    } else {
        score.clamp(0, 100)
    };

    // Dynamic categorization
    let (category, suspected) = if entry.total_node_ids_seen >= 5 {
        (
            AbuseCategory::SybilRotator,
            format!("Sybil Node Rotator ({} distinct Node IDs)", entry.total_node_ids_seen),
        )
    } else if is_known_spy {
        let h = asn_hint.as_ref().unwrap();
        (
            AbuseCategory::PassiveMonitor,
            format!("{} ({})", h.hint_name, h.org),
        )
    } else if entry.find_node_count >= 100 && entry.get_peers_count == 0 && entry.announce_peer_count == 0 {
        (
            AbuseCategory::RoutingScraper,
            format!("DHT Table Scraper ({} find_node probes)", entry.find_node_count),
        )
    } else if final_score >= 70 && total_queries >= 300 && entry.announce_peer_count == 0 {
        (
            AbuseCategory::QueryFlooder,
            format!("High-Rate Query Flooder ({} total queries)", total_queries),
        )
    } else if final_score >= 70 {
        (
            AbuseCategory::UnreciprocatingLeecher,
            "Unreciprocating DHT Leecher (0 Swarm Contribution)".to_string(),
        )
    } else {
        (
            AbuseCategory::LegitimatePeer,
            if let Some(h) = &asn_hint {
                format!("Legitimate Peer ({})", h.org)
            } else {
                "Legitimate Swarm Participant".to_string()
            },
        )
    };

    (final_score, category, suspected)
}

/// Backwards compatibility helper for legacy threat score tests
pub fn calculate_threat_score(
    entry: &SurveillanceNodeEntry,
    asn_hint: Option<AsnHint>,
) -> (i32, String) {
    let (score, _, suspected) = calculate_universal_threat_score(entry, asn_hint);
    (score, suspected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selectel_asn_detection() {
        let ip = IpAddr::V4(Ipv4Addr::new(95, 213, 10, 5));
        let hint = detect_asn_hint(&ip);
        assert!(hint.is_some());
        let hint = hint.unwrap();
        assert_eq!(hint.asn, "AS49505");
        assert!(hint.hint_name.contains("Selectel"));
        assert!(hint.is_known_surveillance);
    }

    #[test]
    fn test_hetzner_asn_detection_is_not_known_surveillance() {
        let ip = IpAddr::V4(Ipv4Addr::new(88, 198, 50, 1));
        let hint = detect_asn_hint(&ip);
        assert!(hint.is_some());
        let hint = hint.unwrap();
        assert_eq!(hint.asn, "AS24940");
        assert!(!hint.is_known_surveillance, "Hetzner is generic hosting, not dedicated surveillance");
    }

    #[test]
    fn test_seedbox_on_hetzner_with_announce_has_hard_immunity() {
        let entry = SurveillanceNodeEntry {
            node_ids: vec![[1u8; 20]],
            total_node_ids_seen: 1,
            get_peers_count: 5,
            find_node_count: 2,
            announce_peer_count: 1, // Swarm announce!
            bep42_violations: 7,    // Random Node ID (standard for many seedboxes)
            bep42_valid: 0,
            sample_hashes: vec![[1u8; 20]],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            is_dirty: true,
        };
        let hetzner_hint = detect_asn_hint(&IpAddr::V4(Ipv4Addr::new(88, 198, 50, 1)));
        let (score, category, desc) = calculate_universal_threat_score(&entry, hetzner_hint);
        assert_eq!(score, 0, "Seedbox with announce must have score 0");
        assert_eq!(category, AbuseCategory::LegitimatePeer);
        assert!(desc.contains("Verified Seedbox"));
    }

    #[test]
    fn test_seedbox_on_ovh_low_volume_not_blocked() {
        let entry = SurveillanceNodeEntry {
            node_ids: vec![[1u8; 20]],
            total_node_ids_seen: 1,
            get_peers_count: 2,
            find_node_count: 3,
            announce_peer_count: 0, // Has not announced yet (just started)
            bep42_violations: 5,
            bep42_valid: 0,
            sample_hashes: vec![[1u8; 20]],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            is_dirty: true,
        };
        let ovh_hint = detect_asn_hint(&IpAddr::V4(Ipv4Addr::new(147, 135, 10, 1)));
        let (score, category, _) = calculate_universal_threat_score(&entry, ovh_hint);
        assert!(score <= 20, "Low-volume node on OVH must be protected by guardrail (score: {})", score);
        assert_eq!(category, AbuseCategory::LegitimatePeer);
    }

    #[test]
    fn test_residential_user_low_volume_not_blocked() {
        let entry = SurveillanceNodeEntry {
            node_ids: vec![[1u8; 20]],
            total_node_ids_seen: 1,
            get_peers_count: 2,
            find_node_count: 1,
            announce_peer_count: 0,
            bep42_violations: 3, // Random node ID
            bep42_valid: 0,
            sample_hashes: vec![[1u8; 20]],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            is_dirty: true,
        };
        let (score, category, _) = calculate_universal_threat_score(&entry, None);
        assert!(score <= 20, "Residential user must have score <= 20");
        assert_eq!(category, AbuseCategory::LegitimatePeer);
    }

    #[test]
    fn test_true_sybil_crawler_is_blocked() {
        let entry = SurveillanceNodeEntry {
            node_ids: vec![[1u8; 20], [2u8; 20], [3u8; 20], [4u8; 20], [5u8; 20]],
            total_node_ids_seen: 5, // 5 distinct IDs rotated!
            get_peers_count: 30,
            find_node_count: 20,
            announce_peer_count: 0,
            bep42_violations: 50,
            bep42_valid: 0,
            sample_hashes: vec![[10u8; 20]],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            is_dirty: true,
        };
        let (score, category, desc) = calculate_universal_threat_score(&entry, None);
        assert!(score >= 70, "Sybil crawler must be >= 70 (score: {})", score);
        assert_eq!(category, AbuseCategory::SybilRotator);
        assert!(desc.contains("Sybil"));
    }

    #[test]
    fn test_true_passive_swarm_monitor_is_blocked() {
        let entry = SurveillanceNodeEntry {
            node_ids: vec![[1u8; 20]],
            total_node_ids_seen: 1,
            get_peers_count: 60,   // High-volume peer harvesting
            find_node_count: 0,
            announce_peer_count: 0, // 0 contribution
            bep42_violations: 60,
            bep42_valid: 0,
            sample_hashes: vec![
                [1u8; 20], [2u8; 20], [3u8; 20], [4u8; 20], [5u8; 20],
                [6u8; 20], [7u8; 20], [8u8; 20], [9u8; 20], [10u8; 20],
            ],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            is_dirty: true,
        };
        let selectel = detect_asn_hint(&IpAddr::V4(Ipv4Addr::new(95, 213, 10, 5)));
        let (score, category, desc) = calculate_universal_threat_score(&entry, selectel);
        assert!(score >= 70, "Selectel passive monitor must be >= 70 (score: {})", score);
        assert_eq!(category, AbuseCategory::PassiveMonitor);
        assert!(desc.contains("IKWYD Spying Node"));
    }

    #[test]
    fn test_true_dht_routing_scraper_is_blocked() {
        let entry = SurveillanceNodeEntry {
            node_ids: vec![[1u8; 20]],
            total_node_ids_seen: 1,
            get_peers_count: 0,
            find_node_count: 120, // 120 table crawl probes
            announce_peer_count: 0,
            bep42_violations: 120,
            bep42_valid: 0,
            sample_hashes: vec![],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            is_dirty: true,
        };
        let (score, category, desc) = calculate_universal_threat_score(&entry, None);
        assert!(score >= 70, "Routing table scraper must be >= 70 (score: {})", score);
        assert_eq!(category, AbuseCategory::RoutingScraper);
        assert!(desc.contains("DHT Table Scraper"));
    }

    #[test]
    fn test_mesh_peers_excludes_caller() {
        let metrics = Arc::new(Metrics::default());
        let recorder = SurveillanceRecorder::new(None, metrics);

        // Preload 5 rival surveillance endpoints into mesh
        let endpoints: [[u8; 6]; 5] = [
            [10, 0, 0, 1, 0x1a, 0xe1], // 10.0.0.1:6881
            [10, 0, 0, 2, 0x1a, 0xe1], // 10.0.0.2:6881
            [10, 0, 0, 3, 0x1a, 0xe1], // 10.0.0.3:6881
            [10, 0, 0, 4, 0x1a, 0xe1], // 10.0.0.4:6881
            [10, 0, 0, 5, 0x1a, 0xe1], // 10.0.0.5:6881
        ];
        {
            let mut mesh = recorder.mesh_endpoints.write().unwrap();
            mesh.extend_from_slice(&endpoints);
        }

        // Caller is 10.0.0.1
        let caller = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let peers = recorder.get_mesh_peers(&caller);

        // Assert caller is strictly excluded from returned peers
        for peer in &peers {
            assert_ne!(
                &peer[0..4],
                &[10, 0, 0, 1],
                "Mesh peer must NEVER redirect back to caller IP"
            );
        }

        // Assert 4 distinct peers were returned
        assert_eq!(peers.len(), 4);
    }

    #[test]
    fn test_mesh_peers_fallback_when_empty() {
        let metrics = Arc::new(Metrics::default());
        let recorder = SurveillanceRecorder::new(None, metrics);

        // Empty mesh returns RFC 5737 dummy peers
        let caller = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let peers = recorder.get_mesh_peers(&caller);
        assert_eq!(peers.len(), 4);
        assert_eq!(&peers[0][0..4], &[192, 0, 2, 1]);
    }
}
