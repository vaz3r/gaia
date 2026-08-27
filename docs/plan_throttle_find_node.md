## Goal Description
The `find_node` flood (~57,500 queries/sec) is driving massive CPU usage and outbound amplification. By responding to all 57k/s `find_node` requests with 8 nodes each, we burn CPU encoding Bencode, taking routing table locks, and wasting outbound bandwidth.

Instead of dropping them entirely (which might cause the network to treat us as completely dead) or using a per-IP rate limiter (which requires locking and memory tracking), we will use a **configurable lock-free probabilistic drop (percentage-based)**.

This approach is mathematically flat: if we set the response rate to 5%, we instantly drop 95% of the flood with near-zero CPU overhead, while still answering enough random requests globally to stay alive in remote routing tables.

## User Review Required
> [!TIP]
> **Throttling Policy:**
> You will be able to configure `CRAW_FIND_NODE_RESPONSE_PERCENT=5` (to answer 5% of queries). If you ever want to answer all of them, set it to `100`. 

## Proposed Changes

### `apps/crawler/src/config.rs`
Introduce a percentage configuration for `find_node` responses.
#### [MODIFY] `apps/crawler/src/config.rs`
```rust
pub struct DhtConfig {
    // ... existing ...
    pub find_node_response_percent: u8,
}
// Default to 100 in Config::default(), but configurable via CRAW_FIND_NODE_RESPONSE_PERCENT
```

### `apps/crawler/src/router.rs`
In `respond_find_node`, generate a random number between 0-99. If it is greater than or equal to `find_node_response_percent`, return immediately.

#### [MODIFY] `apps/crawler/src/router.rs`
```rust
impl Router {
    fn respond_find_node(&self, t: &Bytes, a: &BValue, from: SocketAddr, config_percent: u8) {
        if config_percent < 100 {
            // Fast, lock-free thread-local random
            let roll = rand::random::<u8>() % 100;
            if roll >= config_percent {
                self.metrics.inbound_find_node_dropped.add(1);
                return;
            }
        }
        
        let target = match extract_id20(a, b"target") {
            Some(t) => t,
            None => return,
        };
        // ... (existing response logic)
    }
}
```
*(Note: `config_percent` will be passed down from `Router::new` or read from a shared config).*

### `apps/crawler/src/main.rs` & `apps/crawler/src/metrics.rs`
1. Add `inbound_find_node_dropped` to `Metrics`.
2. Pass the `find_node_response_percent` into the `Router` instantiation.

## Verification Plan

### Automated Tests
Run `cargo clippy` and `cargo test`.

### Manual Verification
1. Set `CRAW_FIND_NODE_RESPONSE_PERCENT=5` in `.env`.
2. Deploy to `zerone`.
3. Check `health.sh`: `inbound_find_node_dropped` should roughly equal 95% of the total `inbound_find_node` rate.
4. Check `top`: The crawler CPU should see a massive drop since we skip serialization and sending for 95% of queries.
