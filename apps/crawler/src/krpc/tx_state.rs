
use bytes::Bytes;
use dashmap::DashMap;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxKind {
    Ping,
    FindNode,
    GetPeers,
    AnnouncePeer,
}

#[derive(Debug)]
pub struct TxEntry {
    #[allow(dead_code)]
    pub kind: TxKind,
    pub sent: Instant,
    pub reply: Option<oneshot::Sender<Bytes>>,
}

pub struct TxTable {
    map: DashMap<Bytes, TxEntry>,
}

impl TxTable {
    pub fn new() -> Self {
        TxTable {
            map: DashMap::new(),
        }
    }

    pub fn insert(&self, t: Bytes, entry: TxEntry) -> bool {
        use dashmap::mapref::entry::Entry;
        match self.map.entry(t) {
            Entry::Vacant(v) => {
                v.insert(entry);
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub fn take(&self, t: &[u8]) -> Option<TxEntry> {
        self.map.remove(t).map(|(_, v)| v)
    }

    pub fn cleanup(&self, now: Instant, ttl: Duration) -> usize {
        let mut removed = 0;
        self.map.retain(|_, e| {
            if now.duration_since(e.sent) > ttl {
                removed += 1;
                false
            } else {
                true
            }
        });
        removed
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

impl Default for TxTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_take() {
        let t = TxTable::new();
        let (tx, _rx) = oneshot::channel();
        let key = Bytes::from_static(b"aa");
        assert!(t.insert(
            key.clone(),
            TxEntry {
                kind: TxKind::Ping,
                sent: Instant::now(),
                reply: Some(tx),
            }
        ));
        assert!(!t.insert(
            key.clone(),
            TxEntry {
                kind: TxKind::Ping,
                sent: Instant::now(),
                reply: None,
            }
        ));
        let e = t.take(&key).unwrap();
        assert_eq!(e.kind, TxKind::Ping);
        assert!(t.take(&key).is_none());
    }
}
