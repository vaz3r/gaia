//! Active retry worker.
//!
//! The sampler only retries a failed hash if the DHT happens to re-report it
//! (opportunistic). This worker actively drains retry-eligible hashes from
//! `scanned` — failed, past `next_attempt`, and under their per-class retry
//! cap — and re-emits them into the fetch pipeline so transient infrastructure
//! failures (timeout, deadline, dht_lookup_failed, ...) get a second chance
//! promptly instead of waiting for a re-report.
//!
//! The worker owns its own concurrency semaphore (isolated from the fresh-fetch
//! pool) and shares the `in_flight` set with `run_fetcher` so a hash already
//! being fetched is never re-queued.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gaia_core::Id20;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::discovery::{FetchRequest, FetchSource};
use crate::stats::CrawlStats;
use crate::storage::Storage;

/// How often the worker scans for retry-eligible hashes.
pub const RETRY_SCAN_INTERVAL: Duration = Duration::from_secs(30);
/// Max hashes selected per scan.
pub const RETRY_BATCH: i64 = 256;
/// After this age (seconds), a terminal `failed` hash gets its attempt budget
/// reset so it can be retried again (it may have gained seeders since). Runs on
/// a slow cadence (RE_EVAL_EVERY scans) so it does not re-flood the queue.
pub const REEVAL_MIN_AGE_SECS: i64 = 12 * 3600;
const RE_EVAL_EVERY: u64 = 20; // 20 * 30s ≈ every 10 minutes

/// Run the retry worker until shutdown. `max_attempts` is the transient-class
/// budget (dead-verdict classes are always capped at 2 by `retry_eligible`).
pub async fn run_retry_worker(
    storage: Storage,
    emit: mpsc::Sender<FetchRequest>,
    semaphore: Arc<Semaphore>,
    in_flight: Arc<std::sync::Mutex<std::collections::HashSet<Id20>>>,
    stats: Arc<CrawlStats>,
    max_attempts: u32,
    shutdown: CancellationToken,
) {
    let mut tick = tokio::time::interval(RETRY_SCAN_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut scan_count: u64 = 0;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tick.tick() => {}
        }

        let now = unix_secs();
        // On a slow cadence, reset the attempt budget of terminal hashes whose
        // last attempt is long past, so long-dead torrents that gained seeders
        // get a fresh chance instead of being dropped forever.
        scan_count += 1;
        if scan_count.is_multiple_of(RE_EVAL_EVERY) {
            match storage.reevaluate_terminal_hashes(now, REEVAL_MIN_AGE_SECS).await {
                Ok(n) => {
                    if n > 0 {
                        tracing::debug!(reset = n, "re-evaluated terminal hashes");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "re-evaluate terminal hashes failed"),
            }
        }

        let eligible = match storage.retry_eligible(now, max_attempts, RETRY_BATCH).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "retry worker eligibility query failed");
                continue;
            }
        };
        stats
            .retry_worker_scans
            .fetch_add(1, Ordering::Relaxed);

        if eligible.is_empty() {
            continue;
        }
        tracing::debug!(eligible = eligible.len(), "retry worker scan");

        for hash_bytes in eligible {
            let hash = Id20(hash_bytes);
            // Skip hashes already being fetched.
            if in_flight.lock().unwrap().contains(&hash) {
                continue;
            }
            // Bound how many retries are in flight at once.
            let permit = semaphore.acquire().await;
            let Ok(permit) = permit else { return };
            let ok = emit
                .send(FetchRequest {
                    hash,
                    occurrences: 1,
                    peer_hint: None,
                    source: FetchSource::Retried,
                    lookup_seed: None,
                    dht_handle: None,
                })
                .await;
            drop(permit);
            if ok.is_err() {
                // Pipeline closed (shutdown).
                return;
            }
        }
    }
}

fn unix_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
