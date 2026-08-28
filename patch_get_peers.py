import re
with open('apps/crawler/src/router.rs', 'r') as f:
    text = f.read()

fast_get_peers_code = """
    fn respond_get_peers_fast(&self, t: &[u8], ih: &[u8; 20], from: SocketAddr) {
        use std::io::Write;
        match self.classify_pool(ih) {
            SybilPool::Bep42 => self.metrics.inbound_get_peers_bep42.add(1),
            SybilPool::Random => self.metrics.inbound_get_peers_random.add(1),
        }
        self.do_harvest(*ih, crate::harvest::Source::GetPeers, None);
        let token = self.token.read().expect("token").generate(from.ip());
        self.metrics.tokens_issued.add(1);
        
        let nodes = self.closest_phantom(ih, 8);
        let compact = crate::dht::routing_table::encode_compact(&nodes);
        
        let mut buf = [0u8; 512];
        let mut pos = 0;
        
        let b1 = b"d1:rd2:id20:";
        buf[pos..pos+b1.len()].copy_from_slice(b1);
        pos += b1.len();
        
        buf[pos..pos+20].copy_from_slice(&self.self_id);
        pos += 20;
        
        let b2 = b"5:nodes";
        buf[pos..pos+b2.len()].copy_from_slice(b2);
        pos += b2.len();
        
        let mut cursor = std::io::Cursor::new(&mut buf[pos..]);
        write!(cursor, "{}:", compact.len()).unwrap();
        pos += cursor.position() as usize;
        
        buf[pos..pos+compact.len()].copy_from_slice(&compact);
        pos += compact.len();
        
        let b3 = b"5:token8:";
        buf[pos..pos+b3.len()].copy_from_slice(b3);
        pos += b3.len();
        
        buf[pos..pos+8].copy_from_slice(&token);
        pos += 8;
        
        let b4 = b"e1:t";
        buf[pos..pos+b4.len()].copy_from_slice(b4);
        pos += b4.len();
        
        let mut cursor = std::io::Cursor::new(&mut buf[pos..]);
        write!(cursor, "{}:", t.len()).unwrap();
        pos += cursor.position() as usize;
        
        buf[pos..pos+t.len()].copy_from_slice(t);
        pos += t.len();
        
        let b5 = b"1:y1:re";
        buf[pos..pos+b5.len()].copy_from_slice(b5);
        pos += b5.len();
        
        self.try_send(&buf[..pos], from);
    }
    
    fn extract_info_hash(buf: &[u8]) -> Option<[u8; 20]> {
        let target = b"9:info_hash20:";
        if let Some(pos) = buf.windows(target.len()).position(|w| w == target) {
            let start = pos + target.len();
            if start + 20 <= buf.len() {
                let mut ih = [0u8; 20];
                ih.copy_from_slice(&buf[start..start+20]);
                return Some(ih);
            }
        }
        None
    }
"""

handle_query_replacement = """                    if q == PING {
                        self.metrics.inbound_ping.add(1);
                        if let Some(t) = header.t {
                            self.respond_ping_fast(t, from);
                        }
                        return;
                    }
                    if q == GET_PEERS {
                        self.metrics.inbound_get_peers.add(1);
                        if let (Some(t), Some(ih)) = (header.t, Self::extract_info_hash(buf)) {
                            self.respond_get_peers_fast(t, &ih, from);
                        }
                        return;
                    }
                    if q == FIND_NODE {"""

text = text.replace('                    if q == PING {', handle_query_replacement.split('                    if q == PING {')[1])
text = text.replace('    fn respond_ping_fast', fast_get_peers_code + '\n    fn respond_ping_fast')

with open('apps/crawler/src/router.rs', 'w') as f:
    f.write(text)
