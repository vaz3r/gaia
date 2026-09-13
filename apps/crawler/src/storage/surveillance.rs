use crate::metrics::{Add1, Metrics};
use dashmap::{DashMap, DashSet};
use sqlx::PgPool;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct SurveillanceNodeEntry {
    pub query_count: u32,
    pub bep42_violations: u32,
    pub bep42_valid: u32,
    pub sample_hashes: Vec<[u8; 20]>,
    pub first_seen: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
}

pub struct SurveillanceRecorder {
    pool: Option<PgPool>,
    nodes: Arc<DashMap<IpAddr, SurveillanceNodeEntry>>,
    blocked_ips: Arc<DashSet<IpAddr>>,
    metrics: Arc<Metrics>,
}

impl SurveillanceRecorder {
    pub fn new(pool: Option<PgPool>, metrics: Arc<Metrics>) -> Arc<Self> {
        Arc::new(Self {
            pool,
            nodes: Arc::new(DashMap::new()),
            blocked_ips: Arc::new(DashSet::new()),
            metrics,
        })
    }

    /// Preload confirmed malicious surveillance IPs from Postgres into memory
    pub async fn load_blocked_nodes(&self) {
        if let Some(pool) = &self.pool {
            let rows: Result<Vec<(String,)>, sqlx::Error> =
                sqlx::query_as("SELECT ip::text FROM dht_surveillance_nodes WHERE is_blocked = TRUE")
                    .fetch_all(pool)
                    .await;
            match rows {
                Ok(ips) => {
                    let mut count = 0;
                    for (ip_str,) in ips {
                        if let Ok(parsed) = ip_str.parse::<IpAddr>() {
                            self.blocked_ips.insert(parsed);
                            count += 1;
                        }
                    }
                    tracing::info!(loaded = count, "surveillance: preloaded blocked surveillance IPs");
                }
                Err(e) => {
                    tracing::warn!(error = %e, "surveillance: failed to preload blocked IPs");
                }
            }
        }
    }

    /// Fast lock-free check if an IP is a known surveillance bot
    #[inline]
    pub fn is_blocked(&self, ip: &IpAddr) -> bool {
        self.blocked_ips.contains(ip)
    }

    /// Record an inbound query event in real time. DashMap shard lookup takes < 100ns.
    pub fn record(&self, ip: IpAddr, bep42_valid: bool, ih: [u8; 20]) {
        if !bep42_valid {
            self.metrics.surveillance_bep42_violations.add(1);
        }

        let mut entry = self.nodes.entry(ip).or_insert_with(|| SurveillanceNodeEntry {
            query_count: 0,
            bep42_violations: 0,
            bep42_valid: 0,
            sample_hashes: Vec::with_capacity(5),
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
        });

        entry.query_count = entry.query_count.saturating_add(1);
        if bep42_valid {
            entry.bep42_valid = entry.bep42_valid.saturating_add(1);
        } else {
            entry.bep42_violations = entry.bep42_violations.saturating_add(1);
        }

        if entry.sample_hashes.len() < 5 && !entry.sample_hashes.contains(&ih) {
            entry.sample_hashes.push(ih);
        }
        entry.last_seen = chrono::Utc::now();
    }

    /// Sweeps in-memory stats, scores suspicious entities, persists to PostgreSQL,
    /// and purges benign single-query users from RAM.
    pub async fn flush(&self) {
        if self.nodes.is_empty() {
            return;
        }

        let mut candidates = Vec::new();
        let mut to_purge = Vec::new();

        // 1. Scan DashMap entries
        for item in self.nodes.iter() {
            let ip = *item.key();
            let entry = item.value().clone();
            let asn_hint = detect_asn_hint(&ip);

            // Benign single-query home users (valid BEP42, low query rate, non-datacenter)
            if entry.bep42_violations == 0 && entry.query_count < 10 && asn_hint.is_none() && !self.blocked_ips.contains(&ip) {
                if (chrono::Utc::now() - entry.last_seen).num_seconds() > 300 {
                    to_purge.push(ip);
                }
                continue;
            }

            // Suspicious or surveillance candidate
            let (score, suspected) = calculate_threat_score(&entry, asn_hint);
            candidates.push((ip, entry, asn_hint, score, suspected));
        }

        // Purge old benign entries
        for ip in to_purge {
            self.nodes.remove(&ip);
        }

        if candidates.is_empty() {
            return;
        }

        // 2. Persist to Postgres if available
        if let Some(pool) = &self.pool {
            let mut tx = match pool.begin().await {
                Ok(tx) => tx,
                Err(e) => {
                    tracing::warn!(error = %e, "surveillance: failed to begin tx");
                    return;
                }
            };

            for (ip, entry, asn_hint, score, suspected) in &candidates {
                let asn = asn_hint.map(|(a, _, _)| a.to_string());
                let org = asn_hint.map(|(_, o, _)| o.to_string());
                let is_blocked = *score >= 60;
                let sample_hashes: Vec<Vec<u8>> = entry.sample_hashes.iter().map(|h| h.to_vec()).collect();

                if is_blocked && !self.blocked_ips.contains(ip) {
                    self.blocked_ips.insert(*ip);
                    self.metrics.surveillance_nodes_flagged.add(1);
                    tracing::info!(ip = %ip, score = score, entity = suspected, "surveillance: flagged & blocked spy bot");
                }

                let query_result = sqlx::query(
                    r#"
                    INSERT INTO dht_surveillance_nodes (
                        ip, asn, org, score, query_count, distinct_hashes,
                        bep42_violations, bep42_compliant_count, suspected_entity,
                        sample_hashes, is_blocked, first_seen, last_seen
                    )
                    VALUES ($1::inet, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
                    ON CONFLICT (ip) DO UPDATE SET
                        asn = COALESCE(dht_surveillance_nodes.asn, EXCLUDED.asn),
                        org = COALESCE(dht_surveillance_nodes.org, EXCLUDED.org),
                        score = GREATEST(dht_surveillance_nodes.score, EXCLUDED.score),
                        query_count = dht_surveillance_nodes.query_count + EXCLUDED.query_count,
                        distinct_hashes = GREATEST(dht_surveillance_nodes.distinct_hashes, EXCLUDED.distinct_hashes),
                        bep42_violations = dht_surveillance_nodes.bep42_violations + EXCLUDED.bep42_violations,
                        bep42_compliant_count = dht_surveillance_nodes.bep42_compliant_count + EXCLUDED.bep42_compliant_count,
                        suspected_entity = CASE 
                            WHEN dht_surveillance_nodes.suspected_entity = 'Unknown Monitor' 
                                 OR dht_surveillance_nodes.suspected_entity = 'Suspicious Query Node'
                            THEN EXCLUDED.suspected_entity 
                            ELSE dht_surveillance_nodes.suspected_entity 
                        END,
                        sample_hashes = ARRAY(
                            SELECT DISTINCT elem FROM UNNEST(dht_surveillance_nodes.sample_hashes || EXCLUDED.sample_hashes) AS elem LIMIT 10
                        ),
                        is_blocked = (GREATEST(dht_surveillance_nodes.score, EXCLUDED.score) >= 60),
                        last_seen = EXCLUDED.last_seen
                    "#,
                )
                .bind(ip.to_string())
                .bind(asn)
                .bind(org)
                .bind(score)
                .bind(entry.query_count as i64)
                .bind(entry.sample_hashes.len() as i32)
                .bind(entry.bep42_violations as i32)
                .bind(entry.bep42_valid as i32)
                .bind(suspected)
                .bind(&sample_hashes)
                .bind(is_blocked)
                .bind(entry.first_seen)
                .bind(entry.last_seen)
                .execute(&mut *tx)
                .await;

                if let Err(e) = query_result {
                    tracing::warn!(error = %e, ip = %ip, "surveillance: failed to upsert node");
                }
            }

            if let Err(e) = tx.commit().await {
                tracing::warn!(error = %e, "surveillance: commit failed");
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
pub fn detect_asn_hint(ip: &IpAddr) -> Option<(&'static str, &'static str, &'static str)> {
    let v4 = match ip {
        IpAddr::V4(v4) => *v4,
        IpAddr::V6(_) => return None,
    };

    // Selectel (IKWYD primary network)
    if match_prefix(&v4, [95, 213, 0, 0], 16)
        || match_prefix(&v4, [185, 129, 100, 0], 22)
        || match_prefix(&v4, [188, 93, 16, 0], 21)
        || match_prefix(&v4, [188, 225, 0, 0], 17)
        || match_prefix(&v4, [85, 119, 144, 0], 20)
    {
        return Some(("AS49505", "Selectel Network", "IKWYD Spying Node (Selectel)"));
    }

    // DataCamp / CDN77 (Passive Torrent Scraper)
    if match_prefix(&v4, [185, 220, 100, 0], 22)
        || match_prefix(&v4, [194, 26, 29, 0], 24)
        || match_prefix(&v4, [185, 107, 56, 0], 22)
        || match_prefix(&v4, [195, 181, 160, 0], 19)
    {
        return Some(("AS60068", "DataCamp Limited", "DataCamp Passive Scraper"));
    }

    // M247 Ltd (Hosting & VPN surveillance egress)
    if match_prefix(&v4, [185, 244, 24, 0], 22)
        || match_prefix(&v4, [185, 246, 128, 0], 22)
        || match_prefix(&v4, [193, 29, 104, 0], 22)
        || match_prefix(&v4, [89, 34, 24, 0], 21)
        || match_prefix(&v4, [45, 138, 16, 0], 22)
    {
        return Some(("AS9009", "M247 Ltd", "M247 DHT Probe"));
    }

    // Hetzner Online
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
        return Some(("AS24940", "Hetzner Online GmbH", "Cloud DHT Crawler (Hetzner)"));
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
        return Some(("AS14061", "DigitalOcean LLC", "Cloud Monitor (DigitalOcean)"));
    }

    // Cogent (MarkMonitor & commercial traffic)
    if match_prefix(&v4, [130, 117, 0, 0], 16)
        || match_prefix(&v4, [154, 54, 0, 0], 16)
        || match_prefix(&v4, [38, 0, 0, 0], 8)
        || match_prefix(&v4, [66, 28, 0, 0], 16)
    {
        return Some(("AS174", "Cogent Communications", "Copyright Monitor (MarkMonitor/Cogent)"));
    }

    // Amazon AWS
    if match_prefix(&v4, [3, 0, 0, 0], 9)
        || match_prefix(&v4, [18, 128, 0, 0], 9)
        || match_prefix(&v4, [34, 192, 0, 0], 10)
        || match_prefix(&v4, [35, 152, 0, 0], 13)
        || match_prefix(&v4, [52, 0, 0, 0], 11)
        || match_prefix(&v4, [54, 0, 0, 0], 12)
    {
        return Some(("AS16509", "Amazon.com Inc", "Cloud Crawler (Amazon AWS)"));
    }

    None
}

/// Computes surveillance threat score (0-100) and suspected entity label
pub fn calculate_threat_score(
    entry: &SurveillanceNodeEntry,
    asn_hint: Option<(&'static str, &'static str, &'static str)>,
) -> (i32, String) {
    let mut score = 0;

    // 1. BEP 42 Violation check (+35 pts, or +45 if 100% violations)
    if entry.bep42_violations > 0 {
        if entry.bep42_valid == 0 {
            score += 45;
        } else {
            score += 35;
        }
    }

    // 2. Datacenter ASN check (+35 pts)
    let suspected = if let Some((_, _, s)) = asn_hint {
        score += 35;
        s.to_string()
    } else if entry.bep42_violations > 0 {
        "Sybil Crawler / Spoofed Node ID".to_string()
    } else if entry.query_count >= 20 {
        "High-Volume DHT Scraper".to_string()
    } else {
        "Suspicious Query Node".to_string()
    };

    // 3. High query rate (+15 to +25 pts)
    if entry.query_count >= 50 {
        score += 25;
    } else if entry.query_count >= 15 {
        score += 15;
    }

    // 4. Multi-hash tracking (+10 to +15 pts)
    if entry.sample_hashes.len() >= 4 {
        score += 15;
    } else if entry.sample_hashes.len() >= 2 {
        score += 10;
    }

    (score.min(100), suspected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selectel_asn_detection() {
        let ip = IpAddr::V4(Ipv4Addr::new(95, 213, 10, 5));
        let hint = detect_asn_hint(&ip);
        assert!(hint.is_some());
        let (asn, _, entity) = hint.unwrap();
        assert_eq!(asn, "AS49505");
        assert!(entity.contains("Selectel"));
    }

    #[test]
    fn test_datacamp_asn_detection() {
        let ip = IpAddr::V4(Ipv4Addr::new(185, 220, 101, 42));
        let hint = detect_asn_hint(&ip);
        assert!(hint.is_some());
        let (asn, _, entity) = hint.unwrap();
        assert_eq!(asn, "AS60068");
        assert!(entity.contains("DataCamp"));
    }

    #[test]
    fn test_threat_scoring() {
        let entry = SurveillanceNodeEntry {
            query_count: 55,
            bep42_violations: 55,
            bep42_valid: 0,
            sample_hashes: vec![[1u8; 20], [2u8; 20], [3u8; 20], [4u8; 20]],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
        };
        let hint = Some(("AS49505", "Selectel", "IKWYD Spying Node (Selectel)"));
        let (score, entity) = calculate_threat_score(&entry, hint);
        assert_eq!(score, 100);
        assert!(entity.contains("Selectel"));
    }
}
