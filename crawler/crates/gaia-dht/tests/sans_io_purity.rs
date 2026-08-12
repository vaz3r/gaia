//! M264: lock the pure DHT iterative-lookup core against I/O creep (gap review
//! §6 — "hold the line on the existing purity islands"). `lookup.rs` holds
//! `IterativeLookup<C>`, the pure Kademlia lookup state machine
//! (`next_to_query` / `feed_nodes` / `is_exhausted`, zero tokio). The DHT lookup
//! *actor* shell (`dht_lookup.rs`) is async by design — §6 declines extracting
//! it, so it is deliberately NOT guarded here.
//!
//! Fail-loud substring tripwire over the file source. If a legitimate future
//! import trips a token, narrow the token here rather than deleting it.

/// Substrings that signal async I/O has crept into a sans-IO module.
const FORBIDDEN: &[&str] = &[
    "use tokio",
    "tokio::",
    ".await",
    "async fn",
    "async move",
    "UdpSocket",
    "TcpStream",
    "std::fs",
];

/// Return the forbidden tokens present in `src` (empty = pure).
fn impurities(src: &str) -> Vec<&'static str> {
    FORBIDDEN
        .iter()
        .copied()
        .filter(|tok| src.contains(tok))
        .collect()
}

#[test]
fn guard_helper_catches_planted_violation() {
    let dirty = "async fn query(&self) { self.socket.recv_from(&mut buf).await; }";
    assert!(impurities(dirty).contains(&"async fn"));
    assert!(impurities(dirty).contains(&".await"));
    let clean = "fn next_to_query(&mut self) -> Option<NodeId> { None }";
    assert!(impurities(clean).is_empty());
}

#[test]
fn lookup_rs_is_pure() {
    let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lookup.rs"));
    let found = impurities(src);
    assert!(found.is_empty(), "lookup.rs gained async I/O: {found:?}");
}
