import re
with open('apps/crawler/src/router.rs', 'r') as f:
    text = f.read()

fast_ping_code = """
    fn respond_ping_fast(&self, t: &[u8], from: SocketAddr) {
        use std::io::Write;
        let mut buf = [0u8; 128];
        let mut pos = 0;
        let b1 = b"d1:rd2:id20:";
        buf[pos..pos+b1.len()].copy_from_slice(b1);
        pos += b1.len();
        buf[pos..pos+20].copy_from_slice(self.self_id());
        pos += 20;
        let b2 = b"e1:t";
        buf[pos..pos+b2.len()].copy_from_slice(b2);
        pos += b2.len();
        
        let mut cursor = std::io::Cursor::new(&mut buf[pos..]);
        write!(cursor, "{}:", t.len()).unwrap();
        let written = cursor.position() as usize;
        pos += written;
        
        if pos + t.len() + 7 <= buf.len() {
            buf[pos..pos+t.len()].copy_from_slice(t);
            pos += t.len();
            let b3 = b"1:y1:re";
            buf[pos..pos+b3.len()].copy_from_slice(b3);
            pos += b3.len();
            self.try_send(&buf[..pos], from);
        }
    }
"""

handle_query_replacement = """                    if q == PING {
                        self.metrics.inbound_ping.add(1);
                        if let Some(t) = header.t {
                            self.respond_ping_fast(t, from);
                        }
                        return;
                    }
                    if q == FIND_NODE {"""

text = text.replace('                    if q == FIND_NODE {', handle_query_replacement)
text = text.replace('    fn send_response', fast_ping_code + '\n    fn send_response')

with open('apps/crawler/src/router.rs', 'w') as f:
    f.write(text)
