//! M263: a DHT actor started via [`DhtHandle::start_unified`] reads inbound
//! datagrams from the demux channel (NOT the socket directly) and sends its
//! replies back through the shared socket — so the reply's source address is
//! the shared, port-mapped listen port.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use gaia_dht::{DhtConfig, DhtHandle};
use tokio::net::UdpSocket;

/// A valid KRPC ping query: `{"a":{"id":"abcdefghij0123456789"},"q":"ping",
/// "t":"aa","y":"q"}` (starts with the bencode-dict byte `'d'`).
const KRPC_PING: &[u8] = b"d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe";

#[tokio::test]
async fn unified_dht_replies_via_shared_socket() {
    // The shared socket the session would own; the DHT sends through it but,
    // in unified mode, never reads it directly (the demux is the sole reader).
    let shared = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let shared_addr = shared.local_addr().unwrap();

    // A probe socket standing in for a remote DHT node querying us.
    let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let probe_addr = probe.local_addr().unwrap();

    let (tx, rx) = tokio::sync::mpsc::channel(16);
    // No DNS bootstrap in tests; we pass our own socket so bind_addr is unused.
    let config = DhtConfig {
        bootstrap_nodes: vec![],
        ..Default::default()
    };

    let (_handle, _ip_rx) =
        DhtHandle::start_unified(config, Arc::clone(&shared), rx).expect("start_unified");

    // Feed the ping as though the session's demux classified it as DHT and
    // forwarded it, tagged with the probe's address as the original sender.
    tx.send((Bytes::from_static(KRPC_PING), probe_addr))
        .await
        .expect("forward ping to demux channel");

    // The DHT must reply THROUGH the shared socket to the probe's address.
    let mut buf = vec![0u8; 1500];
    let (n, from) = tokio::time::timeout(Duration::from_secs(3), probe.recv_from(&mut buf))
        .await
        .expect("DHT did not reply within 3s")
        .expect("probe recv_from");

    assert_eq!(
        from, shared_addr,
        "the reply must originate from the shared (port-mapped) socket, \
         not an ephemeral DHT-owned port"
    );
    assert!(n > 0, "reply must be non-empty");
    assert_eq!(buf[0], b'd', "a KRPC reply is always a bencode dict");
}

#[tokio::test]
async fn unified_dht_does_not_read_shared_socket_directly() {
    // Regression guard for the M263 invariant: in unified mode the actor must
    // NOT call recv_from on the shared socket (that would steal packets from
    // the demux). We prove it indirectly: a datagram delivered to the shared
    // socket's address — but NOT forwarded through the channel — must get no
    // reply, because the actor never reads the socket.
    let shared = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let shared_addr = shared.local_addr().unwrap();

    let probe = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    let (_tx, rx) = tokio::sync::mpsc::channel::<(Bytes, std::net::SocketAddr)>(16);
    let config = DhtConfig {
        bootstrap_nodes: vec![],
        ..Default::default()
    };
    let (_handle, _ip_rx) =
        DhtHandle::start_unified(config, Arc::clone(&shared), rx).expect("start_unified");

    // Send straight at the shared socket's address, bypassing the channel.
    probe
        .send_to(KRPC_PING, shared_addr)
        .await
        .expect("send to shared addr");

    let mut buf = vec![0u8; 1500];
    let got = tokio::time::timeout(Duration::from_secs(1), probe.recv_from(&mut buf)).await;
    assert!(
        got.is_err(),
        "actor replied to a datagram it read off the shared socket directly — \
         it must only consume packets routed through the demux channel"
    );
}
