//! M264: lock the pure wire codec model against I/O creep (gap review §6 —
//! "hold the line on the existing purity islands"). `message.rs` holds the pure
//! `Message<B>` codec types (only `bytes` + `crate::error`); the async-typed
//! sink lives in `codec.rs` (which deps `tokio_util::codec`) and is deliberately
//! NOT guarded here — it is the I/O boundary, not a pure island.
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
    let dirty = "fn f() { stream.write_all(&buf).await }";
    assert_eq!(impurities(dirty), vec![".await"]);
    let clean = "fn decode(buf: &mut Bytes) -> Result<Message<Bytes>, Error> { todo!() }";
    assert!(impurities(clean).is_empty());
}

#[test]
fn message_rs_is_pure() {
    let src = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/message.rs"));
    let found = impurities(src);
    assert!(found.is_empty(), "message.rs gained async I/O: {found:?}");
}
