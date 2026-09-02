# ConnLimiter Memory Leak Fix

## Problem

`ConnLimiter` (`apps/crawler/src/verify/mod.rs:80-101`) stores a `DashMap<IpAddr, Arc<Semaphore>>` that grows unboundedly — every unique peer IP creates a permanent entry. At 65k verifies/hr, this leaks ~500 MB/day and will eventually OOM after 3-5 days.

## Root Cause

No TTL, no max_entries, no sweep. The `acquire()` method always inserts new entries and never removes them.

## Design

Follow the existing `PeerCache` pattern (TTL + max_entries + periodic sweep).

### 1. Add TTL and max_entries to ConnLimiter

```rust
pub struct ConnLimiter {
    inner: dashmap::DashMap<std::net::IpAddr, (Arc<tokio::sync::Semaphore>, Instant)>,
    permits: usize,
    ttl: Duration,
    max_entries: usize,
}
```

### 2. Modify acquire() to record timestamp

```rust
pub async fn acquire(&self, ip: std::net::IpAddr) -> OwnedSemaphorePermit {
    let mut entry = self
        .inner
        .entry(ip)
        .or_insert_with(|| (Arc::new(Semaphore::new(self.permits)), Instant::now()));
    entry.1 = Instant::now(); // touch
    let sem = entry.clone().0;
    self.enforce_bound();
    sem.acquire_owned().await.expect("conn limiter closed")
}
```

### 3. Add evict_expired() method

Use `retain()` for efficient bulk cleanup — remove entries where `last_access.elapsed() > TTL`:

```rust
pub fn evict_expired(&self) -> usize {
    let now = Instant::now();
    let mut evicted = 0;
    self.inner.retain(|_, (_, last_seen)| {
        if now.duration_since(*last_seen) >= self.ttl {
            evicted += 1;
            false
        } else {
            true
        }
    });
    evicted
}
```

### 4. Add enforce_bound() method

Same pattern as PeerCache — try eviction first, then remove oldest 1/8 excess:

```rust
fn enforce_bound(&self) {
    if self.inner.len() <= self.max_entries { return; }
    let _ = self.evict_expired();
    if self.inner.len() <= self.max_entries { return; }
    let excess = self.inner.len() - self.max_entries;
    let target_remove = (excess / 8).max(1);
    // ... remove oldest entries
}
```

### 5. Add config keys

In `config.rs`, `FetchConfig`:

```rust
pub struct FetchConfig {
    // ... existing fields
    pub conn_limiter_ttl_secs: u64,      // default: 60
    pub conn_limiter_max_entries: usize, // default: 1_000_000
}
```

### 6. Update default.toml

```toml
conn_limiter_ttl_secs = 60
conn_limiter_max_entries = 1_000_000
```

### 7. Wire into main.rs

Pass `ttl` and `max_entries` to `ConnLimiter::new()`.

## Rationale

- **TTL=60s**: A peer IP is only relevant for the duration of its connect attempt (TCP/utp timeout is 5s + metadata timeout is 25s = 30s max). 60s provides a 2x safety margin.
- **max_entries=1M**: At ~520K unique IPs/hr, this allows ~2 hours of backlog before aggressive eviction kicks in. Combined with 60s TTL, steady-state is ~10K entries.
- **Touch on acquire**: Active IPs (multi-port seedboxes hit repeatedly) stay alive; idle IPs expire naturally.
- **enforce_bound()**: Called on every `acquire()` — `DashMap::len()` is O(n_shards) but DashMap has 64 shards by default, so this is fast. The 1/8 removal keeps amortized cost low.

## Files to Modify

| File | Change |
|---|---|
| `apps/crawler/src/verify/mod.rs` | Rewrite `ConnLimiter` struct, add `evict_expired()`, `enforce_bound()` |
| `apps/crawler/src/config.rs` | Add `conn_limiter_ttl_secs`, `conn_limiter_max_entries` |
| `apps/crawler/src/default.toml` | Add defaults |
| `apps/crawler/src/main.rs` | Pass new config to `ConnLimiter::new()` |
| `deploy/targets/gaia-node/.env` | Add `CRAW_CONN_LIMITER_TTL_SECS`, `CRAW_CONN_LIMITER_MAX_ENTRIES` |
| `deploy/targets/gaia-node/docker-compose.yml` | Pass new env vars |
| `apps/crawler/src/verify/fetch_pool.rs` | Update test `ConnLimiter::new()` calls |

## Tests

- Unit test: entries expire after TTL
- Unit test: `enforce_bound()` removes excess entries
- Unit test: `acquire()` updates timestamp on re-use
- Unit test: bounded growth under rapid unique IPs

## Rollback

Revert the commit. The old unbounded ConnLimiter will be restored. Memory leak returns but system works.
