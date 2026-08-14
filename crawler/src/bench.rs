//! Offline benchmark harness for fetch/peer-resolution strategies.
//!
//! Reads a DB snapshot (see the `snapshot` command) that contains millions of
//! fetch attempts with recorded outcomes. Samples hashes by outcome class
//! (e.g. previously-failed `empty_peers`, previously-verified `ok`) and replays
//! a strategy against them — measuring how many now resolve peers and, when
//! `--verify`, how many actually yield verified metadata. This lets us iterate
//! on peer-resolution strategies in minutes instead of deploy cycles.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use gaia_core::Id20;
use sqlx::{PgPool, Row};

use crate::cli::BenchFetchArgs;
use crate::fetch::tracker;
use crate::fetch::wire::{self, sha1_info};

/// Run the bench: sample `sample` hashes per outcome class, resolve peers via
/// trackers, optionally verify by dialing, and print per-class stats.
pub async fn run(args: &BenchFetchArgs) -> Result<()> {
    let pool = PgPool::connect(&args.pg)
        .await
        .with_context(|| format!("connect postgres {}", args.pg))?;

    let (where_clause, label) = class_filter(&args.class);
    // Sample distinct hashes (a hash can appear with multiple outcomes).
    let sql = format!(
        "SELECT DISTINCT info_hash FROM scanned WHERE {where_clause} LIMIT $1"
    );
    let rows = sqlx::query(&sql)
        .bind(args.sample as i64)
        .fetch_all(&pool)
        .await
        .context("sample hashes from scanned")?;
    let mut hashes: Vec<[u8; 20]> = Vec::new();
    for row in rows {
        let bytes: Vec<u8> = row.get(0);
        if bytes.len() == 20 {
            let mut h = [0u8; 20];
            h.copy_from_slice(&bytes);
            hashes.push(h);
        }
    }
    println!(
        "bench: class={label} sampled={} (pg={}) concurrency={} verify={}",
        hashes.len(),
        args.pg,
        args.concurrency,
        args.verify
    );

    // Process concurrently, but the tracker client spawns its own tasks; we
    // bound our own in-flight probes.
    let sem = Arc::new(tokio::sync::Semaphore::new(args.concurrency.max(1)));
    let mut tasks = tokio::task::JoinSet::new();
    let start = Instant::now();

    // Statistics accumulate via atomics.
    let stats = Arc::new(BenchStats::default());
    let verify = args.verify;
    for hash in hashes {
        let stats = stats.clone();
        let sem = sem.clone();
        tasks.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore");
            probe_one(hash, verify, &stats).await;
        });
    }
    while tasks.join_next().await.is_some() {}

    let elapsed = start.elapsed();
    let s = &*stats;
    println!();
    println!("=== results ({:.1}s) ===", elapsed.as_secs_f64());
    println!(
        "  peers_found      : {:>5}  ({:.1}% of sampled)",
        s.peers_found.load(Relaxed),
        pct(s.peers_found.load(Relaxed), s.total.load(Relaxed))
    );
    println!(
        "  peers_empty      : {:>5}  (tracker returned no peers)",
        s.peers_empty.load(Relaxed)
    );
    println!(
        "  tracker_errors   : {:>5}  (resolution failed/timeout)",
        s.tracker_errors.load(Relaxed)
    );
    if verify {
        println!(
            "  verified         : {:>5}  ({:.1}% of sampled)",
            s.verified.load(Relaxed),
            pct(s.verified.load(Relaxed), s.total.load(Relaxed))
        );
        println!(
            "  sha1_mismatch    : {:>5}",
            s.sha1_mismatch.load(Relaxed)
        );
        let guard = s.failure_reasons.lock().unwrap();
        let mut reasons: Vec<_> = guard.iter().collect();
        reasons.sort_by_key(|(_, c)| std::cmp::Reverse(**c));
        println!("  dial failure mix:");
        for (reason, count) in reasons.iter().take(8) {
            println!("    {:<22} {}", reason, count);
        }
    }
    println!();

    Ok(())
}

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;

use std::sync::Mutex;

#[derive(Default)]
struct BenchStats {
    total: AtomicU64,
    peers_found: AtomicU64,
    peers_empty: AtomicU64,
    tracker_errors: AtomicU64,
    verified: AtomicU64,
    sha1_mismatch: AtomicU64,
    failure_reasons: Mutex<HashMap<String, u64>>,
}

fn pct(n: u64, d: u64) -> f64 {
    if d == 0 { 0.0 } else { n as f64 / d as f64 * 100.0 }
}

/// Resolve peers via trackers; if `verify`, dial them and record outcomes.
async fn probe_one(hash: [u8; 20], verify: bool, stats: &Arc<BenchStats>) {
    stats.total.fetch_add(1, Relaxed);

    let peers = tracker::resolve_peers_from_trackers(&hash).await;
    if peers.is_empty() {
        stats.peers_empty.fetch_add(1, Relaxed);
        return;
    }
    stats.peers_found.fetch_add(1, Relaxed);

    if !verify {
        return;
    }

    // Dial a bounded subset of the resolved peers in parallel; the first
    // SHA-1-verified metadata wins. Mirrors the production dial logic minus
    // the DHT lookup (tracker peers are the only source here).
    let info_hash = Id20(hash);
    let mut peer_id_bytes = [0u8; 20];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut peer_id_bytes);
    let peer_id = Id20(peer_id_bytes);
    let mut tasks = tokio::task::JoinSet::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let dial_n = std::env::var("BENCH_DIAL").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    for peer in peers.iter().take(dial_n) {
        let peer = *peer;
        let tx = tx.clone();
        tasks.spawn(async move {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                wire::fetch_from_peer(peer, info_hash, peer_id),
            )
            .await;
            let _ = tx.send((peer, result));
        });
    }
    drop(tx);
    let mut any_verified = false;
    while let Some((_peer, result)) = rx.recv().await {
        match result {
            Ok(Ok(meta)) => {
                if sha1_info(&meta.info_bytes) == hash {
                    any_verified = true;
                    break;
                }
            }
            Ok(Err(e)) => {
                use crate::fetch::failure::FetchFailureKind;
                let kind = FetchFailureKind::from_error(&e).as_str().to_string();
                *stats.failure_reasons.lock().unwrap().entry(kind).or_insert(0) += 1;
            }
            Err(_elapsed) => {
                *stats.failure_reasons.lock().unwrap().entry("timeout".into()).or_insert(0) += 1;
            }
        }
    }
    tasks.abort_all();
    if any_verified {
        stats.verified.fetch_add(1, Relaxed);
    }
}

fn class_filter(class: &str) -> (String, &'static str) {
    match class {
        "empty_peers" => ("status='failed' AND failure_reason='empty_peers'".into(), "empty_peers"),
        "timeout" => ("status='failed' AND failure_reason='timeout'".into(), "timeout"),
        "other" => ("status='failed' AND failure_reason='other'".into(), "other"),
        "deadline" => ("status='failed' AND failure_reason='deadline'".into(), "deadline"),
        "ok" => ("status='ok'".into(), "ok (previously verified)"),
        "all" => ("1=1".into(), "all"),
        _ => ("status='failed' AND failure_reason='empty_peers'".into(), "empty_peers"),
    }
}
