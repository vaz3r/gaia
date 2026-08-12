#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "M175: BEP 5 KRPC — port/transaction-id field widths fixed by spec"
)]

//! KRPC message encoding and decoding (BEP 5).
//!
//! KRPC messages are bencoded dictionaries with keys:
//! - `t` — transaction ID (binary string, 2 bytes)
//! - `y` — message type: `q` (query), `r` (response), `e` (error)
//! - `q` — query method name (for queries only)
//! - `a` — query arguments dict (for queries only)
//! - `r` — response values dict (for responses only)
//! - `e` — error list `[code, message]` (for errors only)

use std::collections::BTreeMap;

use gaia_bencode::{self as bencode, BencodeValue};
use gaia_core::Id20;

use crate::compact::{
    CompactNodeInfo, CompactNodeInfo6, encode_compact_nodes, encode_compact_nodes6,
    parse_compact_nodes, parse_compact_nodes6,
};
use crate::error::{Error, Result};

/// 2-byte transaction ID for matching requests to responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransactionId(pub [u8; 2]);

impl TransactionId {
    /// Create a transaction ID from a 16-bit integer.
    #[must_use]
    pub fn from_u16(val: u16) -> Self {
        Self(val.to_be_bytes())
    }

    /// Return the transaction ID as a 16-bit integer.
    #[must_use]
    pub fn as_u16(&self) -> u16 {
        u16::from_be_bytes(self.0)
    }

    /// Parse a transaction ID from raw bytes (pads 1-byte IDs with zero).
    ///
    /// # Errors
    ///
    /// Returns an error if `bytes` is empty.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 2 {
            // Some implementations use 1-byte transaction IDs; pad with zero
            let mut buf = [0u8; 2];
            buf[..bytes.len()].copy_from_slice(bytes);
            return Ok(Self(buf));
        }
        Ok(Self([bytes[0], bytes[1]]))
    }
}

/// A parsed KRPC message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrpcMessage {
    /// Opaque 2-byte identifier matching requests to responses.
    pub transaction_id: TransactionId,
    /// Message payload (query, response, or error).
    pub body: KrpcBody,
    /// BEP 42: Compact IP+port of the message recipient, included in responses.
    pub sender_ip: Option<std::net::SocketAddr>,
    /// BEP 43: Read-only node flag. When true, the sending node should not be
    /// added to the routing table (it cannot store data or respond to queries
    /// from arbitrary nodes).
    pub read_only: bool,
}

/// The body of a KRPC message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KrpcBody {
    /// A query from a remote node.
    Query(KrpcQuery),
    /// A response to one of our queries.
    Response(KrpcResponse),
    /// An error response.
    Error {
        /// KRPC error code.
        code: i64,
        /// Human-readable error description.
        message: String,
    },
}

/// BEP 45: Address family requested in the `want` field of queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WantFamily {
    /// IPv4 nodes (`"n4"` on the wire).
    N4,
    /// IPv6 nodes (`"n6"` on the wire).
    N6,
}

impl WantFamily {
    /// Wire representation as a bencode byte string.
    #[must_use]
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            Self::N4 => b"n4",
            Self::N6 => b"n6",
        }
    }

    /// Parse from wire bytes; returns `None` for unrecognised values.
    #[must_use]
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        match b {
            b"n4" => Some(Self::N4),
            b"n6" => Some(Self::N6),
            _ => None,
        }
    }
}

/// KRPC query types (BEP 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KrpcQuery {
    /// Liveness check.
    Ping {
        /// Querying node's ID.
        id: Id20,
    },
    /// Find the closest nodes to a target ID.
    FindNode {
        /// Querying node's ID.
        id: Id20,
        /// Target node ID to search for.
        target: Id20,
        /// BEP 45: requested address families.
        want: Option<Vec<WantFamily>>,
    },
    /// Find peers downloading a torrent.
    GetPeers {
        /// Querying node's ID.
        id: Id20,
        /// Torrent info hash to search for.
        info_hash: Id20,
        /// BEP 33: if set to 1, exclude seed peers from results.
        noseed: Option<i64>,
        /// BEP 33: if set to 1, include bloom filter scrape data in response.
        scrape: Option<i64>,
        /// BEP 45: requested address families.
        want: Option<Vec<WantFamily>>,
    },
    /// Announce that we are downloading a torrent.
    AnnouncePeer {
        /// Querying node's ID.
        id: Id20,
        /// Torrent info hash being announced.
        info_hash: Id20,
        /// Port we are listening on.
        port: u16,
        /// If true, use the UDP source port instead of the `port` field.
        implied_port: bool,
        /// Write token obtained from a prior `get_peers` response.
        token: Vec<u8>,
    },
    /// BEP 44: get an item from DHT storage.
    Get {
        /// Querying node's ID.
        id: Id20,
        /// Target hash: SHA-1(value) for immutable, SHA-1(pubkey+salt) for mutable.
        target: Id20,
        /// Optional: if set, only return mutable items with seq > this value.
        seq: Option<i64>,
    },
    /// BEP 44: put an item into DHT storage.
    Put {
        /// Querying node's ID.
        id: Id20,
        /// Write token (obtained from a prior get response).
        token: Vec<u8>,
        /// The bencoded value to store.
        value: Vec<u8>,
        /// For mutable items: ed25519 public key (32 bytes).
        key: Option<[u8; 32]>,
        /// For mutable items: ed25519 signature (64 bytes).
        signature: Option<[u8; 64]>,
        /// For mutable items: sequence number.
        seq: Option<i64>,
        /// For mutable items: optional salt.
        salt: Option<Vec<u8>>,
        /// For mutable items: optional CAS (compare-and-swap) expected seq.
        cas: Option<i64>,
    },
    /// BEP 51: sample info hashes from a node's storage.
    SampleInfohashes {
        /// Querying node's ID.
        id: Id20,
        /// Target ID for DHT traversal.
        target: Id20,
    },
}

/// KRPC response types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KrpcResponse {
    /// Response to ping or `announce_peer` — just the node ID.
    NodeId {
        /// Responding node's ID.
        id: Id20,
    },
    /// Response to `find_node`.
    FindNode {
        /// Responding node's ID.
        id: Id20,
        /// Closest known IPv4 nodes.
        nodes: Vec<CompactNodeInfo>,
        /// Closest known IPv6 nodes (BEP 24).
        nodes6: Vec<CompactNodeInfo6>,
    },
    /// Response to `get_peers` — either peers or closer nodes.
    GetPeers(GetPeersResponse),
    /// BEP 44: response to a get query.
    GetItem {
        /// Responding node's ID.
        id: Id20,
        /// Write token for subsequent put operations.
        token: Option<Vec<u8>>,
        /// Closest known IPv4 nodes.
        nodes: Vec<CompactNodeInfo>,
        /// Closest known IPv6 nodes.
        nodes6: Vec<CompactNodeInfo6>,
        /// The stored value (if found).
        value: Option<Vec<u8>>,
        /// For mutable items: ed25519 public key.
        key: Option<[u8; 32]>,
        /// For mutable items: signature.
        signature: Option<[u8; 64]>,
        /// For mutable items: sequence number.
        seq: Option<i64>,
    },
    /// Response to `sample_infohashes` (BEP 51).
    SampleInfohashes(SampleInfohashesResponse),
}

/// `get_peers` response can return peers, closer nodes, or both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetPeersResponse {
    /// Responding node's ID.
    pub id: Id20,
    /// Write token for `announce_peer`.
    pub token: Option<Vec<u8>>,
    /// Direct peer addresses (compact: 6 bytes each for IPv4, 18 bytes for IPv6).
    pub peers: Vec<std::net::SocketAddr>,
    /// Closer nodes (compact: 26 bytes each).
    pub nodes: Vec<CompactNodeInfo>,
    /// Closer IPv6 nodes (BEP 24, compact: 38 bytes each).
    pub nodes6: Vec<CompactNodeInfo6>,
    /// BEP 33: peer/leecher bloom filter (256 bytes).
    pub bfpe: Option<Vec<u8>>,
    /// BEP 33: seed bloom filter (256 bytes).
    pub bfsd: Option<Vec<u8>>,
}

/// Response to `sample_infohashes` (BEP 51).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleInfohashesResponse {
    /// Responding node's ID.
    pub id: Id20,
    /// Minimum seconds before querying this node again.
    pub interval: i64,
    /// Estimated total number of info hashes in this node's storage.
    pub num: i64,
    /// Random sample of info hashes (each 20 bytes).
    pub samples: Vec<Id20>,
    /// Closer nodes (compact format), for DHT traversal.
    pub nodes: Vec<CompactNodeInfo>,
}

impl KrpcQuery {
    /// The query method name as used in the `q` field.
    #[must_use]
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::Ping { .. } => "ping",
            Self::FindNode { .. } => "find_node",
            Self::GetPeers { .. } => "get_peers",
            Self::AnnouncePeer { .. } => "announce_peer",
            Self::Get { .. } => "get",
            Self::Put { .. } => "put",
            Self::SampleInfohashes { .. } => "sample_infohashes",
        }
    }

    /// The querying node's ID.
    #[must_use]
    pub fn sender_id(&self) -> &Id20 {
        match self {
            Self::Ping { id }
            | Self::FindNode { id, .. }
            | Self::GetPeers { id, .. }
            | Self::AnnouncePeer { id, .. }
            | Self::Get { id, .. }
            | Self::Put { id, .. }
            | Self::SampleInfohashes { id, .. } => id,
        }
    }
}

impl KrpcResponse {
    /// The responding node's ID.
    #[must_use]
    pub fn sender_id(&self) -> &Id20 {
        match self {
            Self::NodeId { id } | Self::FindNode { id, .. } | Self::GetItem { id, .. } => id,
            Self::GetPeers(gp) => &gp.id,
            Self::SampleInfohashes(si) => &si.id,
        }
    }
}

// ---- Encoding ----

impl KrpcMessage {
    /// Encode this message to bencode bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if bencode serialization fails.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut dict = BTreeMap::<Vec<u8>, BencodeValue>::new();
        dict.insert(
            b"t".to_vec(),
            BencodeValue::Bytes(self.transaction_id.0.to_vec()),
        );

        if let Some(addr) = &self.sender_ip {
            let ip_bytes = encode_compact_addr(addr);
            dict.insert(b"ip".to_vec(), BencodeValue::Bytes(ip_bytes));
        }

        // BEP 43: include `ro` flag when set
        if self.read_only {
            dict.insert(b"ro".to_vec(), BencodeValue::Integer(1));
        }

        match &self.body {
            KrpcBody::Query(query) => {
                dict.insert(b"y".to_vec(), BencodeValue::Bytes(b"q".to_vec()));
                dict.insert(
                    b"q".to_vec(),
                    BencodeValue::Bytes(query.method_name().as_bytes().to_vec()),
                );
                dict.insert(b"a".to_vec(), encode_query_args(query));
            }
            KrpcBody::Response(resp) => {
                dict.insert(b"y".to_vec(), BencodeValue::Bytes(b"r".to_vec()));
                dict.insert(b"r".to_vec(), encode_response_values(resp));
            }
            KrpcBody::Error { code, message } => {
                dict.insert(b"y".to_vec(), BencodeValue::Bytes(b"e".to_vec()));
                dict.insert(
                    b"e".to_vec(),
                    BencodeValue::List(vec![
                        BencodeValue::Integer(*code),
                        BencodeValue::Bytes(message.as_bytes().to_vec()),
                    ]),
                );
            }
        }

        bencode::to_bytes(&BencodeValue::Dict(dict)).map_err(Error::from)
    }

    /// Decode from bencode bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid bencode or the message
    /// structure is malformed.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let value: BencodeValue = bencode::from_bytes(data)?;
        let dict = value
            .as_dict()
            .ok_or_else(|| Error::InvalidMessage("top-level value is not a dict".into()))?;

        let txn_bytes = dict_bytes(dict, b"t")?;
        let transaction_id = TransactionId::from_bytes(txn_bytes)?;

        let msg_type = dict_str(dict, b"y")?;
        let body = match msg_type {
            b"q" => {
                let method = dict_str(dict, b"q")?;
                let args = dict_dict(dict, b"a")?;
                KrpcBody::Query(decode_query(method, args)?)
            }
            b"r" => {
                let values = dict_dict(dict, b"r")?;
                KrpcBody::Response(decode_response(values, None)?)
            }
            b"e" => {
                let err_list = dict
                    .get(&b"e"[..])
                    .and_then(|v| v.as_list())
                    .ok_or_else(|| Error::InvalidMessage("missing 'e' list".into()))?;
                if err_list.len() < 2 {
                    return Err(Error::InvalidMessage("error list too short".into()));
                }
                let code = err_list[0]
                    .as_int()
                    .ok_or_else(|| Error::InvalidMessage("error code not integer".into()))?;
                let message = err_list[1]
                    .as_bytes_raw()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .ok_or_else(|| Error::InvalidMessage("error message not string".into()))?;
                KrpcBody::Error { code, message }
            }
            other => {
                return Err(Error::InvalidMessage(format!(
                    "unknown message type: {}",
                    String::from_utf8_lossy(other)
                )));
            }
        };

        let sender_ip = dict
            .get(&b"ip"[..])
            .and_then(|v| v.as_bytes_raw())
            .and_then(decode_compact_addr);

        // BEP 43: extract read-only flag
        let read_only = dict
            .get(&b"ro"[..])
            .and_then(gaia_bencode::BencodeValue::as_int)
            .is_some_and(|i| i != 0);

        Ok(Self {
            transaction_id,
            body,
            sender_ip,
            read_only,
        })
    }

    /// Decode from bencode bytes with a query-method hint for response disambiguation.
    ///
    /// When you know which query method this response answers (e.g. `"get"` vs
    /// `"get_peers"`), pass it here so the decoder picks the correct response
    /// variant. This resolves ambiguity for BEP 44 "not found" responses that
    /// share the same wire shape as `get_peers` responses (token + nodes only).
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid bencode or the message
    /// structure is malformed.
    pub fn from_bytes_with_query_hint(data: &[u8], query_method: &str) -> Result<Self> {
        let value: BencodeValue = bencode::from_bytes(data)?;
        let dict = value
            .as_dict()
            .ok_or_else(|| Error::InvalidMessage("top-level value is not a dict".into()))?;

        let txn_bytes = dict_bytes(dict, b"t")?;
        let transaction_id = TransactionId::from_bytes(txn_bytes)?;

        let msg_type = dict_str(dict, b"y")?;
        let body = match msg_type {
            b"q" => {
                let method = dict_str(dict, b"q")?;
                let args = dict_dict(dict, b"a")?;
                KrpcBody::Query(decode_query(method, args)?)
            }
            b"r" => {
                let values = dict_dict(dict, b"r")?;
                KrpcBody::Response(decode_response(values, Some(query_method))?)
            }
            b"e" => {
                let err_list = dict
                    .get(&b"e"[..])
                    .and_then(|v| v.as_list())
                    .ok_or_else(|| Error::InvalidMessage("missing 'e' list".into()))?;
                if err_list.len() < 2 {
                    return Err(Error::InvalidMessage("error list too short".into()));
                }
                let code = err_list[0]
                    .as_int()
                    .ok_or_else(|| Error::InvalidMessage("error code not integer".into()))?;
                let message = err_list[1]
                    .as_bytes_raw()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .ok_or_else(|| Error::InvalidMessage("error message not string".into()))?;
                KrpcBody::Error { code, message }
            }
            other => {
                return Err(Error::InvalidMessage(format!(
                    "unknown message type: {}",
                    String::from_utf8_lossy(other)
                )));
            }
        };

        let sender_ip = dict
            .get(&b"ip"[..])
            .and_then(|v| v.as_bytes_raw())
            .and_then(decode_compact_addr);

        // BEP 43: extract read-only flag
        let read_only = dict
            .get(&b"ro"[..])
            .and_then(gaia_bencode::BencodeValue::as_int)
            .is_some_and(|i| i != 0);

        Ok(Self {
            transaction_id,
            body,
            sender_ip,
            read_only,
        })
    }
}

// ---- Internal encoding helpers ----

fn encode_query_args(query: &KrpcQuery) -> BencodeValue {
    let mut args = BTreeMap::<Vec<u8>, BencodeValue>::new();
    match query {
        KrpcQuery::Ping { id } => {
            args.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
        }
        KrpcQuery::FindNode { id, target, want } => {
            args.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            args.insert(b"target".to_vec(), BencodeValue::Bytes(target.0.to_vec()));
            if let Some(w) = want {
                encode_want(&mut args, w);
            }
        }
        KrpcQuery::SampleInfohashes { id, target } => {
            args.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            args.insert(b"target".to_vec(), BencodeValue::Bytes(target.0.to_vec()));
        }
        KrpcQuery::GetPeers {
            id,
            info_hash,
            noseed,
            scrape,
            want,
        } => {
            args.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            args.insert(
                b"info_hash".to_vec(),
                BencodeValue::Bytes(info_hash.0.to_vec()),
            );
            if let Some(ns) = noseed {
                args.insert(b"noseed".to_vec(), BencodeValue::Integer(*ns));
            }
            if let Some(sc) = scrape {
                args.insert(b"scrape".to_vec(), BencodeValue::Integer(*sc));
            }
            if let Some(w) = want {
                encode_want(&mut args, w);
            }
        }
        KrpcQuery::AnnouncePeer {
            id,
            info_hash,
            port,
            implied_port,
            token,
        } => {
            args.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            if *implied_port {
                args.insert(b"implied_port".to_vec(), BencodeValue::Integer(1));
            }
            args.insert(
                b"info_hash".to_vec(),
                BencodeValue::Bytes(info_hash.0.to_vec()),
            );
            args.insert(b"port".to_vec(), BencodeValue::Integer(i64::from(*port)));
            args.insert(b"token".to_vec(), BencodeValue::Bytes(token.clone()));
        }
        KrpcQuery::Get { id, target, seq } => {
            args.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            if let Some(seq) = seq {
                args.insert(b"seq".to_vec(), BencodeValue::Integer(*seq));
            }
            args.insert(b"target".to_vec(), BencodeValue::Bytes(target.0.to_vec()));
        }
        KrpcQuery::Put {
            id,
            token,
            value,
            key,
            signature,
            seq,
            salt,
            cas,
        } => {
            args.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            if let Some(cas) = cas {
                args.insert(b"cas".to_vec(), BencodeValue::Integer(*cas));
            }
            if let Some(key) = key {
                args.insert(b"k".to_vec(), BencodeValue::Bytes(key.to_vec()));
            }
            if let Some(salt) = salt
                && !salt.is_empty()
            {
                args.insert(b"salt".to_vec(), BencodeValue::Bytes(salt.clone()));
            }
            if let Some(seq) = seq {
                args.insert(b"seq".to_vec(), BencodeValue::Integer(*seq));
            }
            if let Some(sig) = signature {
                args.insert(b"sig".to_vec(), BencodeValue::Bytes(sig.to_vec()));
            }
            args.insert(b"token".to_vec(), BencodeValue::Bytes(token.clone()));
            args.insert(b"v".to_vec(), BencodeValue::Bytes(value.clone()));
        }
    }
    BencodeValue::Dict(args)
}

fn encode_response_values(resp: &KrpcResponse) -> BencodeValue {
    let mut values = BTreeMap::<Vec<u8>, BencodeValue>::new();
    match resp {
        KrpcResponse::NodeId { id } => {
            values.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
        }
        KrpcResponse::FindNode { id, nodes, nodes6 } => {
            values.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            values.insert(
                b"nodes".to_vec(),
                BencodeValue::Bytes(encode_compact_nodes(nodes)),
            );
            if !nodes6.is_empty() {
                values.insert(
                    b"nodes6".to_vec(),
                    BencodeValue::Bytes(encode_compact_nodes6(nodes6)),
                );
            }
        }
        KrpcResponse::GetPeers(gp) => {
            values.insert(b"id".to_vec(), BencodeValue::Bytes(gp.id.0.to_vec()));
            if let Some(token) = &gp.token {
                values.insert(b"token".to_vec(), BencodeValue::Bytes(token.clone()));
            }
            if !gp.peers.is_empty() {
                let peer_list: Vec<BencodeValue> = gp
                    .peers
                    .iter()
                    .map(|addr| match addr {
                        SocketAddr::V4(v4) => {
                            let mut buf = [0u8; 6];
                            buf[..4].copy_from_slice(&v4.ip().octets());
                            buf[4..6].copy_from_slice(&v4.port().to_be_bytes());
                            BencodeValue::Bytes(buf.to_vec())
                        }
                        SocketAddr::V6(v6) => {
                            let mut buf = [0u8; 18];
                            buf[..16].copy_from_slice(&v6.ip().octets());
                            buf[16..18].copy_from_slice(&v6.port().to_be_bytes());
                            BencodeValue::Bytes(buf.to_vec())
                        }
                    })
                    .collect();
                values.insert(b"values".to_vec(), BencodeValue::List(peer_list));
            }
            if !gp.nodes.is_empty() {
                values.insert(
                    b"nodes".to_vec(),
                    BencodeValue::Bytes(encode_compact_nodes(&gp.nodes)),
                );
            }
            if !gp.nodes6.is_empty() {
                values.insert(
                    b"nodes6".to_vec(),
                    BencodeValue::Bytes(encode_compact_nodes6(&gp.nodes6)),
                );
            }
            if let Some(ref bfpe) = gp.bfpe {
                values.insert(b"BFpe".to_vec(), BencodeValue::Bytes(bfpe.clone()));
            }
            if let Some(ref bfsd) = gp.bfsd {
                values.insert(b"BFsd".to_vec(), BencodeValue::Bytes(bfsd.clone()));
            }
        }
        KrpcResponse::GetItem {
            id,
            token,
            nodes,
            nodes6,
            value,
            key,
            signature,
            seq,
        } => {
            values.insert(b"id".to_vec(), BencodeValue::Bytes(id.0.to_vec()));
            if let Some(key) = key {
                values.insert(b"k".to_vec(), BencodeValue::Bytes(key.to_vec()));
            }
            if !nodes.is_empty() {
                values.insert(
                    b"nodes".to_vec(),
                    BencodeValue::Bytes(encode_compact_nodes(nodes)),
                );
            }
            if !nodes6.is_empty() {
                values.insert(
                    b"nodes6".to_vec(),
                    BencodeValue::Bytes(encode_compact_nodes6(nodes6)),
                );
            }
            if let Some(seq) = seq {
                values.insert(b"seq".to_vec(), BencodeValue::Integer(*seq));
            }
            if let Some(sig) = signature {
                values.insert(b"sig".to_vec(), BencodeValue::Bytes(sig.to_vec()));
            }
            if let Some(token) = token {
                values.insert(b"token".to_vec(), BencodeValue::Bytes(token.clone()));
            }
            if let Some(v) = value {
                values.insert(b"v".to_vec(), BencodeValue::Bytes(v.clone()));
            }
        }
        KrpcResponse::SampleInfohashes(si) => {
            values.insert(b"id".to_vec(), BencodeValue::Bytes(si.id.0.to_vec()));
            values.insert(b"interval".to_vec(), BencodeValue::Integer(si.interval));
            if !si.nodes.is_empty() {
                values.insert(
                    b"nodes".to_vec(),
                    BencodeValue::Bytes(encode_compact_nodes(&si.nodes)),
                );
            }
            values.insert(b"num".to_vec(), BencodeValue::Integer(si.num));
            // BEP 51: "samples" is always present, even if empty
            let mut samples_buf = Vec::with_capacity(si.samples.len() * 20);
            for hash in &si.samples {
                samples_buf.extend_from_slice(hash.as_bytes());
            }
            values.insert(b"samples".to_vec(), BencodeValue::Bytes(samples_buf));
        }
    }
    BencodeValue::Dict(values)
}

// ---- BEP 45 want helpers ----

fn encode_want(args: &mut BTreeMap<Vec<u8>, BencodeValue>, want: &[WantFamily]) {
    let list: Vec<BencodeValue> = want
        .iter()
        .map(|w| BencodeValue::Bytes(w.as_bytes().to_vec()))
        .collect();
    args.insert(b"want".to_vec(), BencodeValue::List(list));
}

fn decode_want(args: &BTreeMap<Vec<u8>, BencodeValue>) -> Option<Vec<WantFamily>> {
    let list = args.get(&b"want"[..])?.as_list()?;
    let families: Vec<WantFamily> = list
        .iter()
        .filter_map(|v| v.as_bytes_raw().and_then(WantFamily::from_bytes))
        .collect();
    if families.is_empty() {
        None
    } else {
        Some(families)
    }
}

// ---- Compact address helpers (BEP 42 `ip` field) ----

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

/// Encode a socket address to compact binary format (BEP 42 `ip` field).
fn encode_compact_addr(addr: &SocketAddr) -> Vec<u8> {
    match addr {
        SocketAddr::V4(v4) => {
            let mut buf = Vec::with_capacity(6);
            buf.extend_from_slice(&v4.ip().octets());
            buf.extend_from_slice(&v4.port().to_be_bytes());
            buf
        }
        SocketAddr::V6(v6) => {
            let mut buf = Vec::with_capacity(18);
            buf.extend_from_slice(&v6.ip().octets());
            buf.extend_from_slice(&v6.port().to_be_bytes());
            buf
        }
    }
}

/// Decode a compact binary socket address (BEP 42 `ip` field).
fn decode_compact_addr(data: &[u8]) -> Option<SocketAddr> {
    match data.len() {
        6 => {
            let ip = Ipv4Addr::new(data[0], data[1], data[2], data[3]);
            let port = u16::from_be_bytes([data[4], data[5]]);
            Some(SocketAddr::V4(SocketAddrV4::new(ip, port)))
        }
        18 => {
            let ip = Ipv6Addr::from(<[u8; 16]>::try_from(&data[..16]).unwrap());
            let port = u16::from_be_bytes([data[16], data[17]]);
            Some(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0)))
        }
        _ => None,
    }
}

// ---- Internal decoding helpers ----

fn decode_query(method: &[u8], args: &BTreeMap<Vec<u8>, BencodeValue>) -> Result<KrpcQuery> {
    let id = args_id20(args, b"id")?;
    match method {
        b"ping" => Ok(KrpcQuery::Ping { id }),
        b"find_node" => {
            let target = args_id20(args, b"target")?;
            let want = decode_want(args);
            Ok(KrpcQuery::FindNode { id, target, want })
        }
        b"get_peers" => {
            let info_hash = args_id20(args, b"info_hash")?;
            let noseed = args
                .get(&b"noseed"[..])
                .and_then(gaia_bencode::BencodeValue::as_int);
            let scrape = args
                .get(&b"scrape"[..])
                .and_then(gaia_bencode::BencodeValue::as_int);
            let want = decode_want(args);
            Ok(KrpcQuery::GetPeers {
                id,
                info_hash,
                noseed,
                scrape,
                want,
            })
        }
        b"announce_peer" => {
            let info_hash = args_id20(args, b"info_hash")?;
            let implied_port = args
                .get(&b"implied_port"[..])
                .and_then(gaia_bencode::BencodeValue::as_int)
                .unwrap_or(0)
                != 0;
            // L5 (M241): `port` is attacker-controlled bencode. The previous
            // `args_int(...)? as u16` silently truncated out-of-range values
            // (70000 -> 4464, -1 -> 65535), producing a bogus non-connecting
            // peer entry.
            //
            // Per BEP 5, when implied_port is set the requester's source UDP
            // port is authoritative and `port` is ignored downstream, so we must
            // not reject the query on an out-of-range / zero / absent port there
            // (real clients send junk in that field) — normalize it to 0. When
            // implied_port is unset, `port` is authoritative: range-check it and
            // reject 0.
            let port = if implied_port {
                args.get(&b"port"[..])
                    .and_then(gaia_bencode::BencodeValue::as_int)
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(0)
            } else {
                let port_raw = args_int(args, b"port")?;
                let port = u16::try_from(port_raw).map_err(|_| {
                    Error::InvalidMessage(format!(
                        "announce_peer 'port' {port_raw} is outside the valid range 0..=65535"
                    ))
                })?;
                if port == 0 {
                    return Err(Error::InvalidMessage(
                        "announce_peer 'port' must not be 0 when implied_port is unset".into(),
                    ));
                }
                port
            };
            let token = args
                .get(&b"token"[..])
                .and_then(|v| v.as_bytes_raw())
                .map(<[u8]>::to_vec)
                .ok_or_else(|| Error::InvalidMessage("missing 'token' in announce_peer".into()))?;
            Ok(KrpcQuery::AnnouncePeer {
                id,
                info_hash,
                port,
                implied_port,
                token,
            })
        }
        b"get" => {
            let target = args_id20(args, b"target")?;
            let seq = args
                .get(&b"seq"[..])
                .and_then(gaia_bencode::BencodeValue::as_int);
            Ok(KrpcQuery::Get { id, target, seq })
        }
        b"put" => {
            let token = args
                .get(&b"token"[..])
                .and_then(|v| v.as_bytes_raw())
                .map(<[u8]>::to_vec)
                .ok_or_else(|| Error::InvalidMessage("missing 'token' in put".into()))?;
            let value = args
                .get(&b"v"[..])
                .and_then(|v| v.as_bytes_raw())
                .map(<[u8]>::to_vec)
                .ok_or_else(|| Error::InvalidMessage("missing 'v' in put".into()))?;
            let key = args
                .get(&b"k"[..])
                .and_then(|v| v.as_bytes_raw())
                .and_then(|b| <[u8; 32]>::try_from(b).ok());
            let signature = args
                .get(&b"sig"[..])
                .and_then(|v| v.as_bytes_raw())
                .and_then(|b| <[u8; 64]>::try_from(b).ok());
            let seq = args
                .get(&b"seq"[..])
                .and_then(gaia_bencode::BencodeValue::as_int);
            let salt = args
                .get(&b"salt"[..])
                .and_then(|v| v.as_bytes_raw())
                .map(<[u8]>::to_vec);
            let cas = args
                .get(&b"cas"[..])
                .and_then(gaia_bencode::BencodeValue::as_int);
            Ok(KrpcQuery::Put {
                id,
                token,
                value,
                key,
                signature,
                seq,
                salt,
                cas,
            })
        }
        b"sample_infohashes" => {
            let target = args_id20(args, b"target")?;
            Ok(KrpcQuery::SampleInfohashes { id, target })
        }
        _ => Err(Error::InvalidMessage(format!(
            "unknown query method: {}",
            String::from_utf8_lossy(method)
        ))),
    }
}

/// Decode a KRPC response from bencoded values.
///
/// The optional `query_method` hint disambiguates responses that share the same
/// wire shape (e.g. BEP 44 `get` vs BEP 5 `get_peers` — both may carry only
/// `token` + `nodes`). When the caller knows the originating query method, pass
/// `Some("get")` or `Some("get_peers")` to force the correct variant. Pass
/// `None` to use heuristic detection (suitable for standalone decoding).
fn decode_response(
    values: &BTreeMap<Vec<u8>, BencodeValue>,
    query_method: Option<&str>,
) -> Result<KrpcResponse> {
    let id = args_id20(values, b"id")?;

    // If the caller tells us this is a BEP 44 get response, decode as GetItem
    // regardless of which fields are present (handles "not found" case).
    if query_method == Some("get") {
        return decode_get_item_response(id, values);
    }

    // sample_infohashes response (BEP 51): has "samples" + "interval" + "num"
    let has_samples = values.contains_key(&b"samples"[..]);
    let has_interval = values.contains_key(&b"interval"[..]);

    if has_samples && has_interval {
        let interval = values
            .get(&b"interval"[..])
            .and_then(gaia_bencode::BencodeValue::as_int)
            .unwrap_or(0);
        let num = values
            .get(&b"num"[..])
            .and_then(gaia_bencode::BencodeValue::as_int)
            .unwrap_or(0);

        let samples_bytes = values
            .get(&b"samples"[..])
            .and_then(|v| v.as_bytes_raw())
            .unwrap_or(&[]);
        let mut samples = Vec::new();
        if samples_bytes.len().is_multiple_of(20) {
            for chunk in samples_bytes.chunks_exact(20) {
                if let Ok(hash) = Id20::from_bytes(chunk) {
                    samples.push(hash);
                }
            }
        }

        let nodes =
            if let Some(nodes_bytes) = values.get(&b"nodes"[..]).and_then(|v| v.as_bytes_raw()) {
                parse_compact_nodes(nodes_bytes)?
            } else {
                Vec::new()
            };

        return Ok(KrpcResponse::SampleInfohashes(SampleInfohashesResponse {
            id,
            interval,
            num,
            samples,
            nodes,
        }));
    }

    // BEP 44 get response heuristic: has "k" (mutable key), "sig" (signature),
    // or "v" without "values" (immutable value — not a get_peers peer list).
    let has_values = values.contains_key(&b"values"[..]);
    let has_v = values.contains_key(&b"v"[..]);
    let has_k = values.contains_key(&b"k"[..]);
    let has_sig = values.contains_key(&b"sig"[..]);
    let has_seq = values.contains_key(&b"seq"[..]);

    if has_k || has_sig || (has_v && !has_values) || (has_seq && !has_values) {
        return decode_get_item_response(id, values);
    }

    // get_peers response: has "values" (peers) or "nodes" (closer nodes) + optional "token"
    let has_token = values.contains_key(&b"token"[..]);

    if has_values || has_token {
        let token = values
            .get(&b"token"[..])
            .and_then(|v| v.as_bytes_raw())
            .map(<[u8]>::to_vec);

        let mut peers = Vec::new();
        if let Some(BencodeValue::List(peer_list)) = values.get(&b"values"[..]) {
            for item in peer_list {
                if let Some(data) = item.as_bytes_raw() {
                    match data.len() {
                        6 => {
                            let ip = Ipv4Addr::new(data[0], data[1], data[2], data[3]);
                            let port = u16::from_be_bytes([data[4], data[5]]);
                            peers.push(SocketAddr::V4(SocketAddrV4::new(ip, port)));
                        }
                        18 => {
                            let ip = Ipv6Addr::from(<[u8; 16]>::try_from(&data[..16]).unwrap());
                            let port = u16::from_be_bytes([data[16], data[17]]);
                            peers.push(SocketAddr::V6(SocketAddrV6::new(ip, port, 0, 0)));
                        }
                        _ => {} // skip unknown sizes
                    }
                }
            }
        }

        let nodes =
            if let Some(nodes_bytes) = values.get(&b"nodes"[..]).and_then(|v| v.as_bytes_raw()) {
                parse_compact_nodes(nodes_bytes)?
            } else {
                Vec::new()
            };

        let nodes6 =
            if let Some(nodes6_bytes) = values.get(&b"nodes6"[..]).and_then(|v| v.as_bytes_raw()) {
                parse_compact_nodes6(nodes6_bytes)?
            } else {
                Vec::new()
            };

        let bfpe = values
            .get(&b"BFpe"[..])
            .and_then(|v| v.as_bytes_raw())
            .map(<[u8]>::to_vec);
        let bfsd = values
            .get(&b"BFsd"[..])
            .and_then(|v| v.as_bytes_raw())
            .map(<[u8]>::to_vec);

        return Ok(KrpcResponse::GetPeers(GetPeersResponse {
            id,
            token,
            peers,
            nodes,
            nodes6,
            bfpe,
            bfsd,
        }));
    }

    // find_node response: has "nodes" or "nodes6"
    let has_nodes = values.contains_key(&b"nodes"[..]);
    let has_nodes6 = values.contains_key(&b"nodes6"[..]);

    if has_nodes || has_nodes6 {
        let nodes =
            if let Some(nodes_bytes) = values.get(&b"nodes"[..]).and_then(|v| v.as_bytes_raw()) {
                parse_compact_nodes(nodes_bytes)?
            } else {
                Vec::new()
            };

        let nodes6 =
            if let Some(nodes6_bytes) = values.get(&b"nodes6"[..]).and_then(|v| v.as_bytes_raw()) {
                parse_compact_nodes6(nodes6_bytes)?
            } else {
                Vec::new()
            };

        return Ok(KrpcResponse::FindNode { id, nodes, nodes6 });
    }

    // Plain ID response (ping, announce_peer)
    Ok(KrpcResponse::NodeId { id })
}

/// Decode a BEP 44 `GetItem` response from its value dict.
fn decode_get_item_response(
    id: Id20,
    values: &BTreeMap<Vec<u8>, BencodeValue>,
) -> Result<KrpcResponse> {
    let token = values
        .get(&b"token"[..])
        .and_then(|v| v.as_bytes_raw())
        .map(<[u8]>::to_vec);

    let nodes = if let Some(nodes_bytes) = values.get(&b"nodes"[..]).and_then(|v| v.as_bytes_raw())
    {
        parse_compact_nodes(nodes_bytes)?
    } else {
        Vec::new()
    };

    let nodes6 =
        if let Some(nodes6_bytes) = values.get(&b"nodes6"[..]).and_then(|v| v.as_bytes_raw()) {
            parse_compact_nodes6(nodes6_bytes)?
        } else {
            Vec::new()
        };

    let value = values
        .get(&b"v"[..])
        .and_then(|v| v.as_bytes_raw())
        .map(<[u8]>::to_vec);

    let key = values
        .get(&b"k"[..])
        .and_then(|v| v.as_bytes_raw())
        .and_then(|b| <[u8; 32]>::try_from(b).ok());

    let signature = values
        .get(&b"sig"[..])
        .and_then(|v| v.as_bytes_raw())
        .and_then(|b| <[u8; 64]>::try_from(b).ok());

    let seq = values
        .get(&b"seq"[..])
        .and_then(gaia_bencode::BencodeValue::as_int);

    Ok(KrpcResponse::GetItem {
        id,
        token,
        nodes,
        nodes6,
        value,
        key,
        signature,
        seq,
    })
}

// ---- Dict access helpers ----

fn dict_bytes<'a>(dict: &'a BTreeMap<Vec<u8>, BencodeValue>, key: &[u8]) -> Result<&'a [u8]> {
    dict.get(key).and_then(|v| v.as_bytes_raw()).ok_or_else(|| {
        Error::InvalidMessage(format!(
            "missing or invalid key '{}'",
            String::from_utf8_lossy(key)
        ))
    })
}

fn dict_str<'a>(dict: &'a BTreeMap<Vec<u8>, BencodeValue>, key: &[u8]) -> Result<&'a [u8]> {
    dict_bytes(dict, key)
}

fn dict_dict<'a>(
    dict: &'a BTreeMap<Vec<u8>, BencodeValue>,
    key: &[u8],
) -> Result<&'a BTreeMap<Vec<u8>, BencodeValue>> {
    dict.get(key).and_then(|v| v.as_dict()).ok_or_else(|| {
        Error::InvalidMessage(format!(
            "missing or invalid dict key '{}'",
            String::from_utf8_lossy(key)
        ))
    })
}

fn args_id20(args: &BTreeMap<Vec<u8>, BencodeValue>, key: &[u8]) -> Result<Id20> {
    let bytes = args
        .get(key)
        .and_then(|v| v.as_bytes_raw())
        .ok_or_else(|| {
            Error::InvalidMessage(format!(
                "missing '{}' in args",
                String::from_utf8_lossy(key)
            ))
        })?;
    Id20::from_bytes(bytes).map_err(|e| Error::InvalidMessage(e.to_string()))
}

fn args_int(args: &BTreeMap<Vec<u8>, BencodeValue>, key: &[u8]) -> Result<i64> {
    args.get(key)
        .and_then(gaia_bencode::BencodeValue::as_int)
        .ok_or_else(|| {
            Error::InvalidMessage(format!(
                "missing '{}' integer in args",
                String::from_utf8_lossy(key)
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn test_id() -> Id20 {
        Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap()
    }

    fn target_id() -> Id20 {
        Id20::from_hex("0000000000000000000000000000000000000001").unwrap()
    }

    #[test]
    fn ping_query_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(42),
            body: KrpcBody::Query(KrpcQuery::Ping { id: test_id() }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn ping_response_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(42),
            body: KrpcBody::Response(KrpcResponse::NodeId { id: test_id() }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn find_node_query_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(100),
            body: KrpcBody::Query(KrpcQuery::FindNode {
                id: test_id(),
                target: target_id(),
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn find_node_response_round_trip() {
        let nodes = vec![CompactNodeInfo {
            id: target_id(),
            addr: "10.0.0.1:6881".parse().unwrap(),
        }];
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(100),
            body: KrpcBody::Response(KrpcResponse::FindNode {
                id: test_id(),
                nodes,
                nodes6: Vec::new(),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_peers_query_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(200),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: test_id(),
                info_hash: target_id(),
                noseed: None,
                scrape: None,
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_peers_response_with_peers_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(200),
            body: KrpcBody::Response(KrpcResponse::GetPeers(GetPeersResponse {
                id: test_id(),
                token: Some(b"aoeusnth".to_vec()),
                peers: vec!["192.168.1.1:6881".parse().unwrap()],
                nodes: Vec::new(),
                nodes6: Vec::new(),
                bfpe: None,
                bfsd: None,
            })),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_peers_response_with_nodes_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(200),
            body: KrpcBody::Response(KrpcResponse::GetPeers(GetPeersResponse {
                id: test_id(),
                token: Some(b"token123".to_vec()),
                peers: Vec::new(),
                nodes: vec![CompactNodeInfo {
                    id: target_id(),
                    addr: "10.0.0.1:6881".parse().unwrap(),
                }],
                nodes6: Vec::new(),
                bfpe: None,
                bfsd: None,
            })),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn announce_peer_query_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(300),
            body: KrpcBody::Query(KrpcQuery::AnnouncePeer {
                id: test_id(),
                info_hash: target_id(),
                port: 6881,
                implied_port: true,
                token: b"aoeusnth".to_vec(),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn error_message_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(500),
            body: KrpcBody::Error {
                code: 201,
                message: "A Generic Error Occurred".into(),
            },
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn decode_bep5_ping_example() {
        // BEP 5 example: ping query
        // d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe
        let data = b"d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe";
        let msg = KrpcMessage::from_bytes(data).unwrap();
        assert_eq!(msg.transaction_id.0, *b"aa");
        match &msg.body {
            KrpcBody::Query(KrpcQuery::Ping { id }) => {
                assert_eq!(id.as_bytes(), b"abcdefghij0123456789");
            }
            other => panic!("expected Ping query, got {other:?}"),
        }
    }

    #[test]
    fn decode_bep5_error_example() {
        // BEP 5 example: generic error (corrected length: 24 chars)
        let data = b"d1:eli201e24:A Generic Error Occurrede1:t2:aa1:y1:ee";
        let msg = KrpcMessage::from_bytes(data).unwrap();
        assert_eq!(msg.transaction_id.0, *b"aa");
        match &msg.body {
            KrpcBody::Error { code, message } => {
                assert_eq!(*code, 201);
                assert_eq!(message, "A Generic Error Occurred");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn transaction_id_from_single_byte() {
        let tid = TransactionId::from_bytes(&[0x42]).unwrap();
        assert_eq!(tid.0, [0x42, 0x00]);
    }

    #[test]
    fn query_method_names() {
        assert_eq!(KrpcQuery::Ping { id: Id20::ZERO }.method_name(), "ping");
        assert_eq!(
            KrpcQuery::FindNode {
                id: Id20::ZERO,
                target: Id20::ZERO,
                want: None,
            }
            .method_name(),
            "find_node"
        );
    }

    // --- IPv6 KRPC tests ---

    #[test]
    fn find_node_response_with_nodes6_round_trip() {
        let nodes = vec![CompactNodeInfo {
            id: target_id(),
            addr: "10.0.0.1:6881".parse().unwrap(),
        }];
        let nodes6 = vec![CompactNodeInfo6 {
            id: target_id(),
            addr: "[2001:db8::1]:6881".parse().unwrap(),
        }];
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(100),
            body: KrpcBody::Response(KrpcResponse::FindNode {
                id: test_id(),
                nodes,
                nodes6,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_peers_response_with_nodes6_round_trip() {
        let nodes6 = vec![CompactNodeInfo6 {
            id: target_id(),
            addr: "[::1]:8080".parse().unwrap(),
        }];
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(200),
            body: KrpcBody::Response(KrpcResponse::GetPeers(GetPeersResponse {
                id: test_id(),
                token: Some(b"tok".to_vec()),
                peers: Vec::new(),
                nodes: Vec::new(),
                nodes6,
                bfpe: None,
                bfsd: None,
            })),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_peers_response_with_ipv6_peer_values() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(200),
            body: KrpcBody::Response(KrpcResponse::GetPeers(GetPeersResponse {
                id: test_id(),
                token: Some(b"tok".to_vec()),
                peers: vec![
                    "192.168.1.1:6881".parse().unwrap(),
                    "[2001:db8::1]:8080".parse().unwrap(),
                ],
                nodes: Vec::new(),
                nodes6: Vec::new(),
                bfpe: None,
                bfsd: None,
            })),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    // --- BEP 42 ip field tests ---

    #[test]
    fn response_with_ip_field_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(42),
            body: KrpcBody::Response(KrpcResponse::NodeId { id: test_id() }),
            sender_ip: Some("203.0.113.5:6881".parse().unwrap()),
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.sender_ip, Some("203.0.113.5:6881".parse().unwrap()));
    }

    #[test]
    fn response_with_ipv6_ip_field_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(42),
            body: KrpcBody::Response(KrpcResponse::NodeId { id: test_id() }),
            sender_ip: Some("[2001:db8::1]:6881".parse().unwrap()),
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(
            decoded.sender_ip,
            Some("[2001:db8::1]:6881".parse().unwrap())
        );
    }

    #[test]
    fn message_without_ip_field_parses_as_none() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(42),
            body: KrpcBody::Response(KrpcResponse::NodeId { id: test_id() }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert!(decoded.sender_ip.is_none());
    }

    // --- BEP 44 KRPC tests ---

    #[test]
    fn get_immutable_query_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(400),
            body: KrpcBody::Query(KrpcQuery::Get {
                id: test_id(),
                target: target_id(),
                seq: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_mutable_query_with_seq_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(401),
            body: KrpcBody::Query(KrpcQuery::Get {
                id: test_id(),
                target: target_id(),
                seq: Some(42),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn put_immutable_query_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(402),
            body: KrpcBody::Query(KrpcQuery::Put {
                id: test_id(),
                token: b"tok12345".to_vec(),
                value: b"12:Hello World!".to_vec(),
                key: None,
                signature: None,
                seq: None,
                salt: None,
                cas: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn put_mutable_query_round_trip() {
        let key = [0xABu8; 32];
        let sig = [0xCDu8; 64];
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(403),
            body: KrpcBody::Query(KrpcQuery::Put {
                id: test_id(),
                token: b"tok12345".to_vec(),
                value: b"12:Hello World!".to_vec(),
                key: Some(key),
                signature: Some(sig),
                seq: Some(4),
                salt: Some(b"foobar".to_vec()),
                cas: Some(3),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_response_immutable_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(404),
            body: KrpcBody::Response(KrpcResponse::GetItem {
                id: test_id(),
                token: Some(b"tok".to_vec()),
                nodes: Vec::new(),
                nodes6: Vec::new(),
                value: Some(b"12:Hello World!".to_vec()),
                key: None,
                signature: None,
                seq: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_response_mutable_round_trip() {
        let key = [0xABu8; 32];
        let sig = [0xCDu8; 64];
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(405),
            body: KrpcBody::Response(KrpcResponse::GetItem {
                id: test_id(),
                token: Some(b"tok".to_vec()),
                nodes: vec![CompactNodeInfo {
                    id: target_id(),
                    addr: "10.0.0.1:6881".parse().unwrap(),
                }],
                nodes6: Vec::new(),
                value: Some(b"4:test".to_vec()),
                key: Some(key),
                signature: Some(sig),
                seq: Some(7),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn get_response_not_found_with_hint_round_trip() {
        // "Not found" BEP 44 response: only token + nodes, no v/k/sig/seq.
        // Without the query_method hint this would be decoded as GetPeers.
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(406),
            body: KrpcBody::Response(KrpcResponse::GetItem {
                id: test_id(),
                token: Some(b"tok".to_vec()),
                nodes: vec![CompactNodeInfo {
                    id: target_id(),
                    addr: "10.0.0.1:6881".parse().unwrap(),
                }],
                nodes6: Vec::new(),
                value: None,
                key: None,
                signature: None,
                seq: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        // Use from_bytes_with_query_hint to force GetItem decoding
        let decoded = KrpcMessage::from_bytes_with_query_hint(&bytes, "get").unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn bep44_query_method_names() {
        assert_eq!(
            KrpcQuery::Get {
                id: Id20::ZERO,
                target: Id20::ZERO,
                seq: None,
            }
            .method_name(),
            "get"
        );
        assert_eq!(
            KrpcQuery::Put {
                id: Id20::ZERO,
                token: Vec::new(),
                value: Vec::new(),
                key: None,
                signature: None,
                seq: None,
                salt: None,
                cas: None,
            }
            .method_name(),
            "put"
        );
    }

    // --- BEP 51 KRPC tests ---

    #[test]
    fn sample_infohashes_query_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(400),
            body: KrpcBody::Query(KrpcQuery::SampleInfohashes {
                id: test_id(),
                target: target_id(),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn sample_infohashes_response_round_trip() {
        let sample1 = Id20::from_hex("aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d").unwrap();
        let sample2 = Id20::from_hex("0000000000000000000000000000000000000001").unwrap();
        let nodes = vec![CompactNodeInfo {
            id: target_id(),
            addr: "10.0.0.1:6881".parse().unwrap(),
        }];
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(400),
            body: KrpcBody::Response(KrpcResponse::SampleInfohashes(SampleInfohashesResponse {
                id: test_id(),
                interval: 300,
                num: 42,
                samples: vec![sample1, sample2],
                nodes,
            })),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn sample_infohashes_response_empty_samples() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(401),
            body: KrpcBody::Response(KrpcResponse::SampleInfohashes(SampleInfohashesResponse {
                id: test_id(),
                interval: 60,
                num: 0,
                samples: Vec::new(),
                nodes: Vec::new(),
            })),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn sample_infohashes_query_method_name() {
        assert_eq!(
            KrpcQuery::SampleInfohashes {
                id: Id20::ZERO,
                target: Id20::ZERO,
            }
            .method_name(),
            "sample_infohashes"
        );
    }

    // --- BEP 43 read-only node tests ---

    #[test]
    fn krpc_ro_flag_roundtrip() {
        // Encode a query with read_only: true, decode, verify the flag survives.
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(43),
            body: KrpcBody::Query(KrpcQuery::Ping { id: test_id() }),
            sender_ip: None,
            read_only: true,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert!(decoded.read_only, "ro flag should survive round-trip");
        assert_eq!(decoded.body, msg.body);
    }

    #[test]
    fn krpc_ro_absent_defaults_false() {
        // A message without an `ro` field should decode with read_only == false.
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(43),
            body: KrpcBody::Query(KrpcQuery::Ping { id: test_id() }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert!(!decoded.read_only, "absent ro should default to false");
    }

    // --- BEP 33 DHT scrape tests ---

    #[test]
    fn krpc_get_peers_with_scrape_flag() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(330),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: test_id(),
                info_hash: target_id(),
                noseed: None,
                scrape: Some(1),
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
        match &decoded.body {
            KrpcBody::Query(KrpcQuery::GetPeers { scrape, noseed, .. }) => {
                assert_eq!(*scrape, Some(1));
                assert_eq!(*noseed, None);
            }
            other => panic!("expected GetPeers query, got {other:?}"),
        }
    }

    #[test]
    fn krpc_get_peers_response_with_bloom() {
        let bfpe_data = vec![0xAA; 256];
        let bfsd_data = vec![0x55; 256];
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(331),
            body: KrpcBody::Response(KrpcResponse::GetPeers(GetPeersResponse {
                id: test_id(),
                token: Some(b"tok".to_vec()),
                peers: vec!["192.168.1.1:6881".parse().unwrap()],
                nodes: Vec::new(),
                nodes6: Vec::new(),
                bfpe: Some(bfpe_data.clone()),
                bfsd: Some(bfsd_data.clone()),
            })),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
        match &decoded.body {
            KrpcBody::Response(KrpcResponse::GetPeers(gp)) => {
                assert_eq!(gp.bfpe.as_deref(), Some(bfpe_data.as_slice()));
                assert_eq!(gp.bfsd.as_deref(), Some(bfsd_data.as_slice()));
            }
            other => panic!("expected GetPeers response, got {other:?}"),
        }
    }

    // --- BEP 5 spec vector conformance tests ---

    /// Helper: build an Id20 from a 20-byte ASCII string (as used in BEP 5 examples).
    fn bep5_id(ascii: &[u8; 20]) -> Id20 {
        Id20(*ascii)
    }

    #[test]
    fn krpc_decode_find_node_query_spec() {
        // BEP 5 example: find_node query
        let data = b"d1:ad2:id20:abcdefghij01234567896:target20:mnopqrstuvwxyz123456e1:q9:find_node1:t2:aa1:y1:qe";
        let msg = KrpcMessage::from_bytes(data).unwrap();
        assert_eq!(msg.transaction_id.0, *b"aa");
        assert!(!msg.read_only);
        match &msg.body {
            KrpcBody::Query(KrpcQuery::FindNode { id, target, .. }) => {
                assert_eq!(id.as_bytes(), b"abcdefghij0123456789");
                assert_eq!(target.as_bytes(), b"mnopqrstuvwxyz123456");
            }
            other => panic!("expected FindNode query, got {other:?}"),
        }
    }

    #[test]
    fn krpc_decode_get_peers_query_spec() {
        // BEP 5 example: get_peers query
        let data = b"d1:ad2:id20:abcdefghij01234567899:info_hash20:mnopqrstuvwxyz123456e1:q9:get_peers1:t2:aa1:y1:qe";
        let msg = KrpcMessage::from_bytes(data).unwrap();
        assert_eq!(msg.transaction_id.0, *b"aa");
        assert!(!msg.read_only);
        match &msg.body {
            KrpcBody::Query(KrpcQuery::GetPeers {
                id,
                info_hash,
                noseed,
                scrape,
                ..
            }) => {
                assert_eq!(id.as_bytes(), b"abcdefghij0123456789");
                assert_eq!(info_hash.as_bytes(), b"mnopqrstuvwxyz123456");
                assert_eq!(*noseed, None);
                assert_eq!(*scrape, None);
            }
            other => panic!("expected GetPeers query, got {other:?}"),
        }
    }

    #[test]
    fn krpc_decode_announce_peer_query_spec() {
        // BEP 5 example: announce_peer query
        let data = b"d1:ad2:id20:abcdefghij012345678912:implied_porti1e9:info_hash20:mnopqrstuvwxyz1234564:porti6881e5:token8:aoaborahe1:q13:announce_peer1:t2:aa1:y1:qe";
        let msg = KrpcMessage::from_bytes(data).unwrap();
        assert_eq!(msg.transaction_id.0, *b"aa");
        assert!(!msg.read_only);
        match &msg.body {
            KrpcBody::Query(KrpcQuery::AnnouncePeer {
                id,
                info_hash,
                port,
                implied_port,
                token,
            }) => {
                assert_eq!(id.as_bytes(), b"abcdefghij0123456789");
                assert_eq!(info_hash.as_bytes(), b"mnopqrstuvwxyz123456");
                assert_eq!(*port, 6881);
                assert!(*implied_port);
                assert_eq!(token, b"aoaborah");
            }
            other => panic!("expected AnnouncePeer query, got {other:?}"),
        }
    }

    // ---- M241 L5: announce_peer port range-checking ----

    fn m241_announce_args(
        port: Option<i64>,
        implied: Option<i64>,
    ) -> BTreeMap<Vec<u8>, BencodeValue> {
        let mut args = BTreeMap::new();
        args.insert(b"id".to_vec(), BencodeValue::Bytes(vec![0u8; 20]));
        args.insert(b"info_hash".to_vec(), BencodeValue::Bytes(vec![1u8; 20]));
        if let Some(p) = port {
            args.insert(b"port".to_vec(), BencodeValue::Integer(p));
        }
        if let Some(i) = implied {
            args.insert(b"implied_port".to_vec(), BencodeValue::Integer(i));
        }
        args.insert(b"token".to_vec(), BencodeValue::Bytes(b"tok".to_vec()));
        args
    }

    #[test]
    fn m241_announce_peer_rejects_port_above_u16() {
        // 70000 was silently truncated to 4464 before M241.
        let args = m241_announce_args(Some(70000), None);
        assert!(matches!(
            decode_query(b"announce_peer", &args),
            Err(Error::InvalidMessage(_))
        ));
    }

    #[test]
    fn m241_announce_peer_rejects_negative_port() {
        // -1 was silently truncated to 65535 before M241.
        let args = m241_announce_args(Some(-1), None);
        assert!(matches!(
            decode_query(b"announce_peer", &args),
            Err(Error::InvalidMessage(_))
        ));
    }

    #[test]
    fn m241_announce_peer_rejects_zero_port_without_implied() {
        let args = m241_announce_args(Some(0), None);
        assert!(matches!(
            decode_query(b"announce_peer", &args),
            Err(Error::InvalidMessage(_))
        ));
    }

    #[test]
    fn m241_announce_peer_accepts_zero_port_with_implied() {
        // implied_port=1: the source port is authoritative and `port` is ignored
        // downstream, so port=0 must NOT be rejected.
        let args = m241_announce_args(Some(0), Some(1));
        assert!(matches!(
            decode_query(b"announce_peer", &args),
            Ok(KrpcQuery::AnnouncePeer {
                port: 0,
                implied_port: true,
                ..
            })
        ));
    }

    #[test]
    fn m241_announce_peer_accepts_out_of_range_port_with_implied() {
        // F6: with implied_port set, an out-of-range `port` is ignored downstream
        // and must not reject the whole query (BEP 5 interop) — normalized to 0.
        let args = m241_announce_args(Some(70000), Some(1));
        assert!(matches!(
            decode_query(b"announce_peer", &args),
            Ok(KrpcQuery::AnnouncePeer {
                port: 0,
                implied_port: true,
                ..
            })
        ));
    }

    #[test]
    fn m241_announce_peer_accepts_valid_port() {
        let args = m241_announce_args(Some(6881), None);
        assert!(matches!(
            decode_query(b"announce_peer", &args),
            Ok(KrpcQuery::AnnouncePeer { port: 6881, .. })
        ));
    }

    #[test]
    fn krpc_encode_ping_matches_spec() {
        // BEP 5 example: ping query
        // d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe
        //
        // Our encoder should produce byte-identical output when read_only=false
        // and sender_ip=None, since BTreeMap ordering matches bencode key order
        // and no extra fields are added.
        let msg = KrpcMessage {
            transaction_id: TransactionId(*b"aa"),
            body: KrpcBody::Query(KrpcQuery::Ping {
                id: bep5_id(b"abcdefghij0123456789"),
            }),
            sender_ip: None,
            read_only: false,
        };
        let encoded = msg.to_bytes().unwrap();
        let expected = b"d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe";
        assert_eq!(
            encoded, expected,
            "encoded ping query should match BEP 5 spec bytes exactly"
        );

        // Verify required fields survive decode
        let decoded = KrpcMessage::from_bytes(&encoded).unwrap();
        assert_eq!(decoded.transaction_id.0, *b"aa");
        match &decoded.body {
            KrpcBody::Query(KrpcQuery::Ping { id }) => {
                assert_eq!(id.as_bytes(), b"abcdefghij0123456789");
            }
            other => panic!("expected Ping query, got {other:?}"),
        }
    }

    #[test]
    fn krpc_encode_roundtrip_all_types() {
        // Encode -> decode -> verify field equality for multiple message types.
        let node_id = bep5_id(b"abcdefghij0123456789");
        let target = bep5_id(b"mnopqrstuvwxyz123456");

        // 1. Ping query
        let ping = KrpcMessage {
            transaction_id: TransactionId::from_u16(1),
            body: KrpcBody::Query(KrpcQuery::Ping { id: node_id }),
            sender_ip: None,
            read_only: false,
        };
        let ping_decoded = KrpcMessage::from_bytes(&ping.to_bytes().unwrap()).unwrap();
        assert_eq!(ping, ping_decoded, "ping round-trip mismatch");

        // 2. FindNode query
        let find_node = KrpcMessage {
            transaction_id: TransactionId::from_u16(2),
            body: KrpcBody::Query(KrpcQuery::FindNode {
                id: node_id,
                target,
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let find_node_decoded = KrpcMessage::from_bytes(&find_node.to_bytes().unwrap()).unwrap();
        assert_eq!(
            find_node, find_node_decoded,
            "find_node round-trip mismatch"
        );

        // 3. GetPeers query
        let get_peers = KrpcMessage {
            transaction_id: TransactionId::from_u16(3),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: node_id,
                info_hash: target,
                noseed: None,
                scrape: None,
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let get_peers_decoded = KrpcMessage::from_bytes(&get_peers.to_bytes().unwrap()).unwrap();
        assert_eq!(
            get_peers, get_peers_decoded,
            "get_peers round-trip mismatch"
        );

        // 4. AnnouncePeer query
        let announce = KrpcMessage {
            transaction_id: TransactionId::from_u16(4),
            body: KrpcBody::Query(KrpcQuery::AnnouncePeer {
                id: node_id,
                info_hash: target,
                port: 6881,
                implied_port: true,
                token: b"tok123".to_vec(),
            }),
            sender_ip: None,
            read_only: false,
        };
        let announce_decoded = KrpcMessage::from_bytes(&announce.to_bytes().unwrap()).unwrap();
        assert_eq!(
            announce, announce_decoded,
            "announce_peer round-trip mismatch"
        );
        match &announce_decoded.body {
            KrpcBody::Query(KrpcQuery::AnnouncePeer {
                id,
                info_hash,
                port,
                implied_port,
                token,
            }) => {
                assert_eq!(*id, node_id);
                assert_eq!(*info_hash, target);
                assert_eq!(*port, 6881);
                assert!(*implied_port);
                assert_eq!(token, b"tok123");
            }
            other => panic!("expected AnnouncePeer query, got {other:?}"),
        }

        // 5. Error
        let error = KrpcMessage {
            transaction_id: TransactionId::from_u16(5),
            body: KrpcBody::Error {
                code: 201,
                message: "A Generic Error Occurred".into(),
            },
            sender_ip: None,
            read_only: false,
        };
        let error_decoded = KrpcMessage::from_bytes(&error.to_bytes().unwrap()).unwrap();
        assert_eq!(error, error_decoded, "error round-trip mismatch");

        // 6. Response (ping response — just a node ID)
        let response = KrpcMessage {
            transaction_id: TransactionId::from_u16(6),
            body: KrpcBody::Response(KrpcResponse::NodeId { id: node_id }),
            sender_ip: None,
            read_only: false,
        };
        let response_decoded = KrpcMessage::from_bytes(&response.to_bytes().unwrap()).unwrap();
        assert_eq!(response, response_decoded, "response round-trip mismatch");
        match &response_decoded.body {
            KrpcBody::Response(KrpcResponse::NodeId { id }) => {
                assert_eq!(*id, node_id);
            }
            other => panic!("expected NodeId response, got {other:?}"),
        }
    }

    #[test]
    fn scrape_response_noseed_parsed() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(332),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: test_id(),
                info_hash: target_id(),
                noseed: Some(1),
                scrape: None,
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
        match &decoded.body {
            KrpcBody::Query(KrpcQuery::GetPeers { noseed, scrape, .. }) => {
                assert_eq!(*noseed, Some(1));
                assert_eq!(*scrape, None);
            }
            other => panic!("expected GetPeers query, got {other:?}"),
        }
    }

    // ── BEP 45 want field tests ──────────────────────────────────────

    #[test]
    fn want_family_round_trip_bytes() {
        assert_eq!(WantFamily::from_bytes(b"n4"), Some(WantFamily::N4));
        assert_eq!(WantFamily::from_bytes(b"n6"), Some(WantFamily::N6));
        assert_eq!(WantFamily::from_bytes(b"n8"), None);
        assert_eq!(WantFamily::N4.as_bytes(), b"n4");
        assert_eq!(WantFamily::N6.as_bytes(), b"n6");
    }

    #[test]
    fn find_node_want_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(500),
            body: KrpcBody::Query(KrpcQuery::FindNode {
                id: test_id(),
                target: target_id(),
                want: Some(vec![WantFamily::N4, WantFamily::N6]),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
        match &decoded.body {
            KrpcBody::Query(KrpcQuery::FindNode { want, .. }) => {
                let w = want.as_ref().expect("want should be present");
                assert_eq!(w.len(), 2);
                assert_eq!(w[0], WantFamily::N4);
                assert_eq!(w[1], WantFamily::N6);
            }
            other => panic!("expected FindNode query, got {other:?}"),
        }
    }

    #[test]
    fn get_peers_want_round_trip() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(501),
            body: KrpcBody::Query(KrpcQuery::GetPeers {
                id: test_id(),
                info_hash: target_id(),
                noseed: None,
                scrape: None,
                want: Some(vec![WantFamily::N6]),
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        assert_eq!(msg, decoded);
        match &decoded.body {
            KrpcBody::Query(KrpcQuery::GetPeers { want, .. }) => {
                let w = want.as_ref().expect("want should be present");
                assert_eq!(w, &[WantFamily::N6]);
            }
            other => panic!("expected GetPeers query, got {other:?}"),
        }
    }

    #[test]
    fn find_node_want_none_omitted() {
        let msg = KrpcMessage {
            transaction_id: TransactionId::from_u16(502),
            body: KrpcBody::Query(KrpcQuery::FindNode {
                id: test_id(),
                target: target_id(),
                want: None,
            }),
            sender_ip: None,
            read_only: false,
        };
        let bytes = msg.to_bytes().unwrap();
        let decoded = KrpcMessage::from_bytes(&bytes).unwrap();
        match &decoded.body {
            KrpcBody::Query(KrpcQuery::FindNode { want, .. }) => {
                assert!(want.is_none(), "want should be omitted when None");
            }
            other => panic!("expected FindNode query, got {other:?}"),
        }
    }

    #[test]
    fn want_unknown_family_filtered() {
        let mut args = BTreeMap::new();
        args.insert(
            b"want".to_vec(),
            BencodeValue::List(vec![
                BencodeValue::Bytes(b"n4".to_vec()),
                BencodeValue::Bytes(b"n9".to_vec()),
                BencodeValue::Bytes(b"n6".to_vec()),
            ]),
        );
        let want = decode_want(&args).expect("should parse known families");
        assert_eq!(want, vec![WantFamily::N4, WantFamily::N6]);
    }
}
