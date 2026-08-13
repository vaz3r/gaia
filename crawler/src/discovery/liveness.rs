//! Shared cross-loop liveness counter.
//!
//! Tracks, per infohash, *which distinct DHT nodes* reported it and *when*.
//! A sampled hash is only emitted to the fetcher once enough distinct sources
//! corroborated it within a rolling window — this is the "liveness gate" that
//! culls the ~99% dead-hash tail before any TCP fetch is spent.
//!
//! Design notes (see `openspec/changes/crawler-liveness-gate`):
//! - **Upsert by source**: reports are keyed by source node ID; a node
//!   reporting the same hash again updates its timestamp in place and never
//!   consumes a new slot. This prevents a chatty node from crowding distinct
//!   competitors out of the per-hash list (the ring-crowding bug).
//! - **Cap = distinct sources** (8): a genuinely-new 9th source evicts the
//!   oldest distinct source. Rarely fires at min_seen=3; matters under shadow
//!   accumulation and adversarial high-fanout.
//! - **Per-report expiry**: each report carries its own `Instant`; reports
//!   older than the window are dropped on encounter, and a hash whose reports
//!   all expire is removed.
//! - **Entry lifetime = max(min_seen, min_seen_shadow)**: live emission does
//!   not delete an entry that shadow mode still needs to observe accumulating.
//! - **Global backstop**: `max_entries` (100k) with a periodic sweep evicts
//!   one-hit-wonders that are never re-read, so the steady-state footprint
//!   (~2,900 entries ≈ 0.25-0.9 MB/process) cannot drift upward.

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use gaia_core::Id20;
use smallvec::SmallVec;

/// Per-hash report list. Inline capacity 4 is a perf detail; the semantic cap
/// is `LivenessConfig.cap` distinct sources (default 8).
pub type Reports = SmallVec<[(Id20, Instant); 4]>;

/// Liveness counter configuration.
#[derive(Debug, Clone, Copy)]
pub struct LivenessConfig {
    /// Rolling window; a report older than this expires on encounter.
    pub window: Duration,
    /// Max distinct sources tracked per hash.
    pub cap: usize,
    /// Global entry backstop; oldest entries are evicted past this.
    pub max_entries: usize,
}

/// One hash's live report list (DashMap value). Reports are upserted by source
/// node ID, so the vec holds at most one entry per distinct source.
#[derive(Debug, Clone)]
pub struct Entry {
    reports: Reports,
    /// Total report events (including same-source refreshes), for
    /// discriminating backoff-stalled sources from genuinely sparse hashes at
    /// expiry time.
    sightings: u32,
}

impl Entry {
    fn new(source: Id20, now: Instant) -> Self {
        let mut reports = Reports::new();
        reports.push((source, now));
        Self { reports, sightings: 1 }
    }

    /// Distinct sources with a report within `window` of `now`.
    fn live_count(&self, window: Duration, now: Instant) -> usize {
        self.reports
            .iter()
            .filter(|(_, t)| now.duration_since(*t) <= window)
            .count()
    }

    fn oldest_source(&self) -> usize {
        self.reports
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, t))| *t)
            .map(|(i, _)| i)
            .expect("non-empty reports")
    }

    fn max_reached(&self) -> usize {
        self.reports.len()
    }
}

/// What a `record` call did, for the sampler's emit/shadow logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordOutcome {
    /// This hash is newly known to the counter (first-ever report).
    New,
    /// The hash exists but this report didn't change its distinct-source count
    /// (a repeat from an already-known source, or an expired report).
    Repeat,
    /// A genuinely-new distinct source was added; total distinct count is `n`.
    Gained { distinct: usize },
}

/// Outcome of the periodic sweep, for shadow near-miss accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepEviction {
    pub hash: [u8; 20],
    /// Max distinct sources this entry ever reached (pre-prune).
    pub max_sources: usize,
    /// Total report events (including same-source refreshes). A count-1 near
    /// miss with sightings == max_sources == 1 means the sole source never
    /// refreshed — consistent with a backoff-stalled node; sightings > 1 means
    /// the source kept being re-queried (plain sparsity, not backoff).
    pub sightings: u32,
}

/// Shared liveness counter (one per process, cloned across sampler loops).
#[derive(Debug)]
pub struct LivenessCounter {
    inner: DashMap<[u8; 20], Entry>,
    cfg: LivenessConfig,
}

impl LivenessCounter {
    pub fn new(cfg: LivenessConfig) -> Arc<Self> {
        Arc::new(Self {
            inner: DashMap::new(),
            cfg,
        })
    }

    /// Record that `source` reported `hash` at `now`. Prunes expired reports
    /// for this hash, upserts the source (or adds it if new and under cap),
    /// and enforces the per-hash cap. The *caller* decides entry removal after
    /// a live or shadow emission.
    pub fn record(&self, hash: &[u8; 20], source: Id20, now: Instant) -> RecordOutcome {
        let window = self.cfg.window;

        // Brand-new hash: insert and report New.
        if !self.inner.contains_key(hash) {
            self.inner.insert(*hash, Entry::new(source, now));
            return RecordOutcome::New;
        }

        let mut e = self
            .inner
            .get_mut(hash)
            .expect("contains_key checked above");
        e.sightings = e.sightings.saturating_add(1);

        // Prune expired reports (each report has its own timestamp).
        e.reports.retain(|(_, t)| now.duration_since(*t) <= window);

        // Upsert by source: a repeat updates its timestamp, never a new slot.
        if let Some(slot) = e.reports.iter_mut().find(|(id, _)| *id == source) {
            slot.1 = now;
            return if e.reports.is_empty() { RecordOutcome::New } else { RecordOutcome::Repeat };
        }

        // New distinct source. Enforce cap: evict the oldest distinct source.
        if e.reports.len() >= self.cfg.cap {
            let oldest = e.oldest_source();
            e.reports[oldest] = (source, now);
            return RecordOutcome::Gained { distinct: e.live_count(window, now) };
        }

        e.reports.push((source, now));
        RecordOutcome::Gained {
            distinct: e.live_count(window, now),
        }
    }

    /// Distinct-source count for `hash` within the window (for shadow checks
    /// after a report that didn't add a source).
    pub fn live_count(&self, hash: &[u8; 20], now: Instant) -> usize {
        match self.inner.get(hash) {
            Some(e) => e.live_count(self.cfg.window, now),
            None => 0,
        }
    }

    /// Total report events for `hash` within the window, including same-source
    /// refreshes (the sparse/stalled discriminator: sightings ≥ 2 with a single
    /// distinct source means the source kept re-reporting it = live; sightings
    /// == 1 means the source reported once and never refreshed = dead).
    pub fn live_sightings(&self, hash: &[u8; 20], _now: Instant) -> u32 {
        match self.inner.get(hash) {
            Some(e) => e.sightings,
            None => 0,
        }
    }

    /// Remove a hash entry (after it was emitted or shadow-emitted).
    pub fn remove(&self, hash: &[u8; 20]) {
        self.inner.remove(hash);
    }

    /// Expire entries whose reports are all outside the window, and enforce the
    /// global backstop. Returns evictions for shadow near-miss accounting.
    pub fn sweep(&self, now: Instant) -> Vec<SweepEviction> {
        let window = self.cfg.window;
        let mut evicted = Vec::new();

        // Per-report expiry + record max sources before removal. Snapshot the
        // distinct-source count BEFORE pruning: `max_reached()` reflects the
        // current reports length, which is 0 after all reports expired — so the
        // near-miss bucket must use the pre-prune count.
        self.inner.retain(|hash, entry| {
            let pre_prune = entry.reports.len();
            let sightings = entry.sightings;
            entry.reports.retain(|(_, t)| now.duration_since(*t) <= window);
            if entry.reports.is_empty() {
                evicted.push(SweepEviction {
                    hash: *hash,
                    max_sources: pre_prune,
                    sightings,
                });
                false
            } else {
                true
            }
        });

        // Global backstop: evict oldest entries past max_entries.
        if self.inner.len() > self.cfg.max_entries {
            let overflow = self.inner.len() - self.cfg.max_entries;
            // Collect keys ordered by oldest report, evict the overflow count.
            let mut keys: Vec<([u8; 20], Instant)> = self
                .inner
                .iter()
                .map(|e| (*e.key(), e.value().oldest_time()))
                .collect();
            keys.sort_by_key(|(_, t)| *t);
            for (hash, _) in keys.into_iter().take(overflow) {
                if let Some((_, entry)) = self.inner.remove(&hash) {
                    evicted.push(SweepEviction {
                        hash,
                        max_sources: entry.max_reached(),
                        sightings: entry.sightings,
                    });
                }
            }
        }

        evicted
    }

    /// Number of tracked hashes (diagnostic).
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

impl Entry {
    fn oldest_time(&self) -> Instant {
        self.reports
            .iter()
            .map(|(_, t)| *t)
            .min()
            .expect("non-empty reports")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> Id20 {
        Id20([n; 20])
    }

    fn cfg() -> LivenessConfig {
        LivenessConfig {
            window: Duration::from_secs(120),
            cap: 8,
            max_entries: 100_000,
        }
    }

    #[test]
    fn upsert_by_source_does_not_inflate_count() {
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        let hash = [1u8; 20];
        // A single source reports the hash 5x in quick succession — the first
        // is New, the rest are Repeat (upsert by source, never new slots).
        let first = lc.record(&hash, id(1), now);
        assert_eq!(first, RecordOutcome::New);
        for i in 1..5u8 {
            let o = lc.record(&hash, id(1), now + Duration::from_millis(i as u64));
            assert_eq!(o, RecordOutcome::Repeat, "same source must not add a slot");
        }
        assert_eq!(lc.live_count(&hash, now + Duration::from_secs(1)), 1);
        // A second distinct source adds exactly one.
        let o = lc.record(&hash, id(2), now + Duration::from_secs(1));
        assert_eq!(o, RecordOutcome::Gained { distinct: 2 });
        assert_eq!(lc.live_count(&hash, now + Duration::from_secs(1)), 2);
    }

    #[test]
    fn same_source_repeat_updates_in_place() {
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        let hash = [1u8; 20];
        lc.record(&hash, id(1), now);
        let o = lc.record(&hash, id(1), now + Duration::from_secs(10));
        assert_eq!(o, RecordOutcome::Repeat, "same source must not add a slot");
        assert_eq!(lc.live_count(&hash, now + Duration::from_secs(10)), 1);
    }

    #[test]
    fn distinct_sources_accumulate() {
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        let hash = [1u8; 20];
        lc.record(&hash, id(1), now);
        lc.record(&hash, id(2), now + Duration::from_secs(1));
        lc.record(&hash, id(3), now + Duration::from_secs(2));
        assert_eq!(lc.live_count(&hash, now + Duration::from_secs(2)), 3);
    }

    #[test]
    fn window_expiry_drops_old_reports() {
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        let hash = [1u8; 20];
        lc.record(&hash, id(1), now);
        lc.record(&hash, id(2), now + Duration::from_secs(1));
        // After the window, both expire.
        assert_eq!(
            lc.live_count(&hash, now + Duration::from_secs(200)),
            0,
            "reports older than the window must not count"
        );
    }

    #[test]
    fn cap_evicts_oldest_distinct_source() {
        let cfg = LivenessConfig {
            window: Duration::from_secs(120),
            cap: 3,
            max_entries: 100_000,
        };
        let lc = LivenessCounter::new(cfg);
        let now = Instant::now();
        let hash = [1u8; 20];
        lc.record(&hash, id(1), now);
        lc.record(&hash, id(2), now + Duration::from_secs(1));
        lc.record(&hash, id(3), now + Duration::from_secs(2));
        // 4th distinct source evicts the oldest (id 1).
        let o = lc.record(&hash, id(4), now + Duration::from_secs(3));
        assert_eq!(o, RecordOutcome::Gained { distinct: 3 });
        assert_eq!(lc.live_count(&hash, now + Duration::from_secs(3)), 3);
    }

    #[test]
    fn sweep_evicts_expired_one_hit_wonders() {
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        lc.record(&[1u8; 20], id(1), now);
        lc.record(&[2u8; 20], id(1), now);
        let evicted = lc.sweep(now + Duration::from_secs(200));
        assert_eq!(evicted.len(), 2, "both one-hit-wonders expired");
        assert_eq!(lc.len(), 0);
    }

    #[test]
    fn sweep_reports_pre_prune_distinct_count() {
        // Regression: `max_reached()` reflects the current reports length, which
        // is 0 after all reports expired during pruning. The sweep must report
        // the pre-prune distinct-source count so near-miss buckets work.
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        let hash = [1u8; 20];
        lc.record(&hash, id(1), now);
        lc.record(&hash, id(2), now + Duration::from_secs(1));
        // Both reports expire; sweep must report max_sources == 2 and
        // sightings == 2.
        let evicted = lc.sweep(now + Duration::from_secs(200));
        assert_eq!(evicted.len(), 1);
        assert_eq!(
            evicted[0].max_sources, 2,
            "sweep must report the pre-prune distinct count, got {}",
            evicted[0].max_sources
        );
        assert_eq!(evicted[0].sightings, 2, "two reports recorded");
    }

    #[test]
    fn same_source_refresh_increments_sightings_not_distinct() {
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        let hash = [1u8; 20];
        lc.record(&hash, id(1), now);
        // A same-source refresh increments sightings but not distinct count.
        lc.record(&hash, id(1), now + Duration::from_secs(10));
        lc.record(&hash, id(1), now + Duration::from_secs(20));
        let evicted = lc.sweep(now + Duration::from_secs(200));
        assert_eq!(evicted.len(), 1);
        assert_eq!(evicted[0].max_sources, 1, "still one distinct source");
        assert_eq!(
            evicted[0].sightings, 3,
            "three reports from one source (refresh, not new source)"
        );
    }

    #[test]
    fn backstop_evicts_oldest_past_max_entries() {
        let cfg = LivenessConfig {
            window: Duration::from_secs(120),
            cap: 8,
            max_entries: 3,
        };
        let lc = LivenessCounter::new(cfg);
        let now = Instant::now();
        for i in 0..6u8 {
            let mut h = [0u8; 20];
            h[0] = i;
            lc.record(&h, id(1), now + Duration::from_millis(i as u64));
        }
        let evicted = lc.sweep(now + Duration::from_secs(1));
        assert!(
            lc.len() <= 3,
            "backstop must cap entries, got {}",
            lc.len()
        );
        assert!(!evicted.is_empty());
    }

    #[test]
    fn entry_accumulates_across_reports_without_auto_remove() {
        // Entry removal is the caller's job (live emit must not delete an entry
        // shadow mode still needs to observe). The counter itself just keeps
        // accumulating distinct sources until removed or expired.
        let lc = LivenessCounter::new(cfg());
        let now = Instant::now();
        let hash = [1u8; 20];
        lc.record(&hash, id(1), now);
        lc.record(&hash, id(2), now + Duration::from_secs(1));
        lc.record(&hash, id(3), now + Duration::from_secs(2));
        assert_eq!(
            lc.live_count(&hash, now + Duration::from_secs(2)),
            3,
            "entry survives repeated reports until the caller removes it"
        );
        lc.remove(&hash);
        assert_eq!(lc.live_count(&hash, now + Duration::from_secs(3)), 0, "removed");
    }
}
