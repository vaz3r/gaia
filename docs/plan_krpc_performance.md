## Goal Description
Before we implement Phase 3 (Sybil Weaponization), we need to ensure the crawler's networking layer can handle a massive 10x influx of UDP traffic without buckling the CPU. 

Currently, the crawler parses every single incoming UDP packet into a full Abstract Syntax Tree (`BValue`) before figuring out what to do with it. This involves allocating heavy `BTreeMap` and `Vec` objects on the heap for tens of thousands of packets per second. Because this is done entirely in the single UDP worker thread, it creates a massive CPU bottleneck. 

This plan implements a Zero-Allocation Fast Scanner to bypass AST parsing for dropped packets, and shifts the remaining AST parsing off the networking thread and onto the parallel Tokio worker threads.

## User Review Required
> [!IMPORTANT]
> This plan changes the core KRPC networking pipeline. It will require modifying how `TxEntry` channels work (they will pass raw `Bytes` instead of fully parsed `Message` objects). This is safe, but it touches the hottest path in the crawler.

## Proposed Changes
---
### `apps/crawler/src/krpc/scanner.rs`
#### [NEW] `scanner.rs`
We will implement a small, hand-rolled, zero-allocation bencode scanner that scans the raw `&[u8]` UDP payload in a single pass. It will extract references to `t` (Transaction ID), `y` (Type), and `q` (Query Name) without allocating a single byte of heap memory.

---
### `apps/crawler/src/router.rs`
#### [MODIFY] `router.rs`
We will update `handle_datagram` to run the zero-allocation scanner *first*, before calling `Bytes::copy_from_slice` or `Message::parse`:

1. **Fast-Path `find_node` Drops:** If the scanner detects `y == b"q"` and `q == b"find_node"`, we will roll the 95% drop chance immediately. If it drops, we return instantly. This completely bypasses all heap allocations and AST parsing for the majority of incoming traffic.
2. **Offload Reply Parsing:** If the scanner detects `y == b"r"` or `y == b"e"`, we extract `t`. We look up the transaction in `self.tx`. If a worker is waiting, we send them the unparsed raw `Bytes` payload, bypassing `Message::parse` entirely on the UDP thread.

---
### `apps/crawler/src/krpc/tx_state.rs`
#### [MODIFY] `tx_state.rs`
We will change `TxEntry::reply` from `oneshot::Sender<Message>` to `oneshot::Sender<Bytes>`. This allows the UDP thread to just hand off the raw buffer to the worker.

---
### `apps/crawler/src/dht/walker.rs` & `fetch_peer.rs`
#### [MODIFY] `walker.rs` and `apps/crawler/src/verify/peer_source.rs`
We will update the callers that wait on the `oneshot::Receiver` to accept the raw `Bytes` payload. Once received, these parallel worker threads will call `Message::parse` themselves. Because these threads are scheduled across all available CPU cores by Tokio, this offloads the heavy AST parsing from the single networking thread to the multi-core threadpool.

## Verification Plan
### Automated Tests
- Run `cargo test -p crawler` to ensure the KRPC parser and wire mocks still pass.
- Write a unit test for `scanner::scan` to verify it correctly extracts fields from malformed and valid bencode packets without panicking.

### Manual Verification
- Deploy to `gaia`.
- Run `./deploy/scripts/health.sh --window 15` after deployment. We should see `verify_success` rates matching or exceeding previous levels, while observing `top` or `docker stats` show a significant drop in CPU usage.
