use crate::krpc::codec::{BValue, DecodeError, decode, encode_to_bytes};
use bytes::Bytes;
use std::collections::BTreeMap;

pub const PING: &[u8] = b"ping";
pub const FIND_NODE: &[u8] = b"find_node";
pub const GET_PEERS: &[u8] = b"get_peers";
pub const ANNOUNCE_PEER: &[u8] = b"announce_peer";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub t: Bytes,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Query { q: Bytes, a: BValue },
    Response { r: BValue },
    Error { e: BValue },
}

impl Message {
    pub fn query(t: Bytes, q: &[u8], a: BValue) -> Message {
        Message {
            t,
            kind: Kind::Query {
                q: Bytes::copy_from_slice(q),
                a,
            },
        }
    }

    pub fn response(t: Bytes, r: BValue) -> Message {
        Message {
            t,
            kind: Kind::Response { r },
        }
    }

    #[allow(dead_code)]
    pub fn error(t: Bytes, e: BValue) -> Message {
        Message {
            t,
            kind: Kind::Error { e },
        }
    }

    pub fn parse(buf: &Bytes) -> Result<Message, DecodeError> {
        let root = decode(buf)?;
        Message::from_value(root)
    }

    pub fn from_value(root: BValue) -> Result<Message, DecodeError> {
        let dict = root.as_dict().ok_or(DecodeError {
            msg: "message root is not a dict",
            pos: 0,
        })?;
        let t = dict
            .get(b"t".as_slice())
            .and_then(BValue::as_bytes)
            .cloned()
            .unwrap_or_default();
        let y = dict
            .get(b"y".as_slice())
            .and_then(BValue::as_bytes)
            .ok_or(DecodeError {
                msg: "missing y field",
                pos: 0,
            })?;
        let kind = match y.as_ref() {
            b"q" => {
                let q = dict
                    .get(b"q".as_slice())
                    .and_then(BValue::as_bytes)
                    .cloned()
                    .ok_or(DecodeError {
                        msg: "missing q field",
                        pos: 0,
                    })?;
                let a = dict
                    .get(b"a".as_slice())
                    .cloned()
                    .unwrap_or(BValue::Dict(BTreeMap::new()));
                Kind::Query { q, a }
            }
            b"r" => Kind::Response {
                r: dict.get(b"r".as_slice()).cloned().ok_or(DecodeError {
                    msg: "missing r field",
                    pos: 0,
                })?,
            },
            b"e" => Kind::Error {
                e: dict.get(b"e".as_slice()).cloned().ok_or(DecodeError {
                    msg: "missing e field",
                    pos: 0,
                })?,
            },
            _ => {
                return Err(DecodeError {
                    msg: "unknown y field",
                    pos: 0,
                });
            }
        };
        Ok(Message { t, kind })
    }

    #[allow(dead_code)]
    pub fn args(&self) -> Option<&BValue> {
        match &self.kind {
            Kind::Query { a, .. } => Some(a),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn query_name(&self) -> Option<&Bytes> {
        match &self.kind {
            Kind::Query { q, .. } => Some(q),
            _ => None,
        }
    }

    pub fn encode(&self) -> Bytes {
        let mut entries: Vec<(Bytes, BValue)> = Vec::with_capacity(4);
        if !self.t.is_empty() {
            entries.push((Bytes::from_static(b"t"), BValue::Bytes(self.t.clone())));
        }
        match &self.kind {
            Kind::Query { q, a } => {
                entries.push((
                    Bytes::from_static(b"y"),
                    BValue::Bytes(Bytes::from_static(b"q")),
                ));
                entries.push((Bytes::from_static(b"q"), BValue::Bytes(q.clone())));
                entries.push((Bytes::from_static(b"a"), a.clone()));
            }
            Kind::Response { r } => {
                entries.push((
                    Bytes::from_static(b"y"),
                    BValue::Bytes(Bytes::from_static(b"r")),
                ));
                entries.push((Bytes::from_static(b"r"), r.clone()));
            }
            Kind::Error { e } => {
                entries.push((
                    Bytes::from_static(b"y"),
                    BValue::Bytes(Bytes::from_static(b"e")),
                ));
                entries.push((Bytes::from_static(b"e"), e.clone()));
            }
        }
        encode_to_bytes(&BValue::dict(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ping() {
        let raw = Bytes::from_static(b"d1:ad2:id20:abcdefghij0123456789e1:q4:ping1:t2:aa1:y1:qe");
        let m = Message::parse(&raw).unwrap();
        assert_eq!(m.t, Bytes::from_static(b"aa"));
        assert_eq!(m.query_name(), Some(&Bytes::from_static(b"ping")));
        assert_eq!(
            m.args().and_then(|a| a.get_bytes(b"id")).cloned(),
            Some(Bytes::from_static(b"abcdefghij0123456789"))
        );
    }

    #[test]
    fn roundtrip_response() {
        let r = BValue::dict(vec![
            (
                Bytes::from_static(b"id"),
                BValue::Bytes(Bytes::from_static(b"abcdefghij0123456789")),
            ),
            (
                Bytes::from_static(b"token"),
                BValue::Bytes(Bytes::from_static(b"tok123")),
            ),
        ]);
        let m = Message::response(Bytes::from_static(b"aa"), r);
        let enc = m.encode();
        let back = Message::parse(&enc).unwrap();
        assert_eq!(back, m);
    }
}
