#![warn(missing_docs)]
#![forbid(unsafe_code)]
//! Kademlia DHT implementation: BEP 5 routing, BEP 42 security, BEP 44 storage, BEP 51 indexing.
//!
//! Implements the Mainline DHT protocol: KRPC message encoding, Kademlia
//! routing table, peer discovery, and announce operations.
//!
//! # Architecture
//!
//! The DHT runs as an actor: `DhtHandle::start()` spawns a background task
//! that owns all state (routing table, UDP socket, pending queries). The
//! returned `DhtHandle` is a cheap, cloneable sender for submitting commands.

mod actor;
/// BEP 44 immutable and mutable item storage.
pub mod bep44;
/// BEP 33 bloom filters for DHT scrape swarm estimation.
pub mod bloom;
/// Compact node encoding (26-byte IPv4, 38-byte IPv6).
pub mod compact;
/// Self-contained parallel DHT lookup with 256-node tracking.
pub(crate) mod dht_lookup;
/// DHT error types.
pub mod error;
/// Broadcast surface for runtime DhtHandle replacement (M173 Lane B B5).
pub mod handle;
/// KRPC message encoding and decoding (BEP 5).
pub mod krpc;
/// Generic iterative Kademlia lookup (shared by bootstrap find_node).
pub(crate) mod lookup;
/// BEP 42 node ID generation and validation.
pub mod node_id;
/// Per-info_hash peer storage and announce token management.
pub mod peer_store;
/// Kademlia routing table with k-buckets.
pub mod routing_table;
/// DHT item storage backend.
pub mod storage;

pub use actor::{DhtConfig, DhtEvent, DhtHandle, DhtStats, SampleInfohashesResult};
pub use bep44::{
    ImmutableItem, MAX_SALT_SIZE, MAX_VALUE_SIZE, MutableItem, build_signing_buffer,
    compute_mutable_target,
};
pub use compact::{
    COMPACT_NODE_SIZE, COMPACT_NODE6_SIZE, CompactNodeInfo, CompactNodeInfo6, encode_compact_nodes,
    encode_compact_nodes6, parse_compact_nodes, parse_compact_nodes6,
};
pub use error::{Error, Result};
pub use handle::{DhtBroadcast, DhtReceiver};
pub use krpc::{
    GetPeersResponse, KrpcBody, KrpcMessage, KrpcQuery, KrpcResponse, SampleInfohashesResponse,
    TransactionId,
};
pub use node_id::{
    ExternalIpVoter, IpVoteSource, generate_node_id, is_bep42_exempt, is_valid_node_id,
};
pub use routing_table::{NodeStatus, RoutingTable};
pub use storage::{DhtStorage, InMemoryDhtStorage};
