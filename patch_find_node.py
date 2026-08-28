import re
with open('apps/crawler/src/router.rs', 'r') as f:
    text = f.read()

fast_find_node_code = """
    fn extract_target(buf: &[u8]) -> Option<[u8; 20]> {
        let pat = b"6:target20:";
        if let Some(pos) = buf.windows(pat.len()).position(|w| w == pat) {
            let start = pos + pat.len();
            if start + 20 <= buf.len() {
                let mut t = [0u8; 20];
                t.copy_from_slice(&buf[start..start+20]);
                return Some(t);
            }
        }
        None
    }

    fn respond_find_node_fast(&self, t: &[u8], target: &[u8; 20], from: SocketAddr) {
        use std::io::Write;
        match self.classify_pool(target) {
            SybilPool::Bep42 => self.metrics.inbound_find_node_bep42.add(1),
            SybilPool::Random => self.metrics.inbound_find_node_random.add(1),
        }
        
        let nodes = self.closest_phantom(target, 8);
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
"""

replacement = """                    if q == FIND_NODE {
                        if self.find_node_response_percent < 100
                            && !crate::router::should_answer(self.find_node_response_percent, rand::random::<u16>())
                        {
                            self.metrics.inbound_find_node_dropped.add(1);
                            return;
                        }
                        self.metrics.inbound_find_node.add(1);
                        if let (Some(t), Some(target)) = (header.t, Self::extract_target(buf)) {
                            self.respond_find_node_fast(t, &target, from);
                        }
                        return;
                    }"""

text = text.replace('    fn extract_info_hash', fast_find_node_code + '\n    fn extract_info_hash')

# Be careful replacing the FIND_NODE block, it spans multiple lines.
old_find_node = """                    if q == FIND_NODE {
                        if self.find_node_response_percent < 100
                            && !crate::router::should_answer(self.find_node_response_percent, rand::random::<u16>())
                        {
                            self.metrics.inbound_find_node_dropped.add(1);
                            return;
                        }
                    }"""

text = text.replace(old_find_node, replacement)

with open('apps/crawler/src/router.rs', 'w') as f:
    f.write(text)
