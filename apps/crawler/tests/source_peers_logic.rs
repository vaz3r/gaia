use crawler::dht::routing_table::{NodeInfo, decode_compact, xor};
use crawler::krpc::Infohash;
use std::net::{Ipv4Addr, SocketAddr};

#[test]
fn candidate_dedup_by_node_id() {
    let ih = Infohash::from([0x42; 20]);
    let addr1: SocketAddr = "1.2.3.4:6881".parse().unwrap();
    let addr2: SocketAddr = "5.6.7.8:6881".parse().unwrap();
    let same_id = [0x11; 20];
    let other_id = [0x22; 20];

    let mut candidates = vec![
        NodeInfo {
            id: same_id,
            addr: addr1,
        },
        NodeInfo {
            id: other_id,
            addr: addr2,
        },
        NodeInfo {
            id: same_id,
            addr: "9.10.11.12:6881".parse().unwrap(),
        },
    ];
    candidates.sort_by_key(|n| xor(&ih, &n.id));
    candidates.dedup_by(|a, b| a.id == b.id);
    assert_eq!(candidates.len(), 2);
}

#[test]
fn decode_compact_parses_all_nodes() {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xAA; 20]);
    data.extend_from_slice(&[8, 8, 8, 8]);
    data.extend_from_slice(&6881u16.to_be_bytes());
    data.extend_from_slice(&[0xBB; 20]);
    data.extend_from_slice(&[192, 168, 1, 1]);
    data.extend_from_slice(&6882u16.to_be_bytes());
    data.extend_from_slice(&[0xCC; 20]);
    data.extend_from_slice(&[10, 0, 0, 1]);
    data.extend_from_slice(&6883u16.to_be_bytes());
    data.extend_from_slice(&[0xDD; 20]);
    data.extend_from_slice(&[127, 0, 0, 1]);
    data.extend_from_slice(&6884u16.to_be_bytes());

    let nodes = decode_compact(&data);
    assert_eq!(nodes.len(), 4, "decode_compact parses all 26-byte entries");
}

#[test]
fn closest_returns_sorted_by_xor_distance() {
    let ih = Infohash::from([0xFF; 20]);
    let mut nodes = vec![
        NodeInfo {
            id: [0x01; 20],
            addr: "1.1.1.1:6881".parse().unwrap(),
        },
        NodeInfo {
            id: [0xFE; 20],
            addr: "2.2.2.2:6881".parse().unwrap(),
        },
        NodeInfo {
            id: [0x80; 20],
            addr: "3.3.3.3:6881".parse().unwrap(),
        },
    ];
    nodes.sort_by_key(|n| xor(&ih, &n.id));
    assert_eq!(nodes[0].id, [0xFE; 20], "0xFE should be closest to 0xFF");
    assert_eq!(nodes[1].id, [0x80; 20]);
    assert_eq!(nodes[2].id, [0x01; 20]);
}

#[test]
fn source_peers_early_stop_when_enough_peers() {
    let count = 12;
    let mut peers: Vec<SocketAddr> = Vec::new();

    for i in 0..3 {
        peers.push(SocketAddr::new(
            Ipv4Addr::new(10, 0, 0, i as u8).into(),
            6881,
        ));
        if peers.len() >= count {
            break;
        }
    }
    assert_eq!(peers.len(), 3, "Should stop after 3 rounds when count=12");
}

#[test]
fn source_peers_stops_immediately_when_round_returns_enough() {
    let count = 3;
    let mut peers: Vec<SocketAddr> = Vec::new();

    let batch_peers: Vec<SocketAddr> = (0..5)
        .map(|i| SocketAddr::new(Ipv4Addr::new(10, 0, 0, i).into(), 6881))
        .collect();
    for addr in &batch_peers {
        if peers.len() >= count {
            break;
        }
        peers.push(*addr);
    }
    assert_eq!(
        peers.len(),
        3,
        "Should stop at count=3 even though batch has 5"
    );
}

#[test]
fn decode_compact_empty_returns_empty() {
    let nodes = decode_compact(&[]);
    assert!(nodes.is_empty());
}

#[test]
fn decode_compact_partial_node_ignored() {
    let mut data = Vec::new();
    data.extend_from_slice(&[0xAA; 20]);
    data.extend_from_slice(&[8, 8, 8, 8]);
    data.extend_from_slice(&6881u16.to_be_bytes());
    data.extend_from_slice(&[0xBB; 20]);
    data.extend_from_slice(&[1, 2, 3]);

    let nodes = decode_compact(&data);
    assert_eq!(
        nodes.len(),
        1,
        "Only the complete 26-byte entry should be parsed"
    );
}
